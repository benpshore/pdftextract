//! End-to-end run of the `tpe-refcheck` binary without any network: the
//! on-disk cache is seeded from hand-written fixtures (the documented
//! `doi.org` CSL-JSON and Crossref `/works` shapes) and the tool runs
//! `--offline`, so a cache miss is an error, never a request.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;
use tpe_refcheck::{Cache, ReferenceEntry, query_text};

const DEEP_LEARNING_DOI: &str = "10.1038/nature14539";
const UNREGISTERED_DOI: &str = "10.9999/not.registered";
const UNCACHED_DOI: &str = "10.9999/never.fetched";

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn entry(index: u32, raw: &str) -> ReferenceEntry {
    ReferenceEntry {
        index,
        label: Some(format!("[{index}]")),
        raw: format!("[{index}] {raw}"),
        page: 9,
        ..ReferenceEntry::default()
    }
}

/// Five entries: a verified DOI, the same DOI with wrong year and pages, a
/// DOI-less entry found by query, an unregistered DOI whose query finds
/// nothing, and a DOI that is not in the cache (offline error).
fn entries() -> Vec<ReferenceEntry> {
    let mut verified = entry(
        1,
        "LeCun Y, Bengio Y, Hinton G. Deep learning. Nature. 2015;521(7553):436–444. doi:10.1038/nature14539",
    );
    verified.authors = vec!["LeCun, Y.".into(), "Bengio, Y.".into(), "Hinton, G.".into()];
    verified.title = Some("Deep learning".into());
    verified.year = Some(2015);
    verified.venue = Some("Nature".into());
    verified.volume = Some("521".into());
    verified.pages = Some("436–444".into());
    verified.doi = Some(DEEP_LEARNING_DOI.into());

    let mut wrong = verified.clone();
    wrong.index = 2;
    wrong.label = Some("[2]".into());
    wrong.year = Some(2013);
    wrong.pages = Some("437–445".into());

    let mut queried = entry(
        3,
        "Vaswani A, Shazeer N, Parmar N, et al. Attention is all you need. Advances in Neural Information Processing Systems 30, 2017.",
    );
    queried.authors = vec!["Vaswani, A.".into(), "Shazeer, N.".into()];
    queried.title = Some("Attention is all you need".into());
    queried.year = Some(2017);

    let mut unregistered = entry(
        4,
        "Nobody N. A paper that was never published. J Imaginary Res. 2021;1:1-2. https://doi.org/10.9999/not.registered",
    );
    unregistered.authors = vec!["Nobody, N.".into()];
    unregistered.title = Some("A paper that was never published".into());
    unregistered.year = Some(2021);
    unregistered.doi = Some(UNREGISTERED_DOI.into());

    let mut uncached = entry(
        5,
        "Someone S. Offline entry. 2020. doi:10.9999/never.fetched",
    );
    uncached.doi = Some(UNCACHED_DOI.into());

    vec![verified, wrong, queried, unregistered, uncached]
}

fn seed(cache: &Cache, entries: &[ReferenceEntry]) {
    cache
        .put(
            &Cache::doi_key(DEEP_LEARNING_DOI),
            "https://doi.org/10.1038/nature14539",
            200,
            &fixture("doi_org_nature14539.csl.json"),
        )
        .unwrap();
    cache
        .put(
            &Cache::doi_key(UNREGISTERED_DOI),
            "https://doi.org/10.9999/not.registered",
            404,
            "<html>DOI Not Found</html>",
        )
        .unwrap();
    cache
        .put(
            &format!("crossref:{}", Cache::doi_key(UNREGISTERED_DOI)),
            "https://api.crossref.org/works/10.9999/not.registered",
            404,
            &fixture("crossref_not_found.txt"),
        )
        .unwrap();
    cache
        .put(
            &Cache::query_key(&query_text(&entries[2]), 5),
            "https://api.crossref.org/works?query.bibliographic=...&rows=5",
            200,
            &fixture("crossref_query_attention.json"),
        )
        .unwrap();
    cache
        .put(
            &Cache::query_key(&query_text(&entries[3]), 5),
            "https://api.crossref.org/works?query.bibliographic=...&rows=5",
            200,
            &fixture("crossref_query_nomatch.json"),
        )
        .unwrap();
}

fn run(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_tpe-refcheck"))
        .args(args)
        .env_remove("TPE_MAILTO")
        .output()
        .expect("binary runs");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn offline_run_from_seeded_cache_reports_every_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let entries = entries();
    let input = dir.path().join("paper.references.json");
    // The `tpe bibliography` record shape: an object with `references`.
    let record = serde_json::json!({
        "path": "paper.pdf",
        "status": "found",
        "references": entries,
        "warnings": [],
    });
    fs::write(&input, serde_json::to_string(&record).unwrap()).unwrap();
    let cache_dir = dir.path().join("cache");
    seed(&Cache::open(&cache_dir).unwrap(), &entries);
    let out_dir = dir.path().join("out");

    let (code, stdout, stderr) = run(&[
        input.to_str().unwrap(),
        "--offline",
        "--cache-dir",
        cache_dir.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
        "--stdout",
        "json",
    ]);
    assert_eq!(code, 0, "stderr: {stderr}");
    let report: Value = serde_json::from_str(&stdout).expect("JSON report on stdout");
    assert_eq!(report["tool"], "tpe-refcheck");
    assert_eq!(report["offline"], true);
    assert_eq!(report["polite"], false);
    let lines = report["entries"].as_array().unwrap();
    assert_eq!(lines.len(), 5);

    assert_eq!(lines[0]["verdict"], "verified");
    assert_eq!(lines[0]["method"], "doi");
    assert_eq!(lines[0]["record"]["source"], "doi.org");
    assert_eq!(lines[0]["fields"], serde_json::json!([]));
    assert_eq!(lines[0]["comparison"]["same_work"], true);

    assert_eq!(lines[1]["verdict"], "mismatch");
    assert_eq!(lines[1]["fields"], serde_json::json!(["year", "pages"]));
    assert_eq!(lines[1]["comparison"]["same_work"], true);
    assert_eq!(lines[1]["suggested_doi"], Value::Null);

    assert_eq!(lines[2]["verdict"], "verified");
    assert_eq!(lines[2]["method"], "query");
    assert_eq!(lines[2]["suggested_doi"], "10.5555/3295222.3295349");
    assert_eq!(lines[2]["printed"]["doi"], Value::Null);

    assert_eq!(lines[3]["verdict"], "not-found");
    assert_eq!(lines[3]["suggested_doi"], Value::Null);
    let detail = lines[3]["detail"].as_str().unwrap();
    assert!(detail.contains("not registered"), "{detail}");
    assert!(detail.contains("no candidate matches"), "{detail}");

    assert_eq!(lines[4]["verdict"], "offline-or-error");
    assert!(lines[4]["detail"].as_str().unwrap().contains("offline"));

    let summary = &report["summary"];
    assert_eq!(summary["entries"], 5);
    assert_eq!(summary["verified"], 2);
    assert_eq!(summary["mismatch"], 1);
    assert_eq!(summary["not_found"], 1);
    assert_eq!(summary["error"], 1);
    assert_eq!(summary["requests"], 0, "offline: nothing was requested");
    assert_eq!(summary["cache_hits"], 6);

    let json_file: Value =
        serde_json::from_str(&fs::read_to_string(out_dir.join("refcheck.json")).unwrap()).unwrap();
    assert_eq!(json_file["summary"], report["summary"]);
    let markdown = fs::read_to_string(out_dir.join("refcheck.md")).unwrap();
    assert!(markdown.starts_with("# Reference check: "));
    assert!(markdown.contains("| # | Verdict | Printed DOI | Suggested DOI |"));
    assert!(markdown.contains("| [2] | mismatch | 10.1038/nature14539 |  |"));
    assert!(markdown.contains("| [3] | verified |  | 10.5555/3295222.3295349 |"));
    assert!(
        fs::read_to_string(&input)
            .unwrap()
            .contains("\"references\""),
        "input untouched"
    );

    // --strict turns the mismatch into exit status 2; --no-query leaves the
    // DOI-less entry not-found without consulting the cache.
    let (code, _, _) = run(&[
        input.to_str().unwrap(),
        "--offline",
        "--strict",
        "--cache-dir",
        cache_dir.to_str().unwrap(),
    ]);
    assert_eq!(code, 2);
    let (code, stdout, _) = run(&[
        input.to_str().unwrap(),
        "--offline",
        "--no-query",
        "--limit",
        "3",
        "--cache-dir",
        cache_dir.to_str().unwrap(),
        "--stdout",
        "json",
    ]);
    assert_eq!(code, 0);
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["entries"].as_array().unwrap().len(), 3);
    assert_eq!(report["entries"][2]["verdict"], "not-found");
    assert_eq!(report["entries"][2]["method"], "none");
}

#[test]
fn usage_errors_exit_one() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.json");
    let (code, _, stderr) = run(&[missing.to_str().unwrap()]);
    assert_eq!(code, 1);
    assert!(stderr.contains("cannot read"), "{stderr}");
    let bad = dir.path().join("bad.json");
    fs::write(&bad, "{\"pages\": []}").unwrap();
    let (code, _, stderr) = run(&[bad.to_str().unwrap(), "--offline"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("no reference entries"), "{stderr}");
    let (code, _, stderr) = run(&[
        bad.to_str().unwrap(),
        "--offline",
        "--cache-dir",
        dir.path().to_str().unwrap(),
    ]);
    assert_eq!(code, 1, "{stderr}");
}
