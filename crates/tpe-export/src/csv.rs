//! Zotero CSV: the column set and quoting of Zotero's CSV export
//! translator (`CSV.js`, translator id `25f4c5e2-d790-4daa-a667-797619c7e2f2`).
//!
//! Every field is wrapped in double quotes with embedded quotes doubled
//! (RFC 4180 quoting), runs of CR/LF inside a value become one space,
//! records are separated by `\n` with no trailing newline, multi-value
//! fields join with `; `, and the file starts with a UTF-8 byte-order mark,
//! all as the translator does.

use crate::model::{Export, Item};

/// `exportedFields` of the translator, in export order.
pub const EXPORTED_FIELDS: &[&str] = &[
    "key",
    "itemType",
    "publicationYear",
    "creators/author",
    "title",
    "publicationTitle",
    "ISBN",
    "ISSN",
    "DOI",
    "url",
    "abstractNote",
    "date",
    "dateAdded",
    "dateModified",
    "accessDate",
    "pages",
    "numPages",
    "issue",
    "volume",
    "numberOfVolumes",
    "journalAbbreviation",
    "shortTitle",
    "series",
    "seriesNumber",
    "seriesText",
    "seriesTitle",
    "publisher",
    "place",
    "language",
    "rights",
    "type",
    "archive",
    "archiveLocation",
    "libraryCatalog",
    "callNumber",
    "extra",
    "notes",
    "attachments/path",
    "attachments/url",
    "tags/own",
    "tags/automatic",
    "creators/editor",
    "creators/seriesEditor",
    "creators/translator",
    "creators/contributor",
    "creators/attorneyAgent",
    "creators/bookAuthor",
    "creators/castMember",
    "creators/commenter",
    "creators/composer",
    "creators/cosponsor",
    "creators/counsel",
    "creators/interviewer",
    "creators/producer",
    "creators/recipient",
    "creators/reviewedAuthor",
    "creators/scriptwriter",
    "creators/wordsBy",
    "creators/guest",
    "number",
    "edition",
    "runningTime",
    "scale",
    "medium",
    "artworkSize",
    "filingDate",
    "applicationNumber",
    "assignee",
    "issuingAuthority",
    "country",
    "meetingName",
    "conferenceName",
    "court",
    "references",
    "reporter",
    "legalStatus",
    "priorityNumbers",
    "programmingLanguage",
    "version",
    "system",
    "code",
    "codeNumber",
    "section",
    "session",
    "committee",
    "history",
    "legislativeBody",
];

const VALUE_SEPARATOR: &str = "; ";

/// The header label of one exported field (`writeColumnHeaders`).
pub fn header_label(field: &str) -> String {
    let mut parts = field.split('/');
    let head = parts.next().unwrap_or(field);
    let tail = parts.next().unwrap_or("");
    let label = match head {
        "creators" => tail,
        "tags" => {
            if tail == "own" {
                "Manual Tags"
            } else {
                "Automatic Tags"
            }
        }
        "attachments" => {
            if tail == "url" {
                "Link Attachments"
            } else {
                "File Attachments"
            }
        }
        other => other,
    };
    let mut chars = label.chars();
    let mut capitalised = String::new();
    if let Some(first) = chars.next() {
        capitalised.extend(first.to_uppercase());
    }
    capitalised.extend(chars);
    let mut spaced = String::with_capacity(capitalised.len() + 4);
    let mut previous_lower = false;
    for c in capitalised.chars() {
        if previous_lower && c.is_ascii_uppercase() {
            spaced.push(' ');
        }
        spaced.push(c);
        previous_lower = c.is_ascii_lowercase();
    }
    spaced
}

/// All column headers in order.
pub fn headers() -> Vec<String> {
    EXPORTED_FIELDS.iter().map(|f| header_label(f)).collect()
}

/// `escapeValue`: newline runs become one space, quotes are doubled.
pub fn escape_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut in_newline = false;
    for c in value.chars() {
        match c {
            '\r' | '\n' => {
                if !in_newline {
                    out.push(' ');
                }
                in_newline = true;
            }
            '"' => {
                in_newline = false;
                out.push_str("\"\"");
            }
            _ => {
                in_newline = false;
                out.push(c);
            }
        }
    }
    out
}

/// The value of one field for one item (`getValue`), unquoted.
pub fn value(item: &Item, field: &str) -> String {
    match field {
        "key" => item.key.clone(),
        "itemType" => item.item_type.zotero_name().to_string(),
        "publicationYear" => item.year().map(|y| y.to_string()).unwrap_or_default(),
        "creators/author" => item
            .creators
            .iter()
            .map(|c| escape_value(&c.display()))
            .collect::<Vec<_>>()
            .join(VALUE_SEPARATOR),
        "date" => item.date.clone().unwrap_or_default(),
        "tags/own" => item
            .tags
            .iter()
            .map(|t| escape_value(t))
            .collect::<Vec<_>>()
            .join(VALUE_SEPARATOR),
        _ => plain(item, field).map(escape_value).unwrap_or_default(),
    }
}

fn plain<'a>(item: &'a Item, field: &str) -> Option<&'a str> {
    let value = match field {
        "title" => &item.title,
        "publicationTitle" => &item.publication_title,
        "ISBN" => &item.isbn,
        "ISSN" => &item.issn,
        "DOI" => &item.doi,
        "url" => &item.url,
        "abstractNote" => &item.abstract_note,
        "pages" => &item.pages,
        "issue" => &item.issue,
        "volume" => &item.volume,
        "publisher" => &item.publisher,
        "type" => &item.type_field,
        "extra" => &item.extra,
        "number" => &item.number,
        _ => &None,
    };
    value.as_deref()
}

/// Renders `export` as Zotero CSV.
pub fn render(export: &Export) -> String {
    let mut out = String::from("\u{feff}");
    let header = headers()
        .iter()
        .map(|label| format!("\"{}\"", escape_value(label)))
        .collect::<Vec<_>>()
        .join(",");
    out.push_str(&header);
    for item in &export.items {
        out.push('\n');
        let record = EXPORTED_FIELDS
            .iter()
            .map(|field| format!("\"{}\"", value(item, field)))
            .collect::<Vec<_>>()
            .join(",");
        out.push_str(&record);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_labels_follow_the_translator() {
        assert_eq!(header_label("key"), "Key");
        assert_eq!(header_label("itemType"), "Item Type");
        assert_eq!(header_label("publicationYear"), "Publication Year");
        assert_eq!(header_label("creators/author"), "Author");
        assert_eq!(header_label("ISBN"), "ISBN");
        assert_eq!(header_label("DOI"), "DOI");
        assert_eq!(header_label("url"), "Url");
        assert_eq!(header_label("abstractNote"), "Abstract Note");
        assert_eq!(header_label("numberOfVolumes"), "Number Of Volumes");
        assert_eq!(header_label("tags/own"), "Manual Tags");
        assert_eq!(header_label("tags/automatic"), "Automatic Tags");
        assert_eq!(header_label("attachments/path"), "File Attachments");
        assert_eq!(header_label("attachments/url"), "Link Attachments");
        assert_eq!(header_label("creators/seriesEditor"), "Series Editor");
        assert_eq!(header_label("creators/wordsBy"), "Words By");
        assert_eq!(header_label("legislativeBody"), "Legislative Body");
        assert_eq!(EXPORTED_FIELDS.len(), 87);
        assert_eq!(headers().len(), EXPORTED_FIELDS.len());
    }

    #[test]
    fn values_are_quoted_per_rfc_4180_and_newlines_become_spaces() {
        assert_eq!(escape_value("say \"hi\""), "say \"\"hi\"\"");
        assert_eq!(escape_value("a\r\n\nb"), "a b");
        assert_eq!(escape_value("plain, with comma"), "plain, with comma");
    }
}
