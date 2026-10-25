//! The JSON report and its Markdown rendering.

use std::fmt::Write;

use serde::{Deserialize, Serialize};

use crate::score::{Comparison, Thresholds};

/// The verdict on one entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    /// A record was found and every compared field agrees.
    #[serde(rename = "verified")]
    Verified,
    /// A record was found (by the entry's DOI, or by query when the entry
    /// has none or its DOI resolves to nothing) and at least one compared
    /// field disagrees; `fields` names them.
    #[serde(rename = "mismatch")]
    Mismatch,
    /// Neither the DOI nor a query produced a record that matches the entry.
    #[serde(rename = "not-found")]
    NotFound,
    /// Offline, or a request failed after its retries; nothing is known.
    #[serde(rename = "offline-or-error")]
    Error,
}

impl Verdict {
    /// The serialised name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Mismatch => "mismatch",
            Self::NotFound => "not-found",
            Self::Error => "offline-or-error",
        }
    }
}

/// How the record was obtained.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Method {
    /// The entry's DOI, resolved at `doi.org` or Crossref.
    Doi,
    /// A Crossref bibliographic query on the entry text.
    Query,
    /// No record was obtained.
    None,
}

/// The printed fields that were compared.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Printed {
    pub doi: Option<String>,
    pub title: Option<String>,
    pub first_author: Option<String>,
    pub year: Option<u16>,
    pub venue: Option<String>,
    pub volume: Option<String>,
    pub pages: Option<String>,
}

/// The record fields that were compared.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Found {
    pub doi: Option<String>,
    pub title: Option<String>,
    pub first_author: Option<String>,
    pub years: Vec<u16>,
    pub container: Option<String>,
    pub volume: Option<String>,
    pub pages: Option<String>,
    pub kind: Option<String>,
    pub source: String,
}

/// One entry's line of the report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntryReport {
    pub index: u32,
    pub label: Option<String>,
    pub verdict: Verdict,
    pub method: Method,
    pub printed: Printed,
    pub record: Option<Found>,
    /// The DOI the entry should carry: the matched record's DOI when the
    /// entry printed none or a DOI that resolves to nothing or to another work.
    pub suggested_doi: Option<String>,
    /// Compared fields that disagree (`doi`, `title`, `author`, `year`,
    /// `container`, `volume`, `pages`).
    pub fields: Vec<String>,
    pub comparison: Option<Comparison>,
    /// Human-readable explanation.
    pub detail: Option<String>,
}

/// Verdict counts and request accounting.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub entries: usize,
    pub verified: usize,
    pub mismatch: usize,
    pub not_found: usize,
    pub error: usize,
    pub requests: u64,
    pub cache_hits: u64,
    pub retries: u64,
}

/// The whole report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub tool: String,
    pub version: String,
    /// Unix seconds.
    pub generated_at: u64,
    pub input: String,
    /// Whether a `mailto` contact was sent (Crossref's polite pool).
    pub polite: bool,
    pub offline: bool,
    pub thresholds: Thresholds,
    pub summary: Summary,
    pub entries: Vec<EntryReport>,
}

impl Report {
    /// Count verdicts into `summary`.
    pub fn tally(&mut self) {
        let mut s = Summary {
            entries: self.entries.len(),
            requests: self.summary.requests,
            cache_hits: self.summary.cache_hits,
            retries: self.summary.retries,
            ..Summary::default()
        };
        for e in &self.entries {
            match e.verdict {
                Verdict::Verified => s.verified += 1,
                Verdict::Mismatch => s.mismatch += 1,
                Verdict::NotFound => s.not_found += 1,
                Verdict::Error => s.error += 1,
            }
        }
        self.summary = s;
    }
}

/// Longest Markdown cell before truncation.
const CELL_MAX: usize = 70;

fn cell(text: &str) -> String {
    let mut out = text.replace(['\n', '\r'], " ").replace('|', "\\|");
    if out.chars().count() > CELL_MAX {
        out = out.chars().take(CELL_MAX - 1).collect::<String>() + "…";
    }
    out
}

fn score_cell(value: Option<f32>) -> String {
    value.map_or_else(|| "–".to_string(), |v| format!("{v:.2}"))
}

/// Render the report as a Markdown table with a summary line.
pub fn markdown(report: &Report) -> String {
    let s = &report.summary;
    let mut out = String::new();
    let _ = write!(
        out,
        "# Reference check: {}\n\n{} entries: {} verified, {} mismatch, {} not found, {} offline/error. \
         {} requests, {} cache hits, {} retries. Polite pool: {}.\n\n",
        cell(&report.input),
        s.entries,
        s.verified,
        s.mismatch,
        s.not_found,
        s.error,
        s.requests,
        s.cache_hits,
        s.retries,
        if report.polite {
            "yes"
        } else {
            "no (set TPE_MAILTO)"
        },
    );
    out.push_str(
        "| # | Verdict | Printed DOI | Suggested DOI | Title | Author | Differs | Detail |\n",
    );
    out.push_str("|---|---|---|---|---|---|---|---|\n");
    for e in &report.entries {
        let (title, author) = e
            .comparison
            .as_ref()
            .map_or((None, None), |c| (c.title, c.author));
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            e.label.as_deref().map_or_else(|| e.index.to_string(), cell),
            e.verdict.as_str(),
            e.printed.doi.as_deref().map_or_else(String::new, cell),
            e.suggested_doi.as_deref().map_or_else(String::new, cell),
            score_cell(title),
            score_cell(author),
            e.fields.join(", "),
            e.detail.as_deref().map_or_else(String::new, cell),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_names_match_serde() {
        for v in [
            Verdict::Verified,
            Verdict::Mismatch,
            Verdict::NotFound,
            Verdict::Error,
        ] {
            assert_eq!(
                serde_json::to_string(&v).unwrap(),
                format!("\"{}\"", v.as_str())
            );
        }
    }

    #[test]
    fn markdown_escapes_pipes_and_tallies() {
        let mut report = Report {
            tool: "tpe-refcheck".into(),
            version: "0".into(),
            generated_at: 0,
            input: "in|put.json".into(),
            polite: false,
            offline: true,
            thresholds: Thresholds::default(),
            summary: Summary::default(),
            entries: vec![EntryReport {
                index: 1,
                label: Some("[1]".into()),
                verdict: Verdict::Mismatch,
                method: Method::Doi,
                printed: Printed::default(),
                record: None,
                suggested_doi: None,
                fields: vec!["pages".into()],
                comparison: None,
                detail: Some("a | b\nc".into()),
            }],
        };
        report.tally();
        assert_eq!(report.summary.mismatch, 1);
        assert_eq!(report.summary.entries, 1);
        let md = markdown(&report);
        assert!(md.contains("in\\|put.json"));
        assert!(md.contains("| [1] | mismatch |  |  | – | – | pages | a \\| b c |"));
        assert!(md.contains("set TPE_MAILTO"));
    }
}
