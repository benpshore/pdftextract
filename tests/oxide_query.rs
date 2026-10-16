#![cfg(feature = "pdf-oxide")]

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use serde_json::{Value, json};
use tpe::backend::BackendError;
use tpe::oxide::{Context, Input, Limits, OperationOutput, execute};

fn pdf(pages: &[&str], right_column: bool, actual_text: Option<&str>) -> Vec<u8> {
    let mut document = Document::with_version("1.7");
    let tree = document.new_object_id();
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let mut kids = Vec::new();
    for text in pages {
        let mut operations = Vec::new();
        if !text.is_empty() {
            operations.extend([
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![72.into(), 700.into()]),
            ]);
            if let Some(text) = actual_text {
                let mut bytes = vec![0xfe, 0xff];
                bytes.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
                operations.push(Operation::new(
                    "BDC",
                    vec![
                        "Span".into(),
                        dictionary! {"ActualText" => Object::string_literal(bytes)}.into(),
                    ],
                ));
            }
            operations.push(Operation::new("Tj", vec![Object::string_literal(*text)]));
            if actual_text.is_some() {
                operations.push(Operation::new("EMC", vec![]));
            }
            operations.push(Operation::new("ET", vec![]));
        }
        if right_column {
            operations.extend([
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![360.into(), 700.into()]),
                Operation::new("Tj", vec![Object::string_literal("RIGHT EDGE")]),
                Operation::new("ET", vec![]),
            ]);
        }
        // A real painted rectangle exercises vector extraction independently
        // of the text bounding boxes.
        if !text.is_empty() {
            operations.extend([
                Operation::new("re", vec![70.into(), 650.into(), 180.into(), 30.into()]),
                Operation::new("S", vec![]),
            ]);
        }
        let content = document.add_object(Stream::new(
            dictionary! {},
            Content { operations }.encode().unwrap(),
        ));
        let page = document.add_object(dictionary! {
            "Type" => "Page", "Parent" => tree, "Contents" => content,
            "Resources" => dictionary! {"Font" => dictionary! {"F1" => font}},
            "CropBox" => vec![20.into(),30.into(),600.into(),790.into()],
        });
        kids.push(Object::Reference(page));
    }
    document.objects.insert(
        tree,
        dictionary! {
            "Type" => "Pages", "Count" => i64::try_from(kids.len()).unwrap(), "Kids" => kids,
            "MediaBox" => vec![10.into(),20.into(),610.into(),800.into()],
        }
        .into(),
    );
    let root = document.add_object(dictionary! {"Type" => "Catalog", "Pages" => tree});
    let info = document.add_object(dictionary! {
        "Producer" => Object::string_literal("Synthetic query fixture"),
        "Creator" => Object::string_literal("Repository tests"),
    });
    document.trailer.set("Root", root);
    document.trailer.set("Info", info);
    save(document)
}

fn save(mut document: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn query(bytes: &[u8], operation: &str, params: Value) -> OperationOutput {
    with_limits(bytes, operation, None, &Limits::default(), params).unwrap()
}

// Test cases supply temporary JSON alongside their bounded query configuration.
#[allow(clippy::needless_pass_by_value)]
fn with_limits(
    bytes: &[u8],
    operation: &str,
    pages: Option<(u32, u32)>,
    limits: &Limits,
    params: Value,
) -> Result<OperationOutput, BackendError> {
    let inputs = [Input::new(bytes.to_vec())];
    execute(
        operation,
        &Context::new(&inputs, None, pages, limits),
        &params,
    )
}

fn first(output: &OperationOutput) -> &Value {
    &output.data["result"][0]["value"]
}

#[test]
fn document_geometry_raw_metrics_and_source_identity_survive_queries() {
    let bytes = pdf(&["Alpha beta", "Second page"], false, None);
    let original = bytes.clone();
    let metadata = query(&bytes, "query.document", json!({}));
    assert_eq!(metadata.data["result"]["page_count"], 2);
    assert_eq!(
        metadata.data["result"]["producer"],
        "Synthetic query fixture"
    );
    assert_eq!(
        metadata.data["source_sha256"],
        Input::new(bytes.clone()).sha256
    );
    let geometry = query(&bytes, "query.page", json!({}));
    assert_eq!(
        first(&geometry)["media_box_corners"],
        json!([10.0, 20.0, 610.0, 800.0])
    );
    assert_eq!(
        first(&geometry)["visible_box_corners"],
        json!([20.0, 30.0, 600.0, 790.0])
    );
    assert_eq!(first(&geometry)["width"], 600.0);
    assert_eq!(first(&geometry)["height"], 780.0);
    let chars = query(&bytes, "query.chars", json!({}));
    let character = &first(&chars)[0];
    assert_eq!(character["char"], "A");
    assert_eq!(character["font_name"], "Helvetica");
    for field in [
        "bbox",
        "font_size",
        "advance_width",
        "rendered_advance",
        "ascent",
        "descent",
        "matrix",
        "origin_x",
        "origin_y",
    ] {
        assert!(!character[field].is_null(), "missing {field}");
    }
    let spans = query(&bytes, "query.spans", json!({}));
    let span = &first(&spans)[0];
    assert_eq!(span["mirrored"], false);
    assert_eq!(span["page_rotation_applied"], 0);
    assert!(span["page_bbox"].is_object());
    for output in [metadata, geometry, chars, spans] {
        assert_eq!(output.data["completeness"], "unverified");
        assert!(
            output
                .warnings
                .iter()
                .any(|warning| warning.starts_with("extraction_incomplete:"))
        );
    }
    assert_eq!(bytes, original);
}

#[test]
fn actual_text_unicode_and_search_byte_offsets_are_preserved() {
    let text = "Résumé λ 漢字 🦀";
    let bytes = pdf(&["X"], false, Some(text));
    // Native char mode in pinned 0.3.78 re-maps decoded ActualText UTF-8
    // through WinAnsi. Preserve that raw evidence with a precise diagnostic;
    // the explicitly derived path must independently retain correct Unicode.
    let native = query(&bytes, "query.chars", json!({"representation":"native"}));
    let native_text: String = first(&native)
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value["char"].as_str().unwrap())
        .collect();
    assert_ne!(native_text, text);
    assert!(
        native.data["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning["category"] == "actual_text_character_mapping_unverified")
    );
    let chars = query(
        &bytes,
        "query.chars",
        json!({"representation":"span_derived"}),
    );
    let recovered: String = first(&chars)
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value["char"].as_str().unwrap())
        .collect();
    assert_eq!(recovered, text);
    let spans = query(&bytes, "query.spans", json!({"reading_order":"structure"}));
    let recovered: String = first(&spans)
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value["text"].as_str().unwrap())
        .collect();
    assert_eq!(recovered, text);
    let matches = query(&bytes, "query.search", json!({"pattern":"漢字"}));
    let found = &matches.data["result"]["matches"][0];
    assert_eq!(found["page"], 1);
    assert_eq!(found["match"]["page"], 0);
    assert_eq!(found["match"]["text"], "漢字");
    let start = usize::try_from(found["match"]["start_index"].as_u64().unwrap()).unwrap();
    let end = usize::try_from(found["match"]["end_index"].as_u64().unwrap()).unwrap();
    assert_eq!(&text[start..end], "漢字");
    assert_eq!(matches.data["result"]["truncated"], Value::Null);
    assert_eq!(matches.data["result"]["match_limit_reached"], false);
    assert_eq!(matches.data["status"], "partial");
}

#[test]
fn query_page_selection_is_one_based_and_rejects_unbounded_ranges() {
    let bytes = pdf(&["First page", "Second page"], false, None);
    let output = with_limits(
        &bytes,
        "query.text",
        Some((2, 2)),
        &Limits::default(),
        json!({}),
    )
    .unwrap();
    assert_eq!(output.data["result"].as_array().unwrap().len(), 1);
    assert_eq!(output.data["result"][0]["page"], 2);
    assert!(first(&output).as_str().unwrap().contains("Second page"));
    for range in [(0, 1), (2, 1), (1, 3), (1, u32::MAX)] {
        assert!(matches!(
            with_limits(
                &bytes,
                "query.chars",
                Some(range),
                &Limits::default(),
                json!({})
            ),
            Err(BackendError::PageRange { .. })
        ));
    }
    let limits = Limits {
        max_pages: 1,
        ..Limits::default()
    };
    assert!(matches!(
        with_limits(&bytes, "query.text", None, &limits, json!({})),
        Err(BackendError::Limit(_))
    ));
    assert!(
        with_limits(
            &bytes,
            "query.font_data",
            Some((1, 1)),
            &Limits::default(),
            json!({})
        )
        .is_err()
    );
}

#[test]
fn region_queries_use_pdf_coordinates_and_do_not_silently_ignore_options() {
    let bytes = pdf(&["LEFT CONTENT"], true, None);
    let params = json!({"rect":[50,640,250,100],"kind":"text","mode":"contained"});
    let text = query(&bytes, "query.region", params);
    let text = first(&text).as_str().unwrap();
    assert!(text.contains("LEFT CONTENT"));
    assert!(!text.contains("RIGHT EDGE"));
    for kind in ["chars", "spans", "words", "lines", "paths"] {
        let output = query(
            &bytes,
            "query.region",
            json!({"rect":[50,640,250,100],"kind":kind}),
        );
        assert!(
            !first(&output).as_array().unwrap().is_empty(),
            "region {kind}"
        );
    }
    for params in [
        json!({"rect":[0,0,-1,10]}),
        json!({"rect":[0,0,10,0]}),
        json!({"rect":[0,0,10,10],"mode":"overlap","overlap":0}),
        json!({"rect":[0,0,10,10],"kind":"paths","mode":"contained"}),
        json!({"rect":[0,0,10,10],"kind":"chars","excluded_layers":["x"]}),
    ] {
        assert!(with_limits(&bytes, "query.region", None, &Limits::default(), params).is_err());
    }
}

#[test]
fn search_respects_literal_regex_case_whole_word_and_fails_on_overflow() {
    let bytes = pdf(&["Alpha alphabet ALPHA a.b axb"], false, None);
    let search = query(
        &bytes,
        "query.search",
        json!({"pattern":"alpha","case_insensitive":true,"whole_word":true}),
    );
    assert_eq!(
        search.data["result"]["matches"].as_array().unwrap().len(),
        2
    );
    let literal = query(&bytes, "query.search", json!({"pattern":"a.b"}));
    assert_eq!(
        literal.data["result"]["matches"].as_array().unwrap().len(),
        1
    );
    let regex = query(
        &bytes,
        "query.search",
        json!({"pattern":"a.b","literal":false}),
    );
    assert_eq!(regex.data["result"]["matches"].as_array().unwrap().len(), 2);
    assert!(matches!(
        with_limits(
            &bytes,
            "query.search",
            None,
            &Limits::default(),
            json!({"pattern":"alpha","case_insensitive":true,"max_results":1})
        ),
        Err(BackendError::Limit(_))
    ));
    for params in [
        json!({"pattern":"[","literal":false}),
        json!({"pattern":""}),
        json!({"pattern":"x","max_results":0}),
    ] {
        assert!(with_limits(&bytes, "query.search", None, &Limits::default(), params).is_err());
    }
}

#[test]
fn real_words_lines_structure_paths_and_classification_are_exposed() {
    let bytes = pdf(&["Alpha beta"], false, None);
    let words = query(
        &bytes,
        "query.words",
        json!({"word_gap":2.0,"profile":"conservative"}),
    );
    let texts: Vec<_> = first(&words)
        .as_array()
        .unwrap()
        .iter()
        .map(|word| word["text"].as_str().unwrap())
        .collect();
    assert!(texts.iter().any(|word| word.contains("Alpha")));
    let lines = query(
        &bytes,
        "query.lines",
        json!({"word_gap":2.0,"line_gap":5.0}),
    );
    assert!(first(&lines)[0]["text"].as_str().unwrap().contains("Alpha"));
    let page = query(
        &bytes,
        "query.page_text",
        json!({"reading_order":"column_aware"}),
    );
    assert!(!first(&page)["chars"].as_array().unwrap().is_empty());
    assert_eq!(first(&page)["media_box_width"], 600.0);
    let structured = query(&bytes, "query.structured", json!({"columns":"single"}));
    assert!(!first(&structured)["regions"].as_array().unwrap().is_empty());
    let fonts = query(&bytes, "query.fonts", json!({}));
    assert_eq!(first(&fonts)["body_font_name"], "Helvetica");
    for kind in ["all", "rectangles", "lines"] {
        let paths = query(&bytes, "query.paths", json!({"kind":kind}));
        assert!(first(&paths).is_array());
        if kind != "lines" {
            assert!(!first(&paths).as_array().unwrap().is_empty());
        }
    }
    let classification = query(&bytes, "query.classify", json!({}));
    assert_eq!(first(&classification)["page"], 0);
    assert!(
        first(&classification)["signals"]["text_glyph_count"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(!first(&classification)["reason"].is_null());
    assert!(query(&bytes, "query.layers", json!({})).data["result"]["names"].is_array());
    assert!(first(&query(&bytes, "query.inks", json!({"deep":true}))).is_array());
}

#[test]
fn blank_and_damaged_pages_are_not_claimed_complete_and_limits_are_errors() {
    let blank = pdf(&[""], false, None);
    let text = query(&blank, "query.text", json!({}));
    assert!(first(&text).as_str().unwrap().trim().is_empty());
    assert_eq!(
        first(&query(&blank, "query.classify", json!({})))["kind"],
        "empty"
    );
    let mut damaged = Document::load_mem(&pdf(&["Visible text"], false, None)).unwrap();
    let content = damaged.add_object(Stream::new(
        dictionary! {"Filter"=>"ASCIIHexDecode"},
        b"GG>".to_vec(),
    ));
    let page = *damaged.get_pages().get(&1).unwrap();
    damaged
        .get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Contents", content);
    // The decoded-content preflight now surfaces the failed stream before
    // native character extraction can turn it into an empty successful value.
    assert!(matches!(
        with_limits(
            &save(damaged),
            "query.chars",
            None,
            &Limits::default(),
            json!({})
        ),
        Err(BackendError::Page { .. })
    ));
    assert!(
        with_limits(
            b"not a PDF",
            "query.text",
            None,
            &Limits::default(),
            json!({})
        )
        .is_err()
    );
    let bytes = pdf(&[&"A".repeat(300)], false, None);
    let byte_limit = Limits {
        max_output_bytes: 1024,
        ..Limits::default()
    };
    assert!(matches!(
        with_limits(&bytes, "query.chars", None, &byte_limit, json!({})),
        Err(BackendError::Limit(_))
    ));
    let item_limit = Limits {
        max_items: 8,
        ..Limits::default()
    };
    assert!(matches!(
        with_limits(&bytes, "query.chars", None, &item_limit, json!({})),
        Err(BackendError::Limit(_))
    ));
    assert!(
        with_limits(
            &bytes,
            "query.text",
            None,
            &Limits::default(),
            json!({"silently_ignored":true})
        )
        .is_err()
    );
}

fn with_outline(cycle: bool, destination_cycle: bool) -> Vec<u8> {
    let mut document = Document::load_mem(&pdf(&["First", "Second"], false, None)).unwrap();
    let page = *document.get_pages().get(&2).unwrap();
    let root = document.new_object_id();
    let item = document.new_object_id();
    let dest = document.new_object_id();
    document.objects.insert(
        dest,
        if destination_cycle {
            Object::Reference(dest)
        } else {
            Object::Array(vec![page.into(), "Fit".into()])
        },
    );
    let mut title = vec![0xfe, 0xff];
    title.extend("Résumé λ".encode_utf16().flat_map(u16::to_be_bytes));
    let mut dict = dictionary! {"Title"=>Object::string_literal(title),"Parent"=>root,"Dest"=>dest};
    if cycle {
        dict.set("Next", item);
    }
    document.objects.insert(item, dict.into());
    document.objects.insert(
        root,
        dictionary! {"Type"=>"Outlines","First"=>item,"Last"=>item,"Count"=>1}.into(),
    );
    let catalog = document
        .trailer
        .get(b"Root")
        .unwrap()
        .as_reference()
        .unwrap();
    document
        .get_object_mut(catalog)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Outlines", root);
    save(document)
}

#[test]
fn outlines_keep_unicode_and_destination_and_reject_cycles_before_upstream_walk() {
    let output = query(&with_outline(false, false), "query.outlines", json!({}));
    assert_eq!(output.data["result"][0]["title"], "Résumé λ");
    assert_eq!(output.data["result"][0]["destination"]["page"], 2);
    for bytes in [with_outline(true, false), with_outline(false, true)] {
        let error = with_limits(
            &bytes,
            "query.outlines",
            None,
            &Limits::default(),
            json!({}),
        )
        .unwrap_err();
        assert!(error.to_string().contains("cyclic"));
    }
}

#[test]
fn attachments_and_content_are_exact_bytes_with_app_owned_artifact_names() {
    let mut document = Document::load_mem(&pdf(&["Alpha beta"], false, None)).unwrap();
    let payload = b"synthetic attachment bytes\0\xff";
    let stream = document.add_object(Stream::new(
        dictionary! {"Type"=>"EmbeddedFile"},
        payload.to_vec(),
    ));
    let spec=document.add_object(dictionary! {"Type"=>"Filespec","F"=>Object::string_literal("../../escape.sh"),"EF"=>dictionary! {"F"=>stream}});
    let tree = document
        .add_object(dictionary! {"Names"=>vec![Object::string_literal("attachment"),spec.into()]});
    let catalog = document
        .trailer
        .get(b"Root")
        .unwrap()
        .as_reference()
        .unwrap();
    document
        .get_object_mut(catalog)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Names", dictionary! {"EmbeddedFiles"=>tree});
    let bytes = save(document);
    let output = query(&bytes, "query.attachments", json!({}));
    assert_eq!(output.artifacts.len(), 1);
    assert_eq!(output.artifacts[0].name, "attachment-000000.bin");
    assert_eq!(output.artifacts[0].bytes, payload);
    assert_eq!(output.data["result"][0]["original_name"], "../../escape.sh");
    let mut unicode_document = Document::load_mem(&bytes).unwrap();
    let unicode_name = "../../Résumé λ.sh";
    let mut encoded_name = vec![0xfe, 0xff];
    encoded_name.extend(unicode_name.encode_utf16().flat_map(u16::to_be_bytes));
    unicode_document
        .get_object_mut(spec)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("UF", Object::string_literal(encoded_name));
    let unicode = query(&save(unicode_document), "query.attachments", json!({}));
    assert_eq!(unicode.data["result"][0]["original_name"], unicode_name);
    assert_ne!(
        unicode.data["result"][0]["upstream_projected_name"],
        unicode_name
    );
    assert_eq!(unicode.artifacts[0].name, "attachment-000000.bin");
    assert_eq!(unicode.artifacts[0].bytes, payload);
    let content = query(&bytes, "query.content", json!({}));
    assert_eq!(content.artifacts[0].name, "page-000001-content.bin");
    assert!(String::from_utf8_lossy(&content.artifacts[0].bytes).contains("Alpha beta"));
    let fonts = query(&bytes, "query.font_data", json!({}));
    assert!(fonts.artifacts.is_empty()); // Standard 14 font is not embedded.
    assert_eq!(fonts.data["completeness"], "unverified");
}

#[test]
fn cyclic_attachment_name_tree_is_rejected_without_entering_recursive_decoder() {
    let mut document = Document::load_mem(&pdf(&["x"], false, None)).unwrap();
    let tree = document.new_object_id();
    document.objects.insert(
        tree,
        dictionary! {"Kids"=>vec![Object::Reference(tree)]}.into(),
    );
    let catalog = document
        .trailer
        .get(b"Root")
        .unwrap()
        .as_reference()
        .unwrap();
    document
        .get_object_mut(catalog)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Names", dictionary! {"EmbeddedFiles"=>tree});
    let error = with_limits(
        &save(document),
        "query.attachments",
        None,
        &Limits::default(),
        json!({}),
    )
    .unwrap_err();
    assert!(error.to_string().contains("cyclic"));
}
