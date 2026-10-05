//! Markdown: front matter extraction and normalisation to blocks. Headings
//! (ATX and Setext), lists, fenced code, block quotes and pipe tables are
//! recognised; inline emphasis, links and images are reduced to their text.

use std::collections::BTreeMap;

use crate::html::decode_entities;
use crate::text;
use crate::{Block, DocumentIdentity, Format, FormatsResult, Section};

/// Extract a Markdown file.
pub fn extract(bytes: &[u8], identity: DocumentIdentity) -> FormatsResult {
    let decoded = text::decode(bytes);
    let mut result = FormatsResult::empty(Format::Markdown, identity);
    result
        .metadata
        .insert("encoding".to_string(), decoded.encoding.to_string());
    let normalised = text::normalise_lines(&decoded.text);
    let (front, body) = split_front_matter(&normalised);
    if let Some(front) = front {
        let mut section = Section::new("front_matter", 0, None);
        section
            .blocks
            .push(Block::new("code", front.raw.trim_end()));
        result.sections.push(section);
        for (key, value) in front.fields {
            result.metadata.insert(key, value);
        }
    }
    let blocks = parse_blocks(body);
    result.title = result.metadata.get("title").cloned().or_else(|| {
        blocks
            .iter()
            .find(|b| b.kind == "heading" && b.level == Some(1))
            .map(|b| b.text.clone())
    });
    let index = result.sections.len() as u32;
    let mut section = Section::new("body", index, None);
    section.blocks = blocks;
    result.sections.push(section);
    result
}

/// Front matter found at the top of a file.
pub struct FrontMatter {
    /// The raw lines between the fences.
    pub raw: String,
    /// `key: value` (YAML) or `key = value` (TOML) pairs; nested values are
    /// not interpreted.
    pub fields: BTreeMap<String, String>,
}

/// Split `---`/`+++` front matter off the body.
pub fn split_front_matter(source: &str) -> (Option<FrontMatter>, &str) {
    let fence = if source.starts_with("---\n") {
        "---"
    } else if source.starts_with("+++\n") {
        "+++"
    } else {
        return (None, source);
    };
    let after = &source[4..];
    let mut offset = 0;
    for line in after.split_inclusive('\n') {
        let trimmed = line.trim_end();
        if trimmed == fence || (fence == "---" && trimmed == "...") {
            let raw = after[..offset].to_string();
            let body = &after[offset + line.len()..];
            let mut fields = BTreeMap::new();
            for raw_line in raw.lines() {
                let separator = if fence == "---" { ':' } else { '=' };
                if raw_line.starts_with(char::is_whitespace) || raw_line.starts_with('#') {
                    continue;
                }
                if let Some((key, value)) = raw_line.split_once(separator) {
                    let key = key.trim();
                    let value = value.trim().trim_matches(['"', '\'']).to_string();
                    if !key.is_empty() && !value.is_empty() {
                        fields.insert(key.to_string(), value);
                    }
                }
            }
            return (Some(FrontMatter { raw, fields }), body);
        }
        offset += line.len();
    }
    (None, source)
}

fn is_fence(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") {
        Some("```")
    } else if trimmed.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

fn is_thematic_break(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && (compact.chars().all(|c| c == '-')
            || compact.chars().all(|c| c == '*')
            || compact.chars().all(|c| c == '_'))
}

fn setext_level(line: &str) -> Option<u8> {
    let trimmed = line.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|c| c == '=') {
        Some(1)
    } else if !trimmed.is_empty() && trimmed.chars().all(|c| c == '-') {
        Some(2)
    } else {
        None
    }
}

/// `(depth, text)` when `line` is a list item.
fn list_item(line: &str) -> Option<(u8, &str)> {
    let indent = line.len() - line.trim_start().len();
    let trimmed = line.trim_start();
    let rest = if let Some(rest) = trimmed.strip_prefix(['-', '*', '+']) {
        rest
    } else {
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let after = &trimmed[digits..];
        after.strip_prefix(['.', ')'])?
    };
    if !(rest.starts_with(' ') || rest.starts_with('\t')) {
        return None;
    }
    let text = rest.trim_start();
    let text = text
        .strip_prefix("[ ] ")
        .or_else(|| text.strip_prefix("[x] "))
        .or_else(|| text.strip_prefix("[X] "))
        .unwrap_or(text);
    Some(((indent / 2).min(8) as u8, text))
}

fn table_cells(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut chars = line.trim().chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                chars.next();
                current.push('|');
            }
            '|' => cells.push(std::mem::take(&mut current)),
            other => current.push(other),
        }
    }
    cells.push(current);
    if line.trim().starts_with('|') {
        cells.remove(0);
    }
    if line.trim().ends_with('|') && !line.trim().ends_with("\\|") {
        cells.pop();
    }
    cells.into_iter().map(|c| inline(c.trim())).collect()
}

fn is_table_separator(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.contains('-') && trimmed.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// Turn Markdown body text into blocks.
pub fn parse_blocks(body: &str) -> Vec<Block> {
    let lines: Vec<&str> = body.lines().collect();
    let mut blocks = Vec::new();
    let mut paragraph: Vec<String> = Vec::new();
    let mut i = 0;
    let flush = |paragraph: &mut Vec<String>, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            let text = join_soft(paragraph);
            blocks.push(Block::paragraph(text));
            paragraph.clear();
        }
    };
    while i < lines.len() {
        let line = lines[i];
        if let Some(fence) = is_fence(line) {
            flush(&mut paragraph, &mut blocks);
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with(fence) {
                code.push(lines[i]);
                i += 1;
            }
            blocks.push(Block::new("code", code.join("\n")));
            i += 1;
            continue;
        }
        if line.trim().is_empty() {
            flush(&mut paragraph, &mut blocks);
            i += 1;
            continue;
        }
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix('#') {
            let hashes = 1 + rest.chars().take_while(|c| *c == '#').count();
            let after = &trimmed[hashes..];
            if hashes <= 6 && (after.is_empty() || after.starts_with(' ')) {
                flush(&mut paragraph, &mut blocks);
                let text = after.trim().trim_end_matches('#').trim();
                blocks.push(Block::heading(hashes as u8, inline(text)));
                i += 1;
                continue;
            }
        }
        if let Some(level) = setext_level(line) {
            if paragraph.len() == 1 {
                let text = paragraph.pop().unwrap_or_default();
                blocks.push(Block::heading(level, text));
                i += 1;
                continue;
            }
            if paragraph.is_empty() && is_thematic_break(line) {
                i += 1;
                continue;
            }
        }
        if is_thematic_break(line) && paragraph.is_empty() {
            i += 1;
            continue;
        }
        if trimmed.starts_with('>') {
            flush(&mut paragraph, &mut blocks);
            let mut quoted = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let inner = lines[i].trim_start()[1..]
                    .strip_prefix(' ')
                    .unwrap_or(&lines[i].trim_start()[1..]);
                quoted.push(inline(inner));
                i += 1;
            }
            blocks.push(Block::new("quote", join_soft(&quoted)));
            continue;
        }
        if trimmed.contains('|') && i + 1 < lines.len() && is_table_separator(lines[i + 1]) {
            flush(&mut paragraph, &mut blocks);
            let mut rows = vec![table_cells(line)];
            i += 2;
            while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
                rows.push(table_cells(lines[i]));
                i += 1;
            }
            blocks.push(Block::table(rows));
            continue;
        }
        if let Some((depth, text)) = list_item(line) {
            flush(&mut paragraph, &mut blocks);
            let mut item = vec![inline(text)];
            i += 1;
            while i < lines.len()
                && !lines[i].trim().is_empty()
                && list_item(lines[i]).is_none()
                && lines[i].starts_with([' ', '\t'])
            {
                item.push(inline(lines[i].trim()));
                i += 1;
            }
            blocks.push(Block::list_item(depth, join_soft(&item)));
            continue;
        }
        if is_reference_definition(trimmed) {
            i += 1;
            continue;
        }
        paragraph.push(inline(trimmed));
        i += 1;
    }
    flush(&mut paragraph, &mut blocks);
    blocks
}

fn is_reference_definition(line: &str) -> bool {
    line.starts_with('[')
        && line.find("]:").is_some_and(|i| {
            !line[1..i].is_empty() && line[i + 2..].trim().contains(|c: char| !c.is_whitespace())
        })
}

/// Join lines with a space; a hard break (two trailing spaces or `\`) keeps
/// the newline.
fn join_soft(lines: &[String]) -> String {
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            if out.ends_with('\\') {
                out.pop();
                out.push('\n');
            } else if out.ends_with("  ") {
                out = out.trim_end().to_string();
                out.push('\n');
            } else {
                out.push(' ');
            }
        }
        out.push_str(line);
    }
    out.trim_end().to_string()
}

/// Reduce inline Markdown to its text: links and images to their text,
/// code spans to their content, emphasis markers removed, entities decoded,
/// `<br>` to a newline and other inline tags dropped.
pub fn inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut open: Vec<(char, usize)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() => {
                out.push(chars[i + 1]);
                i += 2;
            }
            '`' => {
                let run = chars[i..].iter().take_while(|c| **c == '`').count();
                if let Some(end) = find_backtick_close(&chars, i + run, run) {
                    let span: String = chars[i + run..end].iter().collect();
                    out.push_str(span.trim());
                    i = end + run;
                } else {
                    out.push('`');
                    i += 1;
                }
            }
            '!' if chars.get(i + 1) == Some(&'[') => {
                if let Some((label, end)) = bracket_link(&chars, i + 1) {
                    out.push_str(&label);
                    i = end;
                } else {
                    out.push('!');
                    i += 1;
                }
            }
            '[' => {
                if let Some((label, end)) = bracket_link(&chars, i) {
                    out.push_str(&label);
                    i = end;
                } else {
                    out.push('[');
                    i += 1;
                }
            }
            '<' => {
                let close = chars[i..].iter().position(|c| *c == '>');
                match close {
                    Some(len) if len > 1 => {
                        let tag: String = chars[i + 1..i + len].iter().collect();
                        let lower = tag.trim().to_ascii_lowercase();
                        if lower.starts_with("http://")
                            || lower.starts_with("https://")
                            || lower.starts_with("mailto:")
                        {
                            out.push_str(tag.trim());
                        } else if lower.starts_with("br") {
                            out.push('\n');
                        } else if !lower
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_alphabetic() || c == '/')
                        {
                            out.push('<');
                            i += 1;
                            continue;
                        }
                        i += len + 1;
                    }
                    _ => {
                        out.push('<');
                        i += 1;
                    }
                }
            }
            '*' | '_' | '~' => {
                let run = chars[i..].iter().take_while(|x| **x == c).count();
                let word_char = |ch: Option<&char>| ch.is_some_and(|ch| ch.is_alphanumeric());
                let intraword = c == '_'
                    && word_char(chars.get(i.wrapping_sub(1)))
                    && word_char(chars.get(i + run));
                let followed_by_space = chars.get(i + run).is_none_or(|x| x.is_whitespace());
                let preceded_by_space = i == 0 || chars[i - 1].is_whitespace();
                let closes = open.last() == Some(&(c, run)) && !preceded_by_space;
                let has_close = chars[i + run..]
                    .windows(run)
                    .any(|w| w.iter().all(|x| *x == c));
                if closes {
                    open.pop();
                } else if !intraword && !followed_by_space && has_close && run <= 3 {
                    open.push((c, run));
                } else {
                    out.extend(chars[i..i + run].iter());
                }
                i += run;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    decode_entities(&out)
}

fn find_backtick_close(chars: &[char], from: usize, run: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '`' {
            let len = chars[i..].iter().take_while(|c| **c == '`').count();
            if len == run {
                return Some(i);
            }
            i += len;
        } else {
            i += 1;
        }
    }
    None
}

/// Parse `[label](target)` or `[label][ref]` or `[label]` starting at `[`;
/// returns the inlined label and the index after the construct.
fn bracket_link(chars: &[char], start: usize) -> Option<(String, usize)> {
    let mut depth = 0;
    let mut i = start;
    while i < chars.len() {
        match chars[i] {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if i >= chars.len() {
        return None;
    }
    let label: String = chars[start + 1..i].iter().collect();
    let label = inline(&label);
    let mut end = i + 1;
    match chars.get(end) {
        Some('(') => {
            let close = chars[end..].iter().position(|c| *c == ')')?;
            end += close + 1;
        }
        Some('[') => {
            let close = chars[end..].iter().position(|c| *c == ']')?;
            end += close + 1;
        }
        _ => {}
    }
    Some((label, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_reduction() {
        assert_eq!(
            inline("**bold** and *em* and `code` and [link](http://x) ![img](y.png)"),
            "bold and em and code and link img"
        );
        assert_eq!(
            inline("snake_case_name stays; \\*literal\\*"),
            "snake_case_name stays; *literal*"
        );
        assert_eq!(
            inline("a &amp; b<br>c <https://e.org>"),
            "a & b\nc https://e.org"
        );
        assert_eq!(inline("2 * 3 * 4 = 24"), "2 * 3 * 4 = 24");
    }

    #[test]
    fn front_matter_and_blocks() {
        let src = "---\ntitle: \"Hello\"\ntags: [a, b]\n---\n# Head\n\nPara one\ncontinues.\n\n- item\n  more\n- other\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nlet x = 1;\n```\n\nSub\n---\n> quoted\n";
        let (front, body) = split_front_matter(src);
        let front = front.unwrap();
        assert_eq!(front.fields.get("title").map(String::as_str), Some("Hello"));
        let blocks = parse_blocks(body);
        let kinds: Vec<(&str, &str)> = blocks
            .iter()
            .map(|b| (b.kind.as_str(), b.text.as_str()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("heading", "Head"),
                ("paragraph", "Para one continues."),
                ("list_item", "item more"),
                ("list_item", "other"),
                ("table", "a\tb\n1\t2"),
                ("code", "let x = 1;"),
                ("heading", "Sub"),
                ("quote", "quoted"),
            ]
        );
        assert_eq!(blocks[6].level, Some(2));
    }
}
