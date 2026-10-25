//! Pure-Rust OCR with `ocrs` on the `rten` runtime. The two model files are
//! pinned by size and sha256 in `models/manifest.json` (embedded here); a
//! file that is missing, the wrong size or the wrong digest makes the engine
//! unavailable with that reason. Nothing is downloaded by this module.

use std::path::{Path, PathBuf};

use image::GrayImage;
use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{Availability, BBox, Block, Engine, EngineOptions, Recognition};
use crate::ImageTextError;

/// The embedded manifest (`crates/tpe-image-text/models/manifest.json`).
pub const MANIFEST_JSON: &str = include_str!("../../models/manifest.json");
/// Default model directory, relative to the working directory.
pub const DEFAULT_MODELS_DIR: &str = ".models/ocrs";
/// Pinned crate versions, kept in step with `Cargo.toml`.
pub const OCRS_VERSION: &str = "0.13.1";
/// Pinned rten version, kept in step with `Cargo.toml`.
pub const RTEN_VERSION: &str = "0.26.0";
/// Only language the pinned models were trained for.
pub const SUPPORTED_LANG: &str = "eng";
/// Lines closer than this fraction of the previous line's height form one block.
const BLOCK_GAP_FACTOR: f32 = 0.8;

/// One pinned model file.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ManifestEntry {
    /// `detection` or `recognition`.
    pub name: String,
    /// Where `fetch-models.sh` downloads it from.
    pub url: String,
    /// File name inside the models directory.
    pub dest: String,
    /// Exact size.
    pub bytes: u64,
    /// Hex sha256 of the file.
    pub sha256: String,
}

/// The embedded manifest.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    /// Manifest schema version.
    pub version: u32,
    /// Pinned files.
    pub entries: Vec<ManifestEntry>,
}

/// Parse the embedded manifest.
pub fn manifest() -> Result<Manifest, String> {
    serde_json::from_str(MANIFEST_JSON)
        .map_err(|e| format!("embedded models manifest is invalid: {e}"))
}

/// Explicit directory, else `.models/ocrs` under the working directory.
pub fn models_dir(explicit: Option<&Path>) -> PathBuf {
    explicit.map_or_else(|| PathBuf::from(DEFAULT_MODELS_DIR), Path::to_path_buf)
}

/// Model files whose size and digest matched the manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedModels {
    /// Directory they were read from.
    pub dir: PathBuf,
    /// Text detection model.
    pub detection: PathBuf,
    /// Text recognition model.
    pub recognition: PathBuf,
}

/// Verify every manifest entry under `dir`; the first failure is the reason.
pub fn verify_models(dir: &Path) -> Result<VerifiedModels, String> {
    let manifest = manifest()?;
    let mut detection = None;
    let mut recognition = None;
    for entry in &manifest.entries {
        let path = dir.join(&entry.dest);
        let bytes = std::fs::read(&path).map_err(|e| {
            format!(
                "{} model missing or unreadable at {} ({e}); run crates/tpe-image-text/fetch-models.sh",
                entry.name,
                path.display()
            )
        })?;
        if bytes.len() as u64 != entry.bytes {
            return Err(format!(
                "{} model at {} is {} bytes, manifest pins {}; refusing to use it",
                entry.name,
                path.display(),
                bytes.len(),
                entry.bytes
            ));
        }
        let digest = hex::encode(Sha256::digest(&bytes));
        if digest != entry.sha256 {
            return Err(format!(
                "{} model at {} has sha256 {digest}, manifest pins {}; refusing to use it",
                entry.name,
                path.display(),
                entry.sha256
            ));
        }
        match entry.name.as_str() {
            "detection" => detection = Some(path),
            "recognition" => recognition = Some(path),
            other => return Err(format!("unknown manifest entry `{other}`")),
        }
    }
    Ok(VerifiedModels {
        dir: dir.to_path_buf(),
        detection: detection.ok_or("manifest has no detection entry")?,
        recognition: recognition.ok_or("manifest has no recognition entry")?,
    })
}

/// A loaded ocrs engine.
pub struct OcrsEngine {
    engine: OcrEngine,
    models: VerifiedModels,
}

impl std::fmt::Debug for OcrsEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OcrsEngine")
            .field("models", &self.models)
            .finish_non_exhaustive()
    }
}

impl OcrsEngine {
    /// Load the verified models.
    pub fn load(models: VerifiedModels) -> Result<Self, String> {
        let detection_model = rten::Model::load_file(&models.detection)
            .map_err(|e| format!("loading {}: {e}", models.detection.display()))?;
        let recognition_model = rten::Model::load_file(&models.recognition)
            .map_err(|e| format!("loading {}: {e}", models.recognition.display()))?;
        let engine = OcrEngine::new(OcrEngineParams {
            detection_model: Some(detection_model),
            recognition_model: Some(recognition_model),
            ..OcrEngineParams::default()
        })
        .map_err(|e| format!("initialising ocrs: {e}"))?;
        Ok(Self { engine, models })
    }

    /// The verified model files in use.
    pub fn models(&self) -> &VerifiedModels {
        &self.models
    }
}

/// Verify and load the models, reporting exactly what blocked it otherwise.
pub fn probe(opts: &EngineOptions) -> (Option<OcrsEngine>, Availability) {
    let dir = models_dir(opts.ocrs_models_dir.as_deref());
    let outcome = verify_models(&dir).and_then(OcrsEngine::load);
    match outcome {
        Ok(engine) => {
            let report = Availability {
                engine: "ocrs".to_string(),
                available: true,
                version: Some(engine.version()),
                detail: format!("models verified in {}", dir.display()),
            };
            (Some(engine), report)
        }
        Err(detail) => (
            None,
            Availability {
                engine: "ocrs".to_string(),
                available: false,
                version: None,
                detail,
            },
        ),
    }
}

impl Engine for OcrsEngine {
    fn name(&self) -> &'static str {
        "ocrs"
    }

    fn version(&self) -> String {
        format!("ocrs {OCRS_VERSION} (rten {RTEN_VERSION}, models manifest v1)")
    }

    fn recognize(&self, image: &GrayImage, lang: &str) -> Result<Recognition, ImageTextError> {
        if lang != SUPPORTED_LANG {
            return Err(ImageTextError::Unsupported(format!(
                "ocrs has only the English/Latin-script models; `{lang}` is not supported (use --engine tesseract --lang {lang})"
            )));
        }
        let source = ImageSource::from_bytes(image.as_raw(), image.dimensions())
            .map_err(|e| ImageTextError::Engine(format!("ocrs input: {e}")))?;
        let input = self
            .engine
            .prepare_input(source)
            .map_err(|e| ImageTextError::Engine(format!("ocrs prepare: {e}")))?;
        let words = self
            .engine
            .detect_words(&input)
            .map_err(|e| ImageTextError::Engine(format!("ocrs detection: {e}")))?;
        let lines = self.engine.find_text_lines(&input, &words);
        let texts = self
            .engine
            .recognize_text(&input, &lines)
            .map_err(|e| ImageTextError::Engine(format!("ocrs recognition: {e}")))?;
        let mut recognised: Vec<(String, BBox)> = Vec::new();
        for line in texts.into_iter().flatten() {
            let text = line.to_string();
            if text.trim().is_empty() {
                continue;
            }
            let rect = line.bounding_rect();
            let bbox = BBox {
                x: u32::try_from(rect.left().max(0)).unwrap_or(0),
                y: u32::try_from(rect.top().max(0)).unwrap_or(0),
                width: u32::try_from(rect.width().max(0)).unwrap_or(0),
                height: u32::try_from(rect.height().max(0)).unwrap_or(0),
            };
            recognised.push((text.trim().to_string(), bbox));
        }
        Ok(Recognition {
            blocks: group_lines(recognised),
            warnings: vec![format!(
                "ocrs {OCRS_VERSION} exposes no per-character confidence; block confidence is null"
            )],
            resource_limits_applied: None,
        })
    }
}

/// Group reading-ordered lines into blocks by vertical gap.
pub fn group_lines(lines: Vec<(String, BBox)>) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut previous: Option<BBox> = None;
    for (text, bbox) in lines {
        let continues = previous.is_some_and(|prev| {
            let prev_bottom = prev.y + prev.height;
            let gap = bbox.y.saturating_sub(prev_bottom) as f32;
            bbox.y >= prev.y && gap <= prev.height.max(1) as f32 * BLOCK_GAP_FACTOR
        });
        match blocks.last_mut() {
            Some(last) if continues => {
                last.text.push('\n');
                last.text.push_str(&text);
                last.bbox = Some(last.bbox.map_or(bbox, |b| b.union(bbox)));
            }
            _ => blocks.push(Block {
                text,
                confidence: None,
                bbox: Some(bbox),
            }),
        }
        previous = Some(bbox);
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_manifest_pins_both_models() {
        let m = manifest().expect("manifest");
        assert_eq!(m.version, 1);
        let names: Vec<&str> = m.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["detection", "recognition"]);
        for e in &m.entries {
            assert_eq!(e.sha256.len(), 64, "{}", e.name);
            assert!(e.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(e.bytes > 0 && e.bytes < 150 * 1024 * 1024);
            assert!(e.url.starts_with("https://"));
        }
    }

    #[test]
    fn verification_fails_closed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = verify_models(dir.path()).expect_err("missing");
        assert!(err.contains("missing or unreadable"), "{err}");
        std::fs::write(dir.path().join("text-detection.rten"), b"short").expect("write");
        let err = verify_models(dir.path()).expect_err("wrong size");
        assert!(err.contains("bytes, manifest pins"), "{err}");
        let m = manifest().expect("manifest");
        let wrong = vec![0u8; usize::try_from(m.entries[0].bytes).expect("size")];
        std::fs::write(dir.path().join("text-detection.rten"), wrong).expect("write");
        let err = verify_models(dir.path()).expect_err("wrong hash");
        assert!(
            err.contains("manifest pins") && err.contains("sha256"),
            "{err}"
        );
    }

    #[test]
    fn lines_group_by_vertical_gap() {
        let line = |y: u32, text: &str| {
            (
                text.to_string(),
                BBox {
                    x: 10,
                    y,
                    width: 100,
                    height: 20,
                },
            )
        };
        let blocks = group_lines(vec![line(0, "a"), line(24, "b"), line(100, "c")]);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].text, "a\nb");
        assert_eq!(blocks[1].text, "c");
        assert_eq!(blocks[0].bbox.map(|b| b.height), Some(44));
    }
}
