//! OCR engines and their honest availability reports. An engine is used only
//! when it is really present: a `tesseract` executable, verified `ocrs` model
//! files, or (never, in this build) docling's ONNX OCR. Nothing downloads.

pub mod docling;
#[cfg(feature = "ocrs")]
pub mod ocrs;
pub mod tesseract;

use std::path::PathBuf;
use std::time::Duration;

use image::GrayImage;
use serde::{Deserialize, Serialize};

use crate::ImageTextError;

/// Engine selection from the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum EngineKind {
    /// First available of tesseract, ocrs, docling.
    Auto,
    /// The `tesseract` command-line program, run with a timeout and resource limits.
    Tesseract,
    /// docling's ONNX OCR models: detected and reported, not runnable in this build.
    Docling,
    /// Pure-Rust ocrs (rten) with the pinned models from `models/manifest.json`.
    Ocrs,
}

impl EngineKind {
    /// Lower-case name as used in reports and `--engine`.
    pub fn name(self) -> &'static str {
        match self {
            EngineKind::Auto => "auto",
            EngineKind::Tesseract => "tesseract",
            EngineKind::Docling => "docling",
            EngineKind::Ocrs => "ocrs",
        }
    }
}

/// Order in which `auto` tries engines.
pub const AUTO_ORDER: [EngineKind; 3] =
    [EngineKind::Tesseract, EngineKind::Ocrs, EngineKind::Docling];

/// Axis-aligned box in pixels of the preprocessed image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BBox {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

impl BBox {
    /// Smallest box containing both.
    #[must_use]
    pub fn union(self, other: BBox) -> BBox {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        BBox {
            x,
            y,
            width: right - x,
            height: bottom - y,
        }
    }
}

/// One block of text (a paragraph or line group) in reading order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
    /// Lines joined with `\n`.
    pub text: String,
    /// Mean recognition confidence in `0..=1`, when the engine reports one.
    pub confidence: Option<f32>,
    /// Position in the preprocessed image, when the engine reports one.
    pub bbox: Option<BBox>,
}

/// What an engine returned for one image.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Recognition {
    /// Blocks in reading order.
    pub blocks: Vec<Block>,
    /// Non-fatal engine messages.
    pub warnings: Vec<String>,
    /// For subprocess engines: the kernel limits in force in the engine
    /// process (`core`, `cpu`, `fsize`, `as`), empty when none could be
    /// installed; `None` for engines that run in this process.
    pub resource_limits_applied: Option<Vec<String>>,
}

impl Recognition {
    /// Plain text: blocks separated by a blank line, ending with a newline.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for (i, block) in self.blocks.iter().enumerate() {
            if i > 0 {
                out.push_str("\n\n");
            }
            out.push_str(block.text.trim_end());
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out
    }
}

/// A usable OCR engine.
pub trait Engine {
    /// Stable lower-case name.
    fn name(&self) -> &'static str;
    /// Version string of the engine as detected.
    fn version(&self) -> String;
    /// Recognise text in a preprocessed grayscale image.
    fn recognize(&self, image: &GrayImage, lang: &str) -> Result<Recognition, ImageTextError>;
}

/// Why an engine is or is not usable right now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Availability {
    /// Engine name.
    pub engine: String,
    /// Whether it can run.
    pub available: bool,
    /// Detected version when available.
    pub version: Option<String>,
    /// Exact reason, e.g. the path probed or the file that failed verification.
    pub detail: String,
}

/// Where to look for engines and how hard to bound them.
#[derive(Clone, Debug)]
pub struct EngineOptions {
    /// Explicit tesseract executable (else `PATH` is searched).
    pub tesseract_bin: Option<PathBuf>,
    /// Path of this tool's own binary, used as the `exec-limited` helper.
    pub limit_helper: Option<PathBuf>,
    /// Wall-clock limit for one external recognition run.
    pub timeout: Duration,
    /// Address-space limit for an external engine process.
    pub address_space_bytes: u64,
    /// Directory holding the ocrs models (else `.models/ocrs`).
    pub ocrs_models_dir: Option<PathBuf>,
    /// Directory holding docling models (else `DOCLING_RS_MODELS_DIR` or `.models`).
    pub docling_models_dir: Option<PathBuf>,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            tesseract_bin: None,
            limit_helper: None,
            timeout: Duration::from_secs(120),
            address_space_bytes: 2 * 1024 * 1024 * 1024,
            ocrs_models_dir: None,
            docling_models_dir: None,
        }
    }
}

/// Probe one engine: the engine when usable, plus the report either way.
pub fn probe(kind: EngineKind, opts: &EngineOptions) -> (Option<Box<dyn Engine>>, Availability) {
    match kind {
        EngineKind::Tesseract => {
            let (engine, report) = tesseract::probe(opts);
            (engine.map(|e| Box::new(e) as Box<dyn Engine>), report)
        }
        EngineKind::Docling => (None, docling::probe(opts)),
        EngineKind::Ocrs => probe_ocrs(opts),
        EngineKind::Auto => {
            for candidate in AUTO_ORDER {
                let (engine, report) = probe(candidate, opts);
                if engine.is_some() {
                    return (engine, report);
                }
            }
            (
                None,
                Availability {
                    engine: "auto".to_string(),
                    available: false,
                    version: None,
                    detail: "no engine available".to_string(),
                },
            )
        }
    }
}

#[cfg(feature = "ocrs")]
fn probe_ocrs(opts: &EngineOptions) -> (Option<Box<dyn Engine>>, Availability) {
    let (engine, report) = ocrs::probe(opts);
    (engine.map(|e| Box::new(e) as Box<dyn Engine>), report)
}

#[cfg(not(feature = "ocrs"))]
fn probe_ocrs(_opts: &EngineOptions) -> (Option<Box<dyn Engine>>, Availability) {
    (
        None,
        Availability {
            engine: "ocrs".to_string(),
            available: false,
            version: None,
            detail: "not compiled: build tpe-image-text with the `ocrs` feature".to_string(),
        },
    )
}

/// Availability of every engine, in `auto` order.
pub fn probe_all(opts: &EngineOptions) -> Vec<Availability> {
    AUTO_ORDER.into_iter().map(|k| probe(k, opts).1).collect()
}

/// The engine to use, or a precise explanation of why none can run.
pub fn select(
    kind: EngineKind,
    opts: &EngineOptions,
) -> Result<(Box<dyn Engine>, Vec<Availability>), ImageTextError> {
    let probes = probe_all(opts);
    let candidates: Vec<EngineKind> = match kind {
        EngineKind::Auto => AUTO_ORDER.to_vec(),
        other => vec![other],
    };
    for candidate in candidates {
        let (engine, _) = probe(candidate, opts);
        if let Some(engine) = engine {
            return Ok((engine, probes));
        }
    }
    let mut reasons: Vec<String> = probes
        .iter()
        .filter(|p| kind == EngineKind::Auto || p.engine == kind.name())
        .map(|p| format!("{}: {}", p.engine, p.detail))
        .collect();
    if reasons.is_empty() {
        reasons.push(format!("{}: not probed", kind.name()));
    }
    Err(ImageTextError::NoEngine(reasons.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_joins_blocks_with_blank_lines() {
        let rec = Recognition {
            blocks: vec![
                Block {
                    text: "a\nb".to_string(),
                    confidence: None,
                    bbox: None,
                },
                Block {
                    text: "c  ".to_string(),
                    confidence: Some(0.5),
                    bbox: None,
                },
            ],
            ..Recognition::default()
        };
        assert_eq!(rec.text(), "a\nb\n\nc\n");
        assert_eq!(Recognition::default().text(), "");
    }

    #[test]
    fn bbox_union_covers_both() {
        let a = BBox {
            x: 10,
            y: 10,
            width: 5,
            height: 5,
        };
        let b = BBox {
            x: 2,
            y: 20,
            width: 4,
            height: 4,
        };
        assert_eq!(
            a.union(b),
            BBox {
                x: 2,
                y: 10,
                width: 13,
                height: 14
            }
        );
    }

    #[test]
    fn selecting_a_missing_tesseract_names_the_reason() {
        let opts = EngineOptions {
            tesseract_bin: Some(PathBuf::from("/nonexistent/tesseract-binary")),
            ..EngineOptions::default()
        };
        let err = select(EngineKind::Tesseract, &opts)
            .err()
            .expect("no engine");
        let msg = err.to_string();
        assert!(msg.contains("no OCR engine available"), "{msg}");
        assert!(msg.contains("/nonexistent/tesseract-binary"), "{msg}");
    }
}
