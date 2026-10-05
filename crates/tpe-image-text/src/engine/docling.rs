//! docling: detection only. docling.rs 1.69.2 performs OCR with PP-OCR ONNX
//! models inside its PDF pipeline (`docling-pdf` feature `ml`: ONNX Runtime
//! fetched at build time plus `PDFium`). The root crate's `docling-text` backend
//! parses PDF text layers and does no OCR at all (docs/DOCLING.md). This tool
//! links neither, so the engine is never runnable here; what it does is detect
//! whether the models from `native/manifest.json` are provisioned and say so.

use std::path::{Path, PathBuf};

use super::{Availability, EngineOptions};

/// Files docling's OCR needs, as pinned in `native/manifest.json`.
pub const MODEL_FILES: [&str; 3] = ["ocr_det.onnx", "ocr_rec_en.onnx", "en_dict.txt"];

/// Where docling looks for models: explicit, `DOCLING_RS_MODELS_DIR`, else `.models`.
pub fn models_dir(explicit: Option<&Path>) -> PathBuf {
    explicit.map_or_else(
        || {
            std::env::var_os("DOCLING_RS_MODELS_DIR")
                .filter(|v| !v.is_empty())
                .map_or_else(|| PathBuf::from(".models"), PathBuf::from)
        },
        Path::to_path_buf,
    )
}

/// Report provisioning state and the reason the engine cannot run in this build.
pub fn probe(opts: &EngineOptions) -> Availability {
    let dir = models_dir(opts.docling_models_dir.as_deref());
    let (present, missing): (Vec<&str>, Vec<&str>) =
        MODEL_FILES.iter().partition(|f| dir.join(f).is_file());
    let state = if missing.is_empty() {
        format!("OCR models present in {}", dir.display())
    } else {
        format!(
            "OCR models not provisioned in {} (missing {}; present {})",
            dir.display(),
            missing.join(", "),
            if present.is_empty() {
                "none".to_string()
            } else {
                present.join(", ")
            }
        )
    };
    Availability {
        engine: "docling".to_string(),
        available: false,
        version: None,
        detail: format!(
            "{state}; not runnable: docling 1.69.2 OCR exists only inside its PDF pipeline (docling-pdf `ml`: ONNX Runtime downloaded at build time + PDFium), which this tool does not link; the root `docling-text` backend performs no OCR"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_missing_models_and_never_claims_availability() {
        let dir = tempfile::tempdir().expect("tempdir");
        let opts = EngineOptions {
            docling_models_dir: Some(dir.path().to_path_buf()),
            ..EngineOptions::default()
        };
        let report = probe(&opts);
        assert!(!report.available);
        assert!(
            report.detail.contains("not provisioned"),
            "{}",
            report.detail
        );
        assert!(report.detail.contains("ocr_det.onnx"), "{}", report.detail);
        for f in MODEL_FILES {
            std::fs::write(dir.path().join(f), b"x").expect("write");
        }
        let report = probe(&opts);
        assert!(!report.available);
        assert!(
            report.detail.contains("models present"),
            "{}",
            report.detail
        );
        assert!(report.detail.contains("not runnable"), "{}", report.detail);
    }

    #[test]
    fn explicit_dir_wins_over_default() {
        assert_eq!(models_dir(Some(Path::new("/x"))), PathBuf::from("/x"));
    }
}
