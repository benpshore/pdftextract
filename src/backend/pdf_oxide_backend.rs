//! Independent positioned-character extraction through pinned PDF Oxide.
//!
//! `extract_chars` retains unmapped U+FFFD glyphs without changing PDF Oxide's
//! process-global high-level span filtering switch. Positions and font metrics
//! come from that parser; normal engine reading order remains responsible for
//! assembly. The upstream character API still has silent content/font fallback
//! paths, so absence of a diagnostic cannot certify complete extraction.

use std::collections::BTreeMap;

use pdf_oxide::PdfDocument;
use pdf_oxide::extractors::warnings::Warning;
use pdf_oxide::object::Object;
use pdf_oxide::structure::table_extractor::Table as OxideTable;

use crate::backend::{BackendError, DocumentSession, EncryptionProblem, Extractor};
use crate::schema::{
    BBox, BackendIdentity, ExtractionDiagnostic, Link, PageText, Span, Table, TableCell, TableRow,
    config_digest,
};

pub(super) const PDF_OXIDE_VERSION: &str = "0.3.78";
/// Hard ceilings complement, but cannot replace, the CLI's worker containment.
pub const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_SELECTED_PAGES: u32 = 256;
pub(super) const MAX_PAGE_CHARS: usize = 100_000;
const MAX_DOCUMENT_CHARS: usize = 500_000;
const MAX_TABLES: usize = 64;
const MAX_PAGE_CELLS: usize = 16_384;
const MAX_DOCUMENT_CELLS: usize = 65_536;
const MAX_STRING_BYTES: usize = 65_536;
const MAX_RETAINED_BYTES: usize = 16 * 1024 * 1024;
const MAX_DIAGNOSTICS: usize = 4096;
const MAX_ANNOTATIONS: usize = 4096;
const UNVERIFIED: &str = "extraction_incomplete: pdf-oxide character extraction retains upstream fallback uncertainty; completeness not independently verified";

/// Optional independent native parser; no OCR, rendering, or external programs.
#[derive(Clone, Copy, Debug, Default)]
pub struct PdfOxideBackend;

impl Extractor for PdfOxideBackend {
    fn max_input_bytes(&self) -> Option<u64> {
        Some(MAX_INPUT_BYTES)
    }

    fn identity(&self) -> BackendIdentity {
        BackendIdentity {
            name: "pdf-oxide".into(),
            version: PDF_OXIDE_VERSION.into(),
            config_digest: config_digest(&BTreeMap::from([
                ("adapter".into(), "2".into()),
                ("api".into(), "extract_chars+extract_tables".into()),
                (
                    "limits".into(),
                    "input64MiB/pages256/chars500000/cells65536/strings16MiB".into(),
                ),
                (
                    "links".into(),
                    "pdf_oxide_objects_bounded_uri_targets".into(),
                ),
                ("mapping_status".into(), "unverified_partial".into()),
                ("upstream_default_features".into(), "disabled".into()),
            ])),
        }
    }

    fn open(
        &self,
        bytes: &[u8],
        password: Option<&str>,
    ) -> Result<Box<dyn DocumentSession>, BackendError> {
        if bytes.len() as u64 > MAX_INPUT_BYTES {
            return Err(limit("input exceeds 64 MiB"));
        }
        let document = PdfDocument::from_bytes(bytes.to_vec())
            .map_err(|error| BackendError::Malformed(format!("pdf-oxide: {error}")))?;
        if document.is_encrypted() {
            let authenticated = document
                .authenticate(password.unwrap_or_default().as_bytes())
                .map_err(|_| BackendError::Encrypted(EncryptionProblem::UnsupportedCipher))?;
            if !authenticated {
                return Err(BackendError::Encrypted(if password.is_some() {
                    EncryptionProblem::WrongPassword
                } else {
                    EncryptionProblem::PasswordRequired
                }));
            }
        }
        let count = document
            .page_count()
            .map_err(|error| BackendError::Malformed(format!("pdf-oxide page tree: {error}")))?;
        let count = u32::try_from(count)
            .map_err(|_| BackendError::Limit("pdf-oxide page count exceeds u32".into()))?;
        if count == 0 {
            return Err(BackendError::PageRange { page: 1, count });
        }
        let opening_warnings = document.take_structured_warnings();
        Ok(Box::new(PdfOxideSession {
            document,
            count,
            opening_warnings,
            budget: Budget::default(),
        }))
    }
}

struct PdfOxideSession {
    document: PdfDocument,
    count: u32,
    opening_warnings: Vec<Warning>,
    budget: Budget,
}

#[derive(Default)]
struct Budget {
    pages: u32,
    chars: usize,
    cells: usize,
    mcids: usize,
    bytes: usize,
    diagnostics: usize,
}

fn limit(message: &str) -> BackendError {
    BackendError::Limit(format!("pdf-oxide {message}"))
}

impl Budget {
    fn string(&mut self, value: &str) -> Result<(), BackendError> {
        if value.len() > MAX_STRING_BYTES {
            return Err(limit("retained string exceeds 64 KiB"));
        }
        self.bytes = self.bytes.saturating_add(value.len());
        if self.bytes > MAX_RETAINED_BYTES {
            return Err(limit("retained strings exceed 16 MiB"));
        }
        Ok(())
    }

    fn warnings(
        &mut self,
        warnings: Vec<Warning>,
        stage: &str,
        output: &mut PageText,
    ) -> Result<(), BackendError> {
        self.diagnostics = self.diagnostics.saturating_add(warnings.len());
        if self.diagnostics > MAX_DIAGNOSTICS {
            return Err(limit("structured diagnostics exceed 4096"));
        }
        for warning in warnings {
            self.string(&warning.message)?;
            output.warnings.push(format_warning(&warning));
            output.diagnostics.push(ExtractionDiagnostic {
                source: format!("pdf-oxide/{PDF_OXIDE_VERSION}"),
                stage: stage.into(),
                category: warning.category.as_str().into(),
                message: warning.message,
                page: warning
                    .page
                    .and_then(|page| u32::try_from(page).ok()?.checked_add(1)),
                spec_section: warning.spec_section.map(str::to_owned),
            });
        }
        Ok(())
    }

    fn tables(&mut self, tables: Vec<OxideTable>) -> Result<Vec<Table>, BackendError> {
        if tables.len() > MAX_TABLES {
            return Err(limit("tables per page exceed 64"));
        }
        let mut page_cells = 0_usize;
        let mut result = Vec::new();
        for table in tables {
            if table.rows.len() > MAX_PAGE_CELLS || table.col_count > MAX_PAGE_CELLS {
                return Err(limit("table dimensions exceed 16384"));
            }
            let mut rows = Vec::new();
            for row in table.rows {
                page_cells = page_cells.saturating_add(row.cells.len());
                self.cells = self.cells.saturating_add(row.cells.len());
                if page_cells > MAX_PAGE_CELLS || self.cells > MAX_DOCUMENT_CELLS {
                    return Err(limit("table cells exceed page/document budget"));
                }
                let mut cells = Vec::new();
                for cell in row.cells {
                    self.string(&cell.text)?;
                    if cell.mcids.len() > MAX_PAGE_CELLS {
                        return Err(limit("cell marked-content IDs exceed 16384"));
                    }
                    self.mcids = self.mcids.saturating_add(cell.mcids.len());
                    if self.mcids > MAX_DOCUMENT_CELLS {
                        return Err(limit("document table marked-content IDs exceed 65536"));
                    }
                    cells.push(TableCell {
                        text: cell.text,
                        bbox: cell.bbox.map(finite_box).transpose()?,
                        is_header: cell.is_header,
                        colspan: cell.colspan,
                        rowspan: cell.rowspan,
                        mcids: cell.mcids,
                    });
                }
                rows.push(TableRow {
                    cells,
                    is_header: row.is_header,
                });
            }
            result.push(Table {
                source: format!("pdf-oxide/{PDF_OXIDE_VERSION}/extract_tables"),
                bbox: table.bbox.map(finite_box).transpose()?,
                has_header: table.has_header,
                col_count: u32::try_from(table.col_count)
                    .map_err(|_| limit("table columns exceed u32"))?,
                rows,
            });
        }
        Ok(result)
    }
}

pub(super) fn finite_box(rect: pdf_oxide::geometry::Rect) -> Result<BBox, BackendError> {
    let bbox = BBox {
        x0: rect.x,
        y0: rect.y,
        x1: rect.x + rect.width,
        y1: rect.y + rect.height,
    };
    if [bbox.x0, bbox.y0, bbox.x1, bbox.y1]
        .iter()
        .all(|n| n.is_finite())
        && bbox.x1 >= bbox.x0
        && bbox.y1 >= bbox.y0
    {
        Ok(bbox)
    } else {
        Err(BackendError::Malformed(
            "pdf-oxide returned invalid table geometry".into(),
        ))
    }
}

fn page_error(page: u32, error: impl std::fmt::Display) -> BackendError {
    BackendError::Page {
        page,
        message: format!("pdf-oxide: {error}"),
    }
}

fn format_warning(warning: &Warning) -> String {
    format!(
        "extraction_incomplete: pdf-oxide {}: {}",
        warning.category.as_str(),
        warning.message
    )
}

fn resolve(document: &PdfDocument, object: &Object) -> Result<Object, String> {
    let mut object = object.clone();
    for _ in 0..8 {
        if let Object::Reference(reference) = object {
            object = document
                .load_object(reference)
                .map_err(|error| error.to_string())?;
        } else {
            return Ok(object);
        }
    }
    Err("indirect object depth exceeds 8".into())
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn rectangle(document: &PdfDocument, object: &Object) -> Result<BBox, String> {
    let Object::Array(values) = resolve(document, object)? else {
        return Err("rectangle is not an array".into());
    };
    if values.len() != 4 {
        return Err("rectangle does not contain four coordinates".into());
    }
    let mut coordinates = [0.0_f32; 4];
    for (coordinate, value) in coordinates.iter_mut().zip(values) {
        let number = match resolve(document, &value)? {
            Object::Integer(number) => number as f64,
            Object::Real(number) => number,
            _ => return Err("rectangle coordinate is not numeric".into()),
        };
        // The engine stores f32 coordinates; reject overflow instead of inventing a box.
        if !number.is_finite() || number.abs() > f64::from(f32::MAX) {
            return Err("rectangle coordinate is not finite".into());
        }
        *coordinate = number as f32;
    }
    Ok(BBox {
        x0: coordinates[0].min(coordinates[2]),
        y0: coordinates[1].min(coordinates[3]),
        x1: coordinates[0].max(coordinates[2]),
        y1: coordinates[1].max(coordinates[3]),
    })
}

fn uri_link(document: &PdfDocument, object: &Object) -> Result<Option<Link>, String> {
    let annotation = resolve(document, object)?;
    let dictionary = annotation
        .as_dict()
        .ok_or("annotation is not a dictionary")?;
    if dictionary.get("Subtype").and_then(Object::as_name) != Some("Link") {
        return Ok(None);
    }
    let Some(action) = dictionary.get("A") else {
        return Ok(None);
    };
    let action = resolve(document, action)?;
    let action = action.as_dict().ok_or("link action is not a dictionary")?;
    if action.get("S").and_then(Object::as_name) != Some("URI") {
        return Ok(None);
    }
    let target = action.get("URI").ok_or("URI action has no target")?;
    let Object::String(target) = resolve(document, target)? else {
        return Err("URI target is not a string".into());
    };
    let uri = String::from_utf8(target).map_err(|_| "URI target is not valid UTF-8")?;
    let bbox = dictionary
        .get("Rect")
        .and_then(|value| rectangle(document, value).ok());
    Ok(Some(Link { bbox, uri }))
}

fn collect_links(document: &PdfDocument, page: &Object, output: &mut PageText) {
    let Some(annotations) = page
        .as_dict()
        .and_then(|dictionary| dictionary.get("Annots"))
    else {
        return;
    };
    let Ok(Object::Array(annotations)) = resolve(document, annotations) else {
        output
            .warnings
            .push("extraction_incomplete: pdf-oxide Annots could not be resolved".into());
        return;
    };
    if annotations.len() > MAX_ANNOTATIONS {
        output
            .warnings
            .push("resource_limit: pdf-oxide annotation count exceeds 4096".into());
    }
    for annotation in annotations.iter().take(MAX_ANNOTATIONS) {
        match uri_link(document, annotation) {
            Ok(Some(link)) => output.links.push(link),
            Ok(None) => {}
            Err(error) => output.warnings.push(format!(
                "extraction_incomplete: pdf-oxide annotation: {error}"
            )),
        }
    }
}

impl DocumentSession for PdfOxideSession {
    fn page_count(&self) -> u32 {
        self.count
    }

    fn max_selected_pages(&self) -> Option<u32> {
        Some(MAX_SELECTED_PAGES)
    }

    fn page_text(&mut self, page: u32) -> Result<PageText, BackendError> {
        if page == 0 || page > self.count {
            return Err(BackendError::PageRange {
                page,
                count: self.count,
            });
        }
        self.budget.pages = self.budget.pages.saturating_add(1);
        if self.budget.pages > MAX_SELECTED_PAGES {
            return Err(limit("page extraction calls exceed 256"));
        }
        let index = usize::try_from(page - 1).map_err(|error| page_error(page, error))?;
        let source_page = self
            .document
            .get_page(index)
            .map_err(|error| page_error(page, error))?;
        let media_box = source_page
            .as_dict()
            .and_then(|dictionary| dictionary.get("MediaBox"))
            .ok_or_else(|| page_error(page, "no MediaBox"))?;
        let bbox = rectangle(&self.document, media_box).map_err(|error| page_error(page, error))?;
        let (width, height) = (bbox.x1 - bbox.x0, bbox.y1 - bbox.y0);
        if ![width, height].iter().all(|n| n.is_finite()) || width <= 0.0 || height <= 0.0 {
            return Err(page_error(page, "invalid or unavailable page geometry"));
        }
        let rotation = self
            .document
            .get_page_rotation(index)
            .map_err(|error| page_error(page, error))?;
        let mut output = PageText::new(page, width, height, rotation);
        output.warnings.push(UNVERIFIED.into());
        self.budget.diagnostics = self.budget.diagnostics.saturating_add(1);
        if self.budget.diagnostics > MAX_DIAGNOSTICS {
            return Err(limit("structured diagnostics exceed 4096"));
        }
        output.diagnostics.push(ExtractionDiagnostic {
            source: "pdf_oxide/0.3.78".into(),
            stage: "extract_chars".into(),
            category: "actual_text_character_mapping_unverified".into(),
            message: "Pinned character extraction can remap non-ASCII ActualText through the font and corrupt Unicode; native character geometry is retained, and span-derived queries provide a separate representation. This diagnostic identifies a known API limitation, not detection of ActualText on this page.".into(),
            page: Some(page),
            spec_section: None,
        });
        self.budget.warnings(
            std::mem::take(&mut self.opening_warnings),
            "open",
            &mut output,
        )?;
        collect_links(&self.document, &source_page, &mut output);
        for link in &output.links {
            self.budget.string(&link.uri)?;
        }
        let chars = self
            .document
            .extract_chars(index)
            .map_err(|error| page_error(page, error))?;
        self.budget.chars = self.budget.chars.saturating_add(chars.len());
        if chars.len() > MAX_PAGE_CHARS || self.budget.chars > MAX_DOCUMENT_CHARS {
            return Err(limit("characters exceed page/document budget"));
        }
        for (sequence, ch) in chars.into_iter().enumerate() {
            self.budget.string(&ch.font_name)?;
            let seq = u32::try_from(sequence)
                .map_err(|_| page_error(page, "character count exceeds u32"))?;
            let bbox = BBox {
                x0: ch.bbox.x,
                y0: ch.bbox.y,
                x1: ch.bbox.x + ch.bbox.width,
                y1: ch.bbox.y + ch.bbox.height,
            };
            let bbox = if [bbox.x0, bbox.y0, bbox.x1, bbox.y1]
                .iter()
                .all(|n| n.is_finite())
                && bbox.x1 >= bbox.x0
                && bbox.y1 >= bbox.y0
            {
                Some(bbox)
            } else {
                output.warnings.push("extraction_incomplete: pdf-oxide invalid glyph geometry; text retained without coordinates".into());
                None
            };
            output.spans.push(Span {
                text: ch.char.to_string(),
                bbox,
                font: (!ch.font_name.is_empty()).then_some(ch.font_name),
                size: (ch.font_size.is_finite() && ch.font_size > 0.0).then_some(ch.font_size),
                seq,
            });
        }
        if output
            .spans
            .iter()
            .any(|span| span.text.contains('\u{fffd}'))
        {
            output.warnings.push("unicode_mapping: pdf-oxide emitted unresolved glyph mappings; replacement characters retained".into());
        }
        self.budget.warnings(
            self.document.take_structured_warnings(),
            "extract_chars",
            &mut output,
        )?;
        if output.spans.is_empty() {
            output.warnings.push("extraction_incomplete: pdf-oxide returned no selectable characters; the page may be blank, damaged, or require OCR; OCR was not run".into());
        }
        match self.document.extract_tables(index) {
            Ok(tables) => output.tables = self.budget.tables(tables)?,
            Err(error) => output.warnings.push(format!(
                "extraction_incomplete: pdf-oxide table extraction failed: {error}"
            )),
        }
        self.budget.warnings(
            self.document.take_structured_warnings(),
            "extract_tables",
            &mut output,
        )?;
        Ok(output)
    }

    fn info(&self) -> BTreeMap<String, String> {
        [
            ("Producer", self.document.document_producer()),
            ("Creator", self.document.document_creator()),
        ]
        .into_iter()
        .filter_map(|(name, value)| value.map(|value| (name.into(), value)))
        .collect()
    }
}
