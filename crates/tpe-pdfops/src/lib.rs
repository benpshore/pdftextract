//! PDF operations that always write new files.
//!
//! Inputs are opened read-only and never modified or deleted; every result is
//! a new file, and an existing output is refused unless the caller asks to
//! replace it. Each operation is a function over `lopdf::Document` plus a
//! [`output::save_document`] that re-parses what it is about to write.
//!
//! | Operation | Module | Mechanism |
//! | --- | --- | --- |
//! | concatenate | [`concat`] | renumber and copy objects, new page tree, optional outline per input |
//! | paginate | [`paginate`] | one new document per page group, optional "Page n of N" overlay stream |
//! | reorient | [`reorient`] | `/Rotate`, explicit or from the dominant text matrix |
//! | normalise to Letter | [`letter`] | scale and centre the visible box through a `cm` prefix stream |
//! | remove open password | [`unlock`] | `lopdf` decryption with the given password, tried once |
//! | linearize | [`linearize`] | explicit unsupported result with the reason |
//! | water-stain, deskew, dewarp | [`clean`] | `PDFium` render, [`raster`] processing, image-backed pages |
//!
//! The raster path needs the `pdfium` feature and `PDFIUM_DYNAMIC_LIB_PATH`;
//! otherwise it returns [`error::PdfOpsError::Unsupported`] with the reason.

// Every function's failure modes are the variants of `PdfOpsError`, which
// documents each one; repeating that list under every signature adds nothing.
#![allow(clippy::missing_errors_doc)]

pub mod clean;
pub mod concat;
pub mod error;
pub mod imagepdf;
pub mod inspect;
pub mod letter;
pub mod linearize;
pub mod output;
pub mod pages;
pub mod paginate;
pub mod raster;
pub mod render;
pub mod reorient;
pub mod unlock;

pub use error::{PdfOpsError, Result};
pub use inspect::{PageInfo, Summary, inspect};
pub use output::Output;
pub use pages::PageSelection;
