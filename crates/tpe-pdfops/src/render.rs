//! Page rendering through `PDFium`. The library is loaded only from the
//! absolute location in `PDFIUM_DYNAMIC_LIB_PATH`; without the `pdfium`
//! feature or that variable the raster path is explicitly unsupported.

use std::path::{Path, PathBuf};

use image::GrayImage;

use crate::error::{PdfOpsError, Result};

/// Environment variable naming the directory (or file) holding `libpdfium`.
pub const ENV_LIBRARY_PATH: &str = "PDFIUM_DYNAMIC_LIB_PATH";

/// The configured library location, or why rendering is unavailable.
pub fn library_location() -> Result<PathBuf> {
    if !cfg!(feature = "pdfium") {
        return Err(PdfOpsError::Unsupported(
            "tpe-pdfops was built without the `pdfium` feature; rebuild with `--features pdfium` and set PDFIUM_DYNAMIC_LIB_PATH to the absolute directory holding libpdfium".into(),
        ));
    }
    let Some(configured) = std::env::var_os(ENV_LIBRARY_PATH) else {
        return Err(PdfOpsError::Unsupported(format!(
            "{ENV_LIBRARY_PATH} is not set; export it as the absolute directory holding libpdfium (for example `$PWD/.pdfium/lib` after `sh native/fetch.sh --pdfium-only`)"
        )));
    };
    let path = PathBuf::from(configured);
    if !path.is_absolute() {
        return Err(PdfOpsError::Unsupported(format!(
            "{ENV_LIBRARY_PATH} must be absolute, not {}",
            path.display()
        )));
    }
    if !path.exists() {
        return Err(PdfOpsError::Unsupported(format!(
            "{ENV_LIBRARY_PATH} points at {}, which does not exist",
            path.display()
        )));
    }
    Ok(path)
}

/// Render every page of `input` to an 8-bit grey image at `dpi`, as the page
/// is displayed (its `/Rotate` applied).
#[cfg(feature = "pdfium")]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
pub fn render_gray_pages(input: &Path, dpi: u32, password: Option<&str>) -> Result<Vec<GrayImage>> {
    use pdfium_render::prelude::*;

    let location = library_location()?;
    let library = if location.is_file() {
        location
    } else {
        Pdfium::pdfium_platform_library_name_at_path(&location)
    };
    let bindings = Pdfium::bind_to_library(&library).map_err(|e| {
        PdfOpsError::Unsupported(format!(
            "cannot load pdfium from {}: {e:?}",
            library.display()
        ))
    })?;
    let pdfium = Pdfium::new(bindings);
    let bytes = std::fs::read(input).map_err(|e| PdfOpsError::io(input, e))?;
    let document = pdfium
        .load_pdf_from_byte_slice(&bytes, password)
        .map_err(|e| {
            PdfOpsError::Invalid(format!(
                "{}: pdfium could not open it: {e:?}",
                input.display()
            ))
        })?;
    let scale = dpi.max(1) as f32 / 72.0;
    let mut pages = Vec::new();
    for (index, page) in document.pages().iter().enumerate() {
        let width = (page.width().value * scale).round().max(1.0);
        let height = (page.height().value * scale).round().max(1.0);
        let config = PdfRenderConfig::new().set_target_size(width as Pixels, height as Pixels);
        let bitmap = page.render_with_config(&config).map_err(|e| {
            PdfOpsError::Invalid(format!(
                "{}: page {}: render failed: {e:?}",
                input.display(),
                index + 1
            ))
        })?;
        pages.push(to_gray(
            &bitmap.as_rgba_bytes(),
            bitmap.width(),
            bitmap.height(),
        ));
    }
    Ok(pages)
}

/// Without the feature every call is the explicit unsupported result.
#[cfg(not(feature = "pdfium"))]
pub fn render_gray_pages(
    _input: &Path,
    _dpi: u32,
    _password: Option<&str>,
) -> Result<Vec<GrayImage>> {
    library_location().map(|_| Vec::new())
}

/// Luma from RGBA bytes (Rec. 601 weights).
#[cfg(feature = "pdfium")]
fn to_gray(rgba: &[u8], width: i32, height: i32) -> GrayImage {
    let (w, h) = (
        u32::try_from(width).unwrap_or(0),
        u32::try_from(height).unwrap_or(0),
    );
    let mut gray = GrayImage::new(w, h);
    let (chunks, _) = rgba.as_chunks::<4>();
    for (pixel, chunk) in gray.pixels_mut().zip(chunks) {
        let luma =
            (u32::from(chunk[0]) * 299 + u32::from(chunk[1]) * 587 + u32::from(chunk[2]) * 114)
                / 1000;
        pixel.0[0] = u8::try_from(luma).unwrap_or(255);
    }
    gray
}
