//! Offline backend comparison harness.
//!
//! Runs every backend compiled into this build over the committed fixture
//! PDFs and a set of synthetic layout fixtures, then prints, per fixture and
//! backend pair, a word-level similarity score and a line diff of the final
//! page text. The score is `2 * LCS(words) / (words_a + words_b)` over the
//! whitespace-normalised page text, so `1.000` means identical words in the
//! same order. Run it with output to inspect the diffs:
//!
//! ```sh
//! cargo test --test engine_compare --features pdf-oxide,pdf-extract -- --nocapture
//! ```
//!
//! Set `TPE_ENGINE_COMPARE_DIR` to a directory of extra PDFs to compare them
//! as well (printed only; no assertion depends on uncommitted files).
//! Poppler and `MuPDF` join the comparison only when their provider and
//! runtime libraries are configured (see `docs/NATIVE_FALLBACK.md`);
//! otherwise the table names the missing configuration.
//!
//! The assertions pin the cross-backend agreement that the fixtures permit
//! and the text-quality behaviours the engine guarantees on them: ligature
//! expansion, line-end de-hyphenation, superscript citation markers,
//! running-head and page-number removal, and two-column reading order.

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use lopdf::content::{Content, Operation};
use lopdf::{Document, Encoding, Object, ObjectId, Stream, dictionary};
use tpe::backend;
use tpe::pipeline::run_job;
use tpe::schema::Job;

// ---------------------------------------------------------------------------
// Fixture construction
// ---------------------------------------------------------------------------

const PAGE_WIDTH: i64 = 612;
const PAGE_HEIGHT: i64 = 792;

/// One text run: font resource name, size, baseline origin and the bytes to
/// show (already encoded for the font).
struct Run {
    font: &'static str,
    size: f32,
    x: f32,
    y: f32,
    bytes: Vec<u8>,
}

fn win_ansi(text: &str) -> Vec<u8> {
    Document::encode_text(&Encoding::SimpleEncoding(b"WinAnsiEncoding"), text)
}

fn run(font: &'static str, size: f32, x: f32, y: f32, text: &str) -> Run {
    Run {
        font,
        size,
        x,
        y,
        bytes: win_ansi(text),
    }
}

fn raw(font: &'static str, size: f32, x: f32, y: f32, bytes: &[u8]) -> Run {
    Run {
        font,
        size,
        x,
        y,
        bytes: bytes.to_vec(),
    }
}

fn page_ops(runs: &[Run]) -> Vec<Operation> {
    let mut ops = Vec::new();
    for r in runs {
        ops.push(Operation::new("BT", vec![]));
        ops.push(Operation::new(
            "Tf",
            vec![
                Object::Name(r.font.as_bytes().to_vec()),
                Object::Real(r.size),
            ],
        ));
        ops.push(Operation::new(
            "Td",
            vec![Object::Real(r.x), Object::Real(r.y)],
        ));
        ops.push(Operation::new(
            "Tj",
            vec![Object::string_literal(r.bytes.clone())],
        ));
        ops.push(Operation::new("ET", vec![]));
    }
    ops
}

fn standard_font(doc: &mut Document, base_font: &str) -> ObjectId {
    doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => Object::Name(base_font.as_bytes().to_vec()),
        "Encoding" => "WinAnsiEncoding",
    })
}

/// Helvetica with a `Differences` array that maps codes 128..=132 to the
/// `fi`, `fl`, `ffi`, `ffl` and `ff` ligature glyph names.
fn ligature_font(doc: &mut Document) -> ObjectId {
    doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => dictionary! {
            "Type" => "Encoding",
            "BaseEncoding" => "WinAnsiEncoding",
            "Differences" => vec![
                Object::Integer(128),
                Object::Name(b"fi".to_vec()),
                Object::Name(b"fl".to_vec()),
                Object::Name(b"ffi".to_vec()),
                Object::Name(b"ffl".to_vec()),
                Object::Name(b"ff".to_vec()),
            ],
        },
    })
}

/// Helvetica whose `ToUnicode` map sends codes 128 and 129 to the
/// presentation forms U+FB01 (`ﬁ`) and U+FB02 (`ﬂ`), as typeset papers do.
fn to_unicode_ligature_font(doc: &mut Document) -> ObjectId {
    let cmap = doc.add_object(Stream::new(
        dictionary! {},
        b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
        /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
        /CMapName /Ligatures def\n/CMapType 2 def\n\
        1 begincodespacerange\n<00><FF>\nendcodespacerange\n\
        2 beginbfchar\n<80><FB01>\n<81><FB02>\nendbfchar\n\
        1 beginbfrange\n<20><7E><0020>\nendbfrange\n\
        endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n"
            .to_vec(),
    ));
    doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => dictionary! {
            "Type" => "Encoding",
            "BaseEncoding" => "WinAnsiEncoding",
            "Differences" => vec![
                Object::Integer(128),
                Object::Name(b"fi".to_vec()),
                Object::Name(b"fl".to_vec()),
            ],
        },
        "ToUnicode" => cmap,
    })
}

/// Build a PDF whose pages each draw the given runs with the font resources
/// `F1` (Helvetica), `F2` (Helvetica-Bold), `F3` (ligature glyph names) and
/// `F4` (ligature `ToUnicode` presentation forms).
fn build_pdf(pages: &[Vec<Run>]) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_root_id = doc.new_object_id();
    let regular = standard_font(&mut doc, "Helvetica");
    let bold = standard_font(&mut doc, "Helvetica-Bold");
    let ligatures = ligature_font(&mut doc);
    let to_unicode = to_unicode_ligature_font(&mut doc);
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! {
            "F1" => regular,
            "F2" => bold,
            "F3" => ligatures,
            "F4" => to_unicode,
        },
    });
    let media_box = Object::Array(vec![
        Object::Integer(0),
        Object::Integer(0),
        Object::Integer(PAGE_WIDTH),
        Object::Integer(PAGE_HEIGHT),
    ]);
    let mut kids = Vec::new();
    for runs in pages {
        let encoded = Content {
            operations: page_ops(runs),
        }
        .encode()
        .expect("content encodes");
        let content_id = doc.add_object(Stream::new(dictionary! {}, encoded));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_root_id,
            "MediaBox" => media_box.clone(),
            "Resources" => resources_id,
            "Contents" => content_id,
        });
        kids.push(Object::Reference(page_id));
    }
    let count = i64::try_from(pages.len()).expect("page count fits");
    doc.objects.insert(
        pages_root_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => Object::Array(kids),
            "Count" => Object::Integer(count),
            "Resources" => resources_id,
            "MediaBox" => media_box,
        }),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_root_id });
    doc.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).expect("document serialises");
    bytes
}

/// Body text that uses the ligature glyphs: `ﬁnite ﬂow ofﬁce afﬂuent effort`
/// encoded through the `F3` Differences codes.
fn ligature_pdf() -> Vec<u8> {
    let mut line = Vec::new();
    line.extend_from_slice(b"The \x80nite \x81ow of the o\x82ce is a\x83uent and the e\x84ort");
    let pages = vec![vec![
        raw("F3", 10.0, 72.0, 700.0, &line),
        run("F1", 10.0, 72.0, 688.0, "is shared across every backend."),
    ]];
    build_pdf(&pages)
}

/// The same sentence through `ToUnicode` presentation forms, which a parser
/// hands on as U+FB01/U+FB02 unless it normalises them.
fn to_unicode_ligature_pdf() -> Vec<u8> {
    let pages = vec![vec![
        raw(
            "F4",
            10.0,
            72.0,
            700.0,
            b"The \x80nite \x81ow of the of\x80ce",
        ),
        run("F1", 10.0, 72.0, 688.0, "is shared across every backend."),
    ]];
    build_pdf(&pages)
}

/// Line-end hyphenation: `pipe-` / `line` must join (`pipeline` is attested
/// later), `state-of-` / `the-art` keeps its printed hyphens, and `self-` /
/// `contained` stays a compound because `self-contained` is attested.
fn hyphen_pdf() -> Vec<u8> {
    let lines = [
        "The extraction pipe-",
        "line recovers the columns of a state-of-",
        "the-art paper, and the self-",
        "contained worker publishes the result. The pipeline",
        "is self-contained by design.",
    ];
    let mut runs = Vec::new();
    let mut y = 700.0;
    for line in lines {
        runs.push(run("F1", 10.0, 72.0, y, line));
        y -= 12.0;
    }
    build_pdf(&[runs])
}

/// Superscript citation markers: a raised `5` after `literature` and a
/// raised `2,3` after `work`.
fn superscript_pdf() -> Vec<u8> {
    let runs = vec![
        run("F1", 10.0, 72.0, 700.0, "This is cited in the literature"),
        run("F1", 6.0, 194.5, 704.0, "5"),
        run("F1", 10.0, 72.0, 688.0, "and extends prior work"),
        run("F1", 6.0, 174.0, 692.0, "2,3"),
        run("F1", 10.0, 72.0, 676.0, "on reading order."),
    ];
    build_pdf(&[runs])
}

/// Three pages with a running head, a page number and a short body each.
fn running_head_pdf() -> Vec<u8> {
    let bodies = [
        [
            "The first page carries the running head above its body.",
            "Its page number sits alone in the footer band.",
        ],
        [
            "The second page repeats the running head verbatim.",
            "Only the page number changes between pages.",
        ],
        [
            "The third page closes the document with the same frame.",
            "Every backend must drop the frame from the page text.",
        ],
    ];
    let pages = bodies
        .iter()
        .enumerate()
        .map(|(index, body)| {
            let number = index + 1;
            vec![
                run(
                    "F1",
                    8.0,
                    72.0,
                    770.0,
                    "Journal of Fixture Studies, Vol. 1 (2026)",
                ),
                run("F1", 10.0, 72.0, 700.0, body[0]),
                run("F1", 10.0, 72.0, 688.0, body[1]),
                run("F1", 8.0, 300.0, 30.0, &number.to_string()),
            ]
        })
        .collect::<Vec<_>>();
    build_pdf(&pages)
}

/// Two columns with a figure caption interleaved in the right column.
fn two_column_pdf() -> Vec<u8> {
    let left = [
        "Left column text begins here and",
        "continues down the left side of",
        "the page in short lines. The caption",
        "on the right must not interrupt it.",
        "The left column ends with this line.",
    ];
    let right_top = [
        "Right column text starts at the",
        "top of the right side of the page.",
    ];
    let right_bottom = [
        "Right column text resumes below",
        "the figure and finishes the page.",
    ];
    let mut runs = Vec::new();
    let mut y = 700.0;
    for line in left {
        runs.push(run("F1", 10.0, 60.0, y, line));
        y -= 12.0;
    }
    let mut y = 700.0;
    for line in right_top {
        runs.push(run("F1", 10.0, 320.0, y, line));
        y -= 12.0;
    }
    runs.push(run(
        "F1",
        9.0,
        320.0,
        640.0,
        "Figure 1: A caption inside the right column.",
    ));
    let mut y = 620.0;
    for line in right_bottom {
        runs.push(run("F1", 10.0, 320.0, y, line));
        y -= 12.0;
    }
    build_pdf(&[runs])
}

/// Two columns whose lines share baselines, the usual journal layout: the
/// row grouping first fuses each pair, and the gutter split must separate
/// them again on every backend.
fn shared_baseline_columns_pdf() -> Vec<u8> {
    let left = [
        "Shared baseline left one begins",
        "shared baseline left two follows",
        "shared baseline left three next",
        "shared baseline left four ends.",
    ];
    let right = [
        "Shared baseline right one begins",
        "shared baseline right two follows",
        "shared baseline right three next",
        "shared baseline right four ends.",
    ];
    let mut runs = Vec::new();
    let mut y = 700.0;
    for (l, r) in left.iter().zip(right.iter()) {
        runs.push(run("F1", 10.0, 60.0, y, l));
        runs.push(run("F1", 10.0, 320.0, y, r));
        y -= 12.0;
    }
    build_pdf(&[runs])
}

struct Fixture {
    name: String,
    bytes: Vec<u8>,
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn committed_pdfs() -> Vec<Fixture> {
    let mut out = Vec::new();
    for relative in [
        "native-worker/native.pdf",
        "native-worker/existing-ocr.pdf",
        "pdfium-unicode/native.pdf",
        "pdfium-unicode/partial-cmap.pdf",
        "pdfium-unicode/existing-ocr.pdf",
    ] {
        let path = fixture_dir().join(relative);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        out.push(Fixture {
            name: format!("fixtures/{relative}"),
            bytes,
        });
    }
    out
}

fn synthetic_fixtures() -> Vec<Fixture> {
    vec![
        Fixture {
            name: "synthetic/paper".into(),
            bytes: common::synthetic_paper(),
        },
        Fixture {
            name: "synthetic/ligatures".into(),
            bytes: ligature_pdf(),
        },
        Fixture {
            name: "synthetic/ligatures-tounicode".into(),
            bytes: to_unicode_ligature_pdf(),
        },
        Fixture {
            name: "synthetic/hyphens".into(),
            bytes: hyphen_pdf(),
        },
        Fixture {
            name: "synthetic/superscripts".into(),
            bytes: superscript_pdf(),
        },
        Fixture {
            name: "synthetic/running-heads".into(),
            bytes: running_head_pdf(),
        },
        Fixture {
            name: "synthetic/two-columns".into(),
            bytes: two_column_pdf(),
        },
        Fixture {
            name: "synthetic/shared-baseline-columns".into(),
            bytes: shared_baseline_columns_pdf(),
        },
    ]
}

fn extra_pdfs() -> Vec<Fixture> {
    let Some(dir) = std::env::var_os("TPE_ENGINE_COMPARE_DIR") else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<Fixture> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
        })
        .filter_map(|path| {
            let bytes = std::fs::read(&path).ok()?;
            Some(Fixture {
                name: format!("extra/{}", path.file_name()?.to_string_lossy()),
                bytes,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

// ---------------------------------------------------------------------------
// Running backends
// ---------------------------------------------------------------------------

/// Backends that can run here, and the ones compiled in but not configured.
fn backend_roster() -> (Vec<&'static str>, Vec<(&'static str, String)>) {
    let mut ready = Vec::new();
    let mut unsupported = Vec::new();
    for name in backend::available() {
        if matches!(name, "routed" | "liteparse-layout" | "docling") {
            // Routing and layout composites are owned elsewhere; this harness
            // compares the primary text parsers.
            continue;
        }
        let Some(extractor) = backend::by_name(name) else {
            continue;
        };
        let probe = backend::probe_pdf().expect("probe builds");
        match extractor.open(&probe, None) {
            Ok(_) => ready.push(name),
            Err(error) => unsupported.push((name, format!("unsupported: {error}"))),
        }
    }
    (ready, unsupported)
}

struct Extracted {
    pages: Vec<String>,
    status: String,
}

fn extract(backend_name: &str, bytes: &[u8]) -> Result<Extracted, String> {
    let (_dir, path) = common::write_temp_pdf(bytes);
    let result = run_job(&Job {
        path: path.to_string_lossy().into_owned(),
        backend: backend_name.into(),
        pages: None,
        password: None,
        max_bytes: None,
        figures_dir: None,
    })
    .map_err(|e| e.to_string())?;
    Ok(Extracted {
        pages: result.pages.iter().map(|p| p.text.clone()).collect(),
        status: result.status.as_str().to_string(),
    })
}

// ---------------------------------------------------------------------------
// Normalisation, similarity and diff
// ---------------------------------------------------------------------------

/// Lines trimmed, internal whitespace collapsed, empty lines dropped.
fn normalise(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect()
}

fn words(lines: &[String]) -> Vec<&str> {
    lines.iter().flat_map(|line| line.split(' ')).collect()
}

/// Length of the longest common subsequence of two slices.
fn lcs_len<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let mut prev = vec![0_usize; b.len() + 1];
    let mut cur = vec![0_usize; b.len() + 1];
    for x in a {
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = if x == y {
                prev[j] + 1
            } else {
                prev[j + 1].max(cur[j])
            };
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// `2 * LCS / (|a| + |b|)` over words; `1.0` for two empty texts.
#[allow(clippy::cast_precision_loss)] // word counts are far below 2^52
fn similarity(a: &[String], b: &[String]) -> f64 {
    let (wa, wb) = (words(a), words(b));
    let total = wa.len() + wb.len();
    if total == 0 {
        return 1.0;
    }
    2.0 * lcs_len(&wa, &wb) as f64 / total as f64
}

/// A line diff (`-` only in `left`, `+` only in `right`, two spaces for
/// common lines).
fn line_diff(left: &[String], right: &[String]) -> String {
    let rows = left.len();
    let cols = right.len();
    let mut table = vec![vec![0_usize; cols + 1]; rows + 1];
    for row in (0..rows).rev() {
        for col in (0..cols).rev() {
            table[row][col] = if left[row] == right[col] {
                table[row + 1][col + 1] + 1
            } else {
                table[row + 1][col].max(table[row][col + 1])
            };
        }
    }
    let mut out = String::new();
    let (mut row, mut col) = (0, 0);
    while row < rows && col < cols {
        if left[row] == right[col] {
            let _ = writeln!(out, "  {}", left[row]);
            row += 1;
            col += 1;
        } else if table[row + 1][col] >= table[row][col + 1] {
            let _ = writeln!(out, "- {}", left[row]);
            row += 1;
        } else {
            let _ = writeln!(out, "+ {}", right[col]);
            col += 1;
        }
    }
    for line in &left[row..] {
        let _ = writeln!(out, "- {line}");
    }
    for line in &right[col..] {
        let _ = writeln!(out, "+ {line}");
    }
    out
}

// ---------------------------------------------------------------------------
// The comparison
// ---------------------------------------------------------------------------

/// One backend's normalised lines of every page and its status, or why it
/// failed.
type Output = Result<(Vec<Vec<String>>, String), String>;

struct Comparison {
    /// Per backend: normalised lines of every page, or the failure.
    outputs: BTreeMap<&'static str, Output>,
}

fn compare(fixture: &Fixture, ready: &[&'static str]) -> Comparison {
    let mut outputs = BTreeMap::new();
    for &name in ready {
        let output = extract(name, &fixture.bytes).map(|e| {
            (
                e.pages.iter().map(|p| normalise(p)).collect::<Vec<_>>(),
                e.status,
            )
        });
        outputs.insert(name, output);
    }
    Comparison { outputs }
}

/// Similarity between two backends over the whole document (pages joined).
fn document_similarity(c: &Comparison, a: &str, b: &str) -> Option<f64> {
    let (Some(Ok((pa, _))), Some(Ok((pb, _)))) = (c.outputs.get(a), c.outputs.get(b)) else {
        return None;
    };
    let fa: Vec<String> = pa.iter().flatten().cloned().collect();
    let fb: Vec<String> = pb.iter().flatten().cloned().collect();
    Some(similarity(&fa, &fb))
}

fn print_report(fixture: &Fixture, c: &Comparison) {
    println!("=== {} ===", fixture.name);
    for (name, output) in &c.outputs {
        match output {
            Ok((pages, status)) => {
                let lines: usize = pages.iter().map(Vec::len).sum();
                let word_count: usize = pages.iter().map(|p| words(p).len()).sum();
                println!(
                    "  {name:<10} {status:<8} pages={} lines={lines} words={word_count}",
                    pages.len()
                );
            }
            Err(error) => println!("  {name:<10} failed: {error}"),
        }
    }
    let names: Vec<&&str> = c.outputs.keys().collect();
    for (i, a) in names.iter().enumerate() {
        for b in &names[i + 1..] {
            let Some(score) = document_similarity(c, a, b) else {
                continue;
            };
            println!("  {a} vs {b}: similarity {score:.3}");
            if score < 1.0 {
                let (Some(Ok((pa, _))), Some(Ok((pb, _)))) = (c.outputs.get(*a), c.outputs.get(*b))
                else {
                    continue;
                };
                for (page, (la, lb)) in pa.iter().zip(pb.iter()).enumerate() {
                    if la != lb {
                        println!("  --- page {} ({a} -, {b} +)", page + 1);
                        print!("{}", line_diff(la, lb));
                    }
                }
            }
        }
    }
}

fn all_fixtures() -> Vec<Fixture> {
    let mut all = synthetic_fixtures();
    all.extend(committed_pdfs());
    all.extend(extra_pdfs());
    all
}

fn joined(lines: &[String]) -> String {
    lines.join("\n")
}

/// The text-quality guarantees one backend's normalised `pages` must meet
/// on the synthetic fixture `fixture`.
fn check_fixture(fixture: &str, name: &str, pages: &[Vec<String>]) {
    let text = joined(&pages.iter().flatten().cloned().collect::<Vec<_>>());
    let context = format!("{name} on {fixture}:\n{text}");
    match fixture {
        "synthetic/ligatures" => {
            assert!(
                text.contains("The finite flow of the office is affluent and the effort"),
                "ligatures expanded: {context}"
            );
            assert!(
                !text.contains('\u{FB01}'),
                "no presentation form: {context}"
            );
        }
        "synthetic/ligatures-tounicode" => {
            assert!(
                text.contains("The finite flow of the office"),
                "ToUnicode ligatures expanded: {context}"
            );
            assert!(
                !text.contains('\u{FB01}'),
                "no presentation form: {context}"
            );
        }
        "synthetic/hyphens" => {
            assert!(text.contains("pipeline"), "de-hyphenated: {context}");
            assert!(
                text.contains("state-of-\nthe-art"),
                "printed hyphen kept at the line end: {context}"
            );
            assert!(
                text.contains("self-\ncontained worker"),
                "attested compound kept: {context}"
            );
            assert!(!text.contains("pipe-\n"), "no dangling hyphen: {context}");
        }
        "synthetic/superscripts" => {
            assert!(
                text.contains("literature\u{2075}"),
                "citation marker: {context}"
            );
            assert!(
                text.contains("work\u{00B2},\u{00B3}"),
                "multi marker: {context}"
            );
        }
        "synthetic/running-heads" => {
            assert!(
                !text.contains("Journal of Fixture Studies"),
                "running head removed: {context}"
            );
            assert!(
                text.contains("The first page carries"),
                "body kept: {context}"
            );
            assert_eq!(pages.len(), 3, "{context}");
            for (index, page) in pages.iter().enumerate() {
                let number = (index + 1).to_string();
                assert!(
                    !page.contains(&number),
                    "page number {number} removed: {context}"
                );
            }
        }
        _ => check_columns(fixture, &context, &text),
    }
}

/// The reading-order guarantees on the column fixtures: left column before
/// right column, captions in place, fused rows split at the gutter.
fn check_columns(fixture: &str, context: &str, text: &str) {
    match fixture {
        "synthetic/two-columns" => {
            let left_end = text
                .find("The left column ends with this line.")
                .expect(context);
            let right_start = text.find("Right column text starts").expect(context);
            let caption = text.find("Figure 1:").expect(context);
            let right_end = text.find("finishes the page.").expect(context);
            assert!(left_end < right_start, "left column first: {context}");
            assert!(
                right_start < caption && caption < right_end,
                "caption in place: {context}"
            );
        }
        "synthetic/shared-baseline-columns" => {
            let left_end = text.find("left four ends.").expect(context);
            let right_start = text.find("Shared baseline right one").expect(context);
            assert!(
                left_end < right_start,
                "fused rows split at the gutter: {context}"
            );
            assert!(
                !text.contains("begins Shared baseline right"),
                "rows not fused across the gutter: {context}"
            );
        }
        "synthetic/paper" => {
            let intro = text.find("Academic PDFs encode").expect(context);
            let left_end = text
                .find("raw bytes of each reference entry.")
                .expect(context);
            let right_start = text.find("Our contribution is a pipeline").expect(context);
            assert!(
                intro < left_end && left_end < right_start,
                "columns in order: {context}"
            );
        }
        _ => {}
    }
}

#[test]
fn backends_agree_on_fixture_text() {
    let (ready, unsupported) = backend_roster();
    println!("backends: {}", ready.join(", "));
    for (name, reason) in &unsupported {
        println!("  {name}: {reason}");
    }
    assert!(ready.contains(&"lopdf"));

    let mut scores: BTreeMap<String, f64> = BTreeMap::new();
    for fixture in all_fixtures() {
        let c = compare(&fixture, &ready);
        print_report(&fixture, &c);
        for (name, output) in &c.outputs {
            assert!(
                output.is_ok(),
                "{name} failed on {}: {:?}",
                fixture.name,
                output.as_ref().err()
            );
        }
        for &other in &ready {
            if other == "lopdf" {
                continue;
            }
            if let Some(score) = document_similarity(&c, "lopdf", other) {
                scores.insert(format!("{}|{other}", fixture.name), score);
            }
        }

        // Text-quality guarantees on the synthetic fixtures, checked on every
        // backend so the normalised output stays identical across parsers.
        for (name, output) in &c.outputs {
            if let Ok((pages, _)) = output {
                check_fixture(&fixture.name, name, pages);
            }
        }
    }

    println!("similarity summary (lopdf vs other):");
    for (key, score) in &scores {
        println!("  {key}: {score:.3}");
    }
    // Where the PDF permits it, the normalised text must be identical.
    for (key, score) in &scores {
        let (fixture, _) = key.split_once('|').expect("key");
        if fixture.starts_with("synthetic/") {
            assert!(
                (*score - 1.0).abs() < f64::EPSILON,
                "{key}: synthetic fixture text differs across backends ({score:.3})"
            );
        }
    }
}

#[test]
fn similarity_and_diff_are_well_defined() {
    let a = normalise("The quick  brown fox\n\n jumps over\n");
    let b = normalise("The quick brown fox\njumps over the dog");
    assert_eq!(a, vec!["The quick brown fox", "jumps over"]);
    assert!((similarity(&a, &a) - 1.0).abs() < f64::EPSILON);
    let score = similarity(&a, &b);
    assert!((score - 2.0 * 6.0 / 14.0).abs() < 1e-9, "{score}");
    let diff = line_diff(&a, &b);
    assert_eq!(
        diff,
        "  The quick brown fox\n- jumps over\n+ jumps over the dog\n"
    );
    assert!((similarity(&[], &[]) - 1.0).abs() < f64::EPSILON);
    assert!(similarity(&a, &[]).abs() < f64::EPSILON);
}
