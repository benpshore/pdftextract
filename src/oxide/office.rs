//! Office conversion with archive/XML preflight and explicit layout semantics.

use std::collections::BTreeSet;
use std::io::{Cursor, Read};

use office_oxide::{Document, DocumentFormat};
use pdf_oxide::converters::office::{OfficeConfig, OfficeConverter, ir_to_pdf_bytes_pub};
use quick_xml::events::{BytesStart, Event};
use serde::Deserialize;
use serde_json::{Value, json};
use zip::{CompressionMethod, ZipArchive};

use super::create::{image_dimensions, invalid, parse_params, pdf_output, upstream};
use super::{Artifact, Context, OperationOutput};
use crate::backend::BackendError;

pub const OPERATIONS: &[&str] = &[
    "office.to_pdf",
    "office.from_pdf_docx",
    "office.from_pdf_pptx",
    "office.from_pdf_xlsx",
    "office.create_markdown",
];

const MAX_PART_BYTES: u64 = 8 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 4096;
const MAX_XML_DEPTH: usize = 64;

pub(super) fn execute(
    operation: &str,
    context: &Context<'_>,
    params: &Value,
) -> Option<Result<OperationOutput, BackendError>> {
    if OPERATIONS.contains(&operation) && context.inputs.len() != 1 {
        return Some(Err(invalid("Office operations require exactly one input")));
    }
    Some(match operation {
        "office.to_pdf" => to_pdf(context, params),
        "office.from_pdf_docx" => from_pdf(context, params, DocumentFormat::Docx),
        "office.from_pdf_pptx" => from_pdf(context, params, DocumentFormat::Pptx),
        "office.from_pdf_xlsx" => from_pdf(context, params, DocumentFormat::Xlsx),
        "office.create_markdown" => from_markdown(context, params),
        _ => return None,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportOptions {
    format: String,
    #[serde(default = "default_import_mode")]
    mode: String,
    #[serde(default = "enabled")]
    include_images: bool,
}

fn default_import_mode() -> String {
    "flow_no_system_fonts".into()
}
const fn enabled() -> bool {
    true
}

fn to_pdf(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    if context.pages.is_some() {
        return Err(BackendError::Unsupported(
            "Office import does not accept PDF page selection".into(),
        ));
    }
    let options: ImportOptions = parse_params(params)?;
    let format = office_format(&options.format)?;
    let bytes = context.input(0)?;
    let report = preflight(bytes, context, format)?;
    let ir = {
        let document = Document::from_reader(Cursor::new(bytes.to_vec()), format)
            .map_err(|error| invalid(&format!("Office parse: {error}")))?;
        document.to_ir()
    };
    validate_ir(&ir, context)?;
    let config = OfficeConfig {
        include_images: options.include_images,
        ..OfficeConfig::default()
    };
    let pdf = match options.mode.as_str() {
        "flow_no_system_fonts" => {
            // Unlike OfficeConverter, this public IR renderer never calls the
            // fixed-path Unicode/CJK fallback loader. It intentionally omits
            // embedded source fonts and positional import routing.
            ir_to_pdf_bytes_pub(&ir, &config).map_err(upstream)?
        }
        "native" => {
            // Pinned OfficeConverter reads fixed host font paths for some text.
            // Admission is process state, never an opt-out supplied in JSON.
            context.require_filesystem_containment()?;
            drop(ir);
            let converter = OfficeConverter::with_config(config);
            match format {
                DocumentFormat::Docx => converter.convert_docx_bytes(bytes),
                DocumentFormat::Xlsx => converter.convert_xlsx_bytes(bytes),
                DocumentFormat::Pptx => converter.convert_pptx_bytes(bytes),
                _ => unreachable!(),
            }
            .map_err(upstream)?
        }
        _ => {
            return Err(BackendError::Unsupported(
                "Office import mode must be flow_no_system_fonts or native".into(),
            ));
        }
    };
    let mut output = pdf_output(pdf, context)?;
    output.artifacts[0].name = "converted.pdf".into();
    output.data["format"] = json!(format.extension());
    output.data["mode"] = json!(options.mode);
    output.data["archive"] = report;
    output.warnings.push("Office conversion is a derivative, not a fidelity guarantee. Tables, formulas, floating content, fonts and pagination may change; upstream can omit unsupported content without returning an error.".into());
    if options.mode == "flow_no_system_fonts" {
        output.warnings.push("Flow import avoids host font discovery and does not preserve embedded fonts or positioned layout; Standard-14 glyph coverage is limited. The source archive remains unchanged.".into());
    } else {
        output.warnings.push("Native Office import may embed fixed system Unicode/CJK fallback fonts; output can vary with the confined worker's installed fonts and their licenses.".into());
    }
    if !options.include_images {
        output
            .warnings
            .push("Images were explicitly omitted by include_images=false.".into());
    }
    Ok(output)
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ExportOptions {
    mode: String,
}
impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            mode: "auto".into(),
        }
    }
}

fn from_pdf(
    context: &Context<'_>,
    params: &Value,
    format: DocumentFormat,
) -> Result<OperationOutput, BackendError> {
    let options: ExportOptions = parse_params(params)?;
    let document = context.open_pdf()?;
    let count = document.page_count().map_err(upstream)?;
    let selected = context.selected_pages(count)?;
    if selected.len() != count {
        return Err(BackendError::Unsupported(
            "PDF-to-Office converts a whole document; extract selected pages first".into(),
        ));
    }
    let mode = export_mode(format, &options.mode, count)?;
    let image_report = pdf_preflight(&document, count, context)?;
    // Use explicit APIs after fallible page_count. Upstream auto entry points
    // use page_count().unwrap_or(0), hiding malformed page trees.
    let bytes = match (format, mode) {
        (DocumentFormat::Docx, "layout") => document.to_docx_bytes_layout(),
        (DocumentFormat::Docx, "flow") => document.to_docx_bytes_flow(),
        (DocumentFormat::Pptx, "layout") => {
            pdf_oxide::converters::pptx_layout::to_pptx_bytes_layout(&document)
        }
        (DocumentFormat::Pptx, "flow") => document.to_pptx_bytes_flow(),
        (DocumentFormat::Xlsx, "layout") => {
            pdf_oxide::converters::xlsx_layout::to_xlsx_bytes_layout(&document)
        }
        (DocumentFormat::Xlsx, "flow") => document.to_xlsx_bytes_flow(),
        _ => unreachable!(),
    }
    .map_err(upstream)?;
    let mut output = office_output(bytes, format, context)?;
    output.data["source_pages"] = json!(count);
    output.data["requested_mode"] = json!(options.mode);
    output.data["effective_mode"] = json!(mode);
    output.data["auto_layout_max_pages"] = json!(layout_threshold(format));
    output.data["content_representation"] = json!(match (format, mode) {
        (DocumentFormat::Xlsx, "layout") => "positioned_drawing_text_and_images",
        (DocumentFormat::Xlsx, _) => "worksheet_cells",
        (DocumentFormat::Docx, "layout") => "positioned_paragraph_frames",
        (DocumentFormat::Pptx, "layout") => "positioned_slide_shapes",
        _ => "flow_paragraphs",
    });
    output.data["image_preflight"] = image_report;
    output.warnings.push("Office output is editable structured content, not proof of visual or semantic fidelity. Upstream image/font/vector conversion may omit or substitute content; source PDF remains unchanged.".into());
    if mode == "flow" {
        output.warnings.push("Flow export can reflow text, change page/slide counts and collapse positioning; XLSX flow places content in cells rather than preserving source coordinates.".into());
    }
    Ok(output)
}

fn export_mode(
    format: DocumentFormat,
    requested: &str,
    count: usize,
) -> Result<&str, BackendError> {
    match requested {
        "auto" => Ok(if count <= layout_threshold(format) {
            "layout"
        } else {
            "flow"
        }),
        "flow" | "layout" => Ok(requested),
        _ => Err(BackendError::Unsupported(
            "Office export mode must be auto, flow or layout".into(),
        )),
    }
}

fn layout_threshold(format: DocumentFormat) -> usize {
    if format == DocumentFormat::Xlsx {
        200
    } else {
        30
    }
}

fn pdf_preflight(
    document: &pdf_oxide::PdfDocument,
    count: usize,
    context: &Context<'_>,
) -> Result<Value, BackendError> {
    let mut images = 0_usize;
    let mut pixels = 0_u64;
    let mut spans = 0_usize;
    let mut encoded_bytes = 0_u64;
    let mut compressed_bytes = 0_u64;
    for index in 0..count {
        let page_spans = document.extract_spans(index).map_err(upstream)?;
        spans = spans
            .checked_add(page_spans.len())
            .ok_or_else(|| BackendError::Limit("span count overflow".into()))?;
        if spans > context.limits.max_items {
            return Err(BackendError::Limit(
                "PDF-to-Office span count exceeds max_items".into(),
            ));
        }
        crate::backend::pdf_oxide_images::reject_unsupported_masks(document, index)?;
        let handles = document.page_image_handles(index).map_err(upstream)?;
        images = images
            .checked_add(handles.len())
            .ok_or_else(|| BackendError::Limit("image count overflow".into()))?;
        if images > context.limits.max_items {
            return Err(BackendError::Limit(
                "PDF-to-Office image count exceeds max_items".into(),
            ));
        }
        // Inspect all page image envelopes before invoking a pixel decoder.
        // Pinned extract_images decodes first and can suppress decode failures.
        for handle in &handles {
            pixels = pixels
                .checked_add(u64::from(handle.width) * u64::from(handle.height))
                .ok_or_else(|| BackendError::Limit("image pixel overflow".into()))?;
            if handle.width == 0
                || handle.height == 0
                || handle.width > 65_535
                || handle.height > 65_535
                || pixels > context.limits.max_pixels
            {
                return Err(BackendError::Limit(
                    "PDF-to-Office image dimensions/pixels exceed limits or are zero".into(),
                ));
            }
            compressed_bytes = compressed_bytes
                .checked_add(handle.byte_size_compressed)
                .ok_or_else(|| BackendError::Limit("compressed image byte overflow".into()))?;
            if handle.byte_size_compressed > MAX_PART_BYTES
                || compressed_bytes > context.limits.max_input_bytes
            {
                return Err(BackendError::Limit(
                    "PDF-to-Office compressed images exceed input limits".into(),
                ));
            }
            if !matches!(handle.bits_per_component, 1 | 2 | 4 | 8 | 16) {
                return Err(invalid("PDF image has an invalid bits-per-component value"));
            }
        }
        for handle in handles {
            let image = handle.decode().map_err(upstream)?;
            if image.width() != handle.width || image.height() != handle.height {
                return Err(invalid(
                    "decoded PDF image dimensions disagree with inspected metadata",
                ));
            }
            // The actual conversion catches these encoder errors and continues.
            // Preflight makes a known encoder failure visible instead.
            let png = image.to_png_bytes().map_err(upstream)?;
            encoded_bytes = encoded_bytes
                .checked_add(u64::try_from(png.len()).unwrap_or(u64::MAX))
                .ok_or_else(|| BackendError::Limit("image byte overflow".into()))?;
            if png.is_empty()
                || u64::try_from(png.len()).unwrap_or(u64::MAX) > MAX_PART_BYTES
                || encoded_bytes > context.limits.max_output_bytes
            {
                return Err(BackendError::Limit(
                    "PDF-to-Office image encoding exceeds output limits or is empty".into(),
                ));
            }
            if png.len() < 24
                || !png.starts_with(b"\x89PNG\r\n\x1a\n")
                || u32::from_be_bytes(png[16..20].try_into().expect("four bytes")) != handle.width
                || u32::from_be_bytes(png[20..24].try_into().expect("four bytes")) != handle.height
            {
                return Err(invalid(
                    "encoded PNG dimensions disagree with inspected PDF image",
                ));
            }
        }
    }
    Ok(
        json!({"spans":spans,"images_inspected":images,"pixels":pixels,
        "compressed_image_bytes":compressed_bytes,"encoded_png_bytes":encoded_bytes,
        "completeness":"unverified: upstream handle enumeration can suppress errors; conversion can omit small images or substitute content"}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkdownOptions {
    format: String,
}

fn from_markdown(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    if context.pages.is_some() {
        return Err(BackendError::Unsupported(
            "Office Markdown creation does not accept page selection".into(),
        ));
    }
    let options: MarkdownOptions = parse_params(params)?;
    let format = office_format(&options.format)?;
    let source = context.input(0)?;
    if source.len() > 4 * 1024 * 1024 {
        return Err(BackendError::Limit("Office Markdown exceeds 4 MiB".into()));
    }
    let markdown =
        std::str::from_utf8(source).map_err(|_| invalid("Office Markdown must be UTF-8"))?;
    if markdown.lines().count() > context.limits.max_items {
        return Err(BackendError::Limit(
            "Office Markdown line count exceeds max_items".into(),
        ));
    }
    let ir = office_oxide::ir::DocumentIR::from_markdown(markdown, format);
    validate_ir(&ir, context)?;
    let mut output_bytes = Cursor::new(Vec::new());
    office_oxide::create::create_from_ir_to_writer(&ir, format, &mut output_bytes)
        .map_err(|error| invalid(&format!("Office creation: {error}")))?;
    let mut output = office_output(output_bytes.into_inner(), format, context)?;
    output.warnings.push("Office Markdown creation uses upstream's supported Markdown subset; rendering and application interoperability require review. No external resource is fetched.".into());
    Ok(output)
}

fn office_output(
    bytes: Vec<u8>,
    format: DocumentFormat,
    context: &Context<'_>,
) -> Result<OperationOutput, BackendError> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > context.limits.max_output_bytes {
        return Err(BackendError::Limit(
            "Office bytes exceed max_output_bytes".into(),
        ));
    }
    let archive = preflight(&bytes, context, format)?;
    Ok(OperationOutput {
        data: json!({"format":format.extension(),"archive":archive,"source_sha256":context.inputs.iter().map(|input|input.sha256.as_str()).collect::<Vec<_>>(),"fidelity":"unverified"}),
        artifacts: vec![Artifact {
            name: format!("converted.{}", format.extension()),
            mime: format.mime_type().into(),
            bytes,
        }],
        warnings: Vec::new(),
    })
}

fn office_format(format: &str) -> Result<DocumentFormat, BackendError> {
    match format {
        "docx" => Ok(DocumentFormat::Docx),
        "xlsx" => Ok(DocumentFormat::Xlsx),
        "pptx" => Ok(DocumentFormat::Pptx),
        _ => Err(BackendError::Unsupported("supported Office containers are docx, xlsx and pptx; legacy CFB, macro-enabled and encrypted Office containers are not admitted".into())),
    }
}

/// Validate every archive member, including ones upstream might ignore. Do not
/// trust ZIP declared sizes, permissive XML parsing, or CRC-error recovery.
fn preflight(
    bytes: &[u8],
    context: &Context<'_>,
    format: DocumentFormat,
) -> Result<Value, BackendError> {
    if !bytes.starts_with(b"PK\x03\x04") {
        return Err(BackendError::Unsupported(
            "Office input must be an unencrypted OOXML ZIP container".into(),
        ));
    }
    let declared_entries = zip_directory_count(bytes, context)?;
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| invalid(&format!("Office ZIP: {error}")))?;
    // zip 8.6 stores central-directory entries in a map and silently replaces
    // duplicate names. Compare the uncollapsed entry count before inspecting.
    if archive.len() != declared_entries {
        return Err(invalid("Office ZIP contains duplicate directory members"));
    }
    if archive.len() > MAX_ARCHIVE_ENTRIES.min(context.limits.max_items) {
        return Err(BackendError::Limit(
            "Office ZIP entry count exceeds limit".into(),
        ));
    }
    if archive
        .has_overlapping_files()
        .map_err(|error| invalid(&format!("Office ZIP extents: {error}")))?
    {
        return Err(invalid("Office ZIP members overlap"));
    }
    let mut expanded = 0_u64;
    let mut names = BTreeSet::new();
    let mut xml = XmlBudget::default();
    let mut images = 0_usize;
    let mut pixels = 0_u64;
    for index in 0..archive.len() {
        let mut member = archive
            .by_index(index)
            .map_err(|error| invalid(&format!("Office ZIP entry: {error}")))?;
        let name = member.name().to_string();
        let lower = name.to_ascii_lowercase();
        if !safe_member_name(&name)
            || member.enclosed_name().is_none()
            || member.is_symlink()
            || !names.insert(lower.clone())
        {
            return Err(invalid(
                "Office ZIP contains a noncanonical, duplicate or symlink member",
            ));
        }
        if member.encrypted()
            || !matches!(
                member.compression(),
                CompressionMethod::Stored | CompressionMethod::Deflated
            )
        {
            return Err(BackendError::Unsupported(
                "encrypted or unsupported Office ZIP compression".into(),
            ));
        }
        if forbidden_part(&lower) {
            return Err(BackendError::Unsupported(format!(
                "active/embedded Office content is not admitted: {name}"
            )));
        }
        let part_limit = MAX_PART_BYTES.min(context.limits.max_input_bytes);
        if member.size() > part_limit {
            return Err(BackendError::Limit(
                "Office ZIP member declared size exceeds limit".into(),
            ));
        }
        let mut data = Vec::new();
        (&mut member)
            .take(part_limit + 1)
            .read_to_end(&mut data)
            .map_err(|error| invalid(&format!("Office ZIP data or CRC: {error}")))?;
        let actual = u64::try_from(data.len()).unwrap_or(u64::MAX);
        if actual > part_limit || actual != member.size() {
            return Err(BackendError::Limit(
                "Office ZIP member expanded size exceeds limit or declaration".into(),
            ));
        }
        expanded = expanded
            .checked_add(actual)
            .ok_or_else(|| BackendError::Limit("Office ZIP expanded size overflow".into()))?;
        if expanded > MAX_EXPANDED_BYTES.min(context.limits.max_input_bytes.saturating_mul(8)) {
            return Err(BackendError::Limit(
                "Office ZIP total expanded bytes exceed limit".into(),
            ));
        }
        if member.is_dir() {
            if !data.is_empty() {
                return Err(invalid("Office ZIP directory contains data"));
            }
            continue;
        }
        #[allow(
            clippy::case_sensitive_file_extension_comparisons,
            reason = "ZIP names are validated as ASCII and normalized to lowercase before this check"
        )]
        let is_xml = lower.ends_with(".xml") || lower.ends_with(".rels");
        if is_xml {
            inspect_xml(&data, &name, context, &mut xml)?;
        } else if lower.contains("/media/") || lower.starts_with("docprops/thumbnail.") {
            let (_, _, count) = image_dimensions(&data, context)?;
            // Also decode PNG now: Office render_image otherwise swallows decode
            // errors. JPEG remains compressed upstream; this is not a complete
            // codec validation claim.
            pdf_oxide::writer::ImageData::from_bytes(&data)
                .map_err(|error| invalid(&format!("Office image: {error}")))?;
            pixels = pixels
                .checked_add(count)
                .ok_or_else(|| BackendError::Limit("Office pixels overflow".into()))?;
            images += 1;
            if images > context.limits.max_items || pixels > context.limits.max_pixels {
                return Err(BackendError::Limit(
                    "Office images exceed count/pixel limits".into(),
                ));
            }
        } else if !(lower.contains("/fonts/")
            && [".ttf", ".otf", ".odttf", ".fntdata"]
                .iter()
                .any(|suffix| lower.ends_with(suffix)))
        {
            return Err(BackendError::Unsupported(format!(
                "unreviewed Office binary part: {name}"
            )));
        }
    }
    let required = match format {
        DocumentFormat::Docx => "word/document.xml",
        DocumentFormat::Xlsx => "xl/workbook.xml",
        DocumentFormat::Pptx => "ppt/presentation.xml",
        _ => unreachable!(),
    };
    if !names.contains("[content_types].xml") || !names.contains(required) {
        return Err(invalid(
            "OOXML content types or selected format's root part is missing",
        ));
    }
    for target in &xml.internal_targets {
        if !names.contains(target) {
            return Err(invalid(&format!(
                "Office relationship points to missing part: {target}"
            )));
        }
    }
    Ok(
        json!({"entries":archive.len(),"expanded_bytes":expanded,"xml_events":xml.events,
        "cells":xml.cells,"sheets_or_slides":xml.pages,"images":images,"image_pixels":pixels,
        "validation":"bounded ZIP/XML preflight; not a sandbox or full Office conformance validator"}),
    )
}

fn safe_member_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 512
        && name.is_ascii()
        && !name.starts_with('/')
        && !name.contains(['\\', ':', '%', '\0'])
        && !name.chars().any(char::is_control)
        && name
            .trim_end_matches('/')
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Bound central-directory allocation before handing the archive to zip.
/// Inputs are <=64 MiB, so ZIP64 and split/self-extracting archives are outside
/// the contract. This checks the envelope only; zip verifies member data/CRC.
fn zip_directory_count(bytes: &[u8], context: &Context<'_>) -> Result<usize, BackendError> {
    if bytes.len() < 22 {
        return Err(invalid("Office ZIP is truncated"));
    }
    let start = bytes.len().saturating_sub(65_557);
    let end = (start..=bytes.len() - 22)
        .rev()
        .find(|offset| {
            bytes[*offset..].starts_with(b"PK\x05\x06")
                && usize::from(u16::from_le_bytes([
                    bytes[*offset + 20],
                    bytes[*offset + 21],
                ])) + *offset
                    + 22
                    == bytes.len()
        })
        .ok_or_else(|| invalid("Office ZIP has no final central-directory record"))?;
    let field = |offset| {
        usize::from(u16::from_le_bytes([
            bytes[end + offset],
            bytes[end + offset + 1],
        ]))
    };
    let count = field(10);
    if field(4) != 0 || field(6) != 0 || field(8) != count || count == 65_535 {
        return Err(BackendError::Unsupported(
            "split and ZIP64 Office archives are not admitted".into(),
        ));
    }
    if count > MAX_ARCHIVE_ENTRIES.min(context.limits.max_items) {
        return Err(BackendError::Limit(
            "Office ZIP declared entry count exceeds limit".into(),
        ));
    }
    let length = u32::from_le_bytes(bytes[end + 12..end + 16].try_into().expect("four bytes"));
    let offset = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().expect("four bytes"));
    if length == u32::MAX
        || offset == u32::MAX
        || u64::from(length) + u64::from(offset) != end as u64
    {
        return Err(BackendError::Unsupported(
            "Office ZIP central-directory layout is noncanonical or ZIP64".into(),
        ));
    }
    Ok(count)
}

fn forbidden_part(name: &str) -> bool {
    [
        "vbaproject",
        "vbadata",
        "activex",
        "/embeddings/",
        "/externallinks/",
        "connections.xml",
        "/querytables/",
        "/customui/",
        "macrosheet",
        "oleobject",
        "attachedtemplate",
    ]
    .iter()
    .any(|token| name.contains(token))
}

#[derive(Default)]
struct XmlBudget {
    events: usize,
    cells: usize,
    pages: usize,
    internal_targets: Vec<String>,
}

fn inspect_xml(
    data: &[u8],
    name: &str,
    context: &Context<'_>,
    budget: &mut XmlBudget,
) -> Result<(), BackendError> {
    let source = std::str::from_utf8(data).map_err(|_| {
        BackendError::Unsupported("Office XML preflight supports UTF-8 only".into())
    })?;
    if source.chars().any(|character| {
        (character < ' ' && !matches!(character, '\t' | '\n' | '\r'))
            || matches!(character, '\u{fffe}' | '\u{ffff}')
    }) {
        return Err(invalid(
            "Office XML contains characters forbidden by XML 1.0",
        ));
    }
    let mut reader = quick_xml::Reader::from_str(source);
    reader.config_mut().check_comments = true;
    reader.config_mut().check_end_names = true;
    let mut depth = 0_usize;
    let mut roots = 0_usize;
    let mut declaration_seen = false;
    loop {
        let event = reader
            .read_event()
            .map_err(|error| invalid(&format!("Office XML {name}: {error}")))?;
        budget.events += 1;
        if budget.events > context.limits.max_items.saturating_mul(32) {
            return Err(BackendError::Limit(
                "Office XML total event count exceeds limit".into(),
            ));
        }
        match event {
            Event::Start(element) => {
                if depth == 0 {
                    roots += 1;
                }
                depth += 1;
                if depth > MAX_XML_DEPTH {
                    return Err(BackendError::Limit("Office XML nesting exceeds 64".into()));
                }
                inspect_element(&element, name, &reader, context, budget)?;
            }
            Event::Empty(element) => {
                if depth == 0 {
                    roots += 1;
                }
                inspect_element(&element, name, &reader, context, budget)?;
            }
            Event::End(_) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| invalid("Office XML unmatched end tag"))?;
            }
            Event::DocType(_) | Event::PI(_) => return Err(BackendError::Unsupported(
                "Office XML DTD/entity declarations and processing instructions are not admitted"
                    .into(),
            )),
            Event::Decl(declaration) => {
                if declaration_seen || depth != 0 || roots != 0 {
                    return Err(invalid(
                        "Office XML declaration is repeated or outside its prolog",
                    ));
                }
                declaration_seen = true;
                if declaration
                    .version()
                    .map_err(|error| invalid(&format!("XML version: {error}")))?
                    .as_ref()
                    != b"1.0"
                {
                    return Err(BackendError::Unsupported(
                        "Office preflight supports XML 1.0 only".into(),
                    ));
                }
                if declaration
                    .encoding()
                    .transpose()
                    .map_err(|error| invalid(&format!("XML encoding: {error}")))?
                    .is_some_and(|encoding| !encoding.eq_ignore_ascii_case(b"utf-8"))
                {
                    return Err(BackendError::Unsupported(
                        "Office XML declaration must use UTF-8".into(),
                    ));
                }
            }
            Event::Text(text) => {
                let decoded = text
                    .decode()
                    .map_err(|error| invalid(&format!("Office XML text: {error}")))?;
                quick_xml::escape::unescape(&decoded)
                    .map_err(|error| invalid(&format!("Office XML entity: {error}")))?;
                if depth == 0 && !decoded.trim().is_empty() {
                    return Err(invalid("Office XML text outside root"));
                }
            }
            Event::GeneralRef(reference) => {
                if depth == 0 {
                    return Err(invalid("Office XML entity reference outside root"));
                }
                let reference = reference
                    .decode()
                    .map_err(|error| invalid(&format!("Office XML entity: {error}")))?;
                let encoded = format!("&{reference};");
                quick_xml::escape::unescape(&encoded)
                    .map_err(|error| invalid(&format!("Office XML entity: {error}")))?;
            }
            Event::CData(_) if depth == 0 => {
                return Err(invalid("Office XML CDATA outside root"));
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if depth != 0 || roots != 1 {
        return Err(invalid("Office XML must contain one complete root element"));
    }
    Ok(())
}

fn inspect_element(
    element: &BytesStart<'_>,
    name: &str,
    reader: &quick_xml::Reader<&[u8]>,
    context: &Context<'_>,
    budget: &mut XmlBudget,
) -> Result<(), BackendError> {
    let element_name = element.local_name();
    let local = std::str::from_utf8(element_name.as_ref())
        .map_err(|_| invalid("Office XML name must be UTF-8"))?;
    if [
        "oleObject",
        "object",
        "control",
        "altChunk",
        "externalLink",
        "connection",
        "attachedTemplate",
        "script",
    ]
    .iter()
    .any(|forbidden| local.eq_ignore_ascii_case(forbidden))
    {
        return Err(BackendError::Unsupported(format!(
            "active/embedded Office XML element: {local}"
        )));
    }
    if local == "sheet" || local == "sldId" {
        budget.pages += 1;
        if budget.pages > context.limits.max_pages as usize {
            return Err(BackendError::Limit(
                "Office sheet/slide count exceeds max_pages".into(),
            ));
        }
    }
    if local == "c" && name.starts_with("xl/worksheets/") {
        budget.cells += 1;
        if budget.cells > context.limits.max_items {
            return Err(BackendError::Limit(
                "Office cell count exceeds max_items".into(),
            ));
        }
    }
    let mut target = None;
    let mut attr_count = 0_usize;
    for attribute in element.attributes() {
        let attribute =
            attribute.map_err(|error| invalid(&format!("Office XML attribute: {error}")))?;
        attr_count += 1;
        if attr_count > 64 {
            return Err(BackendError::Limit(
                "Office XML attributes exceed 64 per element".into(),
            ));
        }
        let attr_name = attribute.key.local_name();
        let key = std::str::from_utf8(attr_name.as_ref())
            .map_err(|_| invalid("Office XML attribute name must be UTF-8"))?;
        let value = attribute
            .decoded_and_normalized_value(quick_xml::XmlVersion::Explicit1_0, reader.decoder())
            .map_err(|error| invalid(&format!("Office XML attribute: {error}")))?;
        if value.len() > 16 * 1024 {
            return Err(BackendError::Limit(
                "Office XML attribute value exceeds 16 KiB".into(),
            ));
        }
        if local == "Relationship" {
            if key == "TargetMode" && !value.eq_ignore_ascii_case("Internal") {
                return Err(BackendError::Unsupported(
                    "external Office relationships are not admitted".into(),
                ));
            }
            if key == "Target" {
                target = Some(value.to_string());
            }
            if key == "Type"
                && [
                    "vba",
                    "oleobject",
                    "activex",
                    "attachedtemplate",
                    "external",
                    "afchunk",
                ]
                .iter()
                .any(|token| value.to_ascii_lowercase().contains(token))
            {
                return Err(BackendError::Unsupported(
                    "active Office relationship type is not admitted".into(),
                ));
            }
        }
        if key == "ContentType"
            && ["macroenabled", "vbaproject", "oleobject", "activex"]
                .iter()
                .any(|token| value.to_ascii_lowercase().contains(token))
        {
            return Err(BackendError::Unsupported(
                "macro/active Office content type is not admitted".into(),
            ));
        }
        // XLSX positions can otherwise expand a tiny sparse input into huge
        // row/column grids during conversion.
        if name.starts_with("xl/worksheets/") {
            if (local == "c" && key == "r")
                || (matches!(local, "dimension" | "mergeCell") && key == "ref")
            {
                cell_range(&value, context.limits.max_items)?;
            }
            if local == "row"
                && key == "r"
                && value.parse::<u64>().map_or(true, |row| {
                    row == 0 || row > context.limits.max_items as u64
                })
            {
                return Err(BackendError::Limit(
                    "Office worksheet row index exceeds limit".into(),
                ));
            }
        }
        if matches!(local, "gridSpan" | "vMerge")
            && key == "val"
            && value
                .parse::<u64>()
                .is_ok_and(|number| number > context.limits.max_items as u64)
        {
            return Err(BackendError::Limit(
                "Office table span exceeds max_items".into(),
            ));
        }
    }
    if local == "Relationship" {
        let target = target.ok_or_else(|| invalid("Office relationship lacks Target"))?;
        budget.internal_targets.push(resolve_target(name, &target)?);
    }
    Ok(())
}

fn resolve_target(relationship_part: &str, target: &str) -> Result<String, BackendError> {
    if target.is_empty()
        || !target.is_ascii()
        || target.contains([':', '\\', '%', '\0', '#', '?'])
        || target.chars().any(char::is_control)
    {
        return Err(BackendError::Unsupported(
            "Office relationship target must be a canonical internal part".into(),
        ));
    }
    let base = if relationship_part == "_rels/.rels" {
        ""
    } else {
        relationship_part
            .rsplit_once("/_rels/")
            .map_or("", |(base, _)| base)
    };
    let mut parts: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        base.split('/').filter(|part| !part.is_empty()).collect()
    };
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(invalid("Office relationship escapes package root"));
                }
            }
            part => parts.push(part),
        }
    }
    Ok(parts.join("/").to_ascii_lowercase())
}

fn cell_range(value: &str, max_items: usize) -> Result<(), BackendError> {
    for coordinate in value.split(':') {
        let coordinate = coordinate.trim_matches('$');
        let mut column = 0_u64;
        let mut split = 0;
        for (index, character) in coordinate.bytes().enumerate() {
            if !character.is_ascii_alphabetic() {
                split = index;
                break;
            }
            column = column
                .checked_mul(26)
                .and_then(|number| {
                    number.checked_add(u64::from(character.to_ascii_uppercase() - b'A' + 1))
                })
                .ok_or_else(|| BackendError::Limit("Office column index overflow".into()))?;
        }
        let row = coordinate
            .get(split..)
            .and_then(|row| row.strip_prefix('$').or(Some(row)))
            .and_then(|row| row.parse::<u64>().ok());
        if column == 0 || column > 256 || row.is_none_or(|row| row == 0 || row > max_items as u64) {
            return Err(BackendError::Limit(
                "Office worksheet range exceeds 256 columns or max_items rows".into(),
            ));
        }
    }
    Ok(())
}

fn validate_ir(
    ir: &office_oxide::ir::DocumentIR,
    context: &Context<'_>,
) -> Result<(), BackendError> {
    if ir.sections.len() > context.limits.max_pages as usize {
        return Err(BackendError::Limit(
            "Office IR sections exceed max_pages".into(),
        ));
    }
    let mut items = 0_usize;
    for section in &ir.sections {
        if let Some(setup) = &section.page_setup
            && (setup.width_twips > 288_000 || setup.height_twips > 288_000)
        {
            return Err(BackendError::Limit(
                "Office page geometry exceeds 14400 points".into(),
            ));
        }
        validate_elements(&section.elements, context, 0, &mut items)?;
        for header in [
            &section.header,
            &section.footer,
            &section.first_page_header,
            &section.first_page_footer,
            &section.even_page_header,
            &section.even_page_footer,
        ]
        .into_iter()
        .flatten()
        {
            validate_elements(&header.content, context, 0, &mut items)?;
        }
    }
    // Serialize only after bounded ZIP/XML parsing; this is a bounded secondary
    // check for IR duplication, not protection against upstream allocation.
    let encoded =
        serde_json::to_vec(ir).map_err(|error| invalid(&format!("Office IR: {error}")))?;
    if u64::try_from(encoded.len()).unwrap_or(u64::MAX) > context.limits.max_input_bytes {
        return Err(BackendError::Limit(
            "Office expanded IR exceeds max_input_bytes".into(),
        ));
    }
    Ok(())
}

fn validate_elements(
    elements: &[office_oxide::ir::Element],
    context: &Context<'_>,
    depth: usize,
    items: &mut usize,
) -> Result<(), BackendError> {
    use office_oxide::ir::Element;
    if depth > MAX_XML_DEPTH {
        return Err(BackendError::Limit("Office IR nesting exceeds 64".into()));
    }
    *items = items
        .checked_add(elements.len())
        .ok_or_else(|| BackendError::Limit("Office IR item count overflow".into()))?;
    if *items > context.limits.max_items {
        return Err(BackendError::Limit(
            "Office IR item count exceeds max_items".into(),
        ));
    }
    for element in elements {
        match element {
            Element::Paragraph(paragraph) => validate_inline(&paragraph.content, context, items)?,
            Element::Heading(heading) => validate_inline(&heading.content, context, items)?,
            Element::TextBox(text_box) => {
                if text_box.width_emu.is_some_and(|value| value > 182_880_000)
                    || text_box.height_emu.is_some_and(|value| value > 182_880_000)
                    || text_box
                        .x_emu
                        .is_some_and(|value| value.unsigned_abs() > 182_880_000)
                    || text_box
                        .y_emu
                        .is_some_and(|value| value.unsigned_abs() > 182_880_000)
                {
                    return Err(BackendError::Limit(
                        "Office text box geometry exceeds 14400 points".into(),
                    ));
                }
                validate_elements(&text_box.content, context, depth + 1, items)?;
            }
            Element::Footnote(note) | Element::Endnote(note) => {
                validate_elements(&note.content, context, depth + 1, items)?;
            }
            Element::Table(table) => {
                for row in &table.rows {
                    *items = items.checked_add(row.cells.len()).ok_or_else(|| {
                        BackendError::Limit("Office IR cell count overflow".into())
                    })?;
                    if *items > context.limits.max_items {
                        return Err(BackendError::Limit(
                            "Office IR cells exceed max_items".into(),
                        ));
                    }
                    let mut columns = 0_u64;
                    for cell in &row.cells {
                        columns += u64::from(cell.col_span);
                        if columns > 256 || cell.row_span as usize > context.limits.max_items {
                            return Err(BackendError::Limit(
                                "Office table span exceeds bounded grid".into(),
                            ));
                        }
                        validate_elements(&cell.content, context, depth + 1, items)?;
                    }
                }
            }
            Element::List(list) => validate_list(list, context, depth + 1, items)?,
            Element::Image(image) => {
                if image
                    .display_width_emu
                    .is_some_and(|value| value > 182_880_000)
                    || image
                        .display_height_emu
                        .is_some_and(|value| value > 182_880_000)
                {
                    return Err(BackendError::Limit(
                        "Office image geometry exceeds 14400 points".into(),
                    ));
                }
            }
            Element::Shape(shape)
                if shape.width_emu > 182_880_000
                    || shape.height_emu > 182_880_000
                    || shape.x_emu.unsigned_abs() > 182_880_000
                    || shape.y_emu.unsigned_abs() > 182_880_000 =>
            {
                return Err(BackendError::Limit(
                    "Office shape geometry exceeds 14400 points".into(),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_list(
    list: &office_oxide::ir::List,
    context: &Context<'_>,
    depth: usize,
    items: &mut usize,
) -> Result<(), BackendError> {
    if depth > MAX_XML_DEPTH {
        return Err(BackendError::Limit("Office list nesting exceeds 64".into()));
    }
    for item in &list.items {
        validate_elements(&item.content, context, depth + 1, items)?;
        if let Some(nested) = &item.nested {
            validate_list(nested, context, depth + 1, items)?;
        }
    }
    Ok(())
}

fn validate_inline(
    runs: &[office_oxide::ir::InlineContent],
    context: &Context<'_>,
    items: &mut usize,
) -> Result<(), BackendError> {
    *items = items
        .checked_add(runs.len())
        .ok_or_else(|| BackendError::Limit("Office IR inline count overflow".into()))?;
    if *items > context.limits.max_items {
        return Err(BackendError::Limit(
            "Office IR inline count exceeds max_items".into(),
        ));
    }
    for item in runs {
        if let office_oxide::ir::InlineContent::Text(text) = item {
            if text
                .font_size_half_pt
                .is_some_and(|size| size == 0 || size > 576)
            {
                return Err(BackendError::Limit(
                    "Office font size exceeds 288 points or is zero".into(),
                ));
            }
            if text
                .hyperlink
                .as_ref()
                .is_some_and(|target| !target.starts_with('#'))
            {
                return Err(BackendError::Unsupported(
                    "Office external hyperlinks are not admitted".into(),
                ));
            }
        }
    }
    Ok(())
}
