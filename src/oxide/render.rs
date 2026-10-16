//! Bounded, memory-only raster operations. The caller must run these in the
//! supervised worker: upstream font discovery is not an in-process sandbox.

use serde_json::Value;

use super::{Context, OperationOutput};
use crate::backend::BackendError;

pub const OPERATIONS: &[&str] = &[
    "render.pages",
    "render.fit",
    "render.region",
    "render.separations",
    "render.flatten",
    "barcode.generate",
];

pub(super) fn execute(
    op: &str,
    ctx: &Context<'_>,
    params: &Value,
) -> Option<Result<OperationOutput, BackendError>> {
    if op == "barcode.generate" {
        return Some(barcode(ctx, params));
    }
    if !OPERATIONS.contains(&op) {
        return None;
    }
    Some(render(op, ctx, params))
}

#[cfg(not(feature = "pdf-oxide-rendering"))]
fn render(_op: &str, _ctx: &Context<'_>, _params: &Value) -> Result<OperationOutput, BackendError> {
    Err(BackendError::Unsupported(
        "pdf-oxide rendering requires the pdf-oxide-rendering build feature and a confined worker"
            .into(),
    ))
}

#[cfg(feature = "pdf-oxide-rendering")]
use raster::render;

#[cfg(feature = "pdf-oxide-rendering")]
mod raster {
    use pdf_oxide::PdfDocument;
    use pdf_oxide::rendering::{self, ImageFormat, RenderOptions, RenderedImage};
    use serde::Deserialize;
    use serde_json::{Value, json};

    use super::{BackendError, Context, OperationOutput};
    use crate::oxide::Artifact;

    const MAX_DIMENSION: u32 = 8192;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Params {
        #[serde(default = "default_dpi")]
        dpi: u32,
        #[serde(default = "default_format")]
        format: String,
        #[serde(default = "default_quality")]
        jpeg_quality: u8,
        #[serde(default)]
        transparent: bool,
        #[serde(default)]
        background: Option<[f32; 4]>,
        #[serde(default)]
        excluded_layers: Vec<String>,
        #[serde(default = "yes")]
        annotations: bool,
        #[serde(default)]
        width: Option<u32>,
        #[serde(default)]
        height: Option<u32>,
        #[serde(default)]
        crop: Option<[f32; 4]>,
        #[serde(default)]
        ink: Option<String>,
    }

    fn default_dpi() -> u32 {
        150
    }
    fn default_format() -> String {
        "png".into()
    }
    fn default_quality() -> u8 {
        85
    }
    fn yes() -> bool {
        true
    }

    fn invalid(message: impl Into<String>) -> BackendError {
        BackendError::Malformed(format!("pdf-oxide render parameters: {}", message.into()))
    }

    // Passed directly to Result::map_err, whose callback consumes the error.
    #[allow(clippy::needless_pass_by_value)]
    fn upstream(error: pdf_oxide::Error) -> BackendError {
        BackendError::Malformed(format!("pdf-oxide rendering: {error}"))
    }

    fn pixels(width: u32, height: u32) -> Result<u64, BackendError> {
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(BackendError::Limit(format!(
                "render dimensions must be between 1 and {MAX_DIMENSION} pixels"
            )));
        }
        Ok(u64::from(width) * u64::from(height))
    }

    fn reserve(total: &mut u64, amount: u64, maximum: u64, what: &str) -> Result<(), BackendError> {
        *total = total
            .checked_add(amount)
            .ok_or_else(|| BackendError::Limit(format!("{what} counter overflow")))?;
        if *total > maximum {
            return Err(BackendError::Limit(format!("{what} exceeds {maximum}")));
        }
        Ok(())
    }

    struct Page {
        index: usize,
        width: u32,
        height: u32,
        points: (f32, f32),
        extent: (u32, u32),
        plates: usize,
    }

    fn visible_box(
        info: &pdf_oxide::document::PageInfo,
    ) -> Result<pdf_oxide::geometry::Rect, BackendError> {
        let media = info.media_box;
        for rect in std::iter::once(&media).chain(info.crop_box.iter()) {
            if ![rect.x, rect.y, rect.width, rect.height]
                .iter()
                .all(|v| v.is_finite())
                || rect.width <= 0.0
                || rect.height <= 0.0
                || !(rect.x + rect.width).is_finite()
                || !(rect.y + rect.height).is_finite()
            {
                return Err(invalid("non-finite or non-positive PDF page box"));
            }
        }
        let Some(crop) = info.crop_box else {
            return Ok(media);
        };
        let x = media.x.max(crop.x);
        let y = media.y.max(crop.y);
        let right = (media.x + media.width).min(crop.x + crop.width);
        let top = (media.y + media.height).min(crop.y + crop.height);
        if right <= x || top <= y {
            return Ok(media);
        }
        Ok(pdf_oxide::geometry::Rect::new(x, y, right - x, top - y))
    }

    #[allow(clippy::cast_precision_loss)]
    fn dimension(points: f32, scale: f32, rounded: bool) -> Result<u32, BackendError> {
        let result = if rounded {
            (points * scale).round().max(1.0)
        } else {
            (points * scale).ceil()
        };
        if !result.is_finite() || result < 1.0 || result > MAX_DIMENSION as f32 {
            return Err(BackendError::Limit(format!(
                "requested raster dimension is outside 1..={MAX_DIMENSION}"
            )));
        }
        // Checked against a small, exactly representable integer bound above.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(result as u32)
    }

    #[allow(clippy::too_many_lines)]
    fn plan(
        doc: &PdfDocument,
        ctx: &Context<'_>,
        op: &str,
        p: &Params,
    ) -> Result<Vec<Page>, BackendError> {
        let selected = ctx.selected_pages(doc.page_count().map_err(upstream)?)?;
        let mut total_pixels = 0;
        let mut total_items = 0_usize;
        let mut pages = Vec::with_capacity(selected.len());
        for index in selected {
            let info = doc.get_page_info(index).map_err(upstream)?;
            if info.rotation.rem_euclid(90) != 0 {
                return Err(BackendError::Unsupported(
                    "rendering requires page rotation divisible by 90".into(),
                ));
            }
            let rect = visible_box(&info)?;
            let turned = info.rotation.rem_euclid(180) != 0;
            let points = if turned {
                (rect.height, rect.width)
            } else {
                (rect.width, rect.height)
            };
            #[allow(clippy::cast_precision_loss)]
            let mut scale = p.dpi as f32 / 72.0;
            if op == "render.fit" {
                let (width, height) = (
                    p.width.ok_or_else(|| invalid("fit width is required"))?,
                    p.height.ok_or_else(|| invalid("fit height is required"))?,
                );
                pixels(width, height)?;
                // Pinned render_page_fit derives scale from MediaBox, then clips
                // to CropBox. Mirror its actual semantics, not the web examples.
                let media = if turned {
                    (info.media_box.height, info.media_box.width)
                } else {
                    (info.media_box.width, info.media_box.height)
                };
                #[allow(clippy::cast_precision_loss)]
                {
                    scale = (width as f32 / media.0.max(1.0)).min(height as f32 / media.1.max(1.0));
                }
            }
            let width = dimension(points.0, scale, op == "render.fit")?;
            let height = dimension(points.1, scale, op == "render.fit")?;
            let mut extent = (width, height);
            if op == "render.region" {
                // The pinned helper ignores rotated/cropped page origins in its
                // crop transform. Refuse those cases instead of mislabelling pixels.
                if info.rotation.rem_euclid(360) != 0
                    || info.media_box.x != 0.0
                    || info.media_box.y != 0.0
                    || rect != info.media_box
                {
                    return Err(BackendError::Unsupported("region rendering in pdf-oxide 0.3.78 is supported only for unrotated zero-origin pages without an effective CropBox".into()));
                }
                let [x, y, crop_width, crop_height] = p
                    .crop
                    .ok_or_else(|| invalid("crop [x,y,width,height] is required"))?;
                if ![x, y, crop_width, crop_height]
                    .iter()
                    .all(|v| v.is_finite())
                    || x < 0.0
                    || y < 0.0
                    || crop_width <= 0.0
                    || crop_height <= 0.0
                    || x + crop_width > points.0
                    || y + crop_height > points.1
                {
                    return Err(invalid(
                        "crop must be a finite positive rectangle wholly inside the page",
                    ));
                }
                extent = (
                    dimension(crop_width, scale, true)?,
                    dimension(crop_height, scale, true)?,
                );
                reserve(
                    &mut total_pixels,
                    pixels(extent.0, extent.1)?,
                    ctx.limits.max_pixels,
                    "render decoded pixel total",
                )?;
            }
            let plates = if op == "render.separations" && p.ink.is_none() {
                let mut inks = vec![
                    "Cyan".to_owned(),
                    "Magenta".to_owned(),
                    "Yellow".to_owned(),
                    "Black".to_owned(),
                ];
                for ink in doc.get_page_inks_deep(index).map_err(upstream)? {
                    if !inks.contains(&ink) {
                        inks.push(ink);
                    }
                    if inks.len() > ctx.limits.max_items {
                        return Err(BackendError::Limit(
                            "separation count limit exceeded".into(),
                        ));
                    }
                }
                inks.len()
            } else {
                1
            };
            total_items = total_items
                .checked_add(plates)
                .ok_or_else(|| BackendError::Limit("render item count overflow".into()))?;
            if total_items > ctx.limits.max_items {
                return Err(BackendError::Limit(
                    "render item count limit exceeded".into(),
                ));
            }
            let plane_pixels = pixels(width, height)?
                .checked_mul(plates as u64)
                .ok_or_else(|| BackendError::Limit("separation pixel count overflow".into()))?;
            reserve(
                &mut total_pixels,
                plane_pixels,
                ctx.limits.max_pixels,
                "render decoded pixel total",
            )?;
            pages.push(Page {
                index,
                width,
                height,
                points,
                extent,
                plates,
            });
        }
        Ok(pages)
    }

    fn record_image(
        image: RenderedImage,
        page: &Page,
        ctx: &Context<'_>,
        total: &mut u64,
    ) -> Result<(Artifact, Value), BackendError> {
        if (image.width, image.height) != page.extent {
            return Err(BackendError::Malformed(
                "renderer changed the validated dimensions; output was not published".into(),
            ));
        }
        reserve(
            total,
            image.data.len() as u64,
            ctx.limits.max_output_bytes,
            "render encoded bytes",
        )?;
        let (extension, mime, encoding) = match image.format {
            ImageFormat::Png => ("png", "image/png", "png"),
            ImageFormat::Jpeg => ("jpg", "image/jpeg", "jpeg"),
            ImageFormat::RawRgba8 => ("rgba", "application/octet-stream", "premultiplied_rgba8"),
        };
        let name = format!("page-{:06}.{extension}", page.index + 1);
        let data = json!({"page":page.index+1,"artifact":name,"width":image.width,"height":image.height,"encoding":encoding,"coordinate_origin":"top_left","status":"partial"});
        Ok((
            Artifact {
                name,
                mime: mime.into(),
                bytes: image.data,
            },
            data,
        ))
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn render(
        op: &str,
        ctx: &Context<'_>,
        params: &Value,
    ) -> Result<OperationOutput, BackendError> {
        let p: Params =
            serde_json::from_value(params.clone()).map_err(|error| invalid(error.to_string()))?;
        if !(1..=600).contains(&p.dpi) || !(1..=100).contains(&p.jpeg_quality) {
            return Err(invalid("DPI must be 1..=600; JPEG quality must be 1..=100"));
        }
        if op == "render.separations" {
            if params.get("format").is_some_and(|format| format != "pgm") {
                return Err(invalid("separation output supports only pgm"));
            }
            if params.get("transparent").is_some() || params.get("annotations").is_some() {
                return Err(invalid(
                    "separation rendering has no transparency or annotation option",
                ));
            }
            if p.background.is_some()
                || !p.excluded_layers.is_empty()
                || params.get("jpeg_quality").is_some()
            {
                return Err(invalid(
                    "separation rendering has no background, excluded_layers, or JPEG quality option",
                ));
            }
        }
        let format = match p.format.as_str() {
            "png" => ImageFormat::Png,
            "jpeg" => ImageFormat::Jpeg,
            "pgm" if op == "render.separations" => ImageFormat::Png,
            "rgba" if op != "render.region" && op != "render.flatten" => ImageFormat::RawRgba8,
            _ => {
                return Err(invalid(
                    "format must be png or jpeg (rgba is also supported for pages and fit)",
                ));
            }
        };
        if op != "render.fit" && (p.width.is_some() || p.height.is_some()) {
            return Err(invalid("width and height apply only to render.fit"));
        }
        if (op == "render.region") != p.crop.is_some() {
            return Err(invalid("crop is required only for render.region"));
        }
        if p.ink.is_some() && op != "render.separations" {
            return Err(invalid("ink applies only to render.separations"));
        }
        if p.ink
            .as_ref()
            .is_some_and(|ink| ink.is_empty() || ink.len() > 256)
        {
            return Err(invalid("ink name must contain 1..=256 UTF-8 bytes"));
        }
        if p.transparent && p.background.is_some() {
            return Err(invalid("transparent and background cannot be combined"));
        }
        if p.background.is_some_and(|color| {
            color
                .iter()
                .any(|component| !component.is_finite() || !(0.0..=1.0).contains(component))
        }) {
            return Err(invalid(
                "background components must be finite values in 0..=1",
            ));
        }
        if p.excluded_layers.len() > ctx.limits.max_items
            || p.excluded_layers
                .iter()
                .any(|name| name.is_empty() || name.len() > 256)
        {
            return Err(BackendError::Limit(
                "excluded layer count or name length limit exceeded".into(),
            ));
        }
        ctx.require_filesystem_containment()?;
        let doc = ctx.open_pdf()?;
        let pages = plan(&doc, ctx, op, &p)?;
        let mut options = RenderOptions::with_dpi(p.dpi);
        options.format = format;
        options.jpeg_quality = p.jpeg_quality;
        options.render_annotations = p.annotations;
        options.max_output_pixels = ctx.limits.max_pixels;
        options.excluded_layers = p.excluded_layers.iter().cloned().collect();
        if let Some(background) = p.background {
            options.background = Some(background);
        }
        if p.transparent {
            options.background = None;
        }
        let mut out = OperationOutput::default();
        let mut records = Vec::with_capacity(pages.len());
        let mut total_bytes = 0;
        let mut writer = (op == "render.flatten").then(pdf_oxide::writer::PdfWriter::new);
        for page in &pages {
            if op == "render.separations" {
                let plates = if let Some(ink) = &p.ink {
                    vec![
                        rendering::render_separation(&doc, page.index, ink, p.dpi)
                            .map_err(upstream)?,
                    ]
                } else {
                    rendering::render_separations(&doc, page.index, p.dpi).map_err(upstream)?
                };
                if plates.len() != page.plates {
                    return Err(BackendError::Malformed(
                        "separation count changed after validation".into(),
                    ));
                }
                for (number, plate) in plates.into_iter().enumerate() {
                    if (plate.width, plate.height) != (page.width, page.height)
                        || plate.data.len() as u64 != pixels(plate.width, plate.height)?
                    {
                        return Err(BackendError::Malformed(
                            "invalid separation raster extent".into(),
                        ));
                    }
                    let mut bytes =
                        format!("P5\n{} {}\n255\n", plate.width, plate.height).into_bytes();
                    bytes.extend_from_slice(&plate.data);
                    reserve(
                        &mut total_bytes,
                        bytes.len() as u64,
                        ctx.limits.max_output_bytes,
                        "render encoded bytes",
                    )?;
                    let name = format!("page-{:06}-ink-{:03}.pgm", page.index + 1, number + 1);
                    records.push(json!({"page":page.index+1,"ink":plate.ink_name,"artifact":name,"width":plate.width,"height":plate.height,"encoding":"pgm_gray8_ink_coverage","status":"partial"}));
                    out.artifacts.push(Artifact {
                        name,
                        mime: "image/x-portable-graymap".into(),
                        bytes,
                    });
                }
                continue;
            }
            let image = match op {
                "render.fit" => rendering::render_page_fit(
                    &doc,
                    page.index,
                    p.width.unwrap_or(1),
                    p.height.unwrap_or(1),
                    &options,
                ),
                "render.region" => {
                    let [x, y, crop_width, crop_height] =
                        p.crop.ok_or_else(|| invalid("missing crop"))?;
                    rendering::render_page_region(
                        &doc,
                        page.index,
                        (x, y, crop_width, crop_height),
                        &options,
                    )
                }
                _ => rendering::render_page(&doc, page.index, &options),
            }
            .map_err(upstream)?;
            let (artifact, record) = record_image(image, page, ctx, &mut total_bytes)?;
            records.push(record);
            if let Some(writer) = &mut writer {
                add_raster_page(writer, page, &artifact.bytes)?;
            } else {
                out.artifacts.push(artifact);
            }
        }
        if let Some(writer) = writer {
            let bytes = writer.finish().map_err(upstream)?;
            if bytes.len() as u64 > ctx.limits.max_output_bytes {
                return Err(BackendError::Limit(
                    "flattened PDF output byte limit exceeded".into(),
                ));
            }
            for (index, record) in records.iter_mut().enumerate() {
                record["artifact"] = json!("flattened.pdf");
                record["derivative_page"] = json!(index + 1);
            }
            out.artifacts.push(Artifact {
                name: "flattened.pdf".into(),
                mime: "application/pdf".into(),
                bytes,
            });
            out.warnings.push("Raster derivative: text, links, tags, editability, and original objects are not retained. This operation is not verified redaction or archival conformance.".into());
        }
        out.data = json!({"source_sha256":ctx.inputs[0].sha256,"operation":op,"pages":records,"dpi":if op=="render.fit" {None} else {Some(p.dpi)},"fit":[p.width,p.height],"crop":p.crop,"render_annotations":p.annotations,"transparent":p.transparent,"background":p.background,"excluded_layers":p.excluded_layers,"source_preserved":true,"status":"partial","diagnostics_page_indexing":"zero_based_upstream","diagnostics":doc.structured_warnings()});
        out.warnings.push("PDF Oxide raster output may omit unsupported content or substitute/drop fonts; successful rendering does not establish visual fidelity. Inspect source and derivative together.".into());
        out.warnings.push("When worker confinement denies host font reads, fonts without usable embedded programs can be rendered as rectangle placeholders by PDF Oxide 0.3.78. Its missing-font path is not fully represented by structured warnings.".into());
        if op == "render.separations" {
            out.warnings.push("Separation output reports ink coverage (0 = no ink, 255 = full tint). Upstream omits or approximates some RGB/Gray/Indexed images, patterns, inline images, annotations and color-management paths; press fidelity is unverified.".into());
        }
        Ok(out)
    }

    fn add_raster_page(
        writer: &mut pdf_oxide::writer::PdfWriter,
        page: &Page,
        bytes: &[u8],
    ) -> Result<(), BackendError> {
        use pdf_oxide::elements::{ColorSpace, ContentElement, ImageContent, ImageFormat};
        let image = pdf_oxide::writer::ImageData::from_bytes(bytes)
            .map_err(|e| BackendError::Malformed(format!("raster PDF image encoding: {e}")))?;
        let image = ImageContent {
            bbox: pdf_oxide::geometry::Rect::new(0.0, 0.0, page.points.0, page.points.1),
            format: match image.format {
                pdf_oxide::writer::ImageFormat::Jpeg => ImageFormat::Jpeg,
                pdf_oxide::writer::ImageFormat::Png => ImageFormat::Png,
                pdf_oxide::writer::ImageFormat::Raw => ImageFormat::Raw,
            },
            data: image.data,
            width: image.width,
            height: image.height,
            bits_per_component: image.bits_per_component,
            color_space: match image.color_space {
                pdf_oxide::writer::ColorSpace::DeviceGray => ColorSpace::Gray,
                pdf_oxide::writer::ColorSpace::DeviceRGB => ColorSpace::RGB,
                pdf_oxide::writer::ColorSpace::DeviceCMYK => ColorSpace::CMYK,
            },
            reading_order: None,
            alt_text: None,
            horizontal_dpi: None,
            vertical_dpi: None,
            soft_mask: image.soft_mask,
            matrix: None,
            is_artifact: false,
        };
        let mut target = writer.add_page(page.points.0, page.points.1);
        target.add_element(&ContentElement::Image(image));
        target.finish();
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn dimensions_and_counters_reject_before_allocation() {
            assert!(pixels(0, 1).is_err());
            assert!(pixels(8193, 1).is_err());
            assert!(dimension(f32::INFINITY, 1.0, false).is_err());
            assert!(dimension(10_000.0, 1.0, false).is_err());
            let mut total = u64::MAX;
            assert!(reserve(&mut total, 1, u64::MAX, "test").is_err());
        }
    }
}

#[cfg(not(feature = "pdf-oxide-barcodes"))]
fn barcode(_ctx: &Context<'_>, _params: &Value) -> Result<OperationOutput, BackendError> {
    Err(BackendError::Unsupported(
        "barcode generation requires the pdf-oxide-barcodes build feature".into(),
    ))
}

#[cfg(feature = "pdf-oxide-barcodes")]
#[allow(clippy::too_many_lines)]
fn barcode(ctx: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    use crate::oxide::Artifact;
    use pdf_oxide::writer::barcode::{
        BarcodeGenerator, BarcodeOptions, BarcodeType, QrCodeOptions, QrErrorCorrection,
    };
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Params {
        kind: String,
        text: String,
        #[serde(default)]
        width: Option<u32>,
        #[serde(default)]
        height: Option<u32>,
        #[serde(default)]
        size: Option<u32>,
        #[serde(default)]
        format: Option<String>,
        #[serde(default)]
        error_correction: Option<String>,
        #[serde(default)]
        quiet_zone: Option<u32>,
        #[serde(default)]
        show_text: bool,
        #[serde(default)]
        foreground: Option<[u8; 4]>,
        #[serde(default)]
        background: Option<[u8; 4]>,
    }
    let invalid = |message: &str| BackendError::Malformed(format!("barcode parameters: {message}"));
    let p: Params = serde_json::from_value(params.clone()).map_err(|e| invalid(&e.to_string()))?;
    if !ctx.inputs.is_empty() {
        return Err(invalid(
            "barcode generation accepts parameters and no document inputs",
        ));
    }
    if p.text.is_empty() || p.text.len() > 4096 {
        return Err(BackendError::Limit(
            "barcode content must contain 1..=4096 UTF-8 bytes".into(),
        ));
    }
    if p.show_text {
        return Err(BackendError::Unsupported(
            "pdf-oxide 0.3.78 does not implement human-readable barcode captions".into(),
        ));
    }
    let format = p.format.as_deref().unwrap_or("png");
    if !["png", "svg", "pdf"].contains(&format) {
        return Err(invalid("format must be png, svg, or pdf"));
    }
    if format == "svg"
        && [p.foreground, p.background]
            .iter()
            .flatten()
            .any(|color| color[3] != 255)
    {
        return Err(BackendError::Unsupported("SVG barcode generation does not preserve alpha in pdf-oxide 0.3.78; use opaque colors or PNG/PDF".into()));
    }
    let (width, height) = if p.kind == "qr" {
        let size = p.size.unwrap_or(256);
        (size, size)
    } else {
        (p.width.unwrap_or(256), p.height.unwrap_or(96))
    };
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > ctx.limits.max_pixels
    {
        return Err(BackendError::Limit(
            "barcode dimension or pixel limit exceeded".into(),
        ));
    }
    if ctx.limits.max_items == 0 {
        return Err(BackendError::Limit("barcode item limit is zero".into()));
    }
    let convert = |e: pdf_oxide::Error| BackendError::Malformed(format!("barcode generation: {e}"));
    let mut bytes = if p.kind == "qr" {
        if p.width.is_some() || p.height.is_some() {
            return Err(invalid("QR dimensions use size"));
        }
        let quiet = p.quiet_zone.unwrap_or(4);
        if quiet > 16 || (177_u64 + 2 * u64::from(quiet)).pow(2) > ctx.limits.max_pixels {
            return Err(BackendError::Limit(
                "QR intermediate pixel budget exceeded".into(),
            ));
        }
        let correction = match p.error_correction.as_deref().unwrap_or("medium") {
            "low" => QrErrorCorrection::Low,
            "medium" => QrErrorCorrection::Medium,
            "quartile" => QrErrorCorrection::Quartile,
            "high" => QrErrorCorrection::High,
            _ => return Err(invalid("unknown QR error correction")),
        };
        let mut options = QrCodeOptions::default()
            .size(width)
            .quiet_zone(quiet)
            .error_correction(correction);
        if let Some(color) = p.foreground {
            options.foreground = color;
        }
        if let Some(color) = p.background {
            options.background = color;
        }
        if format == "svg" {
            BarcodeGenerator::generate_qr_svg(&p.text, &options)
                .map_err(convert)?
                .into_bytes()
        } else {
            BarcodeGenerator::generate_qr(&p.text, &options).map_err(convert)?
        }
    } else {
        if !p.text.is_ascii() {
            return Err(invalid("1D barcode content must be ASCII"));
        }
        if p.size.is_some() || p.quiet_zone.is_some() || p.error_correction.is_some() {
            return Err(invalid(
                "size, quiet_zone and error_correction apply only to QR",
            ));
        }
        // The upstream PNG encoder allocates unscaled bars first. This
        // conservative bound covers all eight encodings before resizing.
        let intermediate_width = (p.text.len() as u64 + 8) * 32;
        let intermediate = intermediate_width * u64::from(height);
        if intermediate_width > 8192 || intermediate > ctx.limits.max_pixels {
            return Err(BackendError::Limit(
                "barcode intermediate pixel budget exceeded".into(),
            ));
        }
        let kind = match p.kind.as_str() {
            "code128" => BarcodeType::Code128,
            "code39" => BarcodeType::Code39,
            "ean13" => BarcodeType::Ean13,
            "ean8" => BarcodeType::Ean8,
            "upca" => BarcodeType::UpcA,
            "itf" => BarcodeType::Itf,
            "code93" => BarcodeType::Code93,
            "codabar" => BarcodeType::Codabar,
            _ => return Err(invalid("unknown barcode kind")),
        };
        let payload = checked_numeric_payload(&p.kind, &p.text)?;
        let mut options = BarcodeOptions::default().width(width).height(height);
        if let Some(color) = p.foreground {
            options.foreground = color;
        }
        if let Some(color) = p.background {
            options.background = color;
        }
        if format == "svg" {
            BarcodeGenerator::generate_1d_svg(kind, payload, &options)
                .map_err(convert)?
                .into_bytes()
        } else {
            BarcodeGenerator::generate_1d(kind, payload, &options).map_err(convert)?
        }
    };
    if bytes.len() as u64 > ctx.limits.max_output_bytes {
        return Err(BackendError::Limit(
            "barcode encoded byte limit exceeded".into(),
        ));
    }
    if format == "pdf" {
        bytes = pdf_oxide::api::Pdf::from_image_bytes(&bytes)
            .map_err(convert)?
            .into_bytes();
    }
    if bytes.len() as u64 > ctx.limits.max_output_bytes {
        return Err(BackendError::Limit(
            "barcode PDF byte limit exceeded".into(),
        ));
    }
    let mime = match format {
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        _ => "image/png",
    };
    let name = format!("barcode.{format}");
    Ok(OperationOutput { data:json!({"kind":p.kind,"artifact":name,"format":format,"requested_width":width,"requested_height":height,"status":"generated","scan_verification":"not_performed"}), artifacts:vec![Artifact { name,mime:mime.into(),bytes }], warnings:vec!["Generation does not establish scanner readability. SVG size follows upstream module dimensions; PDF uses upstream page-fit layout.".into()] })
}

/// Upstream UPC-A accepts a full code by discarding its last byte. Validate a
/// supplied EAN/UPC check digit before supplying body-only ASCII to the encoder.
#[cfg(feature = "pdf-oxide-barcodes")]
fn checked_numeric_payload<'a>(kind: &str, text: &'a str) -> Result<&'a str, BackendError> {
    let body_len = match kind {
        "ean13" => 12,
        "ean8" => 7,
        "upca" => 11,
        _ => return Ok(text),
    };
    if !text.bytes().all(|byte| byte.is_ascii_digit())
        || ![body_len, body_len + 1].contains(&text.len())
    {
        return Err(BackendError::Malformed(format!(
            "{kind} requires {body_len} body digits, optionally followed by one valid check digit"
        )));
    }
    let body = &text[..body_len];
    if text.len() > body_len {
        let sum: u32 = body
            .bytes()
            .rev()
            .enumerate()
            .map(|(index, byte)| u32::from(byte - b'0') * if index % 2 == 0 { 3 } else { 1 })
            .sum();
        let expected = (10 - sum % 10) % 10;
        if u32::from(text.as_bytes()[body_len] - b'0') != expected {
            return Err(BackendError::Malformed(format!(
                "{kind} supplied check digit is invalid"
            )));
        }
    }
    Ok(body)
}
