//! Writing outputs without ever touching an input.
//!
//! Every operation ends here: the document is serialised to memory, parsed
//! again as a self-check, and then written to a path that is not an input and
//! that does not exist yet (unless `force` is set).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use lopdf::Document;

use crate::error::{PdfOpsError, Result};

/// Where an operation writes and whether an existing file may be replaced.
#[derive(Debug, Clone)]
pub struct Output {
    /// The output file (or directory for `paginate`).
    pub path: PathBuf,
    /// Replace an existing output. Inputs are refused even with `force`.
    pub force: bool,
}

impl Output {
    /// An output at `path` that refuses to replace an existing file.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            force: false,
        }
    }

    /// Allow replacing an existing output file.
    #[must_use]
    pub fn force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }
}

/// Refuse an output that names one of the inputs, whatever `force` says.
pub fn ensure_not_input(output: &Path, inputs: &[&Path]) -> Result<()> {
    let output_abs = absolute(output);
    for input in inputs {
        if same_file(&output_abs, &absolute(input)) {
            return Err(PdfOpsError::OutputIsInput(output.to_path_buf()));
        }
    }
    Ok(())
}

fn absolute(path: &Path) -> PathBuf {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()))
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b
}

/// Write `bytes` to `output.path` as a new file. Without `force` an existing
/// file is an error; with it the file is truncated and rewritten.
pub fn write_bytes(output: &Output, bytes: &[u8]) -> Result<()> {
    let path = &output.path;
    let mut file = if output.force {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .map_err(|e| PdfOpsError::io(path, e))?
    } else {
        match File::create_new(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(PdfOpsError::OutputExists(path.clone()));
            }
            Err(e) => return Err(PdfOpsError::io(path, e)),
        }
    };
    file.write_all(bytes)
        .map_err(|e| PdfOpsError::io(path, e))?;
    file.flush().map_err(|e| PdfOpsError::io(path, e))?;
    Ok(())
}

/// Serialise `doc`, parse the bytes again (page count must survive), then
/// write them with [`write_bytes`]. Returns the number of pages written.
pub fn save_document(doc: &mut Document, output: &Output) -> Result<usize> {
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes)
        .map_err(|e| PdfOpsError::io(&output.path, e))?;
    let expected = doc.get_pages().len();
    let reparsed = Document::load_mem(&bytes).map_err(|e| PdfOpsError::pdf(&output.path, e))?;
    let found = reparsed.get_pages().len();
    if found != expected {
        return Err(PdfOpsError::Invalid(format!(
            "self-check failed: serialised document has {found} pages, expected {expected}"
        )));
    }
    write_bytes(output, &bytes)?;
    Ok(found)
}

/// Load a document read-only. An encrypted document is refused with a hint to
/// run `unlock` first, unless `allow_encrypted` is set.
pub fn load_input(path: &Path, allow_encrypted: bool) -> Result<Document> {
    let bytes = std::fs::read(path).map_err(|e| PdfOpsError::io(path, e))?;
    let doc = Document::load_mem(&bytes).map_err(|e| PdfOpsError::pdf(path, e))?;
    if doc.is_encrypted() && !allow_encrypted {
        return Err(PdfOpsError::Invalid(format!(
            "{}: document is encrypted; run `tpe-pdfops unlock --password ...` first",
            path.display()
        )));
    }
    Ok(doc)
}
