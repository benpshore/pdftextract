//! Concatenate N documents into one new file, keeping page order.

use std::path::Path;

use lopdf::{Dictionary, Document, Object, ObjectId, dictionary};

use crate::error::{PdfOpsError, Result};
use crate::inspect::materialise_inherited;
use crate::output::load_input;

/// Options for [`concatenate`].
#[derive(Debug, Clone, Default)]
pub struct ConcatOptions {
    /// Add a top-level outline entry per input (its file stem) pointing at the
    /// input's first page.
    pub outlines: bool,
}

/// Merge `inputs` in order into a new document. Shared resources are copied
/// per input (no deduplication); every input's own outlines are dropped and
/// replaced by one entry per file when `options.outlines` is set.
pub fn concatenate(inputs: &[&Path], options: &ConcatOptions) -> Result<Document> {
    if inputs.is_empty() {
        return Err(PdfOpsError::Invalid(
            "concat needs at least one input".into(),
        ));
    }
    let mut out = Document::with_version("1.5");
    let tree_id = out.new_object_id();
    let mut next_id = out.max_id + 1;
    let mut kids: Vec<Object> = Vec::new();
    let mut first_pages: Vec<(String, ObjectId)> = Vec::new();

    for input in inputs {
        let mut doc = load_input(input, false)?;
        if doc.version.as_str() > out.version.as_str() {
            out.version.clone_from(&doc.version);
        }
        doc.renumber_objects_with(next_id);
        next_id = doc.max_id + 1;
        let page_ids: Vec<ObjectId> = doc.page_iter().collect();
        if page_ids.is_empty() {
            return Err(PdfOpsError::Invalid(format!(
                "{}: no pages",
                input.display()
            )));
        }
        let stem = input.file_stem().map_or_else(
            || input.display().to_string(),
            |s| s.to_string_lossy().into_owned(),
        );
        first_pages.push((stem, page_ids[0]));

        for page_id in &page_ids {
            let mut page = doc
                .get_dictionary(*page_id)
                .map_err(|e| PdfOpsError::pdf(input, e))?
                .clone();
            materialise_inherited(&doc, *page_id, &mut page);
            page.set("Parent", tree_id);
            kids.push(Object::Reference(*page_id));
            out.objects.insert(*page_id, Object::Dictionary(page));
        }
        for (id, object) in doc.objects {
            if out.objects.contains_key(&id) {
                continue;
            }
            match object.type_name().unwrap_or(b"") {
                b"Catalog" | b"Pages" | b"Outlines" => {}
                _ => {
                    out.objects.insert(id, object);
                }
            }
        }
    }

    let count = i64::try_from(kids.len()).unwrap_or(i64::MAX);
    out.max_id = next_id;
    out.objects.insert(
        tree_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => count,
        }),
    );
    let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => tree_id };
    if options.outlines {
        let outlines_id = build_outlines(&mut out, &first_pages);
        catalog.set("Outlines", outlines_id);
        catalog.set("PageMode", "UseOutlines");
    }
    let catalog_id = out.add_object(catalog);
    out.trailer.set("Root", catalog_id);
    out.prune_objects();
    Ok(out)
}

/// One flat outline level: an item per input, each a `/Fit` destination.
fn build_outlines(doc: &mut Document, entries: &[(String, ObjectId)]) -> ObjectId {
    let outlines_id = doc.new_object_id();
    let item_ids: Vec<ObjectId> = entries.iter().map(|_| doc.new_object_id()).collect();
    for (index, ((title, page_id), item_id)) in entries.iter().zip(&item_ids).enumerate() {
        let mut item: Dictionary = dictionary! {
            "Title" => Object::string_literal(title.as_str()),
            "Parent" => outlines_id,
            "Dest" => vec![Object::Reference(*page_id), Object::Name(b"Fit".to_vec())],
        };
        if index > 0 {
            item.set("Prev", item_ids[index - 1]);
        }
        if index + 1 < item_ids.len() {
            item.set("Next", item_ids[index + 1]);
        }
        doc.objects.insert(*item_id, Object::Dictionary(item));
    }
    let count = i64::try_from(item_ids.len()).unwrap_or(i64::MAX);
    let mut outlines = dictionary! { "Type" => "Outlines", "Count" => count };
    if let (Some(first), Some(last)) = (item_ids.first(), item_ids.last()) {
        outlines.set("First", *first);
        outlines.set("Last", *last);
    }
    doc.objects
        .insert(outlines_id, Object::Dictionary(outlines));
    outlines_id
}
