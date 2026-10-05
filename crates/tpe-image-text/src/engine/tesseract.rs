//! The `tesseract` command-line program as an engine. It is found on `PATH`
//! (or given explicitly), run on a temporary PNG of the preprocessed image
//! with `OMP_NUM_THREADS=1`, a wall-clock deadline enforced by the controller
//! (kill + reap) and kernel limits installed by the `exec-limited` helper. Its
//! TSV output supplies per-word confidence and the block/paragraph/line tree.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use image::GrayImage;

use super::{Availability, BBox, Block, Engine, EngineOptions, Recognition};
use crate::ImageTextError;
use crate::limits::limited_command;

/// Most bytes of engine stdout/stderr read back.
const MAX_CAPTURE_BYTES: u64 = 32 * 1024 * 1024;
/// Bytes of stderr quoted in error messages.
const MAX_QUOTED_STDERR: usize = 2000;
/// Deadline for `tesseract --version`.
const VERSION_TIMEOUT: Duration = Duration::from_secs(15);

/// A located tesseract executable.
#[derive(Clone, Debug)]
pub struct TesseractEngine {
    bin: PathBuf,
    version: String,
    helper: Option<PathBuf>,
    timeout: Duration,
    address_space_bytes: u64,
}

/// Locate the executable: an explicit path must exist; otherwise `PATH` is searched.
pub fn find_binary(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return if path.is_file() {
            Ok(path.to_path_buf())
        } else {
            Err(format!(
                "configured tesseract executable does not exist: {}",
                path.display()
            ))
        };
    }
    let Some(path_var) = std::env::var_os("PATH") else {
        return Err("tesseract not found: PATH is unset".to_string());
    };
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join("tesseract");
        if candidate.is_file() && is_executable(&candidate) {
            return Ok(candidate);
        }
    }
    Err("tesseract not found on PATH (install tesseract-ocr, or set TPE_TESSERACT_BIN / --tesseract-bin)".to_string())
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    true
}

/// Detect tesseract and its version.
pub fn probe(opts: &EngineOptions) -> (Option<TesseractEngine>, Availability) {
    let bin = match find_binary(opts.tesseract_bin.as_deref()) {
        Ok(bin) => bin,
        Err(detail) => {
            return (
                None,
                Availability {
                    engine: "tesseract".to_string(),
                    available: false,
                    version: None,
                    detail,
                },
            );
        }
    };
    match detect_version(&bin) {
        Ok(version) => {
            let engine = TesseractEngine {
                bin: bin.clone(),
                version: version.clone(),
                helper: opts.limit_helper.clone(),
                timeout: opts.timeout,
                address_space_bytes: opts.address_space_bytes,
            };
            let report = Availability {
                engine: "tesseract".to_string(),
                available: true,
                version: Some(version.clone()),
                detail: format!(
                    "{} (tesseract {version}); resource limits {}",
                    bin.display(),
                    if opts.limit_helper.is_some() {
                        "on"
                    } else {
                        "off (no helper)"
                    }
                ),
            };
            (Some(engine), report)
        }
        Err(detail) => (
            None,
            Availability {
                engine: "tesseract".to_string(),
                available: false,
                version: None,
                detail: format!("{}: {detail}", bin.display()),
            },
        ),
    }
}

fn detect_version(bin: &Path) -> Result<String, String> {
    let dir = tempfile::tempdir().map_err(|e| format!("temp dir: {e}"))?;
    let mut cmd = Command::new(bin);
    cmd.arg("--version");
    let captured =
        run_captured(cmd, dir.path(), VERSION_TIMEOUT).map_err(|e| format!("--version: {e}"))?;
    if captured.timed_out {
        return Err("--version did not finish within 15 s".to_string());
    }
    // tesseract 4 prints the version on stderr, 5 on stdout.
    let text = String::from_utf8_lossy(&captured.stdout).to_string()
        + &String::from_utf8_lossy(&captured.stderr);
    parse_version(&text).ok_or_else(|| {
        format!(
            "--version printed no `tesseract <version>` line (exit {}): {}",
            captured.status_text,
            quote(&text)
        )
    })
}

/// The version token after `tesseract ` on the first matching line.
pub fn parse_version(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("tesseract "))
        .map(|v| v.trim().trim_start_matches('v').to_string())
        .filter(|v| !v.is_empty())
}

/// Accept `eng`, `chi_sim`, `eng+deu`, `script/Latin`: letters, digits, `_`, `/`,
/// joined by `+`. Rejects anything that could read as an option or a path trick.
pub fn validate_lang(lang: &str) -> Result<(), ImageTextError> {
    let ok = !lang.is_empty()
        && lang.len() <= 64
        && lang.split('+').all(|part| {
            let mut chars = part.chars();
            chars.next().is_some_and(|c| c.is_ascii_alphabetic())
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '/')
                && !part.contains("..")
        });
    if ok {
        Ok(())
    } else {
        Err(ImageTextError::InvalidInput(format!(
            "language `{lang}` is not a tesseract language code (expected e.g. eng, deu, chi_sim, eng+fra)"
        )))
    }
}

struct Captured {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    success: bool,
    status_text: String,
    timed_out: bool,
}

/// Run to completion or deadline, with stdout/stderr spooled to files in `dir`
/// (no pipes, so a chatty child cannot deadlock the controller).
fn run_captured(mut cmd: Command, dir: &Path, timeout: Duration) -> std::io::Result<Captured> {
    let out_path = dir.join("stdout");
    let err_path = dir.join("stderr");
    cmd.stdin(Stdio::null())
        .stdout(File::create(&out_path)?)
        .stderr(File::create(&err_path)?);
    let mut child = cmd.spawn()?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Ok(Captured {
        stdout: read_limited(&out_path)?,
        stderr: read_limited(&err_path)?,
        success: status.is_some_and(|s| s.success()),
        status_text: status.map_or_else(|| "killed".to_string(), |s| s.to_string()),
        timed_out: status.is_none(),
    })
}

fn read_limited(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    File::open(path)?
        .take(MAX_CAPTURE_BYTES)
        .read_to_end(&mut buf)?;
    Ok(buf)
}

fn quote(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.len() > MAX_QUOTED_STDERR {
        let mut end = MAX_QUOTED_STDERR;
        while !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &trimmed[..end])
    } else {
        trimmed.to_string()
    }
}

impl Engine for TesseractEngine {
    fn name(&self) -> &'static str {
        "tesseract"
    }

    fn version(&self) -> String {
        self.version.clone()
    }

    fn recognize(&self, image: &GrayImage, lang: &str) -> Result<Recognition, ImageTextError> {
        validate_lang(lang)?;
        let dir = tempfile::tempdir().map_err(|e| ImageTextError::Io("temp dir".to_string(), e))?;
        let input = dir.path().join("input.png");
        image
            .save(&input)
            .map_err(|e| ImageTextError::Engine(format!("writing temporary PNG: {e}")))?;
        let cpu_seconds = self.timeout.as_secs().max(1);
        let (mut cmd, limited) = limited_command(
            self.helper.as_deref(),
            &self.bin,
            cpu_seconds,
            self.address_space_bytes,
        );
        cmd.arg(&input)
            .arg("stdout")
            .arg("-l")
            .arg(lang)
            .arg("--psm")
            .arg("3")
            .arg("tsv")
            .env("OMP_NUM_THREADS", "1")
            .current_dir(dir.path());
        let captured = run_captured(cmd, dir.path(), self.timeout)
            .map_err(|e| ImageTextError::Engine(format!("starting {}: {e}", self.bin.display())))?;
        if captured.timed_out {
            return Err(ImageTextError::Timeout(
                self.timeout,
                "tesseract was killed".to_string(),
            ));
        }
        let stderr = String::from_utf8_lossy(&captured.stderr).to_string();
        if !captured.success {
            return Err(ImageTextError::Engine(format!(
                "tesseract exited {}: {}",
                captured.status_text,
                quote(&stderr)
            )));
        }
        let stdout = String::from_utf8_lossy(&captured.stdout);
        let mut recognition = parse_tsv(&stdout)?;
        recognition.warnings.extend(
            stderr
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with("Estimating resolution"))
                .map(|l| format!("tesseract: {l}")),
        );
        recognition.resource_limits_applied = Some(limited);
        Ok(recognition)
    }
}

/// (page, block, paragraph) key of a TSV paragraph.
type ParaKey = (u32, u32, u32);

/// One TSV word row.
struct Word {
    text: String,
    conf: f32,
    bbox: BBox,
}

/// Parse tesseract's TSV (level, page, block, par, line, word, left, top,
/// width, height, conf, text) into paragraph blocks with mean word confidence.
pub fn parse_tsv(tsv: &str) -> Result<Recognition, ImageTextError> {
    let mut lines = tsv.lines();
    let header = lines.next().unwrap_or_default();
    if !header.starts_with("level\t") {
        return Err(ImageTextError::Engine(format!(
            "unexpected tesseract output (no TSV header): {}",
            quote(tsv)
        )));
    }
    let mut order: Vec<ParaKey> = Vec::new();
    let mut paras: BTreeMap<ParaKey, BTreeMap<u32, Vec<Word>>> = BTreeMap::new();
    for line in lines {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 12 || fields[0] != "5" {
            continue;
        }
        let num = |i: usize| fields[i].trim().parse::<u32>().ok();
        let (Some(page), Some(block), Some(par), Some(line_no)) = (num(1), num(2), num(3), num(4))
        else {
            continue;
        };
        let (Some(left), Some(top), Some(width), Some(height)) = (num(6), num(7), num(8), num(9))
        else {
            continue;
        };
        let conf = fields[10].trim().parse::<f32>().unwrap_or(-1.0);
        let text = fields[11..].join("\t");
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let key = (page, block, par);
        if !paras.contains_key(&key) {
            order.push(key);
        }
        paras
            .entry(key)
            .or_default()
            .entry(line_no)
            .or_default()
            .push(Word {
                text: text.to_string(),
                conf,
                bbox: BBox {
                    x: left,
                    y: top,
                    width,
                    height,
                },
            });
    }
    let mut blocks = Vec::new();
    for key in order {
        let para = &paras[&key];
        let mut text_lines = Vec::new();
        let mut confs = Vec::new();
        let mut bbox: Option<BBox> = None;
        for words in para.values() {
            text_lines.push(
                words
                    .iter()
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            for w in words {
                if w.conf >= 0.0 {
                    confs.push(w.conf / 100.0);
                }
                bbox = Some(bbox.map_or(w.bbox, |b| b.union(w.bbox)));
            }
        }
        let confidence =
            (!confs.is_empty()).then(|| confs.iter().sum::<f32>() / confs.len() as f32);
        blocks.push(Block {
            text: text_lines.join("\n"),
            confidence,
            bbox,
        });
    }
    Ok(Recognition {
        blocks,
        ..Recognition::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
1\t1\t0\t0\t0\t0\t0\t0\t420\t90\t-1\t\n\
2\t1\t1\t0\t0\t0\t90\t30\t240\t30\t-1\t\n\
3\t1\t1\t1\t0\t0\t90\t30\t240\t30\t-1\t\n\
4\t1\t1\t1\t1\t0\t90\t30\t240\t30\t-1\t\n\
5\t1\t1\t1\t1\t1\t90\t30\t80\t30\t96.5\tHello\n\
5\t1\t1\t1\t1\t2\t180\t30\t70\t30\t91.5\tOCR\n\
5\t1\t1\t1\t1\t3\t260\t30\t70\t30\t88\t42\n\
4\t1\t1\t1\t2\t0\t90\t60\t100\t20\t-1\t\n\
5\t1\t1\t1\t2\t1\t90\t60\t100\t20\t70\tsecond\n\
5\t1\t2\t1\t1\t1\t10\t200\t50\t20\t60\tnext\n\
5\t1\t2\t1\t1\t2\t70\t200\t50\t20\t-1\t \n";

    #[test]
    fn tsv_groups_words_into_lines_and_paragraphs() {
        let rec = parse_tsv(SAMPLE).expect("parse");
        assert_eq!(rec.blocks.len(), 2);
        assert_eq!(rec.blocks[0].text, "Hello OCR 42\nsecond");
        let c = rec.blocks[0].confidence.expect("confidence");
        assert!((c - 0.865).abs() < 1e-4, "{c}");
        assert_eq!(
            rec.blocks[0].bbox,
            Some(BBox {
                x: 90,
                y: 30,
                width: 240,
                height: 50
            })
        );
        assert_eq!(rec.blocks[1].text, "next");
        assert_eq!(rec.text(), "Hello OCR 42\nsecond\n\nnext\n");
    }

    #[test]
    fn tsv_without_header_is_an_error() {
        let err = parse_tsv("Error opening data file\n").expect_err("no header");
        assert!(err.to_string().contains("no TSV header"), "{err}");
    }

    #[test]
    fn version_line_is_parsed_from_either_stream() {
        assert_eq!(
            parse_version("tesseract 5.3.4\n leptonica-1.83.1\n"),
            Some("5.3.4".to_string())
        );
        assert_eq!(
            parse_version("tesseract v4.1.1\n"),
            Some("4.1.1".to_string())
        );
        assert_eq!(parse_version("nothing here"), None);
    }

    #[test]
    fn language_codes_are_validated() {
        for ok in ["eng", "deu", "chi_sim", "eng+fra", "script/Latin", "osd"] {
            validate_lang(ok).unwrap_or_else(|e| panic!("{ok}: {e}"));
        }
        for bad in [
            "",
            "-l",
            "--psm",
            "en g",
            "../etc",
            "a".repeat(65).as_str(),
            "+eng",
            "eng+",
        ] {
            assert!(validate_lang(bad).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn missing_explicit_binary_is_reported_with_its_path() {
        let err = find_binary(Some(Path::new("/nonexistent/tess"))).expect_err("missing");
        assert!(err.contains("/nonexistent/tess"), "{err}");
    }
}
