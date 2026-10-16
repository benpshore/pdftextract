//! Selected-page projections over the exact pinned native converters.
//!
//! Returned converter strings are evidence, not trusted markup. HTML has a
//! separate display projection. All optional filesystem/image paths are off.

use std::io::{self, Write};

use pdf_oxide::PdfDocument;
use pdf_oxide::converters::{ConversionOptions, ReadingOrderMode};
use pdf_oxide::pipeline::BoldMarkerBehavior;
use serde::Serialize;
use serde_json::{Value, json};

use super::{Context, OperationOutput};
use crate::backend::BackendError;
use crate::backend::pdf_oxide_supplements::safe_page_html;

pub const OPERATIONS: &[&str] = &["convert.text", "convert.markdown", "convert.html"];
const SOURCE: &str = "pdf-oxide/0.3.78";
const MAX_PAGE_BYTES: usize = 1024 * 1024;

pub(super) fn execute(
    operation: &str,
    context: &Context<'_>,
    params: &Value,
) -> Option<Result<OperationOutput, BackendError>> {
    OPERATIONS
        .contains(&operation)
        .then(|| run(operation, context, params))
}

fn run(
    operation: &str,
    context: &Context<'_>,
    params: &Value,
) -> Result<OperationOutput, BackendError> {
    if context.inputs.len() != 1 {
        return Err(invalid("conversion requires exactly one PDF input"));
    }
    let baseline_options = options(operation, &json!({}), context.limits.max_pixels)?;
    let effective_options = options(operation, params, context.limits.max_pixels)?;
    let baseline = context.open_pdf()?;
    let count = baseline
        .page_count()
        .map_err(|error| BackendError::Malformed(error.to_string()))?;
    let selected = context.selected_pages(count)?;
    // Separate parser caches keep option-dependent native observations from
    // changing the baseline. Identical options reuse the exact baseline value.
    let changed = effective_options != baseline_options;
    let requested = changed.then(|| context.open_pdf()).transpose()?;
    let mut budget = Budget {
        bytes: context.limits.max_output_bytes,
        items: context.limits.max_items,
    };
    let mut data = json!({
        "source": SOURCE,
        "source_sha256": context.inputs[0].sha256,
        "operation": operation,
        "parameters": params,
        "source_page_count": count,
        "page_numbering": "one-based",
        "completeness": "unverified",
        "baseline": "same native converter with the recorded baseline options; not original PDF text bytes",
        "baseline_options": provenance(&baseline_options),
        "effective_options": provenance(&effective_options),
        "options_changed": changed,
        "method": match operation {
            "convert.text" => "PdfDocument::to_plain_text",
            "convert.markdown" => "PdfDocument::to_markdown",
            _ => "PdfDocument::to_html",
        },
        "images": "omitted by explicit conversion policy",
        "table_policy": "reconstruction disabled; unqualified native candidates are evaluation evidence only; extract_tables=true is unsupported",
        "raw_markup": "untrusted native projection; only display_html is sanitized",
    });
    budget.retain(&data)?;
    let mut diagnostics = Vec::new();
    drain(&baseline, "baseline.open", &mut diagnostics, &mut budget)?;
    if let Some(document) = &requested {
        drain(document, "requested.open", &mut diagnostics, &mut budget)?;
    }
    let mut pages = Vec::with_capacity(selected.len());
    for index in selected {
        let page = u32::try_from(index + 1)
            .map_err(|_| BackendError::Limit("page number exceeds u32".into()))?;
        let original = project(operation, &baseline, index, &baseline_options)?;
        check_text(&original, context.limits.max_output_bytes)?;
        drain(&baseline, "baseline.convert", &mut diagnostics, &mut budget)?;
        let converted = if let Some(document) = &requested {
            let output = project(operation, document, index, &effective_options)?;
            check_text(&output, context.limits.max_output_bytes)?;
            drain(document, "requested.convert", &mut diagnostics, &mut budget)?;
            output
        } else {
            original.clone()
        };
        if converted.trim().is_empty() {
            let diagnostic = json!({
                "source": SOURCE, "stage": "convert", "page": page,
                "category": "no_text_result_unverified",
                "message": "Empty converter output is not proof of an empty page; image-only content and upstream recovery may omit text.",
            });
            budget.item(&diagnostic)?;
            diagnostics.push(diagnostic);
        }
        let value = if operation == "convert.html" {
            let display = safe_page_html(&context.inputs[0].sha256, page, &converted)?;
            json!({
                "page": page, "page_id": format!("pdf-{}-page-{page}", context.inputs[0].sha256),
                "baseline_raw_html": original, "raw_html": converted, "display_html": display,
                "sanitizer": "shared structural-tag allowlist; no source attributes, active tags, styles, URLs or resources",
            })
        } else {
            json!({"page": page, "baseline": original, "converted": converted})
        };
        budget.item(&value)?;
        pages.push(value);
    }
    data["pages"] = Value::Array(pages);
    data["diagnostics"] = Value::Array(diagnostics);
    Ok(OperationOutput {
        data,
        warnings: vec![
            "Conversion completeness, reading order and formatting remain unverified; table reconstruction is disabled. Source bytes and baseline projections are retained. Raw HTML/Markdown must not be rendered as trusted content.".into(),
        ],
        ..OperationOutput::default()
    })
}

fn project(
    operation: &str,
    document: &PdfDocument,
    page: usize,
    options: &ConversionOptions,
) -> Result<String, BackendError> {
    match operation {
        "convert.text" => document.to_plain_text(page, options),
        "convert.markdown" => document.to_markdown(page, options),
        "convert.html" => document.to_html(page, options),
        _ => unreachable!("dispatcher filters operations"),
    }
    .map_err(|error| BackendError::Page {
        page: u32::try_from(page + 1).unwrap_or(u32::MAX),
        message: format!("{SOURCE} {operation}: {error}"),
    })
}

fn check_text(text: &str, max_output: u64) -> Result<(), BackendError> {
    if text.len() > MAX_PAGE_BYTES || text.len() as u64 > max_output {
        return Err(BackendError::Limit(
            "native page conversion exceeds 1 MiB or the configured output limit".into(),
        ));
    }
    Ok(())
}

fn options(
    operation: &str,
    params: &Value,
    max_pixels: u64,
) -> Result<ConversionOptions, BackendError> {
    let object = params
        .as_object()
        .ok_or_else(|| invalid("conversion parameters must be an object"))?;
    for key in object.keys() {
        let allowed = match key.as_str() {
            "reading_order" | "detect_headings" | "extract_tables" | "include_form_fields" => true,
            "preserve_layout" => operation == "convert.html",
            "bold_marker_behavior" => operation == "convert.markdown",
            _ => false,
        };
        if !allowed {
            return Err(invalid(&format!(
                "unsupported {operation} parameter: {key}"
            )));
        }
    }
    let reading_order_mode = match string(params, "reading_order", "structure")? {
        "structure" => ReadingOrderMode::StructureTreeFirst { mcid_order: vec![] },
        "column_aware" => ReadingOrderMode::ColumnAware,
        "top_to_bottom" => ReadingOrderMode::TopToBottomLeftToRight,
        _ => {
            return Err(invalid(
                "reading_order must be structure, column_aware or top_to_bottom",
            ));
        }
    };
    let bold_marker_behavior = match string(params, "bold_marker_behavior", "conservative")? {
        "conservative" => BoldMarkerBehavior::Conservative,
        "aggressive" => BoldMarkerBehavior::Aggressive,
        _ => {
            return Err(invalid(
                "bold_marker_behavior must be conservative or aggressive",
            ));
        }
    };
    if boolean(params, "extract_tables", false)? {
        return Err(invalid(
            "table reconstruction is unsupported: pinned native candidates can split wrapped cells, omit headers/blank rows, or misclassify prose; native table evaluation is not a validated conversion",
        ));
    }
    Ok(ConversionOptions {
        preserve_layout: boolean(params, "preserve_layout", false)?,
        detect_headings: boolean(params, "detect_headings", true)?,
        extract_tables: false,
        include_form_fields: boolean(params, "include_form_fields", true)?,
        reading_order_mode,
        bold_marker_behavior,
        include_images: false,
        image_output_dir: None,
        embed_images: false,
        strip_running_headers_footers: false,
        table_detection_config: None,
        render_formulas: false,
        page_images: None,
        page_dimensions: None,
        max_image_pixels: Some(max_pixels),
        exclude_regions: Vec::new(),
        exclude_regions_mode: pdf_oxide::layout::RectFilterMode::Intersects,
        include_region: None,
        expand_ligatures: false,
        include_artifacts: true,
        ..ConversionOptions::default()
    })
}

fn provenance(options: &ConversionOptions) -> Value {
    json!({
        "preserve_layout": options.preserve_layout,
        "detect_headings": options.detect_headings,
        "extract_tables": options.extract_tables,
        "include_form_fields": options.include_form_fields,
        "reading_order": match options.reading_order_mode {
            ReadingOrderMode::StructureTreeFirst { .. } => "structure",
            ReadingOrderMode::ColumnAware => "column_aware",
            ReadingOrderMode::TopToBottomLeftToRight => "top_to_bottom",
        },
        "bold_marker_behavior": match options.bold_marker_behavior {
            BoldMarkerBehavior::Conservative => "conservative",
            BoldMarkerBehavior::Aggressive => "aggressive",
        },
        "include_images": false, "image_output_dir": null, "embed_images": false,
        "strip_running_headers_footers": false, "table_detection_config": null,
        "render_formulas": false, "page_images": null, "page_dimensions": null,
        "max_image_pixels": options.max_image_pixels,
        "exclude_regions": [], "exclude_regions_mode": "intersects", "include_region": null,
        "expand_ligatures": false, "include_artifacts": true,
        "annotate_skipped_pages": "deprecated and ignored in 0.3.78",
    })
}

fn boolean(params: &Value, name: &str, default: bool) -> Result<bool, BackendError> {
    params.get(name).map_or(Ok(default), |value| {
        value
            .as_bool()
            .ok_or_else(|| invalid(&format!("{name} must be boolean")))
    })
}

fn string<'a>(params: &'a Value, name: &str, default: &'a str) -> Result<&'a str, BackendError> {
    params.get(name).map_or(Ok(default), |value| {
        value
            .as_str()
            .ok_or_else(|| invalid(&format!("{name} must be a string")))
    })
}

fn invalid(message: &str) -> BackendError {
    BackendError::Unsupported(message.into())
}

fn drain(
    document: &PdfDocument,
    stage: &str,
    diagnostics: &mut Vec<Value>,
    budget: &mut Budget,
) -> Result<(), BackendError> {
    for warning in document.take_structured_warnings() {
        let value = json!({
            "source": SOURCE, "stage": stage, "category": warning.category.as_str(),
            "message": warning.message, "page": warning.page.and_then(|page| page.checked_add(1)),
            "upstream_page_index": warning.page, "spec_section": warning.spec_section,
        });
        budget.item(&value)?;
        diagnostics.push(value);
    }
    Ok(())
}

struct Budget {
    bytes: u64,
    items: usize,
}

impl Budget {
    fn item(&mut self, value: &Value) -> Result<(), BackendError> {
        self.items = self
            .items
            .checked_sub(1)
            .ok_or_else(|| BackendError::Limit("conversion page/diagnostic item limit".into()))?;
        self.retain(value)
    }

    fn retain(&mut self, value: &impl Serialize) -> Result<(), BackendError> {
        serde_json::to_writer(Counter(&mut self.bytes), value)
            .map_err(|_| BackendError::Limit("conversion serialized output limit".into()))
    }
}

struct Counter<'a>(&'a mut u64);

impl Write for Counter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        *self.0 = self
            .0
            .checked_sub(bytes.len() as u64)
            .ok_or_else(|| io::Error::other("output limit"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
