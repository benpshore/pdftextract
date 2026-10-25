//! Regressions for input alias publication and the source object namespace.
use lopdf::{Document, Object, Stream, dictionary};
use tpe_pdfops::PdfOpsError;
use tpe_pdfops::output::Output;
use tpe_pdfops::paginate::{PaginateOptions, SplitMode, extract_pages, paginate};

fn low_ids(content: u32, font: u32, page: u32) -> Document {
    let mut doc = Document::with_version("1.5");
    doc.objects.insert(
        (content, 0),
        Object::Stream(Stream::new(
            dictionary! {},
            b"BT /F1 12 Tf 72 72 Td (Original text) Tj ET".to_vec(),
        )),
    );
    doc.objects.insert(
        (font, 0),
        Object::Dictionary(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        }),
    );
    doc.objects.insert(
        (page, 0),
        Object::Dictionary(dictionary! {
            "Type" => "Page", "Parent" => (10, 0),
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => (content, 0),
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => (font, 0) } },
        }),
    );
    doc.objects.insert(
        (10, 0),
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![Object::Reference((page, 0))], "Count" => 1,
        }),
    );
    doc.objects.insert(
        (11, 0),
        Object::Dictionary(dictionary! {
            "Type" => "Catalog", "Pages" => (10, 0),
        }),
    );
    doc.trailer.set("Root", (11, 0));
    doc.max_id = 11;
    doc
}

#[test]
fn low_id_content_font_and_page_survive_split_and_duplicate_stamp() {
    for (content, font, page, allocator) in [
        (1, 2, 3, 11),
        (2, 1, 3, 11),
        (2, 3, 1, 11),
        (1, 2, 3, 0),
        (2, 1, 3, 0),
        (2, 3, 1, 0),
    ] {
        let mut source = low_ids(content, font, page);
        // Also reserve real IDs when allocator metadata is stale.
        source.max_id = allocator;
        for stamp in [false, true] {
            let mut out = extract_pages(&source, &[1, 1], stamp, 1).unwrap();
            let mut bytes = Vec::new();
            out.save_to(&mut bytes).unwrap();
            let reparsed = Document::load_mem(&bytes).unwrap();
            let pages = reparsed.get_pages();
            assert_eq!(pages.len(), 2);
            assert_ne!(pages[&1], pages[&2]);
            for id in pages.values() {
                let page = reparsed.get_dictionary(*id).unwrap();
                let parent = page.get(b"Parent").unwrap().as_reference().unwrap();
                assert!(parent.0 > 11);
                assert_eq!(
                    reparsed.get_object(parent).unwrap().type_name().unwrap(),
                    b"Pages"
                );
                let streams = reparsed.get_page_contents(*id);
                assert!(streams.contains(&(content, 0)));
                assert_eq!(
                    reparsed
                        .get_object((content, 0))
                        .unwrap()
                        .as_stream()
                        .unwrap()
                        .content,
                    source
                        .get_object((content, 0))
                        .unwrap()
                        .as_stream()
                        .unwrap()
                        .content
                );
                let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
                let fonts = resources.get(b"Font").unwrap().as_dict().unwrap();
                assert_eq!(fonts.get(b"F1").unwrap().as_reference().unwrap(), (font, 0));
                assert_eq!(
                    reparsed
                        .get_dictionary((font, 0))
                        .unwrap()
                        .get(b"BaseFont")
                        .unwrap()
                        .as_name()
                        .unwrap(),
                    b"Helvetica"
                );
            }
            let text = reparsed.extract_text(&[1, 2]).unwrap();
            assert_eq!(text.matches("Original text").count(), 2);
            if stamp {
                assert_eq!(text.matches("Page 1 of 1").count(), 2);
            }
        }
    }
}

#[test]
fn paginate_and_cli_refuse_forced_hard_links_without_changing_source() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.pdf");
    let mut doc = low_ids(1, 2, 3);
    doc.max_id = 11;
    doc.save(&input).unwrap();
    let before = std::fs::read(&input).unwrap();
    let alias = dir.path().join("source-001.pdf");
    std::fs::hard_link(&input, &alias).unwrap();
    let result = paginate(
        &input,
        &PaginateOptions {
            mode: SplitMode::EveryN(1),
            stamp: false,
            stem: None,
        },
        &Output::new(dir.path()).force(true),
    );
    assert!(matches!(result, Err(PdfOpsError::OutputIsInput(_))));
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_tpe-pdfops"))
        .arg("concat")
        .arg(&input)
        .arg("--out")
        .arg(&alias)
        .arg("--force")
        .status()
        .unwrap();
    assert!(!status.success());
    assert_eq!(std::fs::read(&input).unwrap(), before);
    assert_eq!(std::fs::read(&alias).unwrap(), before);
}
