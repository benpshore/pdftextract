//! Apple Pages and Numbers packages. A document is a zip (or a directory
//! bundle) whose content lives in `Index/*.iwa`: Snappy-framed protobuf
//! messages whose schema Apple does not publish. [`crate::iwa`] decodes the
//! container and the wire format; this module reads the two message kinds
//! whose text fields are stable across versions and labels the result as a
//! partial, schema-less decode. The package inventory, `preview.pdf` copy
//! and a precise `unsupported` reason cover everything else.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::iwa::{self, Field, Message};
use crate::ooxml::Package;
use crate::xml;
use crate::{
    Block, DocumentIdentity, ExtraFile, Format, FormatsError, FormatsResult, Section, Status,
};

/// `TSWP.StorageArchive`: a text storage (body, header, footer, text box,
/// footnote, rich cell). Field 3 is `repeated string text`.
pub const TYPE_STORAGE_ARCHIVE: u32 = 2001;
/// `TST.TableDataList`: a Numbers table's value list. Field 1 is the list
/// type (1 = strings), field 3 the entries, whose field 3 is the string.
pub const TYPE_TABLE_DATA_LIST: u32 = 6005;

/// The entries of a package, whether it is a zip or a directory bundle.
trait Entries {
    fn names(&self) -> Vec<String>;
    fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, FormatsError>;
}

impl Entries for Package {
    fn names(&self) -> Vec<String> {
        Package::names(self).to_vec()
    }

    fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, FormatsError> {
        Package::read(self, name)
    }
}

struct Directory {
    root: std::path::PathBuf,
    names: Vec<String>,
}

impl Entries for Directory {
    fn names(&self) -> Vec<String> {
        self.names.clone()
    }

    fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, FormatsError> {
        if !self.names.iter().any(|n| n == name) {
            return Ok(None);
        }
        Ok(Some(fs::read(self.root.join(name))?))
    }
}

/// Extract a `.pages`/`.numbers` file that is a zip archive.
pub fn extract_bytes(
    bytes: &[u8],
    identity: DocumentIdentity,
    format: Format,
) -> Result<FormatsResult, FormatsError> {
    let mut package = Package::open(bytes).map_err(|e| {
        FormatsError::Invalid(format!(
            "{} is neither a zip package nor a directory bundle: {e}",
            format.as_str()
        ))
    })?;
    describe(&mut package, identity, format)
}

/// Extract a `.pages`/`.numbers` directory bundle (how macOS stores them when
/// "package" saving is on).
pub fn extract_package(dir: &Path, format: Format) -> Result<FormatsResult, FormatsError> {
    let mut names = Vec::new();
    for entry in walkdir::WalkDir::new(dir).sort_by_file_name() {
        let entry = entry.map_err(|e| FormatsError::Io(e.into()))?;
        if entry.file_type().is_file()
            && let Ok(rel) = entry.path().strip_prefix(dir)
        {
            names.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    let mut directory = Directory {
        root: dir.to_path_buf(),
        names,
    };
    // Identity: the main IWA when present, else the sorted entry list.
    let (hash, size) = if let Some(bytes) = directory.read("Index/Document.iwa")? {
        (crate::sha256_hex(&bytes), bytes.len() as u64)
    } else {
        let listing = directory.names.join("\n");
        (crate::sha256_hex(listing.as_bytes()), 0)
    };
    let identity = DocumentIdentity {
        hash,
        size,
        sources: vec![dir.display().to_string()],
    };
    describe(&mut directory, identity, format)
}

/// What the schema-less decode of `Index/*.iwa` produced.
struct Decoded {
    messages: usize,
    /// Text storages (`TSWP.StorageArchive`), each a list of paragraphs.
    storages: Vec<Vec<String>>,
    /// String lists of Numbers tables (`TST.TableDataList`, type 1).
    string_lists: Vec<Vec<String>>,
}

fn decode_iwa(entries: &mut dyn Entries, names: &[String]) -> Result<Decoded, String> {
    let mut decoded = Decoded {
        messages: 0,
        storages: Vec::new(),
        string_lists: Vec::new(),
    };
    for name in names {
        let Some(bytes) = entries.read(name).map_err(|e| e.to_string())? else {
            continue;
        };
        let stream = iwa::iwa_decompress(&bytes).map_err(|e| format!("{name}: {e}"))?;
        let messages = iwa::parse_archives(&stream).map_err(|e| format!("{name}: {e}"))?;
        decoded.messages += messages.len();
        for message in &messages {
            match message.type_id {
                TYPE_STORAGE_ARCHIVE => {
                    let paragraphs = storage_paragraphs(message);
                    if !paragraphs.is_empty() {
                        decoded.storages.push(paragraphs);
                    }
                }
                TYPE_TABLE_DATA_LIST => {
                    let strings = data_list_strings(message);
                    if !strings.is_empty() {
                        decoded.string_lists.push(strings);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(decoded)
}

/// Paragraphs of a `TSWP.StorageArchive`: its `text` strings split on line
/// and paragraph separators, with attachment and private-use placeholders
/// (page numbers, inline objects) removed.
fn storage_paragraphs(message: &Message) -> Vec<String> {
    let Ok(fields) = iwa::fields(&message.payload) else {
        return Vec::new();
    };
    let mut paragraphs = Vec::new();
    for (number, value) in fields {
        let (3, Field::Bytes(bytes)) = (number, value) else {
            continue;
        };
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue;
        };
        for paragraph in text.split(['\n', '\r', '\u{2029}', '\u{2028}']) {
            let cleaned: String = paragraph
                .chars()
                .filter(|c| *c != '\u{fffc}' && !('\u{e000}'..='\u{f8ff}').contains(c))
                .collect();
            if !cleaned.trim().is_empty() {
                paragraphs.push(cleaned.trim_end().to_string());
            }
        }
    }
    paragraphs
}

/// Strings of a `TST.TableDataList` whose list type is 1 (strings).
fn data_list_strings(message: &Message) -> Vec<String> {
    let Ok(fields) = iwa::fields(&message.payload) else {
        return Vec::new();
    };
    if !fields.contains(&(1, Field::Varint(1))) {
        return Vec::new();
    }
    let mut strings = Vec::new();
    for (number, value) in fields {
        let (3, Field::Bytes(entry)) = (number, value) else {
            continue;
        };
        let Ok(entry_fields) = iwa::fields(entry) else {
            continue;
        };
        for (n, v) in entry_fields {
            if let (3, Field::Bytes(bytes)) = (n, v)
                && let Ok(text) = std::str::from_utf8(bytes)
                && !text.trim().is_empty()
            {
                strings.push(text.to_string());
            }
        }
    }
    strings
}

fn describe(
    entries: &mut dyn Entries,
    identity: DocumentIdentity,
    format: Format,
) -> Result<FormatsResult, FormatsError> {
    let names = entries.names();
    let iwa_names: Vec<String> = names
        .iter()
        .filter(|n| n.starts_with("Index/") && n.ends_with(".iwa"))
        .cloned()
        .collect();
    let has_document = names.iter().any(|n| n == "Index/Document.iwa");
    let app = match format {
        Format::Numbers => "Numbers",
        _ => "Pages",
    };

    let mut result = FormatsResult::empty(format, identity);
    let mut metadata: BTreeMap<String, String> = BTreeMap::new();
    metadata.insert("package.entries".to_string(), names.len().to_string());
    metadata.insert("package.iwa_files".to_string(), iwa_names.len().to_string());
    if let Some(id) = entries.read("Metadata/DocumentIdentifier")? {
        let id = String::from_utf8_lossy(&id).trim().to_string();
        if !id.is_empty() {
            metadata.insert("document_identifier".to_string(), id);
        }
    }
    if let Some(plist) = entries.read("Metadata/BuildVersionHistory.plist")? {
        let strings = plist_strings(&plist);
        for value in &strings {
            if let Some(template) = value.strip_prefix("Template: ") {
                metadata.insert("template".to_string(), template.trim().to_string());
            }
        }
        let builds: Vec<&str> = strings
            .iter()
            .filter(|s| !s.starts_with("Template:"))
            .map(String::as_str)
            .collect();
        if !builds.is_empty() {
            metadata.insert("build_versions".to_string(), builds.join(", "));
        }
    }
    let preview = entries
        .read("preview.pdf")?
        .filter(|pdf| pdf.starts_with(b"%PDF"));
    metadata.insert(
        "preview_pdf".to_string(),
        if preview.is_some() {
            "preview.pdf"
        } else {
            "absent"
        }
        .to_string(),
    );

    if has_document {
        match decode_iwa(entries, &iwa_names) {
            Ok(decoded) => {
                metadata.insert("iwa.messages".to_string(), decoded.messages.to_string());
                fill_sections(&mut result, &decoded, app);
                if result.sections.is_empty() {
                    mark_unsupported(
                        &mut result,
                        &format!(
                            "{app}: decoded {} messages in Index/*.iwa but found no TSWP.StorageArchive ({TYPE_STORAGE_ARCHIVE}) text{}; this document's message types are not known to tpe-formats",
                            decoded.messages,
                            if format == Format::Numbers {
                                format!(" or TST.TableDataList ({TYPE_TABLE_DATA_LIST}) strings")
                            } else {
                                String::new()
                            }
                        ),
                    );
                }
            }
            Err(e) => mark_unsupported(
                &mut result,
                &format!("{app} Index/*.iwa could not be decoded: {e}"),
            ),
        }
    } else {
        mark_unsupported(
            &mut result,
            &format!(
                "{app} package has no Index/Document.iwa ({} entries, {} .iwa files); not a current-format {app} document",
                names.len(),
                iwa_names.len()
            ),
        );
    }

    if let Some(pdf) = preview {
        result.extra_files.push(ExtraFile {
            suffix: "preview.pdf".to_string(),
            bytes: pdf,
        });
        result.warn(
            "preview.pdf copied next to the outputs as <stem>.preview.pdf; it is the author's saved preview, not necessarily the whole document",
        );
    }
    let index = result.sections.len() as u32;
    let mut listing = Section::new("package", index, Some(format!("{app} package")));
    listing.blocks.push(Block::new(
        "code",
        names
            .iter()
            .take(200)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n"),
    ));
    if result.status == Status::Unsupported {
        result.sections.push(listing);
    }
    result.metadata = metadata;
    Ok(result)
}

fn mark_unsupported(result: &mut FormatsResult, reason: &str) {
    result.status = Status::Unsupported;
    result.warnings.insert(0, format!("unsupported: {reason}"));
}

/// Text storages and string lists as sections, with the warning that says
/// exactly how much of the document this is.
fn fill_sections(result: &mut FormatsResult, decoded: &Decoded, app: &str) {
    for paragraphs in &decoded.storages {
        let index = result.sections.len() as u32;
        let mut section = Section::new("text_storage", index, None);
        section.blocks = paragraphs.iter().map(Block::paragraph).collect();
        result.sections.push(section);
    }
    for strings in &decoded.string_lists {
        let index = result.sections.len() as u32;
        let mut section = Section::new("cell_strings", index, None);
        section.blocks = strings.iter().map(Block::paragraph).collect();
        result.sections.push(section);
    }
    if !decoded.storages.is_empty() {
        result.warn(format!(
            "partial: {app} IWA decoded without Apple's schema: text from TSWP.StorageArchive messages ({TYPE_STORAGE_ARCHIVE}) in archive order; body, headers, footers, footnotes and text boxes are not told apart; styles, lists and tables are not decoded; checked on synthetic fixtures only"
        ));
    }
    if !decoded.string_lists.is_empty() {
        result.warn(format!(
            "partial: {app} cell strings from TST.TableDataList ({TYPE_TABLE_DATA_LIST}) string lists in storage order; grid positions, numbers, dates and formula results are not decoded; checked on synthetic fixtures only"
        ));
    }
}

/// Every `<string>` in an XML property list; empty for binary plists.
fn plist_strings(bytes: &[u8]) -> Vec<String> {
    let Ok(source) = std::str::from_utf8(bytes) else {
        return Vec::new();
    };
    if !source.trim_start().starts_with("<?xml") && !source.trim_start().starts_with("<plist") {
        return Vec::new();
    }
    let Ok(doc) = xml::parse("BuildVersionHistory.plist", source) else {
        return Vec::new();
    };
    doc.root_element()
        .descendants()
        .filter(|n| xml::is(*n, "string"))
        .map(xml::text_of)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}
