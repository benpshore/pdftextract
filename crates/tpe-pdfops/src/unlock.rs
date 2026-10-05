//! Remove an open (user) password with `lopdf`'s decryption. The password is
//! tried exactly once; nothing is guessed.

use std::path::Path;

use lopdf::{Document, LoadOptions};

use crate::error::{PdfOpsError, Result};

/// Decrypt `input` with `password` (user or owner password) and return a
/// document that saves without `/Encrypt`.
///
/// `lopdf` decrypts while loading: a document whose user password is empty
/// opens without one, and any other document needs the password at load
/// time. A wrong password is reported as [`PdfOpsError::Invalid`]; an input
/// that was never encrypted is refused the same way.
pub fn remove_password(input: &Path, password: &str) -> Result<Document> {
    let bytes = std::fs::read(input).map_err(|e| PdfOpsError::io(input, e))?;
    let probe = Document::load_mem(&bytes).map_err(|e| PdfOpsError::pdf(input, e))?;
    if !probe.is_encrypted() && !probe.was_encrypted() {
        return Err(PdfOpsError::Invalid(format!(
            "{}: document is not encrypted",
            input.display()
        )));
    }
    let doc = if probe.is_encrypted() {
        Document::load_mem_with_options(&bytes, LoadOptions::with_password(password)).map_err(
            |e| match e {
                lopdf::Error::InvalidPassword | lopdf::Error::Decryption(_) => {
                    PdfOpsError::Invalid(format!("{}: password rejected ({e})", input.display()))
                }
                other => PdfOpsError::pdf(input, other),
            },
        )?
    } else {
        // Empty user password: already open after the plain load.
        probe
    };
    if doc.is_encrypted() {
        return Err(PdfOpsError::Unsupported(format!(
            "{}: lopdf left /Encrypt in place after decryption",
            input.display()
        )));
    }
    if doc.get_pages().is_empty() {
        return Err(PdfOpsError::Invalid(format!(
            "{}: no pages readable after decryption",
            input.display()
        )));
    }
    Ok(doc)
}
