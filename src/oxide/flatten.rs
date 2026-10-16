//! Admission for page carriers that the pinned flatten writer preserves.

use std::collections::HashSet;

use pdf_oxide::PdfDocument;
use pdf_oxide::object::{Object, ObjectRef};

use super::Context;
use super::edit::{bounded_count, upstream};
use crate::backend::BackendError;

fn imprecise_transform() -> BackendError {
    BackendError::Unsupported(
        "flatten appearance transform cannot preserve nondegenerate corners within 0.001 page points using the pinned writer; nonidentity appearance matrices remain unqualified".into(),
    )
}

fn coordinates<const N: usize>(object: Option<&Object>) -> Result<[f64; N], BackendError> {
    let values = object
        .and_then(Object::as_array)
        .ok_or_else(imprecise_transform)?;
    if values.len() != N {
        return Err(imprecise_transform());
    }
    let mut result = [0.0; N];
    for (number, value) in result.iter_mut().zip(values) {
        *number = value
            .as_real()
            .or_else(|| value.as_integer().map(|v| v as f64))
            .filter(|value| value.is_finite())
            .ok_or_else(imprecise_transform)?;
    }
    Ok(result)
}

fn rounded(value: f64, places: usize) -> Result<f64, BackendError> {
    if !value.is_finite() {
        return Err(imprecise_transform());
    }
    format!("{value:.places$}")
        .parse()
        .map_err(|_| imprecise_transform())
}

fn corners([left, bottom, right, top]: [f64; 4]) -> [[f64; 2]; 4] {
    [[left, bottom], [right, bottom], [right, top], [left, top]]
}

/// Admit only geometry preserved by the pinned f32/six-decimal overlay writer.
/// Coefficient error alone is insufficient: a tiny error times a large `BBox`
/// can move the appearance by many page points, or collapse its entire area.
#[allow(
    clippy::float_cmp,
    reason = "exact identity admission and pinned serializer integer branch"
)]
pub(super) fn admit_appearance_transform(
    rectangle: Option<&Object>,
    normal: &Object,
) -> Result<(), BackendError> {
    let rect = coordinates::<4>(rectangle)?;
    let dictionary = normal.as_dict().ok_or_else(imprecise_transform)?;
    let bbox = coordinates::<4>(dictionary.get("BBox"))?;
    if let Some(matrix) = dictionary.get("Matrix") {
        // The pinned writer both retains this matrix on the Form XObject and
        // emits it in the overlay. Nonidentity placement needs separate repair.
        if coordinates::<6>(Some(matrix))? != [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] {
            return Err(imprecise_transform());
        }
    }
    for bounds in [rect, bbox] {
        if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
            return Err(imprecise_transform());
        }
    }
    // Match the pinned writer's intermediate arithmetic and both serializers,
    // then measure the result in page units against the original rectangle.
    let b = bbox.map(|value| value as f32);
    let r = rect.map(|value| value as f32);
    let sx = (r[2] - r[0]) / (b[2] - b[0]);
    let sy = (r[3] - r[1]) / (b[3] - b[1]);
    let tx = r[0] - b[0] * sx;
    let ty = r[1] - b[1] * sy;
    let mut overlay = [0.0; 4];
    for (target, value) in overlay.iter_mut().zip([sx, sy, tx, ty]) {
        *target = rounded(f64::from(value), 6)?;
    }
    if overlay[0] <= 0.0 || overlay[1] <= 0.0 {
        return Err(imprecise_transform());
    }
    let mut saved_bbox = [0.0; 4];
    for (target, value) in saved_bbox.iter_mut().zip(b.map(f64::from)) {
        if !value.is_finite() {
            return Err(imprecise_transform());
        }
        *target = if value.fract() == 0.0 {
            value as i64 as f64
        } else {
            rounded(value, 5)?
        };
    }
    if saved_bbox[2] <= saved_bbox[0] || saved_bbox[3] <= saved_bbox[1] {
        return Err(imprecise_transform());
    }
    // Check source content coordinates and the saved clipping bounds. Rounding
    // either the overlay or its BBox must not move any corner beyond the bound.
    for bounds in [bbox, saved_bbox] {
        for ([x, y], expected) in corners(bounds).into_iter().zip(corners(rect)) {
            let actual = [overlay[0] * x + overlay[2], overlay[1] * y + overlay[3]];
            if actual
                .into_iter()
                .zip(expected)
                .any(|(actual, expected)| !actual.is_finite() || (actual - expected).abs() > 0.001)
            {
                return Err(imprecise_transform());
            }
        }
    }
    Ok(())
}

fn uncertain() -> BackendError {
    BackendError::Unsupported(
        "flattening requires an unambiguous bounded raw page tree and annotation references".into(),
    )
}

fn resolve(document: &PdfDocument, object: &Object) -> Result<Object, BackendError> {
    let mut current = object.clone();
    let mut seen = HashSet::new();
    for _ in 0..8 {
        let Object::Reference(reference) = current else {
            return Ok(current);
        };
        if !seen.insert(reference) {
            return Err(uncertain());
        }
        current = document.load_object(reference).map_err(upstream)?;
    }
    Err(uncertain())
}

enum Step {
    Visit(ObjectRef, Option<ObjectRef>, usize),
    EndBranch(usize, usize),
}

fn raw_pages(
    context: &Context<'_>,
    document: &PdfDocument,
    expected_count: usize,
) -> Result<Vec<Object>, BackendError> {
    // get_page() materializes inherited Resources. The writer reloads raw page
    // dictionaries, so checking that projection would miss resource shadowing.
    let catalog = document.catalog().map_err(upstream)?;
    let root = catalog
        .as_dict()
        .and_then(|dict| dict.get("Pages"))
        .and_then(Object::as_reference)
        .ok_or_else(uncertain)?;
    let mut pending = vec![Step::Visit(root, None, 0)];
    let mut seen = HashSet::new();
    let mut pages = Vec::new();
    while let Some(step) = pending.pop() {
        let (id, parent, depth) = match step {
            Step::Visit(id, parent, depth) => (id, parent, depth),
            Step::EndBranch(first, count) => {
                if pages.len().checked_sub(first) != Some(count) {
                    return Err(uncertain());
                }
                continue;
            }
        };
        bounded_count(depth, 64, "flatten page tree depth")?;
        if !seen.insert(id) {
            return Err(uncertain());
        }
        bounded_count(
            seen.len(),
            context.limits.max_items,
            "flatten page tree nodes",
        )?;
        let object = document.load_object(id).map_err(upstream)?;
        let dict = object.as_dict().ok_or_else(uncertain)?;
        let actual_parent = dict
            .get("Parent")
            .map(|object| object.as_reference().ok_or_else(uncertain))
            .transpose()?;
        if actual_parent != parent {
            return Err(uncertain());
        }
        match dict.get("Type").and_then(Object::as_name) {
            Some("Pages") => {
                let count = dict
                    .get("Count")
                    .and_then(Object::as_integer)
                    .and_then(|count| usize::try_from(count).ok())
                    .ok_or_else(uncertain)?;
                bounded_count(count, context.limits.max_pages as usize, "flatten pages")?;
                let children = dict
                    .get("Kids")
                    .and_then(Object::as_array)
                    .ok_or_else(uncertain)?;
                bounded_count(
                    pending
                        .len()
                        .saturating_add(children.len())
                        .saturating_add(1),
                    context.limits.max_items,
                    "flatten pending page nodes",
                )?;
                pending.push(Step::EndBranch(pages.len(), count));
                for child in children.iter().rev() {
                    pending.push(Step::Visit(
                        child.as_reference().ok_or_else(uncertain)?,
                        Some(id),
                        depth + 1,
                    ));
                }
            }
            Some("Page") => {
                pages.push(object);
                bounded_count(
                    pages.len(),
                    context.limits.max_pages as usize,
                    "flatten pages",
                )?;
            }
            _ => return Err(uncertain()),
        }
    }
    if pages.len() != expected_count {
        return Err(uncertain());
    }
    Ok(pages)
}

fn validate_contents(document: &PdfDocument, object: &Object) -> Result<bool, BackendError> {
    Ok(match object {
        Object::Reference(reference) => {
            matches!(
                document.load_object(*reference).map_err(upstream)?,
                Object::Stream { .. }
            )
        }
        Object::Array(contents) => {
            for content in contents {
                let Object::Reference(reference) = content else {
                    return Ok(false);
                };
                if !matches!(
                    document.load_object(*reference).map_err(upstream)?,
                    Object::Stream { .. }
                ) {
                    return Ok(false);
                }
            }
            true
        }
        _ => false,
    })
}

pub(super) fn admit_pages(
    context: &Context<'_>,
    document: &PdfDocument,
    expected_count: usize,
    selected: &[usize],
    widgets_only: bool,
) -> Result<(), BackendError> {
    let pages = raw_pages(context, document, expected_count)?;
    let mut total = 0_usize;
    for &index in selected {
        let dict = pages
            .get(index)
            .and_then(Object::as_dict)
            .ok_or_else(uncertain)?;
        let Some(annotations) = dict.get("Annots") else {
            continue;
        };
        let annotations = resolve(document, annotations)?;
        let annotations = annotations.as_array().ok_or_else(uncertain)?;
        total = total.saturating_add(annotations.len());
        bounded_count(total, context.limits.max_items, "flatten annotations")?;
        let mut changes_page = false;
        for annotation in annotations {
            let annotation = resolve(document, annotation)?;
            let annotation = annotation.as_dict().ok_or_else(uncertain)?;
            changes_page |= !widgets_only
                || annotation.get("Subtype").and_then(Object::as_name) == Some("Widget");
        }
        if !changes_page {
            continue;
        }
        let valid_contents = dict
            .get("Contents")
            .map(|contents| {
                if let Object::Array(contents) = contents {
                    bounded_count(
                        contents.len(),
                        context.limits.max_items,
                        "flatten content streams",
                    )?;
                }
                validate_contents(document, contents)
            })
            .transpose()?
            .unwrap_or(false);
        if !valid_contents {
            return Err(BackendError::Unsupported(format!(
                "page {} lacks supported explicit /Contents; pinned flattening can remove annotations/widgets without painting their appearance",
                index + 1
            )));
        }
        // The pinned writer resolves only one resource reference and creates a
        // fresh dictionary when the raw page lacks it, shadowing inherited data.
        let resources = match dict.get("Resources") {
            Some(Object::Reference(reference)) => {
                document.load_object(*reference).map_err(upstream)?
            }
            Some(resources) => resources.clone(),
            None => Object::Null,
        };
        if !matches!(resources, Object::Dictionary(_)) {
            return Err(BackendError::Unsupported(format!(
                "page {} lacks a supported local /Resources dictionary; pinned flattening can shadow inherited fonts/images",
                index + 1
            )));
        }
    }
    Ok(())
}
