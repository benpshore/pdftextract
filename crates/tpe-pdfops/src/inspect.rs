//! Re-parse a PDF and report what every operation's tests check: page count,
//! boxes, rotation, encryption and linearization markers.

use std::fmt::Write as _;
use std::path::Path;

use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::error::{PdfOpsError, Result};

/// One page as seen after inheritance from the page tree.
#[derive(Debug, Clone, PartialEq)]
pub struct PageInfo {
    /// 1-based page number.
    pub number: u32,
    /// `/MediaBox` as `[x0, y0, x1, y1]` with the corners normalised.
    pub media_box: [f64; 4],
    /// `/CropBox` when present (normalised).
    pub crop_box: Option<[f64; 4]>,
    /// `/Rotate` reduced to 0, 90, 180 or 270.
    pub rotate: i64,
}

impl PageInfo {
    /// The visible box: `/CropBox` when present, else `/MediaBox`.
    #[must_use]
    pub fn visible_box(&self) -> [f64; 4] {
        self.crop_box.unwrap_or(self.media_box)
    }
}

/// What a document looks like after re-parsing.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// Header version (`1.5`).
    pub version: String,
    /// Number of pages reachable from the page tree.
    pub page_count: usize,
    /// `/Encrypt` present in the trailer.
    pub encrypted: bool,
    /// A `/Linearized` parameter dictionary leads the file.
    pub linearized: bool,
    /// Every page in order.
    pub pages: Vec<PageInfo>,
}

/// Parse `path` and summarise it.
pub fn inspect(path: &Path) -> Result<Summary> {
    let bytes = std::fs::read(path).map_err(|e| PdfOpsError::io(path, e))?;
    let doc = Document::load_mem(&bytes).map_err(|e| PdfOpsError::pdf(path, e))?;
    let mut summary = summarise(&doc);
    summary.linearized = looks_linearized(&bytes);
    Ok(summary)
}

/// Summarise an already-loaded document (the linearization flag needs the
/// raw bytes and stays `false` here).
#[must_use]
pub fn summarise(doc: &Document) -> Summary {
    let pages: Vec<PageInfo> = doc
        .page_iter()
        .enumerate()
        .map(|(index, id)| PageInfo {
            number: u32::try_from(index + 1).unwrap_or(u32::MAX),
            media_box: inherited_box(doc, id, b"MediaBox").unwrap_or([0.0, 0.0, 612.0, 792.0]),
            crop_box: inherited_box(doc, id, b"CropBox"),
            rotate: inherited_rotate(doc, id),
        })
        .collect();
    Summary {
        version: doc.version.clone(),
        page_count: pages.len(),
        encrypted: doc.is_encrypted(),
        linearized: false,
        pages,
    }
}

/// True when a `/Linearized` dictionary appears in the first kilobyte, which
/// is where the specification requires it (PDF 32000-1 Annex F.3.2).
#[must_use]
pub fn looks_linearized(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(1024)];
    head.windows(11).any(|w| w == b"/Linearized")
}

/// Look `key` up on the page, then on each ancestor in the page tree.
pub(crate) fn inherited<'a>(
    doc: &'a Document,
    page_id: ObjectId,
    key: &[u8],
) -> Option<&'a Object> {
    let mut current = page_id;
    for _ in 0..64 {
        let dict = doc.get_dictionary(current).ok()?;
        if let Ok(value) = dict.get(key) {
            return Some(deref(doc, value));
        }
        current = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

/// Follow references until a direct object is reached.
pub(crate) fn deref<'a>(doc: &'a Document, object: &'a Object) -> &'a Object {
    let mut current = object;
    for _ in 0..32 {
        match current {
            Object::Reference(id) => match doc.get_object(*id) {
                Ok(next) => current = next,
                Err(_) => return current,
            },
            _ => return current,
        }
    }
    current
}

/// A numeric operand (`Integer` or `Real`).
pub(crate) fn number(object: &Object) -> Option<f64> {
    match object {
        Object::Integer(i) => i64_to_f64(*i),
        Object::Real(r) => Some(f64::from(*r)),
        _ => None,
    }
}

#[allow(clippy::cast_precision_loss)]
fn i64_to_f64(value: i64) -> Option<f64> {
    // Exact for the magnitudes a PDF coordinate can have.
    i32::try_from(value)
        .ok()
        .map(f64::from)
        .or(Some(value as f64))
}

/// A rectangle `[x0 y0 x1 y1]` with the corners ordered.
pub(crate) fn rectangle(doc: &Document, object: &Object) -> Option<[f64; 4]> {
    let array = deref(doc, object).as_array().ok()?;
    if array.len() != 4 {
        return None;
    }
    let mut values = [0.0; 4];
    for (slot, item) in values.iter_mut().zip(array) {
        *slot = number(deref(doc, item))?;
    }
    Some([
        values[0].min(values[2]),
        values[1].min(values[3]),
        values[0].max(values[2]),
        values[1].max(values[3]),
    ])
}

pub(crate) fn inherited_box(doc: &Document, page_id: ObjectId, key: &[u8]) -> Option<[f64; 4]> {
    inherited(doc, page_id, key).and_then(|object| rectangle(doc, object))
}

/// Effective `/Rotate`, normalised to one of 0, 90, 180, 270.
pub(crate) fn inherited_rotate(doc: &Document, page_id: ObjectId) -> i64 {
    inherited(doc, page_id, b"Rotate")
        .and_then(number)
        .map_or(0, |value| normalise_rotation(rounded(value)))
}

#[allow(clippy::cast_possible_truncation)]
fn rounded(value: f64) -> i64 {
    value.round() as i64
}

/// Reduce any multiple of 90 into `0..360`; other values are treated as 0.
#[must_use]
pub fn normalise_rotation(value: i64) -> i64 {
    if value % 90 != 0 {
        return 0;
    }
    value.rem_euclid(360)
}

/// Make the inherited page attributes explicit on the page dictionary, so the
/// page keeps them when it moves to another page tree.
pub(crate) fn materialise_inherited(doc: &Document, page_id: ObjectId, page: &mut Dictionary) {
    for key in [&b"Resources"[..], b"MediaBox", b"CropBox", b"Rotate"] {
        if !page.has(key)
            && let Some(value) = inherited_raw(doc, page_id, key)
        {
            page.set(key, value.clone());
        }
    }
}

/// Like [`inherited`] but returns the stored object (possibly a reference)
/// so that shared resource dictionaries keep being shared.
fn inherited_raw<'a>(doc: &'a Document, page_id: ObjectId, key: &[u8]) -> Option<&'a Object> {
    let mut current = page_id;
    for _ in 0..64 {
        let dict = doc.get_dictionary(current).ok()?;
        if let Ok(value) = dict.get(key) {
            return Some(value);
        }
        current = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

/// Render a summary as plain text, one page per line.
#[must_use]
pub fn render_summary(path: &Path, summary: &Summary) -> String {
    let mut out = format!(
        "{}\nversion: {}\npages: {}\nencrypted: {}\nlinearized: {}\n",
        path.display(),
        summary.version,
        summary.page_count,
        summary.encrypted,
        summary.linearized
    );
    for page in &summary.pages {
        let [x0, y0, x1, y1] = page.media_box;
        let _ = write!(
            out,
            "page {}: media [{x0} {y0} {x1} {y1}] rotate {}",
            page.number, page.rotate
        );
        if let Some([cx0, cy0, cx1, cy1]) = page.crop_box {
            let _ = write!(out, " crop [{cx0} {cy0} {cx1} {cy1}]");
        }
        out.push('\n');
    }
    out
}
