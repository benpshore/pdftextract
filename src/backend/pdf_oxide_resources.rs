//! Bounded admission checks for known omissions in the pinned Form reader.
//! Parsing a Form into the object cache is not a qualified extraction repair:
//! the cache is bounded and the native negative subtype cache wins thereafter.

use std::collections::HashSet;

use pdf_oxide::PdfDocument;
use pdf_oxide::object::{Object, ObjectRef};

use super::BackendError;
use super::pdf_oxide_content::reject_text_stream_operator_cutoff;

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum Kind {
    Resources,
    XObjects,
    XObject,
}

struct Check<'a> {
    document: &'a PdfDocument,
    active: HashSet<ObjectRef>,
    done: HashSet<(ObjectRef, Kind)>,
    nodes: usize,
    decoded_bytes: usize,
}

fn malformed(error: impl std::fmt::Display) -> BackendError {
    BackendError::Malformed(format!("Form resource preflight: {error}"))
}

fn unsupported() -> BackendError {
    BackendError::Unsupported(
        "pdf_oxide 0.3.78 can omit Form text when its shallow subtype scan reads a nested subtype; cache preloading is not a qualified repair".into(),
    )
}

impl Check<'_> {
    fn budget(&mut self, depth: usize) -> Result<(), BackendError> {
        self.nodes = self.nodes.saturating_add(1);
        if depth > 64 || self.nodes > 4096 {
            return Err(BackendError::Limit(
                "Form resource depth/count exceeds 64/4096".into(),
            ));
        }
        Ok(())
    }

    // Inline nested Subtype values can defeat a cold raw-byte peek even if an
    // earlier caller has primed the full-object cache. Reject this shape too;
    // dictionary order and future cache residency are not correctness proofs.
    fn nested_subtype(&mut self, object: &Object, depth: usize) -> Result<bool, BackendError> {
        self.budget(depth)?;
        match object {
            Object::Dictionary(dict) | Object::Stream { dict, .. } => {
                if dict.contains_key("Subtype") {
                    return Ok(true);
                }
                for value in dict.values() {
                    if self.nested_subtype(value, depth + 1)? {
                        return Ok(true);
                    }
                }
            }
            Object::Array(values) => {
                for value in values {
                    if self.nested_subtype(value, depth + 1)? {
                        return Ok(true);
                    }
                }
            }
            // References are not present as dictionaries in the peeked bytes.
            _ => {}
        }
        Ok(false)
    }

    fn visit(&mut self, object: &Object, kind: Kind, depth: usize) -> Result<(), BackendError> {
        self.budget(depth)?;
        if let Object::Reference(reference) = object {
            if self.active.contains(reference) {
                return Err(malformed("cyclic Form resource graph"));
            }
            if self.done.contains(&(*reference, kind)) {
                return Ok(());
            }
            self.active.insert(*reference);
            // Check before loading: loading first would hide the observed bug.
            let native_form = kind != Kind::XObject || self.document.is_form_xobject(*reference);
            let resolved = self.document.load_object(*reference).map_err(malformed)?;
            if !native_form
                && resolved
                    .as_dict()
                    .and_then(|dict| dict.get("Subtype"))
                    .and_then(Object::as_name)
                    == Some("Form")
            {
                return Err(unsupported());
            }
            self.visit(&resolved, kind, depth + 1)?;
            self.active.remove(reference);
            self.done.insert((*reference, kind));
            return Ok(());
        }
        let dict = object
            .as_dict()
            .ok_or_else(|| malformed("resource is not a dictionary/stream"))?;
        if dict.len() > 4096 {
            return Err(BackendError::Limit(
                "Form resource dictionary exceeds 4096 entries".into(),
            ));
        }
        match kind {
            Kind::Resources => {
                if let Some(objects) = dict.get("XObject") {
                    self.visit(objects, Kind::XObjects, depth + 1)?;
                }
            }
            Kind::XObjects => {
                for value in dict.values() {
                    if !matches!(value, Object::Reference(_)) {
                        return Err(BackendError::Unsupported(
                            "direct XObject resources have no qualified native text traversal"
                                .into(),
                        ));
                    }
                    self.visit(value, Kind::XObject, depth + 1)?;
                }
            }
            Kind::XObject => {
                if dict.get("Subtype").and_then(Object::as_name) != Some("Form") {
                    return Ok(());
                }
                if !matches!(object, Object::Stream { .. }) {
                    return Err(malformed("Form XObject is not a stream"));
                }
                for (key, value) in dict {
                    if key != "Subtype" && self.nested_subtype(value, depth + 1)? {
                        return Err(unsupported());
                    }
                }
                if self.document.is_encrypted() {
                    return Err(BackendError::Unsupported(
                        "Form text preflight cannot inspect encrypted streams through the pinned public byte API".into(),
                    ));
                }
                let bytes = object.decode_stream_data().map_err(malformed)?;
                self.decoded_bytes = self.decoded_bytes.saturating_add(bytes.len());
                if self.decoded_bytes > 16 * 1024 * 1024 {
                    return Err(BackendError::Limit(
                        "Form text preflight decoded streams exceed 16 MiB".into(),
                    ));
                }
                reject_text_stream_operator_cutoff(&bytes)?;
                if let Some(resources) = dict.get("Resources") {
                    self.visit(resources, Kind::Resources, depth + 1)?;
                }
            }
        }
        Ok(())
    }
}

/// Conservatively inspect even unused resources before any text projection.
/// This rejects identified omission/cap paths; it does not certify completeness.
pub(crate) fn reject_misclassified_forms(
    document: &PdfDocument,
    page: usize,
) -> Result<(), BackendError> {
    let mut check = Check {
        document,
        active: HashSet::new(),
        done: HashSet::new(),
        nodes: 0,
        decoded_bytes: 0,
    };
    let mut object = document.get_page(page).map_err(malformed)?;
    let mut parents = HashSet::new();
    for depth in 0..=64 {
        let dict = object
            .as_dict()
            .ok_or_else(|| malformed("page/ancestor is not a dictionary"))?;
        if let Some(resources) = dict.get("Resources") {
            return check.visit(resources, Kind::Resources, 0);
        }
        let Some(parent) = dict.get("Parent") else {
            return Ok(());
        };
        if depth == 64 {
            return Err(BackendError::Limit(
                "Form resource inheritance exceeds 64 ancestors".into(),
            ));
        }
        if let Some(reference) = parent.as_reference() {
            if !parents.insert(reference) {
                return Err(malformed("cyclic page resource inheritance"));
            }
            object = document.load_object(reference).map_err(malformed)?;
        } else {
            object = parent.clone();
        }
    }
    Ok(())
}
