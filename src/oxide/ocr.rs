//! Offline OCR uses request-owned bytes, never model paths, global caches, or
//! AutoExtractor's model acquisition. Recognition remains a derived result.

use serde_json::{Value, json};

use super::{Context, OperationOutput};
use crate::backend::BackendError;

pub const OPERATIONS: &[&str] = &["ocr.classify", "ocr.images", "ml.capabilities"];

pub(super) fn execute(
    op: &str,
    ctx: &Context<'_>,
    params: &Value,
) -> Option<Result<OperationOutput, BackendError>> {
    match op {
        "ocr.classify" => Some(classify(ctx, params)),
        "ocr.images" => Some(recognize(ctx, params)),
        "ml.capabilities" => Some(capabilities(params)),
        _ => None,
    }
}

fn no_parameters(params: &Value) -> Result<(), BackendError> {
    if !params.as_object().is_some_and(serde_json::Map::is_empty) {
        return Err(BackendError::Malformed(
            "this operation accepts an empty parameter object".into(),
        ));
    }
    Ok(())
}

fn capabilities(params: &Value) -> Result<OperationOutput, BackendError> {
    no_parameters(params)?;
    let actual = pdf_oxide::hybrid::SmartLayoutAnalyzer::new().capabilities();
    Ok(OperationOutput {
        data: json!({
            "has_ml_reading_order":actual.has_ml_reading_order,
            "has_ml_heading_detection":actual.has_ml_heading_detection,
            "ml_models_loaded":actual.ml_models_loaded,
            "ocr_backend_compiled":cfg!(feature="pdf-oxide-ocr"),
            "ocr_backend":if cfg!(feature="pdf-oxide-ocr") {Some("tract")} else {None},
            "ocr_models_bundled":false,
            "model_acquisition":"disabled",
            "model_verification":"caller_digests_only; recognition accuracy unverified",
            "status":"capability_report"
        }),
        warnings:vec!["Pinned SmartLayoutAnalyzer has no ML reading-order or heading model implementation, including when its ml feature is enabled. Its reading-order method is classical; deprecated heading detection returns Body for every block.".into()],
        ..OperationOutput::default()
    })
}

fn classify(ctx: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    no_parameters(params)?;
    let doc = ctx.open_pdf()?;
    let failure = |error: pdf_oxide::Error| {
        BackendError::Malformed(format!("PDF Oxide OCR classification: {error}"))
    };
    let pages = ctx.selected_pages(doc.page_count().map_err(failure)?)?;
    if pages.len() > ctx.limits.max_items {
        return Err(BackendError::Limit(
            "OCR classification item limit exceeded".into(),
        ));
    }
    let mut classified = Vec::with_capacity(pages.len());
    for page in pages {
        classified
            .push(json!({"page":page+1,"upstream":doc.classify_page(page).map_err(failure)?}));
    }
    Ok(OperationOutput {
        data:json!({"source_sha256":ctx.inputs[0].sha256,"pages":classified,"ocr_performed":false,"status":"heuristic_classification","diagnostics_page_indexing":"zero_based_upstream","diagnostics":doc.structured_warnings()}),
        warnings:vec!["Page classification is a heuristic, not proof of an empty, image-free, or correctly recognized page. No OCR model was loaded and original extraction was not replaced.".into()],
        ..OperationOutput::default()
    })
}

#[cfg(not(feature = "pdf-oxide-ocr"))]
fn recognize(_ctx: &Context<'_>, _params: &Value) -> Result<OperationOutput, BackendError> {
    Err(BackendError::Unsupported("offline OCR requires the pdf-oxide-ocr build feature (tract) and explicit digest-pinned detection, recognition, and dictionary input bytes; no models are downloaded".into()))
}

#[cfg(feature = "pdf-oxide-ocr")]
#[allow(clippy::too_many_lines)]
fn recognize(ctx: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    use pdf_oxide::ocr::{DetResizeStrategy, OcrConfig, OcrEngine};
    use serde::Deserialize;
    use sha2::{Digest, Sha256};

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Params {
        #[serde(default)]
        detection_sha256: Option<String>,
        #[serde(default)]
        recognition_sha256: Option<String>,
        #[serde(default)]
        dictionary_sha256: Option<String>,
        #[serde(default)]
        detection_threshold: Option<f32>,
        #[serde(default)]
        box_threshold: Option<f32>,
        #[serde(default)]
        recognition_threshold: Option<f32>,
        #[serde(default)]
        max_candidates: Option<usize>,
        #[serde(default)]
        detection_max_side: Option<u32>,
    }
    let invalid =
        |message: &str| BackendError::Malformed(format!("offline OCR parameters: {message}"));
    let p: Params = serde_json::from_value(params.clone()).map_err(|e| invalid(&e.to_string()))?;
    if ctx.inputs.len() != 4 {
        return Err(BackendError::Unsupported("offline OCR requires exactly four request inputs: PDF, detection ONNX, recognition ONNX, and UTF-8 character dictionary. No approved model fixture is bundled.".into()));
    }
    let mut model_hashes = Vec::new();
    for (index, expected, maximum) in [
        (1, p.detection_sha256.as_deref(), 64 * 1024 * 1024_u64),
        (2, p.recognition_sha256.as_deref(), 64 * 1024 * 1024),
        (3, p.dictionary_sha256.as_deref(), 1024 * 1024),
    ] {
        let input = ctx.input(index)?;
        if input.is_empty() || input.len() as u64 > maximum.min(ctx.limits.max_input_bytes) {
            return Err(BackendError::Limit(format!(
                "OCR model input {index} is empty or exceeds its byte limit"
            )));
        }
        let Some(expected) = expected else {
            return Err(BackendError::Unsupported(format!(
                "OCR model input {index} requires an explicit expected SHA-256 digest"
            )));
        };
        if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid(
                "model SHA-256 values must contain 64 hexadecimal digits",
            ));
        }
        let actual = hex::encode(Sha256::digest(input));
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(BackendError::Malformed(format!(
                "OCR model input {index} SHA-256 mismatch"
            )));
        }
        model_hashes.push(actual);
    }
    let dictionary =
        std::str::from_utf8(ctx.input(3)?).map_err(|_| invalid("dictionary must be UTF-8"))?;
    if dictionary.lines().any(|line| line.chars().count() != 1)
        || dictionary.lines().count() > 65_536
        || dictionary.is_empty()
    {
        return Err(invalid(
            "dictionary must contain 1..=65536 lines, exactly one Unicode character per line",
        ));
    }
    let detection = p.detection_threshold.unwrap_or(0.3);
    let boxes = p.box_threshold.unwrap_or(0.6);
    let recognition = p.recognition_threshold.unwrap_or(0.5);
    if [detection, boxes, recognition]
        .iter()
        .any(|threshold| !threshold.is_finite() || !(0.0..=1.0).contains(threshold))
    {
        return Err(invalid("thresholds must be finite values in 0..=1"));
    }
    let candidates = p.max_candidates.unwrap_or(ctx.limits.max_items.min(256));
    if candidates == 0 || candidates > 1024 || candidates > ctx.limits.max_items {
        return Err(BackendError::Limit(
            "OCR candidate limit must be 1..=1024 within the request item limit".into(),
        ));
    }
    let side = p.detection_max_side.unwrap_or(960);
    let padded_side = u64::from(side).div_ceil(32) * 32;
    if !(32..=2048).contains(&side) || padded_side.pow(2) > ctx.limits.max_pixels {
        return Err(BackendError::Limit(
            "OCR detection side must be 32..=2048 within the pixel budget".into(),
        ));
    }
    // ONNX graphs are inputs too. Their parsing/inference has only the worker's
    // hard resource and filesystem boundary, not an upstream safe-model proof.
    ctx.require_filesystem_containment()?;
    let config = OcrConfig {
        det_threshold: detection,
        box_threshold: boxes,
        rec_threshold: recognition,
        det_max_side: side,
        det_resize_strategy: DetResizeStrategy::MaxSide { max_side: side },
        rec_target_height: 48,
        num_threads: 1,
        unclip_ratio: 1.5,
        max_candidates: candidates,
        detect_styles: false,
        det_model_path: None,
        rec_model_path: None,
        dict_path: None,
    };
    let engine = OcrEngine::from_bytes(ctx.input(1)?, ctx.input(2)?, dictionary, config)
        .map_err(|e| BackendError::Malformed(format!("offline OCR model load failed: {e}")))?;
    let doc = ctx.open_pdf()?;
    let failure = |e: pdf_oxide::Error| {
        BackendError::Malformed(format!("offline OCR PDF/image decoding: {e}"))
    };
    let selected = ctx.selected_pages(doc.page_count().map_err(failure)?)?;
    let mut records = Vec::new();
    let mut image_count = 0_usize;
    let mut total_pixels = 0_u64;
    let mut text_bytes = 0_u64;
    let mut serialized_span_bytes = 0_u64;
    let mut total_spans = 0_usize;
    for page in selected {
        let handles = doc.page_image_handles(page).map_err(failure)?;
        image_count = image_count
            .checked_add(handles.len())
            .ok_or_else(|| BackendError::Limit("OCR image count overflow".into()))?;
        if image_count > ctx.limits.max_items {
            return Err(BackendError::Limit("OCR image count limit exceeded".into()));
        }
        let mut images = Vec::new();
        for handle in handles {
            if ![
                handle.bbox.x,
                handle.bbox.y,
                handle.bbox.width,
                handle.bbox.height,
                handle.rotation_degrees,
            ]
            .iter()
            .all(|coordinate| coordinate.is_finite())
            {
                return Err(BackendError::Malformed(
                    "OCR image placement contains non-finite geometry".into(),
                ));
            }
            let pixels = u64::from(handle.width) * u64::from(handle.height);
            total_pixels = total_pixels
                .checked_add(pixels)
                .ok_or_else(|| BackendError::Limit("OCR pixel counter overflow".into()))?;
            if handle.width == 0
                || handle.height == 0
                || handle.width > 8192
                || handle.height > 8192
                || total_pixels > ctx.limits.max_pixels
                || handle.byte_size_compressed > ctx.limits.max_input_bytes
            {
                return Err(BackendError::Limit(
                    "OCR image metadata exceeds pixel, dimension or compressed byte limit".into(),
                ));
            }
            let decoded = handle.decode().map_err(failure)?;
            if (decoded.width(), decoded.height()) != (handle.width, handle.height) {
                return Err(BackendError::Malformed(
                    "OCR decoded image dimensions disagree with preflight metadata".into(),
                ));
            }
            let image = decoded.to_dynamic_image().map_err(failure)?;
            if (image.width(), image.height()) != (handle.width, handle.height) {
                return Err(BackendError::Malformed(
                    "OCR decoded raster dimensions disagree with preflight metadata".into(),
                ));
            }
            let output = engine.ocr_image(&image).map_err(|e| {
                BackendError::Malformed(format!("offline OCR inference failed: {e}"))
            })?;
            total_spans = total_spans
                .checked_add(output.spans.len())
                .ok_or_else(|| BackendError::Limit("OCR span counter overflow".into()))?;
            if total_spans > ctx.limits.max_items {
                return Err(BackendError::Limit("OCR span count limit exceeded".into()));
            }
            let mut spans = Vec::with_capacity(output.spans.len());
            for span in output.spans {
                if !span.confidence.is_finite()
                    || !span.polygon.iter().flatten().all(|v| v.is_finite())
                    || !span.char_confidences.iter().all(|v| v.is_finite())
                {
                    return Err(BackendError::Malformed(
                        "OCR returned non-finite geometry or confidence".into(),
                    ));
                }
                text_bytes = text_bytes
                    .checked_add(span.text.len() as u64)
                    .ok_or_else(|| BackendError::Limit("OCR text counter overflow".into()))?;
                if text_bytes > ctx.limits.max_output_bytes
                    || span.char_confidences.len().saturating_mul(size_of::<f32>()) as u64
                        > ctx.limits.max_output_bytes
                {
                    return Err(BackendError::Limit(
                        "OCR text/confidence output byte limit exceeded".into(),
                    ));
                }
                let record = json!({"text":span.text,"polygon":span.polygon,"confidence":span.confidence,"character_confidences":span.char_confidences});
                let encoded = serde_json::to_vec(&record)
                    .map_err(|error| BackendError::Malformed(error.to_string()))?;
                serialized_span_bytes = serialized_span_bytes
                    .checked_add(encoded.len() as u64)
                    .ok_or_else(|| {
                        BackendError::Limit("OCR serialized span counter overflow".into())
                    })?;
                if serialized_span_bytes > ctx.limits.max_output_bytes {
                    return Err(BackendError::Limit(
                        "OCR serialized span output byte limit exceeded".into(),
                    ));
                }
                spans.push(record);
            }
            let bbox = &handle.bbox;
            images.push(json!({"paint_order":handle.paint_order,"inline":handle.is_inline,"width":handle.width,"height":handle.height,"pdf_bbox":[bbox.x,bbox.y,bbox.width,bbox.height],"rotation_degrees":handle.rotation_degrees,"coordinate_space":"image_pixels_top_left","spans":spans,"status":"partial"}));
        }
        let status = if images.is_empty() {
            "no_images_reported_ocr_not_performed"
        } else {
            "partial"
        };
        records.push(json!({"page":page+1,"images":images,"status":status}));
    }
    Ok(OperationOutput {
        data:json!({"source_sha256":ctx.inputs[0].sha256,"model_sha256":model_hashes,"model_verification":"caller_digest_checked_accuracy_unverified","pages":records,"source_preserved":true,"native_text_replaced":false,"status":"partial","diagnostics_page_indexing":"zero_based_upstream","diagnostics":doc.structured_warnings()}),
        warnings:vec!["OCR recognizes individual painted images. It does not rasterize the full page, deduplicate repeated images, merge native text, or establish extraction completeness. Empty image/recognition output is not proof that the page lacks text.".into(),"Expected digests check model identity only. No approved model fixture or recognition-accuracy evidence is bundled; model execution is bounded by the disposable worker.".into()],
        ..OperationOutput::default()
    })
}
