//! CSV and TSV: delimiter sniffing, RFC 4180 quoting, BOM and encoding
//! handling via [`crate::text::decode`].

use crate::text;
use crate::{Block, DocumentIdentity, Format, FormatsResult, Section};

/// Candidate delimiters, in tie-break order.
const CANDIDATES: [char; 4] = [',', '\t', ';', '|'];

/// Pick the delimiter whose count per line is most often the same non-zero
/// number over the first lines (quotes respected). Ties go to the earlier
/// candidate. `None` when no candidate appears at all.
pub fn sniff_delimiter(sample: &str) -> Option<char> {
    let lines: Vec<&str> = sample
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(50)
        .collect();
    if lines.is_empty() {
        return None;
    }
    let mut best: Option<(char, usize, usize)> = None;
    for candidate in CANDIDATES {
        let counts: Vec<usize> = lines
            .iter()
            .map(|line| count_outside_quotes(line, candidate))
            .collect();
        let Some(&mode) = counts
            .iter()
            .filter(|c| **c > 0)
            .max_by_key(|c| counts.iter().filter(|x| x == c).count())
        else {
            continue;
        };
        let agreeing = counts.iter().filter(|c| **c == mode).count();
        let total: usize = counts.iter().sum();
        let better = match best {
            None => true,
            Some((_, best_agreeing, best_total)) => {
                agreeing > best_agreeing || (agreeing == best_agreeing && total > best_total)
            }
        };
        if better {
            best = Some((candidate, agreeing, total));
        }
    }
    best.map(|(c, _, _)| c)
}

fn count_outside_quotes(line: &str, delimiter: char) -> usize {
    let mut in_quotes = false;
    let mut count = 0;
    for c in line.chars() {
        if c == '"' {
            in_quotes = !in_quotes;
        } else if c == delimiter && !in_quotes {
            count += 1;
        }
    }
    count
}

/// Parse delimited text: quoted fields may contain the delimiter, doubled
/// quotes and line breaks; records end at `\n`, `\r\n` or `\r`.
pub fn parse(text: &str, delimiter: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut field_started = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if !field_started || field.is_empty() => {
                in_quotes = true;
                field_started = true;
            }
            c if c == delimiter => {
                row.push(std::mem::take(&mut field));
                field_started = false;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                field_started = false;
            }
            other => {
                field.push(other);
                field_started = true;
            }
        }
    }
    if field_started || !row.is_empty() || !field.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// Extract a `.csv`/`.tsv` file. TSV always splits on tabs; CSV sniffs.
pub fn extract(bytes: &[u8], identity: DocumentIdentity, format: Format) -> FormatsResult {
    let decoded = text::decode(bytes);
    let mut result = FormatsResult::empty(format, identity);
    result
        .metadata
        .insert("encoding".to_string(), decoded.encoding.to_string());
    if decoded.encoding == "windows-1252" {
        result.warn("input is not valid UTF-8; decoded as windows-1252");
    }
    if decoded.lossy {
        result.warn("partial: malformed byte sequences were replaced with U+FFFD");
    }
    let delimiter = if format == Format::Tsv {
        '\t'
    } else if let Some(d) = sniff_delimiter(&decoded.text) {
        d
    } else {
        result.warn("no delimiter found; each line is one field");
        ','
    };
    let label = match delimiter {
        '\t' => "tab".to_string(),
        other => other.to_string(),
    };
    result.metadata.insert("delimiter".to_string(), label);
    let rows = parse(&decoded.text, delimiter);
    let widths: Vec<usize> = rows.iter().map(Vec::len).collect();
    if let (Some(min), Some(max)) = (widths.iter().min(), widths.iter().max()) {
        if min != max {
            result.warn(format!(
                "ragged rows: between {min} and {max} fields per row"
            ));
        }
        result
            .metadata
            .insert("columns".to_string(), max.to_string());
    }
    result
        .metadata
        .insert("rows".to_string(), rows.len().to_string());
    let mut section = Section::new("table", 0, None);
    section.blocks.push(Block::table(rows));
    result.sections.push(section);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_semicolons_and_tabs() {
        assert_eq!(sniff_delimiter("a;b;c\n1;2;3\n"), Some(';'));
        assert_eq!(sniff_delimiter("a\tb\n1\t2\n"), Some('\t'));
        assert_eq!(sniff_delimiter("a,b\n\"x;y\",2\n"), Some(','));
        assert_eq!(sniff_delimiter("just text\n"), None);
    }

    #[test]
    fn parses_quotes_and_newlines() {
        let rows = parse("a,\"b \"\"q\"\" c\",\"multi\nline\"\r\n1,2,3\n", ',');
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec!["a", "b \"q\" c", "multi\nline"]);
        assert_eq!(rows[1], vec!["1", "2", "3"]);
    }

    #[test]
    fn keeps_empty_fields() {
        assert_eq!(parse("a,,c\n", ','), vec![vec!["a", "", "c"]]);
        assert_eq!(parse("a,b,\n", ','), vec![vec!["a", "b", ""]]);
    }
}
