//! `PowerPoint` (OOXML) presentations: slides in presentation order, text
//! frames, tables and speaker notes.

use roxmltree::Node;

use crate::ooxml::{self, Package};
use crate::xml;
use crate::{Block, DocumentIdentity, Format, FormatsError, FormatsResult, Note, Section};

/// Extract a `.pptx` held in `bytes`.
pub fn extract(bytes: &[u8], identity: DocumentIdentity) -> Result<FormatsResult, FormatsError> {
    let mut package = Package::open(bytes)?;
    let mut result = FormatsResult::empty(Format::Pptx, identity);
    result.metadata = ooxml::document_properties(&mut package)?;
    let slides = slide_parts(&mut package, &mut result)?;
    if slides.is_empty() {
        result.warn("partial: no slides found");
    }
    for (index, part) in slides.iter().enumerate() {
        let Some(source) = package.read_text(part)? else {
            result.warn(format!("partial: slide part {part} listed but missing"));
            continue;
        };
        let doc = xml::parse(part, &source)?;
        let mut section = Section::new("slide", index as u32, None);
        if let Some(tree) = xml::descendant(doc.root_element(), "spTree") {
            walk_shapes(tree, &mut section);
        }
        let rels = package.relationships(part)?;
        for target in rels.values().filter(|t| t.contains("notesSlides/")) {
            if let Some(notes) = package.read_text(target)? {
                let doc = xml::parse(target, &notes)?;
                let text = notes_text(doc.root_element());
                if !text.trim().is_empty() {
                    result.notes.push(Note {
                        kind: "speaker_notes".to_string(),
                        anchor: Some(format!("slide {}", index + 1)),
                        text,
                    });
                }
            }
        }
        result.sections.push(section);
    }
    result.title = result.metadata.get("title").cloned().or_else(|| {
        result
            .sections
            .iter()
            .find_map(|s| s.title.clone())
            .filter(|t| !t.trim().is_empty())
    });
    Ok(result)
}

/// Slide part names in presentation order (`p:sldIdLst`), falling back to
/// numeric file order when the list or its relationships are missing.
fn slide_parts(
    package: &mut Package,
    result: &mut FormatsResult,
) -> Result<Vec<String>, FormatsError> {
    let mut ordered = Vec::new();
    if let Some(source) = package.read_text("ppt/presentation.xml")? {
        let doc = xml::parse("ppt/presentation.xml", &source)?;
        let rels = package.relationships("ppt/presentation.xml")?;
        if let Some(list) = xml::descendant(doc.root_element(), "sldIdLst") {
            for sld in list.children().filter(|n| xml::is(*n, "sldId")) {
                let Some(rid) = xml::rel_attr(sld, "id") else {
                    continue;
                };
                match rels.get(rid) {
                    Some(target) if package.has(target) => ordered.push(target.clone()),
                    _ => result.warn(format!("partial: slide relationship {rid} has no part")),
                }
            }
        }
    }
    if ordered.is_empty() {
        ordered = package
            .names()
            .iter()
            .filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
            .cloned()
            .collect();
        ooxml::numeric_order(&mut ordered);
        if !ordered.is_empty() {
            result.warn("slide order taken from file names (no presentation.xml list)");
        }
    }
    Ok(ordered)
}

/// Shapes of a slide tree, in z-order: placeholders, text boxes, tables and
/// groups. The title placeholder sets the section title.
fn walk_shapes(tree: Node<'_, '_>, section: &mut Section) {
    for child in tree.children().filter(Node::is_element) {
        match child.tag_name().name() {
            "sp" => {
                let placeholder = xml::child(child, "nvSpPr")
                    .and_then(|n| xml::child(n, "nvPr"))
                    .and_then(|n| xml::child(n, "ph"))
                    .and_then(|ph| xml::attr(ph, "type"));
                let Some(body) = xml::child(child, "txBody") else {
                    continue;
                };
                let paragraphs = text_paragraphs(body);
                match placeholder {
                    Some("title" | "ctrTitle") => {
                        let text = paragraphs
                            .iter()
                            .map(|(_, t)| t.trim())
                            .filter(|t| !t.is_empty())
                            .collect::<Vec<_>>()
                            .join(" ");
                        if section.title.is_none() && !text.is_empty() {
                            section.title = Some(text);
                        } else if !text.is_empty() {
                            section.blocks.push(Block::heading(1, text));
                        }
                    }
                    Some("sldNum" | "dt" | "ftr" | "hdr") => {}
                    _ => {
                        for (level, text) in paragraphs {
                            if level > 0 {
                                section.blocks.push(Block::list_item(level, text));
                            } else {
                                section.blocks.push(Block::paragraph(text));
                            }
                        }
                    }
                }
            }
            "graphicFrame" => {
                if let Some(table) = xml::descendant(child, "tbl") {
                    section.blocks.push(Block::table(table_rows(table)));
                }
            }
            "grpSp" => walk_shapes(child, section),
            _ => {}
        }
    }
}

/// `(level, text)` for each `a:p` under `body`.
fn text_paragraphs(body: Node<'_, '_>) -> Vec<(u8, String)> {
    body.children()
        .filter(|n| xml::is(*n, "p"))
        .map(|p| {
            let level = xml::child(p, "pPr")
                .and_then(|pr| xml::attr(pr, "lvl"))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            (level, paragraph_text(p))
        })
        .collect()
}

/// Runs, fields and line breaks of one `a:p`.
pub fn paragraph_text(p: Node<'_, '_>) -> String {
    let mut out = String::new();
    for child in p.children().filter(Node::is_element) {
        match child.tag_name().name() {
            "r" | "fld" => {
                if let Some(t) = xml::child(child, "t") {
                    out.push_str(t.text().unwrap_or(""));
                }
            }
            "br" => out.push('\n'),
            _ => {}
        }
    }
    out
}

fn table_rows(table: Node<'_, '_>) -> Vec<Vec<String>> {
    table
        .children()
        .filter(|n| xml::is(*n, "tr"))
        .map(|tr| {
            tr.children()
                .filter(|n| xml::is(*n, "tc"))
                .map(|tc| {
                    xml::child(tc, "txBody")
                        .map(|body| {
                            text_paragraphs(body)
                                .into_iter()
                                .map(|(_, t)| t)
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default()
                        .replace('\t', " ")
                })
                .collect()
        })
        .collect()
}

/// Speaker notes: the body placeholder of a notes slide (slide-number and
/// slide-image placeholders are skipped).
fn notes_text(root: Node<'_, '_>) -> String {
    let mut paragraphs = Vec::new();
    for sp in root.descendants().filter(|n| xml::is(*n, "sp")) {
        let placeholder = xml::child(sp, "nvSpPr")
            .and_then(|n| xml::child(n, "nvPr"))
            .and_then(|n| xml::child(n, "ph"))
            .and_then(|ph| xml::attr(ph, "type"));
        if !matches!(placeholder, Some("body") | None) {
            continue;
        }
        if let Some(body) = xml::child(sp, "txBody") {
            for (_, text) in text_paragraphs(body) {
                if !text.trim().is_empty() {
                    paragraphs.push(text);
                }
            }
        }
    }
    paragraphs.join("\n")
}
