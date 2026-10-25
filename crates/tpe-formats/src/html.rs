//! HTML to text with heading, list and table structure. Scripts, styles and
//! templates are dropped, entities decoded, whitespace collapsed outside
//! `<pre>`. The tokenizer is tolerant: unknown or unbalanced tags never fail.

use std::collections::BTreeMap;

use crate::text;
use crate::{Block, DocumentIdentity, Format, FormatsResult, Section};

/// Extract an HTML file.
pub fn extract(bytes: &[u8], identity: DocumentIdentity) -> FormatsResult {
    let decoded = text::decode(bytes);
    let mut result = FormatsResult::empty(Format::Html, identity);
    result
        .metadata
        .insert("encoding".to_string(), decoded.encoding.to_string());
    if decoded.encoding == "windows-1252" {
        result.warn("input is not valid UTF-8; decoded as windows-1252");
    }
    let parsed = parse(&decoded.text);
    result.title = parsed.title.clone().filter(|t| !t.is_empty()).or_else(|| {
        parsed
            .blocks
            .iter()
            .find(|b| b.kind == "heading" && b.level == Some(1))
            .map(|b| b.text.clone())
    });
    for (key, value) in parsed.metadata {
        result.metadata.insert(key, value);
    }
    let mut section = Section::new("body", 0, None);
    section.blocks = parsed.blocks;
    result.sections.push(section);
    result
}

/// What [`parse`] found.
#[derive(Debug, Default)]
pub struct Parsed {
    pub title: Option<String>,
    /// `lang`, `meta.<name>` for description/author/keywords/og:* tags.
    pub metadata: BTreeMap<String, String>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Default)]
struct Table {
    rows: Vec<Vec<String>>,
    row: Vec<String>,
    cell: String,
    in_cell: bool,
}

#[derive(Debug, Default)]
struct Builder {
    out: Parsed,
    buffer: String,
    kind: String,
    level: Option<u8>,
    list_depth: u8,
    pre_depth: u32,
    quote_depth: u32,
    skip_depth: u32,
    in_title: bool,
    title: String,
    tables: Vec<Table>,
}

const BLOCK_TAGS: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "nav",
    "aside",
    "ul",
    "ol",
    "dl",
    "dt",
    "dd",
    "figure",
    "figcaption",
    "hr",
    "address",
    "form",
    "fieldset",
    "details",
    "summary",
    "center",
    "body",
    "html",
    "head",
    "menu",
    "legend",
    "caption",
    "thead",
    "tbody",
    "tfoot",
    "option",
    "button",
    "label",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "pre",
    "blockquote",
    "table",
    "tr",
    "td",
    "th",
];

impl Builder {
    fn push_text(&mut self, raw: &str) {
        if self.skip_depth > 0 {
            return;
        }
        let decoded = decode_entities(raw);
        if self.in_title {
            self.title.push_str(&decoded);
            return;
        }
        if let Some(table) = self.tables.last_mut() {
            if table.in_cell {
                append_collapsed(&mut table.cell, &decoded, false);
            }
            return;
        }
        append_collapsed(&mut self.buffer, &decoded, self.pre_depth > 0);
    }

    fn flush(&mut self) {
        if let Some(table) = self.tables.last_mut() {
            if table.in_cell && !table.cell.ends_with('\n') && !table.cell.is_empty() {
                table.cell.push('\n');
            }
            return;
        }
        let text = if self.pre_depth > 0 {
            self.buffer.trim_matches('\n').to_string()
        } else {
            self.buffer.trim().to_string()
        };
        self.buffer.clear();
        if text.is_empty() {
            self.kind.clear();
            self.level = None;
            return;
        }
        let kind = if self.kind.is_empty() {
            if self.quote_depth > 0 {
                "quote"
            } else {
                "paragraph"
            }
        } else {
            self.kind.as_str()
        };
        self.out.blocks.push(Block {
            kind: kind.to_string(),
            level: self.level,
            text,
            rows: None,
        });
        self.kind.clear();
        self.level = None;
    }

    fn open(&mut self, name: &str, attrs: &[(String, String)]) {
        match name {
            "script" | "style" | "template" | "noscript" | "svg" => self.skip_depth += 1,
            "title" => self.in_title = true,
            "meta" => {
                let key = attrs
                    .iter()
                    .find(|(k, _)| k == "name" || k == "property")
                    .map(|(_, v)| v.to_ascii_lowercase());
                let content = attrs
                    .iter()
                    .find(|(k, _)| k == "content")
                    .map(|(_, v)| v.clone());
                if let (Some(key), Some(content)) = (key, content)
                    && matches!(
                        key.as_str(),
                        "description"
                            | "author"
                            | "keywords"
                            | "og:title"
                            | "og:description"
                            | "dc.title"
                            | "dc.creator"
                            | "citation_title"
                            | "citation_doi"
                            | "citation_author"
                    )
                {
                    let entry = self.out.metadata.entry(format!("meta.{key}")).or_default();
                    if entry.is_empty() {
                        *entry = content;
                    } else {
                        entry.push_str("; ");
                        entry.push_str(&content);
                    }
                }
            }
            "html" => {
                if let Some((_, lang)) = attrs.iter().find(|(k, _)| k == "lang") {
                    self.out.metadata.insert("lang".to_string(), lang.clone());
                }
            }
            "br" => {
                if let Some(table) = self.tables.last_mut() {
                    table.cell.push('\n');
                } else {
                    self.buffer.push('\n');
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush();
                self.kind = "heading".to_string();
                self.level = name[1..].parse().ok();
            }
            "li" => {
                self.flush();
                self.kind = "list_item".to_string();
                self.level = Some(self.list_depth.saturating_sub(1));
            }
            "ul" | "ol" => {
                self.flush();
                self.list_depth = self.list_depth.saturating_add(1);
            }
            "pre" => {
                self.flush();
                self.pre_depth += 1;
                self.kind = "code".to_string();
            }
            "blockquote" => {
                self.flush();
                self.quote_depth += 1;
            }
            "table" => {
                self.flush();
                self.tables.push(Table::default());
            }
            "tr" => {
                if let Some(table) = self.tables.last_mut() {
                    end_cell(table);
                    if !table.row.is_empty() {
                        table.rows.push(std::mem::take(&mut table.row));
                    }
                }
            }
            "td" | "th" => {
                if let Some(table) = self.tables.last_mut() {
                    end_cell(table);
                    table.in_cell = true;
                }
            }
            _ if BLOCK_TAGS.contains(&name) => self.flush(),
            _ => {}
        }
    }

    fn close(&mut self, name: &str) {
        match name {
            "script" | "style" | "template" | "noscript" | "svg" => {
                self.skip_depth = self.skip_depth.saturating_sub(1);
            }
            "title" => {
                self.in_title = false;
                let title = collapse(&self.title);
                if self.out.title.is_none() && !title.is_empty() {
                    self.out.title = Some(title);
                }
                self.title.clear();
            }
            "ul" | "ol" => {
                self.flush();
                self.list_depth = self.list_depth.saturating_sub(1);
            }
            "pre" => {
                self.flush();
                self.pre_depth = self.pre_depth.saturating_sub(1);
            }
            "blockquote" => {
                self.flush();
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            "td" | "th" => {
                if let Some(table) = self.tables.last_mut() {
                    end_cell(table);
                }
            }
            "tr" => {
                if let Some(table) = self.tables.last_mut() {
                    end_cell(table);
                    if !table.row.is_empty() {
                        table.rows.push(std::mem::take(&mut table.row));
                    }
                }
            }
            "table" => {
                if let Some(mut table) = self.tables.pop() {
                    end_cell(&mut table);
                    if !table.row.is_empty() {
                        table.rows.push(std::mem::take(&mut table.row));
                    }
                    if let Some(parent) = self.tables.last_mut() {
                        let text = table
                            .rows
                            .iter()
                            .map(|r| r.join(" "))
                            .collect::<Vec<_>>()
                            .join("\n");
                        parent.cell.push_str(&text);
                    } else if !table.rows.is_empty() {
                        self.out.blocks.push(Block::table(table.rows));
                    }
                }
            }
            _ if BLOCK_TAGS.contains(&name) => self.flush(),
            _ => {}
        }
    }
}

fn end_cell(table: &mut Table) {
    if table.in_cell {
        let cell = table.cell.trim().replace('\t', " ");
        table.row.push(cell);
        table.cell.clear();
        table.in_cell = false;
    }
}

/// Append `text` to `buffer`, collapsing whitespace runs to one space
/// (unless `pre`), and never starting with a space after a newline.
fn append_collapsed(buffer: &mut String, text: &str, pre: bool) {
    if pre {
        buffer.push_str(text);
        return;
    }
    for c in text.chars() {
        if c.is_whitespace() {
            if !buffer.is_empty() && !buffer.ends_with([' ', '\n']) {
                buffer.push(' ');
            }
        } else {
            buffer.push(c);
        }
    }
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Tokenise and build blocks.
pub fn parse(source: &str) -> Parsed {
    let mut builder = Builder::default();
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut text_start = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &source[i..];
        if rest.starts_with("<!--") {
            builder.push_text(&source[text_start..i]);
            let end = rest.find("-->").map_or(bytes.len(), |e| i + e + 3);
            i = end;
            text_start = i;
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            builder.push_text(&source[text_start..i]);
            let end = rest.find('>').map_or(bytes.len(), |e| i + e + 1);
            i = end;
            text_start = i;
            continue;
        }
        let next = bytes.get(i + 1).copied().unwrap_or(b' ');
        if !(next.is_ascii_alphabetic() || next == b'/') {
            i += 1;
            continue;
        }
        builder.push_text(&source[text_start..i]);
        let (tag, consumed) = read_tag(rest);
        i += consumed;
        text_start = i;
        match tag {
            Tag::Open { name, attrs } => {
                if matches!(name.as_str(), "script" | "style" | "template") {
                    let close = format!("</{name}");
                    let lower = source[i..].to_ascii_lowercase();
                    let end = lower.find(&close).map_or(bytes.len(), |e| i + e);
                    i = end;
                    text_start = i;
                    continue;
                }
                builder.open(&name, &attrs);
            }
            Tag::Close(name) => builder.close(&name),
        }
    }
    builder.push_text(&source[text_start..]);
    builder.flush();
    while let Some(table) = builder.tables.pop() {
        if !table.rows.is_empty() {
            builder.out.blocks.push(Block::table(table.rows));
        }
    }
    builder.out
}

enum Tag {
    Open {
        name: String,
        attrs: Vec<(String, String)>,
    },
    Close(String),
}

/// Read one tag starting at `<`; returns the tag and bytes consumed.
fn read_tag(rest: &str) -> (Tag, usize) {
    let end = rest.find('>').map_or(rest.len(), |e| e + 1);
    let inner = rest[1..end].trim_end_matches('>').trim();
    if let Some(name) = inner.strip_prefix('/') {
        let name = name
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        return (Tag::Close(name), end);
    }
    let inner = inner.trim_end_matches('/').trim();
    let name_end = inner
        .find(|c: char| c.is_whitespace() || c == '/')
        .unwrap_or(inner.len());
    let name = inner[..name_end].to_ascii_lowercase();
    let attrs = read_attrs(&inner[name_end..]);
    (Tag::Open { name, attrs }, end)
}

fn read_attrs(mut s: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    loop {
        s = s.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        if s.is_empty() {
            break;
        }
        let key_end = s
            .find(|c: char| c.is_whitespace() || c == '=' || c == '/')
            .unwrap_or(s.len());
        let key = s[..key_end].to_ascii_lowercase();
        s = s[key_end..].trim_start();
        let mut value = String::new();
        if let Some(after) = s.strip_prefix('=') {
            let after = after.trim_start();
            if let Some(q) = after.strip_prefix('"') {
                let close = q.find('"').unwrap_or(q.len());
                value = q[..close].to_string();
                s = &q[close.min(q.len())..];
                s = s.strip_prefix('"').unwrap_or(s);
            } else if let Some(q) = after.strip_prefix('\'') {
                let close = q.find('\'').unwrap_or(q.len());
                value = q[..close].to_string();
                s = &q[close.min(q.len())..];
                s = s.strip_prefix('\'').unwrap_or(s);
            } else {
                let close = after
                    .find(|c: char| c.is_whitespace())
                    .unwrap_or(after.len());
                value = after[..close].to_string();
                s = &after[close..];
            }
        }
        if !key.is_empty() {
            attrs.push((key, decode_entities(&value)));
        }
    }
    attrs
}

/// Named entities beyond the XML five: Latin-1 and the common typographic set.
const NAMED: &[(&str, char)] = &[
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("quot", '"'),
    ("apos", '\''),
    ("nbsp", '\u{a0}'),
    ("iexcl", '¡'),
    ("cent", '¢'),
    ("pound", '£'),
    ("curren", '¤'),
    ("yen", '¥'),
    ("brvbar", '¦'),
    ("sect", '§'),
    ("uml", '¨'),
    ("copy", '©'),
    ("ordf", 'ª'),
    ("laquo", '«'),
    ("not", '¬'),
    ("shy", '\u{ad}'),
    ("reg", '®'),
    ("macr", '¯'),
    ("deg", '°'),
    ("plusmn", '±'),
    ("sup2", '²'),
    ("sup3", '³'),
    ("acute", '´'),
    ("micro", 'µ'),
    ("para", '¶'),
    ("middot", '·'),
    ("cedil", '¸'),
    ("sup1", '¹'),
    ("ordm", 'º'),
    ("raquo", '»'),
    ("frac14", '¼'),
    ("frac12", '½'),
    ("frac34", '¾'),
    ("iquest", '¿'),
    ("Agrave", 'À'),
    ("Aacute", 'Á'),
    ("Acirc", 'Â'),
    ("Atilde", 'Ã'),
    ("Auml", 'Ä'),
    ("Aring", 'Å'),
    ("AElig", 'Æ'),
    ("Ccedil", 'Ç'),
    ("Egrave", 'È'),
    ("Eacute", 'É'),
    ("Ecirc", 'Ê'),
    ("Euml", 'Ë'),
    ("Igrave", 'Ì'),
    ("Iacute", 'Í'),
    ("Icirc", 'Î'),
    ("Iuml", 'Ï'),
    ("ETH", 'Ð'),
    ("Ntilde", 'Ñ'),
    ("Ograve", 'Ò'),
    ("Oacute", 'Ó'),
    ("Ocirc", 'Ô'),
    ("Otilde", 'Õ'),
    ("Ouml", 'Ö'),
    ("times", '×'),
    ("Oslash", 'Ø'),
    ("Ugrave", 'Ù'),
    ("Uacute", 'Ú'),
    ("Ucirc", 'Û'),
    ("Uuml", 'Ü'),
    ("Yacute", 'Ý'),
    ("THORN", 'Þ'),
    ("szlig", 'ß'),
    ("agrave", 'à'),
    ("aacute", 'á'),
    ("acirc", 'â'),
    ("atilde", 'ã'),
    ("auml", 'ä'),
    ("aring", 'å'),
    ("aelig", 'æ'),
    ("ccedil", 'ç'),
    ("egrave", 'è'),
    ("eacute", 'é'),
    ("ecirc", 'ê'),
    ("euml", 'ë'),
    ("igrave", 'ì'),
    ("iacute", 'í'),
    ("icirc", 'î'),
    ("iuml", 'ï'),
    ("eth", 'ð'),
    ("ntilde", 'ñ'),
    ("ograve", 'ò'),
    ("oacute", 'ó'),
    ("ocirc", 'ô'),
    ("otilde", 'õ'),
    ("ouml", 'ö'),
    ("divide", '÷'),
    ("oslash", 'ø'),
    ("ugrave", 'ù'),
    ("uacute", 'ú'),
    ("ucirc", 'û'),
    ("uuml", 'ü'),
    ("yacute", 'ý'),
    ("thorn", 'þ'),
    ("yuml", 'ÿ'),
    ("OElig", 'Œ'),
    ("oelig", 'œ'),
    ("Scaron", 'Š'),
    ("scaron", 'š'),
    ("Yuml", 'Ÿ'),
    ("fnof", 'ƒ'),
    ("circ", 'ˆ'),
    ("tilde", '˜'),
    ("ensp", '\u{2002}'),
    ("emsp", '\u{2003}'),
    ("thinsp", '\u{2009}'),
    ("zwnj", '\u{200c}'),
    ("zwj", '\u{200d}'),
    ("ndash", '–'),
    ("mdash", '—'),
    ("lsquo", '‘'),
    ("rsquo", '’'),
    ("sbquo", '‚'),
    ("ldquo", '“'),
    ("rdquo", '”'),
    ("bdquo", '„'),
    ("dagger", '†'),
    ("Dagger", '‡'),
    ("bull", '•'),
    ("hellip", '…'),
    ("permil", '‰'),
    ("prime", '′'),
    ("Prime", '″'),
    ("lsaquo", '‹'),
    ("rsaquo", '›'),
    ("euro", '€'),
    ("trade", '™'),
    ("larr", '←'),
    ("uarr", '↑'),
    ("rarr", '→'),
    ("darr", '↓'),
    ("harr", '↔'),
    ("minus", '−'),
    ("ne", '≠'),
    ("le", '≤'),
    ("ge", '≥'),
    ("infin", '∞'),
    ("alpha", 'α'),
    ("beta", 'β'),
    ("gamma", 'γ'),
    ("delta", 'δ'),
    ("epsilon", 'ε'),
    ("theta", 'θ'),
    ("lambda", 'λ'),
    ("mu", 'μ'),
    ("pi", 'π'),
    ("sigma", 'σ'),
    ("tau", 'τ'),
    ("phi", 'φ'),
    ("omega", 'ω'),
    ("Delta", 'Δ'),
    ("Sigma", 'Σ'),
    ("Omega", 'Ω'),
    ("check", '✓'),
];

/// Decode `&name;`, `&#NNN;` and `&#xHH;`; unknown references are kept.
pub fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '#'))
            .unwrap_or(after.len());
        let name = &after[..end];
        let terminated = after[end..].starts_with(';');
        let decoded = decode_one(name);
        match decoded {
            Some(c) if !name.is_empty() && (terminated || name.starts_with('#')) => {
                out.push(c);
                rest = &after[end + usize::from(terminated)..];
            }
            _ => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_one(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = if let Some(hex) = num.strip_prefix(['x', 'X']) {
            u32::from_str_radix(hex, 16).ok()?
        } else {
            num.parse::<u32>().ok()?
        };
        return match code {
            0x80..=0x9F => encoding_rs::WINDOWS_1252
                .decode_without_bom_handling(&[code as u8])
                .0
                .chars()
                .next(),
            _ => char::from_u32(code),
        };
    }
    NAMED.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities() {
        assert_eq!(
            decode_entities("a &amp; b &lt;c&gt; &#65;&#x42; &eacute;"),
            "a & b <c> AB é"
        );
        assert_eq!(decode_entities("AT&T &unknown; &#150;"), "AT&T &unknown; –");
    }

    #[test]
    fn structure() {
        let parsed = parse(
            "<html lang=\"en\"><head><title> My  Page </title><style>p{}</style>\
             <meta name=\"author\" content=\"A &amp; B\"></head><body>\
             <h1>Head</h1><p>One <b>two</b>\n three</p><script>x<y</script>\
             <ul><li>a</li><li>b<ul><li>c</li></ul></li></ul>\
             <table><tr><th>k</th><th>v</th></tr><tr><td>1</td><td>2</td></tr></table>\
             <pre>  x\n   y</pre></body></html>",
        );
        assert_eq!(parsed.title.as_deref(), Some("My Page"));
        assert_eq!(parsed.metadata.get("lang").map(String::as_str), Some("en"));
        assert_eq!(
            parsed.metadata.get("meta.author").map(String::as_str),
            Some("A & B")
        );
        let kinds: Vec<(&str, &str)> = parsed
            .blocks
            .iter()
            .map(|b| (b.kind.as_str(), b.text.as_str()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("heading", "Head"),
                ("paragraph", "One two three"),
                ("list_item", "a"),
                ("list_item", "b"),
                ("list_item", "c"),
                ("table", "k\tv\n1\t2"),
                ("code", "  x\n   y"),
            ]
        );
        assert_eq!(parsed.blocks[4].level, Some(1));
    }
}
