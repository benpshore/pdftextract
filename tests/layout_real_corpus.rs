//! Independent visual reading-order anchors; no external PDF bytes in git.
//! Run with `TPE_LAYOUT_CORPUS` pointing to hash-verified #272 public PDFs.
use std::path::PathBuf;
use tpe::pipeline::run_job;
use tpe::schema::{Job, sha256_hex};

#[test]
#[ignore = "requires the four public, SHA-pinned PDFs; run in layout witness CI"]
fn visual_real_file_reading_order_witnesses() {
    let root = PathBuf::from(std::env::var_os("TPE_LAYOUT_CORPUS").expect("TPE_LAYOUT_CORPUS"));
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../docs/analysis/layout-eval-2026-10-09/corpus.json"
    ))
    .unwrap();
    let truth: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/layout-witnesses/expected.json")).unwrap();
    for case in truth["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let source = corpus["documents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == id)
            .unwrap();
        let path = root.join(format!("{id}.pdf"));
        assert_eq!(
            sha256_hex(&std::fs::read(&path).unwrap()),
            source["sha256"].as_str().unwrap()
        );
        let result = run_job(&Job {
            path: path.to_str().unwrap().to_string(),
            backend: "lopdf".to_string(),
            pages: None,
            password: None,
            max_bytes: Some(2_000_000),
            figures_dir: None,
        })
        .unwrap();
        let page = usize::try_from(case["page"].as_u64().unwrap() - 1).unwrap();
        let normalized = result.pages[page]
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let mut after = 0;
        for anchor in case["ordered"].as_array().unwrap() {
            let expected = anchor.as_str().unwrap();
            let at=normalized[after..].find(expected).unwrap_or_else(|| panic!("{id} page {} missing/out-of-order anchor {expected:?} after {after}: {normalized}",page+1));
            after += at + expected.len();
        }
    }
}
