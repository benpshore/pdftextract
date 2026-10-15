#![cfg(feature = "pdf-oxide")]

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use lopdf::{Document, Object, Stream, dictionary};
use tpe::backend::BackendError;
use tpe::backend::pdf_oxide_supplements::{CleanupMode, Options, add_supplements};
use tpe::pipeline::run_job;
use tpe::schema::{ExtractionResult, Job, sha256_hex};

fn document(contents: &[Vec<u8>]) -> Document {
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let font = doc.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica","Encoding"=>"WinAnsiEncoding"});
    let mut kids = Vec::new();
    for content in contents {
        let content = doc.add_object(Stream::new(dictionary! {}, content.clone()));
        let page = doc.add_object(dictionary! {
            "Type"=>"Page","Parent"=>pages,"Contents"=>content,
            "MediaBox"=>vec![0.into(),0.into(),612.into(),792.into()],
            "Resources"=>dictionary! {"Font"=>dictionary! {"F1"=>font}},
        });
        kids.push(Object::Reference(page));
    }
    doc.objects.insert(
        pages,
        dictionary! {"Type"=>"Pages","Count"=>i64::try_from(kids.len()).unwrap(),"Kids"=>kids}
            .into(),
    );
    let root = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    doc.trailer.set("Root", root);
    doc
}

fn save(mut doc: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

fn capture(name: &str, bytes: &[u8]) {
    if let Some(root) = std::env::var_os("TPE_OXIDE_FIXTURE_CAPTURE") {
        let root = std::path::PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(name), bytes).unwrap();
    }
}

fn baseline(bytes: &[u8]) -> ExtractionResult {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.pdf");
    std::fs::write(&path, bytes).unwrap();
    run_job(&Job {
        path: path.to_string_lossy().into_owned(),
        backend: "pdf-oxide".into(),
        pages: None,
        password: None,
        max_bytes: Some(8 * 1024 * 1024),
        figures_dir: None,
    })
    .unwrap()
}

fn assert_original_views(before: &ExtractionResult, after: &ExtractionResult) {
    assert_eq!(before.document, after.document);
    assert_eq!(before.status, after.status);
    assert_eq!(before.chunks, after.chunks);
    assert_eq!(before.metadata, after.metadata);
    assert_eq!(before.references, after.references);
    assert_eq!(before.citations, after.citations);
    assert_eq!(before.warnings, after.warnings);
    for (a, b) in before.pages.iter().zip(&after.pages) {
        assert_eq!(a.spans, b.spans);
        assert_eq!(a.lines, b.lines);
        assert_eq!(a.text, b.text);
        assert_eq!(a.tables, b.tables);
        assert_eq!(a.figures, b.figures);
        assert_eq!(a.links, b.links);
        assert_eq!(a.warnings, b.warnings);
    }
}

#[test]
fn whitespace_preview_preserves_citations_duplicate_words_and_hard_hyphens() {
    let bytes=save(document(&[b"BT /F1 12 Tf 72 600 Td (had had  well-being  [12]) Tj 0 -30 Td (Page 1) Tj 0 -30 Td ([12]) Tj ET".to_vec()]));
    capture("supplement-whitespace-citations.pdf", &bytes);
    let mut result = baseline(&bytes);
    let before = result.clone();
    add_supplements(
        &mut result,
        &bytes,
        None,
        &Options {
            cleanup_preview: Some(CleanupMode::Whitespace),
            ..Options::default()
        },
    )
    .unwrap();
    assert_original_views(&before, &result);
    let preview = result.pages[0]
        .supplements
        .as_ref()
        .unwrap()
        .cleanup
        .as_ref()
        .unwrap();
    assert_eq!(preview.before_text, before.pages[0].text);
    for literal in ["had had", "well-being", "[12]", "Page 1"] {
        assert!(
            preview.text.contains(literal),
            "lost {literal}: {}",
            preview.text
        );
    }
    assert!(preview.removed_spans.is_empty());
    assert_eq!(preview.threshold, None);
    assert_ne!(result.backend.config_digest, before.backend.config_digest);
}

fn cleanup_pdf(tag_first: bool) -> Vec<u8> {
    let mut content = Vec::new();
    for index in 0..2 {
        let prefix = if tag_first && index == 0 {
            "/Artifact << /Type /Pagination /Subtype /Header >> BDC "
        } else {
            ""
        };
        let suffix = if tag_first && index == 0 { " EMC" } else { "" };
        content.push(format!("{prefix}BT /F1 12 Tf 72 755 Td (Dose every 8 hours [12]) Tj ET{suffix}\nBT /F1 12 Tf 72 500 Td (Dose every 8 hours [12]) Tj 0 -30 Td (Body on page {}) Tj ET",index+1).into_bytes());
    }
    save(document(&content))
}

#[test]
fn header_cleanup_is_a_separate_candidate_and_records_meaningful_false_removals() {
    let bytes = cleanup_pdf(false);
    capture("supplement-repeated-meaningful-header.pdf", &bytes);
    let mut result = baseline(&bytes);
    let before = result.clone();
    add_supplements(
        &mut result,
        &bytes,
        None,
        &Options {
            cleanup_preview: Some(CleanupMode::Headers),
            html: true,
            images: false,
        },
    )
    .unwrap();
    assert_original_views(&before, &result);
    let mut html_only = before.clone();
    add_supplements(
        &mut html_only,
        &bytes,
        None,
        &Options {
            html: true,
            ..Options::default()
        },
    )
    .unwrap();
    for (page, plain_html) in result.pages.iter().zip(&html_only.pages) {
        let supplemental = page.supplements.as_ref().unwrap();
        let preview = supplemental.cleanup.as_ref().unwrap();
        assert!(preview.document_regions_marked > 0);
        assert_eq!(preview.threshold, Some(0.8));
        assert!(
            preview
                .removed_spans
                .iter()
                .any(|span| span.text.contains("Dose every 8 hours [12]")
                    && span.bbox.is_some_and(|bbox| bbox.y0 > 700.0))
        );
        assert!(
            preview.text.contains("Dose every 8 hours [12]"),
            "same-text body occurrence must remain"
        );
        assert!(
            preview
                .warnings
                .iter()
                .any(|message| message.contains("Candidate preview"))
        );
        assert_eq!(
            supplemental.html,
            plain_html.supplements.as_ref().unwrap().html,
            "cleanup masks leaked into HTML parser"
        );
    }
    let mut fresh = before.clone();
    add_supplements(
        &mut fresh,
        &bytes,
        None,
        &Options {
            html: true,
            ..Options::default()
        },
    )
    .unwrap();
    for (a, b) in fresh.pages.iter().zip(&html_only.pages) {
        assert_eq!(a.supplements, b.supplements);
    }
}

#[test]
fn mixed_tagged_untagged_cleanup_reports_partial_coverage_and_preserves_baseline() {
    let bytes = cleanup_pdf(true);
    capture("supplement-mixed-tagged-headers.pdf", &bytes);
    let mut result = baseline(&bytes);
    let before = result.clone();
    add_supplements(
        &mut result,
        &bytes,
        None,
        &Options {
            cleanup_preview: Some(CleanupMode::Headers),
            ..Options::default()
        },
    )
    .unwrap();
    assert_original_views(&before, &result);
    let first = result.pages[0]
        .supplements
        .as_ref()
        .unwrap()
        .cleanup
        .as_ref()
        .unwrap();
    let second = result.pages[1]
        .supplements
        .as_ref()
        .unwrap()
        .cleanup
        .as_ref()
        .unwrap();
    assert!(!first.removed_spans.is_empty());
    assert!(
        second.removed_spans.is_empty(),
        "pinned tagged-path early return changed; reassess the mixed-document warning"
    );
    assert!(second.text.matches("Dose every 8 hours [12]").count() >= 2);
    assert!(
        second
            .warnings
            .iter()
            .any(|warning| warning.contains("Mixed tagged/untagged"))
    );
}

#[test]
fn page_html_has_stable_app_identity_and_no_active_resource_attributes() {
    let mut doc = document(&[
        br"BT /F1 12 Tf 72 600 Td (<script>alert\(1\)</script> & ordinary text) Tj ET".to_vec(),
    ]);
    let page = *doc.get_pages().get(&1).unwrap();
    let action = doc
        .add_object(dictionary! {"S"=>"URI","URI"=>Object::string_literal("javascript:alert(1)")});
    let annotation=doc.add_object(dictionary! {"Type"=>"Annot","Subtype"=>"Link","A"=>action,"Rect"=>vec![72.into(),590.into(),400.into(),620.into()]});
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", vec![Object::Reference(annotation)]);
    let bytes = save(doc);
    capture("supplement-html-literal-markup.pdf", &bytes);
    let mut result = baseline(&bytes);
    let before = result.clone();
    add_supplements(
        &mut result,
        &bytes,
        None,
        &Options {
            html: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert_original_views(&before, &result);
    let html = result.pages[0]
        .supplements
        .as_ref()
        .unwrap()
        .html
        .as_ref()
        .unwrap();
    assert_eq!(html.page_id, format!("pdf-{}-page-1", sha256_hex(&bytes)));
    assert!(html.display_html.starts_with(&format!(
        "<section class=\"pdf-page\" id=\"{}\">",
        html.page_id
    )));
    assert!(html.display_html.ends_with("</section>"));
    assert!(!html.raw_html.is_empty());
    assert!(html.display_html.contains("ordinary text"));
    for forbidden in [
        "<script",
        "<img",
        "<iframe",
        "<svg",
        "<form",
        " href=",
        " src=",
        " style=",
        " onclick=",
        " onerror=",
    ] {
        assert!(
            !html.display_html.to_ascii_lowercase().contains(forbidden),
            "{forbidden}: {}",
            html.display_html
        );
    }
    assert!(html.sanitizer.contains("structural tags only"));
}

fn image_pdf(
    width: u32,
    height: u32,
    color: &str,
    data: Vec<u8>,
    repeat: usize,
    mask: Option<Vec<u8>>,
    filter: Option<&str>,
) -> Vec<u8> {
    let mut content = Vec::new();
    for index in 0..repeat {
        content.extend_from_slice(
            format!("q 16 0 0 16 {} 100 cm /Im1 Do Q\n", 20 + 20 * index).as_bytes(),
        );
    }
    let mut doc = document(&[content]);
    let mut image = dictionary! {"Type"=>"XObject","Subtype"=>"Image","Width"=>i64::from(width),"Height"=>i64::from(height),"BitsPerComponent"=>8,"ColorSpace"=>color};
    if let Some(filter) = filter {
        image.set("Filter", filter);
    }
    if let Some(mask) = mask {
        let mask=doc.add_object(Stream::new(dictionary! {"Type"=>"XObject","Subtype"=>"Image","Width"=>i64::from(width),"Height"=>i64::from(height),"BitsPerComponent"=>8,"ColorSpace"=>"DeviceGray"},mask));
        image.set("SMask", mask);
    }
    let image = doc.add_object(Stream::new(image, data));
    let page = *doc.get_pages().get(&1).unwrap();
    let resources = doc
        .get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .get_mut(b"Resources")
        .unwrap()
        .as_dict_mut()
        .unwrap();
    resources.set("XObject", dictionary! {"Im1"=>image});
    save(doc)
}

fn png_pixels(bytes: &[u8]) -> (Vec<u8>, Option<Vec<u8>>) {
    // Decode the produced PNG using the upstream writer's independent image
    // decoder. Its returned Flate bytes use PNG predictor 15 with None rows.
    let decoded = pdf_oxide::writer::ImageData::from_png(bytes).unwrap();
    assert_eq!((decoded.width, decoded.height), (8, 8));
    let inflate = |bytes: &[u8]| {
        let mut output = Vec::new();
        flate2::read::ZlibDecoder::new(bytes)
            .take(4096)
            .read_to_end(&mut output)
            .unwrap();
        output
    };
    let remove_none = |rows: Vec<u8>, width: usize| {
        let mut pixels = Vec::new();
        for row in rows.chunks_exact(width + 1) {
            assert_eq!(row[0], 0);
            pixels.extend_from_slice(&row[1..]);
        }
        assert_eq!(pixels.len(), width * 8);
        pixels
    };
    let width = 8 * usize::from(decoded.color_space.components());
    let pixels = remove_none(inflate(&decoded.data), width);
    let alpha = decoded.soft_mask.map(|mask| remove_none(inflate(&mask), 8));
    (pixels, alpha)
}

#[test]
fn images_retain_png_pixels_and_repeated_paint_provenance_in_memory() {
    let bytes = image_pdf(8, 8, "DeviceRGB", [255, 0, 0].repeat(64), 2, None, None);
    capture("supplement-repeated-image.pdf", &bytes);
    let mut result = baseline(&bytes);
    let before = result.clone();
    add_supplements(
        &mut result,
        &bytes,
        None,
        &Options {
            images: true,
            html: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert_original_views(&before, &result);
    let supplements = result.pages[0].supplements.as_ref().unwrap();
    assert_eq!(supplements.images.len(), 2);
    assert!(
        !supplements
            .html
            .as_ref()
            .unwrap()
            .raw_html
            .contains("data:image")
    );
    assert!(
        !supplements
            .html
            .as_ref()
            .unwrap()
            .display_html
            .contains("<img")
    );
    assert_ne!(supplements.images[0].bbox, supplements.images[1].bbox);
    assert_ne!(
        supplements.images[0].paint_order,
        supplements.images[1].paint_order
    );
    for image in &supplements.images {
        assert_eq!((image.width, image.height), (8, 8));
        assert_eq!(image.mime, "image/png");
        assert_eq!(image.sha256, sha256_hex(&image.bytes));
        assert!(!image.inline);
        assert_eq!(image.source_compressed_bytes, 192);
        let (pixels, alpha) = png_pixels(&image.bytes);
        assert_eq!(pixels, [255, 0, 0].repeat(64));
        assert_eq!(alpha, None);
    }
}

#[test]
fn soft_mask_extraction_fails_explicitly_instead_of_returning_opaque_pixels() {
    // Pinned PdfImage has no alpha representation. Until faithful bounded
    // composition is supported, this capability must reject it atomically.
    let bytes = image_pdf(
        8,
        8,
        "DeviceRGB",
        [255, 0, 0].repeat(64),
        1,
        Some(vec![128; 64]),
        None,
    );
    capture("supplement-repeated-image-soft-mask.pdf", &bytes);
    let mut result = baseline(&bytes);
    let before = result.clone();
    let error = add_supplements(
        &mut result,
        &bytes,
        None,
        &Options {
            images: true,
            html: true,
            ..Options::default()
        },
    )
    .unwrap_err();
    assert!(matches!(error, BackendError::Unsupported(_)));
    assert!(error.to_string().to_ascii_lowercase().contains("mask"));
    assert_eq!(result, before);
}

#[test]
fn inline_and_cmyk_images_use_real_memory_extraction() {
    let mut content = b"q 16 0 0 16 20 100 cm BI /W 8 /H 8 /BPC 8 /CS /RGB ID\n".to_vec();
    content.extend([0, 255, 0].repeat(64));
    content.extend_from_slice(b"\nEI Q\n");
    let inline = save(document(&[content]));
    capture("supplement-inline-rgb.pdf", &inline);
    let mut result = baseline(&inline);
    add_supplements(
        &mut result,
        &inline,
        None,
        &Options {
            images: true,
            ..Options::default()
        },
    )
    .unwrap();
    let images = &result.pages[0].supplements.as_ref().unwrap().images;
    assert_eq!(images.len(), 1);
    assert!(images[0].inline);
    assert_eq!(png_pixels(&images[0].bytes).0, [0, 255, 0].repeat(64));
    let cmyk = image_pdf(8, 8, "DeviceCMYK", [255, 0, 0, 0].repeat(64), 1, None, None);
    capture("supplement-device-cmyk.pdf", &cmyk);
    let mut result = baseline(&cmyk);
    add_supplements(
        &mut result,
        &cmyk,
        None,
        &Options {
            images: true,
            ..Options::default()
        },
    )
    .unwrap();
    let images = &result.pages[0].supplements.as_ref().unwrap().images;
    assert_eq!(images.len(), 1);
    let pixels = png_pixels(&images[0].bytes).0;
    assert_eq!(pixels.len(), 192);
    assert!(
        pixels
            .chunks_exact(3)
            .all(|pixel| pixel[0] < pixel[1] && pixel[0] < pixel[2]),
        "cyan should retain green/blue dominance; no color-fidelity claim"
    );
}

#[test]
fn image_empty_and_decode_error_are_distinct_and_supplements_are_atomic() {
    let empty = save(document(&[Vec::new()]));
    let mut result = baseline(&empty);
    add_supplements(
        &mut result,
        &empty,
        None,
        &Options {
            images: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(
        result.pages[0]
            .supplements
            .as_ref()
            .unwrap()
            .images
            .is_empty()
    );
    assert!(
        result.pages[0]
            .diagnostics
            .iter()
            .any(
                |diagnostic| diagnostic.category == "completeness_unverified"
                    && diagnostic.page == Some(1)
            )
    );
    let damaged = image_pdf(
        8,
        8,
        "DeviceRGB",
        b"not a JPEG".to_vec(),
        1,
        None,
        Some("DCTDecode"),
    );
    capture("supplement-malformed-jpeg.pdf", &damaged);
    let mut result = baseline(&damaged);
    let before = result.clone();
    let error = add_supplements(
        &mut result,
        &damaged,
        None,
        &Options {
            images: true,
            html: true,
            cleanup_preview: Some(CleanupMode::Whitespace),
        },
    )
    .unwrap_err();
    assert!(matches!(error, BackendError::Unsupported(_)));
    assert_eq!(
        result, before,
        "a late image failure must not publish earlier cleanup/HTML views"
    );
    let mut result = baseline(&empty);
    let before = result.clone();
    assert!(
        add_supplements(
            &mut result,
            &damaged,
            None,
            &Options {
                html: true,
                ..Options::default()
            }
        )
        .is_err()
    );
    assert_eq!(result, before);
}

fn assert_worker_rejects(bytes: &[u8], expected: &str) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("input.pdf");
    let stdout = root.path().join("stdout.json");
    let stderr = root.path().join("stderr.txt");
    std::fs::write(&path, bytes).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_tpe"))
        .args([
            "extract",
            "--backend",
            "pdf-oxide",
            "--pdf-oxide-images",
            "--json",
            "--timeout-ms",
            "5000",
            "--max-memory-growth-mib",
            "256",
            "--max-output-bytes",
            "1048576",
        ])
        .arg("--db")
        .arg(root.path().join("ledger.sqlite"))
        .arg("--out")
        .arg(root.path().join("out"))
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&stdout).unwrap())
        .stderr(std::fs::File::create(&stderr).unwrap())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("bounded image worker did not finish");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(!status.success());
    let output = std::fs::read_to_string(&stdout).unwrap();
    let diagnostic = std::fs::read_to_string(&stderr).unwrap();
    assert!(output.len() < 1_048_576 && diagnostic.len() < 1_048_576);
    assert!(
        format!("{output}\n{diagnostic}").contains(expected),
        "expected {expected}: {output}\n{diagnostic}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let out = root.path().join("out");
    assert!(!out.exists() || std::fs::read_dir(out).unwrap().next().is_none());
    if root.path().join("ledger.sqlite").exists() {
        assert_eq!(
            tpe::ledger::Ledger::open(&root.path().join("ledger.sqlite"))
                .unwrap()
                .stats()
                .unwrap()
                .runs,
            0
        );
    }
}

#[test]
fn image_limits_fail_in_disposable_workers_without_publication() {
    let pixels = image_pdf(1001, 1000, "DeviceRGB", vec![0; 3003000], 1, None, None);
    assert_worker_rejects(&pixels, "image pixels exceed");
    let count = image_pdf(8, 8, "DeviceRGB", [255, 0, 0].repeat(64), 33, None, None);
    assert_worker_rejects(&count, "images exceed 32/page");
    let compressed = image_pdf(
        8,
        8,
        "DeviceRGB",
        vec![0; 2 * 1024 * 1024 + 1],
        1,
        None,
        None,
    );
    assert_worker_rejects(&compressed, "compressed image exceeds");
}

#[test]
fn cleanup_whole_document_scan_limit_is_atomic_for_a_small_selected_range() {
    let bytes = save(document(&vec![
        b"BT /F1 12 Tf 72 500 Td (Small body) Tj ET"
            .to_vec();
        65
    ]));
    let mut result = baseline(&bytes);
    result.pages.truncate(1);
    let before = result.clone();
    let error = add_supplements(
        &mut result,
        &bytes,
        None,
        &Options {
            cleanup_preview: Some(CleanupMode::Headers),
            ..Options::default()
        },
    )
    .unwrap_err();
    assert!(matches!(error, BackendError::Limit(_)));
    assert_eq!(result, before);
}
