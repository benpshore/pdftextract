//! Image to text: OCR of still images with one `<stem>.txt` and one
//! `<stem>.json` per input, by whichever engine is really available.
//!
//! * [`decode`]: PNG, JPEG, TIFF, BMP, WebP and GIF (first frame) by content
//!   sniffing; HEIC/HEIF refused with the reason.
//! * [`preprocess`]: grayscale, auto-contrast, polarity, 2x upscale for small
//!   text and projection-profile deskew, all pure Rust and all reported.
//! * [`engine`]: `tesseract` (external program under a timeout and kernel
//!   limits), `ocrs` (pure Rust, pinned models) and `docling` (detected only).
//! * [`process_file`] / [`collect_inputs`]: the batch driver used by the
//!   `tpe-image-text` binary. Inputs are never modified; outputs are never
//!   overwritten without `force`.

#![allow(
    clippy::must_use_candidate,
    clippy::module_name_repetitions,
    clippy::missing_errors_doc,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

pub mod decode;
pub mod engine;
pub mod limits;
pub mod preprocess;
mod publication;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use engine::{Availability, BBox, Block, Engine, EngineKind, EngineOptions, Recognition};
pub use preprocess::{PreprocessOptions, PreprocessReport};

/// Tool name written into every report.
pub const TOOL_NAME: &str = "tpe-image-text";
/// Tool version (the crate version; the release version is the git tag).
pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Everything that can go wrong for one input or for the run.
#[derive(Debug, thiserror::Error)]
pub enum ImageTextError {
    /// File system failure on the named path.
    #[error("{0}: {1}")]
    Io(String, std::io::Error),
    /// The bytes are a known format but could not be decoded.
    #[error("{0}: cannot decode: {1}")]
    Decode(String, String),
    /// Explicitly unsupported input or request.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// No engine can run; the message lists every probe.
    #[error("no OCR engine available: {0}")]
    NoEngine(String),
    /// The engine failed.
    #[error("engine error: {0}")]
    Engine(String),
    /// The engine exceeded its wall-clock limit.
    #[error("timed out after {0:?}: {1}")]
    Timeout(Duration, String),
    /// An output path exists and `force` was not given.
    #[error("output exists (use --force to overwrite): {0}")]
    OutputExists(String),
    /// Bad argument.
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

/// Batch settings.
#[derive(Clone, Debug)]
pub struct RunOptions {
    /// Directory receiving `<stem>.txt` and `<stem>.json`.
    pub out_dir: PathBuf,
    /// Overwrite existing outputs.
    pub force: bool,
    /// Language code passed to the engine.
    pub lang: String,
    /// Preprocessing steps.
    pub preprocess: PreprocessOptions,
}

/// Identity of the tool that wrote a report.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolInfo {
    /// Always `tpe-image-text`.
    pub name: String,
    /// Crate version.
    pub version: String,
}

/// Engine identity for a report.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EngineInfo {
    /// Engine name.
    pub name: String,
    /// Detected version.
    pub version: String,
    /// Language requested.
    pub lang: String,
    /// For subprocess engines: the kernel limits in force in the engine
    /// process (`core`, `cpu`, `fsize`, `as`); `None` for in-process engines.
    pub resource_limits_applied: Option<Vec<String>>,
}

/// Decoded image facts.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageInfo {
    /// Container format by content.
    pub format: decode::Format,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// Stage timings in milliseconds.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Timing {
    /// Reading and decoding the file.
    pub decode: u64,
    /// Preprocessing.
    pub preprocess: u64,
    /// Recognition.
    pub ocr: u64,
    /// Preparing the output report, measured before final publication.
    pub write: u64,
    /// Whole file.
    pub total: u64,
}

/// A block as written to JSON.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BlockReport {
    /// Zero-based position in reading order.
    pub index: usize,
    /// Text with lines joined by `\n`.
    pub text: String,
    /// Mean confidence `0..=1`, or null when the engine has none.
    pub confidence: Option<f32>,
    /// Box in preprocessed-image pixels, or null.
    pub bbox: Option<BBox>,
}

/// Paths written for one input.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Outputs {
    /// The `.txt` file.
    pub text: String,
    /// The `.json` file.
    pub json: String,
}

/// The `<stem>.json` document.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FileReport {
    /// Who wrote it.
    pub tool: ToolInfo,
    /// Input path as given.
    pub input: String,
    /// Input size.
    pub input_bytes: u64,
    /// Hex sha256 of the input bytes.
    pub input_sha256: String,
    /// Output paths.
    pub outputs: Outputs,
    /// Engine used.
    pub engine: EngineInfo,
    /// Decoded image.
    pub image: ImageInfo,
    /// What preprocessing did; block boxes refer to its output size.
    pub preprocessing: PreprocessReport,
    /// Coordinate space of block boxes.
    pub bbox_space: String,
    /// Blocks in reading order.
    pub blocks: Vec<BlockReport>,
    /// Characters in the text output.
    pub text_chars: usize,
    /// Timings.
    pub timing_ms: Timing,
    /// Everything non-fatal, from decoding through the engine.
    pub warnings: Vec<String>,
}

/// Outcome of one input in a batch.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FileOutcome {
    /// Input path.
    pub input: String,
    /// Whether both outputs were written.
    pub ok: bool,
    /// The `.txt` path when written.
    pub text_path: Option<String>,
    /// The `.json` path when written.
    pub json_path: Option<String>,
    /// Error text when not ok.
    pub error: Option<String>,
    /// Blocks recognised.
    pub blocks: usize,
    /// Characters written.
    pub text_chars: usize,
    /// Wall time in milliseconds.
    pub elapsed_ms: u64,
    /// Warnings from the report.
    pub warnings: Vec<String>,
}

/// Expand files and directories into the ordered list of inputs. Files given
/// explicitly are always included (their bytes decide the format); directory
/// entries are filtered by extension. Returns notes about skipped entries.
pub fn collect_inputs(
    inputs: &[PathBuf],
    recursive: bool,
) -> Result<(Vec<PathBuf>, Vec<String>), ImageTextError> {
    let mut files = Vec::new();
    let mut notes = Vec::new();
    for input in inputs {
        let meta =
            fs::metadata(input).map_err(|e| ImageTextError::Io(input.display().to_string(), e))?;
        if meta.is_dir() {
            walk(input, recursive, &mut files, &mut notes)?;
        } else {
            files.push(input.clone());
        }
    }
    Ok((files, notes))
}

fn walk(
    dir: &Path,
    recursive: bool,
    files: &mut Vec<PathBuf>,
    notes: &mut Vec<String>,
) -> Result<(), ImageTextError> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| ImageTextError::Io(dir.display().to_string(), e))?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()
        .map_err(|e| ImageTextError::Io(dir.display().to_string(), e))?;
    entries.sort();
    for entry in entries {
        let meta =
            fs::metadata(&entry).map_err(|e| ImageTextError::Io(entry.display().to_string(), e))?;
        if meta.is_dir() {
            if recursive {
                walk(&entry, recursive, files, notes)?;
            } else {
                notes.push(format!(
                    "skipped directory {} (no --recursive)",
                    entry.display()
                ));
            }
        } else if decode::is_candidate(&entry) {
            files.push(entry);
        }
    }
    Ok(())
}

/// Output paths for an input: `<out_dir>/<stem>.txt` and `.json`.
pub fn output_paths(input: &Path, out_dir: &Path) -> Result<(PathBuf, PathBuf), ImageTextError> {
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ImageTextError::InvalidInput(format!("{}: no usable file name", input.display()))
        })?;
    Ok((
        out_dir.join(format!("{stem}.txt")),
        out_dir.join(format!("{stem}.json")),
    ))
}

/// Decode, preprocess, recognise and write both outputs for one input.
pub fn process_file(
    input: &Path,
    engine: &dyn Engine,
    opts: &RunOptions,
) -> Result<FileReport, ImageTextError> {
    process_file_with_inputs(input, engine, opts, &[])
}

fn process_file_with_inputs(
    input: &Path,
    engine: &dyn Engine,
    opts: &RunOptions,
    inputs: &[PathBuf],
) -> Result<FileReport, ImageTextError> {
    let started = Instant::now();
    let (text_path, json_path) = output_paths(input, &opts.out_dir)?;
    let outputs =
        publication::OutputPair::new(input, [&text_path, &json_path], opts.force, inputs)?;
    let bytes = fs::read(input).map_err(|e| ImageTextError::Io(input.display().to_string(), e))?;
    let input_sha256 = hex::encode(Sha256::digest(&bytes));
    let decoded = decode::decode(input)?;
    let decode_ms = ms(started.elapsed());
    let mut warnings = decoded.warnings.clone();

    let t = Instant::now();
    let (gray, preprocessing) = preprocess::preprocess(&decoded.image, opts.preprocess);
    let preprocess_ms = ms(t.elapsed());

    let t = Instant::now();
    let recognition = engine.recognize(&gray, &opts.lang)?;
    let ocr_ms = ms(t.elapsed());
    warnings.extend(recognition.warnings.iter().cloned());
    if recognition.blocks.is_empty() {
        warnings.push("no text recognised".to_string());
    }

    let t = Instant::now();
    let text = recognition.text();
    fs::create_dir_all(&opts.out_dir)
        .map_err(|e| ImageTextError::Io(opts.out_dir.display().to_string(), e))?;
    let mut report = FileReport {
        tool: ToolInfo {
            name: TOOL_NAME.to_string(),
            version: TOOL_VERSION.to_string(),
        },
        input: input.display().to_string(),
        input_bytes: bytes.len() as u64,
        input_sha256,
        outputs: Outputs {
            text: text_path.display().to_string(),
            json: json_path.display().to_string(),
        },
        engine: EngineInfo {
            name: engine.name().to_string(),
            version: engine.version(),
            lang: opts.lang.clone(),
            resource_limits_applied: recognition.resource_limits_applied,
        },
        image: ImageInfo {
            format: decoded.format,
            width: decoded.image.width(),
            height: decoded.image.height(),
        },
        preprocessing,
        bbox_space: "preprocessed image pixels (see preprocessing.output_width/height)".to_string(),
        blocks: recognition
            .blocks
            .iter()
            .enumerate()
            .map(|(index, b)| BlockReport {
                index,
                text: b.text.clone(),
                confidence: b.confidence,
                bbox: b.bbox,
            })
            .collect(),
        text_chars: text.chars().count(),
        timing_ms: Timing {
            decode: decode_ms,
            preprocess: preprocess_ms,
            ocr: ocr_ms,
            write: 0,
            total: 0,
        },
        warnings,
    };
    report.timing_ms.write = ms(t.elapsed());
    report.timing_ms.total = ms(started.elapsed());
    let json = serde_json::to_vec_pretty(&report)
        .map_err(|e| ImageTextError::Engine(format!("serialising report: {e}")))?;
    outputs.publish([text.as_bytes(), &json])?;
    Ok(report)
}

/// Process every input with one engine, never stopping at a failure.
pub fn run_batch(files: &[PathBuf], engine: &dyn Engine, opts: &RunOptions) -> Vec<FileOutcome> {
    files
        .iter()
        .map(|input| {
            let started = Instant::now();
            match process_file_with_inputs(input, engine, opts, files) {
                Ok(report) => FileOutcome {
                    input: input.display().to_string(),
                    ok: true,
                    text_path: Some(report.outputs.text),
                    json_path: Some(report.outputs.json),
                    error: None,
                    blocks: report.blocks.len(),
                    text_chars: report.text_chars,
                    elapsed_ms: ms(started.elapsed()),
                    warnings: report.warnings,
                },
                Err(e) => FileOutcome {
                    input: input.display().to_string(),
                    ok: false,
                    text_path: None,
                    json_path: None,
                    error: Some(e.to_string()),
                    blocks: 0,
                    text_chars: 0,
                    elapsed_ms: ms(started.elapsed()),
                    warnings: Vec::new(),
                },
            }
        })
        .collect()
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_paths_use_the_stem() {
        let (t, j) = output_paths(Path::new("/in/scan 1.PNG"), Path::new("/out")).expect("paths");
        assert_eq!(t, PathBuf::from("/out/scan 1.txt"));
        assert_eq!(j, PathBuf::from("/out/scan 1.json"));
        assert!(output_paths(Path::new("/in/.."), Path::new("/out")).is_err());
    }

    #[test]
    fn collect_inputs_filters_directories_by_extension_and_keeps_explicit_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).expect("mkdir");
        for name in ["b.png", "a.JPG", "notes.txt", "sub/c.heic", "sub/d.pdf"] {
            fs::write(dir.path().join(name), b"x").expect("write");
        }
        let (files, notes) = collect_inputs(&[dir.path().to_path_buf()], false).expect("collect");
        let names: Vec<String> = files
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.JPG", "b.png"]);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("no --recursive"));
        let (files, notes) = collect_inputs(&[dir.path().to_path_buf()], true).expect("collect");
        let names: Vec<String> = files
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.JPG", "b.png", "c.heic"]);
        assert!(notes.is_empty());
        let explicit = dir.path().join("notes.txt");
        let (files, _) = collect_inputs(std::slice::from_ref(&explicit), false).expect("collect");
        assert_eq!(files, vec![explicit]);
        assert!(collect_inputs(&[dir.path().join("missing.png")], false).is_err());
    }
}
