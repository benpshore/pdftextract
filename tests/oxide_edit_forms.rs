#![cfg(feature = "pdf-oxide")]

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use pdf_oxide::PdfDocument;
use pdf_oxide::editor::{DocumentEditor, EditableDocument, FormFieldValue};
use serde_json::{Value, json};
use tpe::backend::BackendError;
use tpe::oxide::{Context, Input, Limits, OperationOutput, execute};

fn fixture(prefix: &str, count: u32) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let tree = doc.new_object_id();
    let font = doc.add_object(
        dictionary! {"Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica"},
    );
    let mut kids = Vec::new();
    for number in 1..=count {
        let text = format!("{prefix} page {number}");
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![72.into(), 700.into()]),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        }
        .encode()
        .unwrap();
        let content = doc.add_object(Stream::new(dictionary! {}, content));
        let page = doc.add_object(dictionary! {"Type" => "Page", "Parent" => tree,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => dictionary! {"Font" => dictionary! {"F1" => font}},
        "Contents" => content});
        kids.push(page.into());
    }
    doc.objects.insert(
        tree,
        dictionary! {"Type" => "Pages", "Kids" => kids, "Count" => count}.into(),
    );
    let root = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => tree});
    doc.trailer.set("Root", root);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "tests pass owned JSON requests and source snapshots through one consistent helper"
)]
fn run(
    operation: &str,
    sources: Vec<Vec<u8>>,
    params: Value,
) -> Result<OperationOutput, BackendError> {
    run_with_pages(operation, sources, None, params)
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "tests pass owned JSON requests and source snapshots through one consistent helper"
)]
fn run_with_pages(
    operation: &str,
    sources: Vec<Vec<u8>>,
    pages: Option<(u32, u32)>,
    params: Value,
) -> Result<OperationOutput, BackendError> {
    let inputs = sources.into_iter().map(Input::new).collect::<Vec<_>>();
    let snapshots = inputs.iter().map(|i| i.bytes.clone()).collect::<Vec<_>>();
    let limits = Limits::default();
    let result = execute(
        operation,
        &Context::new(&inputs, None, pages, &limits),
        &params,
    );
    for (input, original) in inputs.iter().zip(snapshots) {
        assert_eq!(
            input.bytes, original,
            "operation mutated an immutable source"
        );
    }
    result
}

fn one_pdf(output: OperationOutput) -> Vec<u8> {
    assert_eq!(output.artifacts.len(), 1);
    assert_eq!(output.artifacts[0].mime, "application/pdf");
    output.artifacts.into_iter().next().unwrap().bytes
}

fn capture_form_fixture(name: &str, bytes: &[u8]) {
    // Only fixed synthetic fixture names from this file are passed here. The
    // integration owner supplies the optional disposable evidence directory.
    if let Some(root) = std::env::var_os("TPE_OXIDE_FIXTURE_CAPTURE") {
        let root = std::path::PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(name), bytes).unwrap();
    }
}

#[test]
fn page_reorder_duplicates_rotation_and_crop_survive_serialization() {
    let output = run(
        "edit.pages",
        vec![fixture("Original", 3)],
        json!({
            "select": [3, 1, 3], "rotations": [{"page": 3, "degrees": 90}],
            "boxes": [{"page": 3, "crop": [20.0, 30.0, 500.0, 760.0]}]
        }),
    )
    .unwrap();
    let bytes = one_pdf(output);
    let doc = PdfDocument::from_bytes(bytes).unwrap();
    assert_eq!(doc.page_count().unwrap(), 3);
    for (index, original) in [(0, 3), (1, 1), (2, 3)] {
        assert!(
            doc.extract_text(index)
                .unwrap()
                .contains(&format!("Original page {original}"))
        );
    }
    assert_eq!(doc.get_page_rotation(0).unwrap(), 90);
    assert_eq!(doc.get_page_rotation(1).unwrap(), 0);
    assert_eq!(
        doc.get_page_visible_box(0).unwrap(),
        (20.0, 30.0, 500.0, 760.0)
    );
}

#[test]
fn merge_and_split_preserve_requested_text_pages_and_range_bounds() {
    let merged = one_pdf(
        run(
            "edit.merge",
            vec![fixture("First", 2), fixture("Second", 1)],
            json!({}),
        )
        .unwrap(),
    );
    let doc = PdfDocument::from_bytes(merged.clone()).unwrap();
    assert_eq!(doc.page_count().unwrap(), 3);
    assert!(doc.extract_text(2).unwrap().contains("Second page 1"));
    let split = run(
        "edit.split",
        vec![merged],
        json!({"ranges": [[2, 3], [1, 1]]}),
    )
    .unwrap();
    assert_eq!(split.artifacts.len(), 2);
    let first = PdfDocument::from_bytes(split.artifacts[0].bytes.clone()).unwrap();
    assert_eq!(first.page_count().unwrap(), 2);
    assert!(first.extract_text(0).unwrap().contains("First page 2"));
    assert!(first.extract_text(1).unwrap().contains("Second page 1"));
    assert!(matches!(
        run(
            "edit.split",
            vec![fixture("Bounds", 1)],
            json!({"ranges": [[1, u32::MAX]]})
        ),
        Err(BackendError::PageRange { .. })
    ));
}

#[test]
fn edit_rejects_invalid_pages_rectangles_rotations_and_unknown_parameters() {
    let source = fixture("Bounds", 2);
    for params in [
        json!({"select": [0]}),
        json!({"select": [3]}),
        json!({"select": []}),
        json!({"rotations": [{"page": 1, "degrees": 45}]}),
        json!({"boxes": [{"page": 1, "crop": [100, 0, 20, 400]}]}),
        json!({"unexpected_path": "/tmp/should-not-be-read"}),
    ] {
        assert!(run("edit.pages", vec![source.clone()], params).is_err());
    }
    assert!(run("edit.pages", vec![b"not a PDF".to_vec()], json!({})).is_err());
}

#[test]
fn metadata_roundtrip_is_explicit_about_unicode_and_xmp_limitations() {
    let output = run(
        "edit.metadata",
        vec![fixture("Info", 1)],
        json!({"fields": {
            "title": "Verified title", "author": "Synthetic author", "subject": null
        }}),
    )
    .unwrap();
    assert!(output.warnings.iter().any(|w| w.contains("XMP")));
    let mut editor = DocumentEditor::from_bytes(one_pdf(output)).unwrap();
    let info = editor.get_info().unwrap();
    assert_eq!(info.title.as_deref(), Some("Verified title"));
    assert_eq!(info.author.as_deref(), Some("Synthetic author"));
    assert_eq!(info.subject, None);
    assert!(matches!(
        run(
            "edit.metadata",
            vec![fixture("Info", 1)],
            json!({"fields": {"title": "科学"}})
        ),
        Err(BackendError::Unsupported(_))
    ));
}

#[test]
fn annotation_add_update_remove_and_unsafe_action_rejection() {
    let source = fixture("Notes", 1);
    let added = one_pdf(run("edit.annotations", vec![source.clone()], json!({"action":"edit", "changes":[
        {"action":"add", "page":1, "annotation":{"kind":"text", "rect":[72,650,20,20], "contents":"Original note"}},
        {"action":"add", "page":1, "annotation":{"kind":"highlight", "rect":[72,690,100,15]}}
    ]})).unwrap());
    let read = run(
        "edit.annotations",
        vec![added.clone()],
        json!({"action":"inspect"}),
    )
    .unwrap();
    assert_eq!(read.data["annotations"].as_array().unwrap().len(), 2);
    assert_eq!(read.data["annotations"][0]["contents"], "Original note");
    let updated = one_pdf(run("edit.annotations", vec![added], json!({"action":"edit", "changes":[
        {"action":"update", "page":1, "index":0, "contents":"Changed note", "color":[0.0,0.5,1.0]},
        {"action":"remove", "page":1, "index":1}
    ]})).unwrap());
    let read = run(
        "edit.annotations",
        vec![updated],
        json!({"action":"inspect"}),
    )
    .unwrap();
    assert_eq!(read.data["annotations"].as_array().unwrap().len(), 1);
    assert_eq!(read.data["annotations"][0]["contents"], "Changed note");
    assert!(run("edit.annotations", vec![source], json!({"action":"edit", "changes":[
        {"action":"add", "page":1, "annotation":{"kind":"link_uri", "rect":[72,600,100,20], "uri":"javascript:alert(1)"}}
    ]})).is_err());
}

#[test]
fn embedded_bytes_roundtrip_without_document_chosen_output_paths() {
    let source = fixture("Attachment", 1);
    let payload = b"approved synthetic fixture\n".to_vec();
    let attached = one_pdf(
        run(
            "edit.attachments",
            vec![source.clone(), payload.clone()],
            json!({
                "action":"add", "files":[{"input":1,"name":"fixture.txt"}]
            }),
        )
        .unwrap(),
    );
    let extracted = run(
        "edit.attachments",
        vec![attached.clone()],
        json!({"action":"extract"}),
    )
    .unwrap();
    assert_eq!(extracted.artifacts[0].name, "attachment-0001.bin");
    assert_eq!(extracted.artifacts[0].bytes, payload);
    assert_eq!(extracted.data["files"][0]["source_name"], "fixture.txt");
    assert!(
        run(
            "edit.attachments",
            vec![source, b"fixture".to_vec()],
            json!({
                "action":"add", "files":[{"input":1,"name":"../escape.txt"}]
            })
        )
        .is_err()
    );
    assert!(matches!(
        run(
            "edit.attachments",
            vec![attached, b"fixture".to_vec()],
            json!({
                "action":"add", "files":[{"input":1,"name":"second.txt"}]
            })
        ),
        Err(BackendError::Unsupported(_))
    ));
}

fn form_fixture() -> Vec<u8> {
    one_pdf(run("forms.add", vec![fixture("Form", 1)], json!({"fields":[
        {"name":"name", "page":1, "rect":[72,620,200,20], "widget":{"kind":"text","value":"Before","max_length":64}},
        {"name":"agree", "page":1, "rect":[72,580,20,20], "widget":{"kind":"checkbox","checked":false}}
    ]})).unwrap())
}

#[test]
fn form_add_fill_inspect_update_remove_roundtrips_values_and_flags() {
    let source = form_fixture();
    let filled = one_pdf(
        run(
            "forms.fill",
            vec![source.clone()],
            json!({"fields":{
                "name":{"kind":"text","value":"After (test) <tag>&"},
                "agree":{"kind":"boolean","value":true}
            }}),
        )
        .unwrap(),
    );
    let inspection = run("forms.inspect", vec![filled.clone()], json!({})).unwrap();
    let fields = inspection.data["fields"].as_array().unwrap();
    assert!(
        fields
            .iter()
            .any(|f| f["name"] == "name" && f["value"]["value"] == "After (test) <tag>&")
    );
    assert!(
        fields
            .iter()
            .any(|f| f["name"] == "agree" && f["value"]["value"] == true)
    );
    let updated = one_pdf(run("forms.update", vec![filled], json!({"fields":[
        {"name":"name", "readonly":true,"required":true,"tooltip":"Review field", "alignment":1}
    ]})).unwrap());
    let inspection = run("forms.inspect", vec![updated.clone()], json!({})).unwrap();
    let name = inspection.data["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "name")
        .unwrap();
    assert_eq!(name["readonly"], true);
    assert_eq!(name["required"], true);
    assert_eq!(name["tooltip"], "Review field");
    assert!(
        run(
            "forms.fill",
            vec![updated.clone()],
            json!({"fields":{"name":{"kind":"text","value":"Blocked"}}})
        )
        .is_err()
    );
    let removed = one_pdf(run("forms.remove", vec![updated], json!({"names":["agree"]})).unwrap());
    let inspection = run("forms.inspect", vec![removed], json!({})).unwrap();
    assert_eq!(inspection.data["fields"].as_array().unwrap().len(), 1);
    for params in [
        json!({"fields":{"missing":{"kind":"text","value":"x"}}}),
        json!({"fields":{"name":{"kind":"boolean","value":true}}}),
        json!({"fields":{"name":{"kind":"text","value":"x".repeat(65)}}}),
    ] {
        assert!(run("forms.fill", vec![source.clone()], params).is_err());
    }
}

#[test]
fn form_export_escapes_fdf_and_xml_and_respects_output_bounds() {
    let filled = one_pdf(
        run(
            "forms.fill",
            vec![form_fixture()],
            json!({"fields":{
                "name":{"kind":"text","value":"A (B) <C>&"}
            }}),
        )
        .unwrap(),
    );
    let fdf = run(
        "forms.export",
        vec![filled.clone()],
        json!({"format":"fdf"}),
    )
    .unwrap();
    let fdf = String::from_utf8_lossy(&fdf.artifacts[0].bytes);
    assert!(fdf.starts_with("%FDF-1.2"));
    assert!(fdf.contains("A \\(B\\) <C>&"));
    let xfdf = run(
        "forms.export",
        vec![filled.clone()],
        json!({"format":"xfdf"}),
    )
    .unwrap();
    let xfdf = String::from_utf8(xfdf.artifacts[0].bytes.clone()).unwrap();
    assert!(xfdf.contains("A (B) &lt;C&gt;&amp;"));
    assert!(!xfdf.contains("<C>"));
    let inputs = vec![Input::new(filled)];
    let limits = Limits {
        max_output_bytes: 16,
        ..Default::default()
    };
    assert!(matches!(
        execute(
            "forms.export",
            &Context::new(&inputs, None, None, &limits),
            &json!({"format":"xfdf"})
        ),
        Err(BackendError::Limit(_))
    ));
}

#[test]
fn pinned_no_ap_form_regeneration_is_preserved_as_unqualified_evidence() {
    let source = form_fixture();
    capture_form_fixture("forms-no-ap-regeneration-source.pdf", &source);
    let mut native = DocumentEditor::from_bytes(source.clone()).unwrap();
    native.flatten_forms().unwrap();
    let flattened = native.save_to_bytes().unwrap();
    capture_form_fixture("forms-no-ap-regeneration-upstream.pdf", &flattened);
    // Retain the previous success assertions as direct upstream evidence. They
    // establish field removal and boilerplate survival, not value/style fidelity.
    let inspection = run("forms.inspect", vec![flattened.clone()], json!({})).unwrap();
    assert!(inspection.data["fields"].as_array().unwrap().is_empty());
    let parsed = Document::load_mem(&flattened).unwrap();
    let painted_text = invoked_form_streams(&parsed, 1)
        .iter()
        .flat_map(|bytes| {
            Content::decode(bytes)
                .unwrap()
                .operations
                .into_iter()
                .filter(|operation| operation.operator == "Tj")
                .map(|operation| operation.operands[0].as_str().unwrap().to_vec())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        painted_text,
        vec![b"Before".to_vec()],
        "native no-AP regeneration really paints this simple text value; general style preservation remains unqualified"
    );
    let doc = PdfDocument::from_bytes(flattened).unwrap();
    assert!(doc.extract_text(0).unwrap().contains("Form page 1"));
    assert!(matches!(run("forms.flatten", vec![source], json!({})),
        Err(BackendError::Unsupported(reason)) if reason.contains("normal /AP") && reason.contains("not qualified")));
}

fn invoked_form_streams(document: &Document, page_number: u32) -> Vec<Vec<u8>> {
    let page_id = document.get_pages()[&page_number];
    let page = document.get_dictionary(page_id).unwrap();
    let resources = document.get_dict_in_dict(page, b"Resources").unwrap();
    let xobjects = document.get_dict_in_dict(resources, b"XObject").unwrap();
    Content::decode(&document.get_page_content(page_id))
        .unwrap()
        .operations
        .iter()
        .filter(|operation| operation.operator == "Do")
        .map(|operation| {
            let name = operation.operands[0].as_name().unwrap();
            let stream = document
                .dereference(xobjects.get(name).unwrap())
                .unwrap()
                .1
                .as_stream()
                .unwrap();
            stream
                .decompressed_content()
                .unwrap_or_else(|_| stream.content.clone())
        })
        .collect()
}

fn xfa_fixture(xml: &str) -> Vec<u8> {
    let mut doc = Document::load_mem(&fixture("XFA source content", 1)).unwrap();
    let xfa = doc.add_object(Stream::new(dictionary! {}, xml.as_bytes().to_vec()));
    let acroform = doc.add_object(dictionary! {"XFA"=>xfa, "Fields"=>Vec::<Object>::new()});
    let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    doc.get_object_mut(root)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("AcroForm", acroform);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn static_xfa_is_an_explicit_derivative_and_dynamic_or_external_xml_fails() {
    let xml = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.3/"><subform name="form"><field name="name"><ui><textEdit/></ui><value><text>Fixture value</text></value></field></subform></template>"#;
    let source = xfa_fixture(xml);
    let inspection = run(
        "forms.xfa",
        vec![source.clone()],
        json!({"action":"inspect"}),
    )
    .unwrap();
    assert_eq!(inspection.data["has_xfa"], true);
    assert_eq!(inspection.data["fields"].as_array().unwrap().len(), 1);
    assert!(
        run(
            "forms.xfa",
            vec![source.clone()],
            json!({"action":"convert_static","accept_layout_loss":false})
        )
        .is_err()
    );
    let converted = run(
        "forms.xfa",
        vec![source],
        json!({"action":"convert_static","accept_layout_loss":true}),
    )
    .unwrap();
    assert!(
        converted
            .warnings
            .iter()
            .any(|w| w.contains("original visual layout"))
    );
    let inspection = run("forms.inspect", vec![one_pdf(converted)], json!({})).unwrap();
    assert_eq!(inspection.data["fields"].as_array().unwrap().len(), 1);
    for hostile in [
        "<!DOCTYPE template SYSTEM 'https://invalid.example/synthetic.dtd'><template/>",
        "<template><script>synthetic script</script></template>",
        "<template><image href='file:///synthetic-missing'/></template>",
    ] {
        assert!(matches!(
            run(
                "forms.xfa",
                vec![xfa_fixture(hostile)],
                json!({"action":"inspect"})
            ),
            Err(BackendError::Unsupported(_))
        ));
    }
}

#[test]
fn page_and_item_budgets_fail_before_expanding_large_requests() {
    let inputs = vec![Input::new(fixture("Budget", 3))];
    let limits = Limits {
        max_pages: 2,
        ..Default::default()
    };
    assert!(matches!(
        execute(
            "edit.pages",
            &Context::new(&inputs, None, None, &limits),
            &json!({"select":[1]})
        ),
        Err(BackendError::Limit(_))
    ));
    let inputs = vec![Input::new(fixture("Budget", 1))];
    let limits = Limits {
        max_items: 1,
        ..Default::default()
    };
    assert!(matches!(
        execute(
            "edit.split",
            &Context::new(&inputs, None, None, &limits),
            &json!({"ranges":[[1,1],[1,1]]})
        ),
        Err(BackendError::Limit(_))
    ));
}

#[test]
fn text_overlays_roundtrip_and_visually_defective_replacement_is_rejected() {
    let source = fixture("Original", 1);
    let output = run(
        "edit.content",
        vec![source.clone()],
        json!({"changes":[
            {"action":"add_text","page":1,"rect":[72,620,140,20],"text":"Added overlay","size":12}
        ]}),
    )
    .unwrap();
    let doc = PdfDocument::from_bytes(one_pdf(output)).unwrap();
    let text = doc.extract_text(0).unwrap();
    assert!(text.contains("Added overlay"), "{text}");
    assert!(text.contains("Original"), "{text}");
    assert!(matches!(
        run("edit.content", vec![source.clone()], json!({"changes":[
            {"action":"replace_text","page":1,"find":"Original","replacement":"Changed",
                "expected_matches":1,"accept_region_rewrite":true}
        ]})),
        Err(BackendError::Unsupported(message)) if message.contains("black redaction masks")
    ));
    assert!(
        run(
            "edit.content",
            vec![source],
            json!({"changes":[
                {"action":"replace_text","page":1,"find":"Original","replacement":"Changed",
                    "expected_matches":2,"accept_region_rewrite":true}
            ]})
        )
        .is_err()
    );
}

#[test]
fn pinned_dom_replacement_retains_black_mask_regression_evidence() {
    // Deliberately exercise the pinned upstream API directly, outside the
    // adapter's Unsupported guard. These extracted-text assertions used to
    // pass while the visual output was wrong; keep that evidence explicit.
    let source = fixture("Original", 1);
    let original = Document::load_mem(&source).unwrap();
    let original_page = original.get_pages()[&1];
    let original_ops = Content::decode(&original.get_page_content(original_page)).unwrap();
    assert!(!original_ops.operations.iter().any(|op| op.operator == "re"));
    let mut editor = DocumentEditor::from_bytes(source).unwrap();
    let mut page = editor.get_page(0).unwrap();
    let matches = page.find_text_containing("Original");
    assert_eq!(matches.len(), 1);
    for text in matches {
        page.set_text(text.id(), text.text().replace("Original", "Changed"))
            .unwrap();
    }
    editor.save_page(page).unwrap();
    let bytes = editor.save_to_bytes().unwrap();
    let parsed = PdfDocument::from_bytes(bytes.clone()).unwrap();
    let text = parsed.extract_text(0).unwrap();
    assert!(text.contains("Changed"), "{text}");
    assert!(!text.contains("Original"), "{text}");
    let output = Document::load_mem(&bytes).unwrap();
    let ops = Content::decode(&output.get_page_content(output.get_pages()[&1])).unwrap();
    assert!(
        ops.operations.windows(3).any(|window| {
            window[0].operator == "rg"
                && window[0].operands.len() == 3
                && window[0]
                    .operands
                    .iter()
                    .all(|value| value.as_float().unwrap() == 0.0)
                && window[1].operator == "re"
                && window[2].operator == "f"
        }),
        "pinned upstream black-mask evidence changed; visually requalify before enabling replacement"
    );
}

#[test]
fn visual_whiteout_explicitly_retains_recoverable_source_content() {
    let source = fixture("Synthetic visible content", 1);
    assert!(
        run(
            "edit.whiteout",
            vec![source.clone()],
            json!({"regions":[
        {"page":1,"rect":[60,680,500,740]}
    ], "acknowledge_text_remains":false})
        )
        .is_err()
    );
    let output = run(
        "edit.whiteout",
        vec![source],
        json!({"regions":[
        {"page":1,"rect":[60,680,500,740]}
    ], "acknowledge_text_remains":true}),
    )
    .unwrap();
    assert_eq!(output.data["underlying_content_retained"], true);
    let doc = PdfDocument::from_bytes(one_pdf(output)).unwrap();
    assert!(
        doc.extract_text(0)
            .unwrap()
            .contains("Synthetic visible content")
    );
}

fn image_fixture() -> Vec<u8> {
    let mut doc = Document::load_mem(&fixture("Image", 1)).unwrap();
    let image = doc.add_object(Stream::new(
        dictionary! {
            "Type"=>"XObject", "Subtype"=>"Image", "Width"=>1, "Height"=>1,
            "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>8,
        },
        vec![255, 0, 0],
    ));
    let image_content = doc.add_object(Stream::new(
        dictionary! {},
        b"q 40 0 0 30 100 200 cm /Im1 Do Q\n".to_vec(),
    ));
    let page = *doc.get_pages().get(&1).unwrap();
    let page = doc.get_object_mut(page).unwrap().as_dict_mut().unwrap();
    page.get_mut(b"Resources")
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("XObject", dictionary! {"Im1"=>image});
    let original = page.get(b"Contents").unwrap().clone();
    page.set("Contents", vec![original, image_content.into()]);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "exact PDF constants must survive this geometry round-trip without rounding"
)]
fn direct_image_occurrence_bounds_roundtrip_and_unknown_resource_fails() {
    let source = image_fixture();
    let output = run(
        "edit.image_bounds",
        vec![source.clone()],
        json!({"changes":[
            {"page":1,"name":"Im1","rect":[180,260,80,60]}
        ]}),
    )
    .unwrap();
    let mut editor = DocumentEditor::from_bytes(one_pdf(output)).unwrap();
    let images = editor.get_page_images(0).unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].bounds, [180.0, 260.0, 80.0, 60.0]);
    assert!(
        run(
            "edit.image_bounds",
            vec![source],
            json!({"changes":[
                {"page":1,"name":"Absent","rect":[180,260,80,60]}
            ]})
        )
        .is_err()
    );
}

#[test]
fn cyclic_bookmarks_fail_without_entering_the_upstream_unbounded_walk() {
    let mut doc = Document::load_mem(&fixture("Outline", 1)).unwrap();
    let outline = doc.new_object_id();
    let item = doc.new_object_id();
    doc.objects.insert(
        outline,
        dictionary! {"Type"=>"Outlines", "First"=>item,"Last"=>item}.into(),
    );
    doc.objects.insert(item, dictionary! {"Title"=>Object::string_literal("Synthetic cycle"),"Next"=>item,"Parent"=>outline}.into());
    let catalog = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    doc.get_object_mut(catalog)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Outlines", outline);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    assert!(matches!(
        run("edit.bookmarks", vec![bytes], json!({"action":"inspect"})),
        Err(BackendError::Malformed(_))
    ));
}

#[test]
fn choice_fields_keep_multi_values_and_lossy_xfdf_export_is_rejected() {
    let source = one_pdf(run("forms.add", vec![fixture("Choices",1)], json!({"fields":[
        {"name":"choice","page":1,"rect":[72,600,140,20],"widget":{"kind":"combo","options":["red","blue"],"value":"red"}},
        {"name":"list","page":1,"rect":[72,500,140,80],"widget":{"kind":"list","options":["a","b","c"],"values":["a","c"],"multi_select":true}},
        {"name":"inert_button","page":1,"rect":[72,450,100,20],"widget":{"kind":"push_button","caption":"No action"}},
        {"name":"empty_signature","page":1,"rect":[72,400,150,25],"widget":{"kind":"signature"}}
    ]})).unwrap());
    let inspection = run("forms.inspect", vec![source.clone()], json!({})).unwrap();
    let values = inspection.data["fields"].as_array().unwrap();
    let list = values.iter().find(|v| v["name"] == "list").unwrap();
    assert_eq!(list["value"]["value"], json!(["a", "c"]));
    let fdf = run(
        "forms.export",
        vec![source.clone()],
        json!({"format":"fdf"}),
    )
    .unwrap();
    assert!(String::from_utf8_lossy(&fdf.artifacts[0].bytes).contains("[ (a) (c) ]"));
    assert!(matches!(
        run(
            "forms.export",
            vec![source.clone()],
            json!({"format":"xfdf"})
        ),
        Err(BackendError::Unsupported(_))
    ));
    assert!(
        run(
            "forms.fill",
            vec![source],
            json!({"fields":{"empty_signature":{"kind":"text","value":"not a signature"}}})
        )
        .is_err()
    );
}

#[test]
fn hierarchical_form_names_are_not_silently_flattened_or_renamed() {
    let source = one_pdf(run("forms.add", vec![fixture("Hierarchy",1)],json!({"fields":[
        {"name":"person.name","page":1,"rect":[72,600,140,20],"widget":{"kind":"text","value":"Fixture"}}
    ]})).unwrap());
    let inspection = run("forms.inspect", vec![source.clone()], json!({})).unwrap();
    assert!(
        inspection.data["fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["name"] == "person.name")
    );
    assert!(
        run(
            "forms.add",
            vec![source],
            json!({"fields":[
                {"name":"person.name","page":1,"rect":[72,550,140,20],"widget":{"kind":"text"}}
            ]})
        )
        .is_err()
    );
}

#[test]
fn annotation_repair_keeps_unknown_properties_and_ordered_action_identity() {
    let bytes = one_pdf(run("edit.annotations", vec![fixture("Notes", 1)], json!({"action":"edit","changes":[
        {"action":"add","page":1,"annotation":{"kind":"text","rect":[72,650,20,20],"contents":"Keep identity"}}
    ]})).unwrap());
    let mut source = Document::load_mem(&bytes).unwrap();
    let page_id = source.get_pages()[&1];
    let annotation_id = source
        .get_object(page_id)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .as_reference()
        .unwrap();
    source
        .get_object_mut(annotation_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set(
            "SyntheticPreservedKey",
            Object::string_literal("preserve this"),
        );
    let mut bytes = Vec::new();
    source.save_to(&mut bytes).unwrap();
    let output = run("edit.annotations", vec![bytes], json!({"action":"edit","changes":[
        {"action":"add","page":1,"annotation":{"kind":"text","rect":[80,610,20,20],"contents":"Remove new"}},
        {"action":"update","page":1,"index":0,"contents":"科学 note","rect":[90,640,30,25],"color":[0.0,0.5,1.0]},
        {"action":"remove","page":1,"index":1},
        {"action":"add","page":1,"annotation":{"kind":"text","rect":[90,600,20,20],"contents":"Second"}},
        {"action":"update","page":1,"index":1,"contents":"Final second"}
    ]})).unwrap();
    assert_eq!(
        output.data["postserialization_repair"]["reader_writer"],
        "lopdf"
    );
    let bytes = one_pdf(output);
    let output = Document::load_mem(&bytes).unwrap();
    let annotations = output.get_page_annotations(output.get_pages()[&1]).unwrap();
    assert_eq!(annotations.len(), 2);
    assert_eq!(
        annotations[0]
            .get(b"SyntheticPreservedKey")
            .unwrap()
            .as_str()
            .unwrap(),
        b"preserve this"
    );
    let rect = annotations[0]
        .get(b"Rect")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_float().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(rect, vec![90.0, 640.0, 120.0, 665.0]);
    let read = run("edit.annotations", vec![bytes], json!({"action":"inspect"})).unwrap();
    assert_eq!(read.data["annotations"][0]["contents"], "科学 note");
    assert_eq!(read.data["annotations"][1]["contents"], "Final second");
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "exact binary-representable PDF style constants are the independent serialization assertion"
)]
fn form_properties_and_hierarchical_removal_preserve_unrelated_objects() {
    let bytes = one_pdf(run("forms.add", vec![fixture("Form properties",1)], json!({"fields":[
        {"name":"group.remove","page":1,"rect":[72,620,100,20],"widget":{"kind":"text","value":"remove"}},
        {"name":"group.keep","page":1,"rect":[72,580,100,20],"widget":{"kind":"text","value":"keep"}}
    ]})).unwrap());
    let bytes = one_pdf(run("edit.annotations", vec![bytes], json!({"action":"edit","changes":[
        {"action":"add","page":1,"annotation":{"kind":"text","rect":[300,600,20,20],"contents":"Unrelated note"}}
    ]})).unwrap());
    let output = run(
        "forms.update",
        vec![bytes],
        json!({"fields":[
            {"name":"group.keep","readonly":true,"required":true,"tooltip":"科学 tooltip",
            "rect":[90,570,120,25],"max_length":32,"alignment":2,
            "background_color":[0.5,0.75,1.0],"border_color":[1.0,0.0,0.5],"border_width":2.5}
        ]}),
    )
    .unwrap();
    assert_eq!(
        output.data["postserialization_repair"]["upstream"],
        "pdf_oxide 0.3.78"
    );
    let output = run(
        "forms.remove",
        vec![one_pdf(output)],
        json!({"names":["group.remove"]}),
    )
    .unwrap();
    assert!(
        output
            .warnings
            .iter()
            .any(|warning| warning.contains("not confidential data erasure"))
    );
    let bytes = one_pdf(output);
    let inspection = run("forms.inspect", vec![bytes.clone()], json!({})).unwrap();
    let fields = inspection.data["fields"].as_array().unwrap();
    assert!(!fields.iter().any(|field| field["name"] == "group.remove"));
    let retained = fields
        .iter()
        .find(|field| field["name"] == "group.keep")
        .unwrap();
    assert_eq!(retained["value"]["value"], "keep");
    assert_eq!(retained["tooltip"], "科学 tooltip");
    assert_eq!(retained["rect"], json!([90.0, 570.0, 120.0, 25.0]));
    let parsed = Document::load_mem(&bytes).unwrap();
    let annotations = parsed.get_page_annotations(parsed.get_pages()[&1]).unwrap();
    assert_eq!(
        annotations.len(),
        2,
        "one widget and the unrelated note must remain"
    );
    assert!(annotations.iter().any(|annotation| {
        annotation
            .get(b"Contents")
            .ok()
            .and_then(|value| value.as_str().ok())
            == Some(b"Unrelated note".as_slice())
    }));
    let widget = annotations
        .iter()
        .find(|annotation| annotation.get(b"Subtype").unwrap().as_name().unwrap() == b"Widget")
        .unwrap();
    assert_eq!(widget.get(b"Ff").unwrap().as_i64().unwrap() & 3, 3);
    assert_eq!(widget.get(b"MaxLen").unwrap().as_i64().unwrap(), 32);
    assert_eq!(widget.get(b"Q").unwrap().as_i64().unwrap(), 2);
    assert_eq!(
        widget
            .get(b"BS")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"W")
            .unwrap()
            .as_float()
            .unwrap(),
        2.5
    );
    let appearance = widget.get(b"MK").unwrap().as_dict().unwrap();
    assert_eq!(
        appearance
            .get(b"BG")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_float().unwrap())
            .collect::<Vec<_>>(),
        vec![0.5, 0.75, 1.0]
    );
    assert_eq!(
        appearance
            .get(b"BC")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_float().unwrap())
            .collect::<Vec<_>>(),
        vec![1.0, 0.0, 0.5]
    );
}

#[test]
fn cyclic_acroform_read_paths_fail_before_upstream_recursion() {
    let mut source = Document::load_mem(&fixture("Cyclic form", 1)).unwrap();
    let field_id = source.new_object_id();
    source.objects.insert(
        field_id,
        dictionary! {"T"=>Object::string_literal("loop"),"FT"=>"Tx","Kids"=>vec![field_id.into()]}
            .into(),
    );
    let catalog = source.trailer.get(b"Root").unwrap().as_reference().unwrap();
    source
        .get_object_mut(catalog)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("AcroForm", dictionary! {"Fields"=>vec![field_id.into()]});
    let mut bytes = Vec::new();
    source.save_to(&mut bytes).unwrap();
    for (operation, params) in [
        ("forms.inspect", json!({})),
        ("forms.export", json!({"format":"fdf"})),
    ] {
        assert!(matches!(
            run(operation, vec![bytes.clone()], params),
            Err(BackendError::Malformed(_))
        ));
    }
}

#[test]
fn unicode_attachment_names_are_metadata_and_never_output_paths() {
    let payload = b"bounded synthetic attachment".to_vec();
    let bytes = one_pdf(
        run(
            "edit.attachments",
            vec![fixture("Attachment names", 1), payload.clone()],
            json!({"action":"add","files":[{"input":1,"name":"fixture.txt"}]}),
        )
        .unwrap(),
    );
    let mut source = Document::load_mem(&bytes).unwrap();
    let filename = "../科学.txt";
    let mut encoded = vec![0xfe, 0xff];
    encoded.extend(filename.encode_utf16().flat_map(u16::to_be_bytes));
    let mut changed = 0;
    for object in source.objects.values_mut() {
        if let Ok(dict) = object.as_dict_mut()
            && dict
                .get(b"Type")
                .ok()
                .and_then(|value| value.as_name().ok())
                == Some(b"Filespec".as_slice())
        {
            dict.set("UF", Object::string_literal(encoded.clone()));
            changed += 1;
        }
    }
    assert_eq!(changed, 1);
    let mut bytes = Vec::new();
    source.save_to(&mut bytes).unwrap();
    let extracted = run("edit.attachments", vec![bytes], json!({"action":"extract"})).unwrap();
    assert_eq!(extracted.data["files"][0]["source_name"], filename);
    assert_eq!(extracted.artifacts[0].name, "attachment-0001.bin");
    assert_eq!(extracted.artifacts[0].bytes, payload);
}

#[test]
fn merge_rejects_signature_markers_in_every_input_slot() {
    let plain = fixture("Unsigned merge input", 1);
    let mut source = Document::load_mem(&fixture("Synthetic certified input", 1)).unwrap();
    // Only a synthetic certification marker; no private key or real signature.
    let marker = source
        .add_object(dictionary! {"Type"=>"Sig", "Contents"=>Object::string_literal("synthetic")});
    let catalog = source.trailer.get(b"Root").unwrap().as_reference().unwrap();
    source
        .get_object_mut(catalog)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Perms", dictionary! {"DocMDP"=>marker});
    let mut certified = Vec::new();
    source.save_to(&mut certified).unwrap();
    for inputs in [
        vec![plain.clone(), certified.clone()],
        vec![certified, plain],
    ] {
        assert!(matches!(
            run("edit.merge", inputs, json!({})),
            Err(BackendError::Unsupported(_))
        ));
    }
}

fn appearance_form_fixture(inline_form: bool, separate_widget: bool) -> Vec<u8> {
    let mut document = Document::load_mem(&fixture("Appearance sentinel", 1)).unwrap();
    let page = document.get_pages()[&1];
    let font = document
        .add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica"});
    let appearance = document.add_object(Stream::new(
        dictionary! {"Type"=>"XObject", "Subtype"=>"Form", "FormType"=>1,
        "BBox"=>vec![0.into(),0.into(),200.into(),30.into()],
        "Resources"=>dictionary! {"Font"=>dictionary! {"Helv"=>font}}},
        b"q 1 1 1 rg 0 0 200 30 re f 0 0 0 RG 1 w 0.5 0.5 199 29 re S BT /Helv 18 Tf 0 g 2 6 Td (Before) Tj ET Q\n".to_vec(),
    ));
    let widget = document.add_object(dictionary! {
        "Type"=>"Annot", "Subtype"=>"Widget", "P"=>page, "F"=>4,
        "Rect"=>vec![72.into(),580.into(),272.into(),610.into()],
        "DA"=>Object::string_literal("/Helv 18 Tf 0 g"),
        "AP"=>dictionary! {"N"=>appearance},
        "MK"=>dictionary! {"BG"=>vec![1.into(),1.into(),1.into()],
        "BC"=>vec![0.into(),0.into(),0.into()]},
        "BS"=>dictionary! {"W"=>1, "S"=>"S"}
    });
    let field = if separate_widget {
        let field = document.add_object(dictionary! {
            "FT"=>"Tx", "T"=>Object::string_literal("appearance"),
            "V"=>Object::string_literal("Before"), "Kids"=>vec![widget.into()]
        });
        document
            .get_dictionary_mut(widget)
            .unwrap()
            .set("Parent", field);
        field
    } else {
        let field = document.get_dictionary_mut(widget).unwrap();
        field.set("FT", "Tx");
        field.set("T", Object::string_literal("appearance"));
        field.set("V", Object::string_literal("Before"));
        widget
    };
    document
        .get_dictionary_mut(page)
        .unwrap()
        .set("Annots", vec![widget.into()]);
    let form = dictionary! {
        "Fields"=>vec![field.into()], "NeedAppearances"=>false,
        "DA"=>Object::string_literal("/Helv 18 Tf 0 g"),
        "DR"=>dictionary! {"Font"=>dictionary! {"Helv"=>font}},
        "CO"=>vec![field.into()],
        "SyntheticFormSentinel"=>Object::string_literal("preserve original form catalog")
    };
    let form = if inline_form {
        Object::Dictionary(form)
    } else {
        document.add_object(form).into()
    };
    let catalog = document.catalog_mut().unwrap();
    catalog.set("AcroForm", form);
    catalog.set(
        "SyntheticCatalogSentinel",
        Object::string_literal("preserve unrelated catalog"),
    );
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn form_dictionary(document: &Document) -> &lopdf::Dictionary {
    document
        .get_dict_in_dict(document.catalog().unwrap(), b"AcroForm")
        .unwrap()
}

fn sole_form_field(document: &Document) -> &lopdf::Dictionary {
    let fields = form_dictionary(document)
        .get(b"Fields")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(fields.len(), 1);
    document
        .dereference(&fields[0])
        .unwrap()
        .1
        .as_dict()
        .unwrap()
}

fn normal_appearance_bytes(document: &Document) -> Vec<u8> {
    let widgets = document
        .get_page_annotations(document.get_pages()[&1])
        .unwrap();
    assert_eq!(widgets.len(), 1);
    let appearances = document.get_dict_in_dict(widgets[0], b"AP").unwrap();
    let stream = document
        .dereference(appearances.get(b"N").unwrap())
        .unwrap()
        .1
        .as_stream()
        .unwrap();
    stream
        .decompressed_content()
        .unwrap_or_else(|_| stream.content.clone())
}

fn assert_form_sentinels(document: &Document) {
    assert_eq!(
        document
            .catalog()
            .unwrap()
            .get(b"SyntheticCatalogSentinel")
            .unwrap()
            .as_str()
            .unwrap(),
        b"preserve unrelated catalog"
    );
    let form = form_dictionary(document);
    assert_eq!(
        form.get(b"SyntheticFormSentinel")
            .unwrap()
            .as_str()
            .unwrap(),
        b"preserve original form catalog"
    );
    assert!(form.has(b"DR"));
    assert!(form.has(b"DA"));
    assert_eq!(form.get(b"CO").unwrap().as_array().unwrap().len(), 1);
    assert_eq!(
        sole_form_field(document)
            .get(b"T")
            .unwrap()
            .as_str()
            .unwrap(),
        b"appearance"
    );
}

#[test]
fn adding_to_existing_inline_or_indirect_acroform_rejects_without_catalog_loss() {
    for inline_form in [false, true] {
        let source = appearance_form_fixture(inline_form, false);
        let result = run(
            "forms.add",
            vec![source.clone()],
            json!({"fields":[
                {"name":"new","page":1,"rect":[72,500,150,20],"widget":{"kind":"text","value":"New"}}
            ]}),
        );
        assert!(
            matches!(result, Err(BackendError::Unsupported(reason)) if reason.contains("existing AcroForm") && reason.contains("preservation"))
        );
        assert_form_sentinels(&Document::load_mem(&source).unwrap());
    }
}

#[test]
fn pinned_form_add_rebuild_drops_unique_catalog_and_inline_existing_fields() {
    // Preserve direct pinned-library failure evidence independently of adapter
    // rejection. This is intentionally not an assertion of supported behavior.
    for inline_form in [false, true] {
        let source = appearance_form_fixture(inline_form, false);
        let mut editor = DocumentEditor::from_bytes(source.clone()).unwrap();
        editor
            .add_form_field(
                0,
                pdf_oxide::writer::TextFieldWidget::new(
                    "new",
                    pdf_oxide::geometry::Rect::new(72.0, 500.0, 150.0, 20.0),
                )
                .with_value("New"),
            )
            .unwrap();
        let output = editor.save_to_bytes().unwrap();
        let parsed = Document::load_mem(&output).unwrap();
        let form = form_dictionary(&parsed);
        assert!(
            !form.has(b"SyntheticFormSentinel"),
            "pinned catalog-loss evidence changed; review preservation guard before enabling additions"
        );
        assert!(!form.has(b"CO"));
        let names = form
            .get(b"Fields")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|field| {
                parsed
                    .dereference(field)
                    .unwrap()
                    .1
                    .as_dict()
                    .unwrap()
                    .get(b"T")
                    .unwrap()
                    .as_str()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert!(names.contains(&b"new".as_slice()));
        assert_eq!(names.contains(&b"appearance".as_slice()), !inline_form);
        assert_eq!(
            parsed
                .catalog()
                .unwrap()
                .get(b"SyntheticCatalogSentinel")
                .unwrap()
                .as_str()
                .unwrap(),
            b"preserve unrelated catalog"
        );
        assert_form_sentinels(&Document::load_mem(&source).unwrap());
    }
}

#[test]
fn appearance_bearing_fields_and_child_widgets_reject_changed_values_and_styles() {
    for separate_widget in [false, true] {
        let source = appearance_form_fixture(false, separate_widget);
        for value in [
            json!({"kind":"text","value":"After"}),
            json!({"kind":"none"}),
        ] {
            assert!(
                matches!(run("forms.fill", vec![source.clone()], json!({"fields":{"appearance":value}})),
                Err(BackendError::Unsupported(reason)) if reason.contains("/AP") && reason.contains("stale"))
            );
        }
        for property in [
            json!({"rect":[72,580,250,35]}),
            json!({"max_length":32}),
            json!({"alignment":2}),
            json!({"background_color":[1,0,0]}),
            json!({"border_color":[0,1,0]}),
            json!({"border_width":3}),
        ] {
            let mut properties = property;
            properties["name"] = json!("appearance");
            assert!(
                matches!(run("forms.update", vec![source.clone()], json!({"fields":[properties]})),
                Err(BackendError::Unsupported(reason)) if reason.contains("/AP") && reason.contains("stale"))
            );
        }
        assert_form_sentinels(&Document::load_mem(&source).unwrap());
    }
}

#[test]
fn nonvisual_form_properties_and_noop_fill_preserve_existing_appearance_and_catalog() {
    let source = appearance_form_fixture(false, false);
    let original = Document::load_mem(&source).unwrap();
    for (operation, params) in [
        (
            "forms.fill",
            json!({"fields":{"appearance":{"kind":"text","value":"Before"}}}),
        ),
        (
            "forms.update",
            json!({"fields":[{"name":"appearance","readonly":true,"required":true,"tooltip":"Review only"}]}),
        ),
    ] {
        let output = one_pdf(run(operation, vec![source.clone()], params).unwrap());
        let parsed = Document::load_mem(&output).unwrap();
        assert_form_sentinels(&parsed);
        assert_eq!(
            normal_appearance_bytes(&parsed),
            normal_appearance_bytes(&original)
        );
        assert_eq!(
            sole_form_field(&parsed)
                .get(b"V")
                .unwrap()
                .as_str()
                .unwrap(),
            b"Before"
        );
        assert!(
            !form_dictionary(&parsed)
                .get(b"NeedAppearances")
                .unwrap()
                .as_bool()
                .unwrap()
        );
        if operation == "forms.update" {
            let field = sole_form_field(&parsed);
            assert_eq!(field.get(b"Ff").unwrap().as_i64().unwrap() & 3, 3);
            assert_eq!(field.get(b"TU").unwrap().as_str().unwrap(), b"Review only");
        }
    }
}

#[test]
fn pinned_fill_then_separate_flatten_bakes_original_appearance_and_adapter_rejects() {
    // This one-page synthetic fixture is also suitable for independent Poppler
    // rendering: V changes Before -> After while the flattened page paints Before.
    let source = appearance_form_fixture(false, false);
    capture_form_fixture("forms-appearance-source.pdf", &source);
    let original = Document::load_mem(&source).unwrap();
    let mut editor = DocumentEditor::from_bytes(source.clone()).unwrap();
    editor
        .set_form_field_value("appearance", FormFieldValue::Text("After".into()))
        .unwrap();
    let filled = editor.save_to_bytes().unwrap();
    capture_form_fixture("forms-appearance-upstream-filled.pdf", &filled);
    let parsed = Document::load_mem(&filled).unwrap();
    assert_eq!(
        sole_form_field(&parsed)
            .get(b"V")
            .unwrap()
            .as_str()
            .unwrap(),
        b"After"
    );
    assert_eq!(
        normal_appearance_bytes(&parsed),
        normal_appearance_bytes(&original)
    );
    assert!(
        form_dictionary(&parsed)
            .get(b"NeedAppearances")
            .unwrap()
            .as_bool()
            .unwrap()
    );
    assert!(
        matches!(run("forms.flatten", vec![filled.clone()], json!({})),
        Err(BackendError::Unsupported(reason)) if reason.contains("NeedAppearances") && reason.contains("stale"))
    );

    let mut separate_session = DocumentEditor::from_bytes(filled).unwrap();
    separate_session.flatten_forms().unwrap();
    let flattened = separate_session.save_to_bytes().unwrap();
    capture_form_fixture(
        "forms-appearance-upstream-separate-flattened.pdf",
        &flattened,
    );
    let parsed = Document::load_mem(&flattened).unwrap();
    assert!(!parsed.catalog().unwrap().has(b"AcroForm"));
    let page_id = parsed.get_pages()[&1];
    assert!(parsed.get_page_annotations(page_id).unwrap().is_empty());
    let page = parsed.get_dictionary(page_id).unwrap();
    let resources = parsed.get_dict_in_dict(page, b"Resources").unwrap();
    let xobjects = parsed.get_dict_in_dict(resources, b"XObject").unwrap();
    let content = Content::decode(&parsed.get_page_content(page_id)).unwrap();
    let painted_streams = content
        .operations
        .iter()
        .filter(|operation| operation.operator == "Do")
        .map(|operation| {
            let name = operation.operands[0].as_name().unwrap();
            let stream = parsed
                .dereference(xobjects.get(name).unwrap())
                .unwrap()
                .1
                .as_stream()
                .unwrap();
            stream
                .decompressed_content()
                .unwrap_or_else(|_| stream.content.clone())
        })
        .collect::<Vec<_>>();
    assert!(
        painted_streams.iter().any(|bytes| bytes
            .windows(b"(Before) Tj".len())
            .any(|window| window == b"(Before) Tj")),
        "pinned stale-appearance evidence changed; independently requalify flattening before relaxing its guard"
    );
    assert!(!painted_streams.iter().any(|bytes| {
        bytes
            .windows(b"After".len())
            .any(|window| window == b"After")
    }));
    assert_form_sentinels(&Document::load_mem(&source).unwrap());

    // The pinned private regeneration path sees text staged in this same
    // editor session. This control does not establish interactive regeneration
    // or style preservation, and is not substituted for forms.fill.
    let mut same_session = DocumentEditor::from_bytes(source).unwrap();
    same_session
        .set_form_field_value("appearance", FormFieldValue::Text("After".into()))
        .unwrap();
    same_session.flatten_forms().unwrap();
    let control = same_session.save_to_bytes().unwrap();
    capture_form_fixture(
        "forms-appearance-upstream-same-session-control.pdf",
        &control,
    );
    assert_form_control_and_native_reader_evidence(control);
}

fn assert_form_control_and_native_reader_evidence(control: Vec<u8>) {
    let parsed = Document::load_mem(&control).unwrap();
    let page_id = parsed.get_pages()[&1];
    let resources = parsed
        .get_dict_in_dict(parsed.get_dictionary(page_id).unwrap(), b"Resources")
        .unwrap();
    let xobjects = parsed.get_dict_in_dict(resources, b"XObject").unwrap();
    let page_content = Content::decode(&parsed.get_page_content(page_id)).unwrap();
    let invocations = page_content
        .operations
        .iter()
        .filter(|operation| operation.operator == "Do")
        .collect::<Vec<_>>();
    assert_eq!(invocations.len(), 1);
    let form_ref = xobjects
        .get(invocations[0].operands[0].as_name().unwrap())
        .unwrap()
        .as_reference()
        .unwrap();
    let form = parsed.get_object(form_ref).unwrap().as_stream().unwrap();
    assert_eq!(
        form.dict.get(b"Subtype").unwrap().as_name().unwrap(),
        b"Form"
    );
    let bytes = form
        .decompressed_content()
        .unwrap_or_else(|_| form.content.clone());
    let operations = Content::decode(&bytes).unwrap().operations;
    assert_eq!(
        operations
            .iter()
            .map(|operation| operation.operator.as_str())
            .collect::<Vec<_>>(),
        vec!["BT", "rg", "Tf", "Td", "Tj", "ET"],
        "same-session text generation omits original background and border painting; it is not a visual fallback"
    );
    let shown = operations
        .iter()
        .filter(|operation| operation.operator == "Tj")
        .map(|operation| operation.operands[0].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(shown, vec![b"After".as_slice()]);
    let appearance_resources = parsed.get_dict_in_dict(&form.dict, b"Resources").unwrap();
    let fonts = parsed
        .get_dict_in_dict(appearance_resources, b"Font")
        .unwrap();
    let helvetica = fonts.get(b"Helv").unwrap().as_dict().unwrap();
    assert_eq!(
        helvetica.get(b"Subtype").unwrap().as_name().unwrap(),
        b"Type1"
    );

    // Preserve the newly observed native omission, not whitespace-normalized
    // output: is_form_xobject peeks at the FIRST raw /Subtype, incorrectly
    // reading the inline font's Type1 before the enclosing Form subtype.
    let native_ref = pdf_oxide::object::ObjectRef::new(form_ref.0, form_ref.1);
    let cold = PdfDocument::from_bytes(control.clone()).unwrap();
    let cold_text = cold.extract_text(0).unwrap();
    let cold_chars = cold
        .extract_chars(0)
        .unwrap()
        .iter()
        .map(|glyph| glyph.char)
        .collect::<String>();
    assert!(!cold.is_form_xobject(native_ref));
    assert_eq!(cold_text, "Appearance sentinel page 1");
    assert_eq!(cold_chars, "Appearance sentinel page 1");

    // Loading the exact Form before the flawed peek uses its parsed outer
    // dictionary. The same bytes then expose After as native text AND glyphs.
    // This is diagnostic evidence only, never a production parser workaround.
    let preloaded = PdfDocument::from_bytes(control).unwrap();
    let object = preloaded.load_object(native_ref).unwrap();
    assert_eq!(
        object.as_dict().unwrap().get("Subtype").unwrap().as_name(),
        Some("Form")
    );
    assert!(preloaded.is_form_xobject(native_ref));
    let text = preloaded.extract_text(0).unwrap();
    let chars = preloaded
        .extract_chars(0)
        .unwrap()
        .iter()
        .map(|glyph| glyph.char)
        .collect::<String>();
    capture_form_fixture("forms-appearance-native-observations.json", &serde_json::to_vec_pretty(&json!({
        "upstream":"pdf_oxide 0.3.78", "cold_text":cold_text, "cold_chars":cold_chars,
        "preloaded_form_text":text, "preloaded_form_chars":chars,
        "decoded_invoked_text":"After", "same_session_style_preserved":false,
        "limitation":"native raw Subtype peek can mistake an inline font for the enclosing Form XObject; this diagnostic preload is not a production fallback"
    })).unwrap());
    assert!(text.contains("After"));
    assert!(!text.contains("Before"));
    assert!(chars.contains("After"));
    assert!(!chars.contains("Before"));
}

#[test]
fn complete_existing_form_appearance_remains_available_for_flattening() {
    let source = appearance_form_fixture(false, false);
    let output = one_pdf(run("forms.flatten", vec![source], json!({})).unwrap());
    capture_form_fixture("forms-appearance-control-flattened.pdf", &output);
    let parsed = Document::load_mem(&output).unwrap();
    assert!(!parsed.catalog().unwrap().has(b"AcroForm"));
    assert!(
        PdfDocument::from_bytes(output)
            .unwrap()
            .extract_text(0)
            .unwrap()
            .contains("Before")
    );
}

#[test]
fn unregeneratable_widget_without_normal_appearance_rejects_before_field_loss() {
    for separate_widget in [false, true] {
        let mut source =
            Document::load_mem(&appearance_form_fixture(false, separate_widget)).unwrap();
        let field = form_dictionary(&source)
            .get(b"Fields")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .as_reference()
            .unwrap();
        // An approved synthetic unsupported field type makes the native fallback
        // fail explicitly to generate paint; no real signing or private data.
        source
            .get_dictionary_mut(field)
            .unwrap()
            .set("FT", "SyntheticUnsupported");
        let page_id = source.get_pages()[&1];
        let widget = source
            .get_dictionary(page_id)
            .unwrap()
            .get(b"Annots")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .as_reference()
            .unwrap();
        let widget = source.get_dictionary_mut(widget).unwrap();
        if separate_widget {
            widget.set("AP", dictionary! {}); // Missing normal N, on an anonymous child.
        } else {
            widget.remove(b"AP");
        }
        let mut bytes = Vec::new();
        source.save_to(&mut bytes).unwrap();
        let prefix = if separate_widget {
            "forms-missing-normal-child"
        } else {
            "forms-missing-ap-merged"
        };
        capture_form_fixture(&format!("{prefix}-source.pdf"), &bytes);
        assert_form_sentinels(&Document::load_mem(&bytes).unwrap());
        assert!(
            matches!(run("forms.flatten", vec![bytes.clone()], json!({})),
            Err(BackendError::Unsupported(reason)) if reason.contains("normal /AP") && reason.contains("may omit"))
        );

        let mut native = DocumentEditor::from_bytes(bytes.clone()).unwrap();
        native.flatten_forms().unwrap();
        let derivative = native.save_to_bytes().unwrap();
        capture_form_fixture(&format!("{prefix}-upstream-flattened.pdf"), &derivative);
        assert!(
            native
                .flatten_warnings()
                .iter()
                .any(|warning| warning.contains("no /AP") && warning.contains("blank rectangle"))
        );
        let parsed = Document::load_mem(&derivative).unwrap();
        assert!(!parsed.catalog().unwrap().has(b"AcroForm"));
        assert_eq!(
            parsed
                .get_page_annotations(parsed.get_pages()[&1])
                .unwrap()
                .len(),
            1,
            "the native omission leaves the unpainted widget even after dropping its field catalog"
        );
        assert_eq!(
            parsed.get_page_content(parsed.get_pages()[&1]),
            source.get_page_content(page_id),
            "no static appearance for the Before field value was painted"
        );
        assert_unpainted_widget_evidence(&parsed, derivative, prefix, native.flatten_warnings());
        assert_form_sentinels(&Document::load_mem(&bytes).unwrap());
    }
}

fn assert_unpainted_widget_evidence(
    parsed: &Document,
    derivative: Vec<u8>,
    prefix: &str,
    warnings: &[String],
) {
    let paint = Content::decode(&parsed.get_page_content(parsed.get_pages()[&1])).unwrap();
    assert_eq!(
        paint
            .operations
            .iter()
            .map(|operation| operation.operator.as_str())
            .collect::<Vec<_>>(),
        vec!["BT", "Tf", "Td", "Tj", "ET"],
        "no appearance XObject or extra painting was appended"
    );
    assert_eq!(
        paint.operations[3].operands[0].as_str().unwrap(),
        b"Appearance sentinel page 1"
    );
    let content_only_text = parsed.extract_text(&[1]).unwrap();
    assert!(!content_only_text.contains("Before"));
    assert!(content_only_text.contains("Appearance sentinel page 1"));
    let text = PdfDocument::from_bytes(derivative)
        .unwrap()
        .extract_text(0)
        .unwrap();
    // Native text extraction appends retained Widget /V values even after
    // AcroForm removal. Recoverable Before is not proof of flattened paint.
    assert!(text.contains("Before"));
    assert!(text.contains("Appearance sentinel page 1"));
    capture_form_fixture(&format!("{prefix}-observations.json"), &serde_json::to_vec_pretty(&json!({
        "native_text_including_retained_widget_values":text,
        "independent_content_only_text":content_only_text,
        "page_paint_operators":["BT","Tf","Td","Tj","ET"],
        "only_painted_text":"Appearance sentinel page 1",
        "flatten_warnings":warnings,
        "scope":"the field value remains recoverable in the widget but was not painted into static page content"
    })).unwrap());
}

fn link_annotation_fixture(first_has_appearance: bool) -> Vec<u8> {
    let mut document = Document::load_mem(&fixture("Annotation selection sentinel", 2)).unwrap();
    for (number, page_id) in document.get_pages() {
        let mut link = dictionary! {
            "Type"=>"Annot", "Subtype"=>"Link",
            "Rect"=>vec![72.into(),580.into(),172.into(),600.into()],
            "Border"=>vec![0.into(),0.into(),1.into()],
            "A"=>dictionary! {"S"=>"URI", "URI"=>Object::string_literal(format!("https://example.invalid/synthetic-page-{number}"))},
            "SyntheticAnnotationSentinel"=>Object::string_literal(format!("preserve-page-{number}"))
        };
        if number == 1 && first_has_appearance {
            let appearance = document.add_object(Stream::new(
                dictionary! {
                    "Type"=>"XObject", "Subtype"=>"Form", "FormType"=>1,
                    "BBox"=>vec![0.into(),0.into(),100.into(),20.into()],
                    "Resources"=>dictionary! {}
                },
                b"q 0 0 1 RG 1 w 0.5 0.5 99 19 re S Q\n".to_vec(),
            ));
            link.set("AP", dictionary! {"N"=>appearance});
        }
        let annotation = document.add_object(link);
        document
            .get_dictionary_mut(page_id)
            .unwrap()
            .set("Annots", vec![annotation.into()]);
    }
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn pinned_link_only_flatten_retains_interactivity_and_adapter_explicitly_rejects() {
    let source = link_annotation_fixture(false);
    capture_form_fixture("annotations-no-ap-source.pdf", &source);
    let original = Document::load_mem(&source).unwrap();
    let mut editor = DocumentEditor::from_bytes(source.clone()).unwrap();
    editor.flatten_page_annotations(0).unwrap();
    let native = editor.save_to_bytes().unwrap();
    capture_form_fixture("annotations-no-ap-upstream-flattened.pdf", &native);
    let parsed = Document::load_mem(&native).unwrap();
    for page in [1, 2] {
        let before = original
            .get_page_annotations(original.get_pages()[&page])
            .unwrap();
        let after = parsed
            .get_page_annotations(parsed.get_pages()[&page])
            .unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(
            after, before,
            "pinned no-AP flatten retains selected interactive links; preserve this defect evidence"
        );
        let action = after[0].get(b"A").unwrap().as_dict().unwrap();
        assert_eq!(action.get(b"S").unwrap().as_name().unwrap(), b"URI");
    }
    assert!(
        matches!(run_with_pages("edit.annotations", vec![source], Some((1,1)), json!({"action":"flatten"})),
        Err(BackendError::Unsupported(reason)) if reason.contains("normal /AP") && reason.contains("interactive /Annots"))
    );
}

#[test]
fn annotation_flatten_removes_selected_links_and_preserves_nonselected_no_ap_links() {
    let source = link_annotation_fixture(true);
    capture_form_fixture("annotations-selected-ap-source.pdf", &source);
    let original = Document::load_mem(&source).unwrap();
    let output = run_with_pages(
        "edit.annotations",
        vec![source.clone()],
        Some((1, 1)),
        json!({"action":"flatten"}),
    )
    .unwrap();
    assert_eq!(output.data["annotation_removal_verified"], true);
    assert_eq!(output.data["nonselected_annotations_preserved"], true);
    let bytes = one_pdf(output);
    capture_form_fixture("annotations-selected-ap-flattened.pdf", &bytes);
    let parsed = Document::load_mem(&bytes).unwrap();
    let pages = parsed.get_pages();
    assert!(!parsed.get_dictionary(pages[&1]).unwrap().has(b"Annots"));
    assert!(parsed.get_page_annotations(pages[&1]).unwrap().is_empty());
    assert_eq!(
        parsed.get_page_annotations(pages[&2]).unwrap(),
        original
            .get_page_annotations(original.get_pages()[&2])
            .unwrap()
    );
    assert_eq!(parsed.get_page_annotations(pages[&2]).unwrap().len(), 1);
    let content = Content::decode(&parsed.get_page_content(pages[&1])).unwrap();
    assert!(
        content
            .operations
            .iter()
            .any(|operation| operation.operator == "Do")
    );
    assert_eq!(
        parsed.get_page_content(pages[&2]),
        original.get_page_content(original.get_pages()[&2])
    );
    assert!(
        matches!(run_with_pages("edit.annotations", vec![source], Some((2,2)), json!({"action":"flatten"})),
        Err(BackendError::Unsupported(reason)) if reason.contains("normal /AP"))
    );
}

#[derive(Clone, Copy)]
enum FlattenCarrierKind {
    Annotation,
    Form,
}

impl FlattenCarrierKind {
    fn name(self) -> &'static str {
        match self {
            Self::Annotation => "annotation",
            Self::Form => "form",
        }
    }

    fn source(self) -> Vec<u8> {
        match self {
            Self::Annotation => link_annotation_fixture(true),
            Self::Form => appearance_form_fixture(false, false),
        }
    }

    fn upstream(self, bytes: Vec<u8>) -> Vec<u8> {
        let mut editor = DocumentEditor::from_bytes(bytes).unwrap();
        match self {
            Self::Annotation => editor.flatten_page_annotations(0).unwrap(),
            Self::Form => editor.flatten_forms().unwrap(),
        }
        editor.save_to_bytes().unwrap()
    }

    fn adapter(self, bytes: Vec<u8>) -> Result<OperationOutput, BackendError> {
        match self {
            Self::Annotation => run_with_pages(
                "edit.annotations",
                vec![bytes],
                Some((1, 1)),
                json!({"action":"flatten"}),
            ),
            Self::Form => run("forms.flatten", vec![bytes], json!({})),
        }
    }
}

fn flatten_carrier_fixture(
    kind: FlattenCarrierKind,
    contents_present: bool,
    inherited_resources: bool,
) -> Vec<u8> {
    let mut document = Document::load_mem(&kind.source()).unwrap();
    let page_id = document.get_pages()[&1];
    let mut resources = document
        .get_dict_in_dict(document.get_dictionary(page_id).unwrap(), b"Resources")
        .unwrap()
        .clone();
    let image = document.add_object(Stream::new(
        dictionary! {"Type"=>"XObject", "Subtype"=>"Image", "Width"=>1, "Height"=>1,
        "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>8},
        vec![16, 32, 48],
    ));
    resources.set("XObject", dictionary! {"SentinelImage"=>image});
    let mut content = document.get_page_content(page_id);
    content.extend_from_slice(b"\nq 10 0 0 10 300 600 cm /SentinelImage Do Q\n");
    let content = document.add_object(Stream::new(dictionary! {}, content));
    let parent = document
        .get_dictionary(page_id)
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    let page = document.get_dictionary_mut(page_id).unwrap();
    if contents_present {
        page.set("Contents", content);
    } else {
        page.remove(b"Contents");
    }
    if inherited_resources {
        page.remove(b"Resources");
        document
            .get_dictionary_mut(parent)
            .unwrap()
            .set("Resources", resources);
    } else {
        page.set("Resources", resources);
    }
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn effective_page_resources(document: &Document, page_number: u32) -> &lopdf::Dictionary {
    let mut node = document
        .get_dictionary(document.get_pages()[&page_number])
        .unwrap();
    for _ in 0..8 {
        if node.has(b"Resources") {
            return document.get_dict_in_dict(node, b"Resources").unwrap();
        }
        let parent = node.get(b"Parent").unwrap().as_reference().unwrap();
        node = document.get_dictionary(parent).unwrap();
    }
    panic!("bounded synthetic page resources must resolve");
}

fn carrier_resource_state(document: &Document) -> Value {
    let resources = effective_page_resources(document, 1);
    let font = document
        .get_dict_in_dict(resources, b"Font")
        .ok()
        .and_then(|fonts| fonts.get(b"F1").ok())
        .and_then(|font| document.dereference(font).ok())
        .and_then(|(_, font)| font.as_dict().ok())
        .and_then(|font| font.get(b"BaseFont").ok())
        .and_then(|name| name.as_name().ok());
    let image = document
        .get_dict_in_dict(resources, b"XObject")
        .ok()
        .and_then(|objects| objects.get(b"SentinelImage").ok())
        .and_then(|image| document.dereference(image).ok())
        .and_then(|(_, image)| image.as_stream().ok())
        .map(|stream| {
            stream
                .decompressed_content()
                .unwrap_or_else(|_| stream.content.clone())
        });
    json!({"original_font_resolves":font == Some(b"Helvetica"),
        "original_image_bytes_preserved":image == Some(vec![16,32,48])})
}

fn assert_flatten_without_contents_rejected(kind: FlattenCarrierKind) {
    let source = flatten_carrier_fixture(kind, false, false);
    let prefix = format!("flatten-{}-no-contents", kind.name());
    capture_form_fixture(&format!("{prefix}-source.pdf"), &source);
    let original = Document::load_mem(&source).unwrap();
    assert!(!normal_appearance_bytes(&original).is_empty());
    let native = kind.upstream(source.clone());
    capture_form_fixture(&format!("{prefix}-upstream.pdf"), &native);
    let parsed = Document::load_mem(&native).unwrap();
    let page = parsed.get_pages()[&1];
    assert!(parsed.get_page_content(page).is_empty());
    assert!(parsed.get_page_annotations(page).unwrap().is_empty());
    assert!(invoked_form_streams(&parsed, 1).is_empty());
    if matches!(kind, FlattenCarrierKind::Form) {
        assert!(!parsed.catalog().unwrap().has(b"AcroForm"));
    }
    let outcome = kind.adapter(source);
    if let Ok(output) = &outcome {
        capture_form_fixture(
            &format!("{prefix}-adapter-admitted.pdf"),
            &output.artifacts[0].bytes,
        );
    }
    assert!(
        matches!(outcome, Err(BackendError::Unsupported(reason)) if reason.contains("Contents")),
        "flattening must reject missing page Contents before deleting an unpainted appearance"
    );
}

fn assert_inherited_flatten_resources_rejected(kind: FlattenCarrierKind) {
    let source = flatten_carrier_fixture(kind, true, true);
    let prefix = format!("flatten-{}-inherited-resources", kind.name());
    capture_form_fixture(&format!("{prefix}-source.pdf"), &source);
    let original = Document::load_mem(&source).unwrap();
    let expected = json!({"original_font_resolves":true,"original_image_bytes_preserved":true});
    assert_eq!(carrier_resource_state(&original), expected);
    let native = kind.upstream(source.clone());
    capture_form_fixture(&format!("{prefix}-upstream.pdf"), &native);
    let native_document = Document::load_mem(&native).unwrap();
    let native_state = carrier_resource_state(&native_document);
    capture_form_fixture(&format!("{prefix}-observations.json"), &serde_json::to_vec_pretty(&json!({
        "before":expected, "direct_upstream":native_state,
        "scope":"independent effective page resource lookup; appearance existence alone is insufficient"
    })).unwrap());
    assert_eq!(
        native_state,
        json!({"original_font_resolves":false,"original_image_bytes_preserved":false}),
        "preserve the demonstrated native resource-shadowing defect"
    );
    assert!(
        matches!(kind.adapter(source), Err(BackendError::Unsupported(reason))
        if reason.contains("local /Resources") && reason.contains("shadow inherited"))
    );
}

fn assert_explicit_flatten_carriers_preserved(kind: FlattenCarrierKind) {
    for indirect_resources in [false, true] {
        for empty_content in [false, true] {
            let source = flatten_carrier_fixture(kind, true, false);
            let mut original = Document::load_mem(&source).unwrap();
            let page_id = original.get_pages()[&1];
            if empty_content {
                original
                    .get_dictionary_mut(page_id)
                    .unwrap()
                    .set("Contents", Object::Array(vec![]));
            }
            if indirect_resources {
                let resources = original
                    .get_dictionary_mut(page_id)
                    .unwrap()
                    .remove(b"Resources")
                    .unwrap();
                let resources = original.add_object(resources);
                original
                    .get_dictionary_mut(page_id)
                    .unwrap()
                    .set("Resources", resources);
            }
            let before = Content::decode(&original.get_page_content(page_id)).unwrap();
            let mut source = Vec::new();
            original.save_to(&mut source).unwrap();
            let output = one_pdf(kind.adapter(source).unwrap());
            let parsed = Document::load_mem(&output).unwrap();
            assert_eq!(
                carrier_resource_state(&parsed),
                json!({"original_font_resolves":true,"original_image_bytes_preserved":true})
            );
            let after = Content::decode(&parsed.get_page_content(parsed.get_pages()[&1])).unwrap();
            assert!(after.operations.len() >= before.operations.len());
            for (after, before) in after.operations.iter().zip(&before.operations) {
                assert_eq!(after.operator, before.operator);
                assert_eq!(after.operands, before.operands);
            }
            assert!(
                invoked_form_streams(&parsed, 1)
                    .iter()
                    .any(|stream| stream == &normal_appearance_bytes(&original))
            );
            assert!(
                parsed
                    .get_page_annotations(parsed.get_pages()[&1])
                    .unwrap()
                    .is_empty()
            );
            capture_form_fixture(
                &format!(
                    "flatten-{}-explicit-control-{indirect_resources}-{empty_content}.pdf",
                    kind.name()
                ),
                &output,
            );
        }
    }
}

#[test]
fn flatten_page_carrier_annotation_explicit_resources_preserve_content_and_appearance() {
    assert_explicit_flatten_carriers_preserved(FlattenCarrierKind::Annotation);
}

#[test]
fn flatten_page_carrier_form_explicit_resources_preserve_content_and_appearance() {
    assert_explicit_flatten_carriers_preserved(FlattenCarrierKind::Form);
}

#[test]
fn flatten_page_carrier_malformed_contents_rejects_without_publication() {
    for contents in [
        Object::Null,
        Object::Integer(0),
        Object::Array(vec![Object::Null]),
    ] {
        let mut document = Document::load_mem(&FlattenCarrierKind::Annotation.source()).unwrap();
        let page = document.get_pages()[&1];
        document
            .get_dictionary_mut(page)
            .unwrap()
            .set("Contents", contents);
        let mut source = Vec::new();
        document.save_to(&mut source).unwrap();
        assert!(matches!(FlattenCarrierKind::Annotation.adapter(source),
            Err(BackendError::Unsupported(reason)) if reason.contains("Contents")));
    }
}

#[test]
fn flatten_page_carrier_annotation_without_contents_rejects_lost_appearance() {
    assert_flatten_without_contents_rejected(FlattenCarrierKind::Annotation);
}

#[test]
fn flatten_page_carrier_form_without_contents_rejects_lost_appearance() {
    assert_flatten_without_contents_rejected(FlattenCarrierKind::Form);
}

#[test]
fn flatten_page_carrier_annotation_rejects_shadowed_inherited_fonts_images() {
    assert_inherited_flatten_resources_rejected(FlattenCarrierKind::Annotation);
}

#[test]
fn flatten_page_carrier_form_rejects_shadowed_inherited_fonts_images() {
    assert_inherited_flatten_resources_rejected(FlattenCarrierKind::Form);
}

fn state_appearance_fixture(indirect: bool, radio: bool, selected: bool) -> Vec<u8> {
    let mut document = Document::load_mem(&fixture("State appearance sentinel", 1)).unwrap();
    let page = document.get_pages()[&1];
    let make_stream = |content: &[u8]| {
        Stream::new(
            dictionary! {"Type"=>"XObject", "Subtype"=>"Form", "FormType"=>1,
            "BBox"=>vec![0.into(),0.into(),40.into(),40.into()],
            "Resources"=>dictionary! {}},
            content.to_vec(),
        )
    };
    let on = document.add_object(make_stream(
        b"q 1 0 1 rg 3 3 34 34 re f 0 1 1 RG 2 w 5 5 m 35 35 l S Q\n",
    ));
    let off = document.add_object(make_stream(
        b"q 0 0 1 RG 3 w 2 2 36 36 re S 2 38 m 38 2 l S Q\n",
    ));
    let states = dictionary! {"CustomOn"=>on, "Off"=>off};
    let states = if indirect {
        Object::Reference(document.add_object(states))
    } else {
        Object::Dictionary(states)
    };
    let state = if selected { "CustomOn" } else { "Off" };
    let widget = document.add_object(dictionary! {
        "Type"=>"Annot", "Subtype"=>"Widget", "P"=>page, "F"=>4,
        "Rect"=>vec![72.into(),580.into(),112.into(),620.into()],
        "FT"=>"Btn", "Ff"=>if radio { 1 << 15 } else { 0 },
        "T"=>Object::string_literal("custom-state"), "V"=>Object::Name(state.as_bytes().to_vec()),
        "AS"=>Object::Name(state.as_bytes().to_vec()), "AP"=>dictionary! {"N"=>states}
    });
    document
        .get_dictionary_mut(page)
        .unwrap()
        .set("Annots", vec![widget.into()]);
    let form =
        document.add_object(dictionary! {"Fields"=>vec![widget.into()], "NeedAppearances"=>false});
    document.catalog_mut().unwrap().set("AcroForm", form);
    let mut source = Vec::new();
    document.save_to(&mut source).unwrap();
    source
}

fn selected_state_appearance(document: &Document, indirect: bool) -> Vec<u8> {
    let widgets = document
        .get_page_annotations(document.get_pages()[&1])
        .unwrap();
    assert_eq!(widgets.len(), 1);
    let state = widgets[0].get(b"AS").unwrap().as_name().unwrap();
    let appearance = document.get_dict_in_dict(widgets[0], b"AP").unwrap();
    let normal = appearance.get(b"N").unwrap();
    assert_eq!(matches!(normal, Object::Reference(_)), indirect);
    let states = document.dereference(normal).unwrap().1.as_dict().unwrap();
    assert!(!states.has(b"BBox"));
    let stream = document
        .dereference(states.get(state).unwrap())
        .unwrap()
        .1
        .as_stream()
        .unwrap();
    stream
        .decompressed_content()
        .unwrap_or_else(|_| stream.content.clone())
}

fn flatten_state_upstream(source: &[u8], prefix: &str) -> (Vec<u8>, Vec<String>) {
    let mut editor = DocumentEditor::from_bytes(source.to_vec()).unwrap();
    editor.flatten_forms().unwrap();
    let native = editor.save_to_bytes().unwrap();
    capture_form_fixture(&format!("{prefix}-upstream.pdf"), &native);
    (native, editor.flatten_warnings().to_vec())
}

#[test]
fn indirect_widget_state_dictionary_rejects_silent_custom_appearance_replacement() {
    let mut admitted = Vec::new();
    for radio in [false, true] {
        for selected in [false, true] {
            let source = state_appearance_fixture(true, radio, selected);
            let prefix = format!("widget-state-indirect-radio-{radio}-selected-{selected}");
            capture_form_fixture(&format!("{prefix}-source.pdf"), &source);
            let original = Document::load_mem(&source).unwrap();
            let appearance = selected_state_appearance(&original, true);
            let (native, warnings) = flatten_state_upstream(&source, &prefix);
            let parsed = Document::load_mem(&native).unwrap();
            assert!(!parsed.catalog().unwrap().has(b"AcroForm"));
            assert!(
                warnings.is_empty(),
                "preserve the silent regeneration baseline"
            );
            let painted = invoked_form_streams(&parsed, 1);
            assert_eq!(painted.len(), 1);
            assert_ne!(
                painted[0], appearance,
                "pinned native discarded custom state paint"
            );
            capture_form_fixture(
                &format!("{prefix}-observations.json"),
                &serde_json::to_vec_pretty(&json!({
                    "radio_flag":radio,"selected":selected,"flatten_warnings":warnings,
                    "original_appearance":String::from_utf8_lossy(&appearance),
                    "painted_appearance":String::from_utf8_lossy(&painted[0]),
                    "custom_appearance_preserved":false
                }))
                .unwrap(),
            );
            match run("forms.flatten", vec![source], json!({})) {
                Err(BackendError::Unsupported(reason))
                    if reason.contains("indirect /AP /N state") => {}
                Ok(output) => {
                    assert_eq!(output.data["existing_widget_appearances_required"], true);
                    let bytes = one_pdf(output);
                    capture_form_fixture(&format!("{prefix}-adapter-admitted.pdf"), &bytes);
                    assert_ne!(
                        invoked_form_streams(&Document::load_mem(&bytes).unwrap(), 1),
                        vec![appearance]
                    );
                    admitted.push(prefix);
                }
                other => panic!("unexpected flatten result: {other:?}"),
            }
        }
    }
    assert!(
        admitted.is_empty(),
        "custom appearances were replaced without rejection: {admitted:?}"
    );
}

#[test]
fn direct_widget_state_dictionary_preserves_selected_custom_appearance() {
    for radio in [false, true] {
        for selected in [false, true] {
            let source = state_appearance_fixture(false, radio, selected);
            let prefix = format!("widget-state-direct-radio-{radio}-selected-{selected}");
            capture_form_fixture(&format!("{prefix}-source.pdf"), &source);
            let original = Document::load_mem(&source).unwrap();
            let appearance = selected_state_appearance(&original, false);
            let (native, warnings) = flatten_state_upstream(&source, &prefix);
            assert!(warnings.is_empty());
            assert_eq!(
                invoked_form_streams(&Document::load_mem(&native).unwrap(), 1),
                vec![appearance.clone()]
            );
            let output = run("forms.flatten", vec![source], json!({})).unwrap();
            assert_eq!(output.data["existing_widget_appearances_required"], true);
            let bytes = one_pdf(output);
            capture_form_fixture(&format!("{prefix}-adapter.pdf"), &bytes);
            let parsed = Document::load_mem(&bytes).unwrap();
            assert_eq!(invoked_form_streams(&parsed, 1), vec![appearance]);
            assert!(
                parsed
                    .get_page_annotations(parsed.get_pages()[&1])
                    .unwrap()
                    .is_empty()
            );
        }
    }
}
