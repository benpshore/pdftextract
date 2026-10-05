//! The scan itself: run bytes through one of the engine's docling backends
//! and shape the per-page evidence for the browser. This runs inside the
//! disposable worker process (`worker.rs`); the HTTP layer never calls it
//! directly.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use tpe::backend::{BackendError, EncryptionProblem, Extractor};
use tpe::schema::{BBox, BackendIdentity, PageText};

use crate::models;

/// Which backend a scan goes through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Full docling pipeline (layout + PP-OCR); needs the models and `PDFium`.
    #[default]
    Ocr,
    /// Pure-Rust text layer only (no OCR): reads what a PDF already contains.
    Text,
}

impl Mode {
    /// The engine backend name (`tpe::backend::by_name`).
    pub fn backend_name(self) -> &'static str {
        match self {
            Self::Ocr => "docling",
            Self::Text => "docling-text",
        }
    }

    /// The Cargo feature of this crate that compiles the backend in.
    pub fn feature(self) -> &'static str {
        match self {
            Self::Ocr => "docling",
            Self::Text => "docling-text",
        }
    }

    /// Whether this build compiled the backend in.
    pub fn compiled(self) -> bool {
        match self {
            Self::Ocr => cfg!(feature = "docling"),
            Self::Text => cfg!(feature = "docling-text"),
        }
    }

    /// Lower-case name used in JSON and the `mode` query parameter.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ocr => "ocr",
            Self::Text => "text",
        }
    }

    /// Parse the `mode` query parameter.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "ocr" => Some(Self::Ocr),
            "text" => Some(Self::Text),
            _ => None,
        }
    }
}

/// What to scan and how much.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanOptions {
    pub mode: Mode,
    /// Inclusive 1-based page window; `None` means every page (up to `max_pages`).
    pub pages: Option<(u32, u32)>,
    /// Most pages one scan may cover.
    pub max_pages: u32,
}

/// Parse a `pages` query value: `7` or `3-9` (1-based, inclusive).
pub fn parse_pages(value: &str) -> Result<(u32, u32), String> {
    let value = value.trim();
    let (first, last) = match value.split_once('-') {
        Some((first, last)) => (first.trim(), last.trim()),
        None => (value, value),
    };
    let parse = |text: &str| -> Result<u32, String> {
        text.parse::<u32>()
            .map_err(|_| format!("pages must be like `3` or `3-9`, not `{value}`"))
    };
    let (first, last) = (parse(first)?, parse(last)?);
    if first == 0 || last < first {
        return Err(format!(
            "pages must be a 1-based ascending range, not `{value}`"
        ));
    }
    Ok((first, last))
}

/// Short machine-readable failure classes; each maps to an HTTP status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    ModelsNotProvisioned,
    NotCompiled,
    Malformed,
    Encrypted,
    PageRange,
    Limit,
    Unsupported,
    Timeout,
    Busy,
    Internal,
}

impl ErrorCode {
    /// The HTTP status that reports this class.
    pub fn http_status(self) -> u16 {
        match self {
            Self::ModelsNotProvisioned | Self::NotCompiled => 503,
            Self::Malformed | Self::PageRange => 400,
            Self::Encrypted | Self::Unsupported => 422,
            Self::Limit => 413,
            Self::Timeout => 504,
            Self::Busy => 429,
            Self::Internal => 500,
        }
    }
}

/// A scan failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct ScanError {
    pub code: ErrorCode,
    pub message: String,
}

impl ScanError {
    /// Build an error.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<BackendError> for ScanError {
    fn from(error: BackendError) -> Self {
        let code = match &error {
            BackendError::Malformed(_) => ErrorCode::Malformed,
            BackendError::Encrypted(
                EncryptionProblem::PasswordRequired
                | EncryptionProblem::WrongPassword
                | EncryptionProblem::UnsupportedCipher,
            ) => ErrorCode::Encrypted,
            BackendError::PageRange { .. } => ErrorCode::PageRange,
            BackendError::Page { .. } => ErrorCode::Internal,
            BackendError::Unsupported(message) if message.contains("not provisioned") => {
                ErrorCode::ModelsNotProvisioned
            }
            BackendError::Unsupported(_) => ErrorCode::Unsupported,
            BackendError::Limit(_) => ErrorCode::Limit,
        };
        Self::new(code, error.to_string())
    }
}

/// docling's per-page confidence scores (means over the page), when the
/// backend reported them; every field is `None` when a stage did not run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PageConfidence {
    pub parse: Option<f64>,
    pub layout: Option<f64>,
    pub ocr: Option<f64>,
    pub table: Option<f64>,
}

/// Prefix of the informational warning the `docling` backend emits with the
/// page's confidence scores (`confidence: parse=0.991 layout=0.812 ocr=none table=none`).
pub const CONFIDENCE_PREFIX: &str = "confidence:";

/// Parse the backend's confidence line; `None` for any other warning.
pub fn parse_confidence(warning: &str) -> Option<PageConfidence> {
    let rest = warning.strip_prefix(CONFIDENCE_PREFIX)?;
    let mut confidence = PageConfidence::default();
    let mut seen = false;
    for field in rest.split_whitespace() {
        let (key, value) = field.split_once('=')?;
        let value = match value {
            "none" | "nan" => None,
            number => Some(number.parse::<f64>().ok()?),
        };
        match key {
            "parse" => confidence.parse = value,
            "layout" => confidence.layout = value,
            "ocr" => confidence.ocr = value,
            "table" => confidence.table = value,
            _ => return None,
        }
        seen = true;
    }
    seen.then_some(confidence)
}

/// A layout block: one ordered line/paragraph with its role and column.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub text: String,
    pub bbox: Option<BBox>,
    pub role: String,
    pub column: u32,
}

/// One raw positioned span as the backend produced it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpanOut {
    pub text: String,
    pub bbox: Option<BBox>,
    pub seq: u32,
}

/// An image or detected picture region.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FigureOut {
    pub index: u32,
    pub kind: String,
    pub bbox: Option<BBox>,
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
}

/// One scanned page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageOut {
    pub page: u32,
    pub width: f32,
    pub height: f32,
    pub rotation: i32,
    /// `complete` or `partial` (`PageText::extraction_status`).
    pub status: String,
    /// Ordered page text.
    pub text: String,
    pub confidence: Option<PageConfidence>,
    pub blocks: Vec<Block>,
    pub spans: Vec<SpanOut>,
    pub figures: Vec<FigureOut>,
    pub warnings: Vec<String>,
}

/// The result of one scan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanResult {
    pub backend: BackendIdentity,
    pub mode: Mode,
    pub pages_total: u32,
    /// The window actually scanned (1-based, inclusive).
    pub pages_scanned: (u32, u32),
    pub pages: Vec<PageOut>,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
}

/// The backend for `mode`, windowed to `pages` where the backend supports it.
fn backend_for(mode: Mode, pages: Option<(u32, u32)>) -> Option<Box<dyn Extractor>> {
    #[cfg(feature = "docling")]
    if mode == Mode::Ocr {
        let backend = tpe::backend::docling_backend::DoclingBackend::full();
        return Some(Box::new(match pages {
            Some((first, last)) => backend.with_window(first, last),
            None => backend,
        }));
    }
    let _ = pages;
    tpe::backend::by_name(mode.backend_name())
}

/// Scan `pdf` (already PDF bytes) in this process.
pub fn scan_bytes(pdf: &[u8], options: &ScanOptions) -> Result<ScanResult, ScanError> {
    let started = Instant::now();
    let mode = options.mode;
    if !mode.compiled() {
        return Err(ScanError::new(
            ErrorCode::NotCompiled,
            format!(
                "unsupported: this build has no `{}` backend; build tpe-scan-service with \
                 --features {}",
                mode.backend_name(),
                mode.feature()
            ),
        ));
    }
    if mode == Mode::Ocr {
        let models = models::probe(&models::search_dirs(None), false);
        if let Some(problem) = models::provisioning_problem(&models, &models::pdfium()) {
            return Err(ScanError::new(ErrorCode::ModelsNotProvisioned, problem));
        }
    }
    let backend = backend_for(mode, options.pages).ok_or_else(|| {
        ScanError::new(
            ErrorCode::NotCompiled,
            format!(
                "unsupported: backend {} is not registered",
                mode.backend_name()
            ),
        )
    })?;
    let identity = backend.identity();
    let provides_lines = backend.provides_line_layout();
    let mut session = backend.open(pdf, None)?;
    let total = session.page_count();
    let (first, last) = match options.pages {
        Some((first, last)) => {
            if first > total {
                return Err(ScanError::new(
                    ErrorCode::PageRange,
                    format!("page {first} out of range 1..={total}"),
                ));
            }
            (first, last.min(total))
        }
        None => (1, total),
    };
    let count = last - first + 1;
    if count > options.max_pages {
        return Err(ScanError::new(
            ErrorCode::Limit,
            format!(
                "requested {count} pages ({first}-{last}); the limit is {} per scan, use \
                 pages=<first>-<last> to scan a window",
                options.max_pages
            ),
        ));
    }
    let mut pages = Vec::with_capacity(count as usize);
    for number in first..=last {
        let mut page = session.page_text(number)?;
        if !provides_lines || page.lines.is_empty() {
            tpe::reading_order::order_page(&mut page);
        }
        pages.push(shape_page(&page));
    }
    let mut warnings = Vec::new();
    if mode == Mode::Text {
        warnings.push(
            "text mode reads the PDF's own text layer; it performs no OCR, so a scanned page \
             comes back empty"
                .to_string(),
        );
    }
    Ok(ScanResult {
        backend: identity,
        mode,
        pages_total: total,
        pages_scanned: (first, last),
        pages,
        warnings,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

/// Shape one page of engine evidence for the browser.
pub fn shape_page(page: &PageText) -> PageOut {
    let confidence = page.warnings.iter().find_map(|w| parse_confidence(w));
    PageOut {
        page: page.page,
        width: page.width,
        height: page.height,
        rotation: page.rotation,
        status: page.extraction_status().as_str().to_string(),
        text: page.text.clone(),
        confidence,
        blocks: page
            .lines
            .iter()
            .map(|line| Block {
                text: line.text.clone(),
                bbox: line.bbox,
                role: line.role.clone(),
                column: line.column,
            })
            .collect(),
        spans: page
            .spans
            .iter()
            .map(|span| SpanOut {
                text: span.text.clone(),
                bbox: span.bbox,
                seq: span.seq,
            })
            .collect(),
        figures: page
            .figures
            .iter()
            .map(|figure| FigureOut {
                index: figure.index,
                kind: figure.kind.clone(),
                bbox: figure.bbox,
                width_px: figure.width_px,
                height_px: figure.height_px,
            })
            .collect(),
        warnings: page
            .warnings
            .iter()
            .filter(|warning| !warning.starts_with(CONFIDENCE_PREFIX))
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpe::schema::{Line, Span};

    #[test]
    fn page_ranges_parse_singles_and_ascending_pairs_only() {
        assert_eq!(parse_pages("7"), Ok((7, 7)));
        assert_eq!(parse_pages(" 3 - 9 "), Ok((3, 9)));
        assert!(parse_pages("0").is_err());
        assert!(parse_pages("9-3").is_err());
        assert!(parse_pages("a-b").is_err());
        assert!(parse_pages("").is_err());
        assert_eq!(Mode::parse("TEXT"), Some(Mode::Text));
        assert_eq!(Mode::parse("ocr"), Some(Mode::Ocr));
        assert_eq!(Mode::parse("other"), None);
    }

    #[test]
    fn confidence_line_round_trips_and_other_warnings_are_ignored() {
        let parsed = parse_confidence("confidence: parse=0.991 layout=0.812 ocr=none table=nan");
        assert_eq!(
            parsed,
            Some(PageConfidence {
                parse: Some(0.991),
                layout: Some(0.812),
                ocr: None,
                table: None
            })
        );
        assert_eq!(parse_confidence("confidence:"), None);
        assert_eq!(parse_confidence("confidence: bogus=1"), None);
        assert_eq!(parse_confidence("extraction_incomplete: x"), None);
    }

    #[test]
    fn shaped_page_keeps_status_blocks_and_strips_the_confidence_line() {
        let mut page = PageText::new(2, 612.0, 792.0, 90);
        page.spans.push(Span {
            text: "Hello".to_string(),
            bbox: Some(BBox {
                x0: 1.0,
                y0: 2.0,
                x1: 3.0,
                y1: 4.0,
            }),
            font: None,
            size: None,
            seq: 0,
        });
        page.lines.push(Line {
            text: "Hello".to_string(),
            spans: vec![0],
            role: "heading".to_string(),
            ..Line::default()
        });
        page.text = "Hello".to_string();
        page.warnings
            .push("confidence: parse=none layout=0.5 ocr=0.9 table=none".to_string());
        page.warnings
            .push("extraction_incomplete: coverage unverified".to_string());
        let shaped = shape_page(&page);
        assert_eq!(shaped.status, "partial");
        assert_eq!(shaped.rotation, 90);
        assert_eq!(shaped.blocks[0].role, "heading");
        assert_eq!(shaped.spans[0].seq, 0);
        assert_eq!(shaped.confidence.unwrap().ocr, Some(0.9));
        assert_eq!(
            shaped.warnings,
            vec!["extraction_incomplete: coverage unverified"]
        );
    }

    #[test]
    fn backend_errors_map_to_http_classes() {
        let encrypted: ScanError =
            BackendError::Encrypted(EncryptionProblem::PasswordRequired).into();
        assert_eq!(encrypted.code, ErrorCode::Encrypted);
        assert_eq!(encrypted.code.http_status(), 422);
        let limit: ScanError = BackendError::Limit("x".to_string()).into();
        assert_eq!(limit.code.http_status(), 413);
        let malformed: ScanError = BackendError::Malformed("x".to_string()).into();
        assert_eq!(malformed.code.http_status(), 400);
        assert_eq!(ErrorCode::ModelsNotProvisioned.http_status(), 503);
        assert_eq!(ErrorCode::Timeout.http_status(), 504);
        assert_eq!(ErrorCode::Busy.http_status(), 429);
    }

    #[test]
    fn an_uncompiled_mode_is_an_explicit_unsupported_error() {
        let probe = tpe::backend::probe_pdf().unwrap();
        let options = ScanOptions {
            mode: Mode::Text,
            pages: None,
            max_pages: 10,
        };
        let outcome = scan_bytes(&probe, &options);
        if Mode::Text.compiled() {
            let result = outcome.unwrap();
            assert_eq!(result.pages_total, 1);
            assert_eq!(result.pages_scanned, (1, 1));
            assert!(
                result.pages[0].text.contains("probe"),
                "{:?}",
                result.pages[0]
            );
            assert_eq!(result.backend.name, "docling-text");
        } else {
            let error = outcome.unwrap_err();
            assert_eq!(error.code, ErrorCode::NotCompiled);
            assert!(error.message.contains("--features docling-text"), "{error}");
        }
    }

    #[test]
    fn page_windows_are_bounded_by_max_pages_and_total() {
        if !Mode::Text.compiled() {
            return;
        }
        let probe = tpe::backend::probe_pdf().unwrap();
        let too_many = scan_bytes(
            &probe,
            &ScanOptions {
                mode: Mode::Text,
                pages: None,
                max_pages: 0,
            },
        )
        .unwrap_err();
        assert_eq!(too_many.code, ErrorCode::Limit);
        let out_of_range = scan_bytes(
            &probe,
            &ScanOptions {
                mode: Mode::Text,
                pages: Some((2, 3)),
                max_pages: 10,
            },
        )
        .unwrap_err();
        assert_eq!(out_of_range.code, ErrorCode::PageRange);
        let clamped = scan_bytes(
            &probe,
            &ScanOptions {
                mode: Mode::Text,
                pages: Some((1, 9)),
                max_pages: 10,
            },
        )
        .unwrap();
        assert_eq!(clamped.pages_scanned, (1, 1));
    }
}
