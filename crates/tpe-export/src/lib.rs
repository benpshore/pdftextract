//! Reverse-citation exports: an extracted article together with the works
//! in its reference list, written in the shapes Zotero itself exports
//! (Zotero RDF and Zotero CSV) or as a small relational `SQLite` database.
//!
//! The input is the engine's JSON output for one document (an
//! [`tpe::schema::ExtractionResult`]), a `tpe bibliography` record, or one
//! run of the engine's `SQLite` ledger. `docs/EXPORT.md` documents the
//! formats, the Zotero sources they follow and the mapping heuristics.
//!
//! Pipeline: [`input::load`] reads the source into an [`input::Article`],
//! [`model::Export::from_article`] maps it onto Zotero items, and one of
//! [`zotero_rdf::render`], [`csv::render`] or [`sqlite::write`] serialises
//! those items. [`write_export`] ties the three together behind the
//! `--force` overwrite rule.

#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::module_name_repetitions,
    clippy::too_many_lines
)]

pub mod classify;
pub mod csv;
pub mod input;
pub mod model;
pub mod names;
pub mod sqlite;
pub mod zotero_rdf;

use std::fmt;
use std::io::Write;
use std::path::Path;
use std::str::FromStr;

use thiserror::Error;

use crate::model::Export;

/// Errors raised while reading an input or writing an export.
#[derive(Debug, Error)]
pub enum ExportError {
    #[error("{path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("ledger: {0}")]
    Ledger(#[from] tpe::ledger::LedgerError),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("input: {0}")]
    Input(String),
    #[error("output exists: {0} (pass --force to replace it)")]
    OutputExists(String),
}

impl ExportError {
    fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.display().to_string(),
            source,
        }
    }
}

/// An export format selectable on the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Zotero RDF (RDF/XML as the Zotero RDF export translator writes it).
    ZoteroRdf,
    /// Zotero CSV (the column set of Zotero's CSV export translator).
    Csv,
    /// A relational `SQLite` database (`sqlite::SCHEMA_SQL`).
    Sqlite,
}

impl Format {
    /// Every format, in the order the CLI lists them.
    pub const ALL: [Format; 3] = [Format::ZoteroRdf, Format::Csv, Format::Sqlite];

    /// The command-line name of the format.
    pub fn name(self) -> &'static str {
        match self {
            Self::ZoteroRdf => "zotero-rdf",
            Self::Csv => "csv",
            Self::Sqlite => "sqlite",
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Format {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|format| format.name() == s)
            .ok_or_else(|| format!("unknown format `{s}` (expected zotero-rdf, csv or sqlite)"))
    }
}

/// Writes `export` as `format` to `output`. The file is produced beside the
/// output path under a temporary name and renamed into place, so a failed
/// export leaves no partial file. An existing `output` is refused unless
/// `force` is set; then the rename replaces it.
pub fn write_export(
    export: &Export,
    format: Format,
    output: &Path,
    force: bool,
) -> Result<(), ExportError> {
    if output.symlink_metadata().is_ok() && !force {
        return Err(ExportError::OutputExists(output.display().to_string()));
    }
    let parent = match output.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let mut staged = tempfile::Builder::new()
        .prefix(".tpe-export-")
        .tempfile_in(parent)
        .map_err(|source| ExportError::io(parent, source))?;
    match format {
        Format::ZoteroRdf => staged
            .write_all(zotero_rdf::render(export).as_bytes())
            .map_err(|source| ExportError::io(staged.path(), source))?,
        Format::Csv => staged
            .write_all(csv::render(export).as_bytes())
            .map_err(|source| ExportError::io(staged.path(), source))?,
        Format::Sqlite => sqlite::write(export, staged.path())?,
    }
    staged
        .flush()
        .map_err(|source| ExportError::io(staged.path(), source))?;
    let persisted = if force {
        staged.persist(output).map_err(|e| e.error)
    } else {
        staged.persist_noclobber(output).map_err(|e| e.error)
    };
    persisted.map_err(|source| ExportError::io(output, source))?;
    Ok(())
}
