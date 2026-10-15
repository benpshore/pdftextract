//! Bounded, in-memory queries against the pinned PDF Oxide document API.
//!
//! Upstream allocates before many of these calls return. These checks bound
//! retained results; callers processing untrusted PDFs still need the engine's
//! supervised worker. A successful query does not certify extraction fidelity.

use std::collections::{BTreeMap, HashSet};
use std::io::{self, Write};

use pdf_oxide::config::ExtractionProfile;
use pdf_oxide::document::ReadingOrder;
use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::{RectFilterMode, TextChar, TextSpan};
use pdf_oxide::object::Object;
use pdf_oxide::search::{SearchOptions, TextSearcher};
use pdf_oxide::{ColumnMode, Destination, OutlineItem, PdfDocument};
use serde::Serialize;
use serde_json::{Value, json};

use super::{Artifact, Context, OperationOutput};
use crate::backend::BackendError;

const SOURCE: &str = "pdf-oxide/0.3.78";
const UNCERTAIN: &str = "extraction_incomplete: pinned PDF Oxide can silently skip content, fonts, or mappings; query completeness and reading order are unverified";
const MAX_PARAMETER_BYTES: usize = 4096;
const MAX_GRAPH_DEPTH: usize = 64;

pub const OPERATIONS: &[&str] = &[
    "query.document",
    "query.page",
    "query.text",
    "query.chars",
    "query.spans",
    "query.words",
    "query.lines",
    "query.page_text",
    "query.structured",
    "query.region",
    "query.layers",
    "query.inks",
    "query.fonts",
    "query.search",
    "query.classify",
    "query.outlines",
    "query.paths",
    "query.content",
    "query.font_data",
    "query.attachments",
];

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
    validate_params(operation, params)?;
    if context.inputs.len() != 1 {
        return Err(BackendError::Unsupported(
            "query operations require exactly one PDF input".into(),
        ));
    }
    let doc = context.open_pdf()?;
    let count = doc.page_count().map_err(upstream)?;
    let mut budget = Budget::new(context);
    let mut diagnostics = Vec::new();
    drain_diagnostics(&doc, "open", &mut diagnostics, &mut budget)?;
    let mut artifacts = Vec::new();
    let result = match operation {
        "query.document" => {
            reject_partial_selection(context, count)?;
            let mark = doc.mark_info().map_err(upstream)?;
            budget.value(&json!({
                "page_count": count,
                "pdf_version": doc.version(),
                "encrypted": doc.is_encrypted(),
                "authenticated": doc.is_authenticated(),
                "producer": doc.document_producer(),
                "creator": doc.document_creator(),
                "permissions": doc.permissions(),
                "marked": mark.marked,
                "suspect_structure": mark.suspects,
                "user_properties": mark.user_properties,
                "prefers_structure_reading_order": doc.prefers_structure_reading_order(),
                "has_output_intents_declaration": doc.has_output_intents_declaration(),
            }))?
        }
        "query.layers" => {
            reject_partial_selection(context, count)?;
            let names = doc.get_layers().map_err(upstream)?;
            let mut default_off: Vec<_> =
                pdf_oxide::optional_content::compute_default_off_ocgs(&doc)
                    .into_iter()
                    .collect();
            default_off.sort();
            budget.value(&json!({"names": names, "default_off": default_off}))?
        }
        "query.outlines" => {
            require_whole_document(context, count)?;
            let expected = preflight_outlines(&doc, context.limits.max_items)?;
            let outline = doc.get_outline().map_err(upstream)?;
            let mut pending: Vec<_> = outline.as_deref().unwrap_or_default().iter().collect();
            let mut returned = 0_usize;
            while let Some(item) = pending.pop() {
                returned = returned.saturating_add(1);
                if returned > context.limits.max_items {
                    return Err(limit("returned outline nodes"));
                }
                pending.extend(&item.children);
            }
            if returned != expected {
                return Err(invalid(
                    "upstream omitted outline nodes after a parse failure",
                ));
            }
            outline_value(outline.as_deref().unwrap_or_default(), &mut budget, 0)?
        }
        "query.font_data" => {
            require_whole_document(context, count)?;
            let fonts = doc
                .extract_embedded_fonts_with_unicode_maps_and_widths()
                .map_err(upstream)?;
            budget.items(fonts.len())?;
            let mut inventory = Vec::new();
            for (index, (original_name, bytes, unicode_map, widths)) in
                fonts.into_iter().enumerate()
            {
                let name = format!("font-{index:06}.bin");
                let byte_count = bytes.len();
                budget.bytes(byte_count)?;
                let unicode_map: BTreeMap<_, _> = unicode_map.into_iter().collect();
                let widths: BTreeMap<_, _> = widths.into_iter().collect();
                budget.items(unicode_map.len().saturating_add(widths.len()))?;
                inventory.push(budget.value(&json!({
                    "artifact": name, "original_name": original_name,
                    "byte_count": byte_count, "unicode_to_gid": unicode_map,
                    "gid_widths_1000_em": widths,
                    "selection": "upstream deduplicates by base name, largest program then bytewise; other subsets may be omitted",
                }))?);
                artifacts.push(Artifact {
                    name,
                    mime: "application/octet-stream".into(),
                    bytes,
                });
            }
            Value::Array(inventory)
        }
        "query.attachments" => {
            reject_partial_selection(context, count)?;
            preflight_name_tree(&doc, "EmbeddedFiles", context.limits.max_items)?;
            let files = doc.extract_embedded_files().map_err(upstream)?;
            budget.items(files.len())?;
            let mut inventory = Vec::new();
            for (index, (original_name, bytes)) in files.into_iter().enumerate() {
                let name = format!("attachment-{index:06}.bin");
                let byte_count = bytes.len();
                budget.bytes(byte_count)?;
                inventory.push(budget.value(&json!({
                    "artifact": name, "original_name": original_name,
                    "byte_count": byte_count,
                    "filename_encoding": "upstream UTF-8-lossy projection, not a verified filesystem name",
                }))?);
                artifacts.push(Artifact {
                    name,
                    mime: "application/octet-stream".into(),
                    bytes,
                });
            }
            Value::Array(inventory)
        }
        "query.search" => {
            let pages = context.selected_pages(count)?;
            search(&doc, pages, params, &mut budget)?
        }
        _ => {
            let pages = context.selected_pages(count)?;
            let mut result = Vec::new();
            for page in pages {
                let value = if operation == "query.content" {
                    let bytes = doc.get_page_content_data(page).map_err(upstream)?;
                    let name = format!("page-{:06}-content.bin", page + 1);
                    let byte_count = bytes.len();
                    budget.bytes(byte_count)?;
                    artifacts.push(Artifact {
                        name: name.clone(),
                        mime: "application/octet-stream".into(),
                        bytes,
                    });
                    json!({"artifact": name, "byte_count": byte_count})
                } else {
                    page_query(operation, &doc, page, params, &mut budget)
                        .map_err(|error| on_page(page, error))?
                };
                budget.items(1)?;
                result.push(json!({"page": page + 1, "value": value}));
                drain_diagnostics(&doc, operation, &mut diagnostics, &mut budget)?;
            }
            Value::Array(result)
        }
    };
    drain_diagnostics(&doc, operation, &mut diagnostics, &mut budget)?;
    if operation == "query.chars" && optional_str(params, "representation", "native")? == "native" {
        diagnostics.push(budget.value(&json!({
            "source": SOURCE, "stage": "extract_chars", "page": null,
            "category": "actual_text_character_mapping_unverified",
            "message": "Pinned native character mode remaps already decoded non-ASCII ActualText through font bytes and can produce mojibake. Native observations are retained; representation=span_derived uses the span Unicode path with derived character geometry.",
        }))?);
    }
    let data = json!({
        "source": SOURCE,
        "source_sha256": context.inputs[0].sha256,
        "operation": operation,
        "parameters": params,
        "character_representation": if operation == "query.chars" {
            Some(match optional_str(params, "representation", "native")? {
                "span_derived" => json!({"name":"span_derived", "api":"extract_page_text_with_options", "geometry":"derived from span/font metrics; not an independent native glyph observation"}),
                _ => json!({"name":"native", "api":"extract_chars_filtered", "geometry":"native character observations with known ActualText mapping limitation"}),
            })
        } else { None },
        "page_numbering": "outer page fields are one-based; upstream payload indices and MCID scopes remain zero-based",
        "completeness": "unverified",
        "result": result,
        "diagnostics": diagnostics,
    });
    // Include the envelope, parameter echo and diagnostic text in the actual
    // aggregate wire budget, in addition to the per-value checks above.
    let artifact_bytes = artifacts.iter().try_fold(0_u64, |sum, artifact| {
        sum.checked_add(artifact.bytes.len() as u64)
            .ok_or_else(|| limit("artifact byte count overflow"))
    })?;
    let remaining = context
        .limits
        .max_output_bytes
        .checked_sub(artifact_bytes)
        .ok_or_else(|| limit("aggregate output bytes"))?;
    bounded_json(&data, remaining)?;
    Ok(OperationOutput {
        data,
        artifacts,
        warnings: vec![UNCERTAIN.into()],
    })
}

fn page_query(
    operation: &str,
    doc: &PdfDocument,
    page: usize,
    params: &Value,
    budget: &mut Budget,
) -> Result<Value, BackendError> {
    match operation {
        "query.page" => {
            let media = doc.get_page_media_box(page).map_err(upstream)?;
            let visible = doc.get_page_visible_box(page).map_err(upstream)?;
            for value in [
                media.0, media.1, media.2, media.3, visible.0, visible.1, visible.2, visible.3,
            ] {
                finite(value)?;
            }
            budget.value(&json!({
                "media_box_corners": media, "visible_box_corners": visible,
                "width": media.2 - media.0, "height": media.3 - media.1,
                "rotation": doc.get_page_rotation(page).map_err(upstream)?,
                "has_text_layer": doc.has_text_layer(page).map_err(upstream)?,
            }))
        }
        "query.text" => {
            let text = doc
                .extract_text_filtered(
                    page,
                    name_set(params, "excluded_layers")?,
                    name_set(params, "excluded_inks")?,
                )
                .map_err(upstream)?;
            budget.value(&text)
        }
        "query.chars" => {
            let chars = match optional_str(params, "representation", "native")? {
                "native" => {
                    if params.get("reading_order").is_some() {
                        return Err(invalid(
                            "reading_order applies only to span_derived characters",
                        ));
                    }
                    doc.extract_chars_filtered(
                        page,
                        name_set(params, "excluded_layers")?,
                        name_set(params, "excluded_inks")?,
                    )
                    .map_err(upstream)?
                }
                "span_derived" => {
                    if params.get("excluded_layers").is_some()
                        || params.get("excluded_inks").is_some()
                    {
                        return Err(invalid(
                            "pinned page-text derived characters do not accept layer/ink exclusions",
                        ));
                    }
                    doc.extract_page_text_with_options(page, reading_order(params)?)
                        .map_err(upstream)?
                        .chars
                }
                _ => {
                    return Err(invalid(
                        "character representation must be native or span_derived",
                    ));
                }
            };
            chars_value(&chars, budget)
        }
        "query.spans" => {
            let spans = if let Some(merging) = params.get("merging") {
                if params.get("reading_order").is_some()
                    || params.get("excluded_layers").is_some()
                    || params.get("excluded_inks").is_some()
                {
                    return Err(invalid(
                        "merging cannot be combined with reading-order/layer/ink options in the pinned API",
                    ));
                }
                let config = match required_str(merging, "merging")? {
                    "default" => pdf_oxide::extractors::SpanMergingConfig::default(),
                    "adaptive" => pdf_oxide::extractors::SpanMergingConfig::adaptive(),
                    "conservative" => pdf_oxide::extractors::SpanMergingConfig::conservative(),
                    "aggressive" => pdf_oxide::extractors::SpanMergingConfig::aggressive(),
                    _ => return Err(invalid("unknown span merging preset")),
                };
                doc.extract_spans_with_config(page, config)
                    .map_err(upstream)?
            } else if params.get("reading_order").is_some()
                || params.get("excluded_layers").is_some()
                || params.get("excluded_inks").is_some()
            {
                doc.extract_spans_filtered_with_reading_order(
                    page,
                    reading_order(params)?,
                    name_set(params, "excluded_layers")?,
                    name_set(params, "excluded_inks")?,
                )
                .map_err(upstream)?
            } else {
                doc.extract_spans(page).map_err(upstream)?
            };
            spans_value(&spans, budget)
        }
        "query.words" => {
            let words = doc
                .extract_words_with_thresholds(
                    page,
                    threshold(params, "word_gap")?,
                    profile(params)?,
                )
                .map_err(upstream)?;
            for word in &words {
                rect_valid(word.bbox)?;
                check_chars(&word.chars)?;
            }
            budget.value(&words)
        }
        "query.lines" => {
            let lines = doc
                .extract_text_lines_with_thresholds(
                    page,
                    threshold(params, "word_gap")?,
                    threshold(params, "line_gap")?,
                    profile(params)?,
                )
                .map_err(upstream)?;
            for line in &lines {
                rect_valid(line.bbox)?;
                for word in &line.words {
                    rect_valid(word.bbox)?;
                    check_chars(&word.chars)?;
                }
            }
            budget.value(&lines)
        }
        "query.page_text" => {
            let text = doc
                .extract_page_text_with_options(page, reading_order(params)?)
                .map_err(upstream)?;
            let media = doc.get_page_media_box(page).map_err(upstream)?;
            finite(text.page_width)?;
            finite(text.page_height)?;
            Ok(json!({
                "page_width": text.page_width, "page_height": text.page_height,
                "media_box_corners": media,
                "media_box_width": media.2 - media.0, "media_box_height": media.3 - media.1,
                "dimension_note": "pinned page_width/page_height expose upper-right coordinates; use media_box dimensions for nonzero origins",
                "spans": spans_value(&text.spans, budget)?,
                "chars": chars_value(&text.chars, budget)?,
            }))
        }
        "query.structured" => {
            let mode = match optional_str(params, "columns", "auto")? {
                "auto" => ColumnMode::Auto,
                "single" => ColumnMode::Single,
                "two" => ColumnMode::Two,
                _ => return Err(invalid("columns must be auto, single, or two")),
            };
            let structured = doc
                .extract_structured_with_column_mode(page, mode)
                .map_err(upstream)?;
            finite(structured.page_width)?;
            finite(structured.page_height)?;
            budget.items(structured.regions.len())?;
            let mut regions = Vec::new();
            for region in structured.regions {
                rect_valid(region.bbox)?;
                let spans = spans_value(&region.spans, budget)?;
                let mut value = budget.value(&json!({
                    "kind": region.kind, "text": region.text, "bbox": region.bbox,
                    "column_index": region.column_index, "section_id": region.section_id,
                }))?;
                value["spans"] = spans;
                regions.push(value);
            }
            Ok(
                json!({"page_index": page, "page_width": structured.page_width,
                "page_height": structured.page_height, "regions": regions}),
            )
        }
        "query.region" => region(doc, page, params, budget),
        "query.inks" => {
            let inks = if optional_bool(params, "deep", false)? {
                doc.get_page_inks_deep(page)
            } else {
                doc.get_page_inks(page)
            }
            .map_err(upstream)?;
            budget.value(&inks)
        }
        "query.fonts" => {
            let stats = doc.page_font_stats(page).map_err(upstream)?;
            for value in [
                stats.dominant_em,
                stats.dominant_line_height,
                stats.dominant_char_width,
            ] {
                finite(value)?;
            }
            budget.value(&json!({
                "dominant_em": stats.dominant_em,
                "dominant_line_height": stats.dominant_line_height,
                "dominant_char_width": stats.dominant_char_width,
                "body_font_name": stats.body_font_name,
                "measurement": "upstream estimates; empty pages use documented defaults",
            }))
        }
        "query.classify" => {
            let classification = doc.classify_page(page).map_err(upstream)?;
            finite(classification.confidence)?;
            budget.value(&classification)
        }
        "query.paths" => {
            let paths = match optional_str(params, "kind", "all")? {
                "all" => doc.extract_paths(page),
                "rectangles" => doc.extract_rects(page),
                "lines" => doc.extract_lines(page),
                _ => return Err(invalid("path kind must be all, rectangles, or lines")),
            }
            .map_err(upstream)?;
            paths_value(paths, budget)
        }
        _ => Err(BackendError::Unsupported(format!(
            "query operation {operation}"
        ))),
    }
}

fn region(
    doc: &PdfDocument,
    page: usize,
    params: &Value,
    budget: &mut Budget,
) -> Result<Value, BackendError> {
    let values = params
        .get("rect")
        .and_then(Value::as_array)
        .filter(|values| values.len() == 4)
        .ok_or_else(|| invalid("rect must be [x, y, width, height] in PDF points"))?;
    let mut xywh = [0_f32; 4];
    for (target, value) in xywh.iter_mut().zip(values) {
        let value = value
            .as_f64()
            .ok_or_else(|| invalid("rect values must be finite numbers"))?;
        if !value.is_finite() || value.abs() > 1_000_000.0 {
            return Err(invalid("rect coordinate exceeds 1000000 points"));
        }
        *target = value as f32;
    }
    if xywh[2] <= 0.0 || xywh[3] <= 0.0 {
        return Err(invalid("rect width and height must be positive"));
    }
    let rect = Rect::new(xywh[0], xywh[1], xywh[2], xywh[3]);
    let mode = match optional_str(params, "mode", "intersects")? {
        "intersects" => RectFilterMode::Intersects,
        "contained" => RectFilterMode::FullyContained,
        "overlap" => {
            let ratio = params
                .get("overlap")
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && *n > 0.0 && *n <= 1.0)
                .ok_or_else(|| invalid("overlap mode requires a fraction in (0, 1]"))?;
            RectFilterMode::MinOverlap(ratio as f32)
        }
        _ => {
            return Err(invalid(
                "region mode must be intersects, contained, or overlap",
            ));
        }
    };
    if params.get("overlap").is_some() && optional_str(params, "mode", "intersects")? != "overlap" {
        return Err(invalid("overlap applies only to overlap mode"));
    }
    let kind = optional_str(params, "kind", "text")?;
    if kind != "text"
        && (params.get("excluded_layers").is_some() || params.get("excluded_inks").is_some())
    {
        return Err(invalid(
            "pinned region layer/ink filtering supports text only",
        ));
    }
    match kind {
        "text" => budget.value(
            &doc.extract_text_filtered_in_rect(
                page,
                name_set(params, "excluded_layers")?,
                name_set(params, "excluded_inks")?,
                rect,
                mode,
            )
            .map_err(upstream)?,
        ),
        "chars" => chars_value(
            &doc.extract_chars_in_rect(page, rect, mode)
                .map_err(upstream)?,
            budget,
        ),
        "spans" => spans_value(
            &doc.extract_spans_in_rect(page, rect, mode)
                .map_err(upstream)?,
            budget,
        ),
        "words" => budget.value(
            &doc.extract_words_in_rect(page, rect, mode)
                .map_err(upstream)?,
        ),
        "lines" => budget.value(
            &doc.extract_text_lines_in_rect(page, rect, mode)
                .map_err(upstream)?,
        ),
        "paths" => {
            if params.get("mode").is_some()
                && optional_str(params, "mode", "intersects")? != "intersects"
            {
                return Err(invalid("pinned path-region API supports intersects only"));
            }
            paths_value(
                doc.extract_paths_in_rect(page, rect).map_err(upstream)?,
                budget,
            )
        }
        _ => Err(invalid(
            "region kind must be text, chars, spans, words, lines, or paths",
        )),
    }
}

fn search(
    doc: &PdfDocument,
    pages: Vec<usize>,
    params: &Value,
    budget: &mut Budget,
) -> Result<Value, BackendError> {
    let pattern = params
        .get("pattern")
        .ok_or_else(|| invalid("search pattern required"))?;
    let pattern = required_str(pattern, "pattern")?;
    if pattern.is_empty() {
        return Err(invalid("search pattern must not be empty"));
    }
    let literal = optional_bool(params, "literal", true)?;
    let whole_word = optional_bool(params, "whole_word", false)?;
    let case_insensitive = optional_bool(params, "case_insensitive", false)?;
    let maximum = params
        .get("max_results")
        .map_or(Ok(budget.max_items), |v| {
            v.as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n > 0 && *n <= budget.max_items)
                .ok_or_else(|| invalid("max_results must be positive and within max_items"))
        })?;
    let mut expression = if literal {
        regex::escape(pattern)
    } else {
        pattern.to_owned()
    };
    if whole_word {
        expression = format!(r"\b(?:{expression})\b");
    }
    let regex = regex::RegexBuilder::new(&expression)
        .case_insensitive(case_insensitive)
        .size_limit(1024 * 1024)
        .dfa_size_limit(1024 * 1024)
        .nest_limit(128)
        .build()
        .map_err(|error| invalid(&format!("search regex: {error}")))?;
    let mut found = Vec::new();
    for page in pages {
        let options = SearchOptions {
            max_results: maximum.saturating_sub(found.len()).saturating_add(1),
            ..Default::default()
        };
        let matches = TextSearcher::search_page(doc, page, &regex, &options)
            .map_err(|error| on_page(page, upstream(error)))?;
        if found.len().saturating_add(matches.len()) > maximum {
            return Err(limit(
                "search results exceed max_results; no truncated result is published",
            ));
        }
        for item in matches {
            rect_valid(item.bbox)?;
            for rect in &item.span_boxes {
                rect_valid(*rect)?;
            }
            let value = budget.value(&item)?;
            found.push(json!({"page": page + 1, "match": value}));
        }
    }
    Ok(
        json!({"matches": found, "offset_unit": "UTF-8 bytes in upstream span-joined page text", "geometry": "whole matching span boxes, not exact glyph highlights", "truncated": false}),
    )
}

fn spans_value(spans: &[TextSpan], budget: &mut Budget) -> Result<Value, BackendError> {
    budget.items(spans.len())?;
    let mut values = Vec::new();
    for span in spans {
        rect_valid(span.bbox)?;
        rect_valid(span.page_bbox())?;
        for n in [
            span.font_size,
            span.char_spacing,
            span.word_spacing,
            span.horizontal_scaling,
            span.rotation_degrees,
            span.text_rise,
        ] {
            finite(n)?;
        }
        for n in span.char_widths.iter().chain(&span.char_x_offsets) {
            finite(*n)?;
        }
        let mut value = budget.value(span)?;
        // Upstream serde omits these public fields. They matter when consumers
        // compare a rotated/mirrored span against page-space regions.
        value["mirrored"] = json!(span.mirrored);
        value["page_rotation_applied"] = json!(span.page_rotation_applied);
        value["page_bbox"] = json!(span.page_bbox());
        values.push(value);
    }
    Ok(Value::Array(values))
}

fn chars_value(chars: &[TextChar], budget: &mut Budget) -> Result<Value, BackendError> {
    check_chars(chars)?;
    budget.value(chars)
}

fn check_chars(chars: &[TextChar]) -> Result<(), BackendError> {
    for item in chars {
        rect_valid(item.bbox)?;
        for n in [
            item.font_size,
            item.origin_x,
            item.origin_y,
            item.rotation_degrees,
            item.advance_width,
            item.rendered_advance,
            item.ascent,
            item.descent,
            item.color.r,
            item.color.g,
            item.color.b,
        ] {
            finite(n)?;
        }
        if let Some(matrix) = item.matrix {
            for n in matrix {
                finite(n)?;
            }
        }
    }
    Ok(())
}

fn paths_value(
    paths: Vec<pdf_oxide::elements::PathContent>,
    budget: &mut Budget,
) -> Result<Value, BackendError> {
    budget.items(paths.len())?;
    let mut values = Vec::new();
    for path in paths {
        rect_valid(path.bbox)?;
        rect_valid(path.rendered_bbox())?;
        finite(path.stroke_width)?;
        let mut value = budget.value(&path)?;
        value["artifact_type"] = budget.value(&path.artifact_type)?;
        value["rendered_bbox"] = json!(path.rendered_bbox());
        values.push(value);
    }
    Ok(Value::Array(values))
}

fn outline_value(
    items: &[OutlineItem],
    budget: &mut Budget,
    depth: usize,
) -> Result<Value, BackendError> {
    if depth > MAX_GRAPH_DEPTH {
        return Err(limit("outline depth"));
    }
    budget.items(items.len())?;
    let mut result = Vec::new();
    for item in items {
        let destination = match &item.dest {
            Some(Destination::PageIndex(page)) => {
                json!({"kind": "page", "page": page + 1, "upstream_page_index": page})
            }
            Some(Destination::Named(name)) => json!({"kind": "unresolved_name", "name": name}),
            None => Value::Null,
        };
        let mut value = budget.value(&json!({"title": item.title, "destination": destination}))?;
        value["children"] = outline_value(&item.children, budget, depth + 1)?;
        result.push(value);
    }
    Ok(Value::Array(result))
}

fn preflight_outlines(doc: &PdfDocument, max_items: usize) -> Result<usize, BackendError> {
    preflight_name_tree(doc, "Dests", max_items)?;
    let catalog = doc.catalog().map_err(upstream)?;
    let Some(root) = catalog
        .as_dict()
        .and_then(|dict| dict.get("Outlines"))
        .and_then(Object::as_reference)
    else {
        return Ok(0);
    };
    let mut queue = vec![(root, 0_usize)];
    let mut visited = HashSet::new();
    while let Some((reference, depth)) = queue.pop() {
        if depth > MAX_GRAPH_DEPTH || visited.len() >= max_items {
            return Err(limit("outline graph depth/items"));
        }
        if !visited.insert(reference) {
            return Err(invalid("cyclic or multiply referenced outline graph"));
        }
        let object = doc.load_object(reference).map_err(upstream)?;
        let Some(dict) = object.as_dict() else {
            return Err(invalid("outline node is not a dictionary"));
        };
        for (key, next_depth) in [("First", depth + 1), ("Next", depth)] {
            if let Some(reference) = dict.get(key).and_then(Object::as_reference) {
                queue.push((reference, next_depth));
            }
        }
        if let Some(dest) = dict.get("Dest") {
            preflight_destination(doc, dest)?;
        }
        if let Some(action) = dict.get("A") {
            // Resolve only the reference chain. Recursively resolving a GoTo
            // dictionary would enter the destination page's cyclic Parent/Kids
            // graph and duplicate unrelated document objects.
            let action = resolve_chain(doc, action)?;
            if let Some(dest) = action.as_dict().and_then(|a| a.get("D")) {
                preflight_destination(doc, dest)?;
            }
        }
    }
    Ok(visited.len().saturating_sub(1))
}

fn preflight_destination(doc: &PdfDocument, destination: &Object) -> Result<(), BackendError> {
    resolve_chain(doc, destination).map(|_| ())
}

fn resolve_chain(doc: &PdfDocument, object: &Object) -> Result<Object, BackendError> {
    let mut destination = object.clone();
    let mut visited = HashSet::new();
    while let Object::Reference(reference) = destination {
        if visited.len() >= MAX_GRAPH_DEPTH {
            return Err(limit("outline destination reference depth"));
        }
        if !visited.insert(reference) {
            return Err(invalid("cyclic outline destination reference"));
        }
        destination = doc.load_object(reference).map_err(upstream)?;
    }
    Ok(destination)
}

fn preflight_name_tree(doc: &PdfDocument, key: &str, max_items: usize) -> Result<(), BackendError> {
    let catalog = doc.catalog().map_err(upstream)?;
    let Some(names) = catalog.as_dict().and_then(|dict| dict.get("Names")) else {
        return Ok(());
    };
    let names = resolve_chain(doc, names)?;
    let Some(root) = names.as_dict().and_then(|dict| dict.get(key)) else {
        return Ok(());
    };
    let mut queue = vec![(root.clone(), 0_usize)];
    let mut visited = HashSet::new();
    let mut entries = 0_usize;
    while let Some((object, depth)) = queue.pop() {
        if depth > 32 {
            return Err(limit("name-tree depth"));
        }
        if let Object::Reference(reference) = &object {
            if !visited.insert(*reference) {
                return Err(invalid("cyclic or multiply referenced name tree"));
            }
        }
        entries = entries.saturating_add(1);
        if entries > max_items {
            return Err(limit("name-tree entries"));
        }
        let object = resolve_chain(doc, &object)?;
        let dict = object
            .as_dict()
            .ok_or_else(|| invalid("name-tree node is not a dictionary"))?;
        if let Some(names) = dict.get("Names") {
            let names = resolve_chain(doc, names)?;
            let names = names
                .as_array()
                .ok_or_else(|| invalid("name-tree Names is not an array"))?;
            if names.len() % 2 != 0 {
                return Err(invalid("name-tree Names has unmatched key/value"));
            }
            entries = entries.saturating_add(names.len() / 2);
            if entries > max_items {
                return Err(limit("name-tree entries"));
            }
        }
        if let Some(kids) = dict.get("Kids") {
            let kids = resolve_chain(doc, kids)?;
            let kids = kids
                .as_array()
                .ok_or_else(|| invalid("name-tree Kids is not an array"))?;
            if kids
                .len()
                .saturating_add(queue.len())
                .saturating_add(entries)
                > max_items
            {
                return Err(limit("name-tree pending nodes"));
            }
            for kid in kids {
                queue.push((kid.clone(), depth + 1));
            }
        }
    }
    Ok(())
}

fn drain_diagnostics(
    doc: &PdfDocument,
    stage: &str,
    output: &mut Vec<Value>,
    budget: &mut Budget,
) -> Result<(), BackendError> {
    for warning in doc.take_structured_warnings() {
        budget.items(1)?;
        output.push(budget.value(&json!({
            "source": SOURCE, "stage": stage,
            "category": warning.category.as_str(), "message": warning.message,
            "page": warning.page.and_then(|n| n.checked_add(1)),
            "upstream_page_index": warning.page, "spec_section": warning.spec_section,
        }))?);
    }
    Ok(())
}

fn validate_params(operation: &str, params: &Value) -> Result<(), BackendError> {
    let object = params
        .as_object()
        .ok_or_else(|| invalid("query parameters must be an object"))?;
    let allowed: &[&str] = match operation {
        "query.text" => &["excluded_layers", "excluded_inks"],
        "query.chars" => &[
            "excluded_layers",
            "excluded_inks",
            "representation",
            "reading_order",
        ],
        "query.spans" => &[
            "excluded_layers",
            "excluded_inks",
            "reading_order",
            "merging",
        ],
        "query.words" => &["word_gap", "profile"],
        "query.lines" => &["word_gap", "line_gap", "profile"],
        "query.page_text" => &["reading_order"],
        "query.structured" => &["columns"],
        "query.region" => &[
            "rect",
            "mode",
            "overlap",
            "kind",
            "excluded_layers",
            "excluded_inks",
        ],
        "query.inks" => &["deep"],
        "query.paths" => &["kind"],
        "query.search" => &[
            "pattern",
            "literal",
            "whole_word",
            "case_insensitive",
            "max_results",
        ],
        _ => &[],
    };
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(invalid(&format!("unsupported parameter {key}")));
    }
    Ok(())
}

fn optional_bool(params: &Value, key: &str, default: bool) -> Result<bool, BackendError> {
    params.get(key).map_or(Ok(default), |value| {
        value
            .as_bool()
            .ok_or_else(|| invalid(&format!("{key} must be boolean")))
    })
}

fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, BackendError> {
    value
        .as_str()
        .filter(|s| s.len() <= MAX_PARAMETER_BYTES)
        .ok_or_else(|| {
            invalid(&format!(
                "{key} must be a string of at most {MAX_PARAMETER_BYTES} bytes"
            ))
        })
}

fn optional_str<'a>(
    params: &'a Value,
    key: &str,
    default: &'a str,
) -> Result<&'a str, BackendError> {
    params
        .get(key)
        .map_or(Ok(default), |value| required_str(value, key))
}

fn name_set(params: &Value, key: &str) -> Result<HashSet<String>, BackendError> {
    let Some(value) = params.get(key) else {
        return Ok(HashSet::new());
    };
    let names = value
        .as_array()
        .filter(|names| names.len() <= 128)
        .ok_or_else(|| invalid(&format!("{key} must be an array of at most 128 strings")))?;
    names
        .iter()
        .map(|name| required_str(name, key).map(str::to_owned))
        .collect()
}

fn threshold(params: &Value, key: &str) -> Result<Option<f32>, BackendError> {
    params
        .get(key)
        .map(|value| {
            value
                .as_f64()
                .filter(|n| n.is_finite() && *n > 0.0 && *n <= 10_000.0)
                .map(|n| n as f32)
                .ok_or_else(|| invalid(&format!("{key} must be in (0, 10000] PDF points")))
        })
        .transpose()
}

fn profile(params: &Value) -> Result<Option<ExtractionProfile>, BackendError> {
    let Some(value) = params.get("profile") else {
        return Ok(None);
    };
    Ok(Some(match required_str(value, "profile")? {
        "conservative" => ExtractionProfile::CONSERVATIVE,
        "tj_heavy" => ExtractionProfile::TJ_HEAVY,
        "aggressive" => ExtractionProfile::AGGRESSIVE,
        "balanced" => ExtractionProfile::BALANCED,
        "academic" => ExtractionProfile::ACADEMIC,
        "policy" => ExtractionProfile::POLICY,
        "form" => ExtractionProfile::FORM,
        "government" => ExtractionProfile::GOVERNMENT,
        "scanned_ocr" => ExtractionProfile::SCANNED_OCR,
        "adaptive" => ExtractionProfile::ADAPTIVE,
        _ => return Err(invalid("unknown pinned extraction profile")),
    }))
}

fn reading_order(params: &Value) -> Result<ReadingOrder, BackendError> {
    match optional_str(params, "reading_order", "top_to_bottom")? {
        "top_to_bottom" => Ok(ReadingOrder::TopToBottom),
        "column_aware" => Ok(ReadingOrder::ColumnAware),
        "structure" => Ok(ReadingOrder::Structure),
        _ => Err(invalid("unknown reading_order")),
    }
}

fn reject_partial_selection(context: &Context<'_>, count: usize) -> Result<(), BackendError> {
    if let Some((first, last)) = context.pages {
        if first != 1 || usize::try_from(last).ok() != Some(count) {
            return Err(invalid(
                "document-scoped query does not accept a partial page range",
            ));
        }
    }
    Ok(())
}

fn require_whole_document(context: &Context<'_>, count: usize) -> Result<(), BackendError> {
    reject_partial_selection(context, count)?;
    if count > context.limits.max_pages as usize {
        return Err(limit("whole-document query exceeds max_pages"));
    }
    Ok(())
}

fn rect_valid(rect: Rect) -> Result<(), BackendError> {
    for n in [
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        rect.x + rect.width,
        rect.y + rect.height,
    ] {
        finite(n)?;
    }
    if rect.width < 0.0 || rect.height < 0.0 {
        return Err(invalid("upstream returned negative rectangle dimensions"));
    }
    Ok(())
}

fn finite(n: f32) -> Result<(), BackendError> {
    if n.is_finite() {
        Ok(())
    } else {
        Err(invalid("upstream returned non-finite geometry or metrics"))
    }
}

fn upstream(error: impl std::fmt::Display) -> BackendError {
    invalid(&error.to_string())
}
fn invalid(message: &str) -> BackendError {
    BackendError::Malformed(format!("{SOURCE} query: {message}"))
}
fn limit(message: &str) -> BackendError {
    BackendError::Limit(format!("{SOURCE} query: {message}"))
}
fn on_page(page: usize, error: BackendError) -> BackendError {
    match error {
        BackendError::Malformed(message) => BackendError::Page {
            page: u32::try_from(page + 1).unwrap_or(u32::MAX),
            message,
        },
        other => other,
    }
}

struct Budget {
    remaining: u64,
    item_count: usize,
    max_items: usize,
}
impl Budget {
    fn new(context: &Context<'_>) -> Self {
        Self {
            remaining: context.limits.max_output_bytes,
            item_count: 0,
            max_items: context.limits.max_items,
        }
    }
    fn bytes(&mut self, n: usize) -> Result<(), BackendError> {
        self.remaining = self
            .remaining
            .checked_sub(n as u64)
            .ok_or_else(|| limit("retained output bytes"))?;
        Ok(())
    }
    fn items(&mut self, n: usize) -> Result<(), BackendError> {
        self.item_count = self
            .item_count
            .checked_add(n)
            .ok_or_else(|| limit("item count overflow"))?;
        if self.item_count > self.max_items {
            return Err(limit("retained collection entries exceed max_items"));
        }
        Ok(())
    }
    fn value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<Value, BackendError> {
        let bytes = bounded_json(value, self.remaining)?;
        self.bytes(bytes.len())?;
        let value: Value = serde_json::from_slice(&bytes).map_err(upstream)?;
        self.walk(&value, 0)?;
        Ok(value)
    }
    fn walk(&mut self, value: &Value, depth: usize) -> Result<(), BackendError> {
        if depth > MAX_GRAPH_DEPTH {
            return Err(limit("serialized query depth"));
        }
        match value {
            Value::Array(values) => {
                self.items(values.len())?;
                for value in values {
                    self.walk(value, depth + 1)?;
                }
            }
            Value::Object(values) => {
                for value in values.values() {
                    self.walk(value, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

struct CappedWriter {
    bytes: Vec<u8>,
    maximum: u64,
}
impl Write for CappedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if (self.bytes.len() as u64).saturating_add(bytes.len() as u64) > self.maximum {
            return Err(io::Error::other("query output cap"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn bounded_json<T: Serialize + ?Sized>(value: &T, maximum: u64) -> Result<Vec<u8>, BackendError> {
    let mut writer = CappedWriter {
        bytes: Vec::new(),
        maximum,
    };
    serde_json::to_writer(&mut writer, value).map_err(|error| {
        if error.is_io() {
            limit("serialized output exceeds max_output_bytes")
        } else {
            upstream(error)
        }
    })?;
    Ok(writer.bytes)
}
