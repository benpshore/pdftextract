//! Input sniffing and the one-page PDF wrapper for still images, so that a
//! PNG or JPEG scan goes through the same engine backend as a scanned PDF.
//! The image is embedded losslessly (8-bit gray or RGB, Flate) and placed at
//! an assumed 300 dpi; the result records the dimensions and that assumption.

use std::io::Write;

use image::{ImageFormat, ImageReader};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use serde::{Deserialize, Serialize};

/// What the uploaded bytes are, from their magic numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputKind {
    Pdf,
    Png,
    Jpeg,
}

impl InputKind {
    /// Lower-case name used in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Png => "png",
            Self::Jpeg => "jpeg",
        }
    }
}

/// Recognise a PDF (`%PDF-` within the first KiB), PNG or JPEG.
pub fn sniff(bytes: &[u8]) -> Option<InputKind> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(InputKind::Png);
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(InputKind::Jpeg);
    }
    let head = &bytes[..bytes.len().min(1024)];
    head.windows(5)
        .any(|window| window == b"%PDF-")
        .then_some(InputKind::Pdf)
}

/// Resolution assumed for an image's page size (300 dpi, a typical scan).
pub const DEFAULT_DPI: f32 = 300.0;
/// Largest image accepted, in pixels (decoded RGB is three bytes per pixel).
pub const MAX_PIXELS: u64 = 40_000_000;

/// Why an image could not be wrapped.
#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("image could not be decoded: {0}")]
    Decode(String),
    #[error("image of {width}x{height} pixels exceeds the limit of {MAX_PIXELS} pixels")]
    TooLarge { width: u32, height: u32 },
    #[error("wrapper PDF could not be written: {0}")]
    Pdf(String),
}

/// A still image wrapped as a one-page PDF.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrappedImage {
    pub pdf: Vec<u8>,
    pub width_px: u32,
    pub height_px: u32,
    /// Whether the page carries one gray component per pixel (else RGB).
    pub grayscale: bool,
}

impl WrappedImage {
    /// Page width in points at [`DEFAULT_DPI`].
    pub fn width_pt(&self) -> f32 {
        self.width_px as f32 * 72.0 / DEFAULT_DPI
    }

    /// Page height in points at [`DEFAULT_DPI`].
    pub fn height_pt(&self) -> f32 {
        self.height_px as f32 * 72.0 / DEFAULT_DPI
    }
}

/// Decode a PNG or JPEG and embed it on one PDF page.
pub fn wrap_image(bytes: &[u8], kind: InputKind) -> Result<WrappedImage, ImageError> {
    let format = match kind {
        InputKind::Png => ImageFormat::Png,
        InputKind::Jpeg => ImageFormat::Jpeg,
        InputKind::Pdf => return Err(ImageError::Decode("input is already a PDF".to_string())),
    };
    let (width, height) = ImageReader::with_format(std::io::Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|error| ImageError::Decode(error.to_string()))?;
    if width == 0 || height == 0 {
        return Err(ImageError::Decode("image has no pixels".to_string()));
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(ImageError::TooLarge { width, height });
    }
    let decoded = image::load_from_memory_with_format(bytes, format)
        .map_err(|error| ImageError::Decode(error.to_string()))?;
    let grayscale = matches!(
        decoded.color(),
        image::ColorType::L8
            | image::ColorType::L16
            | image::ColorType::La8
            | image::ColorType::La16
    );
    let (color_space, raw) = if grayscale {
        ("DeviceGray", decoded.into_luma8().into_raw())
    } else {
        ("DeviceRGB", decoded.into_rgb8().into_raw())
    };
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(&raw)
        .and_then(|()| encoder.finish())
        .map_err(|error| ImageError::Pdf(error.to_string()))
        .and_then(|compressed| build_pdf(width, height, color_space, compressed))
        .map(|pdf| WrappedImage {
            pdf,
            width_px: width,
            height_px: height,
            grayscale,
        })
}

fn build_pdf(
    width: u32,
    height: u32,
    color_space: &str,
    compressed: Vec<u8>,
) -> Result<Vec<u8>, ImageError> {
    let width_pt = width as f32 * 72.0 / DEFAULT_DPI;
    let height_pt = height as f32 * 72.0 / DEFAULT_DPI;
    let mut doc = Document::with_version("1.5");
    let tree_id = doc.new_object_id();
    let image_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => i64::from(width),
            "Height" => i64::from(height),
            "ColorSpace" => color_space,
            "BitsPerComponent" => 8,
            "Filter" => "FlateDecode",
        },
        compressed,
    ));
    let resources_id = doc.add_object(dictionary! {
        "XObject" => dictionary! { "Im0" => image_id },
    });
    let operations = vec![
        Operation::new("q", vec![]),
        Operation::new(
            "cm",
            vec![
                Object::Real(width_pt),
                0.into(),
                0.into(),
                Object::Real(height_pt),
                0.into(),
                0.into(),
            ],
        ),
        Operation::new("Do", vec!["Im0".into()]),
        Operation::new("Q", vec![]),
    ];
    let content = Content { operations }
        .encode()
        .map_err(|error| ImageError::Pdf(error.to_string()))?;
    let content_id = doc.add_object(Stream::new(dictionary! {}, content));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => tree_id,
        "MediaBox" => vec![0.into(), 0.into(), Object::Real(width_pt), Object::Real(height_pt)],
        "Contents" => content_id,
        "Resources" => resources_id,
    });
    doc.objects.insert(
        tree_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => 1,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => tree_id,
    });
    doc.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes)
        .map_err(|error| ImageError::Pdf(error.to_string()))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32, gray: bool) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        if gray {
            let image = image::GrayImage::from_fn(width, height, |x, y| {
                image::Luma([if (x + y) % 2 == 0 { 20 } else { 240 }])
            });
            image.write_to(&mut out, ImageFormat::Png).unwrap();
        } else {
            let image = image::RgbImage::from_fn(width, height, |x, _| {
                image::Rgb([u8::try_from(x % 256).unwrap(), 128, 30])
            });
            image.write_to(&mut out, ImageFormat::Png).unwrap();
        }
        out.into_inner()
    }

    #[test]
    fn sniff_recognises_pdf_png_jpeg_and_nothing_else() {
        assert_eq!(sniff(b"%PDF-1.4\n"), Some(InputKind::Pdf));
        assert_eq!(sniff(b"\xef\xbb\xbf   %PDF-1.7"), Some(InputKind::Pdf));
        assert_eq!(sniff(&png(2, 2, true)), Some(InputKind::Png));
        assert_eq!(
            sniff(&[0xff, 0xd8, 0xff, 0xe0, 0, 0]),
            Some(InputKind::Jpeg)
        );
        assert_eq!(sniff(b"hello"), None);
        assert_eq!(sniff(b""), None);
        assert_eq!(InputKind::Jpeg.as_str(), "jpeg");
    }

    #[test]
    fn gray_png_becomes_a_device_gray_page_the_engine_reads_as_one_raster() {
        let wrapped = wrap_image(&png(30, 20, true), InputKind::Png).unwrap();
        assert!(wrapped.grayscale);
        assert_eq!((wrapped.width_px, wrapped.height_px), (30, 20));
        assert!((wrapped.width_pt() - 7.2).abs() < 0.01);
        assert!(wrapped.pdf.starts_with(b"%PDF-1.5"));
        let backend = tpe::backend::by_name("lopdf").unwrap();
        let mut session = backend.open(&wrapped.pdf, None).unwrap();
        assert_eq!(session.page_count(), 1);
        let page = session.page_text(1).unwrap();
        assert!(page.spans.iter().all(|span| span.text.trim().is_empty()));
        let rasters: Vec<_> = page
            .figures
            .iter()
            .filter(|figure| figure.kind == "raster")
            .collect();
        assert_eq!(rasters.len(), 1, "{:?}", page.figures);
        assert!((page.width - wrapped.width_pt()).abs() < 0.01);
        assert!((page.height - wrapped.height_pt()).abs() < 0.01);
    }

    #[test]
    fn rgb_png_and_jpeg_are_wrapped_and_junk_is_rejected() {
        let wrapped = wrap_image(&png(8, 4, false), InputKind::Png).unwrap();
        assert!(!wrapped.grayscale);
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(6, 5, image::Rgb([200, 10, 10]))
            .write_to(&mut jpeg, ImageFormat::Jpeg)
            .unwrap();
        let jpeg = jpeg.into_inner();
        assert_eq!(sniff(&jpeg), Some(InputKind::Jpeg));
        let wrapped = wrap_image(&jpeg, InputKind::Jpeg).unwrap();
        assert_eq!((wrapped.width_px, wrapped.height_px), (6, 5));
        assert!(matches!(
            wrap_image(b"\x89PNG\r\n\x1a\nbroken", InputKind::Png),
            Err(ImageError::Decode(_))
        ));
        assert!(matches!(
            wrap_image(b"%PDF-", InputKind::Pdf),
            Err(ImageError::Decode(_))
        ));
    }

    #[test]
    fn oversized_dimensions_are_refused_before_decoding() {
        // A PNG header claiming 7000x7000 pixels (49 MP), no pixel data at all.
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(b"IHDR");
        ihdr.extend_from_slice(&7000_u32.to_be_bytes());
        ihdr.extend_from_slice(&7000_u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(&ihdr);
        bytes.extend_from_slice(&crc32(&ihdr).to_be_bytes());
        // An empty IDAT chunk: the decoder reads the header up to the first IDAT.
        bytes.extend_from_slice(&0_u32.to_be_bytes());
        bytes.extend_from_slice(b"IDAT");
        bytes.extend_from_slice(&crc32(b"IDAT").to_be_bytes());
        let outcome = wrap_image(&bytes, InputKind::Png);
        assert!(
            matches!(
                outcome,
                Err(ImageError::TooLarge {
                    width: 7000,
                    height: 7000
                })
            ),
            "{outcome:?}"
        );
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffff_u32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
}
