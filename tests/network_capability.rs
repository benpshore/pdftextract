//! Local extraction and explicitly separated acquisition in the default build.
#![cfg(not(feature = "network"))]

use std::fs;
use std::process::Command;

use lopdf::{Object, dictionary};
use serde_json::Value;

#[test]
fn local_pdf_preserves_uri_and_metadata_without_acquisition() {
    let root = tempfile::tempdir().unwrap();
    let mut pdf = lopdf::Document::load_mem(&tpe::backend::probe_pdf().unwrap()).unwrap();
    let uri = "https://source.invalid/retained-link";
    let link = pdf.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link",
        "Rect" => vec![72.into(), 710.into(), 200.into(), 735.into()],
        "A" => dictionary! {"S" => "URI", "URI" => Object::string_literal(uri)}
    });
    let page = *pdf.get_pages().get(&1).unwrap();
    pdf.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", vec![Object::Reference(link)]);
    let info = pdf.add_object(dictionary! {
        "Title" => Object::string_literal("Local metadata canary"),
        "Subject" => Object::string_literal("doi:10.1234/local-canary")
    });
    pdf.trailer.set("Info", info);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    let source = root.path().join("source.pdf");
    fs::write(&source, &bytes).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tpe"))
        .args([
            "extract",
            "--backend",
            "lopdf",
            "--json",
            "--timeout-ms",
            "5000",
            "--db",
        ])
        .arg(root.path().join("ledger.sqlite"))
        .arg(&source)
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(record["status"], "complete");
    assert_eq!(record["document"]["hash"], tpe::schema::sha256_hex(&bytes));
    assert_eq!(record["metadata"]["title"], "Local metadata canary");
    assert_eq!(record["pages"][0]["links"][0]["uri"], uri);
    assert_eq!(fs::read(source).unwrap(), bytes);
}

#[test]
fn resolve_cannot_enable_network_or_create_output_in_default_build() {
    let root = tempfile::tempdir().unwrap();
    let csv = root.path().join("references.csv");
    let output = Command::new(env!("CARGO_BIN_EXE_tpe"))
        .args(["bibliography", "--resolve", "--csv"])
        .arg(&csv)
        .arg(root.path().join("not-read.pdf"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("registry resolution is disabled"));
    assert!(!csv.exists());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn optional_server_feature_does_not_implicitly_enable_network() {
    let output = Command::new(env!("CARGO_BIN_EXE_tpe"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("grobid"));
}

#[test]
fn engine_resolver_observes_its_own_capability_after_feature_unification() {
    let mut entry = tpe::schema::ReferenceEntry {
        raw: "Local reference canary. PMID:123456".into(),
        ..Default::default()
    };
    let original = entry.raw.clone();
    let outcome =
        tpe::resolve::Resolver::new(None).resolve_entries(std::slice::from_mut(&mut entry));
    assert_eq!(outcome.resolved, 0);
    assert_eq!(outcome.unresolved, 1);
    assert_eq!(outcome.errors, 1);
    assert!(entry.resolved.is_none());
    assert_eq!(entry.raw, original);
    assert!(entry.attempts.iter().any(|attempt| {
        attempt.outcome == "error"
            && attempt
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("offline mode"))
    }));
}

#[test]
fn corpus_cache_is_usable_but_missing_sources_cannot_download() {
    use tpe::corpus::{Manifest, fetch_item};
    let root = tempfile::tempdir().unwrap();
    let manifest: Manifest = serde_json::from_str(include_str!("../corpus/manifest.json")).unwrap();
    let mut item = manifest.items[0].clone();
    item.pdf_url = "https://source.invalid/paper".into();
    item.pdf_sha256 = None;
    item.source_url = None;
    item.source_sha256 = None;
    let error = fetch_item(&item, root.path(), "fixture", false).unwrap_err();
    assert!(error.to_string().contains("network capability is disabled"));
    let (pdf, _, _) = tpe::corpus::cache_paths(root.path(), &item);
    assert!(!pdf.exists());
    let bytes = tpe::backend::probe_pdf().unwrap();
    fs::write(&pdf, &bytes).unwrap();
    let cached = fetch_item(&item, root.path(), "fixture", false).unwrap();
    assert_eq!(cached.pdf_sha256, tpe::schema::sha256_hex(&bytes));
    assert_eq!(fs::read(cached.pdf_path).unwrap(), bytes);
}
