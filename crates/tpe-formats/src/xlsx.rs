//! Excel (OOXML) workbooks: sheets in workbook order, shared and inline
//! strings, cached formula results, and number formats rendered as Excel
//! displays them where the format is a plain number, percentage, scientific
//! or date/time pattern. Formats this crate does not render keep the raw
//! value and say so in a warning.

use std::collections::{BTreeMap, BTreeSet};

use roxmltree::Node;

use crate::ooxml::{self, Package};
use crate::xml;
use crate::{Block, DocumentIdentity, Format, FormatsError, FormatsResult, Section};

/// Extract a `.xlsx` held in `bytes`.
pub fn extract(bytes: &[u8], identity: DocumentIdentity) -> Result<FormatsResult, FormatsError> {
    let mut package = Package::open(bytes)?;
    let workbook = package.require_text("xl/workbook.xml")?;
    let mut result = FormatsResult::empty(Format::Xlsx, identity);
    result.metadata = ooxml::document_properties(&mut package)?;
    let shared = shared_strings(&mut package)?;
    let styles = Styles::load(&mut package)?;

    let doc = xml::parse("xl/workbook.xml", &workbook)?;
    let date1904 = xml::descendant(doc.root_element(), "workbookPr")
        .and_then(|p| xml::attr(p, "date1904"))
        .is_some_and(|v| v == "1" || v == "true");
    let rels = package.relationships("xl/workbook.xml")?;
    let mut sheets: Vec<(String, String, bool)> = Vec::new();
    if let Some(list) = xml::descendant(doc.root_element(), "sheets") {
        for sheet in list.children().filter(|n| xml::is(*n, "sheet")) {
            let name = xml::attr(sheet, "name").unwrap_or("").to_string();
            let hidden = matches!(xml::attr(sheet, "state"), Some("hidden" | "veryHidden"));
            match xml::rel_attr(sheet, "id").and_then(|rid| rels.get(rid)) {
                Some(target) if package.has(target) => sheets.push((name, target.clone(), hidden)),
                _ => result.warn(format!("partial: sheet '{name}' has no worksheet part")),
            }
        }
    }
    if sheets.is_empty() {
        result.warn("partial: no sheets found");
    }
    let mut unrendered: BTreeSet<String> = BTreeSet::new();
    let mut budget = GridBudget::default();
    for (index, (name, part, hidden)) in sheets.iter().enumerate() {
        let source = package.require_text(part)?;
        let doc = xml::parse(part, &source)?;
        let mut section = Section::new("sheet", index as u32, Some(name.clone()));
        if *hidden {
            result.warn(format!("sheet '{name}' is hidden"));
        }
        let mut sheet = SheetReader {
            shared: &shared,
            styles: &styles,
            date1904,
            unrendered: &mut unrendered,
            warnings: Vec::new(),
        };
        let rows = sheet.rows(doc.root_element(), name, &mut budget)?;
        let count = rows.len();
        result
            .metadata
            .insert(format!("sheet.{}.rows", index + 1), count.to_string());
        for warning in sheet.warnings {
            result.warn(warning);
        }
        section.blocks.push(Block::table(rows));
        result.sections.push(section);
    }
    for code in unrendered {
        result.warn(format!(
            "number format {code:?} not rendered; raw values kept"
        ));
    }
    result.title = result.metadata.get("title").cloned();
    Ok(result)
}

/// `xl/sharedStrings.xml` entries, rich runs concatenated, phonetic runs dropped.
fn shared_strings(package: &mut Package) -> Result<Vec<String>, FormatsError> {
    let Some(source) = package.read_text("xl/sharedStrings.xml")? else {
        return Ok(Vec::new());
    };
    let doc = xml::parse("xl/sharedStrings.xml", &source)?;
    Ok(doc
        .root_element()
        .children()
        .filter(|n| xml::is(*n, "si"))
        .map(string_item)
        .collect())
}

/// Text of an `si` or `is` element: every `t` not inside `rPh`.
fn string_item(node: Node<'_, '_>) -> String {
    let mut out = String::new();
    for t in node.descendants().filter(|n| xml::is(*n, "t")) {
        if t.ancestors().any(|a| xml::is(a, "rPh")) {
            continue;
        }
        out.push_str(t.text().unwrap_or(""));
    }
    out
}

/// Cell style index -> number format code.
struct Styles {
    /// Format code per `cellXfs` index.
    xf_codes: Vec<String>,
}

impl Styles {
    fn load(package: &mut Package) -> Result<Self, FormatsError> {
        let mut custom: BTreeMap<u32, String> = BTreeMap::new();
        let mut xf_codes = Vec::new();
        let Some(source) = package.read_text("xl/styles.xml")? else {
            return Ok(Self { xf_codes });
        };
        let doc = xml::parse("xl/styles.xml", &source)?;
        let root = doc.root_element();
        if let Some(fmts) = xml::child(root, "numFmts") {
            for fmt in fmts.children().filter(|n| xml::is(*n, "numFmt")) {
                if let (Some(id), Some(code)) = (
                    xml::attr(fmt, "numFmtId").and_then(|v| v.parse().ok()),
                    xml::attr(fmt, "formatCode"),
                ) {
                    custom.insert(id, code.to_string());
                }
            }
        }
        if let Some(xfs) = xml::child(root, "cellXfs") {
            for xf in xfs.children().filter(|n| xml::is(*n, "xf")) {
                let id: u32 = xml::attr(xf, "numFmtId")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let code = custom
                    .get(&id)
                    .cloned()
                    .or_else(|| builtin_format(id).map(str::to_string))
                    .unwrap_or_else(|| format!("builtin:{id}"));
                xf_codes.push(code);
            }
        }
        Ok(Self { xf_codes })
    }

    fn code(&self, style: usize) -> &str {
        self.xf_codes.get(style).map_or("General", String::as_str)
    }
}

/// Built-in number formats (ECMA-376 part 1, 18.8.30), en-US spellings.
pub fn builtin_format(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "mm-dd-yy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yy h:mm",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mmss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

struct SheetReader<'a> {
    shared: &'a [String],
    styles: &'a Styles,
    date1904: bool,
    unrendered: &'a mut BTreeSet<String>,
    warnings: Vec<String>,
}

// Dense output has a cost even when almost every coordinate is empty. These
// implementation limits apply to the entire workbook, without refunding work
// for duplicate/trimmed rows or cells. They do not bound ZIP/XML/string memory.
const MAX_ROW: usize = 1_048_576;
const MAX_COLUMN: usize = 16_384;
const MAX_ROW_SLOTS: usize = 100_000;
const MAX_CELL_SLOTS: usize = 1_000_000;

#[derive(Default)]
struct GridBudget {
    rows: usize,
    cells: usize,
}

impl GridBudget {
    fn charge(used: &mut usize, amount: usize, limit: usize) -> Result<(), FormatsError> {
        *used = used
            .checked_add(amount.max(1))
            .filter(|total| *total <= limit)
            .ok_or_else(|| {
                FormatsError::Invalid("XLSX dense grid resource limit exceeded".into())
            })?;
        Ok(())
    }
}

fn row_number(value: &str) -> Option<usize> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value
        .parse::<usize>()
        .ok()
        .filter(|number| (1..=MAX_ROW).contains(number))
}

fn coordinate(reference: &str) -> Option<(usize, usize)> {
    let split = reference
        .bytes()
        .position(|byte| !byte.is_ascii_alphabetic())?;
    if split == 0 {
        return None;
    }
    let mut column = 0usize;
    for byte in reference[..split].bytes() {
        column = column
            .checked_mul(26)?
            .checked_add(usize::from(byte.to_ascii_uppercase() - b'A') + 1)?;
        if column > MAX_COLUMN {
            return None;
        }
    }
    Some((column.checked_sub(1)?, row_number(&reference[split..])?))
}

impl SheetReader<'_> {
    /// Rows of `sheetData`, placed at their row numbers (gaps are empty rows).
    fn rows(
        &mut self,
        root: Node<'_, '_>,
        sheet_name: &str,
        budget: &mut GridBudget,
    ) -> Result<Vec<Vec<String>>, FormatsError> {
        let mut rows: Vec<Vec<String>> = Vec::new();
        let Some(data) = xml::descendant(root, "sheetData") else {
            return Ok(rows);
        };
        for row in data.children().filter(|n| xml::is(*n, "row")) {
            let number = match xml::attr(row, "r") {
                Some(value) => row_number(value),
                None => rows
                    .len()
                    .checked_add(1)
                    .filter(|number| *number <= MAX_ROW),
            }
            .ok_or_else(|| {
                FormatsError::Invalid(format!("XLSX sheet {sheet_name:?}: invalid row coordinate"))
            })?;
            let additional = number.saturating_sub(rows.len());
            GridBudget::charge(&mut budget.rows, additional, MAX_ROW_SLOTS)?;
            rows.try_reserve_exact(additional)
                .map_err(|_| FormatsError::Invalid("XLSX row allocation failed".into()))?;
            if additional > 0 {
                rows.resize_with(number, Vec::new);
            }
            let mut cells: Vec<String> = Vec::new();
            for cell in row.children().filter(|n| xml::is(*n, "c")) {
                let reference = xml::attr(cell, "r");
                let column = match reference {
                    Some(value) => coordinate(value)
                        .filter(|(_, row)| *row == number)
                        .map(|(column, _)| column),
                    None => Some(cells.len()).filter(|column| *column < MAX_COLUMN),
                }
                .ok_or_else(|| {
                    FormatsError::Invalid(format!(
                        "XLSX sheet {sheet_name:?}: invalid cell coordinate"
                    ))
                })?;
                let width = column
                    .checked_add(1)
                    .ok_or_else(|| FormatsError::Invalid("XLSX column overflow".into()))?;
                let additional = width.saturating_sub(cells.len());
                GridBudget::charge(&mut budget.cells, additional, MAX_CELL_SLOTS)?;
                cells
                    .try_reserve_exact(additional)
                    .map_err(|_| FormatsError::Invalid("XLSX cell allocation failed".into()))?;
                if additional > 0 {
                    cells.resize_with(width, String::new);
                }
                let reference = reference.unwrap_or("");
                let value = self.cell_value(cell, sheet_name, reference);
                cells[column] = value;
            }
            while cells.last().is_some_and(String::is_empty) {
                cells.pop();
            }
            rows[number - 1] = cells;
        }
        while rows.last().is_some_and(Vec::is_empty) {
            rows.pop();
        }
        Ok(rows)
    }

    fn cell_value(&mut self, cell: Node<'_, '_>, sheet: &str, reference: &str) -> String {
        let kind = xml::attr(cell, "t").unwrap_or("n");
        let raw = xml::child(cell, "v")
            .and_then(|v| v.text())
            .unwrap_or("")
            .to_string();
        if kind == "inlineStr" {
            return xml::child(cell, "is").map(string_item).unwrap_or_default();
        }
        if xml::child(cell, "f").is_some() && raw.is_empty() && kind != "str" {
            self.warnings.push(format!(
                "partial: {sheet}!{reference}: formula without a cached value"
            ));
            return String::new();
        }
        match kind {
            "s" => {
                if let Some(text) = raw.parse::<usize>().ok().and_then(|i| self.shared.get(i)) {
                    text.clone()
                } else {
                    self.warnings.push(format!(
                        "partial: {sheet}!{reference}: shared string index {raw} out of range"
                    ));
                    String::new()
                }
            }
            "str" | "e" => raw,
            "b" => {
                if raw == "1" {
                    "TRUE".to_string()
                } else {
                    "FALSE".to_string()
                }
            }
            _ => {
                let style: usize = xml::attr(cell, "s")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let code = self.styles.code(style);
                if let Some(text) = render_number(&raw, code, self.date1904) {
                    text
                } else {
                    self.unrendered.insert(code.to_string());
                    raw
                }
            }
        }
    }
}

/// 0-based column of a cell reference (`A1` -> 0, `AB12` -> 27).
pub fn column_index(reference: &str) -> Option<usize> {
    coordinate(reference).map(|(column, _)| column)
}

/// What a format code asks for, from its first section with quoted text,
/// colours and locale tags removed.
#[derive(Debug, PartialEq, Eq)]
enum FormatKind {
    General,
    Number {
        decimals: usize,
        grouping: bool,
        percent: bool,
        scientific: bool,
    },
    Date {
        date: bool,
        time: bool,
    },
    Text,
    Unknown,
}

fn classify_format(code: &str) -> FormatKind {
    if code == "General" {
        return FormatKind::General;
    }
    let mut section = String::new();
    let mut in_quote = false;
    let mut in_bracket = false;
    let mut chars = code.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => in_quote = !in_quote,
            _ if in_quote => {}
            '[' => in_bracket = true,
            ']' => in_bracket = false,
            _ if in_bracket => {}
            ';' => break,
            '\\' | '_' | '*' => {
                chars.next();
            }
            _ => section.push(c),
        }
    }
    if section == "@" {
        return FormatKind::Text;
    }
    let lower = section.to_ascii_lowercase();
    let has_time = lower.contains('h') || lower.contains('s');
    let has_date = lower.contains('y') || lower.contains('d') || (lower.contains('m') && !has_time);
    let has_month_in_time = lower.contains('m') && has_time;
    if has_date || has_time {
        if lower.contains('0') && !lower.contains('.') && !lower.contains("ss.") {
            return FormatKind::Unknown;
        }
        return FormatKind::Date {
            date: has_date,
            time: has_time || has_month_in_time && !has_date,
        };
    }
    if lower.contains('?') || lower.contains('/') {
        return FormatKind::Unknown;
    }
    if !lower.contains('0') && !lower.contains('#') {
        return FormatKind::Unknown;
    }
    let decimals = lower.find('.').map_or(0, |dot| {
        lower[dot + 1..].chars().take_while(|c| *c == '0').count()
    });
    let grouping = lower.contains(',');
    let percent = lower.contains('%');
    let scientific = lower.contains('e');
    FormatKind::Number {
        decimals,
        grouping,
        percent,
        scientific,
    }
}

/// Render the stored numeric text `raw` as `code` would display it. `None`
/// when the format is not one this crate renders.
pub fn render_number(raw: &str, code: &str, date1904: bool) -> Option<String> {
    if raw.is_empty() {
        return Some(String::new());
    }
    let value: f64 = raw.parse().ok()?;
    match classify_format(code) {
        FormatKind::General => Some(general(value)),
        FormatKind::Text => Some(raw.to_string()),
        FormatKind::Number {
            decimals,
            grouping,
            percent,
            scientific,
        } => {
            let shown = if percent { value * 100.0 } else { value };
            if scientific {
                return Some(format_scientific(shown, decimals));
            }
            let mut text = format!("{shown:.decimals$}");
            if grouping {
                text = group_thousands(&text);
            }
            if percent {
                text.push('%');
            }
            Some(text)
        }
        FormatKind::Date { date, time } => Some(format_date_time(value, date, time, date1904)),
        FormatKind::Unknown => None,
    }
}

/// Excel's `General`: integers without a decimal point, otherwise up to ten
/// decimals with trailing zeros dropped.
fn general(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return format!("{value:.0}");
    }
    if value.abs() >= 1e11 || (value != 0.0 && value.abs() < 1e-9) {
        return format_scientific(value, 5);
    }
    let text = format!("{value:.10}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    trimmed.to_string()
}

fn format_scientific(value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$e}");
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let exp: i32 = exponent.parse().unwrap_or(0);
    let sign = if exp < 0 { '-' } else { '+' };
    format!("{mantissa}E{sign}{:02}", exp.abs())
}

fn group_thousands(text: &str) -> String {
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", text),
    };
    let (int_part, frac) = match rest.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (rest, None),
    };
    let mut grouped = String::new();
    for (i, c) in int_part.chars().enumerate() {
        if i > 0 && (int_part.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

/// Serial date/time to ISO text. 1900 system: day 1 is 1900-01-01 and the
/// nonexistent 1900-02-29 (serial 60) is kept as Excel shows it, so serials
/// before 61 use 1899-12-31 as epoch and later ones 1899-12-30.
pub fn format_date_time(serial: f64, date: bool, time: bool, date1904: bool) -> String {
    let days = serial.floor() as i64;
    let mut seconds = ((serial - serial.floor()) * 86_400.0).round() as i64;
    let mut days = days;
    if seconds >= 86_400 {
        seconds -= 86_400;
        days += 1;
    }
    let epoch_days = if date1904 {
        days_from_civil(1904, 1, 1)
    } else if days < 61 {
        days_from_civil(1899, 12, 31)
    } else {
        days_from_civil(1899, 12, 30)
    };
    let (y, m, d) = civil_from_days(epoch_days + days);
    let date_text = format!("{y:04}-{m:02}-{d:02}");
    let time_text = format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    );
    match (date, time) {
        (true, true) => format!("{date_text} {time_text}"),
        (false, true) => time_text,
        _ => date_text,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns() {
        assert_eq!(column_index("A1"), Some(0));
        assert_eq!(column_index("Z9"), Some(25));
        assert_eq!(column_index("AA1"), Some(26));
        assert_eq!(column_index("AB12"), Some(27));
        assert_eq!(column_index("12"), None);
    }

    #[test]
    fn numbers() {
        assert_eq!(render_number("3", "General", false).unwrap(), "3");
        assert_eq!(render_number("3.5", "General", false).unwrap(), "3.5");
        assert_eq!(
            render_number("0.30000000000000004", "General", false).unwrap(),
            "0.3"
        );
        assert_eq!(
            render_number("1234.5", "#,##0.00", false).unwrap(),
            "1,234.50"
        );
        assert_eq!(
            render_number("-1234567", "#,##0", false).unwrap(),
            "-1,234,567"
        );
        assert_eq!(render_number("0.256", "0.0%", false).unwrap(), "25.6%");
        assert_eq!(
            render_number("12345", "0.00E+00", false).unwrap(),
            "1.23E+04"
        );
        assert_eq!(render_number("12", "@", false).unwrap(), "12");
        assert_eq!(render_number("0.5", "# ?/?", false), None);
        assert_eq!(render_number("7", "\"Qty: \"0", false).unwrap(), "7");
    }

    #[test]
    fn dates() {
        assert_eq!(render_number("1", "mm-dd-yy", false).unwrap(), "1900-01-01");
        assert_eq!(
            render_number("59", "yyyy-mm-dd", false).unwrap(),
            "1900-02-28"
        );
        assert_eq!(
            render_number("61", "yyyy-mm-dd", false).unwrap(),
            "1900-03-01"
        );
        assert_eq!(
            render_number("45658", "yyyy-mm-dd", false).unwrap(),
            "2025-01-01"
        );
        assert_eq!(
            render_number("45658.5", "m/d/yy h:mm", false).unwrap(),
            "2025-01-01 12:00:00"
        );
        assert_eq!(render_number("0.75", "h:mm", false).unwrap(), "18:00:00");
        assert_eq!(
            render_number("0", "yyyy-mm-dd", true).unwrap(),
            "1904-01-01"
        );
        assert_eq!(
            render_number("45658", "[$-409]d-mmm-yy;@", false).unwrap(),
            "2025-01-01"
        );
    }
}
