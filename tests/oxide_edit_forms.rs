#![cfg(feature = "pdf-oxide")]

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use pdf_oxide::PdfDocument;
use pdf_oxide::editor::{DocumentEditor, EditableDocument};
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

fn run(
    operation: &str,
    sources: Vec<Vec<u8>>,
    params: Value,
) -> Result<OperationOutput, BackendError> {
    let inputs = sources.into_iter().map(Input::new).collect::<Vec<_>>();
    let snapshots = inputs.iter().map(|i| i.bytes.clone()).collect::<Vec<_>>();
    let limits = Limits::default();
    let result = execute(
        operation,
        &Context::new(&inputs, None, None, &limits),
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
fn flatten_removes_interactive_fields_and_preserves_source_text() {
    let flattened = one_pdf(run("forms.flatten", vec![form_fixture()], json!({})).unwrap());
    let inspection = run("forms.inspect", vec![flattened.clone()], json!({})).unwrap();
    assert!(inspection.data["fields"].as_array().unwrap().is_empty());
    let doc = PdfDocument::from_bytes(flattened).unwrap();
    assert!(doc.extract_text(0).unwrap().contains("Form page 1"));
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
