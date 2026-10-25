//! Word (OOXML) documents: `word/document.xml` paragraphs, runs and tables,
//! headers and footers, footnotes, endnotes, comments and core properties.
//! Deleted tracked changes (`w:del`) are skipped; insertions are kept.

use std::collections::BTreeMap;

use roxmltree::Node;

use crate::ooxml::{self, Package};
use crate::xml;
use crate::{Block, DocumentIdentity, Format, FormatsError, FormatsResult, Note, Section};

/// Style id -> canonical style name from `word/styles.xml` (`Heading1` -> `heading 1`).
type StyleNames = BTreeMap<String, String>;

/// Extract a `.docx` held in `bytes`.
pub fn extract(bytes: &[u8], identity: DocumentIdentity) -> Result<FormatsResult, FormatsError> {
    let mut package = Package::open(bytes)?;
    let main = package.require_text("word/document.xml")?;
    let mut result = FormatsResult::empty(Format::Docx, identity);
    result.metadata = ooxml::document_properties(&mut package)?;
    let styles = style_names(&mut package)?;

    let doc = xml::parse("word/document.xml", &main)?;
    let Some(body) = xml::descendant(doc.root_element(), "body") else {
        return Err(FormatsError::Invalid(
            "word/document.xml has no w:body".into(),
        ));
    };
    let mut section = Section::new("body", 0, None);
    walk_block_container(body, &styles, &mut section.blocks);
    result.sections.push(section);

    // Headers and footers, in part-number order.
    let mut parts: Vec<String> = package
        .names()
        .iter()
        .filter(|n| {
            (n.starts_with("word/header") || n.starts_with("word/footer")) && n.ends_with(".xml")
        })
        .cloned()
        .collect();
    ooxml::numeric_order(&mut parts);
    // Headers first, then footers, each in part-number order.
    parts.sort_by_key(|p| p.starts_with("word/footer"));
    for part in parts {
        let kind = if part.starts_with("word/header") {
            "header"
        } else {
            "footer"
        };
        let Some(source) = package.read_text(&part)? else {
            continue;
        };
        let doc = xml::parse(&part, &source)?;
        let index = result.sections.len() as u32;
        let mut section = Section::new(kind, index, None);
        walk_block_container(doc.root_element(), &styles, &mut section.blocks);
        if section.blocks.iter().any(|b| !b.is_empty()) {
            result.sections.push(section);
        }
    }

    for (part, element, kind) in [
        ("word/footnotes.xml", "footnote", "footnote"),
        ("word/endnotes.xml", "endnote", "endnote"),
    ] {
        if let Some(source) = package.read_text(part)? {
            let doc = xml::parse(part, &source)?;
            for note in doc
                .root_element()
                .children()
                .filter(|n| xml::is(*n, element))
            {
                if matches!(
                    xml::attr(note, "type"),
                    Some("separator" | "continuationSeparator" | "continuationNotice")
                ) {
                    continue;
                }
                let mut blocks = Vec::new();
                walk_block_container(note, &styles, &mut blocks);
                let text = join_blocks(&blocks);
                if !text.trim().is_empty() {
                    result.notes.push(Note {
                        kind: kind.to_string(),
                        anchor: xml::attr(note, "id").map(str::to_string),
                        text,
                    });
                }
            }
        }
    }
    if let Some(source) = package.read_text("word/comments.xml")? {
        let doc = xml::parse("word/comments.xml", &source)?;
        for comment in doc
            .root_element()
            .children()
            .filter(|n| xml::is(*n, "comment"))
        {
            let mut blocks = Vec::new();
            walk_block_container(comment, &styles, &mut blocks);
            let text = join_blocks(&blocks);
            if text.trim().is_empty() {
                continue;
            }
            let anchor = match (xml::attr(comment, "id"), xml::attr(comment, "author")) {
                (Some(id), Some(author)) => Some(format!("{id} by {author}")),
                (Some(id), None) => Some(id.to_string()),
                (None, author) => author.map(str::to_string),
            };
            result.notes.push(Note {
                kind: "comment".to_string(),
                anchor,
                text,
            });
        }
    }

    result.title = result
        .metadata
        .get("title")
        .cloned()
        .or_else(|| first_title_block(&result.sections[0].blocks));
    Ok(result)
}

fn style_names(package: &mut Package) -> Result<StyleNames, FormatsError> {
    let mut names = StyleNames::new();
    let Some(source) = package.read_text("word/styles.xml")? else {
        return Ok(names);
    };
    let doc = xml::parse("word/styles.xml", &source)?;
    for style in doc
        .root_element()
        .children()
        .filter(|n| xml::is(*n, "style"))
    {
        let (Some(id), Some(name)) = (
            xml::attr(style, "styleId"),
            xml::child(style, "name").and_then(|n| xml::attr(n, "val")),
        ) else {
            continue;
        };
        names.insert(id.to_string(), name.to_ascii_lowercase());
    }
    Ok(names)
}

fn first_title_block(blocks: &[Block]) -> Option<String> {
    blocks
        .iter()
        .find(|b| b.kind == "heading" && b.level == Some(0))
        .or_else(|| blocks.iter().find(|b| b.kind == "heading"))
        .map(|b| b.text.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Walk the children of a body-like node, appending blocks. Structured
/// document tags, tracked insertions and smart tags are transparent.
pub fn walk_block_container(node: Node<'_, '_>, styles: &StyleNames, blocks: &mut Vec<Block>) {
    for child in node.children().filter(Node::is_element) {
        match child.tag_name().name() {
            "p" => blocks.push(paragraph_block(child, styles)),
            "tbl" => blocks.push(Block::table(table_rows(child, styles))),
            "sdt" => {
                if let Some(content) = xml::child(child, "sdtContent") {
                    walk_block_container(content, styles, blocks);
                }
            }
            "sectPr" | "del" | "moveFrom" | "pPr" | "tblPr" | "tblGrid" => {}
            _ => walk_block_container(child, styles, blocks),
        }
    }
}

/// Table rows: cell text is the cell's paragraphs joined by `\n`.
fn table_rows(table: Node<'_, '_>, styles: &StyleNames) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for tr in table.descendants().filter(|n| xml::is(*n, "tr")) {
        // Only direct rows of this table: nested tables render inside cells.
        if tr.ancestors().skip(1).find(|a| xml::is(*a, "tbl")) != Some(table) {
            continue;
        }
        let mut row = Vec::new();
        for tc in tr.children().filter(|n| xml::is(*n, "tc")) {
            let mut cell_blocks = Vec::new();
            walk_block_container(tc, styles, &mut cell_blocks);
            row.push(join_blocks(&cell_blocks).replace('\t', " "));
            let span: usize = xml::child(tc, "tcPr")
                .and_then(|p| xml::child(p, "gridSpan"))
                .and_then(|g| xml::attr(g, "val"))
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            for _ in 1..span {
                row.push(String::new());
            }
        }
        rows.push(row);
    }
    rows
}

fn join_blocks(blocks: &[Block]) -> String {
    blocks
        .iter()
        .filter(|b| !b.is_empty())
        .map(|b| b.text.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// One `w:p` as a block, classified by its paragraph style and numbering.
fn paragraph_block(p: Node<'_, '_>, styles: &StyleNames) -> Block {
    let mut text = String::new();
    run_text(p, &mut text);
    let ppr = xml::child(p, "pPr");
    let style_id = ppr
        .and_then(|p| xml::child(p, "pStyle"))
        .and_then(|s| xml::attr(s, "val"))
        .unwrap_or("");
    let style_name = styles
        .get(style_id)
        .cloned()
        .unwrap_or_else(|| style_id.to_ascii_lowercase());
    if let Some(level) = heading_level(&style_name) {
        return Block::heading(level, text);
    }
    if let Some(level) = ppr
        .and_then(|p| xml::child(p, "outlineLvl"))
        .and_then(|o| xml::attr(o, "val"))
        .and_then(|v| v.parse::<u8>().ok())
    {
        return Block::heading(level + 1, text);
    }
    if let Some(num) = ppr.and_then(|p| xml::child(p, "numPr")) {
        let depth = xml::child(num, "ilvl")
            .and_then(|i| xml::attr(i, "val"))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        return Block::list_item(depth, text);
    }
    if style_name.contains("quote") {
        return Block::new("quote", text);
    }
    Block::paragraph(text)
}

/// `title` is level 0 (so the first one is the document title), `heading N`
/// and `HeadingN` are level N.
fn heading_level(style: &str) -> Option<u8> {
    let compact: String = style.chars().filter(|c| !c.is_whitespace()).collect();
    if compact == "title" {
        return Some(0);
    }
    let digits = compact.strip_prefix("heading")?;
    let level: u8 = digits.parse().ok()?;
    (1..=9).contains(&level).then_some(level)
}

/// Append the visible text of runs below `node`: `w:t`, tabs, breaks,
/// footnote markers; `w:del` and field codes are skipped; text boxes and
/// nested paragraphs contribute their text separated by newlines.
pub fn run_text(node: Node<'_, '_>, out: &mut String) {
    for child in node.children().filter(Node::is_element) {
        match child.tag_name().name() {
            "t" => out.push_str(child.text().unwrap_or("")),
            "tab" => out.push('\t'),
            "br" | "cr" => out.push('\n'),
            "noBreakHyphen" => out.push('\u{2011}'),
            "footnoteReference" | "endnoteReference" => {
                if let Some(id) = xml::attr(child, "id") {
                    out.push('[');
                    out.push_str(id);
                    out.push(']');
                }
            }
            "sym" => {
                if let Some(code) = xml::attr(child, "char")
                    .and_then(|c| u32::from_str_radix(c, 16).ok())
                    .and_then(char::from_u32)
                {
                    out.push(code);
                }
            }
            "del"
            | "moveFrom"
            | "rPr"
            | "pPr"
            | "instrText"
            | "fldChar"
            | "delText"
            | "commentRangeStart"
            | "commentRangeEnd"
            | "commentReference"
            | "proofErr"
            | "bookmarkStart"
            | "bookmarkEnd"
            | "softHyphen"
            | "lastRenderedPageBreak" => {}
            "p" => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                run_text(child, out);
            }
            _ => run_text(child, out),
        }
    }
}
