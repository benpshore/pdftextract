//! Field-by-field comparison of a printed reference entry with a registry
//! record. A record is never accepted on its DOI alone: the printed title,
//! first author and year must agree with it, and the container, volume and
//! pages are checked when both sides print them.

use serde::{Deserialize, Serialize};
use tpe::schema::ReferenceEntry;

use crate::normalize::{
    container_similarity, containment, fold, jaccard, page_range, string_similarity,
    surname_similarity, volume_key, word_set, words, year_in,
};
use crate::record::{Record, normalize_doi};

/// Acceptance thresholds (documented in `docs/REFCHECK.md`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    /// Least title similarity for the titles to agree.
    pub title_match: f32,
    /// Title similarity from which the titles are only *weakly* different
    /// (reported, but not enough on its own to call the record another work).
    pub title_weak: f32,
    /// Title similarity that identifies the work without any author evidence.
    pub title_strong: f32,
    /// Least surname similarity for the first authors to agree.
    pub author_match: f32,
    /// Largest year difference that still agrees.
    pub year_tolerance: u16,
    /// Least container similarity for the containers to agree.
    pub container_match: f32,
    /// Least identity-score lead of the best query candidate over the
    /// runner-up with a different DOI.
    pub query_margin: f32,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            title_match: 0.80,
            title_weak: 0.60,
            title_strong: 0.90,
            author_match: 0.85,
            year_tolerance: 1,
            container_match: 0.75,
            query_margin: 0.05,
        }
    }
}

/// How one field compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Both sides have the field and they agree.
    Agree,
    /// Both sides have the field and they disagree.
    Differ,
    /// One side (or both) lacks the field; nothing to compare.
    Unknown,
}

/// The result of comparing one entry with one record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    pub title: Option<f32>,
    pub title_outcome: Outcome,
    pub author: Option<f32>,
    pub author_outcome: Outcome,
    pub year: Outcome,
    pub container: Option<f32>,
    pub container_outcome: Outcome,
    pub volume: Outcome,
    pub pages: Outcome,
    /// Mean of the title, author and year evidence, `0..=1`.
    pub identity: f32,
    /// Do title, author and year say the record is the printed work?
    pub same_work: bool,
    /// Compared fields that disagree, in report order.
    pub differing: Vec<String>,
}

/// Similarity of two titles, `0..=1`: the best of the normalised string
/// similarity and the mean of word Jaccard and word containment. A title
/// that is a word-prefix of the other (a subtitle dropped on one side) with
/// at least three words scores `1`.
pub fn title_similarity(a: &str, b: &str) -> f32 {
    let (fa, fb) = (fold(a), fold(b));
    if fa.is_empty() || fb.is_empty() {
        return 0.0;
    }
    if fa == fb {
        return 1.0;
    }
    let (wa, wb) = (words(a), words(b));
    let (short, long) = if wa.len() <= wb.len() {
        (&wa, &wb)
    } else {
        (&wb, &wa)
    };
    if short.len() >= 3 && long.starts_with(short) {
        return 1.0;
    }
    let (sa, sb) = (word_set(a), word_set(b));
    let sets = f32::midpoint(jaccard(&sa, &sb), containment(&sa, &sb));
    string_similarity(&fa, &fb).max(sets)
}

/// The DOI the entry carries: the `doi.org` link annotation first (an exact
/// string from the PDF), then the printed DOI text.
pub fn entry_doi(entry: &ReferenceEntry) -> Option<String> {
    entry
        .doi_link
        .as_deref()
        .and_then(normalize_doi)
        .or_else(|| entry.doi.as_deref().and_then(normalize_doi))
}

/// The printed first author, as parsed; `None` when no author was parsed.
pub fn printed_first_author(entry: &ReferenceEntry) -> Option<&str> {
    entry
        .authors
        .iter()
        .map(String::as_str)
        .find(|a| !a.trim().is_empty())
}

/// The printed year: the parsed field, else the first year in the raw text.
pub fn printed_year(entry: &ReferenceEntry) -> Option<u16> {
    entry.year.or_else(|| year_in(&entry.raw))
}

/// The record's title with its subtitle, for entries that print both.
fn record_full_title(record: &Record) -> Option<String> {
    let title = record.title.as_deref()?;
    Some(match record.subtitle.as_deref() {
        Some(sub) if !sub.is_empty() => format!("{title}: {sub}"),
        _ => title.to_string(),
    })
}

fn compare_title(
    entry: &ReferenceEntry,
    record: &Record,
    t: &Thresholds,
) -> (Option<f32>, Outcome) {
    let Some(record_title) = record.title.as_deref() else {
        return (None, Outcome::Unknown);
    };
    let printed = match entry.title.as_deref() {
        Some(title) if !title.trim().is_empty() => title.to_string(),
        // No parsed title: the raw entry must contain the record's words.
        _ => {
            let raw = word_set(&entry.raw);
            let needles = word_set(record_title);
            if needles.is_empty() {
                return (None, Outcome::Unknown);
            }
            let score = containment(&needles, &raw);
            let outcome = if score >= t.title_match {
                Outcome::Agree
            } else {
                Outcome::Differ
            };
            return (Some(score), outcome);
        }
    };
    let mut score = title_similarity(&printed, record_title);
    if let Some(full) = record_full_title(record) {
        score = score.max(title_similarity(&printed, &full));
    }
    let outcome = if score >= t.title_match {
        Outcome::Agree
    } else {
        Outcome::Differ
    };
    (Some(score), outcome)
}

fn compare_author(
    entry: &ReferenceEntry,
    record: &Record,
    t: &Thresholds,
) -> (Option<f32>, Outcome) {
    let Some(record_first) = record.authors.first() else {
        return (None, Outcome::Unknown);
    };
    let record_family = record
        .families
        .first()
        .filter(|f| !f.is_empty())
        .map_or_else(|| record_first.clone(), Clone::clone);
    if let Some(printed) = printed_first_author(entry) {
        let score = surname_similarity(printed, &record_family);
        let outcome = if score >= t.author_match {
            Outcome::Agree
        } else {
            Outcome::Differ
        };
        return (Some(score), outcome);
    }
    // No parsed author: the family name must open the raw entry.
    let key = fold(&record_family).replace(' ', "");
    if key.is_empty() {
        return (None, Outcome::Unknown);
    }
    let opening: Vec<String> = words(&entry.raw).into_iter().take(10).collect();
    let mut best: f32 = 0.0;
    for (i, w) in opening.iter().enumerate() {
        best = best.max(string_similarity(w, &key));
        if let Some(next) = opening.get(i + 1) {
            best = best.max(string_similarity(&format!("{w}{next}"), &key));
        }
    }
    let outcome = if best >= t.author_match {
        Outcome::Agree
    } else {
        Outcome::Differ
    };
    (Some(best), outcome)
}

fn compare_year(entry: &ReferenceEntry, record: &Record, t: &Thresholds) -> Outcome {
    let Some(printed) = printed_year(entry) else {
        return Outcome::Unknown;
    };
    if record.years.is_empty() {
        return Outcome::Unknown;
    }
    let within = record
        .years
        .iter()
        .any(|&y| y.abs_diff(printed) <= t.year_tolerance);
    if within {
        Outcome::Agree
    } else {
        Outcome::Differ
    }
}

fn compare_container(
    entry: &ReferenceEntry,
    record: &Record,
    t: &Thresholds,
) -> (Option<f32>, Outcome) {
    let Some(printed) = entry.venue.as_deref().filter(|v| !v.trim().is_empty()) else {
        return (None, Outcome::Unknown);
    };
    let candidates: Vec<&str> = [
        record.container.as_deref(),
        record.container_short.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect();
    if candidates.is_empty() {
        return (None, Outcome::Unknown);
    }
    let score = candidates
        .iter()
        .map(|c| container_similarity(printed, c))
        .fold(0.0_f32, f32::max);
    let outcome = if score >= t.container_match {
        Outcome::Agree
    } else {
        Outcome::Differ
    };
    (Some(score), outcome)
}

fn compare_volume(entry: &ReferenceEntry, record: &Record) -> Outcome {
    let (Some(printed), Some(found)) = (entry.volume.as_deref(), record.volume.as_deref()) else {
        return Outcome::Unknown;
    };
    let (a, b) = (volume_key(printed), volume_key(found));
    if a.is_empty() || b.is_empty() {
        return Outcome::Unknown;
    }
    if a == b {
        Outcome::Agree
    } else {
        Outcome::Differ
    }
}

fn compare_pages(entry: &ReferenceEntry, record: &Record) -> Outcome {
    let Some(printed) = entry.pages.as_deref().and_then(page_range) else {
        return Outcome::Unknown;
    };
    let found = record
        .pages
        .as_deref()
        .and_then(page_range)
        .or_else(|| record.article_number.as_deref().and_then(page_range));
    let Some(found) = found else {
        return Outcome::Unknown;
    };
    if printed.0 == found.0 {
        Outcome::Agree
    } else {
        Outcome::Differ
    }
}

/// Compare `entry` with `record` under `t`.
pub fn compare(entry: &ReferenceEntry, record: &Record, t: &Thresholds) -> Comparison {
    let (title, title_outcome) = compare_title(entry, record, t);
    let (author, author_outcome) = compare_author(entry, record, t);
    let year = compare_year(entry, record, t);
    let (container, container_outcome) = compare_container(entry, record, t);
    let volume = compare_volume(entry, record);
    let pages = compare_pages(entry, record);

    let mut parts: Vec<f32> = Vec::new();
    if let Some(s) = title {
        parts.push(s);
    }
    if let Some(s) = author {
        parts.push(s.min(1.0));
    }
    match year {
        Outcome::Agree => parts.push(1.0),
        Outcome::Differ => parts.push(0.0),
        Outcome::Unknown => {}
    }
    let identity = if parts.is_empty() {
        0.0
    } else {
        parts.iter().sum::<f32>() / crate::normalize::ratio(parts.len(), 1)
    };

    let same_work = match title_outcome {
        Outcome::Agree => {
            author_outcome == Outcome::Agree
                || (author_outcome == Outcome::Unknown && title.unwrap_or(0.0) >= t.title_strong)
        }
        Outcome::Differ => false,
        Outcome::Unknown => {
            author_outcome == Outcome::Agree
                && year == Outcome::Agree
                && (volume == Outcome::Agree || pages == Outcome::Agree)
        }
    };

    let mut differing = Vec::new();
    let flagged = [
        ("title", title_outcome),
        ("author", author_outcome),
        ("year", year),
        ("container", container_outcome),
        ("volume", volume),
        ("pages", pages),
    ];
    for (name, outcome) in flagged {
        if outcome == Outcome::Differ {
            differing.push(name.to_string());
        }
    }

    Comparison {
        title,
        title_outcome,
        author,
        author_outcome,
        year,
        container,
        container_outcome,
        volume,
        pages,
        identity,
        same_work,
        differing,
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn entry(authors: &[&str], title: &str, year: Option<u16>) -> ReferenceEntry {
        ReferenceEntry {
            index: 1,
            raw: format!(
                "{} {title} {}",
                authors.join(", "),
                year.map_or(String::new(), |y| y.to_string())
            ),
            authors: authors.iter().map(ToString::to_string).collect(),
            title: Some(title.to_string()),
            year,
            page: 1,
            ..ReferenceEntry::default()
        }
    }

    fn record(first: &str, title: &str, years: &[u16]) -> Record {
        let family = first.split_whitespace().last().unwrap_or("").to_string();
        Record {
            doi: Some("10.1/x".to_string()),
            title: Some(title.to_string()),
            authors: vec![first.to_string()],
            families: vec![family],
            years: years.to_vec(),
            source: "test".to_string(),
            ..Record::default()
        }
    }

    #[test]
    fn title_similarity_handles_subtitles_diacritics_and_ocr_noise() {
        assert_eq!(
            title_similarity("Attention is all you need", "Attention Is All You Need"),
            1.0
        );
        assert_eq!(
            title_similarity(
                "Attention is all you need",
                "Attention is all you need: a transformer architecture"
            ),
            1.0
        );
        assert!(title_similarity("Deep leaming for images", "Deep learning for images") > 0.9);
        assert_eq!(
            title_similarity("Éléments de géométrie", "Elements de geometrie"),
            1.0
        );
        assert!(
            title_similarity("A survey of deep learning", "Deep learning survey methods") < 0.8
        );
        assert!(title_similarity("Graph neural networks", "Convolutional neural networks") < 0.8);
        assert_eq!(title_similarity("", "x"), 0.0);
    }

    #[test]
    fn subtitle_in_the_record_matches_a_printed_full_title() {
        let e = entry(&["Doe, J."], "Alpha: the beta of gamma", Some(2020));
        let mut r = record("Jane Doe", "Alpha", &[2020]);
        r.subtitle = Some("the beta of gamma".to_string());
        let c = compare(&e, &r, &Thresholds::default());
        assert_eq!(c.title_outcome, Outcome::Agree);
        assert!(c.same_work);
        assert!(c.differing.is_empty());
    }

    #[test]
    fn same_work_needs_author_or_strong_title() {
        let t = Thresholds::default();
        let e = entry(
            &["Vaswani, A.", "et al."],
            "Attention is all you need",
            Some(2017),
        );
        let c = compare(
            &e,
            &record("Ashish Vaswani", "Attention is all you need", &[2017]),
            &t,
        );
        assert!(c.same_work);
        assert_eq!(c.author_outcome, Outcome::Agree);
        let wrong_author = compare(
            &e,
            &record("Yann LeCun", "Attention is all you need", &[2017]),
            &t,
        );
        assert!(!wrong_author.same_work);
        assert_eq!(wrong_author.differing, vec!["author".to_string()]);
        let mut no_author = record("x", "Attention is all you need", &[2017]);
        no_author.authors.clear();
        no_author.families.clear();
        assert!(compare(&e, &no_author, &t).same_work);
    }

    #[test]
    fn preprint_and_published_years_within_one_agree_two_apart_differ() {
        let t = Thresholds::default();
        let e = entry(&["Vaswani, A."], "Attention is all you need", Some(2017));
        let online_first = record("Ashish Vaswani", "Attention is all you need", &[2018]);
        assert_eq!(compare(&e, &online_first, &t).year, Outcome::Agree);
        let print_later = record("Ashish Vaswani", "Attention is all you need", &[2019, 2018]);
        assert_eq!(compare(&e, &print_later, &t).year, Outcome::Agree);
        let published_much_later = record("Ashish Vaswani", "Attention is all you need", &[2019]);
        let c = compare(&e, &published_much_later, &t);
        assert_eq!(c.year, Outcome::Differ);
        assert!(c.same_work, "a year gap does not make it another work");
        assert_eq!(c.differing, vec!["year".to_string()]);
    }

    #[test]
    fn raw_text_stands_in_for_unparsed_title_author_and_year() {
        let t = Thresholds::default();
        let e = ReferenceEntry {
            index: 3,
            raw: "Obtułowicz Ł, Kowalski J. Deep learning for Polish texts. J Mach Learn Res. 2019;20:1-9."
                .to_string(),
            page: 2,
            ..ReferenceEntry::default()
        };
        let mut r = record(
            "Lukasz Obtulowicz",
            "Deep learning for Polish texts",
            &[2019],
        );
        r.container = Some("Journal of Machine Learning Research".to_string());
        let c = compare(&e, &r, &t);
        assert_eq!(c.title_outcome, Outcome::Agree);
        assert_eq!(c.author_outcome, Outcome::Agree);
        assert_eq!(c.year, Outcome::Agree);
        assert_eq!(
            c.container_outcome,
            Outcome::Unknown,
            "no parsed venue: nothing to compare"
        );
        assert!(c.same_work);
    }

    #[test]
    fn container_volume_and_pages_are_checked_when_both_print_them() {
        let t = Thresholds::default();
        let mut e = entry(&["LeCun, Y."], "Deep learning", Some(2015));
        e.venue = Some("Nature".to_string());
        e.volume = Some("521".to_string());
        e.pages = Some("436–44".to_string());
        let mut r = record("Yann LeCun", "Deep learning", &[2015]);
        r.container = Some("Nature".to_string());
        r.volume = Some("521".to_string());
        r.pages = Some("436-444".to_string());
        let c = compare(&e, &r, &t);
        assert!(c.differing.is_empty(), "{:?}", c.differing);
        e.pages = Some("437-444".to_string());
        e.volume = Some("522".to_string());
        e.venue = Some("Science".to_string());
        let c = compare(&e, &r, &t);
        assert_eq!(
            c.differing,
            vec![
                "container".to_string(),
                "volume".to_string(),
                "pages".to_string()
            ]
        );
        assert!(c.same_work);
        r.pages = None;
        r.article_number = Some("437".to_string());
        assert_eq!(compare(&e, &r, &t).pages, Outcome::Agree);
    }

    #[test]
    fn entry_doi_prefers_the_link_annotation() {
        let mut e = entry(&["A, B."], "T", None);
        e.doi = Some("10.1000/PRINTED".to_string());
        assert_eq!(entry_doi(&e).as_deref(), Some("10.1000/printed"));
        e.doi_link = Some("https://doi.org/10.1000/LINKED".to_string());
        assert_eq!(entry_doi(&e).as_deref(), Some("10.1000/linked"));
    }
}
