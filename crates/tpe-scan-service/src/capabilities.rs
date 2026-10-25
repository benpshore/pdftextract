//! The `GET /capabilities` document: identity, what this build compiled in,
//! what is provisioned on this machine, and the service's limits.

use serde::Serialize;

use crate::models::{ModelReport, OCR_LANGUAGES, PdfiumReport, provisioning_problem};
use crate::scan::Mode;
use crate::server::Config;
use crate::{DOCLING_VERSION, SERVICE_NAME, VERSION};

/// The limits a client should respect before sending a request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Limits {
    pub max_body_bytes: u64,
    pub max_pages: u32,
    pub max_concurrent: usize,
    pub scan_timeout_ms: u64,
    pub worker_memory_growth_mib: u64,
}

impl Limits {
    /// The limits of `config`.
    pub fn of(config: &Config) -> Self {
        Self {
            max_body_bytes: config.max_body_bytes,
            max_pages: config.max_pages,
            max_concurrent: config.max_concurrent,
            scan_timeout_ms: u64::try_from(config.scan_timeout.as_millis()).unwrap_or(u64::MAX),
            worker_memory_growth_mib: config.worker_memory_growth_mib,
        }
    }
}

/// Build the capabilities document.
pub fn document(config: &Config, models: &ModelReport, pdfium: &PdfiumReport) -> serde_json::Value {
    let ocr_compiled = Mode::Ocr.compiled();
    let text_compiled = Mode::Text.compiled();
    let provisioning = provisioning_problem(models, pdfium);
    let ocr_available = ocr_compiled && provisioning.is_none();
    let ocr_reason = if ocr_compiled {
        provisioning.clone()
    } else {
        Some(format!(
            "unsupported: this build has no `docling` backend; build tpe-scan-service with \
             --features {}",
            Mode::Ocr.feature()
        ))
    };
    let recognition_present = models
        .files
        .iter()
        .any(|file| file.name == "ocr_rec_en.onnx" && file.path.is_some())
        && models
            .files
            .iter()
            .any(|file| file.name == "en_dict.txt" && file.path.is_some());
    let languages: Vec<&str> = if recognition_present {
        OCR_LANGUAGES.to_vec()
    } else {
        Vec::new()
    };
    serde_json::json!({
        "service": {
            "name": SERVICE_NAME,
            "version": VERSION,
            "docling_version": DOCLING_VERSION,
            "pid": std::process::id(),
            "loopback_only": true,
            "telemetry": false,
        },
        "build": {
            "ocr_compiled": ocr_compiled,
            "text_layer_compiled": text_compiled,
            "features": {"docling": ocr_compiled, "docling-text": text_compiled},
        },
        "ocr": {
            "available": ocr_available,
            "engine": "ppocr",
            "languages": languages,
            "reason": ocr_reason,
            "confidence": "per-page mean scores when the backend reports them; no per-word confidence",
        },
        "text_layer": {
            "available": text_compiled,
            "reason": if text_compiled { None } else { Some(format!(
                "unsupported: build tpe-scan-service with --features {}", Mode::Text.feature())) },
        },
        "models": models,
        "pdfium": pdfium,
        "limits": Limits::of(config),
        "accepts": ["application/pdf", "image/png", "image/jpeg", "multipart/form-data"],
        "modes": [Mode::Ocr.as_str(), Mode::Text.as_str()],
        "origins": config.origins,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{pdfium_from, probe};

    #[test]
    fn document_states_build_provisioning_and_limits_honestly() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new("0123456789abcdef0123456789abcdef".to_string());
        let models = probe(&[dir.path().to_path_buf()], false);
        let value = document(&config, &models, &pdfium_from(None));
        assert_eq!(value["service"]["name"], SERVICE_NAME);
        assert_eq!(value["service"]["docling_version"], DOCLING_VERSION);
        assert_eq!(value["build"]["ocr_compiled"], cfg!(feature = "docling"));
        assert_eq!(value["ocr"]["available"], false);
        assert!(
            value["ocr"]["reason"]
                .as_str()
                .unwrap()
                .contains(if cfg!(feature = "docling") {
                    "models not provisioned"
                } else {
                    "--features docling"
                })
        );
        assert_eq!(value["ocr"]["languages"].as_array().unwrap().len(), 0);
        assert_eq!(value["models"]["provisioned"], false);
        assert_eq!(value["limits"]["max_pages"], config.max_pages);
        assert_eq!(value["limits"]["max_body_bytes"], config.max_body_bytes);
        assert_eq!(value["service"]["telemetry"], false);
    }
}
