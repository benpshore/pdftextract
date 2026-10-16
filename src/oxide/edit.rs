//! Bounded, byte-to-byte PDF edits. Inputs are never overwritten.

use std::collections::{BTreeMap, HashSet};

use pdf_oxide::PdfDocument;
use pdf_oxide::annotation_types::TextMarkupType;
use pdf_oxide::editor::{DocumentEditor, EditableDocument};
use pdf_oxide::elements::{FontSpec, TextContent, TextStyle};
use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::Color;
use pdf_oxide::object::Object;
use pdf_oxide::writer::{
    Annotation, FreeTextAnnotation, LinkAnnotation, ShapeAnnotation, StampAnnotation,
    TextAnnotation, TextMarkupAnnotation,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{Artifact, Context, OperationOutput};
use crate::backend::BackendError;

pub const OPERATIONS: &[&str] = &[
    "edit.pages",
    "edit.merge",
    "edit.split",
    "edit.metadata",
    "edit.annotations",
    "edit.attachments",
    "edit.bookmarks",
    "edit.content",
    "edit.image_bounds",
    "edit.whiteout",
];

pub(super) fn execute(
    operation: &str,
    context: &Context<'_>,
    params: &Value,
) -> Option<Result<OperationOutput, BackendError>> {
    Some(match operation {
        "edit.pages" => pages(context, params),
        "edit.merge" => merge(context, params),
        "edit.split" => split(context, params),
        "edit.metadata" => metadata(context, params),
        "edit.annotations" => annotations(context, params),
        "edit.attachments" => attachments(context, params),
        "edit.bookmarks" => bookmarks(context, params),
        "edit.content" => content(context, params),
        "edit.image_bounds" => image_bounds(context, params),
        "edit.whiteout" => whiteout(context, params),
        _ => return None,
    })
}

fn selected_page(context: &Context<'_>, page: u32, count: usize) -> Result<usize, BackendError> {
    let index = page_index(page, count)?;
    if let Some((first, last)) = context.pages
        && !(first..=last).contains(&page)
    {
        return Err(BackendError::Malformed(
            "edit is outside the requested page selection".into(),
        ));
    }
    Ok(index)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentParams {
    changes: Vec<ContentChange>,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum ContentChange {
    AddText {
        page: u32,
        rect: [f32; 4],
        text: String,
        size: f32,
        #[serde(default)]
        color: Option<[f32; 3]>,
    },
    ReplaceText {
        page: u32,
        find: String,
        replacement: String,
        expected_matches: usize,
        accept_region_rewrite: bool,
    },
}

fn content(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let params: ContentParams = parse(params)?;
    bounded_count(
        params.changes.len(),
        context.limits.max_items,
        "content changes",
    )?;
    let mut editor = edit_document(context)?;
    let count = editor.current_page_count();
    let mut per_page = BTreeMap::<usize, Vec<ContentChange>>::new();
    for change in params.changes {
        let number = match &change {
            ContentChange::AddText { page, .. } | ContentChange::ReplaceText { page, .. } => *page,
        };
        per_page
            .entry(selected_page(context, number, count)?)
            .or_default()
            .push(change);
    }
    let mut records = Vec::new();
    for (index, changes) in per_page {
        let mut page = editor.get_page(index).map_err(upstream)?;
        for change in changes {
            match change {
                ContentChange::AddText {
                    rect,
                    text,
                    size,
                    color,
                    ..
                } => {
                    bounded_string(&text, "overlay text")?;
                    if text.is_empty() || !text.is_ascii() {
                        return Err(BackendError::Unsupported("text overlays currently require nonempty ASCII with the bundled Helvetica mapping".into()));
                    }
                    if !size.is_finite() || !(1.0..=1000.0).contains(&size) {
                        return Err(BackendError::Malformed(
                            "font size must be finite and within 1..=1000".into(),
                        ));
                    }
                    let mut style = TextStyle::default();
                    if let Some(color) = color {
                        let [r, g, b] = rgb(color)?;
                        style.color = Color::new(r, g, b);
                    }
                    page.add_text(TextContent::new(
                        text.clone(),
                        rectangle(rect)?,
                        FontSpec::helvetica(size),
                        style,
                    ));
                    records.push(json!({"page": index + 1, "action": "add_text", "text": text}));
                }
                ContentChange::ReplaceText {
                    find,
                    replacement,
                    expected_matches,
                    accept_region_rewrite,
                    ..
                } => {
                    bounded_string(&find, "find text")?;
                    bounded_string(&replacement, "replacement text")?;
                    if find.is_empty() || expected_matches == 0 || !accept_region_rewrite {
                        return Err(BackendError::Malformed("replacement requires nonempty find, positive expected_matches and explicit accept_region_rewrite".into()));
                    }
                    if !replacement.is_ascii() {
                        return Err(BackendError::Unsupported(
                            "replacement glyph coverage is only verified for ASCII".into(),
                        ));
                    }
                    bounded_count(
                        expected_matches,
                        context.limits.max_items,
                        "text replacements",
                    )?;
                    let matches = page.find_text_containing(&find);
                    bounded_count(
                        matches.len(),
                        context.limits.max_items,
                        "matched text elements",
                    )?;
                    let occurrences: usize = matches
                        .iter()
                        .map(|text| text.text().matches(&find).count())
                        .sum();
                    if occurrences != expected_matches {
                        return Err(BackendError::Malformed(format!(
                            "expected {expected_matches} literal text matches, found {occurrences}"
                        )));
                    }
                    // The pinned save_page path turns DOM erasures into destructive
                    // redactions with a black fill. Text extraction can still report
                    // the replacement while the resulting page paints black masks.
                    return Err(BackendError::Unsupported(
                        "pdf_oxide 0.3.78 text replacement paints black redaction masks; visually correct replacement is not verified".into(),
                    ));
                }
            }
        }
        editor.save_page(page).map_err(upstream)?;
    }
    saved(context, &mut editor, json!({"changes": records}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageBoundsParams {
    changes: Vec<ImageBounds>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageBounds {
    page: u32,
    name: String,
    rect: [f32; 4],
}

fn image_bounds(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let params: ImageBoundsParams = parse(params)?;
    bounded_count(
        params.changes.len(),
        context.limits.max_items,
        "image position changes",
    )?;
    let mut editor = edit_document(context)?;
    let count = editor.current_page_count();
    let mut expected = BTreeMap::new();
    for change in params.changes {
        bounded_string(&change.name, "image resource name")?;
        let page = selected_page(context, change.page, count)?;
        let rect = rectangle(change.rect)?;
        let images = editor.get_page_images(page).map_err(upstream)?;
        bounded_count(
            images.len(),
            context.limits.max_items,
            "page image resources",
        )?;
        if images
            .iter()
            .filter(|image| image.name == change.name)
            .count()
            != 1
        {
            return Err(BackendError::Unsupported(
                "image resource must identify exactly one direct image occurrence".into(),
            ));
        }
        editor
            .set_image_bounds(page, &change.name, rect.x, rect.y, rect.width, rect.height)
            .map_err(upstream)?;
        expected.insert((page, change.name), change.rect);
    }
    let output = saved(
        context,
        &mut editor,
        json!({"image_occurrences_modified": expected.len()}),
    )?;
    let mut check =
        DocumentEditor::from_bytes(output.artifacts[0].bytes.clone()).map_err(upstream)?;
    for ((page, name), rect) in expected {
        let images = check.get_page_images(page).map_err(upstream)?;
        let actual = images
            .iter()
            .find(|image| image.name == name)
            .ok_or_else(|| {
                BackendError::Unsupported("saved image occurrence disappeared".into())
            })?;
        if actual
            .bounds
            .iter()
            .zip(rect)
            .any(|(a, b)| (*a - b).abs() > 0.01)
        {
            return Err(BackendError::Unsupported(
                "pinned editor failed image-bounds round-trip".into(),
            ));
        }
    }
    Ok(output)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WhiteoutParams {
    regions: Vec<WhiteoutRegion>,
    acknowledge_text_remains: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WhiteoutRegion {
    page: u32,
    rect: [f32; 4],
}

fn whiteout(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let params: WhiteoutParams = parse(params)?;
    if !params.acknowledge_text_remains {
        return Err(BackendError::Unsupported("visual whiteout leaves underlying content recoverable; acknowledge_text_remains must be true".into()));
    }
    bounded_count(
        params.regions.len(),
        context.limits.max_items,
        "whiteout regions",
    )?;
    let mut editor = edit_document(context)?;
    let count = editor.current_page_count();
    for region in &params.regions {
        editor
            .erase_region(
                selected_page(context, region.page, count)?,
                pdf_box(region.rect)?,
            )
            .map_err(upstream)?;
    }
    let mut output = saved(
        context,
        &mut editor,
        json!({"visual_whiteout_regions": params.regions.len(), "underlying_content_retained": true}),
    )?;
    output.warnings.push("Visual whiteout only draws a white rectangle: underlying text/images remain recoverable. It must not be used to remove confidential information.".into());
    Ok(output)
}

pub(super) fn parse<T: DeserializeOwned>(params: &Value) -> Result<T, BackendError> {
    serde_json::from_value(params.clone())
        .map_err(|error| BackendError::Malformed(format!("invalid operation parameters: {error}")))
}

pub(super) fn upstream(error: impl std::fmt::Display) -> BackendError {
    BackendError::Malformed(format!("pdf-oxide: {error}"))
}

pub(super) fn repair_error(error: impl std::fmt::Display) -> BackendError {
    BackendError::Malformed(format!("native serialized-object repair: {error}"))
}

pub(super) fn repair_object(
    document: &lopdf::Document,
    object: &lopdf::Object,
) -> Result<(Option<lopdf::ObjectId>, lopdf::Object), BackendError> {
    let mut object = object.clone();
    let mut target = None;
    let mut visited = HashSet::new();
    for _ in 0..8 {
        let lopdf::Object::Reference(reference) = object else {
            return Ok((target, object));
        };
        if !visited.insert(reference) {
            return Err(BackendError::Malformed(
                "cyclic reference in serialized repair".into(),
            ));
        }
        target = Some(reference);
        object = document
            .objects
            .get(&reference)
            .ok_or_else(|| {
                BackendError::Malformed("missing reference in serialized repair".into())
            })?
            .clone();
    }
    Err(BackendError::Limit(
        "serialized repair reference depth exceeds 8".into(),
    ))
}

pub(super) fn pdf_string(value: &str) -> lopdf::Object {
    // Always use UTF-16BE for non-ASCII rather than assuming Latin-1 is PDFDocEncoding.
    let bytes = if value.is_ascii() {
        value.as_bytes().to_vec()
    } else {
        let mut bytes = vec![0xfe, 0xff];
        bytes.extend(value.encode_utf16().flat_map(u16::to_be_bytes));
        bytes
    };
    lopdf::Object::string_literal(bytes)
}

pub(super) fn repaired_output(
    context: &Context<'_>,
    mut output: OperationOutput,
    mut document: lopdf::Document,
    repair: &str,
) -> Result<OperationOutput, BackendError> {
    let count = document.get_pages().len();
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).map_err(repair_error)?;
    let name = output.artifacts[0].name.clone();
    output.artifacts[0] = pdf_artifact(context, name, bytes, count)?;
    output.data["postserialization_repair"] = json!({"reader_writer": "lopdf", "version": "0.45.0",
        "upstream": "pdf_oxide 0.3.78", "repair": repair});
    output.warnings.push(format!("Applied explicit lopdf 0.45.0 serialized-object repair after pdf_oxide 0.3.78: {repair}. The repair targets requested dictionaries and references; complete document preservation and visual fidelity are not established."));
    Ok(output)
}

pub(super) fn bounded_count(count: usize, maximum: usize, label: &str) -> Result<(), BackendError> {
    if count > maximum {
        return Err(BackendError::Limit(format!(
            "{label}: {count} exceeds {maximum}"
        )));
    }
    Ok(())
}

pub(super) fn bounded_string(value: &str, label: &str) -> Result<(), BackendError> {
    bounded_count(value.len(), 65_536, label)?;
    if value.contains('\0') {
        return Err(BackendError::Malformed(format!("{label} contains NUL")));
    }
    Ok(())
}

pub(super) fn page_index(page: u32, count: usize) -> Result<usize, BackendError> {
    if page == 0 || usize::try_from(page).map_or(true, |p| p > count) {
        return Err(BackendError::PageRange {
            page,
            count: u32::try_from(count).unwrap_or(u32::MAX),
        });
    }
    Ok(page as usize - 1)
}

pub(super) fn rectangle(values: [f32; 4]) -> Result<Rect, BackendError> {
    let [x, y, width, height] = values;
    if values
        .iter()
        .any(|v| !v.is_finite() || v.abs() > 1_000_000.0)
        || width <= 0.0
        || height <= 0.0
    {
        return Err(BackendError::Malformed(
            "rectangle must be finite [x, y, positive width, positive height] within 1,000,000 points"
                .into(),
        ));
    }
    Ok(Rect::new(x, y, width, height))
}

pub(super) fn rgb(values: [f32; 3]) -> Result<[f32; 3], BackendError> {
    if values
        .iter()
        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err(BackendError::Malformed(
            "RGB channels must be within 0..=1".into(),
        ));
    }
    Ok(values)
}

pub(super) fn edit_document(context: &Context<'_>) -> Result<DocumentEditor, BackendError> {
    let editor = context.open_editor()?;
    if editor.source().is_encrypted() {
        return Err(BackendError::Unsupported(
            "editing encrypted inputs would discard encryption; explicitly decrypt a derivative first"
                .into(),
        ));
    }
    bounded_count(
        editor.current_page_count(),
        context.limits.max_pages as usize,
        "source pages scanned by editor",
    )?;
    Ok(editor)
}

pub(super) fn pdf_artifact(
    context: &Context<'_>,
    name: String,
    bytes: Vec<u8>,
    expected_pages: usize,
) -> Result<Artifact, BackendError> {
    if bytes.len() as u64 > context.limits.max_output_bytes {
        return Err(BackendError::Limit(
            "serialized PDF exceeds output byte budget".into(),
        ));
    }
    let check = PdfDocument::from_bytes(bytes.clone()).map_err(upstream)?;
    let actual_pages = check.page_count().map_err(upstream)?;
    if actual_pages != expected_pages {
        return Err(BackendError::Malformed(format!(
            "saved PDF page-count verification failed: expected {expected_pages}, got {actual_pages}"
        )));
    }
    Ok(Artifact {
        name,
        mime: "application/pdf".into(),
        bytes,
    })
}

pub(super) fn saved(
    context: &Context<'_>,
    editor: &mut DocumentEditor,
    mut data: Value,
) -> Result<OperationOutput, BackendError> {
    let count = editor.current_page_count();
    bounded_count(count, context.limits.max_pages as usize, "output pages")?;
    let bytes = editor.save_to_bytes().map_err(upstream)?;
    let mut warnings = editor.flatten_warnings().to_vec();
    let diagnostics = editor.source().take_structured_warnings();
    warnings.extend(
        diagnostics
            .iter()
            .map(|warning| format!("{}: {}", warning.category.as_str(), warning.message)),
    );
    data["diagnostics"] = json!(
        diagnostics
            .iter()
            .map(|warning| json!({
        "source": (context.inputs.len() == 1).then(|| context.inputs[0].sha256.as_str()),
        "input_sources": context.inputs.iter().map(|input| input.sha256.as_str()).collect::<Vec<_>>(),
        "attribution": "upstream document and thread-local warning sinks; nested-document attribution may be incomplete",
                "page": warning.page.map(|page| page + 1),
                "category": warning.category.as_str(), "message": warning.message,
                "spec_section": warning.spec_section,
            }))
            .collect::<Vec<_>>()
    );
    warnings.push(
        "Derivative is a full rewrite; structural round-trip checks do not establish visual fidelity or signature validity."
            .into(),
    );
    Ok(OperationOutput {
        data,
        artifacts: vec![pdf_artifact(context, "edited.pdf".into(), bytes, count)?],
        warnings,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageParams {
    /// Source page numbers, one based; order and repetitions are deliberate.
    #[serde(default)]
    select: Option<Vec<u32>>,
    #[serde(default)]
    rotations: Vec<Rotation>,
    #[serde(default)]
    boxes: Vec<PageBox>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rotation {
    page: u32,
    degrees: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageBox {
    page: u32,
    #[serde(default)]
    media: Option<[f32; 4]>,
    #[serde(default)]
    crop: Option<[f32; 4]>,
}

fn pdf_box(values: [f32; 4]) -> Result<[f32; 4], BackendError> {
    let [x0, y0, x1, y1] = values;
    rectangle([x0, y0, x1 - x0, y1 - y0])?;
    Ok(values)
}

fn pages(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let params: PageParams = parse(params)?;
    bounded_count(
        params.rotations.len() + params.boxes.len(),
        context.limits.max_items,
        "page edits",
    )?;
    let mut editor = edit_document(context)?;
    let count = editor.current_page_count();
    let selected = match params.select {
        Some(pages) => {
            if context.pages.is_some() {
                return Err(BackendError::Malformed(
                    "use either request pages or select, not both".into(),
                ));
            }
            bounded_count(
                pages.len(),
                context.limits.max_pages as usize,
                "selected pages",
            )?;
            if pages.is_empty() {
                return Err(BackendError::Malformed("select cannot be empty".into()));
            }
            pages
                .into_iter()
                .map(|p| page_index(p, count))
                .collect::<Result<Vec<_>, _>>()?
        }
        None => context.selected_pages(count)?,
    };
    for rotation in &params.rotations {
        if rotation.degrees % 90 != 0 {
            return Err(BackendError::Malformed(
                "rotation must be a multiple of 90 degrees".into(),
            ));
        }
        editor
            .set_page_rotation(
                page_index(rotation.page, count)?,
                rotation.degrees.rem_euclid(360),
            )
            .map_err(upstream)?;
    }
    for boxes in &params.boxes {
        let page = page_index(boxes.page, count)?;
        if let Some(media) = boxes.media {
            editor
                .set_page_media_box(page, pdf_box(media)?)
                .map_err(upstream)?;
        }
        if let Some(crop) = boxes.crop {
            editor
                .set_page_crop_box(page, pdf_box(crop)?)
                .map_err(upstream)?;
        }
    }
    // Edits address original source pages; selection/reordering is applied last.
    editor.select_pages(&selected).map_err(upstream)?;
    saved(
        context,
        &mut editor,
        json!({"source_pages": selected.iter().map(|p| p + 1).collect::<Vec<_>>() }),
    )
}

fn merge(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let _: Empty = parse(params)?;
    if context.pages.is_some() {
        return Err(BackendError::Unsupported(
            "select pages into derivatives before merging".into(),
        ));
    }
    if context.inputs.len() < 2 {
        return Err(BackendError::Malformed(
            "merge requires at least two input PDFs".into(),
        ));
    }
    bounded_count(
        context.inputs.len(),
        context.limits.max_items,
        "merge inputs",
    )?;
    let mut editor = edit_document(context)?;
    let mut total = editor.current_page_count();
    let mut counts = vec![total];
    // Validate every source and the total page budget before importing any object graph.
    for index in 1..context.inputs.len() {
        let doc = PdfDocument::from_bytes(context.input(index)?.to_vec()).map_err(upstream)?;
        if doc.is_encrypted() {
            return Err(BackendError::Unsupported(
                "merge sources must be explicitly decrypted first".into(),
            ));
        }
        super::security::reject_signed_mutation(&doc, context)?;
        let count = doc.page_count().map_err(upstream)?;
        total = total
            .checked_add(count)
            .ok_or_else(|| BackendError::Limit("merge page overflow".into()))?;
        bounded_count(total, context.limits.max_pages as usize, "merged pages")?;
        counts.push(count);
    }
    for (index, expected) in counts.iter().enumerate().skip(1) {
        let actual = editor
            .merge_from_bytes(context.input(index)?)
            .map_err(upstream)?;
        if actual != *expected {
            return Err(BackendError::Malformed(
                "merge imported an unexpected number of pages".into(),
            ));
        }
    }
    let mut output = saved(
        context,
        &mut editor,
        json!({"input_page_counts": counts, "pages": total}),
    )?;
    output.warnings.push("Merge imports page objects and dependencies; source-level outlines, attachments, labels, and form catalogs are not merged.".into());
    Ok(output)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SplitParams {
    /// One-based inclusive ranges, checked before expansion.
    ranges: Vec<[u32; 2]>,
}

fn split(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let params: SplitParams = parse(params)?;
    if context.pages.is_some() {
        return Err(BackendError::Malformed(
            "split ranges and request pages cannot be combined".into(),
        ));
    }
    bounded_count(
        params.ranges.len(),
        context.limits.max_items,
        "split ranges",
    )?;
    if params.ranges.is_empty() {
        return Err(BackendError::Malformed(
            "split ranges cannot be empty".into(),
        ));
    }
    let mut editor = edit_document(context)?;
    let count = editor.current_page_count();
    let mut total_pages = 0_u64;
    for &[start, end] in &params.ranges {
        page_index(start, count)?;
        page_index(end, count)?;
        if end < start {
            return Err(BackendError::Malformed(
                "split range end precedes start".into(),
            ));
        }
        total_pages += u64::from(end - start) + 1;
        if total_pages > u64::from(context.limits.max_pages) {
            return Err(BackendError::Limit(
                "aggregate split pages exceed page budget".into(),
            ));
        }
    }
    let mut output = OperationOutput::default();
    let mut total_bytes = 0_u64;
    for (index, &[start, end]) in params.ranges.iter().enumerate() {
        let pages: Vec<_> = ((start as usize - 1)..end as usize).collect();
        let bytes = editor.extract_pages_to_bytes(&pages).map_err(upstream)?;
        total_bytes = total_bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| BackendError::Limit("split byte overflow".into()))?;
        if total_bytes > context.limits.max_output_bytes {
            return Err(BackendError::Limit(
                "aggregate split output exceeds byte budget".into(),
            ));
        }
        output.artifacts.push(pdf_artifact(
            context,
            format!("split-{:04}.pdf", index + 1),
            bytes,
            pages.len(),
        )?);
    }
    output.data = json!({"ranges": params.ranges, "pages": total_pages});
    output.warnings.push("Page subsets are derivatives; cross-page links, document-level outline destinations and form catalog fidelity require review.".into());
    Ok(output)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataParams {
    /// Only these Info keys are accepted; null clears a key.
    fields: BTreeMap<String, Option<String>>,
}

fn metadata(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let params: MetadataParams = parse(params)?;
    let mut editor = edit_document(context)?;
    let mut info = editor.get_info().map_err(upstream)?;
    for (key, value) in &params.fields {
        if let Some(value) = value {
            bounded_string(value, "metadata value")?;
            // This pinned DocumentInfo writer emits raw UTF-8, while its reader uses
            // PDFDocEncoding. Reject rather than silently corrupt non-ASCII metadata.
            if !value.is_ascii() {
                return Err(BackendError::Unsupported(
                    "pdf-oxide 0.3.78 Info writer cannot round-trip non-ASCII metadata".into(),
                ));
            }
        }
        let target = match key.as_str() {
            "title" => &mut info.title,
            "author" => &mut info.author,
            "subject" => &mut info.subject,
            "keywords" => &mut info.keywords,
            "creator" => &mut info.creator,
            "producer" => &mut info.producer,
            "creation_date" => &mut info.creation_date,
            "modification_date" => &mut info.mod_date,
            _ => {
                return Err(BackendError::Unsupported(format!(
                    "unsupported Info metadata key: {key}"
                )));
            }
        };
        target.clone_from(value);
    }
    // Rewriting an unchanged non-ASCII Info value has the same upstream defect.
    if [
        &info.title,
        &info.author,
        &info.subject,
        &info.keywords,
        &info.creator,
        &info.producer,
        &info.creation_date,
        &info.mod_date,
    ]
    .iter()
    .filter_map(|v| v.as_ref())
    .any(|v| !v.is_ascii())
    {
        return Err(BackendError::Unsupported(
            "metadata rewrite would corrupt an existing non-ASCII Info value".into(),
        ));
    }
    editor.set_info(info).map_err(upstream)?;
    let mut output = saved(
        context,
        &mut editor,
        json!({"changed_info_keys": params.fields.keys().collect::<Vec<_>>() }),
    )?;
    output.warnings.push(
        "Only the Info dictionary changes; XMP metadata is preserved separately and may disagree."
            .into(),
    );
    Ok(output)
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum AnnotationParams {
    Inspect,
    Edit { changes: Vec<AnnotationChange> },
    Flatten,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum AnnotationChange {
    Add {
        page: u32,
        annotation: AnnotationSpec,
    },
    Update {
        page: u32,
        index: usize,
        #[serde(default)]
        contents: Option<String>,
        #[serde(default)]
        rect: Option<[f32; 4]>,
        #[serde(default)]
        color: Option<[f32; 3]>,
    },
    Remove {
        page: u32,
        index: usize,
    },
}

#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum AnnotationSpec {
    Text { rect: [f32; 4], contents: String },
    FreeText { rect: [f32; 4], contents: String },
    Highlight { rect: [f32; 4] },
    Underline { rect: [f32; 4] },
    Strikeout { rect: [f32; 4] },
    Squiggly { rect: [f32; 4] },
    Square { rect: [f32; 4] },
    Circle { rect: [f32; 4] },
    Stamp { rect: [f32; 4], name: String },
    LinkUri { rect: [f32; 4], uri: String },
    LinkPage { rect: [f32; 4], page: u32 },
}

fn new_annotation(spec: AnnotationSpec, count: usize) -> Result<Annotation, BackendError> {
    Ok(match spec {
        AnnotationSpec::Text { rect, contents } => {
            bounded_string(&contents, "annotation contents")?;
            TextAnnotation::new(rectangle(rect)?, contents).into()
        }
        AnnotationSpec::FreeText { rect, contents } => {
            bounded_string(&contents, "annotation contents")?;
            FreeTextAnnotation::new(rectangle(rect)?, contents).into()
        }
        AnnotationSpec::Highlight { rect } => {
            TextMarkupAnnotation::from_rect(TextMarkupType::Highlight, rectangle(rect)?).into()
        }
        AnnotationSpec::Underline { rect } => {
            TextMarkupAnnotation::from_rect(TextMarkupType::Underline, rectangle(rect)?).into()
        }
        AnnotationSpec::Strikeout { rect } => {
            TextMarkupAnnotation::from_rect(TextMarkupType::StrikeOut, rectangle(rect)?).into()
        }
        AnnotationSpec::Squiggly { rect } => {
            TextMarkupAnnotation::from_rect(TextMarkupType::Squiggly, rectangle(rect)?).into()
        }
        AnnotationSpec::Square { rect } => ShapeAnnotation::square(rectangle(rect)?).into(),
        AnnotationSpec::Circle { rect } => ShapeAnnotation::circle(rectangle(rect)?).into(),
        AnnotationSpec::Stamp { rect, name } => {
            safe_name(&name)?;
            StampAnnotation::custom(rectangle(rect)?, name).into()
        }
        AnnotationSpec::LinkUri { rect, uri } => {
            bounded_string(&uri, "annotation URI")?;
            if uri.chars().any(char::is_control)
                || !(uri.starts_with("https://") || uri.starts_with("http://"))
            {
                return Err(BackendError::Unsupported(
                    "URI annotations accept only explicit HTTP(S) links; no link is fetched".into(),
                ));
            }
            LinkAnnotation::uri(rectangle(rect)?, uri).into()
        }
        AnnotationSpec::LinkPage { rect, page } => {
            LinkAnnotation::goto_page(rectangle(rect)?, page_index(page, count)?).into()
        }
    })
}

fn annotations(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let params: AnnotationParams = parse(params)?;
    if matches!(params, AnnotationParams::Inspect) {
        let doc = context.open_pdf()?;
        let pages = context.selected_pages(doc.page_count().map_err(upstream)?)?;
        let mut records = Vec::new();
        for page in pages {
            for (index, annotation) in doc
                .get_annotations(page)
                .map_err(upstream)?
                .into_iter()
                .enumerate()
            {
                bounded_count(records.len() + 1, context.limits.max_items, "annotations")?;
                // The pinned annotation parser applies UTF-8-lossy decoding even
                // to UTF-16 PDF text strings. Decode the preserved raw dictionary.
                let text = |key: &str| -> Result<Option<String>, BackendError> {
                    let dict = annotation.raw_dict.as_ref().ok_or_else(|| {
                        BackendError::Unsupported(
                            "annotation lacks its original dictionary for text decoding".into(),
                        )
                    })?;
                    let Some(object) = dict.get(key) else {
                        return Ok(None);
                    };
                    let object = doc.resolve_references(object, 8).map_err(upstream)?;
                    let bytes = object.as_string().ok_or_else(|| {
                        BackendError::Malformed(format!(
                            "annotation {key} is not a PDF text string"
                        ))
                    })?;
                    bounded_count(bytes.len(), 65_536, "annotation text bytes")?;
                    Ok(Some(pdf_oxide::optional_content::decode_pdf_text_string(
                        bytes,
                    )))
                };
                let contents = text("Contents")?;
                let author = text("T")?;
                let subject = text("Subj")?;
                records.push(json!({"page": page + 1, "index": index,
                    "subtype": annotation.subtype, "contents": contents,
                    "rect": annotation.rect, "author": author,
                    "subject": subject, "color": annotation.color,
                    "text_encoding": "decoded from original PDF text string bytes",
                    "action": annotation.action.map(|a| format!("{a:?}")),
                    "destination": annotation.destination.map(|d| format!("{d:?}"))}));
            }
        }
        return Ok(OperationOutput {
            data: json!({"annotations": records}),
            ..Default::default()
        });
    }
    let mut editor = edit_document(context)?;
    let count = editor.current_page_count();
    let mut repairs = Vec::new();
    match params {
        AnnotationParams::Inspect => unreachable!(),
        AnnotationParams::Flatten => {
            for page in context.selected_pages(count)? {
                editor.flatten_page_annotations(page).map_err(upstream)?;
            }
        }
        AnnotationParams::Edit { changes } => {
            bounded_count(
                changes.len(),
                context.limits.max_items,
                "annotation changes",
            )?;
            let mut per_page = BTreeMap::<usize, Vec<AnnotationChange>>::new();
            for change in changes {
                let number = match &change {
                    AnnotationChange::Add { page, .. }
                    | AnnotationChange::Update { page, .. }
                    | AnnotationChange::Remove { page, .. } => *page,
                };
                let index = page_index(number, count)?;
                if let Some((first, last)) = context.pages
                    && !(first..=last).contains(&number)
                {
                    return Err(BackendError::Malformed(
                        "annotation edit is outside requested pages".into(),
                    ));
                }
                per_page.entry(index).or_default().push(change);
            }
            for (index, changes) in per_page {
                // get_page() reloads original annotations even after save_page().
                // Keep one DOM instance per changed page to retain earlier changes.
                editor.source().get_annotations(index).map_err(upstream)?;
                let mut page = editor.get_page(index).map_err(upstream)?;
                bounded_count(
                    page.annotation_count(),
                    context.limits.max_items,
                    "page annotations",
                )?;
                repairs.push((index, page.annotation_count(), changes.clone()));
                for change in changes {
                    match change {
                        AnnotationChange::Add { annotation, .. } => {
                            bounded_count(
                                page.annotation_count() + 1,
                                context.limits.max_items,
                                "page annotations",
                            )?;
                            page.add_annotation(new_annotation(annotation, count)?);
                        }
                        AnnotationChange::Update {
                            index,
                            contents,
                            rect,
                            color,
                            ..
                        } => {
                            let annotation = page.annotation_mut(index).ok_or_else(|| {
                                BackendError::Malformed("annotation index does not exist".into())
                            })?;
                            if let Some(contents) = contents {
                                bounded_string(&contents, "annotation contents")?;
                                annotation.set_contents(contents);
                            }
                            if let Some(rect) = rect {
                                annotation.set_rect(rectangle(rect)?);
                            }
                            if let Some(color) = color {
                                let [r, g, b] = rgb(color)?;
                                annotation.set_color(r, g, b);
                            }
                        }
                        AnnotationChange::Remove { index, .. } => {
                            page.remove_annotation(index).ok_or_else(|| {
                                BackendError::Malformed("annotation index does not exist".into())
                            })?;
                        }
                    }
                }
                editor.save_page(page).map_err(upstream)?;
            }
        }
    }
    let output = saved(context, &mut editor, json!({"annotations_modified": true}))?;
    if repairs.is_empty() {
        return Ok(output);
    }
    repair_annotations(context, output, repairs)
}

#[derive(Default)]
struct AnnotationPatch {
    contents: Option<String>,
    rect: Option<[f32; 4]>,
    color: Option<[f32; 3]>,
}

fn repair_annotations(
    context: &Context<'_>,
    output: OperationOutput,
    repairs: Vec<(usize, usize, Vec<AnnotationChange>)>,
) -> Result<OperationOutput, BackendError> {
    let mut document =
        lopdf::Document::load_mem(&output.artifacts[0].bytes).map_err(repair_error)?;
    let page_ids = document.get_pages();
    for (page, original_count, changes) in repairs {
        // Symbolic identities survive removals and additions in a single request.
        // The pinned writer appends only surviving new annotations but never edits
        // or removes original entries; reconcile exactly those known differences.
        let mut live: Vec<usize> = (0..original_count).collect();
        let mut next = original_count;
        let mut patches = BTreeMap::<usize, AnnotationPatch>::new();
        for change in changes {
            match change {
                AnnotationChange::Add { .. } => {
                    live.push(next);
                    next += 1;
                }
                AnnotationChange::Remove { index, .. } => {
                    live.remove(index);
                }
                AnnotationChange::Update {
                    index,
                    contents,
                    rect,
                    color,
                    ..
                } => {
                    let patch = patches.entry(live[index]).or_default();
                    if contents.is_some() {
                        patch.contents = contents;
                    }
                    if rect.is_some() {
                        patch.rect = rect;
                    }
                    if color.is_some() {
                        patch.color = color;
                    }
                }
            }
        }
        let new_symbols: Vec<_> = live
            .iter()
            .copied()
            .filter(|id| *id >= original_count)
            .collect();
        let page_id = *page_ids
            .get(&(page as u32 + 1))
            .ok_or_else(|| BackendError::Malformed("repair page missing".into()))?;
        let mut page_dict = document
            .get_object(page_id)
            .map_err(repair_error)?
            .as_dict()
            .map_err(repair_error)?
            .clone();
        let raw = match page_dict.get(b"Annots") {
            Ok(object) => repair_object(&document, object)?
                .1
                .as_array()
                .map_err(repair_error)?
                .clone(),
            Err(_) => Vec::new(),
        };
        if raw.len() != original_count + new_symbols.len() {
            return Err(BackendError::Unsupported("serialized annotation inventory differs from exact original plus surviving additions".into()));
        }
        let mut objects: BTreeMap<usize, lopdf::Object> = raw
            .iter()
            .take(original_count)
            .cloned()
            .enumerate()
            .collect();
        for (symbol, object) in new_symbols.iter().zip(raw.into_iter().skip(original_count)) {
            objects.insert(*symbol, object);
        }
        let mut final_array = Vec::with_capacity(live.len());
        for symbol in live {
            let mut object = objects.remove(&symbol).ok_or_else(|| {
                BackendError::Malformed("annotation identity missing during repair".into())
            })?;
            if let Some(patch) = patches.remove(&symbol) {
                let (target, resolved) = repair_object(&document, &object)?;
                let mut dict = resolved.as_dict().map_err(repair_error)?.clone();
                if let Some(contents) = patch.contents {
                    dict.set("Contents", pdf_string(&contents));
                }
                if let Some(rect) = patch.rect {
                    let rect = rectangle(rect)?;
                    dict.set(
                        "Rect",
                        vec![
                            rect.x.into(),
                            rect.y.into(),
                            (rect.x + rect.width).into(),
                            (rect.y + rect.height).into(),
                        ],
                    );
                }
                if let Some(color) = patch.color {
                    dict.set(
                        "C",
                        color
                            .into_iter()
                            .map(lopdf::Object::Real)
                            .collect::<Vec<_>>(),
                    );
                }
                if let Some(target) = target {
                    document
                        .objects
                        .insert(target, lopdf::Object::Dictionary(dict));
                } else {
                    object = lopdf::Object::Dictionary(dict);
                }
            }
            final_array.push(object);
        }
        if final_array.is_empty() {
            page_dict.remove(b"Annots");
        } else {
            page_dict.set("Annots", final_array);
        }
        document
            .objects
            .insert(page_id, lopdf::Object::Dictionary(page_dict));
    }
    repaired_output(
        context,
        output,
        document,
        "persist annotation updates/removals and preserve untouched annotation dictionaries",
    )
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum AttachmentParams {
    Extract,
    Add { files: Vec<AttachmentInput> },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AttachmentInput {
    input: usize,
    name: String,
}

fn safe_name(name: &str) -> Result<(), BackendError> {
    if name.is_empty()
        || name.len() > 128
        || name == "."
        || name == ".."
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(BackendError::Malformed(
            "name must be a plain ASCII filename of at most 128 bytes".into(),
        ));
    }
    Ok(())
}

fn has_embedded_files(doc: &PdfDocument) -> Result<bool, BackendError> {
    let catalog = doc.catalog().map_err(upstream)?;
    if let Some(names) = catalog.as_dict().and_then(|d| d.get("Names")) {
        // The writer clones only direct /Names dictionaries; resolving an indirect
        // dictionary here would hide that adding an attachment drops its other trees.
        if matches!(names, Object::Reference(_)) {
            return Ok(true);
        }
        let names = doc.resolve_references(names, 8).map_err(upstream)?;
        return Ok(names
            .as_dict()
            .is_some_and(|d| d.contains_key("EmbeddedFiles")));
    }
    Ok(false)
}

/// Decode names from their original PDF strings before the pinned attachment
/// reader irreversibly applies UTF-8-lossy decoding to UTF-16 /UF bytes.
/// Keys match the upstream projection only for associating decoded byte results.
pub(super) fn attachment_names(
    doc: &PdfDocument,
    maximum: usize,
) -> Result<BTreeMap<String, String>, BackendError> {
    let catalog = doc.catalog().map_err(upstream)?;
    let Some(names) = catalog.as_dict().and_then(|dict| dict.get("Names")) else {
        return Ok(BTreeMap::new());
    };
    let names = doc.resolve_references(names, 8).map_err(upstream)?;
    let Some(root) = names.as_dict().and_then(|dict| dict.get("EmbeddedFiles")) else {
        return Ok(BTreeMap::new());
    };
    let mut pending = vec![(root.clone(), 0_usize)];
    let mut visited = HashSet::new();
    let mut count = 0_usize;
    let mut filenames = BTreeMap::new();
    while let Some((object, depth)) = pending.pop() {
        bounded_count(depth, 32, "attachment name-tree depth")?;
        count += 1;
        bounded_count(count, maximum, "attachment name-tree nodes and entries")?;
        if let Some(reference) = object.as_reference()
            && !visited.insert(reference)
        {
            return Err(BackendError::Malformed(
                "cyclic or reused attachment name-tree node".into(),
            ));
        }
        let object = doc.resolve_references(&object, 8).map_err(upstream)?;
        let dict = object.as_dict().ok_or_else(|| {
            BackendError::Malformed("attachment name-tree node is not a dictionary".into())
        })?;
        if let Some(kids) = dict.get("Kids") {
            let kids = doc.resolve_references(kids, 8).map_err(upstream)?;
            let kids = kids
                .as_array()
                .ok_or_else(|| BackendError::Malformed("attachment Kids is not an array".into()))?;
            bounded_count(
                kids.len().saturating_add(count),
                maximum,
                "attachment name-tree children",
            )?;
            pending.extend(kids.iter().rev().cloned().map(|kid| (kid, depth + 1)));
        }
        if let Some(entries) = dict.get("Names") {
            let entries = doc.resolve_references(entries, 8).map_err(upstream)?;
            let entries = entries.as_array().ok_or_else(|| {
                BackendError::Malformed("attachment Names is not an array".into())
            })?;
            if entries.len() % 2 != 0 {
                return Err(BackendError::Malformed(
                    "odd attachment name-tree entry count".into(),
                ));
            }
            count = count.saturating_add(entries.len() / 2);
            bounded_count(count, maximum, "attachment name-tree entries")?;
            for pair in entries.as_chunks::<2>().0 {
                let spec = doc.resolve_references(&pair[1], 8).map_err(upstream)?;
                let spec = spec.as_dict().ok_or_else(|| {
                    BackendError::Malformed(
                        "attachment file specification is not a dictionary".into(),
                    )
                })?;
                let name = match spec.get("UF").or_else(|| spec.get("F")) {
                    Some(name) => doc.resolve_references(name, 8).map_err(upstream)?,
                    None => Object::String(b"attachment".to_vec()),
                };
                let bytes = name.as_string().ok_or_else(|| {
                    BackendError::Malformed("attachment file name is not a PDF string".into())
                })?;
                let projected = String::from_utf8_lossy(bytes).into_owned();
                let decoded = pdf_oxide::optional_content::decode_pdf_text_string(bytes);
                if let Some(previous) = filenames.insert(projected, decoded.clone())
                    && previous != decoded
                {
                    return Err(BackendError::Unsupported(
                        "ambiguous lossy upstream attachment-name projection".into(),
                    ));
                }
            }
        }
    }
    Ok(filenames)
}

fn attachments(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let params: AttachmentParams = parse(params)?;
    match params {
        AttachmentParams::Extract => {
            let doc = context.open_pdf()?;
            let source_names = attachment_names(&doc, context.limits.max_items)?;
            let files = doc.extract_embedded_files().map_err(upstream)?;
            bounded_count(files.len(), context.limits.max_items, "embedded files")?;
            let mut output = OperationOutput::default();
            let mut records = Vec::new();
            let mut total = 0_u64;
            for (index, (projected_name, bytes)) in files.into_iter().enumerate() {
                let source_name = source_names.get(&projected_name).ok_or_else(|| {
                    BackendError::Malformed(
                        "extracted attachment has no verified source filename".into(),
                    )
                })?;
                total = total
                    .checked_add(bytes.len() as u64)
                    .ok_or_else(|| BackendError::Limit("attachment byte overflow".into()))?;
                if total > context.limits.max_output_bytes {
                    return Err(BackendError::Limit(
                        "attachment output exceeds byte budget".into(),
                    ));
                }
                // Document-supplied names are metadata, never destination paths.
                let name = format!("attachment-{:04}.bin", index + 1);
                records.push(json!({"index": index, "source_name": source_name, "artifact": name, "bytes": bytes.len(),
                    "filename_encoding": "PDF text string decoded from original UF/F bytes", "upstream_name_projection": projected_name}));
                output.artifacts.push(Artifact {
                    name,
                    mime: "application/octet-stream".into(),
                    bytes,
                });
            }
            output.data = json!({"files": records});
            output.warnings.push("Upstream attachment extraction can omit malformed streams without an error; an empty result does not prove absence of attachments.".into());
            Ok(output)
        }
        AttachmentParams::Add { files } => {
            bounded_count(files.len(), context.limits.max_items, "attachment inputs")?;
            let mut editor = edit_document(context)?;
            if has_embedded_files(editor.source())? {
                return Err(BackendError::Unsupported("pinned editor replaces EmbeddedFiles trees and drops indirect Names dictionaries; adding to either existing structure is not supported".into()));
            }
            let mut names = HashSet::new();
            let mut total = 0_u64;
            for file in &files {
                safe_name(&file.name)?;
                if file.input == 0 || !names.insert(file.name.clone()) {
                    return Err(BackendError::Malformed(
                        "attachment input 0 and duplicate names are not allowed".into(),
                    ));
                }
                let bytes = context.input(file.input)?;
                total += bytes.len() as u64;
                if total > context.limits.max_input_bytes || total > context.limits.max_output_bytes
                {
                    return Err(BackendError::Limit(
                        "attachment input bytes exceed budget".into(),
                    ));
                }
                editor
                    .embed_file(&file.name, bytes.to_vec())
                    .map_err(upstream)?;
            }
            saved(
                context,
                &mut editor,
                json!({"attachments_added": files.len()}),
            )
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum BookmarkParams {
    Inspect,
}

fn bookmarks(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let _: BookmarkParams = parse(params)?;
    let doc = context.open_pdf()?;
    let catalog = doc.catalog().map_err(upstream)?;
    let mut records = Vec::new();
    if let Some(outlines) = catalog.as_dict().and_then(|d| d.get("Outlines")) {
        let outlines = doc.resolve_references(outlines, 8).map_err(upstream)?;
        let first = outlines.as_dict().and_then(|d| d.get("First")).cloned();
        let mut pending = first.into_iter().map(|o| (o, 0_usize)).collect::<Vec<_>>();
        let mut visited = HashSet::new();
        while let Some((object, depth)) = pending.pop() {
            bounded_count(records.len() + 1, context.limits.max_items, "bookmarks")?;
            bounded_count(depth, 32, "bookmark depth")?;
            let reference = object.as_reference().ok_or_else(|| {
                BackendError::Malformed("bookmark item must be an indirect object".into())
            })?;
            if !visited.insert(reference) {
                return Err(BackendError::Malformed(
                    "cyclic or reused bookmark node".into(),
                ));
            }
            let item = doc.load_object(reference).map_err(upstream)?;
            let dict = item.as_dict().ok_or_else(|| {
                BackendError::Malformed("bookmark item is not a dictionary".into())
            })?;
            let title = match dict.get("Title") {
                Some(value) => doc
                    .resolve_references(value, 8)
                    .map_err(upstream)?
                    .as_string()
                    .map(pdf_oxide::optional_content::decode_pdf_text_string),
                None => None,
            };
            // Preserve the destination object identity instead of inventing a page for
            // unresolved named destinations. No action is executed or resource fetched.
            let destination = dict.get("Dest").map(|value| match value {
                Object::Name(name) => json!({"named": name}),
                Object::String(bytes) => {
                    json!({"named": pdf_oxide::optional_content::decode_pdf_text_string(bytes)})
                }
                Object::Array(items) => {
                    json!({"page_object": items.first().and_then(Object::as_reference)})
                }
                _ => json!({"unresolved": true}),
            });
            records.push(json!({"object": reference, "depth": depth, "title": title, "destination": destination,
                "has_action": dict.contains_key("A")}));
            if let Some(next) = dict.get("Next") {
                pending.push((next.clone(), depth));
            }
            if let Some(child) = dict.get("First") {
                pending.push((child.clone(), depth + 1));
            }
        }
    }
    Ok(OperationOutput {
        data: json!({"bookmarks": records}),
        warnings: vec!["Existing-document bookmark modification is not exposed by the pinned DocumentEditor; bookmark creation is available in the creation operation.".into()],
        ..Default::default()
    })
}
