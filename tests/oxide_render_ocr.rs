#![cfg(feature = "pdf-oxide")]

use lopdf::{Document, Object, Stream, dictionary};
use serde_json::json;
use tpe::backend::BackendError;
use tpe::oxide::{Context, Input, Limits, execute};

/// Repository-owned vector fixture; no system font or external resource.
fn vector_pdf(pages: usize) -> Vec<u8> {
    let mut document = Document::with_version("1.5");
    let tree = document.new_object_id();
    let mut children = Vec::new();
    for _ in 0..pages {
        let content = document.add_object(Stream::new(
            dictionary! {},
            b"0 0 1 rg 10 10 20 30 re f\n".to_vec(),
        ));
        let page = document.add_object(dictionary! {
            "Type"=>"Page", "Parent"=>tree, "Contents"=>content,
            "MediaBox"=>vec![0.into(),0.into(),72.into(),144.into()],
            "Resources"=>dictionary! {},
        });
        children.push(Object::Reference(page));
    }
    document.objects.insert(
        tree,
        dictionary! {"Type"=>"Pages","Count"=>i64::try_from(pages).unwrap(),"Kids"=>children}
            .into(),
    );
    let root = document.add_object(dictionary! {"Type"=>"Catalog","Pages"=>tree});
    document.trailer.set("Root", root);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn ml_capabilities_report_removed_upstream_implementations() {
    let limits = Limits::default();
    let context = Context::new(&[], None, None, &limits);
    let output = execute("ml.capabilities", &context, &json!({})).unwrap();
    assert_eq!(output.data["has_ml_reading_order"], false);
    assert_eq!(output.data["has_ml_heading_detection"], false);
    assert_eq!(output.data["ml_models_loaded"], false);
    assert_eq!(output.data["ocr_models_bundled"], false);
    assert_eq!(output.data["model_acquisition"], "disabled");
    assert!(
        execute(
            "ml.capabilities",
            &context,
            &json!({"model_path":"/tmp/ignored"})
        )
        .is_err()
    );
}

#[test]
fn classification_preserves_source_and_one_based_page_identity() {
    let inputs = vec![Input::new(vector_pdf(2))];
    let before = inputs[0].bytes.clone();
    let limits = Limits::default();
    let context = Context::new(&inputs, None, Some((2, 2)), &limits);
    let output = execute("ocr.classify", &context, &json!({})).unwrap();
    assert_eq!(output.data["source_sha256"], inputs[0].sha256);
    assert_eq!(output.data["pages"].as_array().unwrap().len(), 1);
    assert_eq!(output.data["pages"][0]["page"], 2);
    assert_eq!(output.data["pages"][0]["upstream"]["page"], 1);
    assert_eq!(output.data["ocr_performed"], false);
    assert_eq!(inputs[0].bytes, before);
    assert!(output.artifacts.is_empty());
}

#[test]
fn classification_rejects_malformed_pdf_and_unbounded_selection() {
    let limits = Limits::default();
    let malformed = [Input::new(b"not a PDF".to_vec())];
    assert!(
        execute(
            "ocr.classify",
            &Context::new(&malformed, None, None, &limits),
            &json!({})
        )
        .is_err()
    );
    let inputs = [Input::new(vector_pdf(2))];
    for pages in [(0, 1), (2, 1), (1, u32::MAX)] {
        assert!(matches!(
            execute(
                "ocr.classify",
                &Context::new(&inputs, None, Some(pages), &limits),
                &json!({})
            ),
            Err(BackendError::PageRange { .. })
        ));
    }
    let small = Limits {
        max_pages: 1,
        ..limits
    };
    assert!(matches!(
        execute(
            "ocr.classify",
            &Context::new(&inputs, None, None, &small),
            &json!({})
        ),
        Err(BackendError::Limit(_))
    ));
}

#[test]
fn rendering_never_silently_reads_unconfined_host_fonts() {
    let inputs = [Input::new(vector_pdf(1))];
    let limits = Limits::default();
    let context = Context::new(&inputs, None, None, &limits);
    for (operation, params) in [
        ("render.pages", json!({})),
        ("render.fit", json!({"width":100,"height":100})),
        ("render.region", json!({"crop":[0,0,36,72]})),
        ("render.separations", json!({})),
        ("render.flatten", json!({})),
    ] {
        let error = execute(operation, &context, &params).unwrap_err();
        assert!(
            matches!(error, BackendError::Unsupported(_)),
            "{operation}: {error}"
        );
        assert!(error.to_string().contains("feature") || error.to_string().contains("containment"));
    }
}

#[test]
fn ocr_requires_explicit_build_and_model_bytes() {
    let inputs = [Input::new(vector_pdf(1))];
    let limits = Limits::default();
    let context = Context::new(&inputs, None, None, &limits);
    let error = execute("ocr.images", &context, &json!({})).unwrap_err();
    assert!(matches!(error, BackendError::Unsupported(_)));
    assert!(error.to_string().contains("model") || error.to_string().contains("ONNX"));
}

#[cfg(feature = "pdf-oxide-ocr")]
#[test]
fn offline_models_require_matching_hashes_before_loading_or_host_access() {
    let inputs = [
        Input::new(vector_pdf(1)),
        Input::new(b"synthetic-invalid-detection-graph".to_vec()),
        Input::new(b"synthetic-invalid-recognition-graph".to_vec()),
        Input::new(b"a\nb\n ".to_vec()),
    ];
    let limits = Limits::default();
    let context = Context::new(&inputs, None, None, &limits);
    let error = execute(
        "ocr.images",
        &context,
        &json!({
            "detection_sha256":"0".repeat(64),
            "recognition_sha256":inputs[2].sha256,
            "dictionary_sha256":inputs[3].sha256,
        }),
    )
    .unwrap_err();
    assert!(matches!(error, BackendError::Malformed(_)));
    assert!(error.to_string().contains("SHA-256 mismatch"));
    let error = execute(
        "ocr.images",
        &context,
        &json!({"detection_model_path":"/tmp/model.onnx"}),
    )
    .unwrap_err();
    assert!(matches!(error, BackendError::Malformed(_)));
}

#[cfg(feature = "pdf-oxide-barcodes")]
#[test]
fn barcode_generates_png_svg_pdf_and_bounds_intermediates() {
    let limits = Limits::default();
    let context = Context::new(&[], None, None, &limits);
    let png = execute(
        "barcode.generate",
        &context,
        &json!({"kind":"qr","text":"Synthetic public fixture","size":256}),
    )
    .unwrap();
    let bytes = &png.artifacts[0].bytes;
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 256);
    assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), 256);
    let svg = execute("barcode.generate", &context, &json!({"kind":"qr","text":"<script src='https://example.invalid/a'></script>","format":"svg"})).unwrap();
    let svg = std::str::from_utf8(&svg.artifacts[0].bytes).unwrap();
    assert!(svg.starts_with("<svg"));
    assert!(!svg.contains("<script"));
    assert!(!svg.contains("example.invalid"));
    let pdf = execute(
        "barcode.generate",
        &context,
        &json!({"kind":"code128","text":"12345678","format":"pdf"}),
    )
    .unwrap();
    let reopened = pdf_oxide::PdfDocument::from_bytes(pdf.artifacts[0].bytes.clone()).unwrap();
    assert_eq!(reopened.page_count().unwrap(), 1);
    assert_eq!(reopened.page_image_handles(0).unwrap().len(), 1);
    for params in [
        json!({"kind":"qr","text":"bounded","size":u32::MAX}),
        json!({"kind":"qr","text":"bounded","quiet_zone":u32::MAX}),
        json!({"kind":"code128","text":"x".repeat(4096),"height":8192}),
    ] {
        assert!(matches!(
            execute("barcode.generate", &context, &params),
            Err(BackendError::Limit(_))
        ));
    }
    let limited = Limits {
        max_output_bytes: 64,
        ..limits
    };
    assert!(matches!(
        execute(
            "barcode.generate",
            &Context::new(&[], None, None, &limited),
            &json!({"kind":"qr","text":"output limit"})
        ),
        Err(BackendError::Limit(_))
    ));
}

#[cfg(feature = "pdf-oxide-barcodes")]
#[test]
fn barcode_rejects_unimplemented_caption_and_unsafe_multibyte_numeric_paths() {
    let limits = Limits::default();
    let context = Context::new(&[], None, None, &limits);
    assert!(matches!(
        execute(
            "barcode.generate",
            &context,
            &json!({"kind":"code128","text":"123","show_text":true})
        ),
        Err(BackendError::Unsupported(_))
    ));
    assert!(matches!(
        execute(
            "barcode.generate",
            &context,
            &json!({"kind":"upca","text":"1234567890é"})
        ),
        Err(BackendError::Malformed(_))
    ));
    for (kind, valid, invalid) in [
        ("upca", "036000291452", "036000291453"),
        ("ean13", "5901234123457", "5901234123458"),
        ("ean8", "12345670", "12345671"),
    ] {
        assert!(
            execute(
                "barcode.generate",
                &context,
                &json!({"kind":kind,"text":valid})
            )
            .is_ok(),
            "{kind}"
        );
        let error = execute(
            "barcode.generate",
            &context,
            &json!({"kind":kind,"text":invalid}),
        )
        .unwrap_err();
        assert!(error.to_string().contains("check digit"), "{error}");
    }
}

#[cfg(feature = "pdf-oxide-barcodes")]
#[test]
fn all_pinned_one_dimensional_barcode_encoders_return_png() {
    let limits = Limits::default();
    let context = Context::new(&[], None, None, &limits);
    for (kind, text) in [
        ("code128", "12345678"),
        ("code39", "HELLO-123"),
        ("ean13", "590123412345"),
        ("ean8", "1234567"),
        ("upca", "03600029145"),
        ("itf", "12345678"),
        ("code93", "HELLO123"),
        ("codabar", "A1234B"),
    ] {
        let output = execute(
            "barcode.generate",
            &context,
            &json!({"kind":kind,"text":text}),
        )
        .unwrap_or_else(|error| panic!("{kind}: {error}"));
        let bytes = &output.artifacts[0].bytes;
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "{kind}");
        assert_eq!(
            u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            256,
            "{kind}"
        );
        assert_eq!(
            u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
            96,
            "{kind}"
        );
    }
}

/// This probe is reached through a separate test process, never the host test
/// runner. The sealed policy applies to every thread before any rendering.
#[cfg(all(feature = "pdf-oxide-rendering", target_os = "linux"))]
#[test]
#[allow(clippy::too_many_lines)]
fn sealed_raster_probe() {
    if std::env::var_os("TPE_TEST_SEALED_RASTER").is_none() {
        return;
    }
    cap_disposable_probe();
    let bytes = vector_pdf(1);
    let before = bytes.clone();
    let mut cmyk_document = Document::load_mem(&bytes).unwrap();
    let mut rotated_document = Document::load_mem(&bytes).unwrap();
    for object in cmyk_document.objects.values_mut() {
        if let Object::Stream(stream) = object {
            stream.content = b"1 0 0 0 k 10 10 20 30 re f\n".to_vec();
        }
    }
    for object in rotated_document.objects.values_mut() {
        if let Ok(page) = object.as_dict_mut()
            && page.has_type(b"Page")
        {
            page.set("Rotate", 90);
        }
    }
    let mut cmyk = Vec::new();
    cmyk_document.save_to(&mut cmyk).unwrap();
    let mut rotated = Vec::new();
    rotated_document.save_to(&mut rotated).unwrap();
    let original = [Input::new(bytes)];
    let cmyk = [Input::new(cmyk)];
    let rotated = [Input::new(rotated)];
    let embedded = [Input::new(
        include_bytes!("fixtures/pdfium-unicode/partial-cmap.pdf").to_vec(),
    )];
    let unembedded = [Input::new(
        include_bytes!("fixtures/native-worker/native.pdf").to_vec(),
    )];
    let limits = Limits::default();

    let evidence = tpe_ffi::oxide_containment::install_sealed_process()
        .expect("sealed-process confinement must be enforced before rendering");
    assert!(tpe_ffi::oxide_containment::installed());
    assert_eq!(evidence.mode, "seccomp_sealed");
    let context = Context::new(&original, None, None, &limits);
    let raw = execute("render.pages", &context, &json!({"dpi":72,"format":"rgba"})).unwrap();
    assert_eq!(raw.data["pages"][0]["width"], 72);
    assert_eq!(raw.data["pages"][0]["height"], 144);
    let pixels = &raw.artifacts[0].bytes;
    let inside = (120 * 72 + 15) * 4;
    assert_eq!(&pixels[inside..inside + 4], &[0, 0, 255, 255]);
    assert_eq!(&pixels[..4], &[255, 255, 255, 255]);
    let turn = execute(
        "render.pages",
        &Context::new(&rotated, None, None, &limits),
        &json!({"dpi":72,"format":"rgba"}),
    )
    .unwrap();
    assert_eq!(turn.data["pages"][0]["width"], 144);
    assert_eq!(turn.data["pages"][0]["height"], 72);
    let inside = (15 * 144 + 20) * 4;
    assert_eq!(
        &turn.artifacts[0].bytes[inside..inside + 4],
        &[0, 0, 255, 255]
    );

    let fit = execute("render.fit", &context, &json!({"width":36,"height":36})).unwrap();
    assert_eq!(fit.data["pages"][0]["width"], 18);
    assert_eq!(fit.data["pages"][0]["height"], 36);
    assert!(fit.data["dpi"].is_null());
    let crop = execute(
        "render.region",
        &context,
        &json!({"dpi":72,"crop":[10,10,20,30]}),
    )
    .unwrap();
    assert_eq!(crop.data["pages"][0]["width"], 20);
    assert_eq!(crop.data["pages"][0]["height"], 30);
    let crop_pdf = pdf_oxide::api::Pdf::from_image_bytes(&crop.artifacts[0].bytes).unwrap();
    let crop_doc = pdf_oxide::PdfDocument::from_bytes(crop_pdf.into_bytes()).unwrap();
    let image = crop_doc.page_image_handles(0).unwrap()[0]
        .decode()
        .unwrap()
        .to_dynamic_image()
        .unwrap();
    assert!(image.to_rgb8().pixels().all(|pixel| pixel.0 == [0, 0, 255]));
    assert!(matches!(
        execute(
            "render.region",
            &Context::new(&rotated, None, None, &limits),
            &json!({"dpi":72,"crop":[10,10,20,30]})
        ),
        Err(BackendError::Unsupported(_))
    ));

    let plates = execute(
        "render.separations",
        &Context::new(&cmyk, None, None, &limits),
        &json!({"dpi":72,"format":"pgm"}),
    )
    .unwrap();
    assert_eq!(plates.artifacts.len(), 4);
    let cyan = plates.data["pages"]
        .as_array()
        .unwrap()
        .iter()
        .position(|plate| plate["ink"] == "Cyan")
        .unwrap();
    let header = b"P5\n72 144\n255\n";
    assert!(plates.artifacts[cyan].bytes.starts_with(header));
    assert_eq!(
        plates.artifacts[cyan].bytes[header.len() + 120 * 72 + 15],
        255
    );
    for (index, plate) in plates.artifacts.iter().enumerate() {
        if index != cyan {
            assert!(plate.bytes[header.len()..].iter().all(|pixel| *pixel == 0));
        }
    }
    let flat = execute("render.flatten", &context, &json!({"dpi":72})).unwrap();
    let reopened = pdf_oxide::PdfDocument::from_bytes(flat.artifacts[0].bytes.clone()).unwrap();
    assert_eq!(reopened.page_count().unwrap(), 1);
    assert_eq!(
        reopened.get_page_media_box(0).unwrap(),
        (0.0, 0.0, 72.0, 144.0)
    );
    assert_eq!(reopened.page_image_handles(0).unwrap().len(), 1);
    assert_eq!(flat.data["pages"][0]["derivative_page"], 1);
    assert_eq!(original[0].bytes, before);
    let embedded_image = execute(
        "render.pages",
        &Context::new(&embedded, None, None, &limits),
        &json!({"dpi":72,"format":"rgba"}),
    )
    .unwrap();
    let nonwhite = embedded_image.artifacts[0]
        .bytes
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[..3] != [255, 255, 255])
        .count();
    assert!(
        nonwhite > 500,
        "embedded approved DejaVu fixture must paint glyph pixels"
    );
    let unembedded_image = execute(
        "render.pages",
        &Context::new(&unembedded, None, None, &limits),
        &json!({"dpi":72}),
    )
    .unwrap();
    assert_eq!(unembedded_image.data["status"], "partial");
    assert!(
        unembedded_image
            .warnings
            .iter()
            .any(|warning| warning.contains("rectangle placeholders"))
    );

    let pixel_limit = Limits {
        max_pixels: 1,
        ..limits.clone()
    };
    assert!(matches!(
        execute(
            "render.pages",
            &Context::new(&original, None, None, &pixel_limit),
            &json!({"dpi":72})
        ),
        Err(BackendError::Limit(_))
    ));
    let output_limit = Limits {
        max_output_bytes: 64,
        ..limits
    };
    assert!(matches!(
        execute(
            "render.pages",
            &Context::new(&original, None, None, &output_limit),
            &json!({"dpi":72,"format":"rgba"})
        ),
        Err(BackendError::Limit(_))
    ));
    println!(
        "SEALED_RASTER_ASSERTIONS_PASSED: pages, rotation, fit, crop, CMYK separations, raster PDF, approved embedded font pixels, unembedded font uncertainty, input preservation, pixel/output bounds"
    );
}

#[cfg(all(feature = "pdf-oxide-rendering", target_os = "linux"))]
#[test]
fn real_raster_operations_run_only_in_a_bounded_sealed_child() {
    bounded_probe(
        "sealed_raster_probe",
        "TPE_TEST_SEALED_RASTER",
        "SEALED_RASTER_ASSERTIONS_PASSED",
    );
}

#[cfg(all(feature = "pdf-oxide-ocr", target_os = "linux"))]
#[test]
fn sealed_invalid_ocr_model_probe() {
    if std::env::var_os("TPE_TEST_SEALED_OCR").is_none() {
        return;
    }
    cap_disposable_probe();
    let inputs = [
        Input::new(vector_pdf(1)),
        Input::new(b"not-an-onnx-detection-model".to_vec()),
        Input::new(b"not-an-onnx-recognition-model".to_vec()),
        Input::new(b"a\nb\n ".to_vec()),
    ];
    let limits = Limits::default();
    let parameters = json!({
        "detection_sha256":inputs[1].sha256,
        "recognition_sha256":inputs[2].sha256,
        "dictionary_sha256":inputs[3].sha256,
    });
    tpe_ffi::oxide_containment::install_sealed_process().expect("sealed OCR worker policy");
    let error = execute(
        "ocr.images",
        &Context::new(&inputs, None, None, &limits),
        &parameters,
    )
    .unwrap_err();
    assert!(matches!(error, BackendError::Malformed(_)));
    assert!(error.to_string().contains("model load failed"), "{error}");
    println!("SEALED_OCR_MALFORMED_MODEL_REJECTED; successful recognition remains unverified");
}

#[cfg(all(feature = "pdf-oxide-ocr", target_os = "linux"))]
#[test]
fn malformed_offline_model_fails_in_a_bounded_sealed_child() {
    bounded_probe(
        "sealed_invalid_ocr_model_probe",
        "TPE_TEST_SEALED_OCR",
        "SEALED_OCR_MALFORMED_MODEL_REJECTED",
    );
}

/// Tiny deterministic ONNX fixtures, generated without a model download or a
/// protobuf dependency. Field numbers follow the official ONNX 1.14.1 schema:
/// <https://github.com/onnx/onnx/blob/v1.14.1/onnx/onnx.proto>
/// These graphs exercise inference plumbing, not recognition of image content.
#[cfg(all(feature = "pdf-oxide-ocr", target_os = "linux"))]
mod synthetic_onnx {
    fn varint(mut value: u64) -> Vec<u8> {
        let mut bytes = Vec::new();
        while value >= 128 {
            bytes.push(u8::try_from(value & 127).unwrap() | 128);
            value >>= 7;
        }
        bytes.push(u8::try_from(value).unwrap());
        bytes
    }

    fn integer(number: u64, value: u64) -> Vec<u8> {
        [varint(number << 3), varint(value)].concat()
    }

    fn field(number: u64, value: &[u8]) -> Vec<u8> {
        [
            varint((number << 3) | 2),
            varint(u64::try_from(value.len()).unwrap()),
            value.to_vec(),
        ]
        .concat()
    }

    fn value_info(name: &str, dimensions: &[Option<u64>]) -> Vec<u8> {
        let mut shape = Vec::new();
        for (index, dimension) in dimensions.iter().enumerate() {
            let dimension = dimension.map_or_else(
                || field(2, format!("dynamic_{index}").as_bytes()),
                |size| integer(1, size),
            );
            shape.extend(field(1, &dimension));
        }
        let tensor_type = [integer(1, 1), field(2, &shape)].concat();
        let value_type = field(1, &tensor_type);
        [field(1, name.as_bytes()), field(2, &value_type)].concat()
    }

    fn tensor(name: &str, dimensions: &[u64], data: &[f32]) -> Vec<u8> {
        assert_eq!(dimensions.iter().product::<u64>(), data.len() as u64);
        let mut tensor = Vec::new();
        for dimension in dimensions {
            tensor.extend(integer(1, *dimension));
        }
        tensor.extend(integer(2, 1)); // TensorProto.FLOAT
        tensor.extend(field(8, name.as_bytes()));
        let raw: Vec<_> = data.iter().flat_map(|value| value.to_le_bytes()).collect();
        tensor.extend(field(9, &raw));
        tensor
    }

    fn node(operation: &str, inputs: &[&str], output: &str) -> Vec<u8> {
        let mut node = Vec::new();
        for input in inputs {
            node.extend(field(1, input.as_bytes()));
        }
        node.extend(field(2, output.as_bytes()));
        node.extend(field(4, operation.as_bytes()));
        node
    }

    /// Output = ReduceSum(x) * 0 + the supplied tensor. The graph declares and
    /// consumes its image input; all coefficients are synthetic 0 or 1 values.
    pub(super) fn graph(dimensions: &[u64], values: &[f32]) -> Vec<u8> {
        let mut reduce = node("ReduceSum", &["x"], "input_sum");
        let keepdims = [field(1, b"keepdims"), integer(3, 0), integer(20, 2)].concat();
        reduce.extend(field(5, &keepdims));
        let mut graph = field(1, &reduce);
        graph.extend(field(1, &node("Mul", &["input_sum", "zero"], "input_zero")));
        graph.extend(field(1, &node("Add", &["base", "input_zero"], "y")));
        graph.extend(field(2, b"synthetic_offline_plumbing_only"));
        graph.extend(field(5, &tensor("zero", &[], &[0.0])));
        graph.extend(field(5, &tensor("base", dimensions, values)));
        graph.extend(field(11, &value_info("x", &[Some(1), Some(3), None, None])));
        let dimensions: Vec<_> = dimensions.iter().copied().map(Some).collect();
        graph.extend(field(12, &value_info("y", &dimensions)));
        [
            integer(1, 7), // ONNX IR version 7
            field(2, b"repository-owned deterministic OCR fixture"),
            field(7, &graph),
            field(8, &integer(2, 11)), // default-domain operator set 11
        ]
        .concat()
    }
}

#[cfg(all(feature = "pdf-oxide-ocr", target_os = "linux"))]
fn repeated_scan_pdf(width: u32, height: u32) -> Vec<u8> {
    let mut document = Document::with_version("1.5");
    let tree = document.new_object_id();
    let mut pixels = vec![255; usize::try_from(width * height * 3).unwrap()];
    for y in 8..height.min(24) {
        for x in 8..width.min(24) {
            let offset = usize::try_from((y * width + x) * 3).unwrap();
            pixels[offset..offset + 3].fill(0);
        }
    }
    let scan = document.add_object(Stream::new(
        dictionary! {
            "Type"=>"XObject", "Subtype"=>"Image", "Width"=>i64::from(width),
            "Height"=>i64::from(height), "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>8,
        },
        pixels,
    ));
    let content = document.add_object(Stream::new(
        dictionary! {},
        b"q 32 0 0 32 5 8 cm /Scan Do Q\nq 32 0 0 32 50 8 cm /Scan Do Q\n".to_vec(),
    ));
    let page = document.add_object(dictionary! {
        "Type"=>"Page", "Parent"=>tree, "Contents"=>content,
        "MediaBox"=>vec![0.into(),0.into(),96.into(),48.into()],
        "Resources"=>dictionary! {"XObject"=>dictionary! {"Scan"=>scan}},
    });
    document.objects.insert(
        tree,
        dictionary! {"Type"=>"Pages", "Count"=>1, "Kids"=>vec![Object::Reference(page)]}.into(),
    );
    let root = document.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>tree});
    document.trailer.set("Root", root);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

#[cfg(all(feature = "pdf-oxide-ocr", target_os = "linux"))]
#[test]
#[allow(clippy::too_many_lines)]
fn sealed_synthetic_ocr_model_probe() {
    if std::env::var_os("TPE_TEST_SEALED_SYNTHETIC_OCR").is_none() {
        return;
    }
    cap_disposable_probe();
    let mut detection = vec![0.0; 32 * 32];
    for y in 8..24 {
        for x in 8..24 {
            detection[y * 32 + x] = 1.0;
        }
    }
    let mut recognition = vec![0.0; 5 * 4];
    for (time, class) in [1, 1, 0, 2, 0].into_iter().enumerate() {
        recognition[time * 4 + class] = 1.0;
    }
    let inputs = [
        Input::new(repeated_scan_pdf(32, 32)),
        Input::new(synthetic_onnx::graph(&[1, 1, 32, 32], &detection)),
        Input::new(synthetic_onnx::graph(&[1, 5, 4], &recognition)),
        Input::new(b"A\nB\n \n".to_vec()),
    ];
    let before: Vec<_> = inputs.iter().map(|input| input.bytes.clone()).collect();
    let empty_detection = Input::new(synthetic_onnx::graph(&[1, 1, 32, 32], &[0.0; 1024]));
    let wrong_shape = Input::new(synthetic_onnx::graph(&[1, 4], &[1.0, 0.0, 0.0, 0.0]));
    let no_images = Input::new(vector_pdf(1));
    let mut thin_inputs = inputs.clone();
    thin_inputs[0] = Input::new(repeated_scan_pdf(1024, 1));
    let limits = Limits {
        max_pixels: 16_384,
        max_items: 8,
        ..Limits::default()
    };
    let parameters = |inputs: &[Input]| {
        json!({
            "detection_sha256":inputs[1].sha256,
            "recognition_sha256":inputs[2].sha256,
            "dictionary_sha256":inputs[3].sha256,
            "detection_max_side":32,
        })
    };
    let evidence = tpe_ffi::oxide_containment::install_sealed_process()
        .expect("sealed synthetic OCR worker policy");
    assert_eq!(evidence.mode, "seccomp_sealed");
    let context = Context::new(&inputs, None, None, &limits);
    let output = execute("ocr.images", &context, &parameters(&inputs)).unwrap();
    assert!(output.artifacts.is_empty());
    assert_eq!(output.data["source_sha256"], inputs[0].sha256);
    assert_eq!(output.data["native_text_replaced"], false);
    assert_eq!(output.data["source_preserved"], true);
    assert_eq!(
        output.data["model_verification"],
        "caller_digest_checked_accuracy_unverified"
    );
    assert_eq!(
        output.data["model_sha256"],
        json!([inputs[1].sha256, inputs[2].sha256, inputs[3].sha256,])
    );
    assert_eq!(output.data["pages"][0]["page"], 1);
    let images = output.data["pages"][0]["images"].as_array().unwrap();
    assert_eq!(
        images.len(),
        2,
        "repeated painted images retain separate provenance"
    );
    assert_ne!(images[0]["paint_order"], images[1]["paint_order"]);
    for (index, image) in images.iter().enumerate() {
        assert_eq!(image["width"], 32);
        assert_eq!(image["height"], 32);
        assert_eq!(image["coordinate_space"], "image_pixels_top_left");
        assert_eq!(image["pdf_bbox"][0], if index == 0 { 5.0 } else { 50.0 });
        assert_eq!(image["spans"].as_array().unwrap().len(), 1);
        let span = &image["spans"][0];
        assert_eq!(span["text"], "AB", "CTC duplicate and blank removal");
        assert_eq!(span["confidence"], 1.0);
        assert_eq!(span["character_confidences"], json!([1.0, 1.0]));
        assert!(span["polygon"].as_array().unwrap().iter().all(|point| {
            point
                .as_array()
                .unwrap()
                .iter()
                .all(|coordinate| (0.0..=32.0).contains(&coordinate.as_f64().unwrap()))
        }));
    }
    for (input, bytes) in inputs.iter().zip(before) {
        assert_eq!(input.bytes, bytes);
    }
    for bounded in [
        Limits {
            max_pixels: 1024,
            ..limits.clone()
        },
        Limits {
            max_items: 1,
            ..limits.clone()
        },
        Limits {
            max_output_bytes: 64,
            ..limits.clone()
        },
    ] {
        assert!(matches!(
            execute(
                "ocr.images",
                &Context::new(&inputs, None, None, &bounded),
                &parameters(&inputs)
            ),
            Err(BackendError::Limit(_))
        ));
    }
    let error = execute(
        "ocr.images",
        &Context::new(&thin_inputs, None, None, &limits),
        &parameters(&thin_inputs),
    )
    .unwrap_err();
    assert!(matches!(error, BackendError::Limit(_)));
    assert!(
        error.to_string().contains("preprocessing pixel budget"),
        "thin image must fail before its expanded recognition tensor is allocated: {error}"
    );
    let mut inputs = inputs;
    inputs[1] = empty_detection;
    let empty = execute(
        "ocr.images",
        &Context::new(&inputs, None, None, &limits),
        &parameters(&inputs),
    )
    .unwrap();
    assert_eq!(empty.data["pages"][0]["images"][0]["spans"], json!([]));
    assert_eq!(empty.data["pages"][0]["status"], "partial");
    inputs[1] = wrong_shape;
    let error = execute(
        "ocr.images",
        &Context::new(&inputs, None, None, &limits),
        &parameters(&inputs),
    )
    .unwrap_err();
    assert!(matches!(error, BackendError::Malformed(_)));
    assert!(
        error.to_string().contains("Unexpected output shape"),
        "{error}"
    );
    inputs[0] = no_images;
    let empty = execute(
        "ocr.images",
        &Context::new(&inputs, None, None, &limits),
        &parameters(&inputs),
    )
    .unwrap();
    assert_eq!(empty.data["pages"][0]["images"], json!([]));
    assert_eq!(
        empty.data["pages"][0]["status"],
        "no_images_reported_ocr_not_performed"
    );
    println!(
        "SEALED_SYNTHETIC_OCR_ASSERTIONS_PASSED: offline byte loading, input preprocessing, deterministic detector, CTC decoding, repeated-image provenance, hashes, bounds and output-shape errors; real OCR accuracy unverified"
    );
}

#[cfg(all(feature = "pdf-oxide-ocr", target_os = "linux"))]
#[test]
fn synthetic_offline_ocr_runs_in_a_bounded_sealed_child() {
    bounded_probe(
        "sealed_synthetic_ocr_model_probe",
        "TPE_TEST_SEALED_SYNTHETIC_OCR",
        "SEALED_SYNTHETIC_OCR_ASSERTIONS_PASSED",
    );
}

#[cfg(all(
    any(feature = "pdf-oxide-rendering", feature = "pdf-oxide-ocr"),
    target_os = "linux"
))]
fn cap_disposable_probe() {
    use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
    for (resource, requested) in [
        (Resource::As, 1024 * 1024 * 1024),
        (Resource::Cpu, 10),
        (Resource::Core, 0),
    ] {
        let inherited = getrlimit(resource);
        let maximum = inherited
            .maximum
            .map_or(requested, |limit| limit.min(requested));
        let current = inherited
            .current
            .map_or(maximum, |limit| limit.min(maximum));
        setrlimit(
            resource,
            Rlimit {
                current: Some(current),
                maximum: Some(maximum),
            },
        )
        .unwrap();
    }
}

#[cfg(all(
    any(feature = "pdf-oxide-rendering", feature = "pdf-oxide-ocr"),
    target_os = "linux"
))]
fn bounded_probe(name: &str, flag: &str, success: &str) {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env(flag, "1")
        .env("RAYON_NUM_THREADS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("sealed probe {name} exceeded 20s and was killed/reaped");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "confined probe {name} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains(success));
}
