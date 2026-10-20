//! Input sniffing and decoding. Formats are detected from the leading bytes,
//! not the file name, so a `.png` holding JPEG data still decodes; the name is
//! only used to choose candidates while walking a directory. GIF uses its first
//! frame. HEIC/HEIF are recognised and refused with a reason: no decoder is
//! linked (libheif is not a dependency), and the tool never guesses pixels.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use image::codecs::gif::GifDecoder;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageReader, Limits};
use serde::{Deserialize, Serialize};

use crate::ImageTextError;

/// Largest image accepted, per side and in total pixels. Beyond this the
/// decoder refuses rather than allocate without bound.
pub const MAX_SIDE: u32 = 20_000;
/// Decoder allocation cap in bytes (the `image` crate enforces it).
pub const MAX_ALLOC_BYTES: u64 = 1 << 30;

/// File extensions picked up while walking a directory (lower-case).
pub const CANDIDATE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "jpe", "tif", "tiff", "bmp", "webp", "gif", "heic", "heif",
];

/// Container format detected from the leading bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// Portable Network Graphics.
    Png,
    /// JPEG (JFIF/EXIF).
    Jpeg,
    /// TIFF, either byte order.
    Tiff,
    /// Windows bitmap.
    Bmp,
    /// WebP (RIFF container).
    WebP,
    /// GIF87a/GIF89a.
    Gif,
    /// HEIF family (`ftyp` brands heic/heix/hevc/mif1/msf1/heif).
    Heif,
    /// Nothing recognisable.
    Unknown,
}

impl Format {
    /// Lower-case name used in reports.
    pub fn name(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpeg",
            Format::Tiff => "tiff",
            Format::Bmp => "bmp",
            Format::WebP => "webp",
            Format::Gif => "gif",
            Format::Heif => "heif",
            Format::Unknown => "unknown",
        }
    }
}

/// Detect the container format from the first bytes of a file.
pub fn sniff(head: &[u8]) -> Format {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Format::Png
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Format::Jpeg
    } else if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
        Format::Tiff
    } else if head.starts_with(b"BM") && head.len() >= 14 {
        Format::Bmp
    } else if head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        Format::WebP
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Format::Gif
    } else if head.len() >= 12 && &head[4..8] == b"ftyp" {
        match &head[8..12] {
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"hevm" | b"hevs"
            | b"mif1" | b"msf1" | b"heif" | b"avif" => Format::Heif,
            _ => Format::Unknown,
        }
    } else {
        Format::Unknown
    }
}

/// Whether a path's extension marks it as an input candidate during a walk.
pub fn is_candidate(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| CANDIDATE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// A decoded still image plus what was learnt on the way.
#[derive(Debug)]
pub struct Decoded {
    /// Detected container format.
    pub format: Format,
    /// Pixels (any colour type the decoder produced).
    pub image: DynamicImage,
    /// Non-fatal findings, e.g. an animated GIF reduced to its first frame.
    pub warnings: Vec<String>,
}

/// Decode a file into pixels, or explain why that is impossible.
pub fn decode(path: &Path) -> Result<Decoded, ImageTextError> {
    let mut file =
        File::open(path).map_err(|e| ImageTextError::Io(path.display().to_string(), e))?;
    let mut head = [0u8; 16];
    let n = read_fully(&mut file, &mut head)
        .map_err(|e| ImageTextError::Io(path.display().to_string(), e))?;
    let format = sniff(&head[..n]);
    let mut warnings = Vec::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext = ext.to_ascii_lowercase();
        let expected = match ext.as_str() {
            "png" => Some(Format::Png),
            "jpg" | "jpeg" | "jpe" => Some(Format::Jpeg),
            "tif" | "tiff" => Some(Format::Tiff),
            "bmp" => Some(Format::Bmp),
            "webp" => Some(Format::WebP),
            "gif" => Some(Format::Gif),
            "heic" | "heif" => Some(Format::Heif),
            _ => None,
        };
        if let Some(expected) = expected
            && expected != format
            && format != Format::Unknown
        {
            warnings.push(format!(
                "extension .{ext} but the bytes are {}; decoded as {}",
                format.name(),
                format.name()
            ));
        }
    }
    match format {
        Format::Heif => {
            return Err(ImageTextError::Unsupported(format!(
                "{}: HEIC/HEIF is not decodable in this build (no libheif is linked); convert it to PNG or JPEG first",
                path.display()
            )));
        }
        Format::Unknown => {
            return Err(ImageTextError::Unsupported(format!(
                "{}: not a recognised image (expected PNG, JPEG, TIFF, BMP, WebP or GIF by content)",
                path.display()
            )));
        }
        _ => {}
    }
    let reader = BufReader::new(
        File::open(path).map_err(|e| ImageTextError::Io(path.display().to_string(), e))?,
    );
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC_BYTES);
    let image = if format == Format::Gif {
        let mut decoder = GifDecoder::new(reader)
            .map_err(|e| ImageTextError::Decode(path.display().to_string(), e.to_string()))?;
        decoder
            .set_limits(limits)
            .map_err(|e| ImageTextError::Decode(path.display().to_string(), e.to_string()))?;
        let mut frames = decoder.into_frames();
        let first = frames
            .next()
            .ok_or_else(|| {
                ImageTextError::Decode(path.display().to_string(), "GIF has no frames".to_string())
            })?
            .map_err(|e| ImageTextError::Decode(path.display().to_string(), e.to_string()))?;
        if frames.next().is_some() {
            warnings.push("gif: animated; only the first frame was read".to_string());
        }
        DynamicImage::ImageRgba8(first.into_buffer())
    } else {
        let mut reader = ImageReader::new(reader)
            .with_guessed_format()
            .map_err(|e| ImageTextError::Decode(path.display().to_string(), e.to_string()))?;
        reader.limits(limits);
        reader
            .decode()
            .map_err(|e| ImageTextError::Decode(path.display().to_string(), e.to_string()))?
    };
    if image.width() == 0 || image.height() == 0 {
        return Err(ImageTextError::Decode(
            path.display().to_string(),
            "image has no pixels".to_string(),
        ));
    }
    Ok(Decoded {
        format,
        image,
        warnings,
    })
}

fn read_fully(file: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        let n = file.read(&mut buf[total..])?;
        if n == 0 {
            break;
        }
        total += n;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_every_container() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"), Format::Png);
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10]), Format::Jpeg);
        assert_eq!(sniff(b"II*\0\x08\0\0\0"), Format::Tiff);
        assert_eq!(sniff(b"MM\0*\0\0\0\x08"), Format::Tiff);
        assert_eq!(sniff(b"BM\x36\x04\0\0\0\0\0\0\x36\0\0\0"), Format::Bmp);
        assert_eq!(sniff(b"RIFF\x24\0\0\0WEBPVP8 "), Format::WebP);
        assert_eq!(sniff(b"GIF89a\x01\0\x01\0"), Format::Gif);
        assert_eq!(sniff(b"\0\0\0\x18ftypheic\0\0\0\0"), Format::Heif);
        assert_eq!(sniff(b"\0\0\0\x18ftypmif1\0\0\0\0"), Format::Heif);
        assert_eq!(sniff(b"\0\0\0\x18ftypisom\0\0\0\0"), Format::Unknown);
        assert_eq!(sniff(b"plain text"), Format::Unknown);
        assert_eq!(sniff(b""), Format::Unknown);
    }

    #[test]
    fn candidate_extensions_are_case_insensitive() {
        assert!(is_candidate(Path::new("a/B.PNG")));
        assert!(is_candidate(Path::new("x.heic")));
        assert!(!is_candidate(Path::new("x.pdf")));
        assert!(!is_candidate(Path::new("noext")));
    }
}
