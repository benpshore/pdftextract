//! Offline source-declared Unicode selection over immutable extraction artifacts.
//! Never invokes a backend or rewrites a production extraction record.

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use tpe_region_evidence::{
    ArtifactStore, Attempt, CONTRACT_VERSION, Decision, EvidenceKind, GeometryFrame, Limits,
    Outcome, RegionEvidence, RuntimeIdentity, Sidecar, SourceIdentity,
};
use tpe_source_fusion::{Limits as FusionLimits, select};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const MAX_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;

const USAGE: &str = "usage: fuse_source --source source.pdf --baseline lopdf.json \
--candidate candidate.json --output derived.json \
[--max-source-bytes N]";

struct Arguments {
    source: PathBuf,
    baseline: PathBuf,
    candidate: PathBuf,
    output: PathBuf,
    max_source_bytes: u64,
}

impl Arguments {
    fn parse() -> Result<Self> {
        Self::parse_from(env::args_os().skip(1))
    }

    fn parse_from(mut args: impl Iterator<Item = OsString>) -> Result<Self> {
        let mut values = BTreeMap::<String, OsString>::new();
        while let Some(flag) = args.next() {
            let flag = flag.to_str().ok_or("argument name must be UTF-8")?;
            if flag == "--help" || flag == "-h" {
                println!("{USAGE}");
                std::process::exit(0);
            }
            if !matches!(
                flag,
                "--source" | "--baseline" | "--candidate" | "--output" | "--max-source-bytes"
            ) {
                return Err(format!("unknown argument {flag}; {USAGE}").into());
            }
            let value = args
                .next()
                .ok_or_else(|| format!("missing value for {flag}"))?;
            if values.insert(flag.into(), value).is_some() {
                return Err(format!("duplicate argument {flag}").into());
            }
        }
        let source = required(&mut values, "--source")?.into();
        let baseline = required(&mut values, "--baseline")?.into();
        let candidate = required(&mut values, "--candidate")?.into();
        let output = required(&mut values, "--output")?.into();
        let max_source_bytes = values
            .remove("--max-source-bytes")
            .map(|value| {
                value
                    .to_str()
                    .ok_or("max-source-bytes must be UTF-8")?
                    .parse::<u64>()
                    .map_err(|_| "max-source-bytes must be an unsigned integer")
            })
            .transpose()?
            .unwrap_or(u64::MAX);
        Ok(Self {
            source,
            baseline,
            candidate,
            output,
            max_source_bytes,
        })
    }
}

fn required(values: &mut BTreeMap<String, OsString>, key: &str) -> Result<OsString> {
    values
        .remove(key)
        .ok_or_else(|| format!("missing {key}; {USAGE}").into())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("fuse_source: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Arguments::parse()?;
    // Reject existing files, directories and dangling symlinks before doing work.
    // The atomic final publication also checks this condition, preventing races.
    match fs::symlink_metadata(&args.output) {
        Ok(_) => return Err("output already exists".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let limits = Limits {
        max_source_bytes: args.max_source_bytes,
        ..Limits::default()
    };
    let mut fusion_limits = FusionLimits::default();
    fusion_limits.source.max_source_bytes = args.max_source_bytes;
    let source_bytes = read_bounded(&args.source, limits.max_source_bytes, "source")?;
    let source = SourceIdentity::of(&source_bytes);

    let mut store = ArtifactStore::default();
    let mut regions = Vec::new();
    let (baseline, document_pages) = add_attempt(
        "baseline",
        read_bounded(
            &args.baseline,
            limits.max_artifact_bytes,
            "baseline artifact",
        )?,
        &source,
        &mut store,
        &mut regions,
        &limits,
    )?;
    let (candidate, _) = add_attempt(
        "candidate",
        read_bounded(
            &args.candidate,
            limits.max_artifact_bytes,
            "candidate artifact",
        )?,
        &source,
        &mut store,
        &mut regions,
        &limits,
    )?;

    let sidecar = Sidecar {
        contract_version: CONTRACT_VERSION,
        source,
        document_pages,
        pages: baseline.pages.clone(),
        baseline,
        alternatives: vec![candidate],
        regions,
        decision: Decision::RetainBaselineAndAbstain,
    };
    let mut sidecar_json = LimitedWriter {
        inner: Vec::new(),
        remaining: limits.max_sidecar_bytes,
    };
    serde_json::to_writer(&mut sidecar_json, &sidecar)?;
    let sidecar = Sidecar::from_json(&sidecar_json.inner, &limits)?;
    let validated = sidecar.validate(&source_bytes, &store, &limits)?;
    // The runner enforces process deadlines/cancellation. This offline call has
    // no background scheduler or native workers; library callers can also pass
    // a live cancellation token directly.
    let derived = select(
        &validated,
        &source_bytes,
        &fusion_limits,
        &AtomicBool::new(false),
    )?;
    write_new_json(&args.output, &derived, MAX_OUTPUT_BYTES)?;
    println!(
        "Wrote source-declared Unicode projection to {}",
        args.output.display()
    );
    Ok(())
}

/// Check file length before allocating and cap the reader in case the file grows.
/// Retain the exact bytes: no normalization or parse/serialize round trip.
fn read_bounded(path: &Path, limit: u64, label: &str) -> Result<Vec<u8>> {
    // Reject ordinary FIFO/device inputs before opening them can block. The
    // opened descriptor is checked again; the runner supplies private paths.
    let path_metadata = fs::metadata(path)?;
    if !path_metadata.is_file() {
        return Err(format!("{label} must be a regular file").into());
    }
    if path_metadata.len() > limit {
        return Err(format!("{label} byte budget exceeded").into());
    }
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(format!("{label} must be a regular file").into());
    }
    if metadata.len() > limit {
        return Err(format!("{label} byte budget exceeded").into());
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(format!("{label} byte budget exceeded").into());
    }
    if bytes.len() as u64 != metadata.len() {
        return Err(format!("{label} changed length during reading").into());
    }
    Ok(bytes)
}

/// This only constructs locators. `Sidecar::validate` checks duplicate JSON
/// members and the full contract before the selection library sees any evidence.
fn add_attempt(
    id: &str,
    bytes: Vec<u8>,
    source: &SourceIdentity,
    store: &mut ArtifactStore,
    regions: &mut Vec<RegionEvidence>,
    limits: &Limits,
) -> Result<(Attempt, u32)> {
    let record: Value = serde_json::from_slice(&bytes)?;
    let artifact = store.insert(bytes, limits)?;
    let document_pages = serde_json::from_value(record["document"]["pages"].clone())?;
    let outcome = match record["status"].as_str() {
        Some("complete") => Outcome::Complete,
        Some("partial") => Outcome::Partial {
            reason: "artifact declares partial extraction".into(),
        },
        Some("failed") => Outcome::Failed {
            reason: "artifact declares failed extraction".into(),
        },
        _ => return Err("artifact must declare complete, partial or failed status".into()),
    };
    let pages = record["pages"].as_array().ok_or("missing artifact pages")?;
    let mut selected_pages = Vec::new();
    for (page_index, page) in pages.iter().enumerate() {
        let page_number = serde_json::from_value(page["page"].clone())?;
        selected_pages.push(page_number);
        for array in ["spans", "lines", "links", "figures"] {
            for (index, _) in page[array]
                .as_array()
                .ok_or("missing artifact evidence array")?
                .iter()
                .enumerate()
            {
                if regions.len() >= limits.max_regions {
                    return Err("region budget exceeded".into());
                }
                let index = u32::try_from(index)?;
                let evidence = match array {
                    "spans" => EvidenceKind::Text { span_index: index },
                    "lines" => EvidenceKind::Layout { line_index: index },
                    "links" => EvidenceKind::Uri { link_index: index },
                    "figures" => EvidenceKind::Figure {
                        figure_index: index,
                    },
                    _ => unreachable!(),
                };
                regions.push(RegionEvidence {
                    id: format!("{id}/{page_index}/{array}/{index}"),
                    attempt_id: id.into(),
                    artifact: artifact.clone(),
                    page_index: u32::try_from(page_index)?,
                    page: page_number,
                    evidence,
                });
            }
        }
    }
    let backend =
        serde_json::from_value::<tpe_region_evidence::BackendIdentity>(record["backend"].clone())?;
    // These exact schema-5 adapters emit PDF user-space boxes: see
    // src/backend/pdfium_backend.rs (bbox/unrotated_size), lopdf_backend and
    // src/regions.rs. This is a producer declaration, not runtime attestation.
    // Unknown adapters/versions have no established coordinate convention here.
    let frame = match (backend.name.as_str(), backend.version.as_str()) {
        ("lopdf", "0.45.0") | ("pdfium", "dynamic-binding-0.8.37") => {
            GeometryFrame::ProducerDeclaredPdfUserSpaceUnrotated
        }
        _ => GeometryFrame::Unknown,
    };
    Ok((
        Attempt {
            id: id.into(),
            source: source.clone(),
            pages: selected_pages,
            backend,
            runtime: RuntimeIdentity::Unknown {
                reason: "this offline CLI did not inspect the extraction runtime binary".into(),
            },
            outcome,
            artifact: Some(artifact),
            frame,
        },
        document_pages,
    ))
}

struct LimitedWriter<W> {
    inner: W,
    remaining: u64,
}

impl<W: std::io::Write> std::io::Write for LimitedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() as u64 > self.remaining {
            return Err(std::io::Error::other("output byte budget exceeded"));
        }
        let written = self.inner.write(bytes)?;
        self.remaining -= written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

struct TemporaryFile(PathBuf);

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Link the fully written file into place: unlike rename, hard_link cannot
/// replace an existing destination. Both paths are on the same filesystem.
fn write_new_json(path: &Path, value: &impl Serialize, limit: u64) -> Result<()> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temp =
        TemporaryFile(parent.join(format!(".source-fusion-{}-{nonce}.tmp", std::process::id())));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp.0)?;
    let mut writer = LimitedWriter {
        inner: file,
        remaining: limit,
    };
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.inner.sync_all()?;
    fs::hard_link(&temp.0, path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let nonce = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "source-fusion-cli-test-{}-{}-{nonce}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system clock")
                    .as_nanos()
            ));
            fs::create_dir(&path).expect("create private test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn only_source_and_extraction_inputs_are_accepted() {
        let flags = [
            "--source",
            "source.pdf",
            "--baseline",
            "baseline.json",
            "--candidate",
            "candidate.json",
            "--output",
            "derived.json",
        ];
        let args = Arguments::parse_from(flags.into_iter().map(OsString::from))
            .expect("source-only arguments");
        assert_eq!(args.max_source_bytes, u64::MAX);
        for forbidden in ["--review", "--truth", "--render", "--trusted-review-sha256"] {
            let error = Arguments::parse_from(
                flags
                    .into_iter()
                    .chain([forbidden, "external-input"])
                    .map(OsString::from),
            )
            .err()
            .expect("external proof input rejected");
            assert!(error.to_string().contains("unknown argument"));
        }
    }

    #[test]
    fn bounded_reader_retains_exact_bytes_and_rejects_oversize() {
        let directory = TestDirectory::new();
        let path = directory.0.join("input.json");
        let exact = b"{ \"not_normalized\": true }\r\n";
        fs::write(&path, exact).expect("write input");
        assert_eq!(
            read_bounded(&path, exact.len() as u64, "artifact").expect("bounded read"),
            exact
        );
        assert!(
            read_bounded(&path, exact.len() as u64 - 1, "artifact")
                .expect_err("oversized input rejected")
                .to_string()
                .contains("artifact byte budget exceeded")
        );
    }

    #[test]
    fn bounded_reader_rejects_nonregular_input() {
        let directory = TestDirectory::new();
        assert!(
            read_bounded(&directory.0, u64::MAX, "source")
                .expect_err("directory rejected")
                .to_string()
                .contains("regular file")
        );
    }

    #[test]
    fn atomic_publication_does_not_replace_existing_output() {
        let directory = TestDirectory::new();
        let path = directory.0.join("derived.json");
        fs::write(&path, b"existing artifact").expect("write original");
        assert!(write_new_json(&path, &serde_json::json!({ "changed": true }), 1024).is_err());
        assert_eq!(
            fs::read(&path).expect("read original"),
            b"existing artifact"
        );
        assert_eq!(
            fs::read_dir(&directory.0).expect("list directory").count(),
            1
        );
    }

    #[test]
    fn publication_budget_failure_removes_temporary_output() {
        let directory = TestDirectory::new();
        let path = directory.0.join("derived.json");
        assert!(write_new_json(&path, &serde_json::json!({ "text": "too much" }), 4).is_err());
        assert!(!path.exists());
        assert_eq!(
            fs::read_dir(&directory.0).expect("list directory").count(),
            0
        );
    }

    #[test]
    fn successful_publication_contains_complete_json() {
        let directory = TestDirectory::new();
        let path = directory.0.join("derived.json");
        let value = serde_json::json!({ "pages": [{ "text": "selected text" }] });
        write_new_json(&path, &value, 1024).expect("publish complete JSON");
        let actual: Value = serde_json::from_slice(&fs::read(&path).expect("read output"))
            .expect("parse published JSON");
        assert_eq!(actual, value);
        assert_eq!(
            fs::read_dir(&directory.0).expect("list directory").count(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn atomic_publication_does_not_follow_output_symlink() {
        let directory = TestDirectory::new();
        let target = directory.0.join("target.json");
        let path = directory.0.join("derived.json");
        fs::write(&target, b"protected artifact").expect("write target");
        std::os::unix::fs::symlink(&target, &path).expect("create symlink");
        assert!(write_new_json(&path, &serde_json::json!({ "changed": true }), 1024).is_err());
        assert_eq!(
            fs::read(&target).expect("read target"),
            b"protected artifact"
        );
        assert!(
            fs::symlink_metadata(&path)
                .expect("symlink metadata")
                .is_symlink()
        );
    }
}
