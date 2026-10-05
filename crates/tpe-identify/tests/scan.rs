//! End-to-end: small PDFs generated with `lopdf`, text extracted by the
//! engine's `lopdf` backend, grouped, named, copied/renamed and undone.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use tempfile::tempdir;
use tpe::schema::sha256_hex;
use tpe_identify::rename::{Op, apply, read_manifest, undo};
use tpe_identify::{GroupKind, LoadOptions, Report, Role, ScanOptions, scan};

const VOCAB: [&str; 32] = [
    "shingle", "article", "identity", "digest", "corpus", "library", "version", "preprint",
    "journal", "author", "title", "method", "result", "signal", "sample", "measure", "random",
    "matrix", "kernel", "theorem", "lemma", "model", "train", "error", "noise", "layer", "graph",
    "node", "edge", "path", "field", "space",
];

/// Deterministic pseudo-prose of `n` words.
fn prose(n: usize, seed: u64) -> Vec<String> {
    let mut state = seed;
    (0..n)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            VOCAB[usize::try_from((state >> 33) % VOCAB.len() as u64).unwrap_or(0)].to_string()
        })
        .collect()
}

struct Fixture<'a> {
    title: &'a str,
    author: &'a str,
    created: &'a str,
    producer: &'a str,
    doi: &'a str,
    words: &'a [String],
}

/// One-page PDF with Helvetica: a title line, a DOI line and the body in
/// lines of ten words.
fn pdf(f: &Fixture) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let tree_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let mut operations = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["F1".into(), 18.into()]),
        Operation::new("Td", vec![72.into(), 740.into()]),
        Operation::new("Tj", vec![Object::string_literal(f.title)]),
        Operation::new("Tf", vec!["F1".into(), 10.into()]),
        Operation::new("Td", vec![0.into(), (-24).into()]),
        Operation::new(
            "Tj",
            vec![Object::string_literal(format!(
                "{} doi:{} 2021",
                f.author, f.doi
            ))],
        ),
    ];
    for line in f.words.chunks(10) {
        operations.push(Operation::new("Td", vec![0.into(), (-14).into()]));
        operations.push(Operation::new(
            "Tj",
            vec![Object::string_literal(line.join(" "))],
        ));
    }
    operations.push(Operation::new("ET", vec![]));
    let content = Content { operations }.encode().unwrap();
    let content_id = doc.add_object(Stream::new(dictionary! {}, content));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => tree_id, "Contents" => content_id, "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    doc.objects.insert(
        tree_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1,
        }),
    );
    let info_id = doc.add_object(dictionary! {
        "Title" => Object::string_literal(f.title),
        "Author" => Object::string_literal(f.author),
        "CreationDate" => Object::string_literal(f.created),
        "Producer" => Object::string_literal(f.producer),
    });
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => tree_id });
    doc.trailer.set("Root", catalog_id);
    doc.trailer.set("Info", info_id);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

/// The corpus every test scans: `a.pdf` with an exact copy, a re-saved
/// copy (same text, different bytes), a near-duplicate (two words changed),
/// an unrelated paper `b.pdf`, and `c.pdf`, a different work whose
/// canonical name collides with `a`'s.
fn corpus(dir: &Path) -> Vec<PathBuf> {
    let a_words = prose(160, 1);
    let mut near = a_words.clone();
    near[40] = "altered".to_string();
    near[120] = "changed".to_string();
    let a = Fixture {
        title: "Shingle Based Identification of Articles",
        author: "Ada Lovelace",
        created: "D:20210315120000Z",
        producer: "fixture 1",
        doi: "10.1000/tpe.identify.1",
        words: &a_words,
    };
    let files = [
        ("a.pdf", pdf(&a)),
        ("a-copy.pdf", pdf(&a)),
        (
            "a-resaved.pdf",
            pdf(&Fixture {
                producer: "fixture 2 (re-saved)",
                ..a
            }),
        ),
        (
            "a-near.pdf",
            pdf(&Fixture {
                producer: "fixture 3",
                words: &near,
                ..a
            }),
        ),
        (
            "b.pdf",
            pdf(&Fixture {
                title: "An Unrelated Study of Other Things",
                author: "Charles Babbage",
                created: "D:20190101000000Z",
                producer: "fixture 4",
                doi: "10.1000/tpe.identify.2",
                words: &prose(160, 2),
            }),
        ),
        (
            "c.pdf",
            pdf(&Fixture {
                title: "Shingle Based Identification of the Articles",
                author: "Ada Lovelace",
                created: "D:20210601000000Z",
                producer: "fixture 5",
                doi: "10.1000/tpe.identify.3",
                words: &prose(160, 3),
            }),
        ),
    ];
    let mut paths = Vec::new();
    for (name, bytes) in files {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        paths.push(path);
    }
    paths
}

fn options(rename_into: Option<&Path>) -> ScanOptions {
    ScanOptions {
        load: LoadOptions {
            extract: true,
            ..LoadOptions::default()
        },
        rename_into: rename_into.map(Path::to_path_buf),
        ..ScanOptions::default()
    }
}

fn name_of(report: &Report, file: &str) -> String {
    let index = report
        .inputs
        .iter()
        .find(|i| i.path.as_deref().is_some_and(|p| p.ends_with(file)))
        .unwrap_or_else(|| panic!("{file} not scanned: {:?}", report.errors))
        .index;
    report
        .rename
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .find(|e| e.index == index)
        .unwrap()
        .to
        .clone()
}

#[test]
fn exact_resaved_near_and_distinct_pdfs_are_grouped_and_named() {
    let dir = tempdir().unwrap();
    corpus(dir.path());
    let report = scan(&[dir.path().to_path_buf()], &options(None));
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.inputs.len(), 6);
    for input in &report.inputs {
        assert_eq!(input.text_source, "extract:lopdf");
        assert!(
            input.words >= 150,
            "{:?}: {} words",
            input.path,
            input.words
        );
        let expected_year = if input.path.as_deref().unwrap().ends_with("b.pdf") {
            2019
        } else {
            2021
        };
        assert_eq!(input.key.year, Some(expected_year));
        assert!(
            input
                .key
                .doi
                .as_deref()
                .is_some_and(|d| d.starts_with("10.1000/tpe.identify.")),
            "{:?}",
            input.key
        );
    }
    assert_eq!(report.groups.len(), 3, "{:#?}", report.groups);
    let work = &report.groups[0];
    assert_eq!(work.kind, GroupKind::NearDuplicates);
    assert_eq!(work.members.len(), 4);
    assert!(work.confidence >= 0.85, "{}", work.confidence);
    let a_hash = sha256_hex(&fs::read(dir.path().join("a.pdf")).unwrap());
    assert_eq!(report.inputs[work.canonical].sha256, a_hash);
    assert_eq!(
        work.members
            .iter()
            .filter(|m| m.role == Role::Duplicate)
            .count(),
        3
    );
    assert!(
        report.groups[1..]
            .iter()
            .all(|g| g.kind == GroupKind::Single)
    );

    assert_eq!(
        name_of(&report, "a-copy.pdf"),
        "Lovelace_2021_Shingle-Based-Identification-Articles.pdf"
    );
    assert_eq!(
        name_of(&report, "a-near.pdf"),
        "Lovelace_2021_Shingle-Based-Identification-Articles_dup2.pdf"
    );
    assert_eq!(
        name_of(&report, "a-resaved.pdf"),
        "Lovelace_2021_Shingle-Based-Identification-Articles_dup3.pdf"
    );
    assert_eq!(
        name_of(&report, "a.pdf"),
        "Lovelace_2021_Shingle-Based-Identification-Articles_dup4.pdf"
    );
    assert_eq!(
        name_of(&report, "b.pdf"),
        "Babbage_2019_Unrelated-Study-Other-Things.pdf"
    );
    assert_eq!(
        name_of(&report, "c.pdf"),
        "Lovelace_2021_Shingle-Based-Identification-Articles_2.pdf"
    );
    assert!(
        report
            .rename
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .all(|e| e.op == Op::Copy)
    );

    let json = serde_json::to_string(&report).unwrap();
    let back: Report = serde_json::from_str(&json).unwrap();
    assert_eq!(back.groups, report.groups);
    let table = tpe_identify::render_table(&report);
    assert!(table.contains("near_duplicates"));
    assert!(table.contains("6 inputs, 3 works, 1 with more than one member, 0 errors"));
}

#[test]
fn apply_copies_verified_bytes_and_undo_removes_only_untouched_copies() {
    let dir = tempdir().unwrap();
    let out = tempdir().unwrap();
    let sources = corpus(dir.path());
    let before: Vec<Vec<u8>> = sources.iter().map(|p| fs::read(p).unwrap()).collect();
    let report = scan(&[dir.path().to_path_buf()], &options(Some(out.path())));
    let plan = report.rename.clone().unwrap();
    let (manifest, manifest_path) = apply(&plan, out.path()).unwrap();
    assert!(manifest_path.ends_with("tpe-identify-manifest.json"));
    assert_eq!(manifest.entries.len(), 6);
    assert!(
        manifest
            .entries
            .iter()
            .all(|e| e.skipped.is_none() && e.op == Op::Copy),
        "{manifest:#?}"
    );
    for entry in &manifest.entries {
        let copied = fs::read(&entry.to).unwrap();
        assert_eq!(sha256_hex(&copied), entry.sha256);
        assert_eq!(copied, fs::read(&entry.from).unwrap());
    }
    // Inputs untouched, in place and unchanged.
    let after: Vec<Vec<u8>> = sources.iter().map(|p| fs::read(p).unwrap()).collect();
    assert_eq!(before, after);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 6);

    // A second apply cannot overwrite what exists and writes a second manifest.
    let (again, second_path) = apply(&plan, out.path()).unwrap();
    assert!(second_path.ends_with("tpe-identify-manifest-2.json"));
    assert!(
        again
            .entries
            .iter()
            .all(|e| e.skipped.as_deref() == Some("target exists")),
        "{again:#?}"
    );

    // Undo: a dry run changes nothing; a tampered copy is kept.
    let stored = read_manifest(&manifest_path).unwrap();
    assert_eq!(stored, manifest);
    let tampered = PathBuf::from(&stored.entries[1].to);
    fs::write(&tampered, b"%PDF-1.4 tampered").unwrap();
    let dry = undo(&stored, true);
    assert_eq!(dry.iter().filter(|e| e.action == "remove").count(), 5);
    assert_eq!(fs::read_dir(out.path()).unwrap().count(), 8);
    let done = undo(&stored, false);
    assert_eq!(done.iter().filter(|e| e.action == "remove").count(), 5);
    let kept = done.iter().find(|e| e.to == stored.entries[1].to).unwrap();
    assert_eq!(kept.action, "skip");
    assert_eq!(
        kept.reason.as_deref(),
        Some("renamed file is missing or changed")
    );
    assert!(tampered.exists());
    let remaining: Vec<String> = fs::read_dir(out.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(remaining.len(), 3, "{remaining:?}");
    assert_eq!(
        before,
        sources
            .iter()
            .map(|p| fs::read(p).unwrap())
            .collect::<Vec<_>>()
    );
}

#[test]
fn apply_renames_inside_the_output_directory_and_undo_restores_names() {
    let dir = tempdir().unwrap();
    let sources = corpus(dir.path());
    let hashes: Vec<String> = sources
        .iter()
        .map(|p| sha256_hex(&fs::read(p).unwrap()))
        .collect();
    let report = scan(&[dir.path().to_path_buf()], &options(Some(dir.path())));
    let plan = report.rename.clone().unwrap();
    assert!(plan.entries.iter().all(|e| e.op == Op::Rename), "{plan:#?}");
    // A file appearing at a target name between plan and apply is never overwritten.
    let blocked = dir
        .path()
        .join("Babbage_2019_Unrelated-Study-Other-Things.pdf");
    fs::write(&blocked, b"%PDF-1.4 someone else's file").unwrap();
    let (manifest, _) = apply(&plan, dir.path()).unwrap();
    let skipped: Vec<&str> = manifest
        .entries
        .iter()
        .filter_map(|e| e.skipped.as_deref())
        .collect();
    assert_eq!(skipped, vec!["target exists"]);
    assert_eq!(fs::read(&blocked).unwrap(), b"%PDF-1.4 someone else's file");
    assert!(dir.path().join("b.pdf").exists());
    for entry in manifest.entries.iter().filter(|e| e.skipped.is_none()) {
        assert!(
            !Path::new(&entry.from).exists(),
            "{} still exists",
            entry.from
        );
        assert_eq!(sha256_hex(&fs::read(&entry.to).unwrap()), entry.sha256);
    }
    let restored = undo(&manifest, false);
    assert_eq!(restored.iter().filter(|e| e.action == "restore").count(), 5);
    for (path, hash) in sources.iter().zip(&hashes) {
        assert_eq!(
            &sha256_hex(&fs::read(path).unwrap()),
            hash,
            "{}",
            path.display()
        );
    }
}

#[test]
fn cli_scan_is_a_dry_run_and_rejects_apply_without_a_directory() {
    let dir = tempdir().unwrap();
    let out = tempdir().unwrap();
    corpus(dir.path());
    let json = out.path().join("report.json");
    let output = Command::new(env!("CARGO_BIN_EXE_tpe-identify"))
        .args(["scan", "--json"])
        .arg(&json)
        .arg("--rename-into")
        .arg(out.path())
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("dry run: nothing was copied or renamed"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Lovelace_2021_Shingle-Based-Identification-Articles.pdf"),
        "{stdout}"
    );
    let report: Report = serde_json::from_slice(&fs::read(&json).unwrap()).unwrap();
    assert_eq!(report.inputs.len(), 6);
    assert_eq!(
        fs::read_dir(out.path()).unwrap().count(),
        1,
        "only the report was written"
    );

    let rejected = Command::new(env!("CARGO_BIN_EXE_tpe-identify"))
        .args(["scan", "--apply"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("--rename-into"));

    let missing = Command::new(env!("CARGO_BIN_EXE_tpe-identify"))
        .args(["undo"])
        .arg(out.path().join("nope.json"))
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
}
