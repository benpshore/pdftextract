//! Golden-file, well-formedness and round-trip tests for the three formats.
//! The fixture is a hand-written engine result; the goldens were produced
//! once by the binary and reviewed by hand (no refresh switch on purpose).

#![allow(clippy::too_many_lines)]

use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags};
use tpe::ledger::Ledger;
use tpe::schema::ExtractionResult;
use tpe_export::input::{self, RunSelector};
use tpe_export::model::Export;
use tpe_export::{ExportError, Format, csv, sqlite, write_export, zotero_rdf};

const FIXTURE: &str = include_str!("fixtures/sample.json");
const GOLDEN_RDF: &str = include_str!("golden/sample.rdf");
const GOLDEN_CSV: &str = include_str!("golden/sample.csv");
const GOLDEN_SQLITE: &str = include_str!("golden/sample.sqlite.txt");

const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const Z_NS: &str = "http://www.zotero.org/namespaces/export#";
const DC_NS: &str = "http://purl.org/dc/elements/1.1/";
const DCTERMS_NS: &str = "http://purl.org/dc/terms/";
const BIB_NS: &str = "http://purl.org/net/biblio#";
const FOAF_NS: &str = "http://xmlns.com/foaf/0.1/";
const PRISM_NS: &str = "http://prismstandard.org/namespaces/1.2/basic/";

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.json")
}

fn export() -> Export {
    Export::from_article(&input::parse_json(FIXTURE.as_bytes()).expect("fixture parses"))
}

fn about<'a>(node: roxmltree::Node<'a, 'a>) -> &'a str {
    node.attribute((RDF_NS, "about")).expect("rdf:about")
}

fn child_text<'a>(node: roxmltree::Node<'a, 'a>, ns: &str, name: &str) -> Option<&'a str> {
    node.children()
        .find(|c| c.has_tag_name((ns, name)))
        .and_then(|c| c.text())
}

#[test]
fn zotero_rdf_matches_golden() {
    assert_eq!(zotero_rdf::render(&export()), GOLDEN_RDF);
}

#[test]
fn zotero_rdf_is_well_formed_and_links_references_to_the_article() {
    let text = zotero_rdf::render(&export());
    let doc = roxmltree::Document::parse(&text).expect("well-formed XML");
    let root = doc.root_element();
    assert_eq!(root.tag_name().name(), "RDF");
    assert_eq!(root.tag_name().namespace(), Some(RDF_NS));
    for (prefix, uri) in [
        ("z", Z_NS),
        ("bib", BIB_NS),
        ("dc", DC_NS),
        ("dcterms", DCTERMS_NS),
        ("foaf", FOAF_NS),
        ("prism", PRISM_NS),
    ] {
        assert!(
            root.namespaces()
                .any(|ns| ns.name() == Some(prefix) && ns.uri() == uri),
            "namespace {prefix} declared"
        );
    }
    let items: Vec<_> = root
        .children()
        .filter(roxmltree::Node::is_element)
        .collect();
    assert_eq!(items.len(), 11);
    assert_eq!(about(items[0]), "#item_1");
    let item_types: Vec<&str> = items
        .iter()
        .map(|n| child_text(*n, Z_NS, "itemType").expect("z:itemType"))
        .collect();
    assert_eq!(
        item_types,
        [
            "journalArticle",
            "journalArticle",
            "book",
            "bookSection",
            "conferencePaper",
            "thesis",
            "report",
            "webpage",
            "preprint",
            "journalArticle",
            "journalArticle",
        ]
    );
    assert_eq!(items[0].tag_name().name(), "Article");
    assert_eq!(items[0].tag_name().namespace(), Some(BIB_NS));
    assert_eq!(items[4].tag_name().name(), "Description");
    assert_eq!(items[8].tag_name().name(), "Description");

    let relations: Vec<&str> = items[0]
        .children()
        .filter(|c| c.has_tag_name((DC_NS, "relation")))
        .map(|c| c.attribute((RDF_NS, "resource")).expect("rdf:resource"))
        .collect();
    assert_eq!(relations.len(), 10);
    for item in &items[1..] {
        let referenced_by: Vec<&str> = item
            .children()
            .filter(|c| c.has_tag_name((DCTERMS_NS, "isReferencedBy")))
            .map(|c| c.attribute((RDF_NS, "resource")).expect("rdf:resource"))
            .collect();
        assert_eq!(referenced_by, ["#item_1"]);
        let back: Vec<&str> = item
            .children()
            .filter(|c| c.has_tag_name((DC_NS, "relation")))
            .map(|c| c.attribute((RDF_NS, "resource")).expect("rdf:resource"))
            .collect();
        assert_eq!(back, ["#item_1"]);
        assert!(
            relations.contains(&about(*item)),
            "article relates to {}",
            about(*item)
        );
    }
    assert!(
        items[0]
            .children()
            .all(|c| !c.has_tag_name((DCTERMS_NS, "isReferencedBy")))
    );

    let seq = items[0]
        .descendants()
        .find(|n| n.has_tag_name((RDF_NS, "Seq")))
        .expect("rdf:Seq");
    let persons: Vec<(String, String)> = seq
        .children()
        .filter(|n| n.has_tag_name((RDF_NS, "li")))
        .map(|li| {
            let person = li.first_element_child().expect("foaf:Person");
            assert!(person.has_tag_name((FOAF_NS, "Person")));
            (
                child_text(person, FOAF_NS, "surname")
                    .unwrap_or("")
                    .to_string(),
                child_text(person, FOAF_NS, "givenName")
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect();
    assert_eq!(
        persons,
        [
            ("Lovelace".to_string(), "Ada".to_string()),
            ("Babbage".to_string(), "Charles".to_string()),
            ("de la Tour".to_string(), "Émile".to_string()),
        ]
    );

    let journal = items[1]
        .descendants()
        .find(|n| n.has_tag_name((BIB_NS, "Journal")))
        .expect("bib:Journal container");
    assert_eq!(
        child_text(journal, DC_NS, "title"),
        Some("Journal of Important Results")
    );
    assert_eq!(child_text(journal, PRISM_NS, "volume"), Some("12"));
    assert_eq!(child_text(journal, PRISM_NS, "number"), Some("3"));
    assert_eq!(
        child_text(journal, DC_NS, "identifier"),
        Some("DOI 10.1000/jir.2019.001")
    );
    assert_eq!(
        child_text(items[1], DC_NS, "title"),
        Some("A study of \"things\" & other <matters>")
    );
    assert_eq!(child_text(items[1], Z_NS, "PMID"), Some("12345678"));

    assert_eq!(about(items[3]), "https://example.org/handbook/ch5");
    let uri_value = items[3]
        .descendants()
        .find(|n| n.has_tag_name((RDF_NS, "value")))
        .and_then(|n| n.text());
    assert_eq!(uri_value, Some("https://example.org/handbook/ch5"));
    assert_eq!(about(items[7]), "https://example.org/things");
    assert_eq!(
        about(items[10]),
        "#item_11",
        "a reused URL falls back to an item id"
    );

    let organization = items[5]
        .descendants()
        .find(|n| n.has_tag_name((FOAF_NS, "Organization")))
        .expect("foaf:Organization");
    assert_eq!(
        child_text(organization, FOAF_NS, "name"),
        Some("Massachusetts Institute of Technology")
    );
    assert_eq!(child_text(items[5], Z_NS, "type"), Some("PhD thesis"));
    assert_eq!(
        child_text(items[8], PRISM_NS, "number"),
        Some("arXiv:2201.00001")
    );
}

#[test]
fn issn_makes_the_journal_a_shared_root() {
    let mut export = export();
    export.items[1].issn = Some("1234-5678".to_string());
    let text = zotero_rdf::render(&export);
    assert!(text.contains("<dcterms:isPartOf rdf:resource=\"urn:issn:1234-5678\"/>"));
    assert!(text.contains("<bib:Journal rdf:about=\"urn:issn:1234-5678\">"));
    assert!(text.contains("<dc:identifier>ISSN 1234-5678</dc:identifier>"));
    let doc = roxmltree::Document::parse(&text).expect("well-formed XML");
    let abouts: Vec<&str> = doc
        .root_element()
        .children()
        .filter(roxmltree::Node::is_element)
        .map(|n| n.attribute((RDF_NS, "about")).expect("rdf:about"))
        .collect();
    assert_eq!(abouts.len(), 12);
    assert_eq!(abouts[1], "#item_2");
    assert_eq!(abouts[2], "urn:issn:1234-5678");
}

#[test]
fn csv_matches_golden() {
    assert_eq!(csv::render(&export()), GOLDEN_CSV);
}

#[test]
fn csv_parses_per_rfc_4180() {
    let text = csv::render(&export());
    assert!(text.starts_with('\u{feff}'));
    assert!(!text.ends_with('\n'));
    let rows = parse_csv(text.trim_start_matches('\u{feff}'));
    assert_eq!(rows.len(), 12);
    assert_eq!(rows[0], csv::headers());
    assert_eq!(
        rows[0][..12],
        [
            "Key",
            "Item Type",
            "Publication Year",
            "Author",
            "Title",
            "Publication Title",
            "ISBN",
            "ISSN",
            "DOI",
            "Url",
            "Abstract Note",
            "Date",
        ]
    );
    for row in &rows {
        assert_eq!(row.len(), csv::EXPORTED_FIELDS.len());
    }
    let col = |name: &str| rows[0].iter().position(|h| h == name).expect(name);
    assert_eq!(
        rows[1][col("Title")],
        "Reverse citation export: a worked example"
    );
    assert_eq!(
        rows[1][col("Author")],
        "Lovelace, Ada; Babbage, Charles; de la Tour, Émile"
    );
    assert_eq!(rows[1][col("Publication Year")], "2024");
    assert_eq!(rows[1][col("Date")], "2024");
    assert_eq!(rows[1][col("Manual Tags")], "citations; zotero");
    assert_eq!(
        rows[1][col("Abstract Note")],
        "An abstract with a \"quoted\" phrase & an ampersand. Second paragraph of the abstract."
    );
    assert_eq!(
        rows[2][col("Title")],
        "A study of \"things\" & other <matters>"
    );
    assert_eq!(rows[2][col("DOI")], "10.1000/jir.2019.001");
    assert_eq!(
        rows[2][col("Publication Title")],
        "Journal of Important Results"
    );
    assert_eq!(rows[2][col("Key")].len(), 8);
    assert!(
        rows[2][col("Extra")]
            .starts_with("Reference index: 1 Reference label: [1] Reference text: ")
    );
    assert_eq!(rows[6][col("Item Type")], "thesis");
    assert_eq!(rows[6][col("Type")], "PhD thesis");
    assert_eq!(
        rows[6][col("Publisher")],
        "Massachusetts Institute of Technology"
    );
    assert_eq!(rows[7][col("Number")], "TR-2018-07");
    assert_eq!(rows[9][col("Item Type")], "preprint");
    assert_eq!(rows[9][col("Number")], "arXiv:2201.00001");
    assert_eq!(rows[9][col("Publisher")], "arXiv");
    assert_eq!(rows[10][col("Title")], "A title only the registry knows");
    assert_eq!(rows[10][col("DOI")], "10.1000/linked.9");
    let keys: std::collections::BTreeSet<&String> = rows[1..].iter().map(|r| &r[0]).collect();
    assert_eq!(keys.len(), 11, "keys are unique");
}

/// A minimal RFC 4180 reader: quoted fields, doubled quotes, `\n` records.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' => quoted = true,
                ',' => row.push(std::mem::take(&mut field)),
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                '\r' => {}
                other => field.push(other),
            }
        }
    }
    assert!(!quoted, "unterminated quoted field");
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn dump(conn: &Connection) -> String {
    let mut out = String::new();
    let tables = [
        ("export_meta", "WHERE key != 'generator' ORDER BY key"),
        ("works", "ORDER BY id"),
        ("creators", "ORDER BY work_id, seq"),
        ("identifiers", "ORDER BY work_id, scheme, value"),
        ("tags", "ORDER BY work_id, seq"),
        ("citations", "ORDER BY citing_work_id, cited_work_id"),
    ];
    for (table, clause) in tables {
        let _ = writeln!(out, "== {table}");
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM {table} {clause}"))
            .expect("select");
        let names: Vec<String> = stmt
            .column_names()
            .iter()
            .map(ToString::to_string)
            .collect();
        out.push_str(&names.join("|"));
        out.push('\n');
        let width = names.len();
        let rows = stmt
            .query_map([], |row| {
                let mut cells = Vec::with_capacity(width);
                for i in 0..width {
                    let value: Value = row.get(i)?;
                    cells.push(match value {
                        Value::Null => "NULL".to_string(),
                        Value::Integer(i) => i.to_string(),
                        Value::Real(f) => f.to_string(),
                        Value::Text(t) => t.replace('\n', "\\n"),
                        Value::Blob(b) => format!("blob:{}", b.len()),
                    });
                }
                Ok(cells.join("|"))
            })
            .expect("query");
        for row in rows {
            out.push_str(&row.expect("row"));
            out.push('\n');
        }
    }
    out
}

#[test]
fn sqlite_dump_matches_golden_with_wal_off() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sample.sqlite");
    write_export(&export(), Format::Sqlite, &path, false).expect("sqlite export");
    assert!(!dir.path().join("sample.sqlite-wal").exists());
    assert!(!dir.path().join("sample.sqlite-shm").exists());
    assert!(!dir.path().join("sample.sqlite-journal").exists());
    let conn = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .expect("open read-only");
    let journal: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .expect("journal_mode");
    assert_eq!(journal, "delete");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .expect("user_version");
    assert_eq!(version, sqlite::USER_VERSION);
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .expect("integrity_check");
    assert_eq!(integrity, "ok");
    let violations = conn
        .prepare("PRAGMA foreign_key_check")
        .expect("prepare")
        .query_map([], |_| Ok(()))
        .expect("query")
        .count();
    assert_eq!(violations, 0);
    let generator: String = conn
        .query_row(
            "SELECT value FROM export_meta WHERE key = 'generator'",
            [],
            |r| r.get(0),
        )
        .expect("generator");
    assert!(generator.starts_with("tpe-export "));
    let actual = dump(&conn);
    if actual != GOLDEN_SQLITE {
        eprintln!("--- actual dump ---\n{actual}--- end ---");
    }
    assert_eq!(actual, GOLDEN_SQLITE);
}

#[test]
fn outputs_are_identical_across_runs() {
    let dir = tempfile::tempdir().expect("tempdir");
    for format in Format::ALL {
        let a = dir.path().join(format!("a.{}", format.name()));
        let b = dir.path().join(format!("b.{}", format.name()));
        write_export(&export(), format, &a, false).expect("first");
        write_export(&export(), format, &b, false).expect("second");
        if format == Format::Sqlite {
            // Page content is deterministic, but SQLite stamps a change
            // counter in the header; compare the rows.
            let ca = Connection::open_with_flags(&a, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            let cb = Connection::open_with_flags(&b, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            assert_eq!(dump(&ca), dump(&cb));
        } else {
            assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
        }
    }
}

#[test]
fn cli_refuses_to_overwrite_without_force() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("out.csv");
    let bin = env!("CARGO_BIN_EXE_tpe-export");
    let fixture = fixture_path();
    let run = |extra: &[&str]| {
        Command::new(bin)
            .arg("--format")
            .arg("csv")
            .arg("--input")
            .arg(&fixture)
            .arg("--output")
            .arg(&out)
            .args(extra)
            .output()
            .expect("run tpe-export")
    };
    let first = run(&[]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        String::from_utf8_lossy(&first.stdout).contains("1 article, 10 references"),
        "{}",
        String::from_utf8_lossy(&first.stdout)
    );
    std::fs::write(&out, "sentinel").expect("overwrite with a sentinel");
    let second = run(&[]);
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("output exists"));
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "sentinel");
    let forced = run(&["--force"]);
    assert!(
        forced.status.success(),
        "{}",
        String::from_utf8_lossy(&forced.stderr)
    );
    assert_eq!(std::fs::read_to_string(&out).unwrap(), GOLDEN_CSV);
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, ["out.csv"], "no staging files left behind");

    let bad_format = Command::new(bin)
        .args(["--format", "bibtex", "--input"])
        .arg(&fixture)
        .arg("--output")
        .arg(dir.path().join("x"))
        .output()
        .expect("run");
    assert!(!bad_format.status.success());
    assert!(String::from_utf8_lossy(&bad_format.stderr).contains("unknown format"));

    let missing = Command::new(bin)
        .args(["--format", "csv", "--input"])
        .arg(dir.path().join("missing.json"))
        .arg("--output")
        .arg(dir.path().join("y"))
        .output()
        .expect("run");
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).starts_with("error: "));
    assert!(!dir.path().join("y").exists());
}

#[test]
fn ledger_input_exports_the_same_items_as_json() {
    let result: ExtractionResult = serde_json::from_str(FIXTURE).expect("fixture is a result");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("ledger.sqlite");
    let run = {
        let mut ledger = Ledger::open(&path).expect("open ledger");
        ledger.write_result(&result).expect("write run")
    };
    let expected = export();
    let latest = input::load(&path, &RunSelector::Latest).expect("latest run");
    assert_eq!(Export::from_article(&latest), expected);
    let by_hash =
        input::load(&path, &RunSelector::HashPrefix("9F2C1D".to_string())).expect("by hash prefix");
    assert_eq!(Export::from_article(&by_hash), expected);
    let by_run = input::load(&path, &RunSelector::RunId(run)).expect("by run id");
    assert_eq!(Export::from_article(&by_run), expected);
    let missing = input::load(&path, &RunSelector::HashPrefix("ffff".to_string()));
    assert!(matches!(missing, Err(ExportError::Input(_))), "{missing:?}");
    let no_run = input::load(&path, &RunSelector::RunId(run + 100));
    assert!(matches!(no_run, Err(ExportError::Ledger(_))), "{no_run:?}");

    let out = dir.path().join("from-ledger.rdf");
    let status = Command::new(env!("CARGO_BIN_EXE_tpe-export"))
        .args(["--format", "zotero-rdf", "--hash", "9f2c", "--input"])
        .arg(&path)
        .arg("--output")
        .arg(&out)
        .status()
        .expect("run");
    assert!(status.success());
    assert_eq!(std::fs::read_to_string(&out).unwrap(), GOLDEN_RDF);
}

#[test]
fn foreign_sqlite_files_are_refused_untouched() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("other.sqlite");
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE t (x); INSERT INTO t VALUES (1);")
        .unwrap();
    let error = input::load(&path, &RunSelector::Latest).expect_err("refused");
    assert!(
        error.to_string().contains("not an engine ledger"),
        "{error}"
    );
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 1, "no ledger schema was added");
}

#[test]
fn bibliography_records_and_wrapped_results_are_accepted() {
    let record = r#"{
        "path": "paper.pdf", "sha256": "ab", "status": "found", "extraction_status": "complete",
        "references": [{"index": 1, "label": "1.", "raw": "1. A. Author, Title, J. Ex., 2001.",
            "authors": ["A. Author"], "title": "Title", "year": 2001, "venue": "J. Ex.",
            "volume": null, "issue": null, "pages": null, "doi": null, "arxiv_id": null, "url": null, "page": 3}],
        "paper": {"doi": "10.1000/p", "title": "The paper", "authors": ["Writer, W."], "year": 2002,
            "venue": "Venue", "source": "crossref", "method": "metadata", "score": 1.0},
        "warnings": [], "plausible": true, "elapsed_ms": 1.0, "error": null
    }"#;
    let article = input::parse_json(record.as_bytes()).expect("record parses");
    assert_eq!(article.source_hash.as_deref(), Some("ab"));
    assert_eq!(article.metadata.title.as_deref(), Some("The paper"));
    assert_eq!(article.metadata.authors.len(), 1);
    assert_eq!(article.references.len(), 1);
    let export = Export::from_article(&article);
    assert_eq!(export.items.len(), 2);
    assert_eq!(export.citing().doi.as_deref(), Some("10.1000/p"));
    assert_eq!(
        export.cited()[0].publication_title.as_deref(),
        Some("J. Ex.")
    );

    let failed = r#"{"path": "p.pdf", "sha256": null, "status": "failed", "references": [], "error": "boom"}"#;
    assert!(matches!(
        input::parse_json(failed.as_bytes()),
        Err(ExportError::Input(_))
    ));

    let wrapped = format!("{{\"result\": {FIXTURE}, \"output_paths\": []}}");
    let article = input::parse_json(wrapped.as_bytes()).expect("wrapped result parses");
    assert_eq!(Export::from_article(&article), export_fixture());

    assert!(matches!(
        input::parse_json(b"{\"hello\": 1}"),
        Err(ExportError::Input(_))
    ));
    assert!(matches!(
        input::parse_json(b"not json"),
        Err(ExportError::Json(_))
    ));
}

fn export_fixture() -> Export {
    export()
}
