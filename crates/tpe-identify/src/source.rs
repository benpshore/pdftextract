//! Loading inputs: PDFs (hashed with the engine's `acquire` snapshot) and
//! engine result JSON files, plus the three ways to obtain text for a PDF
//! without re-extracting it (a results directory, a ledger) and the
//! fallback extraction with the pure-Rust `lopdf` backend.

use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use thiserror::Error;
use tpe::acquire::{self, AcquireError};
use tpe::ledger::{Ledger, LedgerError};
use tpe::pipeline::{self, PipelineError};
use tpe::schema::{ExtractionResult, Job};

/// One thing to identify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputSpec {
    /// A PDF file.
    Pdf(PathBuf),
    /// An engine result (`ExtractionResult` JSON); the PDF is located from
    /// its recorded sources when one still exists.
    Result(PathBuf),
}

impl InputSpec {
    /// The file this input was named by.
    pub fn path(&self) -> &Path {
        match self {
            Self::Pdf(p) | Self::Result(p) => p,
        }
    }
}

/// How to find text for a PDF.
#[derive(Clone, Debug, Default)]
pub struct LoadOptions {
    /// Directory of `tpe extract --out` outputs (`<hash>.json` or
    /// `<hash>.pdf.json`).
    pub results_dir: Option<PathBuf>,
    /// A `tpe` ledger; the latest run for the hash is used.
    pub ledger: Option<PathBuf>,
    /// Extract with the `lopdf` backend when nothing stored is found.
    pub extract: bool,
    /// Largest PDF accepted, in bytes.
    pub max_bytes: Option<u64>,
}

/// Why an input could not be loaded.
#[derive(Debug, Error)]
pub enum LoadError {
    #[error("read: {0}")]
    Acquire(#[from] AcquireError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("result json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("ledger: {0}")]
    Ledger(#[from] LedgerError),
    #[error("extraction: {0}")]
    Pipeline(#[from] PipelineError),
    #[error("extraction panicked: {0}")]
    Panicked(String),
    #[error("not a PDF: missing %PDF header")]
    NotPdf,
}

/// A loaded input: its bytes' identity and, when found, the engine's result.
#[derive(Debug)]
pub struct Loaded {
    pub path: Option<PathBuf>,
    pub size: u64,
    pub sha256: String,
    pub result: Option<ExtractionResult>,
    pub text_source: String,
    pub warnings: Vec<String>,
}

/// Expand the command-line paths: directories are walked for `*.pdf`
/// (case-insensitive), `*.json` files are engine results, everything else
/// is a PDF. Missing paths are reported, not fatal. The order is the
/// command-line order, with directory contents sorted.
pub fn collect(paths: &[PathBuf]) -> (Vec<InputSpec>, Vec<(PathBuf, String)>) {
    let mut specs = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        match fs::metadata(path) {
            Ok(meta) if meta.is_dir() => walk(path, &mut specs, &mut errors),
            Ok(_) => specs.push(spec_for(path)),
            Err(e) => errors.push((path.clone(), format!("io: {e}"))),
        }
    }
    (specs, errors)
}

fn spec_for(path: &Path) -> InputSpec {
    let is_json = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    if is_json {
        InputSpec::Result(path.to_path_buf())
    } else {
        InputSpec::Pdf(path.to_path_buf())
    }
}

fn walk(dir: &Path, specs: &mut Vec<InputSpec>, errors: &mut Vec<(PathBuf, String)>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            errors.push((dir.to_path_buf(), format!("io: {e}")));
            return;
        }
    };
    let mut children: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    children.sort();
    for child in children {
        if child.is_dir() {
            walk(&child, specs, errors);
        } else if child
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
        {
            specs.push(InputSpec::Pdf(child));
        }
    }
}

/// Load one input.
pub fn load(spec: &InputSpec, options: &LoadOptions) -> Result<Loaded, LoadError> {
    match spec {
        InputSpec::Result(path) => load_result_file(path),
        InputSpec::Pdf(path) => load_pdf(path, options),
    }
}

fn load_result_file(path: &Path) -> Result<Loaded, LoadError> {
    let bytes = fs::read(path)?;
    let result: ExtractionResult = serde_json::from_slice(&bytes)?;
    let mut warnings = Vec::new();
    let pdf = result
        .document
        .sources
        .iter()
        .map(|s| PathBuf::from(&s.path))
        .find(|p| p.is_file());
    if pdf.is_none() {
        warnings.push(
            "no recorded source path exists on disk; the input can be grouped but not renamed"
                .to_string(),
        );
    }
    Ok(Loaded {
        path: pdf,
        size: result.document.size,
        sha256: result.document.hash.0.clone(),
        text_source: format!("json:{}", path.display()),
        result: Some(result),
        warnings,
    })
}

fn load_pdf(path: &Path, options: &LoadOptions) -> Result<Loaded, LoadError> {
    let snapshot = acquire::snapshot(path, options.max_bytes)?;
    if !snapshot.bytes.starts_with(b"%PDF")
        && !snapshot.bytes[..snapshot.bytes.len().min(1024)]
            .windows(4)
            .any(|w| w == b"%PDF")
    {
        return Err(LoadError::NotPdf);
    }
    let sha256 = snapshot.hash.0.clone();
    let size = snapshot.source.size;
    drop(snapshot);
    let mut warnings = Vec::new();

    if let Some(dir) = &options.results_dir {
        for candidate in [
            dir.join(format!("{sha256}.json")),
            dir.join(format!("{sha256}.pdf.json")),
        ] {
            if candidate.is_file() {
                match fs::read(&candidate)
                    .map_err(LoadError::from)
                    .and_then(|b| Ok(serde_json::from_slice::<ExtractionResult>(&b)?))
                {
                    Ok(result) if result.document.hash.0 == sha256 => {
                        return Ok(Loaded {
                            path: Some(path.to_path_buf()),
                            size,
                            sha256,
                            result: Some(result),
                            text_source: format!("results:{}", candidate.display()),
                            warnings,
                        });
                    }
                    Ok(_) => warnings.push(format!(
                        "{}: document hash differs; ignored",
                        candidate.display()
                    )),
                    Err(e) => warnings.push(format!("{}: {e}; ignored", candidate.display())),
                }
            }
        }
    }

    if let Some(db) = &options.ledger {
        if db.is_file() {
            match run_from_ledger(db, &sha256) {
                Ok(Some((run, result))) => {
                    return Ok(Loaded {
                        path: Some(path.to_path_buf()),
                        size,
                        sha256,
                        result: Some(result),
                        text_source: format!("ledger:{}#{run}", db.display()),
                        warnings,
                    });
                }
                Ok(None) => {}
                Err(e) => warnings.push(format!("ledger {}: {e}; ignored", db.display())),
            }
        } else {
            warnings.push(format!("ledger {} does not exist; ignored", db.display()));
        }
    }

    if options.extract {
        let job = Job {
            path: path.to_string_lossy().into_owned(),
            backend: "lopdf".to_string(),
            pages: None,
            password: None,
            max_bytes: options.max_bytes,
            figures_dir: None,
        };
        let result = catch_unwind(AssertUnwindSafe(|| pipeline::run_job(&job)))
            .map_err(|payload| LoadError::Panicked(panic_message(payload.as_ref())))??;
        return Ok(Loaded {
            path: Some(path.to_path_buf()),
            size,
            sha256,
            result: Some(result),
            text_source: "extract:lopdf".to_string(),
            warnings,
        });
    }

    warnings.push("no stored text and extraction disabled: byte identity only".to_string());
    Ok(Loaded {
        path: Some(path.to_path_buf()),
        size,
        sha256,
        result: None,
        text_source: "none:extraction disabled".to_string(),
        warnings,
    })
}

fn run_from_ledger(
    db: &Path,
    sha256: &str,
) -> Result<Option<(i64, ExtractionResult)>, LedgerError> {
    let ledger = Ledger::open(db)?;
    let Some(run) = ledger.latest_run_for_prefix(sha256)? else {
        return Ok(None);
    };
    let result = ledger.load_result(run)?;
    if result.document.hash.0 != sha256 {
        return Ok(None);
    }
    Ok(Some((run, result)))
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn collect_walks_directories_and_classifies_files() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("sub");
        fs::create_dir(&nested).unwrap();
        fs::write(dir.path().join("b.PDF"), b"%PDF-1.4").unwrap();
        fs::write(dir.path().join("a.pdf"), b"%PDF-1.4").unwrap();
        fs::write(dir.path().join("notes.txt"), b"x").unwrap();
        fs::write(nested.join("c.pdf"), b"%PDF-1.4").unwrap();
        let result_json = dir.path().join("r.json");
        fs::write(&result_json, b"{}").unwrap();
        let missing = dir.path().join("missing.pdf");

        let (specs, errors) = collect(&[
            dir.path().to_path_buf(),
            result_json.clone(),
            missing.clone(),
        ]);
        assert_eq!(
            specs,
            vec![
                InputSpec::Pdf(dir.path().join("a.pdf")),
                InputSpec::Pdf(dir.path().join("b.PDF")),
                InputSpec::Pdf(nested.join("c.pdf")),
                InputSpec::Result(result_json),
            ]
        );
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, missing);
    }

    #[test]
    fn non_pdf_bytes_are_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("x.pdf");
        fs::write(&path, b"hello world this is not a pdf").unwrap();
        let err = load(&InputSpec::Pdf(path), &LoadOptions::default()).unwrap_err();
        assert!(matches!(err, LoadError::NotPdf), "{err}");
    }

    #[test]
    fn pdf_without_text_sources_is_byte_identity_only() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("x.pdf");
        fs::write(&path, b"%PDF-1.4\n%%EOF\n").unwrap();
        let loaded = load(&InputSpec::Pdf(path.clone()), &LoadOptions::default()).unwrap();
        assert!(loaded.result.is_none());
        assert_eq!(loaded.path, Some(path));
        assert_eq!(loaded.sha256.len(), 64);
        assert!(loaded.text_source.starts_with("none:"));
    }

    #[test]
    fn malformed_result_json_is_an_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("r.json");
        fs::write(&path, b"{\"schema_version\": 1}").unwrap();
        let err = load(&InputSpec::Result(path), &LoadOptions::default()).unwrap_err();
        assert!(matches!(err, LoadError::Json(_)), "{err}");
    }
}
