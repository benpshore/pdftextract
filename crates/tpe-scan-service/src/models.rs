//! Provisioning probe for the native artifacts the OCR path needs: the four
//! docling.rs `models-v1` files and the `PDFium` library (`docs/NATIVE.md`).
//! The search order mirrors docling-core's resolver so that what this report
//! says is provisioned is what the pipeline will actually load.

use std::fmt::Write;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

/// The docling.rs `models-v1` files pinned in `native/manifest.json`.
pub const MODEL_FILES: [&str; 4] = [
    "layout_heron_int8.onnx",
    "ocr_det.onnx",
    "ocr_rec_en.onnx",
    "en_dict.txt",
];
/// Environment variable docling-core consults for the models directory.
pub const MODELS_DIR_ENV: &str = "DOCLING_RS_MODELS_DIR";
/// Environment variable docling-pdf consults for the `PDFium` library.
pub const PDFIUM_ENV: &str = "PDFIUM_DYNAMIC_LIB_PATH";
/// OCR recognition languages the provisioned model pair covers.
pub const OCR_LANGUAGES: [&str; 1] = ["en"];

/// One model file: where it was found and its identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelFile {
    pub name: String,
    pub path: Option<PathBuf>,
    pub bytes: Option<u64>,
    pub sha256: Option<String>,
}

/// The outcome of a models probe.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelReport {
    /// Directories consulted, in resolution order.
    pub searched: Vec<PathBuf>,
    pub files: Vec<ModelFile>,
    pub provisioned: bool,
    pub missing: Vec<String>,
}

/// Directories docling-core consults for `.models/<file>`, in order: the
/// working directory's `.models`, `explicit` (or `$DOCLING_RS_MODELS_DIR`),
/// then `.models` next to the executable and one level above it.
pub fn search_dirs(explicit: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join(".models"));
    }
    let configured = explicit.map(Path::to_path_buf).or_else(|| {
        std::env::var_os(MODELS_DIR_ENV)
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
    });
    if let Some(dir) = configured {
        dirs.push(dir);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.join(".models"));
        if let Some(parent) = dir.parent() {
            dirs.push(parent.join(".models"));
        }
    }
    let mut unique: Vec<PathBuf> = Vec::with_capacity(dirs.len());
    for dir in dirs {
        if !unique.contains(&dir) {
            unique.push(dir);
        }
    }
    unique
}

/// Look for every model file in `dirs`; `hash` also computes SHA-256 digests
/// (about 82 MB to read when everything is present).
pub fn probe(dirs: &[PathBuf], hash: bool) -> ModelReport {
    let mut files = Vec::with_capacity(MODEL_FILES.len());
    let mut missing = Vec::new();
    for name in MODEL_FILES {
        let found = dirs
            .iter()
            .map(|dir| dir.join(name))
            .find(|path| path.is_file());
        if let Some(path) = found {
            let bytes = std::fs::metadata(&path).ok().map(|meta| meta.len());
            let sha256 = if hash { sha256_file(&path).ok() } else { None };
            files.push(ModelFile {
                name: name.to_string(),
                path: Some(path),
                bytes,
                sha256,
            });
        } else {
            missing.push(name.to_string());
            files.push(ModelFile {
                name: name.to_string(),
                path: None,
                bytes: None,
                sha256: None,
            });
        }
    }
    ModelReport {
        searched: dirs.to_vec(),
        provisioned: missing.is_empty(),
        files,
        missing,
    }
}

/// Re-probe presence cheaply, keeping the digests of files whose path and
/// size are unchanged since `previous`.
pub fn refresh(previous: &ModelReport, hash_new: bool) -> ModelReport {
    let mut current = probe(&previous.searched, false);
    for file in &mut current.files {
        let known = previous
            .files
            .iter()
            .find(|old| old.name == file.name && old.path == file.path && old.bytes == file.bytes);
        match known {
            Some(old) => file.sha256.clone_from(&old.sha256),
            None if hash_new => {
                file.sha256 = file.path.as_deref().and_then(|path| sha256_file(path).ok());
            }
            None => {}
        }
    }
    current
}

/// Hex SHA-256 of a file, streamed.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

/// Where the `PDFium` library is configured to be, and whether it is there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PdfiumReport {
    /// The raw `PDFIUM_DYNAMIC_LIB_PATH` value, when set.
    pub configured: Option<String>,
    /// The library file that value names (directory values get the platform file name).
    pub library: Option<PathBuf>,
    pub present: bool,
}

/// Platform file name of the `PDFium` shared library.
pub fn pdfium_library_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "libpdfium.dylib"
    } else if cfg!(windows) {
        "pdfium.dll"
    } else {
        "libpdfium.so"
    }
}

/// Interpret a `PDFIUM_DYNAMIC_LIB_PATH` value the way the engine does:
/// it must be absolute; a directory is completed with the library name.
pub fn pdfium_from(configured: Option<&str>) -> PdfiumReport {
    let Some(value) = configured.map(str::trim).filter(|value| !value.is_empty()) else {
        return PdfiumReport {
            configured: None,
            library: None,
            present: false,
        };
    };
    let path = Path::new(value);
    if !path.is_absolute() {
        return PdfiumReport {
            configured: Some(value.to_string()),
            library: None,
            present: false,
        };
    }
    let library = if path.is_dir() {
        path.join(pdfium_library_name())
    } else {
        path.to_path_buf()
    };
    let present = library.is_file();
    PdfiumReport {
        configured: Some(value.to_string()),
        library: Some(library),
        present,
    }
}

/// The `PDFium` report for this process's environment.
pub fn pdfium() -> PdfiumReport {
    pdfium_from(std::env::var(PDFIUM_ENV).ok().as_deref())
}

/// The precise message for an OCR request that cannot run with this
/// provisioning, or `None` when everything is in place.
pub fn provisioning_problem(models: &ModelReport, pdfium: &PdfiumReport) -> Option<String> {
    let mut problems = Vec::new();
    if !models.provisioned {
        let searched: Vec<String> = models
            .searched
            .iter()
            .map(|dir| dir.display().to_string())
            .collect();
        problems.push(format!(
            "models not provisioned: missing {} (searched {})",
            models.missing.join(", "),
            if searched.is_empty() {
                "no directory".to_string()
            } else {
                searched.join(", ")
            }
        ));
    }
    if !pdfium.present {
        problems.push(match &pdfium.configured {
            None => format!("PDFium library not configured: {PDFIUM_ENV} is unset"),
            Some(value) if !Path::new(value).is_absolute() => {
                format!("PDFium path {value} is not absolute ({PDFIUM_ENV})")
            }
            Some(value) => format!("PDFium library not found at {value}"),
        });
    }
    if problems.is_empty() {
        return None;
    }
    Some(format!(
        "{}; run `sh native/fetch.sh` from the repository root (about 82 MB of models plus the \
         PDFium library), then start the service with {MODELS_DIR_ENV}=<repo>/.models and \
         {PDFIUM_ENV}=<repo>/.pdfium/lib as absolute paths",
        problems.join("; ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_reports_each_missing_file_and_digests_present_ones() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("en_dict.txt"), b"a\nb\n").unwrap();
        let report = probe(&[dir.path().to_path_buf()], true);
        assert!(!report.provisioned);
        assert_eq!(
            report.missing,
            vec!["layout_heron_int8.onnx", "ocr_det.onnx", "ocr_rec_en.onnx"]
        );
        let dict = report
            .files
            .iter()
            .find(|file| file.name == "en_dict.txt")
            .unwrap();
        assert_eq!(dict.bytes, Some(4));
        assert_eq!(
            dict.sha256.as_deref(),
            Some(
                sha256_file(dir.path().join("en_dict.txt").as_path())
                    .unwrap()
                    .as_str()
            )
        );
        assert_eq!(dict.sha256.as_ref().map(String::len), Some(64));
        assert!(
            dict.sha256
                .as_ref()
                .unwrap()
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        );
        let message = provisioning_problem(&report, &pdfium_from(None)).unwrap();
        assert!(message.starts_with("models not provisioned: missing layout_heron_int8.onnx"));
        assert!(message.contains("native/fetch.sh"));
        assert!(message.contains("PDFIUM_DYNAMIC_LIB_PATH is unset"));
    }

    #[test]
    fn a_complete_directory_is_provisioned_and_refresh_keeps_digests() {
        let dir = tempfile::tempdir().unwrap();
        for name in MODEL_FILES {
            std::fs::write(dir.path().join(name), name.as_bytes()).unwrap();
        }
        let report = probe(&[dir.path().to_path_buf()], true);
        assert!(report.provisioned && report.missing.is_empty());
        assert!(report.files.iter().all(|file| file.sha256.is_some()));
        let refreshed = refresh(&report, false);
        assert_eq!(refreshed, report);
        std::fs::remove_file(dir.path().join("ocr_det.onnx")).unwrap();
        let refreshed = refresh(&report, false);
        assert_eq!(refreshed.missing, vec!["ocr_det.onnx"]);
        assert!(!refreshed.provisioned);
    }

    #[test]
    fn pdfium_path_must_be_absolute_and_present() {
        assert!(!pdfium_from(None).present);
        assert!(!pdfium_from(Some("   ")).present);
        let relative = pdfium_from(Some(".pdfium/lib"));
        assert!(!relative.present && relative.library.is_none());
        let dir = tempfile::tempdir().unwrap();
        let missing = pdfium_from(dir.path().to_str());
        assert!(!missing.present);
        assert_eq!(
            missing.library.as_deref(),
            Some(dir.path().join(pdfium_library_name()).as_path())
        );
        let file = dir.path().join(pdfium_library_name());
        std::fs::write(&file, []).unwrap();
        assert!(pdfium_from(dir.path().to_str()).present);
        assert!(pdfium_from(file.to_str()).present);
        let problem = provisioning_problem(
            &probe(&[dir.path().to_path_buf()], false),
            &pdfium_from(Some(".pdfium/lib")),
        )
        .unwrap();
        assert!(problem.contains("is not absolute"));
    }

    #[test]
    fn search_order_starts_with_the_working_directory_and_dedups() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = search_dirs(Some(dir.path()));
        assert_eq!(dirs[0], std::env::current_dir().unwrap().join(".models"));
        assert_eq!(dirs[1], dir.path());
        let mut sorted = dirs.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), dirs.len());
    }
}
