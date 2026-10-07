//! Error type shared by every operation.

use std::path::PathBuf;

/// Why an operation did not produce its output.
#[derive(Debug, thiserror::Error)]
pub enum PdfOpsError {
    /// The file system refused a read or write.
    #[error("{path}: {source}")]
    Io {
        /// The file involved.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// `lopdf` could not parse or serialise a document.
    #[error("{path}: {source}")]
    Pdf {
        /// The file involved.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: lopdf::Error,
    },
    /// The output already exists and `--force` was not given.
    #[error("output exists (pass --force to replace it): {0}")]
    OutputExists(PathBuf),
    /// The output would overwrite an input; inputs are never modified.
    #[error("output must not be an input file: {0}")]
    OutputIsInput(PathBuf),
    /// The request itself is wrong (bad page range, missing password, ...).
    #[error("{0}")]
    Invalid(String),
    /// The capability is not available in this build or on this machine; the
    /// message says exactly why and what would enable it.
    #[error("unsupported: {0}")]
    Unsupported(String),
}

impl PdfOpsError {
    pub(crate) fn io(path: &std::path::Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    pub(crate) fn pdf(path: &std::path::Path, source: lopdf::Error) -> Self {
        Self::Pdf {
            path: path.to_path_buf(),
            source,
        }
    }

    /// True for [`PdfOpsError::Unsupported`]: the request was valid but this
    /// build or machine cannot carry it out.
    #[must_use]
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::Unsupported(_))
    }
}

/// Result alias for the crate.
pub type Result<T> = std::result::Result<T, PdfOpsError>;
