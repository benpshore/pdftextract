//! Conservative image preflight for the pinned decoder's missing alpha support.
//! This is a bounded resource check, not a complete page renderer or PDF audit.

use std::collections::HashSet;

use pdf_oxide::PdfDocument;
use pdf_oxide::content::{Operator, parse_content_stream_images_only};
use pdf_oxide::object::{Object, ObjectRef};

use super::BackendError;

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum Kind {
    Resources,
    XObjects,
    XObject,
    States,
    State,
    Patterns,
    Pattern,
}

struct Check<'a> {
    document: &'a PdfDocument,
    active: HashSet<ObjectRef>,
    done: HashSet<(ObjectRef, Kind)>,
    nodes: usize,
    decoded_bytes: usize,
}

fn malformed(error: impl std::fmt::Display) -> BackendError {
    BackendError::Malformed(format!("image resource preflight: {error}"))
}

fn unsupported() -> BackendError {
    BackendError::Unsupported(
        "masked image extraction is unsupported: pdf_oxide 0.3.78 does not preserve image alpha/soft masks in its memory PNG API".into(),
    )
}

fn masks(dict: &std::collections::HashMap<String, Object>) -> Result<(), BackendError> {
    for key in ["SMask", "Mask"] {
        if dict
            .get(key)
            .is_some_and(|value| !matches!(value, Object::Null) && value.as_name() != Some("None"))
        {
            return Err(unsupported());
        }
    }
    if ["ImageMask", "IM"].iter().any(|key| {
        dict.get(*key)
            .is_some_and(|value| value.as_bool() != Some(false))
    }) || dict
        .get("SMaskInData")
        .is_some_and(|value| value.as_integer() != Some(0))
    {
        return Err(unsupported());
    }
    Ok(())
}

impl Check<'_> {
    fn content(&mut self, bytes: &[u8]) -> Result<(), BackendError> {
        self.decoded_bytes = self.decoded_bytes.saturating_add(bytes.len());
        if self.decoded_bytes > 8 * 1024 * 1024 {
            return Err(BackendError::Limit(
                "image preflight decoded streams exceed 8 MiB".into(),
            ));
        }
        let operators = parse_content_stream_images_only(bytes).map_err(malformed)?;
        if operators.len() > 100_000 {
            return Err(BackendError::Limit(
                "image preflight exceeds 100000 operators".into(),
            ));
        }
        for operator in operators {
            if let Operator::InlineImage { dict, .. } = operator {
                masks(&dict)?;
            }
        }
        Ok(())
    }

    fn visit(&mut self, object: &Object, kind: Kind, depth: usize) -> Result<(), BackendError> {
        self.nodes = self.nodes.saturating_add(1);
        if depth > 64 || self.nodes > 4096 {
            return Err(BackendError::Limit(
                "image resource depth/count exceeds 64/4096".into(),
            ));
        }
        if let Object::Reference(reference) = object {
            if self.active.contains(reference) {
                return Err(malformed("cyclic image resource graph"));
            }
            if self.done.contains(&(*reference, kind)) {
                return Ok(());
            }
            self.active.insert(*reference);
            let resolved = self.document.load_object(*reference).map_err(malformed)?;
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
                "image resource dictionary exceeds 4096 entries".into(),
            ));
        }
        match kind {
            Kind::Resources => {
                for (key, next) in [
                    ("XObject", Kind::XObjects),
                    ("ExtGState", Kind::States),
                    ("Pattern", Kind::Patterns),
                ] {
                    if let Some(value) = dict.get(key) {
                        self.visit(value, next, depth + 1)?;
                    }
                }
            }
            Kind::XObjects | Kind::States | Kind::Patterns => {
                let next = match kind {
                    Kind::XObjects => Kind::XObject,
                    Kind::States => Kind::State,
                    _ => Kind::Pattern,
                };
                for value in dict.values() {
                    self.visit(value, next, depth + 1)?;
                }
            }
            Kind::State => masks(dict)?,
            Kind::XObject | Kind::Pattern => {
                masks(dict)?;
                let is_form = dict.get("Subtype").and_then(Object::as_name) == Some("Form");
                if let Some(resources) = dict.get("Resources") {
                    self.visit(resources, Kind::Resources, depth + 1)?;
                }
                if is_form || (kind == Kind::Pattern && matches!(object, Object::Stream { .. })) {
                    if self.document.is_encrypted() {
                        return Err(BackendError::Unsupported("masked-image preflight cannot inspect encrypted Form/pattern streams through the pinned public byte API".into()));
                    }
                    let bytes = object.decode_stream_data().map_err(malformed)?;
                    self.content(&bytes)?;
                }
            }
        }
        Ok(())
    }
}

/// Reject transparency carriers the upstream memory image decoder would drop.
/// Scans even unused page resources conservatively; does not modify the source.
pub fn reject_unsupported_masks(document: &PdfDocument, page: usize) -> Result<(), BackendError> {
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
            check.visit(resources, Kind::Resources, 0)?;
            break;
        }
        let Some(parent) = dict.get("Parent") else {
            break;
        };
        if depth == 64 {
            return Err(BackendError::Limit(
                "image resource inheritance exceeds 64 ancestors".into(),
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
    let bytes = document.get_page_content_data(page).map_err(malformed)?;
    check.content(&bytes)
}
