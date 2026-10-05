//! Reading the article to export: an engine JSON result, a `tpe
//! bibliography` record, or a run from the `SQLite` ledger.

use std::fs;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use tpe::ledger::Ledger;
use tpe::schema::{Author, ExtractionResult, Metadata, ReferenceEntry, Resolved};

use crate::ExportError;

/// The 16-byte header every `SQLite` database file starts with.
const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";

/// What an export is built from: the paper's metadata and its reference list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Article {
    /// SHA-256 of the source document, when the input carried it.
    pub source_hash: Option<String>,
    pub metadata: Metadata,
    pub references: Vec<ReferenceEntry>,
}

impl Article {
    /// The article of one extraction result.
    pub fn from_result(result: ExtractionResult) -> Self {
        Self {
            source_hash: Some(result.document.hash.0),
            metadata: result.metadata,
            references: result.references,
        }
    }
}

/// Which run of a ledger to export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunSelector {
    /// The most recently finished run of any document.
    Latest,
    /// The most recently finished run whose document hash starts with the
    /// lower-case hex prefix.
    HashPrefix(String),
    /// A run by its row id.
    RunId(i64),
}

/// The subset of a `tpe bibliography` record (`docs/BIBLIOGRAPHY.md`) an
/// export needs. Unknown fields are ignored; missing ones default.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BibliographyRecord {
    status: String,
    sha256: Option<String>,
    references: Vec<ReferenceEntry>,
    paper: Option<Resolved>,
}

impl BibliographyRecord {
    fn into_article(self) -> Article {
        let metadata = self
            .paper
            .map(|paper| Metadata {
                title: paper.title,
                authors: paper
                    .authors
                    .into_iter()
                    .map(|name| Author {
                        name,
                        ..Author::default()
                    })
                    .collect(),
                doi: paper.doi,
                year: paper.year,
                venue: paper.venue,
                ..Metadata::default()
            })
            .unwrap_or_default();
        Article {
            source_hash: self.sha256,
            metadata,
            references: self.references,
        }
    }
}

/// Reads `path`, sniffing a `SQLite` ledger by its file header and
/// otherwise parsing JSON. `selector` only applies to ledgers.
pub fn load(path: &Path, selector: &RunSelector) -> Result<Article, ExportError> {
    let bytes = fs::read(path).map_err(|source| ExportError::io(path, source))?;
    if bytes.starts_with(SQLITE_MAGIC) {
        load_ledger(path, selector)
    } else {
        parse_json(&bytes)
    }
}

/// Parses the engine's JSON output: an extraction result (`<hash>.json`
/// from `tpe extract --out`, or the object `tpe extract --json` prints)
/// or one `tpe bibliography` record.
pub fn parse_json(bytes: &[u8]) -> Result<Article, ExportError> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if let Some(result) = value.get("result").filter(|r| is_extraction_result(r)) {
        let result: ExtractionResult = serde_json::from_value(result.clone())?;
        return Ok(Article::from_result(result));
    }
    if is_extraction_result(&value) {
        let result: ExtractionResult = serde_json::from_value(value)?;
        return Ok(Article::from_result(result));
    }
    if value
        .get("references")
        .is_some_and(serde_json::Value::is_array)
        && value
            .get("status")
            .is_some_and(serde_json::Value::is_string)
    {
        let record: BibliographyRecord = serde_json::from_value(value)?;
        if record.status == "failed" {
            return Err(ExportError::Input(
                "the bibliography record is a failed scan; nothing to export".to_string(),
            ));
        }
        return Ok(record.into_article());
    }
    Err(ExportError::Input(
        "not an engine extraction result or a tpe bibliography record".to_string(),
    ))
}

fn is_extraction_result(value: &serde_json::Value) -> bool {
    value.get("schema_version").is_some()
        && value.get("document").is_some()
        && value.get("metadata").is_some()
        && value.get("references").is_some()
}

/// Loads one run from a ledger. The file is first checked read-only for
/// the ledger tables so a foreign database is never given a schema.
pub fn load_ledger(path: &Path, selector: &RunSelector) -> Result<Article, ExportError> {
    {
        let probe = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let tables: i64 = probe.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' \
             AND name IN ('schema_meta', 'runs', 'metadata', 'references')",
            [],
            |row| row.get(0),
        )?;
        if tables != 4 {
            return Err(ExportError::Input(format!(
                "{} is a SQLite database but not an engine ledger",
                path.display()
            )));
        }
    }
    let ledger = Ledger::open(path)?;
    let run = match selector {
        RunSelector::RunId(id) => *id,
        RunSelector::HashPrefix(prefix) => ledger
            .latest_run_for_prefix(&prefix.to_ascii_lowercase())?
            .ok_or_else(|| ExportError::Input(format!("no run for hash prefix {prefix}")))?,
        RunSelector::Latest => ledger
            .latest_run_for_prefix("")?
            .ok_or_else(|| ExportError::Input("the ledger has no runs".to_string()))?,
    };
    let result = ledger.load_result(run)?;
    Ok(Article::from_result(result))
}
