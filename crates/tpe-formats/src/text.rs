//! Plain text, and the byte decoding shared with delimited text: a BOM
//! decides UTF-8/UTF-16, valid UTF-8 is kept, anything else is read as
//! Windows-1252 with a warning.

use crate::{Block, DocumentIdentity, Format, FormatsResult, Section};

/// Decoded text and the encoding label used.
pub struct Decoded {
    pub text: String,
    /// `utf-8`, `utf-8-bom`, `utf-16le`, `utf-16be` or `windows-1252`.
    pub encoding: &'static str,
    /// Set when the decoder had to replace malformed sequences.
    pub lossy: bool,
}

/// Decode `bytes` to text. Never fails: an undecodable byte becomes U+FFFD.
pub fn decode(bytes: &[u8]) -> Decoded {
    if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        let (text, lossy) = utf8(rest);
        return Decoded {
            text,
            encoding: "utf-8-bom",
            lossy,
        };
    }
    if bytes.starts_with(b"\xFF\xFE") {
        let (text, _, lossy) = encoding_rs::UTF_16LE.decode(&bytes[2..]);
        return Decoded {
            text: text.into_owned(),
            encoding: "utf-16le",
            lossy,
        };
    }
    if bytes.starts_with(b"\xFE\xFF") {
        let (text, _, lossy) = encoding_rs::UTF_16BE.decode(&bytes[2..]);
        return Decoded {
            text: text.into_owned(),
            encoding: "utf-16be",
            lossy,
        };
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Decoded {
            text: text.to_string(),
            encoding: "utf-8",
            lossy: false,
        };
    }
    let (text, _, lossy) = encoding_rs::WINDOWS_1252.decode(bytes);
    Decoded {
        text: text.into_owned(),
        encoding: "windows-1252",
        lossy,
    }
}

fn utf8(bytes: &[u8]) -> (String, bool) {
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_string(), false),
        Err(_) => (String::from_utf8_lossy(bytes).into_owned(), true),
    }
}

/// Normalise line endings to `\n` and drop trailing whitespace per line.
pub fn normalise_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        out.push_str(line.trim_end());
        out.push('\n');
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// Extract a plain-text file: paragraphs are runs of non-blank lines.
pub fn extract(bytes: &[u8], identity: DocumentIdentity) -> FormatsResult {
    let decoded = decode(bytes);
    let mut result = FormatsResult::empty(Format::Text, identity);
    result
        .metadata
        .insert("encoding".to_string(), decoded.encoding.to_string());
    if decoded.encoding == "windows-1252" {
        result.warn("input is not valid UTF-8; decoded as windows-1252");
    }
    if decoded.lossy {
        result.warn("partial: malformed byte sequences were replaced with U+FFFD");
    }
    let normalised = normalise_lines(&decoded.text);
    let mut section = Section::new("body", 0, None);
    for paragraph in normalised.split("\n\n") {
        let paragraph = paragraph.trim_matches('\n');
        if !paragraph.trim().is_empty() {
            section.blocks.push(Block::paragraph(paragraph));
        }
    }
    result.sections.push(section);
    result
}
