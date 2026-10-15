//! Optional PDF Oxide views, built separately from the original extraction.
//!
//! These APIs allocate upstream-owned vectors before local size checks. Use
//! them only inside the supervised worker for untrusted PDFs. Local budgets
//! complement that boundary; they are not a parser sandbox.

use std::collections::{BTreeMap, HashMap, HashSet};

use pdf_oxide::PdfDocument;
use pdf_oxide::converters::ConversionOptions;
use serde::{Deserialize, Serialize};

use super::pdf_oxide_backend::{
    MAX_INPUT_BYTES, MAX_PAGE_CHARS, MAX_SELECTED_PAGES, PDF_OXIDE_VERSION, finite_box,
};
use super::{BackendError, EncryptionProblem};
use crate::schema::{
    CleanupPreview, ExtractionDiagnostic, ExtractionResult, HtmlPage, MemoryImage, PageSupplements,
    PageText, Span, config_digest, sha256_hex,
};

const MAX_CLEANUP_DOCUMENT_PAGES: usize = 64;
const MAX_HTML_PAGE_BYTES: usize = 1024 * 1024;
const MAX_SUPPLEMENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 1_000_000;
const MAX_TOTAL_IMAGE_PIXELS: u64 = 8_000_000;
const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOTAL_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_PAGE_IMAGES: usize = 32;
const MAX_DOCUMENT_IMAGES: usize = 128;
const CLEANUP_THRESHOLD: f32 = 0.8;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum CleanupMode {
    Whitespace,
    Headers,
    Footers,
    Artifacts,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Options {
    pub cleanup_preview: Option<CleanupMode>,
    pub html: bool,
    pub images: bool,
}

impl Options {
    pub fn enabled(&self) -> bool {
        self.cleanup_preview.is_some() || self.html || self.images
    }
}

#[derive(Default)]
struct Budget {
    bytes: usize,
    images: usize,
    image_pixels: u64,
    image_bytes: usize,
    characters: usize,
    diagnostics: usize,
}

fn limit(message: &str) -> BackendError {
    BackendError::Limit(format!("pdf-oxide supplements: {message}"))
}

impl Budget {
    fn retain(&mut self, bytes: usize) -> Result<(), BackendError> {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > MAX_SUPPLEMENT_BYTES {
            return Err(limit("retained data exceeds 16 MiB"));
        }
        Ok(())
    }

    fn spans(&mut self, document: &PdfDocument, index: usize) -> Result<Vec<Span>, BackendError> {
        let raw = document.extract_spans(index).map_err(upstream)?;
        if raw.len() > MAX_PAGE_CHARS {
            return Err(limit("page spans exceed 100000"));
        }
        let mut spans = Vec::new();
        for (sequence, span) in raw.into_iter().enumerate() {
            self.characters = self.characters.saturating_add(span.text.chars().count());
            if self.characters > 500_000 {
                return Err(limit("preview characters exceed 500000"));
            }
            self.retain(span.text.len().saturating_add(span.font_name.len()))?;
            spans.push(Span {
                text: span.text,
                bbox: Some(finite_box(span.bbox)?),
                font: (!span.font_name.is_empty()).then_some(span.font_name),
                size: (span.font_size.is_finite() && span.font_size > 0.0)
                    .then_some(span.font_size),
                seq: u32::try_from(sequence).map_err(|_| limit("span index overflow"))?,
            });
        }
        Ok(spans)
    }

    fn warnings(
        &mut self,
        document: &PdfDocument,
        stage: &str,
    ) -> Result<Vec<ExtractionDiagnostic>, BackendError> {
        let warnings = document.take_structured_warnings();
        self.diagnostics = self.diagnostics.saturating_add(warnings.len());
        if self.diagnostics > 4096 {
            return Err(limit("diagnostics exceed 4096"));
        }
        warnings
            .into_iter()
            .map(|warning| {
                self.retain(warning.message.len())?;
                Ok(ExtractionDiagnostic {
                    source: source(stage),
                    stage: stage.into(),
                    category: warning.category.as_str().into(),
                    message: warning.message,
                    page: warning
                        .page
                        .and_then(|page| u32::try_from(page).ok()?.checked_add(1)),
                    spec_section: warning.spec_section.map(str::to_owned),
                })
            })
            .collect()
    }
}

fn source(api: &str) -> String {
    format!("pdf-oxide/{PDF_OXIDE_VERSION}/{api}")
}

fn upstream(error: impl std::fmt::Display) -> BackendError {
    BackendError::Unsupported(format!("pdf-oxide requested supplement failed: {error}"))
}

fn open(bytes: &[u8], password: Option<&str>) -> Result<PdfDocument, BackendError> {
    let document = PdfDocument::from_bytes(bytes.to_vec()).map_err(upstream)?;
    if document.is_encrypted() {
        let ok = document
            .authenticate(password.unwrap_or_default().as_bytes())
            .map_err(|_| BackendError::Encrypted(EncryptionProblem::UnsupportedCipher))?;
        if !ok {
            return Err(BackendError::Encrypted(if password.is_some() {
                EncryptionProblem::WrongPassword
            } else {
                EncryptionProblem::PasswordRequired
            }));
        }
    }
    Ok(document)
}

/// Add opt-in views atomically. `bytes` must hash to the original result's PDF.
/// On error, `result` is unchanged. No network, image files, or source mutation.
///
/// The CLI calls this inside its memory/deadline/cancellation boundary. Library
/// callers must provide equivalent process containment for untrusted input.
pub fn add_supplements(
    result: &mut ExtractionResult,
    bytes: &[u8],
    password: Option<&str>,
    options: &Options,
) -> Result<(), BackendError> {
    if !options.enabled() {
        return Ok(());
    }
    if bytes.len() as u64 > MAX_INPUT_BYTES || result.pages.len() > MAX_SELECTED_PAGES as usize {
        return Err(limit("input or selected-page cap exceeded"));
    }
    if result.backend.name != "pdf-oxide"
        || result.document.hash.0 != sha256_hex(bytes)
        || result.pages.iter().any(|page| page.supplements.is_some())
    {
        return Err(BackendError::Unsupported(
            "supplements require a matching, original pdf-oxide result".into(),
        ));
    }
    let mut budget = Budget::default();
    let document = open(bytes, password)?;
    let count = document.page_count().map_err(upstream)?;
    if count != result.document.pages as usize {
        return Err(upstream("page count changed"));
    }
    let mut diagnostics = budget.warnings(&document, "supplements_open")?;
    // Cleanup owns a distinct parser. Erase masks and caches cannot affect
    // the baseline, HTML conversion, image handles, or another request.
    let mut before = BTreeMap::new();
    let mut regions = 0;
    let cleanup_document = if let Some(mode) = options.cleanup_preview {
        if matches!(mode, CleanupMode::Whitespace) {
            None
        } else {
            if count > MAX_CLEANUP_DOCUMENT_PAGES {
                return Err(limit(
                    "header/footer cleanup scans the whole document; at most 64 pages",
                ));
            }
            let clean = open(bytes, password)?;
            for page in &result.pages {
                before.insert(
                    page.page,
                    budget.spans(&clean, checked_index(page.page, count)?)?,
                );
            }
            diagnostics.extend(budget.warnings(&clean, "before_cleanup")?);
            regions = match mode {
                CleanupMode::Headers => clean.remove_headers(CLEANUP_THRESHOLD),
                CleanupMode::Footers => clean.remove_footers(CLEANUP_THRESHOLD),
                CleanupMode::Artifacts => clean.remove_artifacts(CLEANUP_THRESHOLD),
                CleanupMode::Whitespace => unreachable!(),
            }
            .map_err(upstream)?;
            diagnostics.extend(budget.warnings(&clean, "cleanup_scan")?);
            Some(clean)
        }
    } else {
        None
    };
    let mut pending = Vec::new();
    for page in &result.pages {
        let index = checked_index(page.page, count)?;
        let mut views = PageSupplements::default();
        if let Some(mode) = options.cleanup_preview {
            let preview = if let Some(clean) = &cleanup_document {
                let original = before
                    .remove(&page.page)
                    .ok_or_else(|| upstream("missing cleanup baseline"))?;
                let after = budget.spans(clean, index)?;
                diagnostics.extend(budget.warnings(clean, "cleanup_page")?);
                cleanup_preview(page, mode, regions, original, after)?
            } else {
                use pdf_oxide::converters::whitespace::{
                    normalize_horizontal_whitespace, normalize_whitespace,
                };
                CleanupPreview {
                    source: source("normalize_horizontal_whitespace+normalize_whitespace"),
                    operation: "whitespace".into(), threshold: None, document_regions_marked: 0,
                    before_text: page.text.clone(),
                    text: normalize_whitespace(&normalize_horizontal_whitespace(&page.text)),
                    removed_spans: Vec::new(), warnings: vec!["Optional whitespace preview; original text and spans retained. No dehyphenation or page-artifact regex removal.".into()],
                }
            };
            budget.retain(preview.before_text.len().saturating_add(preview.text.len()))?;
            views.cleanup = Some(preview);
        }
        if options.html {
            // No user/document path or URL enters conversion options. Explicitly
            // disable images; image extraction has its own budgets and errors.
            let conversion = ConversionOptions {
                preserve_layout: false,
                include_images: false,
                image_output_dir: None,
                embed_images: false,
                max_image_pixels: Some(MAX_IMAGE_PIXELS),
                strip_running_headers_footers: false,
                ..ConversionOptions::default()
            };
            let raw_html = document.to_html(index, &conversion).map_err(upstream)?;
            if raw_html.len() > MAX_HTML_PAGE_BYTES {
                return Err(limit("HTML page exceeds 1 MiB"));
            }
            let page_id = format!("pdf-{}-page-{}", result.document.hash.0, page.page);
            let display_html = wrap_html(&page_id, &raw_html);
            if display_html.len() > 2 * MAX_HTML_PAGE_BYTES {
                return Err(limit("sanitized HTML page exceeds 2 MiB"));
            }
            budget.retain(raw_html.len().saturating_add(display_html.len()))?;
            views.html = Some(HtmlPage {
                source: source("to_html"), page_id, raw_html, display_html,
                sanitizer: "ammonia/4.1.2; structural tags only; no attributes, URLs, images, CSS, SVG or forms".into(),
            });
            diagnostics.extend(budget.warnings(&document, "to_html")?);
        }
        if options.images {
            views.images = images(&document, index, &mut budget)?;
            diagnostics
                .extend(budget.warnings(&document, "page_image_handles/decode/to_png_bytes")?);
            diagnostics.push(ExtractionDiagnostic {
                source: source("page_image_handles"), stage: "images".into(), category: "completeness_unverified".into(),
                message: "Handle enumeration can silently omit malformed images; an empty list does not establish an image-free page. Requested decode/PNG errors fail the job.".into(),
                page: Some(page.page), spec_section: None,
            });
        }
        pending.push(views);
    }
    let serialized_options = serde_json::to_string(options).map_err(upstream)?;
    let identity = config_digest(&BTreeMap::from([
        ("base".into(), result.backend.config_digest.clone()),
        ("pdf_oxide_supplements_v1".into(), serialized_options),
    ]));
    for (page, views) in result.pages.iter_mut().zip(pending) {
        page.supplements = Some(views);
    }
    // Document-scoped diagnostics remain None; don't guess a source page.
    if let Some(page) = result.pages.first_mut() {
        page.diagnostics.extend(diagnostics);
    }
    result.backend.config_digest = identity;
    Ok(())
}

fn checked_index(page: u32, count: usize) -> Result<usize, BackendError> {
    let index = page.checked_sub(1).ok_or(BackendError::PageRange {
        page,
        count: u32::try_from(count).unwrap_or(u32::MAX),
    })? as usize;
    if index >= count {
        return Err(upstream("selected page is outside document"));
    }
    Ok(index)
}

fn ordered_text(page: &PageText, spans: Vec<Span>) -> String {
    let mut view = PageText::new(page.page, page.width, page.height, page.rotation);
    view.spans = spans;
    crate::reading_order::order_page(&mut view);
    view.text
}

fn cleanup_preview(
    page: &PageText,
    mode: CleanupMode,
    regions: usize,
    before: Vec<Span>,
    after: Vec<Span>,
) -> Result<CleanupPreview, BackendError> {
    // Counts preserve duplicate meaningful spans. Comparing only text would
    // conceal removal of a header whose wording also appears in the body.
    let key = |span: &Span| {
        serde_json::to_string(&(&span.text, span.bbox, &span.font, span.size)).map_err(upstream)
    };
    let mut remaining = HashMap::<String, usize>::new();
    for span in &after {
        *remaining.entry(key(span)?).or_default() += 1;
    }
    let mut removed_spans = Vec::new();
    for span in &before {
        let count = remaining.entry(key(span)?).or_default();
        if *count == 0 {
            removed_spans.push(span.clone());
        } else {
            *count -= 1;
        }
    }
    Ok(CleanupPreview {
        source: source("remove_headers/remove_footers+extract_spans"),
        operation: format!("{mode:?}").to_lowercase(), threshold: Some(CLEANUP_THRESHOLD),
        document_regions_marked: u32::try_from(regions).map_err(|_| limit("cleanup region count overflow"))?,
        before_text: ordered_text(page, before), text: ordered_text(page, after), removed_spans,
        warnings: vec!["Candidate preview only: repeated prose/table headers/citations can be removed. Inspect removed_spans; original extraction is retained. Mixed tagged/untagged documents may be only partly cleaned. Erase masks do not redact or rewrite the source PDF.".into()],
    })
}

pub(crate) fn safe_page_html(sha256: &str, page: u32, raw: &str) -> Result<String, BackendError> {
    if sha256.len() != 64
        || !sha256
            .bytes()
            .all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch))
        || page == 0
    {
        return Err(upstream("invalid application-owned HTML page identity"));
    }
    if raw.len() > 1024 * 1024 {
        return Err(limit("raw page HTML exceeds 1 MiB"));
    }
    let wrapped = wrap_html(&format!("pdf-{sha256}-page-{page}"), raw);
    if wrapped.len() > 2 * 1024 * 1024 {
        return Err(limit("sanitized page HTML exceeds 2 MiB"));
    }
    Ok(wrapped)
}

fn wrap_html(page_id: &str, raw: &str) -> String {
    let tags: HashSet<&str> = [
        "p",
        "br",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "strong",
        "em",
        "b",
        "i",
        "u",
        "s",
        "sub",
        "sup",
        "pre",
        "code",
        "blockquote",
        "ul",
        "ol",
        "li",
        "table",
        "thead",
        "tbody",
        "tfoot",
        "tr",
        "th",
        "td",
        "caption",
    ]
    .into_iter()
    .collect();
    let sanitized = ammonia::Builder::default()
        .tags(tags)
        .tag_attributes(HashMap::new())
        .generic_attributes(HashSet::new())
        .url_schemes(HashSet::new())
        .link_rel(None)
        .clean(raw)
        .to_string();
    // page_id is constructed only from a verified lowercase hex digest and u32.
    format!("<section class=\"pdf-page\" id=\"{page_id}\">{sanitized}</section>")
}

fn images(
    document: &PdfDocument,
    index: usize,
    budget: &mut Budget,
) -> Result<Vec<MemoryImage>, BackendError> {
    super::pdf_oxide_images::reject_unsupported_masks(document, index)?;
    let handles = document.page_image_handles(index).map_err(upstream)?;
    budget.images = budget.images.saturating_add(handles.len());
    if handles.len() > MAX_PAGE_IMAGES || budget.images > MAX_DOCUMENT_IMAGES {
        return Err(limit("images exceed 32/page or 128/document"));
    }
    let mut images = Vec::new();
    for handle in handles {
        let pixels = u64::from(handle.width) * u64::from(handle.height);
        budget.image_pixels = budget.image_pixels.saturating_add(pixels);
        if pixels == 0 || pixels > MAX_IMAGE_PIXELS || budget.image_pixels > MAX_TOTAL_IMAGE_PIXELS
        {
            return Err(limit(
                "image pixels exceed 1 million/image or 8 million/document (or are zero)",
            ));
        }
        if handle.byte_size_compressed > MAX_IMAGE_BYTES as u64 {
            return Err(limit("compressed image exceeds 2 MiB"));
        }
        let decoded = handle.decode().map_err(upstream)?;
        if decoded.width() != handle.width || decoded.height() != handle.height {
            return Err(upstream(
                "decoded image dimensions differ from inspected dimensions",
            ));
        }
        let bytes = decoded.to_png_bytes().map_err(upstream)?;
        budget.image_bytes = budget.image_bytes.saturating_add(bytes.len());
        if bytes.len() > MAX_IMAGE_BYTES || budget.image_bytes > MAX_TOTAL_IMAGE_BYTES {
            return Err(limit(
                "encoded PNG bytes exceed 2 MiB/image or 8 MiB/document",
            ));
        }
        // Encoded JPEG dimensions can disagree with the PDF dictionary. Verify
        // the actual PNG envelope after decoding; process limits bound the work.
        if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
            return Err(upstream("encoder did not produce a PNG"));
        }
        let width = u32::from_be_bytes(bytes[16..20].try_into().map_err(upstream)?);
        let height = u32::from_be_bytes(bytes[20..24].try_into().map_err(upstream)?);
        if width != handle.width || height != handle.height {
            return Err(upstream("PNG dimensions differ from inspected image"));
        }
        budget.retain(bytes.len())?;
        images.push(MemoryImage {
            source: source("page_image_handles/decode/to_png_bytes"),
            paint_order: u32::try_from(handle.paint_order)
                .map_err(|_| limit("image index overflow"))?,
            inline: handle.is_inline,
            bbox: finite_box(handle.bbox)?,
            width,
            height,
            mime: "image/png".into(),
            sha256: sha256_hex(&bytes),
            source_filters: handle
                .filter_chain
                .iter()
                .map(|filter| format!("{filter:?}"))
                .collect(),
            source_compressed_bytes: handle.byte_size_compressed,
            bytes,
        });
    }
    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::wrap_html;

    #[test]
    fn display_html_has_no_active_attributes_or_resource_urls() {
        let raw = r#"<script>alert(1)</script><style>@import 'https://invalid.test';</style><p onclick="alert(1)">Body</p><img src="http://127.0.0.1" onerror="alert(1)"><a href="javascript:alert(1)">Link</a><svg><a xlink:href="https://invalid.test">SVG</a></svg><form><input autofocus onfocus="alert(1)"></form><table><tr><td style="background:url(https://invalid.test)">Cell</td></tr></table>"#;
        let html = wrap_html("pdf-0123-page-1", raw);
        for forbidden in [
            "<script",
            "<style",
            "<img",
            "<svg",
            "<form",
            "<input",
            "onclick",
            "onerror",
            "onfocus",
            "href=",
            "src=",
            "style=",
            "http:",
            "https:",
            "javascript:",
        ] {
            assert!(!html.contains(forbidden), "{html}");
        }
        assert!(html.contains("<p>Body</p>"));
        assert!(html.contains("<td>Cell</td>"));
        assert!(html.starts_with("<section class=\"pdf-page\" id=\"pdf-0123-page-1\">"));
    }
}
