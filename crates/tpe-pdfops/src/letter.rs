//! Normalise every page to US Letter by scaling and centring its visible box.

use std::path::Path;

use lopdf::{Document, Object, ObjectId, Stream, dictionary};

use crate::error::{PdfOpsError, Result};
use crate::inspect::{deref, inherited_box, inherited_rotate, rectangle};
use crate::output::load_input;
use crate::pages::PageSelection;

/// US Letter in points (portrait).
pub const LETTER: (f64, f64) = (612.0, 792.0);

/// How one page was fitted.
#[derive(Debug, Clone, PartialEq)]
pub struct Fit {
    /// 1-based page number.
    pub number: u32,
    /// Uniform scale applied to the content.
    pub scale: f64,
    /// Translation applied after scaling.
    pub offset: (f64, f64),
    /// The new `/MediaBox` (`[0 0 612 792]` or `[0 0 792 612]` for pages whose
    /// `/Rotate` is 90 or 270, so the displayed page is portrait Letter).
    pub media_box: [f64; 4],
}

/// Fit the selected pages (all when `selection` is `None`) into Letter,
/// keeping the aspect ratio. The page's visible box (`/CropBox`, else
/// `/MediaBox`) is scaled to fit and centred; the original content streams
/// are kept and wrapped by a `q <matrix> cm` prefix and a `Q` suffix stream.
/// Annotation rectangles are moved with the content.
pub fn normalise_to_letter(
    input: &Path,
    selection: Option<&PageSelection>,
) -> Result<(Document, Vec<Fit>)> {
    let mut doc = load_input(input, false)?;
    let page_ids: Vec<ObjectId> = doc.page_iter().collect();
    let mut fits = Vec::new();
    for (index, page_id) in page_ids.into_iter().enumerate() {
        let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        if selection.is_some_and(|s| !s.contains(number)) {
            continue;
        }
        let fit = fit_page(&mut doc, page_id, number).map_err(|e| match e {
            PdfOpsError::Invalid(msg) => {
                PdfOpsError::Invalid(format!("{}: page {number}: {msg}", input.display()))
            }
            other => other,
        })?;
        fits.push(fit);
    }
    Ok((doc, fits))
}

fn fit_page(doc: &mut Document, page_id: ObjectId, number: u32) -> Result<Fit> {
    let media = inherited_box(doc, page_id, b"MediaBox")
        .ok_or_else(|| PdfOpsError::Invalid("no usable /MediaBox".into()))?;
    let source = inherited_box(doc, page_id, b"CropBox")
        .map_or(media, |crop| intersect(crop, media).unwrap_or(media));
    let [x0, y0, x1, y1] = source;
    let (w, h) = (x1 - x0, y1 - y0);
    if w <= 0.0 || h <= 0.0 {
        return Err(PdfOpsError::Invalid(format!(
            "degenerate page box {source:?}"
        )));
    }
    let rotate = inherited_rotate(doc, page_id);
    let (target_w, target_h) = if rotate == 90 || rotate == 270 {
        (LETTER.1, LETTER.0)
    } else {
        LETTER
    };
    let scale = (target_w / w).min(target_h / h);
    let tx = (target_w - scale * w) / 2.0 - scale * x0;
    let ty = (target_h - scale * h) / 2.0 - scale * y0;

    let existing = doc.get_page_contents(page_id);
    let prefix = format!(
        "q {} 0 0 {} {} {} cm\n",
        fmt(scale),
        fmt(scale),
        fmt(tx),
        fmt(ty)
    );
    let prefix_id = doc.add_object(Stream::new(dictionary! {}, prefix.into_bytes()));
    let suffix_id = doc.add_object(Stream::new(dictionary! {}, b"\nQ\n".to_vec()));
    let mut contents = vec![Object::Reference(prefix_id)];
    contents.extend(existing.into_iter().map(Object::Reference));
    contents.push(Object::Reference(suffix_id));

    let media_box = [0.0, 0.0, target_w, target_h];
    let annots = annotation_ids(doc, page_id);
    for annot_id in annots {
        if let Ok(annot) = doc.get_dictionary_mut(annot_id)
            && let Ok(rect) = annot.get(b"Rect").cloned()
            && let Some([ax0, ay0, ax1, ay1]) = direct_rectangle(&rect)
        {
            annot.set(
                "Rect",
                vec![
                    real(scale * ax0 + tx),
                    real(scale * ay0 + ty),
                    real(scale * ax1 + tx),
                    real(scale * ay1 + ty),
                ],
            );
        }
    }

    let page = doc
        .get_dictionary_mut(page_id)
        .map_err(|e| PdfOpsError::Invalid(e.to_string()))?;
    page.set("Contents", contents);
    page.set("MediaBox", boxed(media_box));
    page.set("CropBox", boxed(media_box));
    for key in [&b"BleedBox"[..], b"TrimBox", b"ArtBox"] {
        page.remove(key);
    }
    page.set("Rotate", rotate);
    Ok(Fit {
        number,
        scale,
        offset: (tx, ty),
        media_box,
    })
}

fn intersect(a: [f64; 4], b: [f64; 4]) -> Option<[f64; 4]> {
    let r = [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ];
    (r[2] > r[0] && r[3] > r[1]).then_some(r)
}

fn boxed(rect: [f64; 4]) -> Vec<Object> {
    rect.iter().map(|v| real(*v)).collect()
}

/// PDF numbers are `f32` in lopdf; page coordinates fit comfortably.
#[allow(clippy::cast_possible_truncation)]
fn real(value: f64) -> Object {
    Object::Real(value as f32)
}

fn fmt(value: f64) -> String {
    let s = format!("{value:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// Indirect annotation dictionaries of the page (direct ones are rare and
/// left untouched).
fn annotation_ids(doc: &Document, page_id: ObjectId) -> Vec<ObjectId> {
    let Ok(page) = doc.get_dictionary(page_id) else {
        return Vec::new();
    };
    let Ok(annots) = page.get(b"Annots") else {
        return Vec::new();
    };
    deref(doc, annots)
        .as_array()
        .map(|items| items.iter().filter_map(|o| o.as_reference().ok()).collect())
        .unwrap_or_default()
}

fn direct_rectangle(object: &Object) -> Option<[f64; 4]> {
    // No document needed: annotation rectangles are direct arrays.
    let doc = Document::new();
    rectangle(&doc, object)
}
