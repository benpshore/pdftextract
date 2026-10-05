//! Every operation on lopdf-built fixtures, each output re-parsed with lopdf.

// Boxes are written as exact integers and read back as such; exact
// comparison is the point of these assertions.
#![allow(clippy::float_cmp)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lopdf::content::{Content, Operation};
use lopdf::encryption::crypt_filters::{Aes128CryptFilter, CryptFilter};
use lopdf::{
    Document, EncryptionState, EncryptionVersion, Object, Permissions, Stream, dictionary,
};
use tpe_pdfops::clean::{CleanOptions, clean};
use tpe_pdfops::concat::{ConcatOptions, concatenate};
use tpe_pdfops::imagepdf::ImageEncoding;
use tpe_pdfops::inspect::inspect;
use tpe_pdfops::letter::normalise_to_letter;
use tpe_pdfops::linearize::{REASON, linearize};
use tpe_pdfops::output::{Output, ensure_not_input, save_document};
use tpe_pdfops::paginate::{PaginateOptions, STAMP_FONT, SplitMode, paginate};
use tpe_pdfops::reorient::{Orientation, reorient};
use tpe_pdfops::unlock::remove_password;
use tpe_pdfops::{PageSelection, PdfOpsError};

/// One fixture page: size, `/Rotate`, shown text and an optional text matrix.
struct PageSpec {
    width: i32,
    height: i32,
    rotate: Option<i64>,
    text: &'static str,
    tm: Option<[f32; 6]>,
}

fn page(width: i32, height: i32, text: &'static str) -> PageSpec {
    PageSpec {
        width,
        height,
        rotate: None,
        text,
        tm: None,
    }
}

fn build(pages: &[PageSpec]) -> Document {
    let mut doc = Document::with_version("1.5");
    let tree_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let mut kids = Vec::new();
    for spec in pages {
        let mut operations = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 24.into()]),
        ];
        if let Some(tm) = spec.tm {
            operations.push(Operation::new(
                "Tm",
                tm.iter().map(|v| (*v).into()).collect(),
            ));
        } else {
            operations.push(Operation::new(
                "Td",
                vec![72.into(), (spec.height - 92).into()],
            ));
        }
        if !spec.text.is_empty() {
            operations.push(Operation::new(
                "Tj",
                vec![Object::string_literal(spec.text)],
            ));
        }
        operations.push(Operation::new("ET", vec![]));
        let content = Content { operations }.encode().unwrap();
        let content_id = doc.add_object(Stream::new(dictionary! {}, content));
        let mut dict = dictionary! {
            "Type" => "Page",
            "Parent" => tree_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), spec.width.into(), spec.height.into()],
        };
        if let Some(rotate) = spec.rotate {
            dict.set("Rotate", rotate);
        }
        let page_id = doc.add_object(dict);
        kids.push(Object::Reference(page_id));
    }
    let count = i64::try_from(kids.len()).unwrap();
    doc.objects.insert(
        tree_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => tree_id });
    doc.trailer.set("Root", catalog_id);
    doc
}

fn write(dir: &Path, name: &str, doc: &mut Document) -> PathBuf {
    let path = dir.join(name);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    std::fs::write(&path, bytes).unwrap();
    path
}

fn five_pages(dir: &Path) -> PathBuf {
    let mut doc = build(&[
        page(612, 792, "Page 1"),
        page(595, 842, "Page 2"),
        page(400, 300, "Page 3"),
        page(612, 792, "Page 4"),
        page(612, 792, "Page 5"),
    ]);
    write(dir, "five.pdf", &mut doc)
}

fn page_text(doc: &Document, number: u32) -> Vec<u8> {
    let id = doc.get_pages()[&number];
    doc.get_page_content(id)
}

#[test]
fn concat_keeps_page_order_boxes_and_adds_outlines() {
    let dir = tempfile::tempdir().unwrap();
    let a = five_pages(dir.path());
    let b = write(
        dir.path(),
        "two.pdf",
        &mut build(&[page(200, 100, "B1"), page(300, 150, "B2")]),
    );
    let before = (std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());

    let out = dir.path().join("merged.pdf");
    let mut doc = concatenate(&[&a, &b], &ConcatOptions { outlines: true }).unwrap();
    save_document(&mut doc, &Output::new(&out)).unwrap();

    let summary = inspect(&out).unwrap();
    assert_eq!(summary.page_count, 7);
    assert!(!summary.encrypted);
    let widths: Vec<f64> = summary.pages.iter().map(|p| p.media_box[2]).collect();
    assert_eq!(
        widths,
        vec![612.0, 595.0, 400.0, 612.0, 612.0, 200.0, 300.0]
    );
    let reparsed = Document::load(&out).unwrap();
    assert!(page_text(&reparsed, 6).windows(2).any(|w| w == b"B1"));
    let outlines = reparsed
        .catalog()
        .unwrap()
        .get(b"Outlines")
        .unwrap()
        .as_reference()
        .unwrap();
    let outlines = reparsed.get_dictionary(outlines).unwrap();
    assert_eq!(outlines.get(b"Count").unwrap().as_i64().unwrap(), 2);
    let first = reparsed
        .get_dictionary(outlines.get(b"First").unwrap().as_reference().unwrap())
        .unwrap();
    assert_eq!(first.get(b"Title").unwrap().as_str().unwrap(), b"five");
    let dest_page = first.get(b"Dest").unwrap().as_array().unwrap()[0]
        .as_reference()
        .unwrap();
    assert_eq!(reparsed.get_pages()[&1], dest_page);

    assert_eq!(
        before,
        (std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap()),
        "inputs untouched"
    );
}

#[test]
fn paginate_by_ranges_and_by_count_with_stamps() {
    let dir = tempfile::tempdir().unwrap();
    let input = five_pages(dir.path());
    let before = std::fs::read(&input).unwrap();

    let ranges = PaginateOptions {
        mode: SplitMode::Ranges(vec![
            PageSelection::parse("1-2").unwrap(),
            PageSelection::parse("4,3").unwrap(),
        ]),
        stamp: false,
        stem: Some("part".into()),
    };
    let out_dir = dir.path().join("ranges");
    let written = paginate(&input, &ranges, &Output::new(&out_dir)).unwrap();
    assert_eq!(
        written,
        vec![out_dir.join("part-001.pdf"), out_dir.join("part-002.pdf")]
    );
    let first = inspect(&written[0]).unwrap();
    assert_eq!(first.page_count, 2);
    assert_eq!(first.pages[1].media_box, [0.0, 0.0, 595.0, 842.0]);
    let second = inspect(&written[1]).unwrap();
    assert_eq!(second.page_count, 2);
    assert_eq!(second.pages[0].media_box[2], 612.0);
    assert_eq!(second.pages[1].media_box, [0.0, 0.0, 400.0, 300.0]);
    let reparsed = Document::load(&written[1]).unwrap();
    assert!(page_text(&reparsed, 2).windows(6).any(|w| w == b"Page 3"));

    let every = PaginateOptions {
        mode: SplitMode::EveryN(2),
        stamp: true,
        stem: None,
    };
    let out_dir = dir.path().join("every");
    let written = paginate(&input, &every, &Output::new(&out_dir)).unwrap();
    assert_eq!(written.len(), 3);
    let counts: Vec<usize> = written
        .iter()
        .map(|p| inspect(p).unwrap().page_count)
        .collect();
    assert_eq!(counts, vec![2, 2, 1]);
    assert!(written[0].ends_with("five-001.pdf"));
    let last = Document::load(&written[2]).unwrap();
    let content = page_text(&last, 1);
    assert!(
        content.windows(11).any(|w| w == b"Page 5 of 5"),
        "stamp present"
    );
    assert!(
        content.windows(6).any(|w| w == b"Page 5"),
        "original content kept"
    );
    let page_id = last.get_pages()[&1];
    let fonts = last.get_page_fonts(page_id).unwrap();
    assert!(fonts.contains_key(STAMP_FONT.as_bytes()));
    assert!(fonts.contains_key(&b"F1"[..]));

    assert_eq!(before, std::fs::read(&input).unwrap(), "input untouched");
}

#[test]
fn reorient_explicit_rotation_respects_selection() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = build(&[
        page(612, 792, "a"),
        page(612, 792, "b"),
        page(612, 792, "c"),
    ]);
    let input = write(dir.path(), "three.pdf", &mut doc);
    let selection = PageSelection::parse("2-3").unwrap();
    let (mut rotated, outcomes) =
        reorient(&input, Orientation::Rotate(270), Some(&selection)).unwrap();
    let out = dir.path().join("rotated.pdf");
    save_document(&mut rotated, &Output::new(&out)).unwrap();
    let summary = inspect(&out).unwrap();
    let rotations: Vec<i64> = summary.pages.iter().map(|p| p.rotate).collect();
    assert_eq!(rotations, vec![0, 270, 270]);
    assert_eq!(outcomes[0].note.as_deref(), Some("not selected"));

    let (mut again, _) = reorient(&out, Orientation::Rotate(180), None).unwrap();
    let out2 = dir.path().join("rotated2.pdf");
    save_document(&mut again, &Output::new(&out2)).unwrap();
    let rotations: Vec<i64> = inspect(&out2)
        .unwrap()
        .pages
        .iter()
        .map(|p| p.rotate)
        .collect();
    assert_eq!(rotations, vec![180, 90, 90]);
    assert!(matches!(
        reorient(&out, Orientation::Rotate(45), None),
        Err(PdfOpsError::Invalid(_))
    ));
}

#[test]
fn reorient_auto_uses_the_dominant_text_matrix() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = build(&[
        PageSpec {
            tm: Some([1.0, 0.0, 0.0, 1.0, 72.0, 700.0]),
            ..page(612, 792, "upright")
        },
        PageSpec {
            tm: Some([0.0, 1.0, -1.0, 0.0, 500.0, 72.0]),
            ..page(612, 792, "runs up the page")
        },
        PageSpec {
            tm: Some([-1.0, 0.0, 0.0, -1.0, 500.0, 700.0]),
            rotate: Some(90),
            ..page(612, 792, "upside down")
        },
        page(612, 792, ""),
    ]);
    let input = write(dir.path(), "auto.pdf", &mut doc);
    let (mut fixed, outcomes) = reorient(&input, Orientation::Auto, None).unwrap();
    let out = dir.path().join("auto-out.pdf");
    save_document(&mut fixed, &Output::new(&out)).unwrap();
    let rotations: Vec<i64> = inspect(&out)
        .unwrap()
        .pages
        .iter()
        .map(|p| p.rotate)
        .collect();
    assert_eq!(rotations, vec![0, 90, 180, 0]);
    assert_eq!(outcomes[3].note.as_deref(), Some("no text on the page"));
    assert_eq!(outcomes[2].before, 90);
}

#[test]
fn letter_scales_and_centres_each_page() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = build(&[
        page(595, 842, "A4"),
        PageSpec {
            rotate: Some(90),
            ..page(400, 300, "landscape via rotate")
        },
        page(612, 792, "already letter"),
    ]);
    let input = write(dir.path(), "sizes.pdf", &mut doc);
    let before = std::fs::read(&input).unwrap();
    let (mut fitted, fits) = normalise_to_letter(&input, None).unwrap();
    let out = dir.path().join("letter.pdf");
    save_document(&mut fitted, &Output::new(&out)).unwrap();

    let summary = inspect(&out).unwrap();
    assert_eq!(summary.page_count, 3);
    assert_eq!(summary.pages[0].media_box, [0.0, 0.0, 612.0, 792.0]);
    assert_eq!(summary.pages[0].crop_box, Some([0.0, 0.0, 612.0, 792.0]));
    // Rotated page: the box is landscape Letter so the display is portrait.
    assert_eq!(summary.pages[1].media_box, [0.0, 0.0, 792.0, 612.0]);
    assert_eq!(summary.pages[1].rotate, 90);
    assert_eq!(summary.pages[2].media_box, [0.0, 0.0, 612.0, 792.0]);

    // A4 -> Letter: limited by height, centred horizontally.
    let a4 = &fits[0];
    assert!((a4.scale - 792.0 / 842.0).abs() < 1e-9);
    assert!((a4.offset.0 - (612.0 - a4.scale * 595.0) / 2.0).abs() < 1e-9);
    assert!(a4.offset.1.abs() < 1e-9);
    assert!((fits[2].scale - 1.0).abs() < 1e-9);

    let reparsed = Document::load(&out).unwrap();
    let page_id = reparsed.get_pages()[&1];
    let streams = reparsed.get_page_contents(page_id);
    assert_eq!(streams.len(), 3, "prefix, original, suffix");
    let prefix = reparsed
        .get_object(streams[0])
        .unwrap()
        .as_stream()
        .unwrap();
    assert!(prefix.content.starts_with(b"q ") && prefix.content.trim_ascii_end().ends_with(b"cm"));
    let suffix = reparsed
        .get_object(streams[2])
        .unwrap()
        .as_stream()
        .unwrap();
    assert_eq!(suffix.content.trim_ascii(), b"Q");
    assert!(page_text(&reparsed, 1).windows(2).any(|w| w == b"A4"));
    assert_eq!(before, std::fs::read(&input).unwrap(), "input untouched");
}

fn encrypted_fixture(dir: &Path, name: &str, aes: bool) -> PathBuf {
    let mut doc = build(&[page(612, 792, "Secret page")]);
    doc.trailer.set(
        "ID",
        vec![
            Object::string_literal("0123456789abcdef"),
            Object::string_literal("0123456789abcdef"),
        ],
    );
    let permissions = Permissions::all();
    let state = if aes {
        let filter: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);
        EncryptionState::try_from(EncryptionVersion::V4 {
            document: &doc,
            encrypt_metadata: true,
            crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
            stream_filter: b"StdCF".to_vec(),
            string_filter: b"StdCF".to_vec(),
            owner_password: "owner-secret",
            user_password: "user-secret",
            permissions,
        })
        .unwrap()
    } else {
        EncryptionState::try_from(EncryptionVersion::V2 {
            document: &doc,
            owner_password: "owner-secret",
            user_password: "user-secret",
            key_length: 128,
            permissions,
        })
        .unwrap()
    };
    doc.encrypt(&state).unwrap();
    write(dir, name, &mut doc)
}

#[test]
fn unlock_removes_the_open_password_once_given() {
    let dir = tempfile::tempdir().unwrap();
    for (name, aes) in [("rc4.pdf", false), ("aes.pdf", true)] {
        let input = encrypted_fixture(dir.path(), name, aes);
        let before = std::fs::read(&input).unwrap();
        assert!(
            inspect(&input).unwrap().encrypted,
            "{name} fixture is encrypted"
        );

        let mut doc = remove_password(&input, "user-secret").unwrap();
        let out = dir.path().join(format!("open-{name}"));
        save_document(&mut doc, &Output::new(&out)).unwrap();
        let summary = inspect(&out).unwrap();
        assert!(!summary.encrypted, "{name}: output has no /Encrypt");
        assert_eq!(summary.page_count, 1);
        let reparsed = Document::load(&out).unwrap();
        assert!(
            page_text(&reparsed, 1)
                .windows(11)
                .any(|w| w == b"Secret page"),
            "{name}: content readable"
        );

        let mut owner = remove_password(&input, "owner-secret").unwrap();
        save_document(
            &mut owner,
            &Output::new(dir.path().join(format!("owner-{name}"))),
        )
        .unwrap();

        match remove_password(&input, "wrong") {
            Err(PdfOpsError::Invalid(msg)) => assert!(msg.contains("password rejected"), "{msg}"),
            other => panic!("{name}: wrong password accepted: {other:?}"),
        }
        assert_eq!(
            before,
            std::fs::read(&input).unwrap(),
            "{name}: input untouched"
        );
    }
    let plain = five_pages(dir.path());
    match remove_password(&plain, "anything") {
        Err(PdfOpsError::Invalid(msg)) => assert!(msg.contains("not encrypted")),
        other => panic!("plain input accepted: {other:?}"),
    }
    // Other operations refuse encrypted inputs instead of guessing.
    let locked = encrypted_fixture(dir.path(), "locked.pdf", false);
    match concatenate(&[&locked], &ConcatOptions::default()) {
        Err(PdfOpsError::Invalid(msg)) => assert!(msg.contains("unlock")),
        other => panic!("encrypted input accepted: {other:?}"),
    }
}

#[test]
fn linearize_is_an_explicit_unsupported_result() {
    let dir = tempfile::tempdir().unwrap();
    let input = five_pages(dir.path());
    match linearize(&input) {
        Err(err) => {
            assert!(err.is_unsupported(), "{err}");
            assert!(err.to_string().contains(REASON));
        }
        Ok(()) => panic!("linearize claimed success"),
    }
    assert!(!inspect(&input).unwrap().linearized);
}

#[test]
fn outputs_are_new_files_and_never_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let input = five_pages(dir.path());
    let out = dir.path().join("copy.pdf");
    let mut doc = concatenate(&[&input], &ConcatOptions::default()).unwrap();
    save_document(&mut doc, &Output::new(&out)).unwrap();
    match save_document(&mut doc, &Output::new(&out)) {
        Err(PdfOpsError::OutputExists(path)) => assert_eq!(path, out),
        other => panic!("{other:?}"),
    }
    save_document(&mut doc, &Output::new(&out).force(true)).unwrap();
    assert_eq!(inspect(&out).unwrap().page_count, 5);
    match ensure_not_input(&input, &[&input]) {
        Err(PdfOpsError::OutputIsInput(_)) => {}
        other => panic!("{other:?}"),
    }
    let relative = Path::new("five.pdf");
    assert!(ensure_not_input(&dir.path().join(relative), &[&input]).is_err());
    assert!(ensure_not_input(&out, &[&input]).is_ok());
}

#[test]
fn cli_runs_concat_and_reports_unsupported_linearize() {
    let dir = tempfile::tempdir().unwrap();
    let input = five_pages(dir.path());
    let out = dir.path().join("cli.pdf");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_tpe-pdfops"))
        .args([
            "concat",
            input.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(inspect(&out).unwrap().page_count, 5);

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_tpe-pdfops"))
        .args([
            "linearize",
            input.to_str().unwrap(),
            "--out",
            dir.path().join("lin.pdf").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported"));
    assert!(!dir.path().join("lin.pdf").exists());

    let refused = std::process::Command::new(env!("CARGO_BIN_EXE_tpe-pdfops"))
        .args([
            "concat",
            input.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--force"));
}

fn clean_options() -> CleanOptions {
    CleanOptions {
        water_stain: true,
        dewarp: true,
        deskew: true,
        dpi: 72,
        encoding: ImageEncoding::Jpeg { quality: 80 },
        password: None,
    }
}

#[cfg(feature = "pdfium")]
#[test]
fn clean_renders_with_pdfium_and_writes_image_pages() {
    if std::env::var_os("PDFIUM_DYNAMIC_LIB_PATH").is_none() {
        eprintln!("skipped: set PDFIUM_DYNAMIC_LIB_PATH to run the raster clean-up test");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut doc = build(&[page(612, 792, "Rendered text"), page(400, 300, "Small")]);
    let input = write(dir.path(), "render.pdf", &mut doc);
    let before = std::fs::read(&input).unwrap();
    for (name, encoding, filter) in [
        (
            "jpeg.pdf",
            ImageEncoding::Jpeg { quality: 80 },
            &b"DCTDecode"[..],
        ),
        ("flate.pdf", ImageEncoding::Flate, &b"FlateDecode"[..]),
    ] {
        let options = CleanOptions {
            encoding,
            ..clean_options()
        };
        let (mut cleaned, report) = clean(&input, &options).unwrap();
        assert_eq!(report.len(), 2);
        assert!(report[0].skew_degrees.unwrap().abs() <= 0.5, "{report:?}");
        let out = dir.path().join(name);
        save_document(&mut cleaned, &Output::new(&out)).unwrap();
        let summary = inspect(&out).unwrap();
        assert_eq!(summary.page_count, 2);
        assert!(
            (summary.pages[0].media_box[2] - 612.0).abs() <= 1.0,
            "{summary:?}"
        );
        assert!(
            (summary.pages[1].media_box[3] - 300.0).abs() <= 1.0,
            "{summary:?}"
        );
        let reparsed = Document::load(&out).unwrap();
        let page_id = reparsed.get_pages()[&1];
        let images = reparsed.get_page_images(page_id).unwrap();
        assert_eq!(images.len(), 1);
        let expected = vec![String::from_utf8_lossy(filter).into_owned()];
        assert_eq!(images[0].filters.as_deref(), Some(&expected[..]), "{name}");
        assert_eq!(images[0].width, 612);
    }
    assert_eq!(before, std::fs::read(&input).unwrap(), "input untouched");
}

#[cfg(not(feature = "pdfium"))]
#[test]
fn clean_is_unsupported_without_the_pdfium_feature() {
    let dir = tempfile::tempdir().unwrap();
    let input = five_pages(dir.path());
    match clean(&input, &clean_options()) {
        Err(err) => {
            assert!(err.is_unsupported(), "{err}");
            assert!(err.to_string().contains("--features pdfium"), "{err}");
        }
        Ok(_) => panic!("clean claimed success without pdfium"),
    }
}

#[test]
fn clean_needs_at_least_one_operation() {
    let dir = tempfile::tempdir().unwrap();
    let input = five_pages(dir.path());
    let options = CleanOptions {
        water_stain: false,
        dewarp: false,
        deskew: false,
        ..clean_options()
    };
    assert!(matches!(
        clean(&input, &options),
        Err(PdfOpsError::Invalid(_))
    ));
}
