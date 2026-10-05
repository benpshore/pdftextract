//! Text extraction from non-PDF inputs, one result per input.
//!
//! [`extract_path`] reads a file (never modified), picks a decoder by extension
//! or by sniffing, and returns a [`FormatsResult`]: plain `text`, a structured
//! view (`sections` of `blocks`, tables as rows, `notes`, `metadata`) and
//! `warnings`. The shapes mirror the engine's `ExtractionResult` where they
//! overlap: a SHA-256 `document.hash`, a `backend` identity, a `status`, flat
//! `warnings`, and text joined by `\n\n` between blocks.
//!
//! Decoders: OOXML `docx`/`pptx`/`xlsx` ([`docx`], [`pptx`], [`xlsx`]),
//! delimited text ([`delimited`]), HTML ([`html`]), Markdown ([`markdown`]),
//! plain text ([`text`]), Apple Pages/Numbers packages ([`iwork`]) and audio
//! through a local speech engine ([`audio`]). Nothing here touches the
//! network; a capability that is missing on this machine is reported as
//! [`Status::Unsupported`] with the reason, never faked.

#![allow(
    clippy::must_use_candidate,
    clippy::module_name_repetitions,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::similar_names,
    clippy::struct_excessive_bools,
    // Zip entry names, not file-system paths: `.xml`/`.iwa` are exact.
    clippy::case_sensitive_file_extension_comparisons
)]

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub mod audio;
pub mod delimited;
pub mod docx;
pub mod html;
pub mod iwa;
pub mod iwork;
pub mod markdown;
pub mod ooxml;
pub mod pptx;
pub mod text;
pub mod xlsx;
pub mod xml;

/// Version of the JSON shape this crate writes. Bump when a stored field
/// changes meaning or is removed.
pub const SCHEMA_VERSION: u32 = 1;

/// Name recorded in `backend.name`.
pub const BACKEND_NAME: &str = "tpe-formats";

/// Errors that make an input fail (as opposed to being unsupported).
#[derive(Debug, thiserror::Error)]
pub enum FormatsError {
    /// Reading the input or writing an output failed.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// The container (zip/package) could not be opened or lacks a required part.
    #[error("container: {0}")]
    Container(String),
    /// An XML part did not parse.
    #[error("xml: {0}")]
    Xml(String),
    /// The input is not what its name says, or is malformed beyond recovery.
    #[error("invalid input: {0}")]
    Invalid(String),
    /// The input's kind is recognised but cannot be decoded here. The message
    /// says exactly why and, when it applies, what to install.
    #[error("unsupported: {0}")]
    Unsupported(String),
}

impl From<zip::result::ZipError> for FormatsError {
    fn from(err: zip::result::ZipError) -> Self {
        Self::Container(err.to_string())
    }
}

impl From<roxmltree::Error> for FormatsError {
    fn from(err: roxmltree::Error) -> Self {
        Self::Xml(err.to_string())
    }
}

/// Outcome of one input. `complete` and `partial` follow the engine's
/// meaning (partial: something was extracted but a warning says what is
/// missing or uncertain); `unsupported` carries its reason in `warnings`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Decoded without a known gap.
    Complete,
    /// Decoded with a gap named in `warnings`.
    Partial,
    /// Nothing usable came out; the error is in `warnings`.
    Failed,
    /// Recognised but not decodable on this machine or by this crate.
    Unsupported,
}

impl Status {
    /// Lower-case label, as serialised.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Unsupported => "unsupported",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The input kinds this crate recognises.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// Word (OOXML) document.
    Docx,
    /// `PowerPoint` (OOXML) presentation.
    Pptx,
    /// Excel (OOXML) workbook.
    Xlsx,
    /// Comma- or otherwise-delimited text (`.csv`).
    Csv,
    /// Tab-separated text (`.tsv`, `.tab`).
    Tsv,
    /// HTML.
    Html,
    /// Markdown.
    Markdown,
    /// Plain text.
    Text,
    /// Apple Pages package.
    Pages,
    /// Apple Numbers package.
    Numbers,
    /// Audio (`wav`, `mp3`, `m4a`).
    Audio,
}

impl Format {
    /// Lower-case label, as serialised.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Pptx => "pptx",
            Self::Xlsx => "xlsx",
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Html => "html",
            Self::Markdown => "markdown",
            Self::Text => "text",
            Self::Pages => "pages",
            Self::Numbers => "numbers",
            Self::Audio => "audio",
        }
    }

    /// The format a file-name extension selects, if any.
    pub fn from_extension(ext: &str) -> Option<Self> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "docx" | "docm" | "dotx" => Self::Docx,
            "pptx" | "pptm" | "potx" => Self::Pptx,
            "xlsx" | "xlsm" | "xltx" => Self::Xlsx,
            "csv" => Self::Csv,
            "tsv" | "tab" => Self::Tsv,
            "html" | "htm" | "xhtml" => Self::Html,
            "md" | "markdown" | "mdown" | "mkd" => Self::Markdown,
            "txt" | "text" | "log" => Self::Text,
            "pages" => Self::Pages,
            "numbers" => Self::Numbers,
            "wav" | "mp3" | "m4a" => Self::Audio,
            _ => return None,
        })
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One unit of content: a paragraph, heading, list item, table, code block or
/// quotation. `text` is always filled (a table's text is its rows joined by
/// `\n`, cells by `\t`); `rows` is set for tables only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    /// `paragraph`, `heading`, `list_item`, `table`, `code` or `quote`.
    pub kind: String,
    /// Heading level (1-based) or list nesting depth (0-based); `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    /// The block's text; tables are rendered tab-separated.
    pub text: String,
    /// Table cells, row-major, as displayed. Rows may differ in length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<Vec<Vec<String>>>,
}

impl Block {
    /// A block of `kind` with `text` and no level.
    pub fn new(kind: &str, text: impl Into<String>) -> Self {
        Self {
            kind: kind.to_string(),
            level: None,
            text: text.into(),
            rows: None,
        }
    }

    /// A paragraph.
    pub fn paragraph(text: impl Into<String>) -> Self {
        Self::new("paragraph", text)
    }

    /// A heading of `level` (1-based).
    pub fn heading(level: u8, text: impl Into<String>) -> Self {
        Self {
            level: Some(level),
            ..Self::new("heading", text)
        }
    }

    /// A list item at nesting `depth` (0-based).
    pub fn list_item(depth: u8, text: impl Into<String>) -> Self {
        Self {
            level: Some(depth),
            ..Self::new("list_item", text)
        }
    }

    /// A table from rows; `text` is the tab-separated rendering.
    pub fn table(rows: Vec<Vec<String>>) -> Self {
        let text = rows
            .iter()
            .map(|row| row.join("\t"))
            .collect::<Vec<_>>()
            .join("\n");
        Self {
            kind: "table".to_string(),
            level: None,
            text,
            rows: Some(rows),
        }
    }

    /// Whether the block carries no text at all.
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// A run of blocks with a role: a document body, a slide, a sheet, a header
/// or footer, a transcript. `index` is 0-based within the input.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    /// `body`, `slide`, `sheet`, `header`, `footer`, `front_matter`, `table`,
    /// `transcript` or `package`.
    pub kind: String,
    /// 0-based position among sections of the input.
    pub index: u32,
    /// Slide title, sheet name, header kind, and so on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub blocks: Vec<Block>,
}

impl Section {
    /// An empty section of `kind` at `index`.
    pub fn new(kind: &str, index: u32, title: Option<String>) -> Self {
        Self {
            kind: kind.to_string(),
            index,
            title,
            blocks: Vec::new(),
        }
    }
}

/// Text attached to the document rather than in its flow: speaker notes,
/// footnotes, endnotes, comments.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// `speaker_notes`, `footnote`, `endnote` or `comment`.
    pub kind: String,
    /// What the note belongs to: a slide index (`slide 3`), a footnote id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    pub text: String,
}

/// Where the bytes came from and their identity, as the engine records it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentIdentity {
    /// Lower-case hex SHA-256 of the input file (for a package directory, of
    /// its `Index/Document.iwa` when present, else of the sorted file list).
    pub hash: String,
    /// Bytes hashed.
    pub size: u64,
    /// The path as given on the command line.
    pub sources: Vec<String>,
}

/// Identity of the code (and local engine, for audio) that produced a result.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendIdentity {
    /// Always [`BACKEND_NAME`].
    pub name: String,
    /// This crate's version.
    pub version: String,
    /// External engine used, e.g. `whisper-cli` with its model path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
}

impl BackendIdentity {
    /// This crate, with an optional external engine label.
    pub fn new(engine: Option<String>) -> Self {
        Self {
            name: BACKEND_NAME.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            engine,
        }
    }
}

/// Everything produced for one input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatsResult {
    pub schema_version: u32,
    pub document: DocumentIdentity,
    pub backend: BackendIdentity,
    pub format: Format,
    pub status: Status,
    /// Document title when the input states one (core properties, `<title>`,
    /// front matter) or shows one (a `Title` paragraph, the first heading).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// String-valued properties as found: core properties, `<meta>` tags,
    /// front matter, delimiter/encoding, engine details. Never guessed.
    pub metadata: BTreeMap<String, String>,
    pub sections: Vec<Section>,
    pub notes: Vec<Note>,
    /// Diagnostics. `unsupported:` and `failed:` prefixes carry the status.
    pub warnings: Vec<String>,
    /// Plain text: sections in order, blocks joined by `\n\n`, then notes.
    pub text: String,
    /// Bytes to write next to the outputs as `<stem>.<suffix>` (for example a
    /// Pages package's `preview.pdf`). Not serialised.
    #[serde(skip)]
    pub extra_files: Vec<ExtraFile>,
}

/// A file copied out of the input for another tool to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtraFile {
    /// Appended to the stem: `preview.pdf` gives `<stem>.preview.pdf`.
    pub suffix: String,
    pub bytes: Vec<u8>,
}

impl FormatsResult {
    /// A result for `format` with identity filled and nothing extracted yet.
    pub fn empty(format: Format, document: DocumentIdentity) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            document,
            backend: BackendIdentity::new(None),
            format,
            status: Status::Complete,
            title: None,
            metadata: BTreeMap::new(),
            sections: Vec::new(),
            notes: Vec::new(),
            warnings: Vec::new(),
            text: String::new(),
            extra_files: Vec::new(),
        }
    }

    /// An unsupported result carrying `reason` (prefixed `unsupported:`).
    pub fn unsupported(format: Format, document: DocumentIdentity, reason: &str) -> Self {
        let mut result = Self::empty(format, document);
        result.status = Status::Unsupported;
        result.warnings.push(format!("unsupported: {reason}"));
        result
    }

    /// Add a warning; `partial:` warnings downgrade a complete result.
    pub fn warn(&mut self, message: impl Into<String>) {
        let message = message.into();
        if message.starts_with("partial:") && self.status == Status::Complete {
            self.status = Status::Partial;
        }
        self.warnings.push(message);
    }

    /// Fill `text` from `sections` and `notes`, and drop empty blocks.
    pub fn finish(&mut self) {
        for section in &mut self.sections {
            section.blocks.retain(|block| !block.is_empty());
        }
        let mut parts: Vec<String> = Vec::new();
        for section in &self.sections {
            let mut piece = String::new();
            if let Some(title) = section.title.as_deref().filter(|t| !t.trim().is_empty()) {
                piece.push_str(title.trim());
            }
            for block in &section.blocks {
                if !piece.is_empty() {
                    piece.push_str("\n\n");
                }
                piece.push_str(block.text.trim_end());
            }
            if !piece.is_empty() {
                parts.push(piece);
            }
        }
        for note in &self.notes {
            if note.text.trim().is_empty() {
                continue;
            }
            let label = match note.anchor.as_deref() {
                Some(anchor) => format!("[{} {}]", note.kind.replace('_', " "), anchor),
                None => format!("[{}]", note.kind.replace('_', " ")),
            };
            parts.push(format!("{label}\n{}", note.text.trim_end()));
        }
        self.text = parts.join("\n\n");
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        if self.status == Status::Complete && self.text.trim().is_empty() {
            self.warn("partial: no text found");
        }
    }
}

/// Lower-case hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Identity for a regular file: hash and size of its bytes.
pub fn identity_of(path: &Path, bytes: &[u8]) -> DocumentIdentity {
    DocumentIdentity {
        hash: sha256_hex(bytes),
        size: bytes.len() as u64,
        sources: vec![path.display().to_string()],
    }
}

/// Decide the format of `path` from its extension, else by sniffing `head`
/// (the first bytes of the file). `None` means nothing here can read it.
pub fn detect_format(path: &Path, head: &[u8]) -> Option<Format> {
    if let Some(format) = path
        .extension()
        .and_then(|e| e.to_str())
        .and_then(Format::from_extension)
    {
        return Some(format);
    }
    if path.is_dir() {
        return None;
    }
    sniff(head)
}

fn sniff(head: &[u8]) -> Option<Format> {
    if head.starts_with(b"PK\x03\x04") {
        return ooxml::sniff_zip_kind(head);
    }
    if head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WAVE") {
        return Some(Format::Audio);
    }
    if head.starts_with(b"ID3") || head.get(4..8) == Some(b"ftyp") {
        return Some(Format::Audio);
    }
    let lower: String = head
        .iter()
        .take(512)
        .map(|b| b.to_ascii_lowercase() as char)
        .collect();
    let trimmed = lower.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
    if trimmed.starts_with("<!doctype html") || trimmed.starts_with("<html") {
        return Some(Format::Html);
    }
    if std::str::from_utf8(head).is_ok() || head.is_empty() {
        return Some(Format::Text);
    }
    None
}

/// Options that steer extraction.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// How to find a local speech engine; `None` uses the process environment.
    pub audio: Option<audio::EngineEnv>,
}

/// Read `path` (never modified) and extract it. A recognised-but-unreadable
/// input returns `Ok` with [`Status::Unsupported`]; an unknown kind or a
/// malformed file is an `Err`.
pub fn extract_path(path: &Path, options: &Options) -> Result<FormatsResult, FormatsError> {
    let path_string = path.display().to_string();
    if path.is_dir() {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        return match Format::from_extension(ext) {
            Some(Format::Pages) => iwork::extract_package(path, Format::Pages),
            Some(Format::Numbers) => iwork::extract_package(path, Format::Numbers),
            _ => Err(FormatsError::Invalid(format!(
                "{path_string} is a directory (pass --recursive to walk it)"
            ))),
        };
    }
    let bytes = fs::read(path)?;
    let Some(format) = detect_format(path, &bytes[..bytes.len().min(4096)]) else {
        return Err(FormatsError::Unsupported(format!(
            "{path_string}: no decoder for this file (unknown extension and no recognised signature)"
        )));
    };
    let identity = identity_of(path, &bytes);
    let mut result = match format {
        Format::Docx => docx::extract(&bytes, identity)?,
        Format::Pptx => pptx::extract(&bytes, identity)?,
        Format::Xlsx => xlsx::extract(&bytes, identity)?,
        Format::Csv => delimited::extract(&bytes, identity, Format::Csv),
        Format::Tsv => delimited::extract(&bytes, identity, Format::Tsv),
        Format::Html => html::extract(&bytes, identity),
        Format::Markdown => markdown::extract(&bytes, identity),
        Format::Text => text::extract(&bytes, identity),
        Format::Pages | Format::Numbers => iwork::extract_bytes(&bytes, identity, format)?,
        Format::Audio => {
            let env = options
                .audio
                .clone()
                .unwrap_or_else(audio::EngineEnv::from_process);
            audio::extract(path, &bytes, identity, &env)?
        }
    };
    result.finish();
    Ok(result)
}

/// Output file paths for one input written into `out_dir` under `stem`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outputs {
    /// `<stem>.json`, always written.
    pub json: PathBuf,
    /// `<stem>.txt`, written when the status is complete or partial.
    pub txt: Option<PathBuf>,
    /// Files copied out of the input (`<stem>.preview.pdf`).
    pub extra: Vec<PathBuf>,
}

/// Write `<stem>.json` (and `<stem>.txt` when the status is complete or
/// partial) into `out_dir`. Without `force`, outputs that already exist are
/// left alone and `Ok(None)` says nothing was written.
pub fn write_outputs(
    result: &FormatsResult,
    out_dir: &Path,
    stem: &str,
    force: bool,
) -> Result<Option<Outputs>, FormatsError> {
    fs::create_dir_all(out_dir)?;
    let json_path = out_dir.join(format!("{stem}.json"));
    let txt_path = out_dir.join(format!("{stem}.txt"));
    let writes_txt = matches!(result.status, Status::Complete | Status::Partial);
    if !force && json_path.exists() && (!writes_txt || txt_path.exists()) {
        return Ok(None);
    }
    let json = serde_json::to_string_pretty(result)
        .map_err(|e| FormatsError::Invalid(format!("serialising result: {e}")))?;
    fs::write(&json_path, json)?;
    let txt = if writes_txt {
        fs::write(&txt_path, &result.text)?;
        Some(txt_path)
    } else {
        None
    };
    let mut extra = Vec::new();
    for file in &result.extra_files {
        let path = out_dir.join(format!("{stem}.{}", file.suffix));
        if path.exists() && !force {
            continue;
        }
        fs::write(&path, &file.bytes)?;
        extra.push(path);
    }
    Ok(Some(Outputs {
        json: json_path,
        txt,
        extra,
    }))
}
