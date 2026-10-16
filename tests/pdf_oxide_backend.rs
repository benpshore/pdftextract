#![cfg(feature = "pdf-oxide")]

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use tpe::backend::pdf_oxide_backend::PdfOxideBackend;
use tpe::backend::{BackendError, Extractor, by_name, probe_pdf};
use tpe::pipeline::run_job;
use tpe::schema::{Job, Status};

fn fixture(actual_text: Option<&str>, damaged: bool) -> Vec<u8> {
    let mut document = Document::with_version("1.5");
    let tree = document.new_object_id();
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let mut operations = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["F1".into(), 12.into()]),
        Operation::new("Td", vec![72.into(), 700.into()]),
    ];
    if let Some(text) = actual_text {
        let mut unicode = vec![0xfe, 0xff];
        unicode.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
        operations.push(Operation::new(
            "BDC",
            vec![
                "Span".into(),
                dictionary! { "ActualText" => Object::string_literal(unicode) }.into(),
            ],
        ));
    }
    operations.push(Operation::new("Tj", vec![Object::string_literal("X")]));
    if actual_text.is_some() {
        operations.push(Operation::new("EMC", vec![]));
    }
    operations.push(Operation::new("ET", vec![]));
    let content = if damaged {
        Stream::new(
            dictionary! { "Filter" => "ASCIIHexDecode" },
            b"GG>".to_vec(),
        )
    } else {
        Stream::new(dictionary! {}, Content { operations }.encode().unwrap())
    };
    let content = document.add_object(content);
    let page = document.add_object(dictionary! {
        "Type" => "Page", "Parent" => tree, "Contents" => content,
        "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        "Rotate" => 90,
    });
    document.objects.insert(
        tree,
        dictionary! {
            "Type" => "Pages", "Count" => 1, "Kids" => vec![page.into()],
            "MediaBox" => vec![10.into(), 20.into(), 610.into(), 800.into()],
        }
        .into(),
    );
    let root = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => tree });
    document.trailer.set("Root", root);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn form_fixture(inline_font: bool, inherited: bool, cyclic: bool) -> Vec<u8> {
    let mut document = Document::with_version("1.7");
    let tree = document.new_object_id();
    let font = dictionary! {
        "Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica", "Encoding"=>"WinAnsiEncoding",
    };
    let font = if inline_font {
        Object::Dictionary(font)
    } else {
        Object::Reference(document.add_object(font))
    };
    let resources = document.new_object_id();
    // Resources precedes the outer Subtype, reproducing the native shallow
    // scan's confusion between an inline font and its enclosing Form.
    let form = document.add_object(Stream::new(
        dictionary! {
            "Resources"=>dictionary!{"Font"=>dictionary!{"F1"=>font}},
            "Subtype"=>"Form", "Type"=>"XObject",
            "BBox"=>vec![0.into(),0.into(),612.into(),792.into()],
        },
        b"BT /F1 12 Tf 72 700 Td (FORM-CONTROL) Tj ET".to_vec(),
    ));
    document.objects.insert(
        resources,
        dictionary! {"XObject"=>dictionary!{"Fm"=>form}}.into(),
    );
    if cyclic {
        document
            .get_object_mut(form)
            .unwrap()
            .as_stream_mut()
            .unwrap()
            .dict
            .set("Resources", resources);
    }
    let content = document.add_object(Stream::new(dictionary! {}, b"/Fm Do".to_vec()));
    let page = document.add_object(dictionary! {
        "Type"=>"Page", "Parent"=>tree, "Contents"=>content,
        "MediaBox"=>vec![0.into(),0.into(),612.into(),792.into()],
    });
    document.objects.insert(
        tree,
        dictionary! {"Type"=>"Pages","Kids"=>vec![page.into()],"Count"=>1}.into(),
    );
    document
        .get_object_mut(if inherited { tree } else { page })
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Resources", resources);
    let root = document.add_object(dictionary! {"Type"=>"Catalog","Pages"=>tree});
    document.trailer.set("Root", root);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn shallow_form_subtype_omission_is_rejected_before_native_projection() {
    for inherited in [false, true] {
        let bytes = form_fixture(true, inherited, false);
        let cold = pdf_oxide::PdfDocument::from_bytes(bytes.clone()).unwrap();
        assert!(
            !cold.extract_text(0).unwrap().contains("FORM-CONTROL"),
            "fixture must preserve the actual upstream omission"
        );
        let independent = Document::load_mem(&bytes).unwrap();
        assert!(
            independent
                .objects
                .values()
                .any(|object| object.as_stream().is_ok_and(|stream| stream
                    .content
                    .windows(b"FORM-CONTROL".len())
                    .any(|part| part == b"FORM-CONTROL")))
        );
        let error = PdfOxideBackend
            .open(&bytes, None)
            .unwrap()
            .page_text(1)
            .unwrap_err();
        assert!(
            matches!(error, BackendError::Unsupported(ref message) if message.contains("omit Form text")),
            "{error}"
        );
        let inputs = [tpe::oxide::Input::new(bytes)];
        let limits = tpe::oxide::Limits::default();
        let context = tpe::oxide::Context::new(&inputs, None, None, &limits);
        for operation in [
            "query.text",
            "query.chars",
            "query.search",
            "convert.text",
            "convert.html",
        ] {
            let params = if operation == "query.search" {
                serde_json::json!({"pattern":"FORM-CONTROL"})
            } else {
                serde_json::json!({})
            };
            assert!(
                matches!(tpe::oxide::execute(operation, &context, &params), Err(BackendError::Unsupported(ref message)) if message.contains("omit Form text")),
                "{operation}"
            );
        }
    }
}

#[test]
fn form_preflight_preserves_indirect_font_controls_and_rejects_cycles() {
    for inherited in [false, true] {
        let bytes = form_fixture(false, inherited, false);
        let output = PdfOxideBackend
            .open(&bytes, None)
            .unwrap()
            .page_text(1)
            .unwrap();
        assert_eq!(
            output
                .spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>(),
            "FORM-CONTROL"
        );
    }
    let bytes = form_fixture(false, false, true);
    let error = PdfOxideBackend
        .open(&bytes, None)
        .unwrap()
        .page_text(1)
        .unwrap_err();
    assert!(
        matches!(error, BackendError::Malformed(ref message) if message.contains("cyclic Form resource")),
        "{error}"
    );
}

#[test]
fn immutable_bytes_real_geometry_fonts_and_ranges() {
    let mut bytes = fixture(None, false);
    let mut session = PdfOxideBackend.open(&bytes, None).unwrap();
    bytes.fill(0);
    assert_eq!(session.page_count(), 1);
    let page = session.page_text(1).unwrap();
    assert_eq!((page.width, page.height, page.rotation), (600.0, 780.0, 90));
    assert_eq!(
        page.spans
            .iter()
            .map(|span| span.text.as_str())
            .collect::<String>(),
        "X"
    );
    let span = &page.spans[0];
    assert_eq!(span.font.as_deref(), Some("Helvetica"));
    assert_eq!(span.size, Some(12.0));
    assert!((span.bbox.unwrap().x0 - 72.0).abs() < 0.1);
    assert!(span.bbox.unwrap().y0 > 680.0);
    assert_eq!(page.extraction_status(), Status::Partial);
    for page in [0, 2] {
        assert!(matches!(
            session.page_text(page),
            Err(BackendError::PageRange { .. })
        ));
    }
}

#[test]
fn actual_text_recovery_is_distinct_from_lopdf() {
    let bytes = fixture(Some("Recovered text"), false);
    let text = |backend: &dyn Extractor| {
        backend
            .open(&bytes, None)
            .unwrap()
            .page_text(1)
            .unwrap()
            .spans
            .into_iter()
            .map(|span| span.text)
            .collect::<String>()
    };
    assert_eq!(text(by_name("lopdf").unwrap().as_ref()), "X");
    assert_eq!(text(&PdfOxideBackend), "Recovered text");
}

#[test]
fn replacement_glyphs_are_retained_without_global_span_configuration() {
    let mut document = Document::load_mem(&fixture(None, false)).unwrap();
    let cmap = document.add_object(Stream::new(
        dictionary! {},
        b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
        /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
        /CMapName /Unmapped def\n/CMapType 2 def\n\
        1 begincodespacerange\n<00><FF>\nendcodespacerange\n\
        1 beginbfchar\n<58><FFFD>\nendbfchar\n\
        endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n"
            .to_vec(),
    ));
    for object in document.objects.values_mut() {
        if let Ok(font) = object.as_dict_mut()
            && font.has_type(b"Font")
        {
            font.set("ToUnicode", cmap);
        }
    }
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let page = PdfOxideBackend
        .open(&bytes, None)
        .unwrap()
        .page_text(1)
        .unwrap();
    assert!(page.spans.iter().any(|span| span.text.contains('\u{fffd}')));
    assert!(
        page.warnings
            .iter()
            .any(|warning| warning.starts_with("unicode_mapping:"))
    );
}

#[test]
fn uri_annotation_keeps_actual_target_and_indirect_rectangle() {
    let mut document = Document::load_mem(&fixture(None, false)).unwrap();
    let target = "https://doi.org/10.1234/actual-target";
    let uri = document.add_object(Object::string_literal(target));
    let action = document.add_object(dictionary! { "S" => "URI", "URI" => uri });
    let rectangle = document.add_object(vec![72.into(), 695.into(), 110.into(), 713.into()]);
    let annotation = document.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link", "A" => action, "Rect" => rectangle,
        "Contents" => Object::string_literal("Different label, not a DOI"),
    });
    let annotations = document.add_object(vec![Object::Reference(annotation)]);
    let page = *document.get_pages().get(&1).unwrap();
    document
        .get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", annotations);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let result = PdfOxideBackend
        .open(&bytes, None)
        .unwrap()
        .page_text(1)
        .unwrap();
    assert_eq!(result.links.len(), 1);
    assert_eq!(result.links[0].uri, target);
    assert_eq!(
        result.links[0].bbox,
        Some(tpe::schema::BBox {
            x0: 72.0,
            y0: 695.0,
            x1: 110.0,
            y1: 713.0
        })
    );
}

#[test]
fn silent_content_failure_stays_partial_in_pipeline_and_ledger() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("damaged.pdf");
    std::fs::write(&path, fixture(None, true)).unwrap();
    let result = run_job(&Job {
        path: path.to_string_lossy().into_owned(),
        backend: "pdf-oxide".into(),
        pages: None,
        password: None,
        max_bytes: None,
        figures_dir: None,
    })
    .unwrap();
    assert_eq!(result.backend.name, "pdf-oxide");
    assert_eq!(result.backend.version, "0.3.78");
    assert_eq!(result.document.pages, 1);
    assert_eq!(result.status, Status::Partial);
    assert_eq!(result.pages[0].extraction_status(), Status::Partial);
    assert_eq!(result.chunks[0].status, Status::Partial);
    assert!(result.pages[0].text.is_empty());
    let roundtrip = serde_json::from_str::<tpe::schema::ExtractionResult>(
        &serde_json::to_string(&result).unwrap(),
    )
    .unwrap();
    assert_eq!(roundtrip, result);
    let mut ledger = tpe::ledger::Ledger::open_in_memory().unwrap();
    let run = ledger.write_result(&result).unwrap();
    assert_eq!(ledger.load_result(run).unwrap().status, Status::Partial);
}

#[test]
fn supervised_cli_runs_pdf_oxide_and_reports_partial() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("probe.pdf");
    std::fs::write(&path, probe_pdf().unwrap()).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_tpe"))
        .args(["extract", "--backend", "pdf-oxide", "--json"])
        .arg("--db")
        .arg(directory.path().join("ledger.sqlite"))
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "partial");
    assert_eq!(result["backend"]["name"], "pdf-oxide");
}

fn table_fixture() -> Vec<u8> {
    let mut document = Document::load_mem(&fixture(None, false)).unwrap();
    let page_id = *document.get_pages().get(&1).unwrap();
    let content = b"0.5 w\n72 600 m 372 600 l S\n72 630 m 372 630 l S\n72 660 m 372 660 l S\n72 690 m 372 690 l S\n72 600 m 72 690 l S\n172 600 m 172 690 l S\n272 600 m 272 690 l S\n372 600 m 372 690 l S\nBT /F1 12 Tf 1 0 0 1 80 672 Tm (Item) Tj 1 0 0 1 180 672 Tm (Qty) Tj 1 0 0 1 280 672 Tm (Cost) Tj 1 0 0 1 80 642 Tm (Alpha) Tj 1 0 0 1 180 642 Tm (2) Tj 1 0 0 1 280 642 Tm (10) Tj 1 0 0 1 80 612 Tm (Beta) Tj 1 0 0 1 280 612 Tm (20) Tj ET";
    let stream = document.add_object(Stream::new(dictionary! {}, content.to_vec()));
    let page = document
        .get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    page.set("Contents", stream);
    page.set("Rotate", 0);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn table_candidates_keep_raw_rows_and_explicit_validation_error_in_ledger() {
    let bytes = table_fixture();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("table.pdf");
    std::fs::write(&path, &bytes).unwrap();
    let result = run_job(&Job {
        path: path.to_string_lossy().into_owned(),
        backend: "pdf-oxide".into(),
        pages: None,
        password: None,
        max_bytes: None,
        figures_dir: None,
    })
    .unwrap();
    let tables = &result.pages[0].tables;
    assert_eq!(tables.len(), 1, "{tables:#?}");
    let table = &tables[0];
    assert_eq!(table.col_count, 3);
    assert_eq!(table.source, "pdf-oxide/0.3.78/extract_tables");
    assert!(
        table
            .validation_error
            .as_deref()
            .unwrap()
            .starts_with("unsupported_table_validation:")
    );
    assert!(table.bbox.is_some());
    let cells: Vec<Vec<&str>> = table
        .rows
        .iter()
        .map(|row| row.cells.iter().map(|cell| cell.text.as_str()).collect())
        .collect();
    assert_eq!(
        cells,
        vec![
            vec!["Item", "Qty", "Cost"],
            vec!["Alpha", "2", "10"],
            vec!["Beta", "", "20"]
        ]
    );
    assert!(
        table
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .all(|cell| cell.rowspan >= 1 && cell.colspan >= 1)
    );
    let mut ledger = tpe::ledger::Ledger::open_in_memory().unwrap();
    let run = ledger.write_result(&result).unwrap();
    assert_eq!(ledger.load_result(run).unwrap(), result);
    if let Some(directory) = std::env::var_os("TPE_OXIDE_FIXTURE_CAPTURE") {
        std::fs::write(
            std::path::Path::new(&directory).join("ruled-table-empty-cell.pdf"),
            bytes,
        )
        .unwrap();
    }
}

#[test]
fn page_selection_is_bounded_before_extraction_and_large_documents_allow_subranges() {
    let mut document = Document::load_mem(&fixture(None, false)).unwrap();
    let first = *document.get_pages().get(&1).unwrap();
    let page = document.get_object(first).unwrap().clone();
    let parent = page
        .as_dict()
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    let mut kids = vec![Object::Reference(first)];
    for _ in 1..257 {
        kids.push(document.add_object(page.clone()).into());
    }
    let tree = document
        .get_object_mut(parent)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    tree.set("Kids", kids);
    tree.set("Count", 257);
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("257-pages.pdf");
    document.save(&path).unwrap();
    let mut job = Job {
        path: path.to_string_lossy().into_owned(),
        backend: "pdf-oxide".into(),
        pages: None,
        password: None,
        max_bytes: None,
        figures_dir: None,
    };
    let error = run_job(&job).unwrap_err().to_string();
    assert!(
        error.contains("selected 257 pages exceeds backend limit 256"),
        "{error}"
    );
    job.pages = Some((257, 257));
    let result = run_job(&job).unwrap();
    assert_eq!(result.pages.len(), 1);
    assert_eq!(result.pages[0].page, 257);
    assert_eq!(result.document.pages, 257);
}

#[test]
fn malformed_and_zero_page_documents_fail_explicitly() {
    assert!(PdfOxideBackend.open(b"not a PDF", None).is_err());
    let mut document = Document::load_mem(&fixture(None, false)).unwrap();
    let page = *document.get_pages().get(&1).unwrap();
    let parent = document
        .get_object(page)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    let tree = document
        .get_object_mut(parent)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    tree.set("Kids", Vec::<Object>::new());
    tree.set("Count", 0);
    // Remove the orphan Page object too: upstream recovery can legitimately
    // reconstruct a damaged tree from an otherwise surviving page dictionary.
    document.objects.remove(&page);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    assert!(matches!(
        PdfOxideBackend.open(&bytes, None),
        Err(BackendError::PageRange { count: 0, .. } | BackendError::Malformed(_))
    ));
}
