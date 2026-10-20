//! Validate extracted reference entries against DOI and Crossref records.
//!
//! The input is an engine JSON output (an `extract` result, a `tpe
//! bibliography` record, a list of either, or bare reference entries). For
//! every entry the checker obtains a registry record, by the entry's DOI
//! when it has one (`doi.org` content negotiation, then Crossref) and by a
//! Crossref bibliographic query otherwise, and compares the record with
//! what is printed: title, first author, year, container, volume and pages.
//! A record counts only when it matches what is printed; a DOI that
//! resolves to another work is a mismatch, not a verification.
//!
//! Verdicts are `verified`, `mismatch` (with the differing fields),
//! `not-found` and `offline-or-error`; a mismatch or not-found entry carries
//! the suggested DOI when a query identified the work. See `docs/REFCHECK.md`.

#![allow(clippy::must_use_candidate, clippy::module_name_repetitions)]

pub mod cache;
pub mod client;
pub mod normalize;
pub mod record;
pub mod report;
pub mod score;

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
pub use tpe::schema::ReferenceEntry;

pub use crate::cache::Cache;
pub use crate::client::{Client, ClientConfig, Lookup, MAILTO_ENV, Transport};
pub use crate::record::Record;
pub use crate::report::{EntryReport, Found, Method, Printed, Report, Verdict};
pub use crate::score::{Comparison, Outcome, Thresholds};

/// Longest entry text sent as a bibliographic query.
pub const QUERY_CHARS: usize = 300;
/// Rows asked from a bibliographic query.
pub const QUERY_ROWS: u32 = 5;

/// The input could not be read as reference entries.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// Not JSON (nor JSON Lines).
    #[error("input is not JSON: {0}")]
    Json(String),
    /// JSON, but not an engine output shape.
    #[error("input has no reference entries: {0}")]
    Shape(String),
}

fn entries_of(value: &Value, out: &mut Vec<ReferenceEntry>) -> Result<(), LoadError> {
    match value {
        Value::Object(map) => {
            if let Some(refs) = map.get("references") {
                return entries_of(refs, out);
            }
            if map.contains_key("raw") {
                let entry: ReferenceEntry = serde_json::from_value(value.clone())
                    .map_err(|e| LoadError::Shape(e.to_string()))?;
                out.push(entry);
                return Ok(());
            }
            Err(LoadError::Shape(
                "object has neither `references` nor the entry fields".to_string(),
            ))
        }
        Value::Array(items) => {
            for item in items {
                entries_of(item, out)?;
            }
            Ok(())
        }
        _ => Err(LoadError::Shape("not an object or array".to_string())),
    }
}

/// The reference entries in an engine JSON output: an object with
/// `references` (an extraction result or a bibliography record), an array
/// of such objects or of entries, or JSON Lines of objects.
///
/// # Errors
/// When the text is not JSON or holds no recognisable entries.
pub fn load_entries(text: &str) -> Result<Vec<ReferenceEntry>, LoadError> {
    let mut out = Vec::new();
    match serde_json::from_str::<Value>(text) {
        Ok(value) => entries_of(&value, &mut out)?,
        Err(first) => {
            // JSON Lines: one object per non-empty line.
            let mut any = false;
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                let value: Value =
                    serde_json::from_str(line).map_err(|_| LoadError::Json(first.to_string()))?;
                entries_of(&value, &mut out)?;
                any = true;
            }
            if !any {
                return Err(LoadError::Json(first.to_string()));
            }
        }
    }
    Ok(out)
}

/// The text sent as a bibliographic query: the raw entry without its label,
/// whitespace collapsed, cut at [`QUERY_CHARS`] characters.
pub fn query_text(entry: &ReferenceEntry) -> String {
    let mut raw = entry.raw.trim();
    if let Some(label) = entry.label.as_deref()
        && let Some(rest) = raw.strip_prefix(label.trim())
    {
        raw = rest.trim_start_matches([' ', '.', ')', ']', '\t']);
    }
    let collapsed: Vec<&str> = raw.split_whitespace().collect();
    let text = collapsed.join(" ");
    if text.is_empty() {
        let mut parts: Vec<String> = Vec::new();
        parts.extend(entry.authors.iter().take(3).cloned());
        parts.extend(entry.title.clone());
        parts.extend(entry.venue.clone());
        parts.extend(entry.year.map(|y| y.to_string()));
        return parts.join(" ");
    }
    text.chars().take(QUERY_CHARS).collect()
}

/// The best query candidate, or why none was accepted.
#[derive(Clone, Debug, PartialEq)]
pub enum Selection {
    /// One candidate identifies the printed work and leads clearly.
    Accepted(Box<(Record, Comparison)>),
    /// Two different DOIs identify the work within the query margin.
    Ambiguous(String, String),
    /// No candidate identifies the work; the best identity score seen.
    None(Option<f32>),
}

/// Pick the query candidate for `entry`: the same-work candidate with the
/// highest identity score, unless the runner-up with a different DOI is
/// within `thresholds.query_margin` of it.
pub fn select_candidate(
    entry: &ReferenceEntry,
    records: &[Record],
    thresholds: &Thresholds,
) -> Selection {
    let mut scored: Vec<(Record, Comparison)> = records
        .iter()
        .map(|r| (r.clone(), score::compare(entry, r, thresholds)))
        .collect();
    let best_any = scored
        .iter()
        .map(|(_, c)| c.identity)
        .fold(None, |acc: Option<f32>, s| {
            Some(acc.map_or(s, |a| a.max(s)))
        });
    scored
        .retain(|(r, c)| r.doi.as_deref().and_then(record::normalize_doi).is_some() && c.same_work);
    scored.sort_by(|a, b| b.1.identity.total_cmp(&a.1.identity));
    let Some((best, best_cmp)) = scored.first().cloned() else {
        return Selection::None(best_any);
    };
    let rival = scored.iter().skip(1).find(|(r, _)| r.doi != best.doi);
    if let Some((rival, rival_cmp)) = rival
        && best_cmp.identity - rival_cmp.identity < thresholds.query_margin
    {
        return Selection::Ambiguous(
            best.doi.clone().unwrap_or_default(),
            rival.doi.clone().unwrap_or_default(),
        );
    }
    Selection::Accepted(Box::new((best, best_cmp)))
}

/// The checker: a client plus the thresholds.
pub struct Checker<'a> {
    client: &'a Client,
    thresholds: Thresholds,
    query: bool,
}

fn printed(entry: &ReferenceEntry) -> Printed {
    Printed {
        doi: score::entry_doi(entry),
        title: entry.title.clone(),
        first_author: score::printed_first_author(entry).map(str::to_string),
        year: score::printed_year(entry),
        venue: entry.venue.clone(),
        volume: entry.volume.clone(),
        pages: entry.pages.clone(),
    }
}

fn found(record: &Record) -> Found {
    Found {
        doi: record.doi.clone(),
        title: record.title.clone(),
        first_author: record.authors.first().cloned(),
        years: record.years.clone(),
        container: record.container.clone(),
        volume: record.volume.clone(),
        pages: record
            .pages
            .clone()
            .or_else(|| record.article_number.clone()),
        kind: record.kind.clone(),
        source: record.source.clone(),
    }
}

impl<'a> Checker<'a> {
    /// A checker with the default thresholds and query fallback on.
    pub fn new(client: &'a Client) -> Self {
        Self {
            client,
            thresholds: Thresholds::default(),
            query: true,
        }
    }

    /// Replace the thresholds.
    #[must_use]
    pub fn with_thresholds(mut self, thresholds: Thresholds) -> Self {
        self.thresholds = thresholds;
        self
    }

    /// Enable or disable Crossref bibliographic queries (entries without a
    /// DOI are then `not-found` without a request).
    #[must_use]
    pub fn with_query(mut self, query: bool) -> Self {
        self.query = query;
        self
    }

    /// The thresholds in use.
    pub fn thresholds(&self) -> &Thresholds {
        &self.thresholds
    }

    fn blank(entry: &ReferenceEntry) -> EntryReport {
        EntryReport {
            index: entry.index,
            label: entry.label.clone(),
            verdict: Verdict::NotFound,
            method: Method::None,
            printed: printed(entry),
            record: None,
            suggested_doi: None,
            fields: Vec::new(),
            comparison: None,
            detail: None,
        }
    }

    /// Run the bibliographic query for `entry`; `Err` is the request error.
    fn query_for(&self, entry: &ReferenceEntry) -> Result<Selection, String> {
        let text = query_text(entry);
        if text.is_empty() {
            return Ok(Selection::None(None));
        }
        let records = self.client.query(&text, QUERY_ROWS)?;
        Ok(select_candidate(entry, &records, &self.thresholds))
    }

    /// Fill `out` from a query selection. `printed_doi` is the DOI the entry
    /// carries (which resolved to nothing or to another work), if any.
    fn apply_query(
        &self,
        entry: &ReferenceEntry,
        out: &mut EntryReport,
        printed_doi: Option<&str>,
    ) {
        match self.query_for(entry) {
            Ok(Selection::Accepted(accepted)) => {
                let (record, comparison) = *accepted;
                out.method = Method::Query;
                out.record = Some(found(&record));
                out.suggested_doi.clone_from(&record.doi);
                out.fields.clone_from(&comparison.differing);
                if printed_doi.is_some() && record.doi.as_deref() != printed_doi {
                    out.fields.insert(0, "doi".to_string());
                }
                out.verdict = if out.fields.is_empty() {
                    Verdict::Verified
                } else {
                    Verdict::Mismatch
                };
                out.detail = Some(format!(
                    "query: {} ({}) identity {:.2}",
                    record.doi.as_deref().unwrap_or("no DOI"),
                    record.source,
                    comparison.identity
                ));
                out.comparison = Some(comparison);
            }
            Ok(Selection::Ambiguous(a, b)) => {
                out.method = Method::Query;
                out.verdict = Verdict::NotFound;
                out.detail = Some(format!("query: ambiguous between {a} and {b}"));
            }
            Ok(Selection::None(best)) => {
                out.verdict = Verdict::NotFound;
                out.detail = Some(match best {
                    Some(score) => {
                        format!("query: no candidate matches (best identity {score:.2})")
                    }
                    None => "query: no candidates".to_string(),
                });
            }
            Err(e) => {
                out.verdict = Verdict::Error;
                out.detail = Some(format!("query: {e}"));
            }
        }
    }

    /// Check one entry.
    pub fn check_entry(&self, entry: &ReferenceEntry) -> EntryReport {
        let mut out = Self::blank(entry);
        let Some(doi) = score::entry_doi(entry) else {
            if self.query {
                self.apply_query(entry, &mut out, None);
            } else {
                out.detail = Some("no DOI printed; queries disabled".to_string());
            }
            return out;
        };
        match self.client.resolve_doi(&doi) {
            Lookup::Found(record) => {
                let comparison = score::compare(entry, &record, &self.thresholds);
                out.method = Method::Doi;
                out.record = Some(found(&record));
                out.fields.clone_from(&comparison.differing);
                let doi_matches = record.doi.as_deref() == Some(doi.as_str());
                if !doi_matches {
                    out.fields.insert(0, "doi".to_string());
                }
                let same_work = comparison.same_work;
                out.verdict = if !out.fields.is_empty() {
                    Verdict::Mismatch
                } else if same_work {
                    Verdict::Verified
                } else {
                    Verdict::NotFound
                };
                out.detail = Some(format!(
                    "doi: {} ({}) identity {:.2}{}",
                    doi,
                    record.source,
                    comparison.identity,
                    if same_work {
                        ""
                    } else {
                        "; insufficient positive same-work evidence"
                    }
                ));
                out.comparison = Some(comparison);
                if (!same_work || !doi_matches) && self.query {
                    // The printed DOI points elsewhere: look for the right one.
                    let mut suggestion = Self::blank(entry);
                    self.apply_query(entry, &mut suggestion, Some(&doi));
                    if matches!(suggestion.verdict, Verdict::Verified | Verdict::Mismatch) {
                        out.suggested_doi = suggestion.suggested_doi;
                        if !out.fields.iter().any(|f| f == "doi") {
                            out.fields.insert(0, "doi".to_string());
                        }
                        out.detail = Some(format!(
                            "{}; {}",
                            out.detail.unwrap_or_default(),
                            suggestion.detail.unwrap_or_default()
                        ));
                    }
                }
            }
            Lookup::NotFound => {
                out.detail = Some(format!("doi: {doi} is not registered"));
                if self.query {
                    let detail = out.detail.take();
                    self.apply_query(entry, &mut out, Some(&doi));
                    out.detail = Some(format!(
                        "{}; {}",
                        detail.unwrap_or_default(),
                        out.detail.take().unwrap_or_default()
                    ));
                }
            }
            Lookup::Error(e) => {
                out.verdict = Verdict::Error;
                out.detail = Some(format!("doi: {doi}: {e}"));
            }
        }
        out
    }

    /// Check every entry and assemble the report for `input`.
    pub fn check_entries(&self, entries: &[ReferenceEntry], input: &str) -> Report {
        let lines: Vec<EntryReport> = entries.iter().map(|e| self.check_entry(e)).collect();
        let stats = self.client.stats();
        let mut report = Report {
            tool: "tpe-refcheck".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            generated_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            input: input.to_string(),
            polite: self.client.config().mailto.is_some(),
            offline: self.client.config().offline,
            thresholds: self.thresholds.clone(),
            summary: report::Summary {
                requests: stats.requests,
                cache_hits: stats.cache_hits,
                retries: stats.retries,
                ..report::Summary::default()
            },
            entries: lines,
        };
        report.tally();
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_every_engine_shape() {
        let entry = r#"{"index":1,"label":"[1]","raw":"A. B. Title. 2020.","authors":["B, A."],"title":"Title","year":2020,"venue":null,"volume":null,"issue":null,"pages":null,"doi":null,"arxiv_id":null,"url":null,"page":3}"#;
        let result = format!(r#"{{"schema_version":5,"references":[{entry}],"warnings":[]}}"#);
        assert_eq!(load_entries(&result).unwrap().len(), 1);
        let record =
            format!(r#"{{"path":"a.pdf","status":"found","references":[{entry},{entry}]}}"#);
        assert_eq!(load_entries(&record).unwrap().len(), 2);
        let jsonl = format!("{record}\n\n{result}\n");
        assert_eq!(load_entries(&jsonl).unwrap().len(), 3);
        assert_eq!(load_entries(&format!("[{entry}]")).unwrap().len(), 1);
        assert!(matches!(load_entries("not json"), Err(LoadError::Json(_))));
        assert!(matches!(
            load_entries(r#"{"pages":[]}"#),
            Err(LoadError::Shape(_))
        ));
        assert!(matches!(load_entries("42"), Err(LoadError::Shape(_))));
    }

    #[test]
    fn query_text_drops_the_label_and_truncates() {
        let entry = ReferenceEntry {
            label: Some("[12]".to_string()),
            raw: format!(
                "[12]  Smith, J.   A title.\n Journal 2020. {}",
                "x".repeat(400)
            ),
            ..ReferenceEntry::default()
        };
        let text = query_text(&entry);
        assert!(text.starts_with("Smith, J. A title. Journal 2020."));
        assert_eq!(text.chars().count(), QUERY_CHARS);
        let fields_only = ReferenceEntry {
            authors: vec!["Smith, J.".to_string()],
            title: Some("A title".to_string()),
            year: Some(2020),
            ..ReferenceEntry::default()
        };
        assert_eq!(query_text(&fields_only), "Smith, J. A title 2020");
    }

    #[test]
    fn candidate_selection_needs_identity_and_a_clear_lead() {
        let entry = ReferenceEntry {
            raw: "Vaswani A, et al. Attention is all you need. NeurIPS 2017.".to_string(),
            authors: vec!["Vaswani, A.".to_string()],
            title: Some("Attention is all you need".to_string()),
            year: Some(2017),
            ..ReferenceEntry::default()
        };
        let make = |doi: &str, title: &str, year: u16| Record {
            doi: Some(doi.to_string()),
            title: Some(title.to_string()),
            authors: vec!["Ashish Vaswani".to_string()],
            families: vec!["Vaswani".to_string()],
            years: vec![year],
            source: "crossref".to_string(),
            ..Record::default()
        };
        let t = Thresholds::default();
        let right = make("10.1000/right", "Attention is all you need", 2017);
        let other = make(
            "10.1000/other",
            "Tensor2Tensor for neural machine translation",
            2018,
        );
        match select_candidate(&entry, &[other.clone(), right.clone()], &t) {
            Selection::Accepted(accepted) => {
                let (r, c) = *accepted;
                assert_eq!(r.doi.as_deref(), Some("10.1000/right"));
                assert!(c.same_work);
            }
            other => panic!("{other:?}"),
        }
        // Two different DOIs with identical evidence (a preprint and its
        // published version) are not decided by response order.
        let twin = make("10.1000/twin", "Attention is all you need", 2017);
        assert!(matches!(
            select_candidate(&entry, &[right.clone(), twin], &t),
            Selection::Ambiguous(_, _)
        ));
        // The same DOI twice is not ambiguity.
        assert!(matches!(
            select_candidate(&entry, &[right.clone(), right], &t),
            Selection::Accepted(_)
        ));
        assert!(matches!(
            select_candidate(&entry, &[other], &t),
            Selection::None(Some(_))
        ));
        assert_eq!(select_candidate(&entry, &[], &t), Selection::None(None));
    }
}
