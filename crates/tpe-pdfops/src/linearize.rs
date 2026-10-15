//! Linearization ("fast web view") is reported, not performed.
//!
//! A linearized file needs a leading `/Linearized` parameter dictionary with
//! exact byte offsets, a first-page cross-reference section, a hint stream
//! with page-offset and shared-object tables, and an object order that puts
//! the first page's objects first (PDF 32000-1 Annex F). `lopdf` 0.45 writes
//! classic or object-stream files only and exposes no offset table to build
//! the hint stream from, so this crate refuses rather than emit a file that
//! merely claims to be linearized.

use std::path::Path;

use crate::error::{PdfOpsError, Result};
use crate::inspect;

/// The reason `linearize` is unsupported, word for word.
pub const REASON: &str = "linearization is not implemented: lopdf 0.45 cannot write the /Linearized \
parameter dictionary, first-page xref section and hint stream with correct byte offsets, and a \
file that only claims to be linearized would mislead readers (use qpdf --linearize)";

/// Report whether `input` is already linearized, then refuse with [`REASON`].
pub fn linearize(input: &Path) -> Result<()> {
    let bytes = std::fs::read(input).map_err(|e| PdfOpsError::io(input, e))?;
    let already = inspect::looks_linearized(&bytes);
    Err(PdfOpsError::Unsupported(if already {
        format!(
            "{REASON}; note: {} already carries a /Linearized dictionary",
            input.display()
        )
    } else {
        REASON.to_string()
    }))
}
