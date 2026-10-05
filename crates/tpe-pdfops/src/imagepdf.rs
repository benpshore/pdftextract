//! Build a PDF whose pages are single full-page grey images.

use image::GrayImage;
use image::codecs::jpeg::JpegEncoder;
use lopdf::{Document, Object, Stream, dictionary};

use crate::error::{PdfOpsError, Result};

/// How page images are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageEncoding {
    /// `/DCTDecode` (JPEG) at this quality, 1..=100.
    Jpeg {
        /// JPEG quality.
        quality: u8,
    },
    /// `/FlateDecode`, lossless.
    Flate,
}

/// One page per image; the page is `width/dpi x height/dpi` inches.
pub fn image_document(pages: &[GrayImage], dpi: u32, encoding: ImageEncoding) -> Result<Document> {
    if pages.is_empty() {
        return Err(PdfOpsError::Invalid("no pages to write".into()));
    }
    let dpi = f64::from(dpi.max(1));
    let mut doc = Document::with_version("1.5");
    let tree_id = doc.new_object_id();
    let mut kids = Vec::with_capacity(pages.len());
    for picture in pages {
        let (w, h) = picture.dimensions();
        let (w_pt, h_pt) = (f64::from(w) * 72.0 / dpi, f64::from(h) * 72.0 / dpi);
        let image_id = doc.add_object(image_stream(picture, encoding)?);
        let content = format!("q {w_pt:.4} 0 0 {h_pt:.4} 0 0 cm /Im0 Do Q\n");
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => tree_id,
            "MediaBox" => vec![0.into(), 0.into(), real(w_pt), real(h_pt)],
            "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image_id } },
            "Contents" => content_id,
        });
        kids.push(Object::Reference(page_id));
    }
    let count = i64::try_from(kids.len()).unwrap_or(i64::MAX);
    doc.objects.insert(
        tree_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => tree_id });
    doc.trailer.set("Root", catalog_id);
    Ok(doc)
}

#[allow(clippy::cast_possible_truncation)]
fn real(value: f64) -> Object {
    Object::Real(value as f32)
}

fn image_stream(image: &GrayImage, encoding: ImageEncoding) -> Result<Stream> {
    let (w, h) = image.dimensions();
    let mut dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => i64::from(w),
        "Height" => i64::from(h),
        "ColorSpace" => "DeviceGray",
        "BitsPerComponent" => 8,
    };
    match encoding {
        ImageEncoding::Jpeg { quality } => {
            let mut bytes = Vec::new();
            let mut encoder = JpegEncoder::new_with_quality(&mut bytes, quality.clamp(1, 100));
            encoder
                .encode(image.as_raw(), w, h, image::ExtendedColorType::L8)
                .map_err(|e| PdfOpsError::Invalid(format!("jpeg encoding: {e}")))?;
            dict.set("Filter", "DCTDecode");
            Ok(Stream::new(dict, bytes).with_compression(false))
        }
        ImageEncoding::Flate => {
            let mut stream = Stream::new(dict, image.as_raw().clone());
            stream
                .compress()
                .map_err(|e| PdfOpsError::Invalid(format!("flate encoding: {e}")))?;
            Ok(stream)
        }
    }
}
