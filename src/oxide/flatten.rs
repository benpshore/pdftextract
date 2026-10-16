//! Admission for page carriers that the pinned flatten writer preserves.

use std::collections::HashSet;

use pdf_oxide::PdfDocument;
use pdf_oxide::object::{Object, ObjectRef};

use super::Context;
use super::edit::{bounded_count, upstream};
use crate::backend::BackendError;

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
