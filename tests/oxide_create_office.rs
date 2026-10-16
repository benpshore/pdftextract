#![cfg(feature = "pdf-oxide")]

use std::io::{Cursor, Write};

use pdf_oxide::writer::{DocumentBuilder, PageSize};
use serde_json::{Value, json};
use tpe::backend::BackendError;
use tpe::oxide::{Context, Input, Limits, OperationOutput, execute};
use zip::write::SimpleFileOptions;

fn run(
    operation: &str,
    bytes: Vec<Vec<u8>>,
    params: Value,
) -> Result<OperationOutput, BackendError> {
    run_with_limits(operation, bytes, params, &Limits::default())
}

// Test cases pass owned temporary JSON alongside owned fixture buffers.
#[allow(clippy::needless_pass_by_value)]
fn run_with_limits(
    operation: &str,
    bytes: Vec<Vec<u8>>,
    params: Value,
    limits: &Limits,
) -> Result<OperationOutput, BackendError> {
    let inputs: Vec<_> = bytes.into_iter().map(Input::new).collect();
    execute(
        operation,
        &Context::new(&inputs, None, None, limits),
        &params,
    )
}

fn source_pdf(pages: usize) -> Vec<u8> {
    let mut builder = DocumentBuilder::new();
    for page in 1..=pages {
        builder
            .page(PageSize::Letter)
            .font("Helvetica", 12.0)
            .at(72.0, 700.0)
            .text(&format!("Source page {page}"))
            .done();
    }
    builder.build().unwrap()
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::ImageBuffer::from_pixel(
        width,
        height,
        image::Rgba([30, 80, 150, 127]),
    ))
    .write_to(&mut bytes, image::ImageFormat::Png)
    .unwrap();
    bytes.into_inner()
}

fn zip_parts(parts: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let method = if deflate {
        zip::CompressionMethod::Deflated
    } else {
        zip::CompressionMethod::Stored
    };
    for (name, data) in parts {
        writer
            .start_file(
                *name,
                SimpleFileOptions::default().compression_method(method),
            )
            .unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

const CONTENT_TYPES: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
const RELS: &[u8] = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
const WORD: &[u8] = br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Hello source</w:t></w:r></w:p></w:body></w:document>"#;

fn docx(extra: &[(&str, &[u8])], body: &[u8], deflate: bool) -> Vec<u8> {
    let mut parts = vec![
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", RELS),
        ("word/document.xml", body),
    ];
    parts.extend_from_slice(extra);
    zip_parts(&parts, deflate)
}

#[test]
fn creation_round_trips_text_and_preserves_input_identity() {
    for (operation, content) in [
        ("create.text", "Hello source"),
        ("create.markdown", "# Hello source\n\nBody text"),
        ("create.html", "<h1>Hello source</h1><p>Body text</p>"),
    ] {
        let input = Input::new(content.as_bytes().to_vec());
        let identity = input.sha256.clone();
        let inputs = [input];
        let limits = Limits::default();
        let result = execute(
            operation,
            &Context::new(&inputs, None, None, &limits),
            &json!({}),
        )
        .unwrap();
        let document =
            pdf_oxide::PdfDocument::from_bytes(result.artifacts[0].bytes.clone()).unwrap();
        assert_eq!(document.page_count().unwrap(), 1);
        assert!(document.extract_text(0).unwrap().contains("Hello source"));
        assert_eq!(result.data["source_sha256"][0], identity);
        assert_eq!(inputs[0].bytes, content.as_bytes());
        assert!(!result.warnings.is_empty());
    }
}

#[test]
fn composition_retains_font_size_geometry_and_page_order() {
    let result = run("create.document", Vec::new(), json!({"title":"Synthetic composition", "pages":[
        {"width":400,"height":500,"elements":[
            {"kind":"text","x":40,"y":430,"text":"First source","size":20},
            {"kind":"rect","x":40,"y":100,"width":100,"height":80,"fill":[0.2,0.4,0.6]},
            {"kind":"line","x1":40,"y1":200,"x2":200,"y2":200}
        ]},
        {"width":400,"height":500,"elements":[{"kind":"text","x":40,"y":430,"text":"Second source","size":12}]}
    ]})).unwrap();
    let document = pdf_oxide::PdfDocument::from_bytes(result.artifacts[0].bytes.clone()).unwrap();
    assert_eq!(document.page_count().unwrap(), 2);
    assert!(document.extract_text(0).unwrap().contains("First source"));
    assert!(document.extract_text(1).unwrap().contains("Second source"));
    assert!(
        document
            .extract_chars(0)
            .unwrap()
            .iter()
            .any(|character| (character.font_size - 20.0).abs() < 0.01)
    );
    assert_eq!(result.data["items"], 4);
}

#[test]
fn html_css_rejects_known_page_unit_mismatch_and_resources() {
    // This font is already embedded in the pinned registry package. Its notice
    // is retained under third_party/pdf_oxide; no host font discovery occurs.
    let font = pdf_oxide::fonts::bundled::DEJAVU_SANS.to_vec();
    let params = json!({"css":"p {font-family: 'Fixture'; font-size: 18px}", "fonts":[{"input":1,"family":"Fixture"}]});
    let error = run(
        "create.html_css",
        vec![b"<p>HTML source text</p>".to_vec(), font.clone()],
        params.clone(),
    )
    .unwrap_err();
    assert!(matches!(error, BackendError::Unsupported(_)));
    assert!(error.to_string().contains("page-unit mismatch"));
    for html in [
        "<img src='https://invalid.example/test.png'>",
        "<p style='background:url(file:///private)'>x</p>",
    ] {
        assert!(matches!(
            run(
                "create.html_css",
                vec![html.as_bytes().to_vec(), font.clone()],
                params.clone()
            ),
            Err(BackendError::Unsupported(_))
        ));
    }
}

#[test]
fn buffered_table_creation_fails_explicitly_for_known_layout_defects() {
    let params = json!({"pages":[{"width":500,"height":500,"elements":[{
        "kind":"table","x":50,"y":430,"width":360,"size":12,"header":true,
        "rows":[
            [{"text":"Name"},{"text":"Value"},{"text":"Unit"}],
            [{"text":"Alpha"},{"text":"12"},{"text":"kg"}],
            [{"text":"Beta"},{"text":""},{"text":"kg"}]
        ]
    }]}]});
    let mut wrapped = params.clone();
    wrapped["pages"][0]["elements"][0]["rows"][1][0]["text"] = json!(
        "A long wrapped description that must remain one logical row without crossing column borders"
    );
    let mut empty_row = params.clone();
    empty_row["pages"][0]["elements"][0]["rows"][1] = json!([{"text":""},{"text":""},{"text":""}]);
    for request in [params, wrapped, empty_row] {
        let error = run("create.document", Vec::new(), request).unwrap_err();
        assert!(matches!(error, BackendError::Unsupported(_)));
        assert!(error.to_string().contains("cell borders"));
    }
}

#[test]
fn html_css_table_semantics_fail_before_known_cell_overlap() {
    let font = pdf_oxide::fonts::bundled::DEJAVU_SANS.to_vec();
    for (html, css) in [
        (
            "<table><tr><th>Name</th><th>Value</th></tr><tr><td>Alpha long description</td><td>12</td></tr></table>",
            "",
        ),
        (
            "<div><table><tbody><tr><td>Nested table</td><td>2</td></tr><tr><td></td><td></td></tr></tbody></table></div>",
            "",
        ),
        (
            "<div style='display:table'><span style='display:table-cell'>Alpha</span><span style='display:table-cell'>12</span></div>",
            "",
        ),
        (
            "<div class='grid'><div>Alpha</div><div>12</div></div>",
            ".grid {display: /*hidden*/ t\\61 ble} .grid div {display:table-cell}",
        ),
        (
            "<div><span>Alpha</span><span>12</span></div>",
            "@media print {div {display:table-row} span {display:table-cell}}",
        ),
    ] {
        assert!(
            matches!(
                run(
                    "create.html_css",
                    vec![html.as_bytes().to_vec(), font.clone()],
                    json!({"css":css,"fonts":[{"input":1,"family":"Fixture"}]}),
                ),
                Err(BackendError::Unsupported(_))
            ),
            "{html} / {css}"
        );
    }
}

#[test]
fn image_creation_checks_real_decode_pixels_and_total_count() {
    let first = png(16, 16);
    let second = png(8, 8);
    let result = run(
        "create.images",
        vec![first.clone(), second.clone()],
        json!({}),
    )
    .unwrap();
    assert_eq!(result.data["pages"], 2);
    assert_eq!(result.data["input_pixels"], 320);
    let document = pdf_oxide::PdfDocument::from_bytes(result.artifacts[0].bytes.clone()).unwrap();
    assert_eq!(document.extract_images(0).unwrap()[0].width(), 16);
    // Direct upstream extraction can drop masks; inspect the generated PDF
    // object graph independently to verify creation retained transparency.
    let generated = lopdf::Document::load_mem(&result.artifacts[0].bytes).unwrap();
    let mask_ref = generated
        .objects
        .values()
        .find_map(|object| {
            let stream = object.as_stream().ok()?;
            stream.dict.get(b"SMask").ok()?.as_reference().ok()
        })
        .expect("transparent PNG creation must retain a soft mask");
    let mask = generated.get_object(mask_ref).unwrap().as_stream().unwrap();
    assert_eq!(mask.dict.get(b"Width").unwrap().as_i64().unwrap(), 16);
    assert_eq!(mask.dict.get(b"Height").unwrap().as_i64().unwrap(), 16);

    let small = Limits {
        max_pixels: 300,
        ..Limits::default()
    };
    assert!(matches!(
        run_with_limits(
            "create.images",
            vec![first.clone(), second],
            json!({}),
            &small
        ),
        Err(BackendError::Limit(_))
    ));
    let mut corrupt = first;
    corrupt.truncate(40);
    assert!(run("create.images", vec![corrupt], json!({})).is_err());
    // A valid-sized JPEG header without image entropy must not be embedded as
    // a successfully verified image merely because upstream accepts its SOF.
    let jpeg = vec![
        0xff, 0xd8, 0xff, 0xc0, 0, 11, 8, 0, 1, 0, 1, 1, 1, 0x11, 0, 0xff, 0xd9,
    ];
    assert!(run("create.images", vec![jpeg], json!({})).is_err());
}

#[test]
fn creation_rejects_active_html_invalid_dimensions_and_output_overflow() {
    for html in [
        "<script>alert(1)</script>",
        "<p onclick='alert(1)'>x</p>",
        "<img src='http://127.0.0.1:9/private'>",
        "<iframe src='file:///private'></iframe>",
        "<style>p{color:red}</style><p>x</p>",
    ] {
        assert!(matches!(
            run("create.html", vec![html.as_bytes().to_vec()], json!({})),
            Err(BackendError::Unsupported(_))
        ));
    }
    assert!(run("create.text", vec![b"source".to_vec()], json!({"width":-1})).is_err());
    assert!(matches!(
        run(
            "create.text",
            vec![b"source".to_vec()],
            json!({"font_size":30})
        ),
        Err(BackendError::Unsupported(_))
    ));
    assert!(run("create.text", vec![vec![0xff, 0xfe]], json!({})).is_err());
    assert!(matches!(
        run(
            "create.text",
            vec![b"source".to_vec()],
            json!({"subject":"ignored upstream"})
        ),
        Err(BackendError::Unsupported(_))
    ));
    assert!(
        run(
            "create.markdown",
            vec![b"source".to_vec()],
            json!({"keywords":"ignored upstream"})
        )
        .is_err()
    );
    assert!(matches!(
        run("create.images", vec![png(8, 8)], json!({"font_size":30})),
        Err(BackendError::Unsupported(_))
    ));
    let tiny = Limits {
        max_output_bytes: 100,
        ..Limits::default()
    };
    assert!(matches!(
        run_with_limits("create.text", vec![b"source".to_vec()], json!({}), &tiny),
        Err(BackendError::Limit(_))
    ));
    assert!(
        run(
            "create.html_css",
            vec![b"<p>x</p>".to_vec()],
            json!({"fonts":[]})
        )
        .is_err()
    );
}

#[test]
fn minimal_office_import_is_bytes_only_and_keeps_original() {
    let original = docx(&[], WORD, false);
    let input = Input::new(original.clone());
    let limits = Limits::default();
    let inputs = [input];
    let result = execute(
        "office.to_pdf",
        &Context::new(&inputs, None, None, &limits),
        &json!({"format":"docx"}),
    )
    .unwrap();
    assert_eq!(result.data["mode"], "flow_no_system_fonts");
    let document = pdf_oxide::PdfDocument::from_bytes(result.artifacts[0].bytes.clone()).unwrap();
    assert!(document.extract_text(0).unwrap().contains("Hello source"));
    assert_eq!(inputs[0].bytes, original);
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("embedded fonts"))
    );
    assert!(matches!(
        run(
            "office.to_pdf",
            vec![original],
            json!({"format":"docx","mode":"native"}),
        ),
        Err(BackendError::Unsupported(_))
    ));
}

#[test]
fn office_export_and_import_parse_all_three_formats() {
    for (operation, format) in [
        ("office.from_pdf_docx", "docx"),
        ("office.from_pdf_pptx", "pptx"),
        ("office.from_pdf_xlsx", "xlsx"),
    ] {
        let exported = run(operation, vec![source_pdf(2)], json!({})).unwrap();
        assert_eq!(exported.data["effective_mode"], "layout");
        assert_eq!(exported.data["source_pages"], 2);
        let bytes = exported.artifacts[0].bytes.clone();
        let parsed = office_oxide::Document::from_reader(
            Cursor::new(bytes.clone()),
            office_oxide::DocumentFormat::from_extension(format).unwrap(),
        )
        .unwrap();
        if format == "xlsx" {
            // Layout export stores editable DrawingML text, not worksheet cell
            // values. office_oxide's cell-only Markdown view omits drawings.
            let sheets = &parsed.as_xlsx().unwrap().worksheets;
            assert_eq!(sheets.len(), 2);
            for (index, sheet) in sheets.iter().enumerate() {
                let expected = format!("Source page {}", index + 1);
                assert!(sheet.text_shapes.iter().any(|shape| shape.text == expected));
            }
            assert_eq!(
                exported.data["content_representation"],
                "positioned_drawing_text_and_images"
            );
        } else {
            assert!(parsed.to_markdown().contains("Source page 1"), "{format}");
            assert!(parsed.to_markdown().contains("Source page 2"), "{format}");
        }
        let imported = run("office.to_pdf", vec![bytes], json!({"format":format})).unwrap();
        let document =
            pdf_oxide::PdfDocument::from_bytes(imported.artifacts[0].bytes.clone()).unwrap();
        assert!(document.page_count().unwrap() > 0);
        let rendered_text = (0..document.page_count().unwrap())
            .map(|page| document.extract_text(page).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            rendered_text.contains("Source page 1"),
            "{format}: {rendered_text}"
        );
        assert!(
            rendered_text.contains("Source page 2"),
            "{format}: {rendered_text}"
        );
    }
}

#[test]
fn office_image_export_checks_metadata_before_decode_and_rejects_alpha_loss() {
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::ImageBuffer::from_pixel(
        16,
        16,
        image::Rgb([30, 80, 150]),
    ))
    .write_to(&mut encoded, image::ImageFormat::Png)
    .unwrap();
    let opaque_pdf = run("create.images", vec![encoded.into_inner()], json!({}))
        .unwrap()
        .artifacts
        .remove(0)
        .bytes;
    let masked_pdf = run("create.images", vec![png(16, 16)], json!({}))
        .unwrap()
        .artifacts
        .remove(0)
        .bytes;
    // The actual stream remains tiny. A failed dictionary bound must occur
    // before a decoder can try to allocate its declared ten billion pixels.
    let mut exaggerated = lopdf::Document::load_mem(&opaque_pdf).unwrap();
    for object in exaggerated.objects.values_mut() {
        if let Ok(stream) = object.as_stream_mut()
            && stream
                .dict
                .get(b"Subtype")
                .and_then(lopdf::Object::as_name)
                .ok()
                == Some(b"Image")
        {
            stream.dict.set("Width", 100_000_i64);
            stream.dict.set("Height", 100_000_i64);
        }
    }
    let mut oversized_pdf = Vec::new();
    exaggerated.save_to(&mut oversized_pdf).unwrap();
    for operation in [
        "office.from_pdf_docx",
        "office.from_pdf_pptx",
        "office.from_pdf_xlsx",
    ] {
        let output = run(operation, vec![opaque_pdf.clone()], json!({})).unwrap();
        assert_eq!(output.data["image_preflight"]["images_inspected"], 1);
        let mut archive = zip::ZipArchive::new(Cursor::new(&output.artifacts[0].bytes)).unwrap();
        let mut matching_images = 0;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).unwrap();
            if entry.name().contains("/media/") {
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut bytes).unwrap();
                let decoded = image::load_from_memory(&bytes).unwrap().into_rgb8();
                assert_eq!(decoded.dimensions(), (16, 16), "{operation}");
                assert!(
                    decoded.pixels().all(|pixel| pixel.0 == [30, 80, 150]),
                    "{operation}"
                );
                matching_images += 1;
            }
        }
        assert_eq!(matching_images, 1, "{operation}");
        assert!(
            matches!(
                run(operation, vec![oversized_pdf.clone()], json!({})),
                Err(BackendError::Limit(_))
            ),
            "{operation}"
        );
        assert!(
            matches!(
                run(operation, vec![masked_pdf.clone()], json!({})),
                Err(BackendError::Unsupported(_))
            ),
            "{operation}"
        );
    }
}

#[test]
fn office_layout_thresholds_are_explicit_at_30_31_and_200_201() {
    for (operation, count, expected) in [
        ("office.from_pdf_docx", 30, "layout"),
        ("office.from_pdf_docx", 31, "flow"),
        ("office.from_pdf_pptx", 30, "layout"),
        ("office.from_pdf_pptx", 31, "flow"),
        ("office.from_pdf_xlsx", 200, "layout"),
        ("office.from_pdf_xlsx", 201, "flow"),
    ] {
        let result = run(operation, vec![source_pdf(count)], json!({})).unwrap();
        assert_eq!(result.data["effective_mode"], expected);
        assert_eq!(result.data["source_pages"], count);
        assert!(result.artifacts[0].bytes.starts_with(b"PK\x03\x04"));
    }
}

#[test]
fn office_markdown_creation_is_real_archive_and_rejects_legacy_formats() {
    for format in ["docx", "xlsx", "pptx"] {
        let result = run(
            "office.create_markdown",
            vec![b"# Source heading\n\nSource paragraph".to_vec()],
            json!({"format":format}),
        )
        .unwrap();
        let document = office_oxide::Document::from_reader(
            Cursor::new(result.artifacts[0].bytes.clone()),
            office_oxide::DocumentFormat::from_extension(format).unwrap(),
        )
        .unwrap();
        assert!(document.to_markdown().contains("Source"));
    }
    for format in ["doc", "xls", "ppt", "docm", "xlsm", "pptm"] {
        assert!(matches!(
            run(
                "office.to_pdf",
                vec![b"synthetic".to_vec()],
                json!({"format":format})
            ),
            Err(BackendError::Unsupported(_))
        ));
    }
}

#[test]
fn office_rejects_macros_ole_external_links_and_xml_tricks() {
    let external = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="http://127.0.0.1:9/private" TargetMode="External"/></Relationships>"#;
    for bytes in [
        docx(&[("word/vbaProject.bin", b"synthetic macro")], WORD, false),
        docx(&[("word/_rels/document.xml.rels", external)], WORD, false),
        docx(
            &[],
            br#"<w:document xmlns:w="urn:word"><w:object/></w:document>"#,
            false,
        ),
        docx(
            &[],
            br#"<!DOCTYPE x [<!ENTITY a SYSTEM "file:///private">]><x>&a;</x>"#,
            false,
        ),
        docx(
            &[],
            br#"<w:document xmlns:w="urn:word"><w:p></w:r></w:document>"#,
            false,
        ),
        docx(&[], b"<x><y a='1' a='2'/></x>", false),
        docx(&[], b"<![CDATA[outside]]><x/>", false),
        docx(&[], b"&amp;<x/>", false),
        docx(&[], b"<x/><?xml version='1.0'?>", false),
        docx(&[], b"<x>\x01</x>", false),
        docx(&[("../escape.xml", b"<x/>")], WORD, false),
    ] {
        assert!(run("office.to_pdf", vec![bytes], json!({"format":"docx"})).is_err());
    }
}

#[test]
fn office_rejects_crc_damage_zip_duplicates_and_bounded_expansion() {
    let mut corrupt = docx(&[], WORD, false);
    let index = corrupt
        .windows(12)
        .position(|bytes| bytes == b"Hello source")
        .unwrap();
    corrupt[index] = b'J';
    let error = run("office.to_pdf", vec![corrupt], json!({"format":"docx"})).unwrap_err();
    assert!(error.to_string().contains("CRC"));

    let mut duplicate = docx(
        &[("word/a.xml", b"<x/>"), ("word/b.xml", b"<x/>")],
        WORD,
        false,
    );
    for index in 0..duplicate.len().saturating_sub(10) {
        if &duplicate[index..index + 10] == b"word/b.xml" {
            duplicate[index + 5] = b'a';
        }
    }
    let error = run("office.to_pdf", vec![duplicate], json!({"format":"docx"})).unwrap_err();
    assert!(error.to_string().contains("duplicate"));

    let mut large = WORD.to_vec();
    large.extend(std::iter::repeat_n(b' ', 70_000));
    let compressed = docx(&[], &large, true);
    assert!(compressed.len() < 65_536);
    let limits = Limits {
        max_input_bytes: 65_536,
        ..Limits::default()
    };
    assert!(matches!(
        run_with_limits(
            "office.to_pdf",
            vec![compressed],
            json!({"format":"docx"}),
            &limits
        ),
        Err(BackendError::Limit(_))
    ));
}

#[test]
fn office_whole_document_conversion_rejects_partial_selection_and_page_limits() {
    let inputs = [Input::new(source_pdf(2))];
    let limits = Limits::default();
    let context = Context::new(&inputs, None, Some((1, 1)), &limits);
    assert!(matches!(
        execute("office.from_pdf_docx", &context, &json!({})),
        Err(BackendError::Unsupported(_))
    ));
    let limits = Limits {
        max_pages: 1,
        ..Limits::default()
    };
    assert!(matches!(
        run_with_limits(
            "office.from_pdf_docx",
            vec![source_pdf(2)],
            json!({}),
            &limits
        ),
        Err(BackendError::Limit(_))
    ));
}
