//! Shared OOXML plumbing: opening the zip container, reading parts, resolving
//! relationships and the core/app document properties.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use zip::ZipArchive;

use crate::xml;
use crate::{Format, FormatsError};

/// An opened OOXML package (or any zip) held in memory.
pub struct Package {
    archive: ZipArchive<Cursor<Vec<u8>>>,
    names: Vec<String>,
}

impl Package {
    /// Open the zip in `bytes`.
    pub fn open(bytes: &[u8]) -> Result<Self, FormatsError> {
        let archive = ZipArchive::new(Cursor::new(bytes.to_vec()))?;
        let names = archive.file_names().map(str::to_string).collect();
        Ok(Self { archive, names })
    }

    /// Every entry name, in archive order.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Whether `name` is an entry (exact match).
    pub fn has(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }

    /// Read an entry fully, `None` when it is absent.
    pub fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, FormatsError> {
        if !self.has(name) {
            return Ok(None);
        }
        let mut file = self.archive.by_name(name)?;
        let mut out = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut out)?;
        Ok(Some(out))
    }

    /// Read an entry as UTF-8 text (a BOM is dropped), `None` when absent.
    pub fn read_text(&mut self, name: &str) -> Result<Option<String>, FormatsError> {
        let Some(bytes) = self.read(name)? else {
            return Ok(None);
        };
        let text = String::from_utf8(bytes)
            .map_err(|e| FormatsError::Xml(format!("{name}: not UTF-8 ({e})")))?;
        Ok(Some(text.trim_start_matches('\u{feff}').to_string()))
    }

    /// Read an entry that must exist.
    pub fn require_text(&mut self, name: &str) -> Result<String, FormatsError> {
        self.read_text(name)?
            .ok_or_else(|| FormatsError::Container(format!("missing part {name}")))
    }

    /// Relationship id -> target for the `.rels` part of `part` (for example
    /// `ppt/presentation.xml` -> `ppt/_rels/presentation.xml.rels`). Targets
    /// are resolved against the part's directory; absolute targets (`/ppt/x`)
    /// lose their leading slash.
    pub fn relationships(&mut self, part: &str) -> Result<BTreeMap<String, String>, FormatsError> {
        let (dir, file) = match part.rfind('/') {
            Some(i) => (&part[..i], &part[i + 1..]),
            None => ("", part),
        };
        let rels_name = if dir.is_empty() {
            format!("_rels/{file}.rels")
        } else {
            format!("{dir}/_rels/{file}.rels")
        };
        let mut map = BTreeMap::new();
        let Some(source) = self.read_text(&rels_name)? else {
            return Ok(map);
        };
        let doc = xml::parse(&rels_name, &source)?;
        for rel in doc
            .root_element()
            .children()
            .filter(|n| xml::is(*n, "Relationship"))
        {
            let (Some(id), Some(target)) = (xml::attr(rel, "Id"), xml::attr(rel, "Target")) else {
                continue;
            };
            if xml::attr(rel, "TargetMode") == Some("External") {
                continue;
            }
            map.insert(id.to_string(), resolve_target(dir, target));
        }
        Ok(map)
    }
}

/// Join `target` onto `dir`, collapsing `..` segments.
pub fn resolve_target(dir: &str, target: &str) -> String {
    if let Some(stripped) = target.strip_prefix('/') {
        return stripped.to_string();
    }
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// Which OOXML kind a zip is, from its first central-directory-free bytes:
/// the first local file header's name usually says `word/`, `ppt/` or `xl/`,
/// and `[Content_Types].xml` is first in files Office writes. Falls back to
/// scanning the head for the folder names.
pub fn sniff_zip_kind(head: &[u8]) -> Option<Format> {
    let find = |needle: &[u8]| head.windows(needle.len()).any(|w| w == needle);
    if find(b"word/") {
        Some(Format::Docx)
    } else if find(b"ppt/") {
        Some(Format::Pptx)
    } else if find(b"xl/") {
        Some(Format::Xlsx)
    } else {
        None
    }
}

/// Core (`docProps/core.xml`) and app (`docProps/app.xml`) properties as
/// `key -> value`, keys in their Dublin Core / OOXML spelling without prefix.
pub fn document_properties(
    package: &mut Package,
) -> Result<BTreeMap<String, String>, FormatsError> {
    let mut out = BTreeMap::new();
    if let Some(core) = package.read_text("docProps/core.xml")? {
        let doc = xml::parse("docProps/core.xml", &core)?;
        for node in doc
            .root_element()
            .children()
            .filter(roxmltree::Node::is_element)
        {
            let value = xml::text_of(node);
            if !value.trim().is_empty() {
                out.insert(node.tag_name().name().to_string(), value.trim().to_string());
            }
        }
    }
    if let Some(app) = package.read_text("docProps/app.xml")? {
        let doc = xml::parse("docProps/app.xml", &app)?;
        for node in doc
            .root_element()
            .children()
            .filter(roxmltree::Node::is_element)
        {
            let name = node.tag_name().name();
            if matches!(
                name,
                "Application"
                    | "AppVersion"
                    | "Pages"
                    | "Words"
                    | "Slides"
                    | "Notes"
                    | "Company"
                    | "TotalTime"
                    | "Characters"
                    | "Paragraphs"
                    | "Lines"
            ) {
                let value = xml::text_of(node);
                if !value.trim().is_empty() {
                    out.insert(format!("app.{name}"), value.trim().to_string());
                }
            }
        }
    }
    Ok(out)
}

/// Sort part names that end in a number (`slide10.xml` after `slide9.xml`).
pub fn numeric_order(names: &mut [String]) {
    names.sort_by_key(|name| {
        let digits: String = name
            .rsplit('/')
            .next()
            .unwrap_or(name)
            .chars()
            .filter(char::is_ascii_digit)
            .collect();
        (digits.parse::<u64>().unwrap_or(u64::MAX), name.clone())
    });
}
