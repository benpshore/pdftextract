//! Bounded, byte-only PDF creation. No caller-controlled paths reach upstream.

use pdf_oxide::api::{Pdf, PdfBuilder};
use pdf_oxide::geometry::Rect;
use pdf_oxide::html_css::html::{NodeKind, parse_document};
use pdf_oxide::writer::{DocumentBuilder, DocumentMetadata, ImageData, PageSize};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Artifact, Context, OperationOutput};
use crate::backend::BackendError;

pub const OPERATIONS: &[&str] = &[
    "create.text",
    "create.markdown",
    "create.html",
    "create.html_css",
    "create.images",
    "create.document",
];

const MAX_CONTENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_FONT_BYTES: usize = 8 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_DIMENSION: f32 = 14_400.0;

pub(super) fn execute(
    operation: &str,
    context: &Context<'_>,
    params: &Value,
) -> Option<Result<OperationOutput, BackendError>> {
    if OPERATIONS.contains(&operation) && context.pages.is_some() {
        return Some(Err(BackendError::Unsupported(
            "creation operations do not accept source PDF page selection".into(),
        )));
    }
    Some(match operation {
        "create.text" | "create.markdown" | "create.html" => {
            from_content(operation, context, params)
        }
        "create.html_css" => from_html_css(context, params),
        "create.images" => from_images(context, params),
        "create.document" => compose(context, params),
        _ => return None,
    })
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct CreateOptions {
    width: f32,
    height: f32,
    margin: f32,
    font_size: f32,
    line_height: f32,
    title: Option<String>,
    author: Option<String>,
    subject: Option<String>,
}

impl Default for CreateOptions {
    fn default() -> Self {
        Self {
            width: 612.0,
            height: 792.0,
            margin: 72.0,
            font_size: 12.0,
            line_height: 1.2,
            title: None,
            author: None,
            subject: None,
        }
    }
}

impl CreateOptions {
    fn validate(&self) -> Result<(), BackendError> {
        dimension(self.width, "width")?;
        dimension(self.height, "height")?;
        if !self.margin.is_finite()
            || self.margin < 0.0
            || self.margin * 2.0 + self.font_size * self.line_height * 2.0
                >= self.width.min(self.height)
        {
            return Err(invalid("margins must leave space for at least two lines"));
        }
        if !self.font_size.is_finite() || !(1.0..=144.0).contains(&self.font_size) {
            return Err(invalid("font_size must be 1..=144 points"));
        }
        if !self.line_height.is_finite() || !(1.0..=4.0).contains(&self.line_height) {
            return Err(invalid("line_height must be 1..=4"));
        }
        for text in [&self.title, &self.author, &self.subject]
            .into_iter()
            .flatten()
        {
            if text.len() > 4096 || text.contains('\0') {
                return Err(invalid("metadata must be at most 4096 bytes without NUL"));
            }
        }
        Ok(())
    }

    fn builder(&self) -> PdfBuilder {
        let mut builder = PdfBuilder::new()
            .page_size(PageSize::Custom(self.width, self.height))
            .margin(self.margin)
            .font_size(self.font_size)
            .line_height(self.line_height);
        if let Some(value) = &self.title {
            builder = builder.title(value);
        }
        if let Some(value) = &self.author {
            builder = builder.author(value);
        }
        if let Some(value) = &self.subject {
            builder = builder.subject(value);
        }
        builder
    }
}

fn from_content(
    operation: &str,
    context: &Context<'_>,
    params: &Value,
) -> Result<OperationOutput, BackendError> {
    if context.inputs.len() != 1 {
        return Err(invalid(
            "text/Markdown/basic HTML creation requires exactly one input",
        ));
    }
    let options: CreateOptions = parse_params(params)?;
    options.validate()?;
    let source = text_input(context)?;
    check_content_items(source, context)?;
    if operation == "create.html" {
        inspect_html(source, false, context)?;
    }
    // The pinned plain-text builder applies font_size only to spacing, not to
    // the emitted font. Refuse that misleading option instead of accepting it.
    #[allow(
        clippy::float_cmp,
        reason = "Only the exact supported default is admitted; a tolerance would accept ignored overrides"
    )]
    let unsupported_font = options.font_size != 12.0;
    if operation == "create.text" && unsupported_font {
        return Err(BackendError::Unsupported(
            "pdf_oxide 0.3.78 plain-text creation emits 12pt text; use create.document for other sizes"
                .into(),
        ));
    }
    if operation == "create.text" && options.subject.is_some() {
        return Err(BackendError::Unsupported(
            "pdf_oxide 0.3.78 plain-text creation does not serialize subject metadata".into(),
        ));
    }
    let mut pdf = match operation {
        "create.text" => options.builder().from_text(source),
        "create.markdown" => options.builder().from_markdown(source),
        "create.html" => options.builder().from_html(source),
        _ => unreachable!(),
    }
    .map_err(upstream)?;
    let mut output = pdf_output(pdf.save_to_bytes().map_err(upstream)?, context)?;
    output.data["operation"] = json!(operation);
    let bundled =
        operation != "create.text" && pdf_oxide::fonts::bundled::needs_unicode_font(source);
    output.data["font_policy"] = json!(if bundled {
        "pinned_bundled_dejavu_regular_bold_with_standard14_courier_code"
    } else {
        "standard14"
    });
    output.warnings.push(
        "Creation uses the pinned text writer; glyph coverage, wrapping and visual fidelity are not independently verified. Source bytes remain unchanged."
            .into(),
    );
    if bundled {
        output.data["bundled_font_sha256"] = json!([
            crate::schema::sha256_hex(pdf_oxide::fonts::bundled::DEJAVU_SANS),
            crate::schema::sha256_hex(pdf_oxide::fonts::bundled::DEJAVU_SANS_BOLD),
        ]);
        output.warnings.push("Unicode Markdown/HTML prose uses pinned bundled DejaVu fonts; italic styling can degrade and non-ASCII code/table content can lose glyphs in Courier.".into());
    }
    if operation == "create.html" {
        output.warnings.push(
            "Basic HTML creation uses upstream HTML-to-Markdown conversion, not browser layout; CSS, images and unsupported HTML formatting are not represented."
                .into(),
        );
    }
    Ok(output)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FontInput {
    input: usize,
    family: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HtmlCssOptions {
    #[serde(default)]
    css: String,
    fonts: Vec<FontInput>,
}

fn from_html_css(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let options: HtmlCssOptions = parse_params(params)?;
    let html = text_input(context)?;
    if options.css.len() > MAX_CONTENT_BYTES || options.fonts.is_empty() || options.fonts.len() > 8
    {
        return Err(BackendError::Limit(
            "HTML/CSS needs 1..=8 font inputs and at most 4 MiB of CSS".into(),
        ));
    }
    inspect_css(&options.css)?;
    inspect_html(html, true, context)?;
    let mut fonts = Vec::with_capacity(options.fonts.len());
    let mut names = std::collections::BTreeSet::new();
    for font in options.fonts {
        if font.family.is_empty()
            || font.family.len() > 128
            || font.family.chars().any(char::is_control)
            || !names.insert(font.family.to_lowercase())
        {
            return Err(invalid(
                "font families must be distinct printable names, at most 128 bytes",
            ));
        }
        let bytes = context.input(font.input)?;
        if bytes.len() > MAX_FONT_BYTES {
            return Err(BackendError::Limit("font input exceeds 8 MiB".into()));
        }
        fonts.push((font.family, bytes.to_vec()));
    }
    let mut pdf = Pdf::from_html_css_with_fonts(html, &options.css, fonts).map_err(upstream)?;
    let mut output = pdf_output(pdf.save_to_bytes().map_err(upstream)?, context)?;
    let verified =
        pdf_oxide::PdfDocument::from_bytes(output.artifacts[0].bytes.clone()).map_err(upstream)?;
    let (expected_width, expected_height) = PageSize::A4.dimensions();
    for page in 0..verified.page_count().map_err(upstream)? {
        let (x0, y0, x1, y1) = verified.get_page_media_box(page).map_err(upstream)?;
        let (width, height) = (x1 - x0, y1 - y0);
        if !width.is_finite()
            || !height.is_finite()
            || (width - expected_width).abs() > 1.0
            || (height - expected_height).abs() > 1.0
        {
            // The pinned A4 CSS configuration is 794x1123 CSS pixels, but
            // its writer emits those as PDF points. Do not publish the known
            // unit mismatch or silently rescale font/layout resources.
            return Err(BackendError::Unsupported(format!(
                "pdf_oxide 0.3.78 HTML/CSS page-unit mismatch: page {} is {width}x{height} PDF points, expected A4; publication is disabled pending verified geometry correction",
                page + 1,
            )));
        }
    }
    output.data["operation"] = json!("create.html_css");
    output.warnings.push(
        "Pinned HTML/CSS layout is not a browser. It uses A4 pagination and supplied font bytes; complex CSS, glyph shaping and image fidelity remain unverified. No external resource is fetched."
            .into(),
    );
    Ok(output)
}

fn inspect_html(html: &str, css_mode: bool, context: &Context<'_>) -> Result<(), BackendError> {
    let dom = parse_document(html);
    if dom.nodes.len() > context.limits.max_items {
        return Err(BackendError::Limit(
            "HTML node count exceeds max_items".into(),
        ));
    }
    let mut image_pixels = 0_u64;
    let mut image_bytes = 0_u64;
    for node in &dom.nodes {
        let mut ancestor = node.parent;
        let mut depth = 0;
        while let Some(index) = ancestor {
            depth += 1;
            if depth > 64 {
                return Err(BackendError::Limit("HTML nesting exceeds 64".into()));
            }
            let index = usize::try_from(index).map_err(|_| invalid("HTML node index overflow"))?;
            ancestor = dom
                .nodes
                .get(index)
                .ok_or_else(|| invalid("invalid HTML node index"))?
                .parent;
        }
        match &node.kind {
            NodeKind::Element { tag, attrs } => {
                if css_mode
                    && matches!(
                        tag.as_str(),
                        "table"
                            | "thead"
                            | "tbody"
                            | "tfoot"
                            | "tr"
                            | "th"
                            | "td"
                            | "col"
                            | "colgroup"
                            | "caption"
                    )
                {
                    return Err(BackendError::Unsupported(
                        "pdf_oxide 0.3.78 HTML/CSS table layout can overlap cells; table creation is disabled pending geometry and render verification".into(),
                    ));
                }
                if matches!(
                    tag.as_str(),
                    "script"
                        | "iframe"
                        | "frame"
                        | "object"
                        | "embed"
                        | "link"
                        | "base"
                        | "meta"
                        | "audio"
                        | "video"
                        | "source"
                        | "form"
                        | "input"
                        | "button"
                        | "textarea"
                        | "select"
                ) {
                    return Err(BackendError::Unsupported(format!(
                        "HTML element {tag} is outside the passive creation contract"
                    )));
                }
                if !css_mode && (tag == "img" || tag == "style") {
                    return Err(BackendError::Unsupported(
                        "basic HTML creation cannot retain images or CSS; use create.html_css"
                            .into(),
                    ));
                }
                for (name, value) in attrs {
                    if name.starts_with("on")
                        || matches!(name.as_str(), "srcdoc" | "action" | "formaction")
                    {
                        return Err(BackendError::Unsupported(
                            "active HTML attributes are unsupported".into(),
                        ));
                    }
                    if name == "href" && !value.starts_with('#') {
                        return Err(BackendError::Unsupported(
                            "external HTML links are unsupported in creation".into(),
                        ));
                    }
                    if name == "style" {
                        if !css_mode {
                            return Err(BackendError::Unsupported(
                                "basic HTML creation cannot retain inline CSS".into(),
                            ));
                        }
                        inspect_css(value)?;
                    }
                    if name == "src" {
                        if tag != "img" || !css_mode || !value.starts_with("data:image/") {
                            return Err(BackendError::Unsupported(
                                "only in-memory image data URIs are supported".into(),
                            ));
                        }
                        let bytes = pdf_oxide::html_css::paint::decode_image_src(value)
                            .ok_or_else(|| invalid("invalid image data URI"))?;
                        let (_, _, pixels) = image_dimensions(&bytes, context)?;
                        image_pixels = image_pixels.checked_add(pixels).ok_or_else(|| {
                            BackendError::Limit("image pixel count overflow".into())
                        })?;
                        image_bytes += u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                        if image_pixels > context.limits.max_pixels
                            || image_bytes > context.limits.max_input_bytes
                        {
                            return Err(BackendError::Limit(
                                "HTML image total exceeds pixel or byte limit".into(),
                            ));
                        }
                        // Upstream HTML silently omits undecodable images. Make failure explicit.
                        ImageData::from_bytes(&bytes)
                            .map_err(|error| invalid(&format!("HTML image: {error}")))?;
                    }
                }
                if tag == "img" && !attrs.iter().any(|(name, _)| name == "src") {
                    return Err(invalid("HTML image lacks an in-memory src"));
                }
            }
            NodeKind::RawText { host_tag, body } if host_tag == "style" => inspect_css(body)?,
            _ => {}
        }
    }
    Ok(())
}

fn inspect_css(css: &str) -> Result<(), BackendError> {
    use pdf_oxide::html_css::css::tokenizer::{Token, tokenize};
    // Match decoded CSS tokens so escapes and comments cannot hide a resource
    // request or route a div through the defective table layout implementation.
    for (token, _) in tokenize(css).map_err(|error| invalid(&format!("CSS tokenize: {error}")))? {
        match token {
            Token::Ident(name)
                if matches!(
                    name.to_ascii_lowercase().as_str(),
                    "table"
                        | "inline-table"
                        | "table-row"
                        | "table-cell"
                        | "table-row-group"
                        | "table-header-group"
                        | "table-footer-group"
                        | "table-column"
                        | "table-column-group"
                        | "table-caption"
                ) =>
            {
                return Err(BackendError::Unsupported(
                    "CSS table layout is disabled pending geometry and render verification".into(),
                ));
            }
            Token::Url(_) | Token::BadUrl => {
                return Err(BackendError::Unsupported(
                    "CSS resource URLs are unsupported".into(),
                ));
            }
            Token::Function(name)
                if matches!(name.to_ascii_lowercase().as_str(), "url" | "expression") =>
            {
                return Err(BackendError::Unsupported(
                    "CSS resource/script functions are unsupported".into(),
                ));
            }
            Token::AtKeyword(name)
                if matches!(name.to_ascii_lowercase().as_str(), "import" | "font-face") =>
            {
                return Err(BackendError::Unsupported(
                    "CSS imports and font loading are unsupported".into(),
                ));
            }
            _ => {}
        }
    }
    let lower = css.to_ascii_lowercase();
    if [
        "url(",
        "@import",
        "@font-face",
        "expression(",
        "javascript:",
    ]
    .iter()
    .any(|token| lower.contains(token))
    {
        return Err(BackendError::Unsupported(
            "CSS resources, imports, embedded scripts and font loading are unsupported; supply font input bytes"
                .into(),
        ));
    }
    Ok(())
}

fn from_images(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let options: CreateOptions = parse_params(params)?;
    options.validate()?;
    #[allow(
        clippy::float_cmp,
        reason = "Exact defaults distinguish absent metrics from unsupported image text-layout overrides"
    )]
    let unsupported_metrics = options.font_size != 12.0 || options.line_height != 1.2;
    if unsupported_metrics {
        return Err(BackendError::Unsupported(
            "image creation does not use font_size or line_height".into(),
        ));
    }
    if context.inputs.is_empty()
        || context.inputs.len() > context.limits.max_items
        || context.inputs.len() > context.limits.max_pages as usize
    {
        return Err(BackendError::Limit(
            "image count must be 1..=max_pages and max_items".into(),
        ));
    }
    let mut total_pixels = 0_u64;
    let mut images = Vec::with_capacity(context.inputs.len());
    for index in 0..context.inputs.len() {
        let bytes = context.input(index)?;
        let (_, _, pixels) = image_dimensions(bytes, context)?;
        total_pixels = total_pixels
            .checked_add(pixels)
            .ok_or_else(|| BackendError::Limit("image pixel count overflow".into()))?;
        if total_pixels > context.limits.max_pixels {
            return Err(BackendError::Limit(
                "total image pixels exceed max_pixels".into(),
            ));
        }
        images.push(
            ImageData::from_bytes(bytes).map_err(|error| invalid(&format!("image: {error}")))?,
        );
    }
    let mut pdf = options
        .builder()
        .from_image_data_multiple(images)
        .map_err(upstream)?;
    let mut output = pdf_output(pdf.save_to_bytes().map_err(upstream)?, context)?;
    output.data["operation"] = json!("create.images");
    output.data["input_pixels"] = json!(total_pixels);
    output.warnings.push("PNG/JPEG embedding preserves source order, one image per page. JPEG compressed data is retained; color, transparency and visual fidelity require review.".into());
    Ok(output)
}

/// Header bounds are checked before upstream PNG allocation/JPEG embedding.
/// The supervised process remains the memory/time boundary for the decoder.
pub(super) fn image_dimensions(
    bytes: &[u8],
    context: &Context<'_>,
) -> Result<(u32, u32, u64), BackendError> {
    if bytes.len() > MAX_IMAGE_BYTES
        || u64::try_from(bytes.len()).unwrap_or(u64::MAX) > context.limits.max_input_bytes
    {
        return Err(BackendError::Limit(
            "image bytes exceed per-image limit".into(),
        ));
    }
    let (width, height) = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        if bytes.len() < 33 || bytes[8..16] != [0, 0, 0, 13, b'I', b'H', b'D', b'R'] {
            return Err(invalid("truncated or invalid PNG IHDR"));
        }
        let width = u32::from_be_bytes(bytes[16..20].try_into().expect("four bytes"));
        let height = u32::from_be_bytes(bytes[20..24].try_into().expect("four bytes"));
        (width, height)
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        jpeg_dimensions(bytes)?
    } else {
        return Err(BackendError::Unsupported(
            "image creation supports PNG and JPEG bytes only".into(),
        ));
    };
    let pixels = u64::from(width) * u64::from(height);
    if width == 0
        || height == 0
        || width > 65_535
        || height > 65_535
        || pixels > context.limits.max_pixels
    {
        return Err(BackendError::Limit(
            "image dimensions exceed pixel limit or are zero".into(),
        ));
    }
    let format = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        image::ImageFormat::Png
    } else {
        image::ImageFormat::Jpeg
    };
    let mut decoder = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(width);
    limits.max_image_height = Some(height);
    // Decoder allocation accounting is best effort; worker address-space and
    // deadline limits remain mandatory. Width/height checks are strict.
    limits.max_alloc = Some(
        context
            .limits
            .max_pixels
            .saturating_mul(16)
            .min(256 * 1024 * 1024),
    );
    decoder.limits(limits);
    let bitmap = decoder
        .decode()
        .map_err(|error| invalid(&format!("image decode: {error}")))?;
    if bitmap.width() != width || bitmap.height() != height {
        return Err(invalid("decoded image dimensions disagree with preflight"));
    }
    Ok((width, height, pixels))
}

fn jpeg_dimensions(bytes: &[u8]) -> Result<(u32, u32), BackendError> {
    if !bytes.ends_with(&[0xff, 0xd9]) {
        return Err(invalid("JPEG is missing its end marker"));
    }
    let mut offset = 2_usize;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xff {
            return Err(invalid("invalid JPEG marker"));
        }
        while offset < bytes.len() && bytes[offset] == 0xff {
            offset += 1;
        }
        let marker = *bytes
            .get(offset)
            .ok_or_else(|| invalid("truncated JPEG marker"))?;
        offset += 1;
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let length_bytes = bytes
            .get(offset..offset + 2)
            .ok_or_else(|| invalid("truncated JPEG segment"))?;
        let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
        if length < 2
            || offset
                .checked_add(length)
                .is_none_or(|end| end > bytes.len())
        {
            return Err(invalid("invalid JPEG segment length"));
        }
        if matches!(marker, 0xc0..=0xc2) {
            if length < 8
                || bytes[offset + 2] != 8
                || !matches!(bytes[offset + 7], 1 | 3 | 4)
                || length < 8 + usize::from(bytes[offset + 7]) * 3
            {
                return Err(invalid("unsupported or truncated JPEG frame"));
            }
            return Ok((
                u32::from(u16::from_be_bytes([bytes[offset + 5], bytes[offset + 6]])),
                u32::from(u16::from_be_bytes([bytes[offset + 3], bytes[offset + 4]])),
            ));
        }
        offset += length;
    }
    Err(BackendError::Unsupported(
        "JPEG frame is absent or not an 8-bit baseline/progressive frame".into(),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Composition {
    pages: Vec<CompositionPage>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    author: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompositionPage {
    width: f32,
    height: f32,
    elements: Vec<CompositionElement>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum CompositionElement {
    Text {
        x: f32,
        y: f32,
        text: String,
        size: f32,
        #[serde(default = "default_font")]
        font: String,
    },
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        #[serde(default)]
        fill: Option<[f32; 3]>,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    },
    Image {
        input: usize,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    Table {
        x: f32,
        y: f32,
        width: f32,
        size: f32,
        rows: Vec<Vec<CompositionCell>>,
        #[serde(default)]
        header: bool,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompositionCell {
    text: String,
    #[serde(default = "one")]
    colspan: usize,
    #[serde(default = "one")]
    rowspan: usize,
}

const fn one() -> usize {
    1
}

fn default_font() -> String {
    "Helvetica".into()
}

fn compose(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let composition: Composition = parse_params(params)?;
    if composition.pages.iter().any(|page| {
        page.elements
            .iter()
            .any(|element| matches!(element, CompositionElement::Table { .. }))
    }) {
        // Preserve the reviewed implementation below for the checkpoint's
        // source record, but do not publish known defective table derivatives.
        return Err(BackendError::Unsupported(
            "pdf_oxide 0.3.78 buffered table creation can put text across cell borders; table creation is disabled pending geometry and render verification".into(),
        ));
    }
    if composition.pages.is_empty() || composition.pages.len() > context.limits.max_pages as usize {
        return Err(BackendError::Limit(
            "composition page count must be 1..=max_pages".into(),
        ));
    }
    let mut item_count = 0_usize;
    let mut text_bytes = 0_usize;
    let mut total_pixels = 0_u64;
    let mut builder = DocumentBuilder::new().compress_streams(true);
    let metadata = CreateOptions {
        title: composition.title,
        author: composition.author,
        ..CreateOptions::default()
    };
    metadata.validate()?;
    let mut document_metadata = DocumentMetadata::new();
    if let Some(title) = &metadata.title {
        document_metadata = document_metadata.title(title);
    }
    if let Some(author) = &metadata.author {
        document_metadata = document_metadata.author(author);
    }
    builder = builder.metadata(document_metadata);
    for spec in composition.pages {
        dimension(spec.width, "page width")?;
        dimension(spec.height, "page height")?;
        item_count = item_count
            .checked_add(spec.elements.len())
            .ok_or_else(|| BackendError::Limit("item count overflow".into()))?;
        if item_count > context.limits.max_items {
            return Err(BackendError::Limit("composition exceeds max_items".into()));
        }
        let mut page = builder.page(PageSize::Custom(spec.width, spec.height));
        for element in spec.elements {
            match element {
                CompositionElement::Text {
                    x,
                    y,
                    text,
                    size,
                    font,
                } => {
                    point(x, y, spec.width, spec.height)?;
                    if !size.is_finite() || !(1.0..=144.0).contains(&size) {
                        return Err(invalid("text size must be 1..=144"));
                    }
                    if ![
                        "Helvetica",
                        "Helvetica-Bold",
                        "Helvetica-Oblique",
                        "Helvetica-BoldOblique",
                        "Times-Roman",
                        "Times-Bold",
                        "Times-Italic",
                        "Times-BoldItalic",
                        "Courier",
                        "Courier-Bold",
                        "Courier-Oblique",
                        "Courier-BoldOblique",
                        "Symbol",
                        "ZapfDingbats",
                    ]
                    .contains(&font.as_str())
                    {
                        return Err(BackendError::Unsupported(
                            "composition font must be a Standard-14 font".into(),
                        ));
                    }
                    text_bytes = text_bytes
                        .checked_add(text.len())
                        .ok_or_else(|| BackendError::Limit("text size overflow".into()))?;
                    if text_bytes > MAX_CONTENT_BYTES {
                        return Err(BackendError::Limit("composition text exceeds 4 MiB".into()));
                    }
                    page = page.font(&font, size).at(x, y).text(&text);
                }
                CompositionElement::Rect {
                    x,
                    y,
                    width,
                    height,
                    fill,
                } => {
                    region(x, y, width, height, spec.width, spec.height)?;
                    page = if let Some([r, g, b]) = fill {
                        if [r, g, b]
                            .iter()
                            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
                        {
                            return Err(invalid("fill channels must be 0..=1"));
                        }
                        page.filled_rect(x, y, width, height, r, g, b)
                    } else {
                        page.rect(x, y, width, height)
                    };
                }
                CompositionElement::Line { x1, y1, x2, y2 } => {
                    point(x1, y1, spec.width, spec.height)?;
                    point(x2, y2, spec.width, spec.height)?;
                    page = page.line(x1, y1, x2, y2);
                }
                CompositionElement::Image {
                    input,
                    x,
                    y,
                    width,
                    height,
                } => {
                    region(x, y, width, height, spec.width, spec.height)?;
                    let bytes = context.input(input)?;
                    let (_, _, pixels) = image_dimensions(bytes, context)?;
                    total_pixels = total_pixels
                        .checked_add(pixels)
                        .ok_or_else(|| BackendError::Limit("image pixel count overflow".into()))?;
                    if total_pixels > context.limits.max_pixels {
                        return Err(BackendError::Limit(
                            "composition image pixels exceed max_pixels".into(),
                        ));
                    }
                    page = page
                        .image_from_bytes(bytes, Rect::new(x, y, width, height))
                        .map_err(upstream)?;
                }
                CompositionElement::Table {
                    x,
                    y,
                    width,
                    size,
                    rows,
                    header,
                } => {
                    point(x, y, spec.width, spec.height)?;
                    dimension(width, "table width")?;
                    if x + width > spec.width || !size.is_finite() || !(1.0..=72.0).contains(&size)
                    {
                        return Err(invalid(
                            "table must fit page width and use a 1..=72 point font",
                        ));
                    }
                    let (table, height) = buffered_table(
                        rows,
                        width,
                        size,
                        header,
                        context,
                        &mut item_count,
                        &mut text_bytes,
                    )?;
                    if !height.is_finite() || height > y {
                        return Err(BackendError::Limit("buffered table does not fit the page; split rows across explicit pages".into()));
                    }
                    page = page.font("Helvetica", size).at(x, y).table(table);
                }
            }
        }
        page.done();
    }
    let mut output = pdf_output(builder.build().map_err(upstream)?, context)?;
    output.data["operation"] = json!("create.document");
    output.data["items"] = json!(item_count);
    output.warnings.push("Explicit composition coordinates use PDF points from the bottom-left. Glyph coverage and text fit are not independently verified; text does not automatically wrap or clip to the page.".into());
    Ok(output)
}

fn buffered_table(
    rows: Vec<Vec<CompositionCell>>,
    width: f32,
    size: f32,
    header: bool,
    context: &Context<'_>,
    items: &mut usize,
    text_bytes: &mut usize,
) -> Result<(pdf_oxide::writer::Table, f32), BackendError> {
    use pdf_oxide::writer::{ColumnWidth, FontManager, FontMetrics, Table, TableCell, TableStyle};
    struct Metrics(FontManager);
    impl FontMetrics for Metrics {
        fn text_width(&self, text: &str, size: f32) -> f32 {
            self.0
                .text_width(text, "Helvetica", size)
                .max(self.0.text_width(text, "Helvetica-Bold", size))
        }
    }
    if rows.is_empty() || rows.len() > context.limits.max_items {
        return Err(BackendError::Limit(
            "table rows must be nonempty and bounded by max_items".into(),
        ));
    }
    let mut column_count = None;
    let mut table_rows = Vec::with_capacity(rows.len());
    for row in rows {
        *items = items
            .checked_add(row.len())
            .ok_or_else(|| BackendError::Limit("table cell count overflow".into()))?;
        if row.is_empty() || *items > context.limits.max_items {
            return Err(BackendError::Limit(
                "table cells must be nonempty and bounded by max_items".into(),
            ));
        }
        let mut columns = 0_usize;
        let mut cells = Vec::with_capacity(row.len());
        for cell in row {
            if cell.rowspan != 1 {
                return Err(BackendError::Unsupported("pinned buffered-table rowspan placement can overlap subsequent cells; only rowspan=1 is admitted".into()));
            }
            if cell.colspan == 0 || cell.colspan > 64 {
                return Err(invalid("table colspan must be 1..=64"));
            }
            columns += cell.colspan;
            if columns > 64 {
                return Err(BackendError::Limit("table exceeds 64 columns".into()));
            }
            *text_bytes = text_bytes
                .checked_add(cell.text.len())
                .ok_or_else(|| BackendError::Limit("table text size overflow".into()))?;
            if *text_bytes > MAX_CONTENT_BYTES {
                return Err(BackendError::Limit("composition text exceeds 4 MiB".into()));
            }
            cells.push(TableCell::text(cell.text).colspan(cell.colspan));
        }
        if column_count.is_some_and(|count| count != columns) {
            return Err(invalid("table rows must cover the same number of columns"));
        }
        column_count = Some(columns);
        table_rows.push(cells);
    }
    let count = column_count.expect("nonempty rows");
    let count_f32 = f32::from(u16::try_from(count).expect("at most 64 columns"));
    if width / count_f32 < 12.0 {
        return Err(invalid("table columns must be at least 12 points wide"));
    }
    let mut table = Table::new(table_rows)
        .with_width(width)
        .with_style(TableStyle::default().font("Helvetica", size))
        .with_column_widths(vec![ColumnWidth::Fixed(width / count_f32); count]);
    if header {
        table = table.with_header_row();
    }
    let height = table
        .calculate_layout(width, &Metrics(FontManager::new()))
        .total_height;
    Ok((table, height))
}

pub(super) fn pdf_output(
    bytes: Vec<u8>,
    context: &Context<'_>,
) -> Result<OperationOutput, BackendError> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > context.limits.max_output_bytes {
        return Err(BackendError::Limit(
            "generated PDF exceeds max_output_bytes".into(),
        ));
    }
    let document = pdf_oxide::PdfDocument::from_bytes(bytes.clone()).map_err(upstream)?;
    let pages = document.page_count().map_err(upstream)?;
    if pages == 0 || pages > context.limits.max_pages as usize {
        return Err(BackendError::Limit(
            "generated PDF page count must be 1..=max_pages".into(),
        ));
    }
    Ok(OperationOutput {
        data: json!({"pages": pages, "source_sha256": context.inputs.iter().map(|input| input.sha256.as_str()).collect::<Vec<_>>(), "fidelity": "unverified"}),
        artifacts: vec![Artifact {
            name: "created.pdf".into(),
            mime: "application/pdf".into(),
            bytes,
        }],
        warnings: Vec::new(),
    })
}

fn text_input<'a>(context: &'a Context<'_>) -> Result<&'a str, BackendError> {
    let bytes = context.input(0)?;
    if bytes.len() > MAX_CONTENT_BYTES {
        return Err(BackendError::Limit("creation text exceeds 4 MiB".into()));
    }
    std::str::from_utf8(bytes).map_err(|_| invalid("creation content must be UTF-8"))
}

fn check_content_items(source: &str, context: &Context<'_>) -> Result<(), BackendError> {
    if source.lines().count() > context.limits.max_items || source.contains('\0') {
        return Err(BackendError::Limit(
            "content line count exceeds max_items or contains NUL".into(),
        ));
    }
    Ok(())
}

fn dimension(value: f32, label: &str) -> Result<(), BackendError> {
    if !value.is_finite() || !(1.0..=MAX_DIMENSION).contains(&value) {
        return Err(invalid(&format!("{label} must be 1..=14400 points")));
    }
    Ok(())
}

fn point(x: f32, y: f32, width: f32, height: f32) -> Result<(), BackendError> {
    if !x.is_finite()
        || !y.is_finite()
        || !(0.0..=width).contains(&x)
        || !(0.0..=height).contains(&y)
    {
        return Err(invalid("coordinates must be finite and within the page"));
    }
    Ok(())
}

fn region(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    page_width: f32,
    page_height: f32,
) -> Result<(), BackendError> {
    dimension(width, "region width")?;
    dimension(height, "region height")?;
    point(x, y, page_width, page_height)?;
    point(x + width, y + height, page_width, page_height)
}

pub(super) fn parse_params<T: serde::de::DeserializeOwned>(
    params: &Value,
) -> Result<T, BackendError> {
    serde_json::from_value(params.clone()).map_err(|error| invalid(&format!("parameters: {error}")))
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Owned error signature is required for direct Result::map_err adapters"
)]
pub(super) fn upstream(error: pdf_oxide::Error) -> BackendError {
    BackendError::Malformed(format!("pdf_oxide: {error}"))
}

pub(super) fn invalid(message: &str) -> BackendError {
    BackendError::Malformed(message.into())
}
