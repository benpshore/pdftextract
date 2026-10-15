#![cfg(feature = "pdf-oxide")]

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use serde_json::{Value, json};
use tpe::backend::BackendError;
use tpe::oxide::{Context, Input, Limits, OperationOutput, execute};

fn document(contents: &[Vec<u8>]) -> Document {
    let mut document = Document::with_version("1.7");
    let tree = document.new_object_id();
    let font = document.add_object(dictionary! {
        "Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica", "Encoding"=>"WinAnsiEncoding",
    });
    let mut kids = Vec::new();
    for content in contents {
        let stream = document.add_object(Stream::new(dictionary! {}, content.clone()));
        let page = document.add_object(dictionary! {
            "Type"=>"Page", "Parent"=>tree, "Contents"=>stream,
            "MediaBox"=>vec![0.into(),0.into(),612.into(),792.into()],
            "Resources"=>dictionary! {"Font"=>dictionary! {"F1"=>font}},
        });
        kids.push(Object::Reference(page));
    }
    document.objects.insert(
        tree,
        dictionary! {"Type"=>"Pages", "Count"=>i64::try_from(kids.len()).unwrap(), "Kids"=>kids}
            .into(),
    );
    let root = document.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>tree});
    document.trailer.set("Root", root);
    document
}

fn save(mut document: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn text_content(text: &str, actual_text: Option<&str>) -> Vec<u8> {
    let mut operations = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["F1".into(), 12.into()]),
        Operation::new("Td", vec![72.into(), 700.into()]),
    ];
    if let Some(text) = actual_text {
        let mut bytes = vec![0xfe, 0xff];
        bytes.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
        operations.push(Operation::new(
            "BDC",
            vec![
                "Span".into(),
                dictionary! {"ActualText"=>Object::string_literal(bytes)}.into(),
            ],
        ));
    }
    operations.push(Operation::new("Tj", vec![Object::string_literal(text)]));
    if actual_text.is_some() {
        operations.push(Operation::new("EMC", vec![]));
    }
    operations.push(Operation::new("ET", vec![]));
    Content { operations }.encode().unwrap()
}

fn convert(
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

fn capture(name: &str, bytes: &[u8]) {
    if let Some(root) = std::env::var_os("TPE_OXIDE_FIXTURE_CAPTURE") {
        let root = std::path::PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(name), bytes).unwrap();
    }
}

#[test]
fn unicode_actual_text_and_literal_meaning_survive_all_three_converters() {
    let unicode = "Résumé λ 漢字 🦀";
    let mut content = text_content("placeholder", Some(unicode));
    content.extend_from_slice(b"\nBT /F1 12 Tf 72 650 Td (had had well-being [12]) Tj ET");
    let bytes = save(document(&[content]));
    capture("convert-unicode-citations.pdf", &bytes);
    let original = bytes.clone();
    for operation in ["convert.text", "convert.markdown", "convert.html"] {
        let result = convert(&bytes, operation, None, &Limits::default(), json!({})).unwrap();
        let page = &result.data["pages"][0];
        let (baseline, converted) = if operation == "convert.html" {
            ("baseline_raw_html", "raw_html")
        } else {
            ("baseline", "converted")
        };
        let text = page[converted].as_str().unwrap();
        for expected in [unicode, "had had", "well-being", "[12]"] {
            assert!(
                text.contains(expected),
                "{operation}: missing {expected} in {text}"
            );
        }
        assert_eq!(page[baseline], page[converted]);
        assert_eq!(page["page"], 1);
        assert_eq!(result.data["source"], "pdf-oxide/0.3.78");
        assert_eq!(
            result.data["source_sha256"],
            Input::new(bytes.clone()).sha256
        );
        assert_eq!(result.data["completeness"], "unverified");
        assert_eq!(result.data["effective_options"]["include_artifacts"], true);
        assert_eq!(
            result.data["effective_options"]["strip_running_headers_footers"],
            false
        );
        assert_eq!(result.data["effective_options"]["expand_ligatures"], false);
        assert!(!result.warnings.is_empty());
        assert!(result.artifacts.is_empty());
    }
    assert_eq!(bytes, original);
}

#[test]
fn changed_html_layout_retains_original_baseline_and_sanitizes_display() {
    let bytes = save(document(&[text_content(
        "<script>literal</script> & text",
        None,
    )]));
    capture("convert-html-literal-markup.pdf", &bytes);
    let default = convert(&bytes, "convert.html", None, &Limits::default(), json!({})).unwrap();
    let changed = convert(
        &bytes,
        "convert.html",
        None,
        &Limits::default(),
        json!({"preserve_layout":true,"detect_headings":false}),
    )
    .unwrap();
    let page = &changed.data["pages"][0];
    assert_eq!(
        page["baseline_raw_html"],
        default.data["pages"][0]["raw_html"]
    );
    assert_ne!(page["raw_html"], page["baseline_raw_html"]);
    assert_eq!(changed.data["options_changed"], true);
    assert_eq!(changed.data["baseline_options"]["preserve_layout"], false);
    assert_eq!(changed.data["effective_options"]["preserve_layout"], true);
    let page_id = format!("pdf-{}-page-1", Input::new(bytes.clone()).sha256);
    assert_eq!(page["page_id"], page_id);
    let display = page["display_html"].as_str().unwrap();
    assert!(display.starts_with(&format!("<section class=\"pdf-page\" id=\"{page_id}\">")));
    assert!(display.contains("literal") && display.contains("text"));
    for forbidden in [
        "<script",
        "<img",
        "<svg",
        "<iframe",
        "<form",
        " style=",
        " href=",
        " src=",
        " onerror=",
    ] {
        assert!(
            !display.to_ascii_lowercase().contains(forbidden),
            "{display}"
        );
    }
    assert_eq!(default.data["pages"][0]["page_id"], page["page_id"]);
    let fresh = convert(&bytes, "convert.html", None, &Limits::default(), json!({})).unwrap();
    assert_eq!(default.data["pages"], fresh.data["pages"]);
}

#[test]
fn selected_page_numbers_and_ranges_are_explicit_and_bounded() {
    let bytes = save(document(&[
        text_content("FIRST", None),
        text_content("SECOND", None),
        text_content("THIRD", None),
    ]));
    let result = convert(
        &bytes,
        "convert.text",
        Some((2, 2)),
        &Limits::default(),
        json!({"reading_order":"column_aware"}),
    )
    .unwrap();
    assert_eq!(result.data["source_page_count"], 3);
    let pages = result.data["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0]["page"], 2);
    assert!(pages[0]["converted"].as_str().unwrap().contains("SECOND"));
    assert!(!pages[0]["converted"].as_str().unwrap().contains("FIRST"));
    for range in [(0, 1), (3, 2), (4, 4), (1, u32::MAX)] {
        assert!(matches!(
            convert(
                &bytes,
                "convert.text",
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
        convert(&bytes, "convert.markdown", None, &limits, json!({})),
        Err(BackendError::Limit(_))
    ));
    let limits = Limits {
        max_items: 2,
        ..Limits::default()
    };
    assert!(matches!(
        convert(&bytes, "convert.text", None, &limits, json!({})),
        Err(BackendError::Limit(_))
    ));
}

#[test]
fn empty_and_damaged_inputs_do_not_become_claims_of_complete_extraction() {
    let bytes = save(document(&[Vec::new()]));
    for operation in ["convert.text", "convert.markdown", "convert.html"] {
        let result = convert(&bytes, operation, None, &Limits::default(), json!({})).unwrap();
        assert_eq!(result.data["completeness"], "unverified");
        assert!(!result.warnings.is_empty());
        if operation != "convert.html" {
            assert_eq!(result.data["pages"][0]["converted"], "");
            assert!(result.data["diagnostics"].as_array().unwrap().iter().any(
                |value| value["category"] == "no_text_result_unverified" && value["page"] == 1
            ));
        }
    }
    assert!(matches!(
        convert(
            b"not a PDF",
            "convert.text",
            None,
            &Limits::default(),
            json!({})
        ),
        Err(BackendError::Malformed(_))
    ));
    let mut damaged = document(&[text_content("lost", None)]);
    let page = *damaged.get_pages().get(&1).unwrap();
    let stream = damaged
        .get_object(page)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Contents")
        .unwrap()
        .as_reference()
        .unwrap();
    damaged.objects.insert(
        stream,
        Stream::new(
            dictionary! {"Filter"=>"FlateDecode"},
            b"bad deflate".to_vec(),
        )
        .into(),
    );
    let result = convert(
        &save(damaged),
        "convert.text",
        None,
        &Limits::default(),
        json!({}),
    );
    // The pinned parser may recover empty. Retain that observation as
    // unverified; never fabricate successful recovery of the lost text.
    if let Ok(result) = result {
        assert_eq!(result.data["completeness"], "unverified");
        assert_ne!(result.data["pages"][0]["converted"], "lost");
        assert!(!result.warnings.is_empty());
    }
}

#[test]
fn path_image_formula_and_silent_transform_parameters_are_rejected() {
    let bytes = save(document(&[text_content("Keep me", None)]));
    for params in [
        json!({"include_images":true}),
        json!({"image_output_dir":"/tmp/unrequested"}),
        json!({"page_images":["/etc/passwd"]}),
        json!({"render_formulas":true}),
        json!({"strip_running_headers_footers":true}),
        json!({"include_artifacts":false}),
        json!({"expand_ligatures":true}),
        json!({"reading_order":"invented"}),
        json!({"extract_tables":"yes"}),
        json!({"detect_headings":null}),
        json!({"preserve_layout":true}),
        json!(null),
    ] {
        assert!(matches!(
            convert(&bytes, "convert.text", None, &Limits::default(), params),
            Err(BackendError::Unsupported(_))
        ));
    }
    let result = convert(&bytes, "convert.markdown", None, &Limits::default(), json!({"bold_marker_behavior":"aggressive", "include_form_fields":false,"extract_tables":false,"reading_order":"top_to_bottom"})).unwrap();
    assert!(
        result.data["pages"][0]["converted"]
            .as_str()
            .unwrap()
            .contains("Keep me")
    );
    assert_eq!(
        result.data["effective_options"]["bold_marker_behavior"],
        "aggressive"
    );
    assert_eq!(
        result.data["effective_options"]["include_form_fields"],
        false
    );
    assert_eq!(result.data["baseline_options"]["include_form_fields"], true);
}

#[test]
fn image_resources_are_omitted_without_image_decode_or_output_files() {
    let mut document = document(&[text_content("Text beside image", None)]);
    let image = document.add_object(Stream::new(dictionary! {"Type"=>"XObject","Subtype"=>"Image","Width"=>8,"Height"=>8,"BitsPerComponent"=>8,"ColorSpace"=>"DeviceRGB","Filter"=>"DCTDecode"}, b"not a JPEG".to_vec()));
    let page = *document.get_pages().get(&1).unwrap();
    document
        .get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .get_mut(b"Resources")
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("XObject", dictionary! {"Image"=>image});
    let bytes = save(document);
    for operation in ["convert.text", "convert.markdown", "convert.html"] {
        let result = convert(&bytes, operation, None, &Limits::default(), json!({})).unwrap();
        assert_eq!(result.data["effective_options"]["include_images"], false);
        assert_eq!(result.data["effective_options"]["embed_images"], false);
        assert_eq!(
            result.data["effective_options"]["image_output_dir"],
            Value::Null
        );
        assert_eq!(result.data["effective_options"]["page_images"], Value::Null);
        assert!(result.artifacts.is_empty());
        let output = result.data["pages"][0].to_string();
        assert!(output.contains("Text beside image"));
        assert!(!output.contains("<img") && !output.contains("data:image"));
    }
}

#[test]
fn output_overflow_is_an_error_without_changing_input() {
    let bytes = save(document(&[text_content(&"quoted text ".repeat(100), None)]));
    let inputs = [Input::new(bytes.clone())];
    let limits = Limits {
        max_output_bytes: 128,
        ..Limits::default()
    };
    for operation in ["convert.text", "convert.markdown", "convert.html"] {
        assert!(matches!(
            execute(
                operation,
                &Context::new(&inputs, None, None, &limits),
                &json!({})
            ),
            Err(BackendError::Limit(_))
        ));
        assert_eq!(inputs[0].bytes, bytes);
    }
    let mut bad_hash = Input::new(bytes);
    bad_hash.sha256 = "\"><script>".into();
    assert!(
        execute(
            "convert.html",
            &Context::new(&[bad_hash], None, None, &Limits::default()),
            &json!({})
        )
        .is_err()
    );
}
