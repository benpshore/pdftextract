//! Rotate pages through `/Rotate`, explicitly or from the dominant text
//! direction in the content stream.

use std::path::Path;

use lopdf::content::Content;
use lopdf::{Document, Object, ObjectId};

use crate::error::{PdfOpsError, Result};
use crate::inspect::{inherited_rotate, normalise_rotation, number};
use crate::output::load_input;
use crate::pages::PageSelection;

/// How a page should be turned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Add this many degrees clockwise (90, 180 or 270) to the page's rotation.
    Rotate(i64),
    /// Set `/Rotate` so the dominant text direction reads left to right;
    /// pages whose direction cannot be decided are left unchanged.
    Auto,
}

/// What happened to one page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageOutcome {
    /// 1-based page number.
    pub number: u32,
    /// `/Rotate` before.
    pub before: i64,
    /// `/Rotate` after (equal to `before` when undecided or unselected).
    pub after: i64,
    /// Why the page was left alone, if it was.
    pub note: Option<String>,
}

/// Rotate the selected pages (all when `selection` is `None`).
pub fn reorient(
    input: &Path,
    orientation: Orientation,
    selection: Option<&PageSelection>,
) -> Result<(Document, Vec<PageOutcome>)> {
    if let Orientation::Rotate(degrees) = orientation
        && !matches!(degrees, 90 | 180 | 270)
    {
        return Err(PdfOpsError::Invalid(format!(
            "rotation must be 90, 180 or 270 degrees, not {degrees}"
        )));
    }
    let mut doc = load_input(input, false)?;
    let page_ids: Vec<ObjectId> = doc.page_iter().collect();
    let mut outcomes = Vec::with_capacity(page_ids.len());
    for (index, page_id) in page_ids.into_iter().enumerate() {
        let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        let before = inherited_rotate(&doc, page_id);
        if selection.is_some_and(|s| !s.contains(number)) {
            outcomes.push(PageOutcome {
                number,
                before,
                after: before,
                note: Some("not selected".into()),
            });
            continue;
        }
        let (after, note) = match orientation {
            Orientation::Rotate(degrees) => (normalise_rotation(before + degrees), None),
            Orientation::Auto => {
                let content = doc.get_page_content(page_id);
                match decide_rotation(&content) {
                    Decision::Rotate(degrees) => (degrees, None),
                    Decision::NoText => (before, Some("no text on the page".into())),
                    Decision::Ambiguous => (before, Some("no dominant text direction".into())),
                }
            }
        };
        if after != before {
            let page = doc
                .get_dictionary_mut(page_id)
                .map_err(|e| PdfOpsError::pdf(input, e))?;
            page.set("Rotate", after);
        }
        outcomes.push(PageOutcome {
            number,
            before,
            after,
            note,
        });
    }
    Ok((doc, outcomes))
}

/// Result of reading a content stream for its text direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The absolute `/Rotate` that makes the dominant text read left to right.
    Rotate(i64),
    /// No text-showing operator was found.
    NoText,
    /// Several directions share the page with no clear majority.
    Ambiguous,
}

/// Share of text (by shown bytes) one direction needs to count as dominant.
const DOMINANCE: f64 = 0.6;

/// A 2x3 affine matrix `[a b c d e f]` as in PDF.
#[derive(Debug, Clone, Copy)]
struct Matrix([f64; 6]);

impl Matrix {
    const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// `self x other` (apply `self` first, then `other`), PDF row-vector order.
    fn then(self, other: Self) -> Self {
        let m = self.0;
        let n = other.0;
        Self([
            m[0] * n[0] + m[1] * n[2],
            m[0] * n[1] + m[1] * n[3],
            m[2] * n[0] + m[3] * n[2],
            m[2] * n[1] + m[3] * n[3],
            m[4] * n[0] + m[5] * n[2] + n[4],
            m[4] * n[1] + m[5] * n[3] + n[5],
        ])
    }

    fn from_operands(operands: &[Object]) -> Option<Self> {
        if operands.len() != 6 {
            return None;
        }
        let mut m = [0.0; 6];
        for (slot, operand) in m.iter_mut().zip(operands) {
            *slot = number(operand)?;
        }
        Some(Self(m))
    }
}

/// Decide the page rotation from the text matrices in `content`.
///
/// Every text-showing operator contributes the length of what it shows,
/// weighted into one of four quadrants from the direction of the text-space
/// x axis after `Tm` and the current transformation (`cm`, with `q`/`Q`).
/// Upright text (direction +x) needs `/Rotate 0`; text running up the page
/// needs 90; upside-down text 180; text running down the page 270.
#[must_use]
pub fn decide_rotation(content: &[u8]) -> Decision {
    let Ok(parsed) = Content::decode(content) else {
        return Decision::NoText;
    };
    let mut ctm = Matrix::IDENTITY;
    let mut stack: Vec<Matrix> = Vec::new();
    let mut tm = Matrix::IDENTITY;
    let mut weights = [0.0_f64; 4];
    for op in &parsed.operations {
        match op.operator.as_str() {
            "q" => stack.push(ctm),
            "Q" => ctm = stack.pop().unwrap_or(Matrix::IDENTITY),
            "cm" => {
                if let Some(m) = Matrix::from_operands(&op.operands) {
                    ctm = m.then(ctm);
                }
            }
            "BT" => tm = Matrix::IDENTITY,
            "Tm" => {
                if let Some(m) = Matrix::from_operands(&op.operands) {
                    tm = m;
                }
            }
            "Tj" | "'" | "\"" | "TJ" => {
                let shown = shown_length(&op.operands);
                if shown == 0 {
                    continue;
                }
                let m = tm.then(ctm);
                let angle = m.0[1].atan2(m.0[0]).to_degrees();
                weights[quadrant(angle)] += f64::from(shown);
            }
            _ => {}
        }
    }
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return Decision::NoText;
    }
    let (best, weight) = weights
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map_or((0, 0.0), |(i, w)| (i, *w));
    if weight / total < DOMINANCE {
        return Decision::Ambiguous;
    }
    Decision::Rotate(i64::try_from(best).unwrap_or(0) * 90)
}

/// Quadrant index 0..4 of a text direction angle in degrees.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn quadrant(angle: f64) -> usize {
    ((angle / 90.0).round().rem_euclid(4.0)) as usize
}

/// Bytes shown by a text-showing operator's operands (saturating).
fn shown_length(operands: &[Object]) -> u32 {
    let mut total: u32 = 0;
    for operand in operands {
        match operand {
            Object::String(bytes, _) => {
                total = total.saturating_add(u32::try_from(bytes.len()).unwrap_or(u32::MAX));
            }
            Object::Array(items) => {
                for item in items {
                    if let Object::String(bytes, _) = item {
                        total =
                            total.saturating_add(u32::try_from(bytes.len()).unwrap_or(u32::MAX));
                    }
                }
            }
            _ => {}
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(ops: &str) -> Vec<u8> {
        ops.as_bytes().to_vec()
    }

    #[test]
    fn upright_text_needs_no_rotation() {
        let content = stream("BT /F1 12 Tf 1 0 0 1 72 700 Tm (Hello) Tj ET");
        assert_eq!(decide_rotation(&content), Decision::Rotate(0));
    }

    #[test]
    fn text_running_up_the_page_needs_90() {
        let content = stream("BT /F1 12 Tf 0 1 -1 0 500 72 Tm (Hello) Tj ET");
        assert_eq!(decide_rotation(&content), Decision::Rotate(90));
    }

    #[test]
    fn upside_down_text_needs_180_and_cm_counts() {
        let content = stream("q -1 0 0 -1 612 792 cm BT /F1 12 Tf (Hello) Tj ET Q");
        assert_eq!(decide_rotation(&content), Decision::Rotate(180));
    }

    #[test]
    fn text_running_down_needs_270() {
        let content = stream("BT /F1 12 Tf 0 -1 1 0 72 700 Tm [(Hello) -20 (World)] TJ ET");
        assert_eq!(decide_rotation(&content), Decision::Rotate(270));
    }

    #[test]
    fn mixed_directions_are_ambiguous_and_empty_pages_have_no_text() {
        let content = stream("BT 1 0 0 1 0 0 Tm (abcd) Tj 0 1 -1 0 0 0 Tm (abcd) Tj ET");
        assert_eq!(decide_rotation(&content), Decision::Ambiguous);
        assert_eq!(
            decide_rotation(&stream("0 0 m 10 10 l S")),
            Decision::NoText
        );
    }
}
