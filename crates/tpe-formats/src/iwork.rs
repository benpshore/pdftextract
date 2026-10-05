//! Apple Pages and Numbers packages. A document is a zip (or a directory
//! bundle) whose text lives in `Index/*.iwa`: Snappy-framed protobuf messages
//! whose schema Apple does not publish. This crate opens the package, records
//! what it contains, copies out `preview.pdf` when the author saved one (so
//! the PDF engine can read that), and reports the text itself as
//! unsupported with the exact reason. Nothing is guessed from the IWA bytes.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::ooxml::Package;
use crate::xml;
use crate::{DocumentIdentity, ExtraFile, Format, FormatsError, FormatsResult, Section};

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

fn describe(
    entries: &mut dyn Entries,
    identity: DocumentIdentity,
    format: Format,
) -> Result<FormatsResult, FormatsError> {
    let names = entries.names();
    let iwa: Vec<&String> = names
        .iter()
        .filter(|n| n.starts_with("Index/") && n.ends_with(".iwa"))
        .collect();
    let has_document = names.iter().any(|n| n == "Index/Document.iwa");
    let app = match format {
        Format::Numbers => "Numbers",
        _ => "Pages",
    };
    let reason = if has_document {
        format!(
            "{app} text is stored in Index/*.iwa as Snappy-framed protobuf messages without a published schema; tpe-formats does not decode IWA (no faithful decoder is available here). Export the document as PDF or DOCX, or use the copied preview.pdf when present"
        )
    } else {
        format!(
            "{app} package has no Index/Document.iwa ({} entries, {} .iwa files); not a current-format {app} document",
            names.len(),
            iwa.len()
        )
    };
    let mut result = FormatsResult::unsupported(format, identity, &reason);
    let mut metadata: BTreeMap<String, String> = BTreeMap::new();
    metadata.insert("package.entries".to_string(), names.len().to_string());
    metadata.insert("package.iwa_files".to_string(), iwa.len().to_string());
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
        let builds: Vec<&String> = strings
            .iter()
            .filter(|s| !s.starts_with("Template:"))
            .collect();
        if !builds.is_empty() {
            metadata.insert(
                "build_versions".to_string(),
                builds
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
    }
    if let Some(pdf) = entries.read("preview.pdf")? {
        if pdf.starts_with(b"%PDF") {
            metadata.insert("preview_pdf".to_string(), "preview.pdf".to_string());
            result.extra_files.push(ExtraFile {
                suffix: "preview.pdf".to_string(),
                bytes: pdf,
            });
            result.warn(
                "preview.pdf copied next to the outputs as <stem>.preview.pdf; it is the author's saved preview, not necessarily the whole document",
            );
        }
    } else {
        metadata.insert("preview_pdf".to_string(), "absent".to_string());
    }
    let mut section = Section::new("package", 0, Some(format!("{app} package")));
    section.blocks.push(crate::Block::new(
        "code",
        names
            .iter()
            .take(200)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n"),
    ));
    result.sections.push(section);
    result.metadata = metadata;
    Ok(result)
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
