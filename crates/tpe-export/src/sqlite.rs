//! `SQLite` export: works, creators, identifiers, tags and citation edges.
//!
//! The database is written with WAL off (`journal_mode = DELETE`, the
//! rollback journal) inside one transaction, so a failed export leaves no
//! half-written file and the result is a single file with no `-wal` or
//! `-shm` companions. `user_version` is [`USER_VERSION`].

use std::path::Path;

use rusqlite::{Connection, params};

use crate::ExportError;
use crate::model::{Export, Item};

/// `PRAGMA user_version` of the databases this crate writes.
pub const USER_VERSION: i64 = 1;

/// The schema, verbatim.
pub const SCHEMA_SQL: &str = "\
CREATE TABLE export_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE works (
    id INTEGER PRIMARY KEY,
    role TEXT NOT NULL CHECK (role IN ('citing', 'cited')),
    zotero_key TEXT NOT NULL UNIQUE,
    item_type TEXT NOT NULL,
    reference_index INTEGER,
    label TEXT,
    title TEXT,
    abstract TEXT,
    publication_title TEXT,
    publisher TEXT,
    type TEXT,
    number TEXT,
    date TEXT,
    year INTEGER,
    volume TEXT,
    issue TEXT,
    pages TEXT,
    extra TEXT,
    raw TEXT
);
CREATE TABLE creators (
    work_id INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    creator_type TEXT NOT NULL,
    last_name TEXT NOT NULL,
    first_name TEXT,
    PRIMARY KEY (work_id, seq)
);
CREATE TABLE identifiers (
    work_id INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    scheme TEXT NOT NULL CHECK (scheme IN ('doi', 'issn', 'isbn', 'url', 'arxiv', 'pmid', 'pmcid')),
    value TEXT NOT NULL,
    PRIMARY KEY (work_id, scheme, value)
);
CREATE INDEX identifiers_value ON identifiers(scheme, value);
CREATE TABLE tags (
    work_id INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    tag TEXT NOT NULL,
    PRIMARY KEY (work_id, seq)
);
CREATE TABLE citations (
    citing_work_id INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    cited_work_id INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    reference_index INTEGER NOT NULL,
    label TEXT,
    PRIMARY KEY (citing_work_id, cited_work_id)
);
";

/// Writes `export` to a new database at `path` in one transaction.
pub fn write(export: &Export, path: &Path) -> Result<(), ExportError> {
    let mut conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = DELETE; PRAGMA synchronous = FULL; PRAGMA foreign_keys = ON;",
    )?;
    let tx = conn.transaction()?;
    tx.execute_batch(SCHEMA_SQL)?;
    tx.execute_batch(&format!("PRAGMA user_version = {USER_VERSION};"))?;
    {
        let mut meta = tx.prepare("INSERT INTO export_meta (key, value) VALUES (?1, ?2)")?;
        meta.execute(params![
            "generator",
            concat!("tpe-export ", env!("CARGO_PKG_VERSION"))
        ])?;
        meta.execute(params!["schema_version", USER_VERSION.to_string()])?;
        meta.execute(params!["zotero_schema_version", "45"])?;
        if let Some(hash) = &export.source_hash {
            meta.execute(params!["source_sha256", hash])?;
        }
        meta.execute(params!["work_count", export.items.len().to_string()])?;
    }
    {
        let mut work = tx.prepare(
            "INSERT INTO works (id, role, zotero_key, item_type, reference_index, label, title, \
             abstract, publication_title, publisher, type, number, date, year, volume, issue, \
             pages, extra, raw) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, \
             ?14, ?15, ?16, ?17, ?18, ?19)",
        )?;
        let mut creator = tx.prepare(
            "INSERT INTO creators (work_id, seq, creator_type, last_name, first_name) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        let mut identifier =
            tx.prepare("INSERT INTO identifiers (work_id, scheme, value) VALUES (?1, ?2, ?3)")?;
        let mut tag = tx.prepare("INSERT INTO tags (work_id, seq, tag) VALUES (?1, ?2, ?3)")?;
        let mut citation = tx.prepare(
            "INSERT INTO citations (citing_work_id, cited_work_id, reference_index, label) \
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        let citing = export.citing().id;
        for item in &export.items {
            let role = if item.reference_index.is_some() {
                "cited"
            } else {
                "citing"
            };
            work.execute(params![
                item.id,
                role,
                item.key,
                item.item_type.zotero_name(),
                item.reference_index,
                item.label,
                item.title,
                item.abstract_note,
                item.publication_title,
                item.publisher,
                item.type_field,
                item.number,
                item.date,
                item.year(),
                item.volume,
                item.issue,
                item.pages,
                item.extra,
                item.raw,
            ])?;
            for (seq, c) in (0_u32..).zip(&item.creators) {
                creator.execute(params![item.id, seq, "author", c.last_name, c.first_name])?;
            }
            for (scheme, value) in identifiers(item) {
                identifier.execute(params![item.id, scheme, value])?;
            }
            for (seq, t) in (0_u32..).zip(&item.tags) {
                tag.execute(params![item.id, seq, t])?;
            }
            if let Some(index) = item.reference_index {
                citation.execute(params![citing, item.id, index, item.label])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

/// The identifiers of an item in a fixed scheme order.
pub fn identifiers(item: &Item) -> Vec<(&'static str, &str)> {
    let mut out = Vec::new();
    let pairs = [
        ("doi", &item.doi),
        ("issn", &item.issn),
        ("isbn", &item.isbn),
        ("url", &item.url),
        ("arxiv", &item.arxiv_id),
        ("pmid", &item.pmid),
        ("pmcid", &item.pmcid),
    ];
    for (scheme, value) in pairs {
        if let Some(value) = value {
            out.push((scheme, value.as_str()));
        }
    }
    out
}
