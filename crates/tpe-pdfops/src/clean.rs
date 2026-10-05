//! The raster path: render with `PDFium`, clean with [`crate::raster`], write
//! image-backed pages.

use std::path::Path;

use lopdf::Document;

use crate::error::{PdfOpsError, Result};
use crate::imagepdf::{ImageEncoding, image_document};
use crate::raster;
use crate::render::render_gray_pages;

/// Which clean-ups to run and how to store the result.
#[derive(Debug, Clone)]
pub struct CleanOptions {
    /// Adaptive background normalisation.
    pub water_stain: bool,
    /// Simple baseline straightening.
    pub dewarp: bool,
    /// Projection-profile deskew.
    pub deskew: bool,
    /// Render resolution.
    pub dpi: u32,
    /// Page image encoding.
    pub encoding: ImageEncoding,
    /// Open password for the input, if any.
    pub password: Option<String>,
}

/// What happened to one page.
#[derive(Debug, Clone, PartialEq)]
pub struct PageClean {
    /// 1-based page number.
    pub number: u32,
    /// Rotation applied by deskew, clockwise degrees.
    pub skew_degrees: Option<f32>,
    /// Largest per-strip shift corrected by dewarp, in pixels.
    pub dewarp_shift_px: Option<f32>,
}

/// Largest skew searched for, in degrees either way.
pub const MAX_SKEW_DEGREES: f32 = 5.0;

/// Render, clean and rebuild. Fails with [`PdfOpsError::Unsupported`] when
/// `PDFium` is unavailable.
pub fn clean(input: &Path, options: &CleanOptions) -> Result<(Document, Vec<PageClean>)> {
    if !(options.water_stain || options.dewarp || options.deskew) {
        return Err(PdfOpsError::Invalid(
            "choose at least one of --water-stain, --dewarp, --deskew".into(),
        ));
    }
    let rendered = render_gray_pages(input, options.dpi, options.password.as_deref())?;
    if rendered.is_empty() {
        return Err(PdfOpsError::Invalid(format!(
            "{}: no pages rendered",
            input.display()
        )));
    }
    let tile = (options.dpi / 6).max(16);
    let mut pages = Vec::with_capacity(rendered.len());
    let mut report = Vec::with_capacity(rendered.len());
    for (index, mut image) in rendered.into_iter().enumerate() {
        let mut entry = PageClean {
            number: u32::try_from(index + 1).unwrap_or(u32::MAX),
            skew_degrees: None,
            dewarp_shift_px: None,
        };
        if options.water_stain {
            image = raster::normalise_background(&image, tile);
        }
        if options.deskew {
            let (fixed, angle) = raster::deskew(&image, MAX_SKEW_DEGREES);
            image = fixed;
            entry.skew_degrees = Some(angle);
        }
        if options.dewarp {
            let (fixed, shifts) = raster::dewarp(&image, 12);
            image = fixed;
            entry.dewarp_shift_px =
                Some(shifts.iter().copied().fold(0.0_f32, |a, b| a.max(b.abs())));
        }
        pages.push(image);
        report.push(entry);
    }
    let doc = image_document(&pages, options.dpi, options.encoding)?;
    Ok((doc, report))
}
