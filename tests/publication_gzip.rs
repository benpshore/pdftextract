//! `--gzip`: published outputs are RFC 1952 gzip members under `<name>.gz`,
//! staged, synced, linked and rolled back exactly like uncompressed ones. The
//! ledger is never compressed. Tests that need the system `gzip` skip with a
//! message when it is absent.

mod common;

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use flate2::read::GzDecoder;
use tpe::publication::{Compression, GzipLevel, StagedOutputs};

use common::{TITLE, synthetic_paper, write_temp_pdf};

/// ID1, ID2 and the deflate method byte that open every gzip member.
const GZIP_MAGIC: [u8; 3] = [0x1f, 0x8b, 0x08];

fn gunzip(path: &Path) -> Vec<u8> {
    let mut decoded = Vec::new();
    GzDecoder::new(fs::File::open(path).unwrap())
        .read_to_end(&mut decoded)
        .unwrap();
    decoded
}

/// Sorted file names in `dir`, including staging leftovers.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// `false` (after printing why) when the system gzip cannot run here.
fn system_gzip_available() -> bool {
    match Command::new("gzip").arg("--version").output() {
        Ok(output) if output.status.success() => true,
        _ => {
            eprintln!("skipping: the system gzip is not available on this machine");
            false
        }
    }
}

fn pair(source: &Path, compression: Compression) -> StagedOutputs {
    StagedOutputs::stage_with(
        source,
        &[
            (".json", b"{\"pages\":2}\n".to_vec()),
            (".txt", "page one\u{c}page two\n".as_bytes().to_vec()),
        ],
        compression,
    )
    .unwrap()
}

#[test]
fn gzip_levels_are_validated_and_default_to_six() {
    assert!(Compression::gzip(0).is_err());
    assert!(Compression::gzip(10).is_err());
    assert!(GzipLevel::new(255).is_err());
    for level in 1..=9 {
        let compression = Compression::gzip(level).unwrap();
        assert_eq!(compression.gzip_level().unwrap().get(), level);
        assert_eq!(compression.suffix(), ".gz");
    }
    assert_eq!(GzipLevel::default().get(), 6);
    assert_eq!(Compression::default(), Compression::None);
    assert_eq!(Compression::None.suffix(), "");
    assert_eq!(Compression::None.gzip_level(), None);
}

#[test]
fn gzip_outputs_publish_under_gz_names_and_decode_to_the_originals() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("paper.pdf");
    let mut staged = pair(&source, Compression::gzip(6).unwrap());
    let paths = staged.publish_then(|| Ok(())).unwrap();
    assert_eq!(
        paths,
        [
            dir.path().join("paper.json.gz"),
            dir.path().join("paper.txt.gz")
        ]
    );
    for (path, original) in paths.iter().zip([
        b"{\"pages\":2}\n".as_slice(),
        "page one\u{c}page two\n".as_bytes(),
    ]) {
        assert!(fs::read(path).unwrap().starts_with(&GZIP_MAGIC), "{path:?}");
        assert_eq!(gunzip(path), original, "{path:?}");
    }
    drop(staged);
    assert_eq!(names(dir.path()), ["paper.json.gz", "paper.txt.gz"]);
}

#[test]
fn uncompressed_staging_is_unchanged_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("paper.pdf");
    let mut staged = StagedOutputs::stage(&source, &[(".txt", b"plain".to_vec())]).unwrap();
    let paths = staged.publish_then(|| Ok(())).unwrap();
    assert_eq!(paths, [dir.path().join("paper.txt")]);
    assert_eq!(fs::read(&paths[0]).unwrap(), b"plain");
    let mut explicit = StagedOutputs::stage_with(
        &dir.path().join("other.pdf"),
        &[(".txt", b"plain".to_vec())],
        Compression::None,
    )
    .unwrap();
    assert_eq!(
        explicit.publish_then(|| Ok(())).unwrap(),
        [dir.path().join("other.txt")]
    );
}

#[test]
fn gzip_outputs_never_clobber_and_take_the_next_generation() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("paper.pdf");
    let taken = dir.path().join("paper.txt.gz");
    fs::write(&taken, b"other owner").unwrap();
    let mut staged = pair(&source, Compression::gzip(9).unwrap());
    let paths = staged.publish_then(|| Ok(())).unwrap();
    assert_eq!(
        paths,
        [
            dir.path().join("paper 2.json.gz"),
            dir.path().join("paper 2.txt.gz")
        ]
    );
    assert_eq!(fs::read(&taken).unwrap(), b"other owner");
    assert!(!dir.path().join("paper.json.gz").exists());
    assert_eq!(gunzip(&paths[0]), b"{\"pages\":2}\n");
    assert_eq!(gunzip(&paths[1]), "page one\u{c}page two\n".as_bytes());
}

#[test]
fn failed_commit_removes_gzip_outputs_and_staging_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut staged = pair(&dir.path().join("paper.pdf"), Compression::gzip(1).unwrap());
    let result = staged.publish_then(|| Err("injected commit failure".into()));
    assert_eq!(result.unwrap_err(), "injected commit failure");
    drop(staged);
    assert!(names(dir.path()).is_empty());
}

/// Hands out `remaining` bytes of a repeating pattern in reads of at most
/// 4 KiB, recording the largest buffer it was ever asked to fill.
struct Chunked {
    remaining: usize,
    offset: usize,
    largest_request: usize,
}

impl Read for Chunked {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.largest_request = self.largest_request.max(buf.len());
        let count = buf.len().min(4096).min(self.remaining);
        for (index, byte) in buf[..count].iter_mut().enumerate() {
            *byte = b"the quick brown fox jumps over the lazy dog\n"[(self.offset + index) % 44];
        }
        self.offset += count;
        self.remaining -= count;
        Ok(count)
    }
}

#[test]
fn large_outputs_stream_through_the_encoder_in_bounded_reads() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("paper.pdf");
    let total = 6 * 1024 * 1024;
    let mut reader = Chunked {
        remaining: total,
        offset: 0,
        largest_request: 0,
    };
    let mut streams: Vec<(&str, &mut dyn Read)> = vec![(".txt", &mut reader)];
    let mut staged =
        StagedOutputs::stage_streams(&source, &mut streams, Compression::gzip(1).unwrap()).unwrap();
    let paths = staged.publish_then(|| Ok(())).unwrap();
    assert_eq!(paths, [dir.path().join("paper.txt.gz")]);
    assert_eq!(reader.remaining, 0);
    assert!(
        reader.largest_request <= 1024 * 1024,
        "a whole-output read request was made: {}",
        reader.largest_request
    );
    let compressed = fs::metadata(&paths[0]).unwrap().len();
    assert!(compressed < (total / 10) as u64, "{compressed} bytes");
    let decoded = gunzip(&paths[0]);
    assert_eq!(decoded.len(), total);
    assert!(decoded.starts_with(b"the quick brown fox"));
}

#[test]
fn system_gzip_verifies_and_decompresses_the_outputs() {
    if !system_gzip_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut staged = pair(&dir.path().join("paper.pdf"), Compression::gzip(9).unwrap());
    let paths = staged.publish_then(|| Ok(())).unwrap();
    for (path, original) in paths.iter().zip([
        b"{\"pages\":2}\n".as_slice(),
        "page one\u{c}page two\n".as_bytes(),
    ]) {
        let check = Command::new("gunzip").arg("-t").arg(path).output().unwrap();
        assert!(
            check.status.success(),
            "gunzip -t {path:?}: {}",
            String::from_utf8_lossy(&check.stderr)
        );
        let decoded = Command::new("gzip").arg("-dc").arg(path).output().unwrap();
        assert!(
            decoded.status.success(),
            "{}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        assert_eq!(decoded.stdout, original, "{path:?}");
    }
}

fn tpe() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tpe"))
}

/// One `extract --json` run; returns the parsed record and its output paths.
fn extract_json(command: &mut Command) -> (serde_json::Value, Vec<PathBuf>) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "{stdout}");
    let value: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(value["status"], "complete", "{value}");
    let paths: Vec<PathBuf> = value["output_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| PathBuf::from(path.as_str().unwrap()))
        .collect();
    (value, paths)
}

#[test]
fn extract_cli_gzip_publishes_gz_outputs_and_keeps_the_ledger_plain() {
    let (dir, pdf) = write_temp_pdf(&synthetic_paper());
    let db = dir.path().join("ledger.sqlite");
    let out = dir.path().join("out");
    let (value, paths) = extract_json(
        tpe()
            .args(["extract", "--json", "--gzip", "--db"])
            .arg(&db)
            .arg("--out")
            .arg(&out)
            .arg(&pdf),
    );
    let hash = value["document"]["hash"].as_str().unwrap();
    assert_eq!(hash.len(), 64, "{value}");
    assert_eq!(
        paths,
        [
            out.join(format!("{hash}.json.gz")),
            out.join(format!("{hash}.txt.gz"))
        ]
    );
    assert_eq!(
        names(&out),
        [format!("{hash}.json.gz"), format!("{hash}.txt.gz")],
        "only the two compressed outputs, no plain or staging files"
    );
    for path in &paths {
        assert!(fs::read(path).unwrap().starts_with(&GZIP_MAGIC), "{path:?}");
    }
    let json: serde_json::Value = serde_json::from_slice(&gunzip(&paths[0])).unwrap();
    assert_eq!(json["document"]["hash"], hash);
    assert_eq!(json["document"]["pages"], 2);
    let text = String::from_utf8(gunzip(&paths[1])).unwrap();
    assert!(text.contains(TITLE), "{text}");
    assert_eq!(text.matches('\u{c}').count(), 1, "two pages, one separator");

    let ledger = fs::read(&db).unwrap();
    assert!(
        ledger.starts_with(b"SQLite format 3\0"),
        "ledger must stay plain"
    );
    let stats = tpe().args(["stats", "--db"]).arg(&db).output().unwrap();
    assert!(stats.status.success());
    let stats = String::from_utf8(stats.stdout).unwrap();
    assert!(stats.contains("runs: 1\n"), "{stats}");
    assert!(stats.contains("complete: 1\n"), "{stats}");

    // A rerun keeps the published files and chooses the next generation.
    let (_, rerun) = extract_json(
        tpe()
            .args(["extract", "--json", "--gzip", "--gzip-level", "1", "--db"])
            .arg(&db)
            .arg("--out")
            .arg(&out)
            .arg(&pdf),
    );
    assert_eq!(
        rerun,
        [
            out.join(format!("{hash} 2.json.gz")),
            out.join(format!("{hash} 2.txt.gz"))
        ]
    );
    assert_eq!(gunzip(&rerun[1]).len(), text.len());

    if system_gzip_available() {
        for path in paths.iter().chain(&rerun) {
            let check = Command::new("gunzip").arg("-t").arg(path).output().unwrap();
            assert!(check.status.success(), "gunzip -t {path:?}");
        }
        let decoded = Command::new("gzip")
            .arg("-dc")
            .arg(&paths[1])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8(decoded.stdout).unwrap(), text);
    }
}

#[test]
fn extract_cli_flag_decides_and_the_environment_only_relays() {
    let (dir, pdf) = write_temp_pdf(&synthetic_paper());
    let db = dir.path().join("ledger.sqlite");
    let out = dir.path().join("out");
    // No --gzip: an inherited relay variable must not compress anything.
    let (value, paths) = extract_json(
        tpe()
            .env("TPE_GZIP_LEVEL", "9")
            .args(["extract", "--json", "--db"])
            .arg(&db)
            .arg("--out")
            .arg(&out)
            .arg(&pdf),
    );
    let hash = value["document"]["hash"].as_str().unwrap();
    assert_eq!(
        paths,
        [
            out.join(format!("{hash}.json")),
            out.join(format!("{hash}.txt"))
        ]
    );
    for path in &paths {
        assert!(
            !fs::read(path).unwrap().starts_with(&GZIP_MAGIC),
            "{path:?}"
        );
    }
    // --gzip-level overrides a stale relay value.
    let (_, paths) = extract_json(
        tpe()
            .env("TPE_GZIP_LEVEL", "9")
            .args(["extract", "--json", "--gzip", "--gzip-level", "2", "--db"])
            .arg(&db)
            .arg("--out")
            .arg(&out)
            .arg(&pdf),
    );
    assert_eq!(
        paths,
        [
            out.join(format!("{hash}.json.gz")),
            out.join(format!("{hash}.txt.gz"))
        ]
    );
    assert!(fs::read(&paths[0]).unwrap().starts_with(&GZIP_MAGIC));
}

#[test]
fn extract_cli_rejects_misused_gzip_flags() {
    let (dir, pdf) = write_temp_pdf(&synthetic_paper());
    let db = dir.path().join("ledger.sqlite");
    let out = dir.path().join("out");
    for (args, expected) in [
        (vec!["--gzip-level", "9"], "--gzip"),
        (vec!["--gzip"], "--out"),
        (vec!["--gzip", "--gzip-level", "0", "--out", "out"], "0"),
        (vec!["--gzip", "--gzip-level", "10", "--out", "out"], "10"),
        (
            vec!["--gzip", "--gzip-level", "fast", "--out", "out"],
            "fast",
        ),
    ] {
        let output = tpe()
            .current_dir(dir.path())
            .args(["extract", "--db"])
            .arg(&db)
            .args(&args)
            .arg(&pdf)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{args:?}: {stderr}");
        assert!(!out.exists(), "{args:?} must not publish");
        assert!(!db.exists(), "{args:?} must not create a ledger");
    }
}

#[test]
fn extract_help_documents_the_gzip_options() {
    let output = tpe().args(["extract", "--help"]).output().unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--gzip"), "{help}");
    assert!(help.contains("--gzip-level <1-9>"), "{help}");
    assert!(help.contains("[default: 6]"), "{help}");
}
