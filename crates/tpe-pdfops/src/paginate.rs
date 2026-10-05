//! Split one document into several new files by page ranges or by a fixed
//! number of pages per file, optionally stamping the source page number.

use std::path::{Path, PathBuf};

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, ObjectId, dictionary};

use crate::error::{PdfOpsError, Result};
use crate::inspect::{inherited_box, materialise_inherited};
use crate::output::{Output, ensure_not_input, load_input, save_document};
use crate::pages::PageSelection;

/// How the pages are grouped into output files.
#[derive(Debug, Clone)]
pub enum SplitMode {
    /// One output per selection, in the order given (`--pages 1-3 --pages 7`).
    Ranges(Vec<PageSelection>),
    /// Consecutive groups of `n` pages (`--every n`); the last may be shorter.
    EveryN(u32),
}

/// Options for [`paginate`].
#[derive(Debug, Clone)]
pub struct PaginateOptions {
    /// How to group pages.
    pub mode: SplitMode,
    /// Stamp "Page n of total" (source numbering) at the bottom centre of every
    /// page as a new content stream.
    pub stamp: bool,
    /// Output file stem; files are `<stem>-NNN.pdf` inside the output directory.
    pub stem: Option<String>,
}

/// Compute the page groups for `page_count` pages.
pub fn groups(mode: &SplitMode, page_count: u32) -> Result<Vec<Vec<u32>>> {
    match mode {
        SplitMode::Ranges(selections) => selections.iter().map(|s| s.resolve(page_count)).collect(),
        SplitMode::EveryN(n) => {
            if *n == 0 {
                return Err(PdfOpsError::Invalid("--every must be at least 1".into()));
            }
            Ok((1..=page_count)
                .collect::<Vec<u32>>()
                .chunks(*n as usize)
                .map(<[u32]>::to_vec)
                .collect())
        }
    }
}

/// Split `input` into files inside `output.path` (a directory, created when
/// missing). Returns the files written, in order.
pub fn paginate(input: &Path, options: &PaginateOptions, output: &Output) -> Result<Vec<PathBuf>> {
    let source = load_input(input, false)?;
    let page_count = u32::try_from(source.get_pages().len()).unwrap_or(u32::MAX);
    if page_count == 0 {
        return Err(PdfOpsError::Invalid(format!(
            "{}: no pages",
            input.display()
        )));
    }
    let groups = groups(&options.mode, page_count)?;
    std::fs::create_dir_all(&output.path).map_err(|e| PdfOpsError::io(&output.path, e))?;
    let stem = options.stem.clone().unwrap_or_else(|| {
        input
            .file_stem()
            .map_or_else(|| "pages".to_string(), |s| s.to_string_lossy().into_owned())
    });
    let mut written = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        let path = output.path.join(format!("{stem}-{:03}.pdf", index + 1));
        ensure_not_input(&path, &[input])?;
        let mut doc = extract_pages(&source, group, options.stamp, page_count)?;
        let file = Output {
            path: path.clone(),
            force: output.force,
        };
        save_document(&mut doc, &file)?;
        written.push(path);
    }
    Ok(written)
}

/// A new document holding `pages` (1-based, in order) of `source`.
pub fn extract_pages(
    source: &Document,
    pages: &[u32],
    stamp: bool,
    total: u32,
) -> Result<Document> {
    let source_pages = source.get_pages();
    let mut out = Document::with_version(source.version.as_str());
    let tree_id = out.new_object_id();
    // Keep every object id: the kept pages reference fonts, images and
    // resources by id; unreferenced ones are pruned at the end.
    for (id, object) in &source.objects {
        match object.type_name().unwrap_or(b"") {
            b"Catalog" | b"Pages" | b"Page" | b"Outlines" => {}
            _ => {
                out.objects.insert(*id, object.clone());
            }
        }
    }
    out.max_id = out.max_id.max(source.max_id) + 1;
    let mut kids = Vec::new();
    for page_number in pages {
        let page_id = *source_pages
            .get(page_number)
            .ok_or_else(|| PdfOpsError::Invalid(format!("page {page_number} is out of range")))?;
        let mut page = source
            .get_dictionary(page_id)
            .map_err(|e| PdfOpsError::Invalid(format!("page {page_number}: {e}")))?
            .clone();
        materialise_inherited(source, page_id, &mut page);
        page.set("Parent", tree_id);
        // A page listed twice gets a fresh id the second time.
        let new_id = if out.objects.contains_key(&page_id) {
            out.new_object_id()
        } else {
            page_id
        };
        out.objects.insert(new_id, Object::Dictionary(page));
        kids.push(Object::Reference(new_id));
        if stamp {
            stamp_page(&mut out, new_id, *page_number, total)?;
        }
    }
    let count = i64::try_from(kids.len()).unwrap_or(i64::MAX);
    out.objects.insert(
        tree_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
    );
    let catalog_id = out.add_object(dictionary! { "Type" => "Catalog", "Pages" => tree_id });
    out.trailer.set("Root", catalog_id);
    out.prune_objects();
    Ok(out)
}

/// PDF numbers are `f32` in lopdf; page coordinates fit comfortably.
#[allow(clippy::cast_possible_truncation)]
fn real(value: f64) -> Object {
    Object::Real(value as f32)
}

/// Resource name of the stamp font, chosen to avoid clashing with page fonts.
pub const STAMP_FONT: &str = "TPEStampHelv";

/// Append a content stream showing `Page n of total` at the bottom centre,
/// inside its own `q`/`Q` and `BT`/`ET`, with Helvetica added to the page's
/// own resources (a shared resource dictionary is copied first).
fn stamp_page(doc: &mut Document, page_id: ObjectId, number: u32, total: u32) -> Result<()> {
    let media = inherited_box(doc, page_id, b"MediaBox").unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    // Own the resources: a referenced dictionary may be shared with other pages.
    let resources = match doc
        .get_dictionary(page_id)
        .ok()
        .and_then(|p| p.get(b"Resources").ok())
    {
        Some(Object::Reference(id)) => doc.get_dictionary(*id).ok().cloned().unwrap_or_default(),
        Some(Object::Dictionary(d)) => d.clone(),
        _ => lopdf::Dictionary::new(),
    };
    let mut resources = resources;
    let mut fonts = match resources.get(b"Font") {
        Ok(Object::Reference(id)) => doc.get_dictionary(*id).ok().cloned().unwrap_or_default(),
        Ok(Object::Dictionary(d)) => d.clone(),
        _ => lopdf::Dictionary::new(),
    };
    fonts.set(STAMP_FONT, font_id);
    resources.set("Font", fonts);
    doc.get_dictionary_mut(page_id)
        .map_err(|e| PdfOpsError::Invalid(format!("page {number}: {e}")))?
        .set("Resources", resources);

    let text = format!("Page {number} of {total}");
    let size = 10.0_f64;
    // Helvetica digits and most lowercase glyphs are about 0.55 em wide.
    let width = f64::from(u16::try_from(text.len()).unwrap_or(u16::MAX)) * 0.55 * size;
    let x = f64::midpoint(media[0], media[2]) - width / 2.0;
    let y = media[1] + 24.0;
    let content = Content {
        operations: vec![
            Operation::new("q", vec![]),
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Name(STAMP_FONT.into()), real(size)]),
            Operation::new("Td", vec![real(x), real(y)]),
            Operation::new("Tj", vec![Object::string_literal(text)]),
            Operation::new("ET", vec![]),
            Operation::new("Q", vec![]),
        ],
    }
    .encode()
    .map_err(|e| PdfOpsError::Invalid(format!("stamp encoding: {e}")))?;
    doc.add_page_contents(page_id, content)
        .map_err(|e| PdfOpsError::Invalid(format!("page {number}: {e}")))
}
