//! Bibliographic records as `doi.org` (CSL-JSON) and Crossref (`/works`)
//! return them, reduced to the fields the scorer compares.
//!
//! Both shapes share the CSL vocabulary: `title`, `author` (`family`,
//! `given` or `literal`/`name`), `issued`/`published-print`/`published-online`
//! with `date-parts`, `container-title`, `volume`, `issue`, `page`, `DOI`,
//! `type`. Crossref wraps every string that CSL stores plain in a one-element
//! array (`"title": ["..."]`), so every string field is read either way.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A record as one registry returned it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// Lower-case DOI without a resolver prefix.
    pub doi: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    /// Authors in record order as `Given Family` (or the literal name).
    pub authors: Vec<String>,
    /// Family names in record order; empty strings for literal-only names.
    pub families: Vec<String>,
    /// Distinct years from `issued`, `published-print`, `published-online`
    /// and `published`, in that order. A print year can differ from the
    /// online-first year; the scorer accepts either.
    pub years: Vec<u16>,
    pub container: Option<String>,
    pub container_short: Option<String>,
    pub volume: Option<String>,
    pub issue: Option<String>,
    pub pages: Option<String>,
    pub article_number: Option<String>,
    /// CSL item type (`article-journal`, `paper-conference`, `posted-content`...).
    pub kind: Option<String>,
    /// `doi.org` or `crossref`.
    pub source: String,
}

/// A response that was not the documented shape.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// The body was not JSON.
    #[error("response is not JSON: {0}")]
    Json(String),
    /// The JSON lacked the expected envelope.
    #[error("unexpected response shape: {0}")]
    Shape(String),
}

/// `value[key]` as a string: a plain string, or the first string of an array.
fn string_field(value: &Value, key: &str) -> Option<String> {
    match value.get(key)? {
        Value::String(s) => non_empty(s),
        Value::Array(items) => items.iter().find_map(|v| v.as_str().and_then(non_empty)),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn non_empty(s: &str) -> Option<String> {
    let trimmed = s.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// The year of a CSL date (`{"date-parts": [[y, m, d]]}`).
fn date_year(value: &Value) -> Option<u16> {
    let first = value.get("date-parts")?.get(0)?.get(0)?;
    match first {
        Value::Number(n) => n.as_u64().and_then(|y| u16::try_from(y).ok()),
        Value::String(s) => s.get(..4)?.parse().ok(),
        _ => None,
    }
}

/// Lower-case the DOI and strip a resolver prefix; `None` unless it has the
/// `10.<prefix>/<suffix>` shape.
pub fn normalize_doi(raw: &str) -> Option<String> {
    let mut s = raw.trim();
    for prefix in [
        "https://doi.org/",
        "http://doi.org/",
        "https://dx.doi.org/",
        "http://dx.doi.org/",
        "doi.org/",
        "doi:",
        "DOI:",
        "Doi:",
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim();
            break;
        }
    }
    // Require a whole DOI, then reuse the resolver's balanced delimiter rule.
    // In particular, a closing parenthesis inside a balanced suffix is data.
    let (prefix, suffix) = s.split_once('/')?;
    let digits = prefix.strip_prefix("10.")?;
    if !(4..=9).contains(&digits.len())
        || !digits.bytes().all(|b| b.is_ascii_digit())
        || suffix.is_empty()
        || s.chars().any(char::is_whitespace)
    {
        return None;
    }
    tpe::resolve::doi_in(s)
}

/// One record from a CSL-JSON item or a Crossref `message` work object.
pub fn from_csl(item: &Value, source: &str) -> Record {
    let mut authors = Vec::new();
    let mut families = Vec::new();
    if let Some(list) = item.get("author").and_then(Value::as_array) {
        // Crossref marks the first author with `sequence: first`; put it first.
        let mut ordered: Vec<&Value> = list.iter().collect();
        if let Some(pos) = ordered
            .iter()
            .position(|a| a.get("sequence").and_then(Value::as_str) == Some("first"))
        {
            let first = ordered.remove(pos);
            ordered.insert(0, first);
        }
        for a in ordered {
            let family = string_field(a, "family");
            let given = string_field(a, "given");
            let literal = string_field(a, "literal").or_else(|| string_field(a, "name"));
            let display = match (&given, &family, &literal) {
                (Some(g), Some(f), _) => format!("{g} {f}"),
                (None, Some(f), _) => f.clone(),
                (_, None, Some(l)) => l.clone(),
                (Some(g), None, None) => g.clone(),
                (None, None, None) => continue,
            };
            authors.push(display);
            families.push(family.or(literal).unwrap_or_default());
        }
    }
    let mut years = Vec::new();
    for key in ["issued", "published-print", "published-online", "published"] {
        if let Some(y) = item.get(key).and_then(date_year)
            && !years.contains(&y)
        {
            years.push(y);
        }
    }
    Record {
        doi: string_field(item, "DOI")
            .or_else(|| string_field(item, "doi"))
            .and_then(|d| normalize_doi(&d)),
        title: string_field(item, "title"),
        subtitle: string_field(item, "subtitle"),
        authors,
        families,
        years,
        container: string_field(item, "container-title"),
        container_short: string_field(item, "container-title-short")
            .or_else(|| string_field(item, "short-container-title")),
        volume: string_field(item, "volume"),
        issue: string_field(item, "issue"),
        pages: string_field(item, "page"),
        article_number: string_field(item, "article-number"),
        kind: string_field(item, "type"),
        source: source.to_string(),
    }
}

fn registry_record(item: &Value, source: &str) -> Result<Record, ParseError> {
    // Unknown fields are harmless, but identity fields must have CSL shapes.
    for key in ["DOI", "doi", "title", "subtitle"] {
        if let Some(value) = item.get(key) {
            let valid = match value {
                Value::Null | Value::String(_) => true,
                Value::Array(values) if matches!(key, "title" | "subtitle") => {
                    values.iter().all(Value::is_string)
                }
                _ => false,
            };
            if !valid {
                return Err(ParseError::Shape(format!("invalid registry {key} field")));
            }
        }
    }
    if let Some(authors) = item.get("author")
        && !authors.is_null()
        && !authors
            .as_array()
            .is_some_and(|list| list.iter().all(Value::is_object))
    {
        return Err(ParseError::Shape(
            "invalid registry author field".to_string(),
        ));
    }
    let record = from_csl(item, source);
    if !item.is_object() || record.doi.is_none() {
        return Err(ParseError::Shape(
            "registry item lacks a valid DOI".to_string(),
        ));
    }
    Ok(record)
}

/// The record in a `doi.org` content-negotiation body
/// (`Accept: application/vnd.citationstyles.csl+json`).
///
/// # Errors
/// When the body is not a JSON object with a valid DOI.
pub fn parse_csl_json(body: &str) -> Result<Record, ParseError> {
    let value: Value = serde_json::from_str(body).map_err(|e| ParseError::Json(e.to_string()))?;
    if !value.is_object() {
        return Err(ParseError::Shape(
            "CSL-JSON item is not an object".to_string(),
        ));
    }
    registry_record(&value, "doi.org")
}

fn crossref_message(body: &str) -> Result<Value, ParseError> {
    let value: Value = serde_json::from_str(body).map_err(|e| ParseError::Json(e.to_string()))?;
    let status = value.get("status").and_then(Value::as_str).unwrap_or("");
    if status != "ok" {
        return Err(ParseError::Shape(format!("Crossref status {status:?}")));
    }
    value
        .get("message")
        .cloned()
        .ok_or_else(|| ParseError::Shape("Crossref response without message".to_string()))
}

/// The record in a Crossref `GET /works/{doi}` body.
///
/// # Errors
/// When the body is not the `{"status":"ok","message-type":"work",...}` envelope.
pub fn parse_crossref_work(body: &str) -> Result<Record, ParseError> {
    let message = crossref_message(body)?;
    if !message.is_object() {
        return Err(ParseError::Shape(
            "Crossref work message is not an object".to_string(),
        ));
    }
    registry_record(&message, "crossref")
}

/// The records in a Crossref `GET /works?query.bibliographic=...` body.
///
/// # Errors
/// When the body is not the `work-list` envelope with `message.items`.
pub fn parse_crossref_works(body: &str) -> Result<Vec<Record>, ParseError> {
    let message = crossref_message(body)?;
    let items = message
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| ParseError::Shape("Crossref work-list without items".to_string()))?;
    items
        .iter()
        .map(|item| registry_record(item, "crossref"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dois_normalise_and_reject_non_dois() {
        assert_eq!(
            normalize_doi("https://doi.org/10.1038/Nature14539."),
            Some("10.1038/nature14539".to_string())
        );
        assert_eq!(
            normalize_doi("doi:10.1000/ABC"),
            Some("10.1000/abc".to_string())
        );
        assert_eq!(normalize_doi("10.1000"), None);
        assert_eq!(normalize_doi("11.1000/x"), None);
        assert_eq!(normalize_doi("10.1000/a b"), None);
        assert_eq!(normalize_doi(""), None);
    }

    #[test]
    fn balanced_doi_suffixes_follow_the_resolver() {
        for raw in [
            "10.1000/example(abc)",
            "https://doi.org/10.1000/example(abc)).",
            "DOI:10.1000/example[abc]}",
            "10.1000/example(abc)[x]{y}<z>.",
            "10.1002/(SICI)1097-0258(19980815)17:15<1741::AID-SIM868>3.0.CO;2-8",
        ] {
            assert_eq!(normalize_doi(raw), tpe::resolve::doi_in(raw), "{raw}");
        }
        assert_eq!(
            normalize_doi("10.1000/example(abc)"),
            Some("10.1000/example(abc)".into())
        );
        assert_eq!(normalize_doi("10.1/10.1000/abc"), None);
    }

    #[test]
    fn malformed_registry_records_are_errors() {
        for item in [
            "{}",
            r#"{"error":"not a record"}"#,
            r#"{"title":"A title"}"#,
            r#"{"DOI":"invalid"}"#,
            "null",
        ] {
            assert!(parse_csl_json(item).is_err(), "{item}");
            assert!(
                parse_crossref_work(&format!(r#"{{"status":"ok","message":{item}}}"#)).is_err(),
                "{item}"
            );
            assert!(
                parse_crossref_works(&format!(
                    r#"{{"status":"ok","message":{{"items":[{item}]}}}}"#
                ))
                .is_err(),
                "{item}"
            );
        }
    }

    #[test]
    fn csl_item_and_crossref_work_reduce_to_the_same_record() {
        let csl = r#"{"DOI":"10.1038/nature14539","type":"article-journal","title":"Deep learning",
            "author":[{"family":"LeCun","given":"Yann"},{"family":"Bengio","given":"Yoshua"},{"family":"Hinton","given":"Geoffrey"}],
            "issued":{"date-parts":[[2015,5,27]]},"container-title":"Nature","container-title-short":"Nature",
            "volume":"521","issue":"7553","page":"436-444"}"#;
        let crossref = r#"{"status":"ok","message-type":"work","message-version":"1.0.0","message":{
            "DOI":"10.1038/nature14539","type":"journal-article","title":["Deep learning"],
            "author":[{"family":"Bengio","given":"Yoshua","sequence":"additional"},{"family":"LeCun","given":"Yann","sequence":"first"},{"family":"Hinton","given":"Geoffrey","sequence":"additional"}],
            "issued":{"date-parts":[[2015,5,27]]},"published-print":{"date-parts":[[2015,5]]},
            "container-title":["Nature"],"short-container-title":["Nature"],
            "volume":"521","issue":"7553","page":"436-444"}}"#;
        let a = parse_csl_json(csl).unwrap();
        let b = parse_crossref_work(crossref).unwrap();
        assert_eq!(a.doi.as_deref(), Some("10.1038/nature14539"));
        assert_eq!(a.title.as_deref(), Some("Deep learning"));
        assert_eq!(a.authors[0], "Yann LeCun");
        assert_eq!(b.authors[0], "Yann LeCun");
        assert_eq!(b.families[0], "LeCun");
        assert_eq!(a.years, vec![2015]);
        assert_eq!(b.years, vec![2015]);
        assert_eq!(b.container.as_deref(), Some("Nature"));
        assert_eq!(b.pages.as_deref(), Some("436-444"));
        assert_eq!(a.source, "doi.org");
        assert_eq!(b.source, "crossref");
    }

    #[test]
    fn work_list_and_bad_shapes() {
        let list = r#"{"status":"ok","message-type":"work-list","message":{"items":[
            {"DOI":"10.1000/a","title":["A"],"author":[{"name":"Some Consortium"}],"issued":{"date-parts":[[null]]},"published-online":{"date-parts":[[2018,1]]}},
            {"DOI":"10.1000/b","title":[]}
        ],"total-results":2}}"#;
        let records = parse_crossref_works(list).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].authors, vec!["Some Consortium".to_string()]);
        assert_eq!(records[0].years, vec![2018]);
        assert_eq!(records[1].title, None);
        assert!(parse_crossref_works("Resource not found.").is_err());
        assert!(parse_crossref_work(r#"{"status":"failed"}"#).is_err());
        assert!(parse_csl_json("[]").is_err());
    }
}
