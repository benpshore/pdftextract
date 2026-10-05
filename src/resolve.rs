//! Resolve reference entries, and the paper itself, to DOI records that
//! are verified against what is printed.
//!
//! A DOI is an exact string: one wrong character resolves to nothing or,
//! worse, to another work. So the DOI is taken from the most reliable
//! place first, and every record is checked against the printed entry
//! before it is accepted:
//!
//! 1. a `doi.org` link annotation placed on the entry ([`attach_links`]),
//!    an exact string from the PDF's annotation dictionary;
//! 2. the DOI printed in the entry text;
//! 3. a bibliographic query on the entry text.
//!
//! A record is accepted only when its first author agrees with the printed
//! first author (or, when no author was parsed, its title agrees with the
//! printed title), and its year is within one of the printed year. Anything
//! else stays unresolved and the entry keeps only its printed fields.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use tpe_biblio::doi_metadata::{self, PublisherMetadata};
use tpe_biblio::util::with_query;
use tpe_biblio::{
    BiblioError, Client, DiskCache, Fetcher, Lookup, PaperRecord, RetryPolicy, crossref,
    normalize_doi, pmc, pubmed,
};
use unicode_normalization::UnicodeNormalization;

use crate::metadata::{PaperIdentifiers, extract_identifiers};
use crate::schema::{Attempt, Metadata, PageText, ReferenceEntry, Resolved};

/// Crossref host, for the polite-pool rate limit.
const CROSSREF_HOST: &str = "api.crossref.org";
/// Interval between Crossref requests. The anonymous pool answers HTTP 429
/// well below its nominal limit for bibliographic queries; the polite pool
/// (a `mailto`) is faster and steadier.
const CROSSREF_INTERVAL: Duration = Duration::from_millis(200);
/// Retries after HTTP 429 or a transport failure, with backoff
/// `RETRY_BASE`, doubled each time.
const RETRIES: u32 = 4;
/// First backoff after a failed request.
const RETRY_BASE: Duration = Duration::from_millis(1500);
/// Rows asked from a bibliographic query.
const QUERY_ROWS: u32 = 5;
/// Longest entry text sent as a query.
const QUERY_CHARS: usize = 300;
/// Least title agreement for an accepted record when no author was parsed.
const TITLE_MIN: f32 = 0.7;
/// Least title agreement for the paper's own record found by query.
const PAPER_TITLE_MIN: f32 = 0.85;

/// A DOI in running text: `10.<registrant>/<suffix>`, without trailing
/// sentence punctuation.
fn doi_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b10\.\d{4,9}/[^\s\x22\x27]+").expect("valid regex"))
}

/// The DOI in `text` (a URI or a printed string), lower-cased, without a
/// resolver prefix or trailing punctuation; `None` when there is none.
#[must_use]
pub fn doi_in(text: &str) -> Option<String> {
    let found = doi_re().find(text)?;
    let mut doi = found.as_str();
    // SICI identifiers contain balanced parentheses and angle brackets. Only
    // remove citation delimiters with no opening partner inside the DOI.
    loop {
        let before = doi;
        doi = doi.trim_end_matches(['.', ',', ';', ':']);
        for (open, close) in [('(', ')'), ('[', ']'), ('{', '}'), ('<', '>')] {
            while doi.ends_with(close) && doi.matches(close).count() > doi.matches(open).count() {
                doi = &doi[..doi.len() - close.len_utf8()];
            }
        }
        if doi == before {
            break;
        }
    }
    normalize_doi(doi)
}

/// Attach `doi.org` link annotations to the entries they sit on. An entry
/// owns the links whose centre lies at or below its first line and in its
/// column (`x` between the entry's left edge and the page's midline plus a
/// margin), closer than any later entry. When one entry carries several
/// distinct DOIs the topmost is kept and the rest are ignored.
pub fn attach_links(entries: &mut [ReferenceEntry], pages: &[PageText]) {
    for page in pages {
        let dois: Vec<(f32, f32, String)> = page
            .links
            .iter()
            .filter_map(|link| {
                let bbox = link.bbox?;
                let doi = doi_in(&link.uri)?;
                Some((f32::midpoint(bbox.y0, bbox.y1), bbox.x0, doi))
            })
            .collect();
        if dois.is_empty() {
            continue;
        }
        let reach = page.width * 0.6;
        for (cy, cx, doi) in dois {
            // The lowest entry that starts at or above the link, in its column.
            let mut best: Option<(usize, f32)> = None;
            for (k, entry) in entries.iter().enumerate() {
                if entry.page != page.page {
                    continue;
                }
                let Some(anchor) = entry.anchor else {
                    continue;
                };
                let top = anchor.y1;
                if top < cy - 0.5 * (anchor.y1 - anchor.y0) {
                    continue;
                }
                if cx < anchor.x0 - 12.0 || cx > anchor.x0 + reach {
                    continue;
                }
                if best.is_none_or(|(_, y)| anchor.y0 < y) {
                    best = Some((k, anchor.y0));
                }
            }
            if let Some((k, _)) = best
                && entries[k].doi_link.is_none()
            {
                entries[k].doi_link = Some(doi);
            }
        }
    }
}

/// Letters and digits of `text`, lower-cased, diacritics removed. Stroked
/// and ligature letters that NFKD leaves alone are mapped by hand, since
/// registries often store their ASCII forms (`Obtułowicz` as `Obtulowicz`).
fn folded(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.nfkd() {
        if !(c.is_alphanumeric() || c.is_whitespace()) {
            continue;
        }
        match c {
            'ł' | 'Ł' => out.push('l'),
            'đ' | 'Đ' | 'ð' | 'Ð' => out.push('d'),
            'ø' | 'Ø' => out.push('o'),
            'ı' => out.push('i'),
            'ß' => out.push_str("ss"),
            'æ' | 'Æ' => out.push_str("ae"),
            'œ' | 'Œ' => out.push_str("oe"),
            'þ' | 'Þ' => out.push_str("th"),
            other => out.extend(other.to_lowercase()),
        }
    }
    out
}

/// Levenshtein similarity in `0..=1` over chars.
fn similarity(a: &str, b: &str) -> f32 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let distance = prev[b.len()];
    let longest = a.len().max(b.len());
    1.0 - (distance as f32) / (longest as f32)
}

/// The family name of a record author (`Given Family`): its last word, or
/// the last two when the second-last is a lower-case particle.
fn family_of(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    match words.as_slice() {
        [] => String::new(),
        [.., particle, last]
            if particle.chars().next().is_some_and(char::is_lowercase) && particle.len() <= 4 =>
        {
            format!("{particle} {last}")
        }
        [.., last] => (*last).to_string(),
    }
}

/// Agreement between the record's first author and the printed first
/// author: 1 when the record's family name is a word of the printed name,
/// otherwise the best similarity of the family name to any printed word.
#[cfg(test)]
fn author_agreement(printed: &str, record: &str) -> f32 {
    let family = folded(&family_of(record));
    let printed = folded(printed);
    if family.is_empty() || printed.is_empty() {
        return 0.0;
    }
    if printed.split_whitespace().any(|w| w == family) || printed.contains(&family) {
        return 1.0;
    }
    printed
        .split_whitespace()
        .map(|w| similarity(w, &family))
        .fold(0.0, f32::max)
}

/// Agreement between two titles after folding.
fn title_agreement(a: &str, b: &str) -> f32 {
    let a = folded(a);
    let b = folded(b);
    let a = a.split_whitespace().collect::<Vec<_>>().join(" ");
    let b = b.split_whitespace().collect::<Vec<_>>().join(" ");
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    similarity(&a, &b)
}

/// Least share of a record title's words that must appear in the entry.
const TITLE_OVERLAP_MIN: f32 = 0.6;
/// Least similarity between a record family name and an entry word.
const FAMILY_WORD_MIN: f32 = 0.8;

/// Words of `folded` text that are at least `min` chars long, deduplicated.
fn words(text: &str, min: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in text.split_whitespace() {
        if w.chars().count() >= min && !out.iter().any(|o| o == w) {
            out.push(w.to_string());
        }
    }
    out
}

/// Does the record's first-author family name appear in the printed entry?
/// A word match, a match with the spaces removed (a floating accent glyph
/// splits `Rühland` into `Ru Èhland`), or a close word.
fn family_in_entry(family: &str, raw_folded: &str) -> bool {
    let family = folded(family);
    if family.is_empty() {
        return false;
    }
    if raw_folded.split_whitespace().any(|w| w == family) {
        return true;
    }
    let squashed: String = raw_folded.chars().filter(|c| !c.is_whitespace()).collect();
    let family_squashed: String = family.chars().filter(|c| !c.is_whitespace()).collect();
    // The first author opens the entry: `RowJR` for `Row JR`.
    if squashed.starts_with(&family_squashed) {
        return true;
    }
    if family_squashed.chars().count() >= 4 && squashed.contains(&family_squashed) {
        return true;
    }
    // A floating accent glyph splits a name (`Ru Èhland`, `Arau Âjo`): compare
    // one- and two-word windows of the entry's opening with the spaces removed.
    let opening: Vec<&str> = raw_folded.split_whitespace().take(8).collect();
    for (k, w) in opening.iter().enumerate() {
        if w.chars().count() >= 4 && similarity(w, &family_squashed) >= FAMILY_WORD_MIN {
            return true;
        }
        if let Some(next) = opening.get(k + 1) {
            let pair = format!("{w}{next}");
            if pair.chars().count() >= 4 && similarity(&pair, &family_squashed) >= FAMILY_WORD_MIN {
                return true;
            }
        }
    }
    false
}

/// Share of the record title's words (4+ chars) that the entry contains,
/// or `None` when the title has fewer than two such words.
fn title_overlap(title: &str, raw_folded: &str) -> Option<f32> {
    let needles = words(&folded(title), 4);
    if needles.len() < 2 {
        return None;
    }
    let hay = words(raw_folded, 1);
    let hits = needles
        .iter()
        .filter(|n| hay.iter().any(|h| h == *n))
        .count();
    Some(hits as f32 / needles.len() as f32)
}

/// Does the record year, or the year before or after it, appear in the entry?
fn year_in_entry(year: u16, raw_folded: &str) -> bool {
    let candidates = [year.saturating_sub(1), year, year.saturating_add(1)];
    raw_folded
        .split_whitespace()
        .any(|w| candidates.iter().any(|y| w == y.to_string()))
}

/// Publishers sometimes append the citing paper's own citation to the last
/// reference. Its author, year and title are not evidence for that reference.
/// Preserve the original text on the entry for inspection.
fn reference_text(raw: &str) -> &str {
    static FOOTER: OnceLock<Regex> = OnceLock::new();
    let footer = FOOTER.get_or_init(|| {
        Regex::new(r"(?i)\bcite\s+this\s+(?:article|paper)\s+as\s*:").expect("valid regex")
    });
    footer.find(raw).map_or(raw, |m| &raw[..m.start()])
}

/// Multiple distinct printed DOIs can mark joined references or a reference
/// to several works. Neither DOI order nor a query match chooses the intended
/// identity. Reuse the citation parser's line-wrap repair, and treat repeated
/// forms of the same DOI as one identity.
fn reject_ambiguous_printed_dois(entry: &mut ReferenceEntry) -> bool {
    let text = reference_text(&entry.raw);
    let mut starts = crate::citations::doi_start_re().find_iter(text).peekable();
    let mut search_from = 0;
    let mut first = None;
    while let Some(start) = starts.next() {
        // Bound each repair to its own DOI. A broken first identifier must not
        // hide later ones, and adjacent DOIs must not be joined as line wraps.
        // Keep the preceding context: doi.org/ enables the parser's URL-wrap
        // repair. Advancing past the previous start skips malformed prefixes.
        let begin = search_from;
        search_from = start.end();
        let end = starts.peek().map_or(text.len(), regex::Match::start);
        let Some((_, doi)) = crate::citations::find_doi(&text[begin..end]) else {
            continue;
        };
        if let Some(doi) = normalize_doi(&doi) {
            if first.as_ref().is_some_and(|previous| previous != &doi) {
                entry.attempts.push(Attempt {
                    method: "printed".to_string(),
                    doi: None,
                    outcome: "ambiguous".to_string(),
                    detail: Some("entry contains multiple distinct printed DOIs".to_string()),
                });
                return true;
            }
            first = Some(doi);
        }
    }
    false
}

/// Verify `record` against the printed `entry`: the score, or why not
/// (which check failed, with both values). The checks read the raw entry
/// text, not the parsed fields, so a parser slip cannot reject a correct
/// record: the record's first-author family name must appear in the
/// entry, its year (within one) must appear, and, when the record title
/// has words to check, most of them must appear.
fn verify(record: &PaperRecord, entry: &ReferenceEntry) -> Result<f32, String> {
    let raw = folded(reference_text(&entry.raw));
    let mut score_parts: Vec<f32> = Vec::new();
    if let Some(year) = record.year {
        if !year_in_entry(year, &raw) {
            return Err(format!(
                "year: record {year} not in entry (printed {})",
                entry.year.map_or("none".to_string(), |y| y.to_string())
            ));
        }
        score_parts.push(1.0);
    }
    let title = title_overlap(&record.title, &raw);
    if let Some(first) = record.authors.first() {
        let family = family_of(first);
        if !family_in_entry(&family, &raw) {
            return Err(format!(
                "first author: record {first:?} ({family}) not in entry (printed {})",
                entry
                    .authors
                    .first()
                    .map_or("none".to_string(), |a| format!("{a:?}"))
            ));
        }
        score_parts.push(1.0);
        if let Some(overlap) = title {
            if overlap < TITLE_OVERLAP_MIN {
                return Err(format!(
                    "title: record {:?} shares {:.0}% of its words with the entry",
                    record.title,
                    overlap * 100.0
                ));
            }
            score_parts.push(overlap);
        }
    } else {
        let Some(overlap) = title else {
            return Err("nothing to compare: record has no author and no usable title".to_string());
        };
        if overlap < TITLE_MIN {
            return Err(format!(
                "title: record {:?} shares {:.0}% of its words with the entry (no record author)",
                record.title,
                overlap * 100.0
            ));
        }
        score_parts.push(overlap);
    }
    if score_parts.is_empty() {
        return Err("nothing to compare: record has no year, author or title".to_string());
    }
    Ok(score_parts.iter().sum::<f32>() / score_parts.len() as f32)
}

/// The accepted record as stored on the entry.
fn resolved_from(record: &PaperRecord, doi: &str, method: &str, score: f32) -> Resolved {
    Resolved {
        doi: (!doi.is_empty()).then(|| doi.to_string()),
        pmid: record.pmid.clone(),
        pmcid: record.pmcid.clone(),
        title: (!record.title.is_empty()).then(|| record.title.clone()),
        authors: record.authors.clone(),
        year: record.year,
        venue: record.venue.clone(),
        source: record.source.clone(),
        method: method.to_string(),
        score,
    }
}

/// How a batch of entries resolved.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Outcome {
    pub entries: usize,
    pub resolved: usize,
    /// A record was found but disagreed with the printed entry.
    pub rejected: usize,
    pub unresolved: usize,
    /// Entries with at least one failed registry request, including enrichment.
    /// An error can coexist with a successful resolution through another source.
    pub errors: usize,
    pub by_method: BTreeMap<String, usize>,
}

/// Crossref `/works?query.bibliographic=…`: the query field meant for whole
/// citation strings (author, title, venue and year weighed together), unlike
/// the plain `query`.
fn bibliographic_search(
    client: &Client,
    text: &str,
    rows: u32,
) -> Result<Vec<tpe_biblio::Found>, BiblioError> {
    let n = rows.to_string();
    let mut pairs: Vec<(&str, &str)> = vec![("query.bibliographic", text), ("rows", n.as_str())];
    if let Some(m) = client.mailto() {
        pairs.push(("mailto", m));
    }
    let url = with_query(&format!("{}/works", crossref::BASE), &pairs);
    crossref::parse_crossref_found(&client.get_text(&url, &[])?)
}

/// Does a meaningful word of the record's venue (3+ chars, `RNA`, `Lancet`)
/// appear in the entry? Function words such as "and" in a title are not
/// journal evidence. A word already supplied by the candidate title is not
/// independent venue evidence either. Venue names help distinguish articles
/// from preprints or posters when title, authors and year all agree.
fn venue_in_entry(venue: &str, raw_folded: &str, title: &str) -> bool {
    let needles = words(&folded(venue), 3);
    if needles.is_empty() {
        return false;
    }
    let hay = words(raw_folded, 1);
    let title_words = words(&folded(title), 1);
    needles.iter().any(|n| {
        !matches!(
            n.as_str(),
            "and" | "the" | "for" | "with" | "from" | "into" | "via"
        ) && !title_words.iter().any(|word| word == n)
            && hay.iter().any(|h| h == n)
    })
}

/// Minimum winning margin between distinct query candidates. Search order is
/// not identity evidence; unresolved ambiguity is preferable to a false edge.
const QUERY_MARGIN: f32 = 0.05;

fn select_query_record(
    entry: &mut ReferenceEntry,
    records: impl IntoIterator<Item = PaperRecord>,
) -> Option<Resolved> {
    let raw = folded(reference_text(&entry.raw));
    let mut candidates: Vec<(f32, Resolved)> = Vec::new();
    for record in records {
        let Some(doi) = record.doi.as_deref().and_then(normalize_doi) else {
            continue;
        };
        // Author/year alone cannot establish a search result's identity (e.g.
        // two works by Smith in 2020). Exact-ID lookups have separate evidence.
        let checked = if title_overlap(&record.title, &raw).is_none() {
            Err("title: insufficient title evidence for a bibliographic query".to_string())
        } else {
            verify(&record, entry)
        };
        match checked {
            Ok(score) => {
                let venue_bonus = if record
                    .venue
                    .as_deref()
                    .is_some_and(|v| venue_in_entry(v, &raw, &record.title))
                {
                    0.5
                } else {
                    0.0
                };
                let total = score + venue_bonus;
                entry.attempts.push(Attempt {
                    method: "query".to_string(),
                    doi: Some(doi.clone()),
                    outcome: "candidate".to_string(),
                    detail: Some(format!("metadata agrees, ranking score {total:.2}")),
                });
                candidates.push((total, resolved_from(&record, &doi, "query", score)));
            }
            Err(detail) => entry.attempts.push(Attempt {
                method: "query".to_string(),
                doi: Some(doi),
                outcome: "mismatch".to_string(),
                detail: Some(detail),
            }),
        }
    }
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (score, best) = candidates.first()?;
    if candidates
        .iter()
        .any(|(other, record)| record.doi != best.doi && score - other < QUERY_MARGIN)
    {
        entry.attempts.push(Attempt {
            method: "query".to_string(),
            doi: None,
            outcome: "ambiguous".to_string(),
            detail: Some("distinct DOIs have indistinguishable bibliographic evidence".to_string()),
        });
        return None;
    }
    let best = best.clone();
    entry.attempts.push(Attempt {
        method: "query".to_string(),
        doi: best.doi.clone(),
        outcome: "verified".to_string(),
        detail: None,
    });
    Some(best)
}

/// Prefer the parser's wrap repair, except when raw text extends that same DOI
/// (the parser can trim balanced suffix punctuation).
fn printed_doi(entry: &ReferenceEntry) -> Option<String> {
    let parsed = entry.doi.as_deref().and_then(doi_in);
    let raw = doi_in(reference_text(&entry.raw));
    match (parsed, raw) {
        (Some(parsed), Some(raw)) if raw.starts_with(&parsed) => Some(raw),
        (Some(parsed), _) => Some(parsed),
        (None, raw) => raw,
    }
}

fn exact_paper_record(title: &str, doi: &str, record: &PaperRecord) -> Option<Resolved> {
    if record.doi.as_deref().and_then(normalize_doi) != normalize_doi(doi) {
        return None;
    }
    let score = if title.is_empty() {
        1.0
    } else {
        title_agreement(title, &record.title)
    };
    (score >= PAPER_TITLE_MIN).then(|| resolved_from(record, doi, "metadata", score))
}

fn select_paper_record(
    title: &str,
    records: impl IntoIterator<Item = PaperRecord>,
) -> Option<Resolved> {
    let mut candidates: Vec<(f32, Resolved)> = records
        .into_iter()
        .filter_map(|record| {
            let doi = record.doi.as_deref().and_then(normalize_doi)?;
            let score = title_agreement(title, &record.title);
            (score >= PAPER_TITLE_MIN)
                .then(|| (score, resolved_from(&record, &doi, "query", score)))
        })
        .collect();
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (score, best) = candidates.first()?;
    if candidates
        .iter()
        .any(|(other, record)| record.doi != best.doi && score - other < QUERY_MARGIN)
    {
        return None;
    }
    Some(best.clone())
}

/// Count unresolved metadata disagreements, excluding candidates withheld only
/// because identity evidence is ambiguous. Inspect only this resolution pass.
fn count_rejection(attempts: &[Attempt], outcome: &mut Outcome) {
    if attempts.iter().any(|a| a.outcome == "mismatch")
        && !attempts.iter().any(|a| a.outcome == "ambiguous")
    {
        outcome.rejected += 1;
    }
}

/// The retry policy: `RETRIES` attempts after a rate limit, a server
/// `Retry-After`, a transport failure or a 5xx answer, backing off
/// `RETRY_BASE`, `2 x RETRY_BASE`, ... and honouring `Retry-After` up to
/// `RETRY_MAX_WAIT`.
const fn retry_policy() -> RetryPolicy {
    RetryPolicy {
        retries: RETRIES,
        base: RETRY_BASE,
        max_wait: RETRY_MAX_WAIT,
    }
}

/// Longest wait honoured for one retry (backoff or `Retry-After`).
const RETRY_MAX_WAIT: Duration = Duration::from_secs(60);

/// Run `request` under [`retry_policy`].
fn with_retry<T>(request: impl FnMut() -> Result<T, BiblioError>) -> Result<T, BiblioError> {
    retry_policy().run(request, std::thread::sleep)
}

/// Explicit identifiers in the reference text. Bare numbers are not PMIDs.
fn biomedical_ids(text: &str) -> Vec<pmc::Identifier> {
    static PMID: OnceLock<Regex> = OnceLock::new();
    static PMCID: OnceLock<Regex> = OnceLock::new();
    let text = reference_text(text);
    let pubmed_pattern = PMID.get_or_init(|| Regex::new(
        r"(?i)\b(?:PMID\s*:\s*|pubmed\.ncbi\.nlm\.nih\.gov/|ncbi\.nlm\.nih\.gov/pubmed/)([1-9]\d{0,11})\b"
    ).expect("valid regex"));
    let central_pattern =
        PMCID.get_or_init(|| Regex::new(r"(?i)\bPMC([1-9]\d{0,11})\b").expect("valid regex"));
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    for found in pubmed_pattern.captures_iter(text) {
        let id = pmc::Identifier::Pmid(found[1].to_string());
        if seen.insert(id.clone()) {
            ids.push(id);
        }
    }
    for found in central_pattern.captures_iter(text) {
        let id = pmc::Identifier::Pmcid(format!("PMC{}", &found[1]));
        if seen.insert(id.clone()) {
            ids.push(id);
        }
    }
    ids
}

/// Accept one exact registry identity only if its metadata agrees with the
/// printed reference. Distinct matching biomedical records remain ambiguous.
fn select_biomedical_record(
    entry: &mut ReferenceEntry,
    id: &pmc::Identifier,
    records: impl IntoIterator<Item = PaperRecord>,
) -> Option<Resolved> {
    let mut accepted: Vec<Resolved> = Vec::new();
    for record in records {
        if !id.matches(&record) {
            entry.attempts.push(Attempt {
                method: "europepmc".to_string(),
                doi: record.doi.clone(),
                outcome: "mismatch".to_string(),
                detail: Some(
                    "lookup returned a different or missing biomedical identifier".to_string(),
                ),
            });
            continue;
        }
        if biomedical_ids(&entry.raw)
            .iter()
            .any(|printed| !printed.matches(&record))
        {
            entry.attempts.push(Attempt {
                method: "europepmc".to_string(),
                doi: record.doi.clone(),
                outcome: "mismatch".to_string(),
                detail: Some("printed PMID and PMCID do not identify the same record".to_string()),
            });
            continue;
        }
        match verify(&record, entry) {
            Ok(score) => {
                let resolved = resolved_from(
                    &record,
                    record.doi.as_deref().unwrap_or(""),
                    "europepmc",
                    score,
                );
                if !accepted.iter().any(|r| {
                    r.doi == resolved.doi && r.pmid == resolved.pmid && r.pmcid == resolved.pmcid
                }) {
                    accepted.push(resolved);
                }
            }
            Err(detail) => entry.attempts.push(Attempt {
                method: "europepmc".to_string(),
                doi: record.doi.clone(),
                outcome: "mismatch".to_string(),
                detail: Some(detail),
            }),
        }
    }
    let outcome = match accepted.len() {
        0 => "not_found",
        1 => "verified",
        _ => "ambiguous",
    };
    entry.attempts.push(Attempt {
        method: "europepmc".to_string(),
        doi: accepted.first().and_then(|r| r.doi.clone()),
        outcome: outcome.to_string(),
        detail: Some(format!("exact lookup {id:?}")),
    });
    if accepted.len() == 1 {
        accepted.pop()
    } else {
        None
    }
}

/// Explicit biomedical IDs are authoritative. If none can verify all printed
/// IDs, leave the entry unresolved rather than accepting a DOI/query fallback.
/// Returns whether explicit IDs were present (and therefore handled).
fn resolve_explicit_biomedical(
    entry: &mut ReferenceEntry,
    mut lookup: impl FnMut(&mut ReferenceEntry, &pmc::Identifier) -> Option<Resolved>,
) -> bool {
    let ids = biomedical_ids(&entry.raw);
    if ids.is_empty() {
        return false;
    }
    for id in ids {
        if let Some(resolved) = lookup(entry, &id) {
            entry.resolved = Some(resolved);
            break;
        }
    }
    true
}

/// A registry resolver (Crossref, Europe PMC, `PubMed`, `DataCite`,
/// `doi.org`) with the polite-pool rate limit, an optional on-disk response
/// cache and retries that honour `Retry-After`. Network use is opt-in: the
/// resolver only exists when the caller asks for resolution.
pub struct Resolver {
    client: Client,
    cache: Option<DiskCache>,
}

/// Everything learned about the paper itself in one resolution pass.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaperResolution {
    /// The accepted record, verified against the printed metadata.
    pub resolved: Option<Resolved>,
    /// `resolved`, `mismatch` (records came back but disagreed), `ambiguous`,
    /// `not_found`, `unresolved: offline` (or another reason no registry
    /// could be asked), or `unresolved: no identifier or title`.
    pub status: String,
    /// Every lookup and its outcome, in order.
    pub attempts: Vec<Attempt>,
    /// The identifiers the paper carries, with their provenance.
    pub identifiers: PaperIdentifiers,
    /// Publisher, licence, funder and ISSN facts for the accepted DOI.
    pub publisher: Option<PublisherMetadata>,
}

impl Resolver {
    /// A resolver identifying as `tpe` with `mailto` in the `User-Agent` and
    /// the query string for Crossref's polite pool. No cache.
    #[must_use]
    pub fn new(mailto: Option<&str>) -> Self {
        let client = Client::polite("tpe", env!("CARGO_PKG_VERSION"), mailto)
            .with_host_interval(CROSSREF_HOST, CROSSREF_INTERVAL);
        Self {
            client,
            cache: None,
        }
    }

    /// A resolver configured from the environment: the contact address from
    /// `TPE_MAILTO` (never hard-coded) and the response cache from
    /// `TPE_BIBLIO_CACHE_DIR` (no cache when unset).
    #[must_use]
    pub fn from_env() -> Self {
        let mailto = tpe_biblio::mailto_from_env();
        let resolver = Self::new(mailto.as_deref());
        match tpe_biblio::cache_dir_from_env() {
            Some(dir) => resolver.with_cache_dir(&dir),
            None => resolver,
        }
    }

    /// Cache registry responses under `dir` for [`tpe_biblio::cache::DEFAULT_TTL`].
    #[must_use]
    pub fn with_cache_dir(mut self, dir: &Path) -> Self {
        self.cache = Some(DiskCache::new(dir, tpe_biblio::cache::DEFAULT_TTL));
        self
    }

    /// Enable or disable offline mode: every uncached request fails with
    /// `offline` and nothing is sent.
    #[must_use]
    pub fn with_offline(mut self, offline: bool) -> Self {
        self.client = self.client.with_offline(offline);
        self
    }

    /// The cached, retrying fetcher the exact-identifier lookups go through.
    fn fetcher(&self) -> Fetcher<'_> {
        Fetcher::new(&self.client)
            .with_cache(self.cache.as_ref())
            .with_policy(retry_policy())
    }

    fn try_biomedical(&self, entry: &mut ReferenceEntry, id: &pmc::Identifier) -> Option<Resolved> {
        match with_retry(|| pmc::fetch_identifier(&self.client, id)) {
            Ok(found) => select_biomedical_record(entry, id, found.into_iter().map(|f| f.record)),
            Err(err) => {
                entry.attempts.push(Attempt {
                    method: "europepmc".to_string(),
                    doi: None,
                    outcome: "error".to_string(),
                    detail: Some(format!("{id:?}: {err}")),
                });
                None
            }
        }
    }

    /// Enrich an already verified DOI with exact biomedical cross-identifiers.
    /// Failure leaves the accepted Crossref record intact and logs the attempt.
    fn enrich_identifiers(&self, entry: &mut ReferenceEntry) {
        let Some(resolved) = &entry.resolved else {
            return;
        };
        if resolved.pmid.is_some() || resolved.pmcid.is_some() {
            return;
        }
        let Some(doi) = resolved.doi.clone() else {
            return;
        };
        if let Some(extra) = self.try_biomedical(entry, &pmc::Identifier::Doi(doi))
            && let Some(resolved) = &mut entry.resolved
        {
            resolved.pmid = extra.pmid;
            resolved.pmcid = extra.pmcid;
        }
    }

    /// Fetch and verify one DOI for `entry`, logging the attempt.
    fn try_doi(
        &self,
        entry: &mut ReferenceEntry,
        doi: &str,
        method: &str,
    ) -> Result<Option<Resolved>, BiblioError> {
        let mut attempt = Attempt {
            method: method.to_string(),
            doi: Some(doi.to_string()),
            outcome: String::new(),
            detail: None,
        };
        let found = match crossref::fetch_work_cached(&self.fetcher(), doi) {
            Ok(found) => found.map(|work| work.to_record()),
            Err(err) => {
                attempt.outcome = "error".to_string();
                attempt.detail = Some(err.to_string());
                entry.attempts.push(attempt);
                return Err(err);
            }
        };
        let Some(record) = found else {
            attempt.outcome = "not_found".to_string();
            entry.attempts.push(attempt);
            return Ok(None);
        };
        if record.doi.as_deref().and_then(normalize_doi).as_deref() != Some(doi) {
            attempt.outcome = "mismatch".to_string();
            attempt.detail = Some("lookup returned a different or missing DOI".to_string());
            entry.attempts.push(attempt);
            return Ok(None);
        }
        let outcome = match verify(&record, entry) {
            Ok(score) => {
                attempt.outcome = "verified".to_string();
                Some(resolved_from(&record, doi, method, score))
            }
            Err(detail) => {
                attempt.outcome = "mismatch".to_string();
                attempt.detail = Some(detail);
                None
            }
        };
        entry.attempts.push(attempt);
        Ok(outcome)
    }

    /// Resolve one entry in place; returns the method that succeeded.
    fn resolve_entry(
        &self,
        entry: &mut ReferenceEntry,
    ) -> Result<Option<&'static str>, BiblioError> {
        if reject_ambiguous_printed_dois(entry) {
            return Ok(None);
        }
        if resolve_explicit_biomedical(entry, |entry, id| self.try_biomedical(entry, id)) {
            return Ok(entry.resolved.as_ref().map(|_| "europepmc"));
        }
        let mut candidates: Vec<(String, &'static str)> = Vec::new();
        if let Some(doi) = entry.doi_link.as_deref().and_then(doi_in) {
            candidates.push((doi, "link"));
        }
        if let Some(doi) = printed_doi(entry)
            && !candidates.iter().any(|(d, _)| *d == doi)
        {
            candidates.push((doi, "printed"));
        }
        for (doi, method) in &candidates {
            // Keep logged errors; another registry may still resolve it.
            if let Ok(Some(resolved)) = self.try_doi(entry, doi, method) {
                entry.resolved = Some(resolved);
                return Ok(Some(method));
            }
        }
        for (doi, _) in &candidates {
            if let Some(resolved) = self.try_biomedical(entry, &pmc::Identifier::Doi(doi.clone())) {
                entry.resolved = Some(resolved);
                return Ok(Some("europepmc"));
            }
        }
        let query: String = reference_text(&entry.raw)
            .chars()
            .take(QUERY_CHARS)
            .collect();
        let found = match with_retry(|| bibliographic_search(&self.client, &query, QUERY_ROWS)) {
            Ok(found) => found,
            Err(err) => {
                entry.attempts.push(Attempt {
                    method: "query".to_string(),
                    doi: None,
                    outcome: "error".to_string(),
                    detail: Some(err.to_string()),
                });
                return Err(err);
            }
        };
        if found.is_empty() {
            entry.attempts.push(Attempt {
                method: "query".to_string(),
                doi: None,
                outcome: "not_found".to_string(),
                detail: None,
            });
        }
        if let Some(resolved) = select_query_record(entry, found.into_iter().map(|f| f.record)) {
            entry.resolved = Some(resolved);
            return Ok(Some("query"));
        }
        Ok(None)
    }

    /// Resolve every entry that has no record yet. Errors are counted, not
    /// returned: a failed request leaves that entry unresolved.
    pub fn resolve_entries(&self, entries: &mut [ReferenceEntry]) -> Outcome {
        let mut outcome = Outcome {
            entries: entries.len(),
            ..Outcome::default()
        };
        for entry in entries.iter_mut() {
            let first_attempt = entry.attempts.len();
            if entry.resolved.is_some() {
                self.enrich_identifiers(entry);
                outcome.resolved += 1;
            } else {
                match self.resolve_entry(entry) {
                    Ok(Some(method)) => {
                        self.enrich_identifiers(entry);
                        outcome.resolved += 1;
                        *outcome.by_method.entry(method.to_string()).or_insert(0) += 1;
                    }
                    Ok(None) => outcome.unresolved += 1,
                    Err(_) => {
                        outcome.unresolved += 1;
                    }
                }
            }
            if entry.resolved.is_none() {
                count_rejection(&entry.attempts[first_attempt..], &mut outcome);
            }
            if entry.attempts[first_attempt..]
                .iter()
                .any(|a| a.outcome == "error")
            {
                outcome.errors += 1;
            }
        }
        outcome
    }

    /// The paper's own record: its metadata DOI when it verifies against
    /// the metadata title, otherwise its printed PMID verified the same way,
    /// otherwise a query on the title verified by title agreement of at
    /// least [`PAPER_TITLE_MIN`]. See [`Self::resolve_paper_report`] for
    /// the attempts and the reason when nothing is accepted.
    #[must_use]
    pub fn resolve_paper(&self, meta: &Metadata) -> Option<Resolved> {
        self.resolve_paper_report(meta, &[]).resolved
    }

    /// Resolve the paper itself and say how. `pages` (page 1 is enough)
    /// supplies printed PMID, PMCID and ISSN values; `&[]` uses only the
    /// metadata and the `/Info` strings.
    ///
    /// 1. The DOI from the metadata is looked up (Crossref, then `DataCite`,
    ///    then `doi.org`); the record must carry that DOI, its title must
    ///    agree with the metadata title ([`PAPER_TITLE_MIN`]) and, when the
    ///    paper prints a labelled ISSN, the record's ISSNs must include it.
    /// 2. A printed PMID is looked up in `PubMed`; the summary must carry
    ///    that PMID and agree on title (and first author and year when both
    ///    sides have them).
    /// 3. A title of 12+ characters is searched on Crossref and accepted
    ///    only when one record agrees clearly better than any other.
    ///
    /// A registry that could not be asked leaves the status `unresolved:
    /// <reason>` rather than `not_found`.
    #[must_use]
    pub fn resolve_paper_report(&self, meta: &Metadata, pages: &[PageText]) -> PaperResolution {
        let identifiers = extract_identifiers(meta, pages);
        let title = meta.title.as_deref().unwrap_or("");
        let mut report = PaperResolution {
            identifiers,
            status: "unresolved: no identifier or title".to_string(),
            ..PaperResolution::default()
        };
        let fetcher = self.fetcher();

        if let Some(doi) = report.identifiers.doi.clone() {
            let mut attempt = Attempt {
                method: "metadata".to_string(),
                doi: Some(doi.clone()),
                outcome: String::new(),
                detail: None,
            };
            match doi_metadata::resolve_doi(&fetcher, &doi) {
                Lookup::Found(found) => {
                    match verify_paper_doi(title, &doi, &found, &report.identifiers.issns) {
                        Ok(resolved) => {
                            attempt.outcome = "verified".to_string();
                            attempt.detail = Some(format!("{} record", found.source));
                            report.attempts.push(attempt);
                            report.publisher = Some(found.publisher);
                            report.resolved = Some(resolved);
                            report.status = "resolved".to_string();
                            return report;
                        }
                        Err(detail) => {
                            attempt.outcome = "mismatch".to_string();
                            attempt.detail = Some(detail);
                            report.status = "mismatch".to_string();
                        }
                    }
                }
                Lookup::NotFound => {
                    attempt.outcome = "not_found".to_string();
                    report.status = "not_found".to_string();
                }
                Lookup::Unresolved(reason) => {
                    attempt.outcome = "error".to_string();
                    attempt.detail = Some(reason.clone());
                    report.status = format!("unresolved: {reason}");
                }
            }
            report.attempts.push(attempt);
        }

        if let Some(pmid) = report.identifiers.pmid.clone() {
            let mut attempt = Attempt {
                method: "pmid".to_string(),
                doi: None,
                outcome: String::new(),
                detail: Some(format!("PMID {pmid}")),
            };
            match pubmed::resolve_pmid(&fetcher, &pmid) {
                Lookup::Found(summary) if summary.pmid == pmid => {
                    let record = summary.to_record();
                    match verify_paper_record(meta, &record) {
                        Ok(score) => {
                            attempt.outcome = "verified".to_string();
                            attempt.doi.clone_from(&record.doi);
                            report.attempts.push(attempt);
                            report.resolved = Some(resolved_from(
                                &record,
                                record.doi.as_deref().unwrap_or(""),
                                "pmid",
                                score,
                            ));
                            report.status = "resolved".to_string();
                            if let Some(doi) = record.doi.as_deref()
                                && let Lookup::Found(found) =
                                    doi_metadata::resolve_doi(&fetcher, doi)
                            {
                                report.publisher = Some(found.publisher);
                            }
                            return report;
                        }
                        Err(detail) => {
                            attempt.outcome = "mismatch".to_string();
                            attempt.detail = Some(detail);
                            if !report.status.starts_with("unresolved") {
                                report.status = "mismatch".to_string();
                            }
                        }
                    }
                }
                Lookup::Found(_) | Lookup::NotFound => {
                    attempt.outcome = "not_found".to_string();
                    if report.status.starts_with("unresolved: no identifier") {
                        report.status = "not_found".to_string();
                    }
                }
                Lookup::Unresolved(reason) => {
                    attempt.outcome = "error".to_string();
                    attempt.detail = Some(reason.clone());
                    report.status = format!("unresolved: {reason}");
                }
            }
            report.attempts.push(attempt);
        }

        if title.len() < 12 {
            return report;
        }
        let mut attempt = Attempt {
            method: "query".to_string(),
            doi: None,
            outcome: String::new(),
            detail: None,
        };
        match with_retry(|| bibliographic_search(&self.client, title, QUERY_ROWS)) {
            Ok(found) => {
                let records: Vec<PaperRecord> = found.into_iter().map(|f| f.record).collect();
                let compatible = records
                    .iter()
                    .filter(|r| title_agreement(title, &r.title) >= PAPER_TITLE_MIN)
                    .count();
                match select_paper_record(title, records) {
                    Some(resolved) => {
                        attempt.outcome = "verified".to_string();
                        attempt.doi.clone_from(&resolved.doi);
                        report.attempts.push(attempt);
                        if let Some(doi) = resolved.doi.as_deref()
                            && let Lookup::Found(found) = doi_metadata::resolve_doi(&fetcher, doi)
                        {
                            report.publisher = Some(found.publisher);
                        }
                        report.resolved = Some(resolved);
                        report.status = "resolved".to_string();
                        return report;
                    }
                    None if compatible > 1 => {
                        attempt.outcome = "ambiguous".to_string();
                        attempt.detail =
                            Some("distinct DOIs agree with the title equally well".to_string());
                        if !report.status.starts_with("unresolved") {
                            report.status = "ambiguous".to_string();
                        }
                    }
                    None => {
                        attempt.outcome = "not_found".to_string();
                        if report.status.starts_with("unresolved: no identifier") {
                            report.status = "not_found".to_string();
                        }
                    }
                }
            }
            Err(err) => {
                let reason = tpe_biblio::lookup::unresolved_reason(&err);
                attempt.outcome = "error".to_string();
                attempt.detail = Some(reason.clone());
                report.status = format!("unresolved: {reason}");
            }
        }
        report.attempts.push(attempt);
        report
    }
}

/// Accept a DOI record for the paper only when it carries the requested
/// DOI, its title agrees with the metadata title and no labelled printed
/// ISSN contradicts the record's ISSNs.
fn verify_paper_doi(
    title: &str,
    doi: &str,
    found: &doi_metadata::DoiMetadata,
    printed_issns: &[String],
) -> Result<Resolved, String> {
    let Some(resolved) = exact_paper_record(title, doi, &found.record) else {
        let same_doi = found
            .record
            .doi
            .as_deref()
            .and_then(normalize_doi)
            .as_deref()
            == Some(doi);
        return Err(if same_doi {
            format!(
                "title: record {:?} agrees {:.0}% with the metadata title {title:?}",
                found.record.title,
                title_agreement(title, &found.record.title) * 100.0
            )
        } else {
            "lookup returned a different or missing DOI".to_string()
        });
    };
    if !printed_issns.is_empty()
        && !found.publisher.issn.is_empty()
        && !printed_issns
            .iter()
            .any(|p| found.publisher.issn.contains(p))
    {
        return Err(format!(
            "issn: printed {} not among the record's {}",
            printed_issns.join(", "),
            found.publisher.issn.join(", ")
        ));
    }
    Ok(resolved)
}

/// Verify a record found by identifier against the paper's metadata: the
/// title must agree ([`PAPER_TITLE_MIN`]); when both sides name a first
/// author its family name must appear in the printed name; when both have a
/// year they must be within one. With no title on either side there is
/// nothing to compare.
fn verify_paper_record(meta: &Metadata, record: &PaperRecord) -> Result<f32, String> {
    let title = meta.title.as_deref().unwrap_or("");
    if title.is_empty() || record.title.is_empty() {
        return Err("nothing to compare: no title on one side".to_string());
    }
    let score = title_agreement(title, &record.title);
    if score < PAPER_TITLE_MIN {
        return Err(format!(
            "title: record {:?} agrees {:.0}% with the metadata title {title:?}",
            record.title,
            score * 100.0
        ));
    }
    if let (Some(printed), Some(first)) = (meta.authors.first(), record.authors.first()) {
        let family = family_of(first);
        if !family_in_entry(&family, &folded(&printed.name)) {
            return Err(format!(
                "first author: record {first:?} ({family}) is not the printed {:?}",
                printed.name
            ));
        }
    }
    if let (Some(printed), Some(year)) = (meta.year, record.year)
        && printed.abs_diff(year) > 1
    {
        return Err(format!("year: record {year} is not the printed {printed}"));
    }
    Ok(score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{BBox, Link};

    #[test]
    fn doi_strings_are_cut_at_sentence_punctuation_and_lower_cased() {
        assert_eq!(
            doi_in("https://doi.org/10.1000/ABC.123."),
            Some("10.1000/abc.123".to_string())
        );
        assert_eq!(
            doi_in("doi: 10.1016/j.cell.2020.01.001)"),
            Some("10.1016/j.cell.2020.01.001".to_string())
        );
        assert_eq!(doi_in("no doi here"), None);
    }

    #[test]
    fn doi_lookup_keeps_balanced_biomedical_suffixes() {
        for doi in [
            "10.1016/S0140-6736(20)30183-5",
            "10.1002/(SICI)1097-0258(19980815)17:15<1741::AID-SIM868>3.0.CO;2-8",
            "10.1000/example(abc)",
        ] {
            assert_eq!(
                doi_in(&format!("doi: {doi}).")),
                Some(doi.to_ascii_lowercase())
            );
        }
        assert_eq!(
            doi_in("[doi:10.1000/example]."),
            Some("10.1000/example".to_string())
        );
    }

    fn query_record(doi: &str) -> PaperRecord {
        PaperRecord {
            doi: Some(doi.to_string()),
            title: "Molecular mechanisms of inflammation".to_string(),
            authors: vec!["Jane Smith".to_string()],
            year: Some(2020),
            source: "crossref".to_string(),
            ..PaperRecord::default()
        }
    }

    fn query_entry() -> ReferenceEntry {
        ReferenceEntry {
            raw: "Smith J. Molecular mechanisms of inflammation. Immunology 2020; 1:2-3."
                .to_string(),
            ..ReferenceEntry::default()
        }
    }

    #[test]
    fn query_ambiguity_is_not_decided_by_response_order() {
        let a = query_record("10.1000/a");
        let b = query_record("10.1000/b");
        for records in [[a.clone(), b.clone()], [b, a]] {
            let mut entry = query_entry();
            assert!(select_query_record(&mut entry, records).is_none());
            assert_eq!(entry.attempts.last().unwrap().outcome, "ambiguous");
            assert!(!entry.attempts.iter().any(|a| a.outcome == "verified"));
        }
    }

    #[test]
    fn query_needs_title_evidence_and_deduplicates_identical_dois() {
        let mut incomplete = query_record("10.1000/missing-title");
        incomplete.title.clear();
        let mut entry = query_entry();
        assert!(select_query_record(&mut entry, [incomplete]).is_none());
        let exact = query_record("10.1000/a");
        let selected = select_query_record(&mut entry, [exact.clone(), exact]).unwrap();
        assert_eq!(selected.doi.as_deref(), Some("10.1000/a"));
        assert_eq!(entry.attempts.last().unwrap().outcome, "verified");
    }

    #[test]
    fn query_uses_venue_evidence_to_distinguish_versions() {
        for venue in ["Immunology", "RNA", "The Lancet"] {
            let preprint = query_record("10.1000/preprint");
            let mut article = query_record("10.1000/article");
            article.venue = Some(venue.to_string());
            let mut entry = query_entry();
            entry.raw = entry.raw.replace("Immunology", venue);
            let selected = select_query_record(&mut entry, [preprint, article]).unwrap();
            assert_eq!(selected.doi.as_deref(), Some("10.1000/article"));
        }
    }

    #[test]
    fn venue_function_word_does_not_outrank_a_better_title_match() {
        // PMC11033918 reference 9, independently labeled by its JATS DOI.
        // The live registry returned a related conference abstract whose venue
        // shares only "and" with the citation. That is not journal evidence.
        let article = PaperRecord {
            doi: Some("10.1200/op.20.00266".to_string()),
            title: "Restricted mouth opening in head and neck cancer: etiology, prevention, and treatment".to_string(),
            authors: vec!["W. Abboud".to_string()],
            year: Some(2020),
            venue: Some("JCO Oncology Practice".to_string()),
            ..PaperRecord::default()
        };
        let abstract_record = PaperRecord {
            doi: Some("10.1016/j.ijom.2019.03.511".to_string()),
            title: "Reduced mouth opening in head and neck cancer patients".to_string(),
            authors: vec!["W. Abboud".to_string()],
            year: Some(2019),
            venue: Some("International Journal of Oral and Maxillofacial Surgery".to_string()),
            ..PaperRecord::default()
        };
        for records in [
            [article.clone(), abstract_record.clone()],
            [abstract_record, article],
        ] {
            let mut entry = ReferenceEntry {
                raw: "Abboud. Restricted mouth opening in head and neck cancer: etiology, prevention, and treatment. 2020.".to_string(),
                ..ReferenceEntry::default()
            };
            let selected = select_query_record(&mut entry, records).unwrap();
            assert_eq!(selected.doi.as_deref(), Some("10.1200/op.20.00266"));
        }
    }

    #[test]
    fn venue_function_word_does_not_remove_query_ambiguity() {
        let mut a = query_record("10.1000/a");
        a.title = "Molecular mechanisms for inflammation".to_string();
        let mut b = a.clone();
        b.doi = Some("10.1000/b".to_string());
        b.venue = Some("Journal for Biomedical Research".to_string());
        for records in [[a.clone(), b.clone()], [b, a]] {
            let mut entry = ReferenceEntry {
                raw: "Smith J. Molecular mechanisms for inflammation. 2020.".to_string(),
                ..ReferenceEntry::default()
            };
            assert!(select_query_record(&mut entry, records).is_none());
            assert_eq!(entry.attempts.last().unwrap().outcome, "ambiguous");
        }
    }

    #[test]
    fn venue_word_in_candidate_title_does_not_remove_query_ambiguity() {
        // A merged input from PMC9866640:33 and PMC3777682:64 contains both
        // complete titles. "Animal" is already title evidence for Rossiter;
        // it must not also count as independent journal evidence.
        let animal = PaperRecord {
            doi: Some("10.1007/s11250-008-9266-7".to_string()),
            title: "Living with transboundary animal diseases (TADs)".to_string(),
            authors: vec!["Paul B. Rossiter".to_string()],
            year: Some(2008),
            venue: Some("Tropical Animal Health and Production".to_string()),
            ..PaperRecord::default()
        };
        let visual = PaperRecord {
            doi: Some("10.1017/s1355617711000981".to_string()),
            title: "Impaired visual scanning and memory for faces in high-functioning autism spectrum disorders: it's not just the eyes".to_string(),
            authors: vec!["J. Snow".to_string()],
            year: Some(2011),
            venue: Some("Journal of the International Neuropsychological Society".to_string()),
            ..PaperRecord::default()
        };
        for records in [[animal.clone(), visual.clone()], [visual, animal]] {
            let mut entry = ReferenceEntry {
                raw: "Rossiter. Living with transboundary animal diseases (TADs). 2009. / Snow. Impaired visual scanning and memory for faces in high-functioning autism spectrum disorders: it's not just the eyes. 2011.".to_string(),
                ..ReferenceEntry::default()
            };
            assert!(select_query_record(&mut entry, records).is_none());
            assert_eq!(entry.attempts.last().unwrap().outcome, "ambiguous");
        }
    }

    #[test]
    fn merged_wrapped_dois_are_ambiguous_before_any_registry_request() {
        // Unchanged extraction from natural:PMC12745427:4 in the independent
        // 384-case cohort: part of Bendau's entry is joined to Biddle's entry.
        // The live baseline incorrectly accepted Biddle after rejecting Bendau.
        let raw = "Sport bei depressiven Erkrankungen. NeuroTransmitter 33, 52–61. doi: 10.1007/ s15016-021-9343-y Biddle, S. J. H., and Asare, M. (2011). Physical activity and mental health in children and adolescents: a review of reviews. Br. J. Sports Med. 45, 886–895. doi: 10.1136/ bjsports-2011-090185 Bosnak-Guclu, M., Arikan, H., Savci, S., Inal-Ince, D., Tulumen, E., Aytemir, K., et al.";
        let mut entries = [ReferenceEntry {
            raw: raw.to_string(),
            doi: Some("10.1007/s15016-021-9343-y".to_string()),
            doi_link: Some("10.1007/s15016-021-9343-y".to_string()),
            ..ReferenceEntry::default()
        }];
        let resolver = Resolver::new(None).with_offline(true);
        let outcome = resolver.resolve_entries(&mut entries);
        assert_eq!(outcome.unresolved, 1);
        assert_eq!(
            outcome.errors, 0,
            "ambiguity must not need a network request"
        );
        assert_eq!(outcome.rejected, 0);
        assert!(entries[0].resolved.is_none());
        assert_eq!(entries[0].attempts.len(), 1);
        assert_eq!(entries[0].attempts[0].outcome, "ambiguous");
        assert_eq!(entries[0].raw, raw);
    }

    #[test]
    fn repeated_doi_forms_do_not_create_ambiguity() {
        for raw in [
            "Smith. Molecular mechanisms of inflammation. 2020. doi:10.1000/ABC. https://doi.org/10.1000/abc",
            "Smith. Molecular mechanisms of inflammation. 2020. doi:10.1000/ ABC; doi:10.1000/abc",
            "Smith. Molecular mechanisms of inflammation. 2020. 10.1000/ABC 10.1000/abc",
            "Smith. Molecular mechanisms of inflammation. 2020. https://doi.org/10.1000/abc def2; doi:10.1000/abcdef2",
        ] {
            let mut entry = ReferenceEntry {
                raw: raw.to_string(),
                ..ReferenceEntry::default()
            };
            assert!(!reject_ambiguous_printed_dois(&mut entry));
            assert!(entry.attempts.is_empty());
        }
        let mut entry = query_entry();
        entry.raw.push_str(" 10.1000/a 10.1000/b");
        assert!(reject_ambiguous_printed_dois(&mut entry));
    }

    #[test]
    fn broken_doi_does_not_hide_later_conflicting_identifiers() {
        let mut entries = [ReferenceEntry {
            raw: "truncated doi: 10.1000/ ; Smith. Molecular mechanisms of inflammation. 2020. doi:10.1000/a Jones. Another complete citation. 2021. doi:10.1000/b".to_string(),
            ..ReferenceEntry::default()
        }];
        let resolver = Resolver::new(None).with_offline(true);
        let outcome = resolver.resolve_entries(&mut entries);
        assert_eq!(outcome.unresolved, 1);
        assert_eq!(outcome.errors, 0);
        assert_eq!(outcome.rejected, 0);
        assert_eq!(entries[0].attempts.len(), 1);
        assert_eq!(entries[0].attempts[0].outcome, "ambiguous");
    }

    #[test]
    fn citing_article_footer_cannot_verify_the_last_reference() {
        // natural:PMC3408377:60: the footer DOI precedes the explicit boundary.
        // It must still fail verification against Gaddis's reference text.
        let raw = "60. Gaddis NC, Chertova E, Sheehy AM, Henderson LE, Malim MH: Comprehensive investigation of the molecular defect in vif-deficient human immunodeficiency virus type 1 virions. J Virol 2003, 77:5810–5820. doi:10.1186/1742-4690-9-53 Cite this article as: Arjan-Odedra et al.: Endogenous MOV10 inhibits the retrotransposition of endogenous retroelements but not the replication of exogenous retroviruses. Retrovirology 2012 9:53.";
        let citing_article = PaperRecord {
            doi: Some("10.1186/1742-4690-9-53".to_string()),
            title: "Endogenous MOV10 inhibits the retrotransposition of endogenous retroelements but not the replication of exogenous retroviruses".to_string(),
            authors: vec!["Shetal Arjan-Odedra".to_string()],
            year: Some(2012),
            venue: Some("Retrovirology".to_string()),
            ..PaperRecord::default()
        };
        let correct = PaperRecord {
            doi: Some("10.1128/jvi.77.10.5810-5820.2003".to_string()),
            title: "Comprehensive investigation of the molecular defect in vif-deficient human immunodeficiency virus type 1 virions".to_string(),
            authors: vec!["Nathaniel C. Gaddis".to_string()],
            year: Some(2003),
            venue: Some("Journal of Virology".to_string()),
            ..PaperRecord::default()
        };
        for records in [
            [citing_article.clone(), correct.clone()],
            [correct.clone(), citing_article.clone()],
        ] {
            let mut entry = ReferenceEntry {
                raw: raw.to_string(),
                ..ReferenceEntry::default()
            };
            assert!(verify(&citing_article, &entry).is_err());
            assert!(verify(&correct, &entry).is_ok());
            assert_eq!(
                select_query_record(&mut entry, records).unwrap().doi,
                correct.doi
            );
            assert_eq!(entry.raw, raw);
        }
        assert!(biomedical_ids("Gaddis 2003. Cite this article as: PMID:22727223").is_empty());
    }

    #[test]
    #[ignore = "requires live Crossref access"]
    fn live_crossref_exact_identifier() {
        let mut entries = [ReferenceEntry {
            raw: "Piwowar H, Priem J, Larivière V, et al. The state of OA: a large-scale analysis of the prevalence and impact of Open Access articles. PeerJ 2018. doi:10.7717/peerj.4375".to_string(),
            ..ReferenceEntry::default()
        }];
        let outcome = Resolver::new(None).resolve_entries(&mut entries);
        assert_eq!(outcome.resolved, 1, "{:?}", entries[0].attempts);
        assert_eq!(
            entries[0].resolved.as_ref().unwrap().doi.as_deref(),
            Some("10.7717/peerj.4375")
        );
        assert_eq!(entries[0].resolved.as_ref().unwrap().method, "printed");
    }

    #[test]
    fn printed_doi_preserves_repairs_and_raw_extensions() {
        let mut entry = ReferenceEntry {
            raw: "doi: 10.1145/364399 1.3648400".to_string(),
            doi: Some("10.1145/3643991.3648400".to_string()),
            ..ReferenceEntry::default()
        };
        assert_eq!(
            printed_doi(&entry).as_deref(),
            Some("10.1145/3643991.3648400")
        );
        entry.raw = "doi: 10.1000/example(abc)".to_string();
        entry.doi = Some("10.1000/example(abc".to_string());
        assert_eq!(printed_doi(&entry).as_deref(), Some("10.1000/example(abc)"));
        entry.doi = None;
        assert_eq!(printed_doi(&entry), doi_in(&entry.raw));
    }

    #[test]
    fn paper_exact_lookup_requires_normalized_requested_doi() {
        let mut record = query_record("10.1000/other");
        for title in ["", record.title.as_str()] {
            assert!(exact_paper_record(title, "10.1000/requested", &record).is_none());
        }
        record.doi = None;
        assert!(exact_paper_record("", "10.1000/requested", &record).is_none());
        record.doi = Some("https://doi.org/10.1000/REQUESTED".to_string());
        assert!(exact_paper_record(&record.title, "10.1000/requested", &record).is_some());
    }

    #[test]
    fn paper_query_checks_near_ties_in_both_orders() {
        let a = query_record("10.1000/a");
        let mut b = query_record("10.1000/b");
        b.title.push('s');
        for records in [[a.clone(), b.clone()], [b.clone(), a.clone()]] {
            assert!(select_paper_record(&a.title, records).is_none());
        }
        b.doi = Some("https://doi.org/10.1000/A".to_string());
        assert!(select_paper_record(&a.title, [a.clone(), b.clone()]).is_some());
        b.doi = Some("10.1000/b".to_string());
        b.title = "Unrelated research findings".to_string();
        assert_eq!(
            select_paper_record(&a.title, [b, a.clone()])
                .unwrap()
                .doi
                .as_deref(),
            Some("10.1000/a")
        );
    }

    #[test]
    fn ambiguity_is_unresolved_without_metadata_rejection() {
        let mut entry = query_entry();
        let mut wrong = query_record("10.1000/wrong");
        wrong.title = "Unrelated research findings".to_string();
        assert!(
            select_query_record(
                &mut entry,
                [
                    wrong.clone(),
                    query_record("10.1000/a"),
                    query_record("10.1000/b")
                ]
            )
            .is_none()
        );
        let mut outcome = Outcome::default();
        count_rejection(&entry.attempts, &mut outcome);
        assert_eq!(outcome.rejected, 0);
        entry.attempts.clear();
        assert!(select_query_record(&mut entry, [wrong]).is_none());
        count_rejection(&entry.attempts, &mut outcome);
        assert_eq!(outcome.rejected, 1);
    }

    #[test]
    fn explicit_biomedical_ids_are_typed_and_deduplicated() {
        assert_eq!(
            biomedical_ids(
                "PMID: 123456 PMID:123456 https://pubmed.ncbi.nlm.nih.gov/123456/ PMCID:PMC7654321"
            ),
            vec![
                pmc::Identifier::Pmid("123456".to_string()),
                pmc::Identifier::Pmcid("PMC7654321".to_string())
            ]
        );
        assert!(biomedical_ids("2020; 123456:7654321").is_empty());
    }

    fn biomedical_record() -> PaperRecord {
        PaperRecord {
            pmid: Some("123456".to_string()),
            pmcid: Some("PMC7654321".to_string()),
            doi: None,
            source: "europepmc".to_string(),
            ..query_record("unused")
        }
    }

    #[test]
    fn pubmed_record_without_doi_resolves_without_inventing_one() {
        let mut entry = query_entry();
        entry.raw.push_str(" PMID:123456");
        let resolved = select_biomedical_record(
            &mut entry,
            &pmc::Identifier::Pmid("123456".to_string()),
            [biomedical_record()],
        )
        .unwrap();
        assert_eq!(resolved.doi, None);
        assert_eq!(resolved.pmid.as_deref(), Some("123456"));
        assert_eq!(resolved.pmcid.as_deref(), Some("PMC7654321"));
        assert_eq!(resolved.source, "europepmc");
        let json = serde_json::to_value(&resolved).unwrap();
        assert!(json["doi"].is_null());
        assert_eq!(serde_json::from_value::<Resolved>(json).unwrap(), resolved);
    }

    #[test]
    fn biomedical_lookup_rejects_wrong_ids_metadata_and_conflicts() {
        let id = pmc::Identifier::Pmid("123456".to_string());
        let mut wrong = biomedical_record();
        wrong.pmid = Some("999999".to_string());
        assert!(select_biomedical_record(&mut query_entry(), &id, [wrong]).is_none());
        let mut wrong = biomedical_record();
        wrong.title = "An unrelated clinical investigation".to_string();
        assert!(select_biomedical_record(&mut query_entry(), &id, [wrong]).is_none());
        let mut entry = query_entry();
        entry.raw.push_str(" PMID:123456 PMCID:PMC111111");
        assert!(select_biomedical_record(&mut entry, &id, [biomedical_record()]).is_none());
        assert!(entry.attempts.iter().any(|a| a.outcome == "mismatch"));
    }

    #[test]
    fn biomedical_lookup_does_not_pick_distinct_records_by_order() {
        let mut second = biomedical_record();
        second.pmcid = Some("PMC111111".to_string());
        let mut entry = query_entry();
        assert!(
            select_biomedical_record(
                &mut entry,
                &pmc::Identifier::Pmid("123456".to_string()),
                [biomedical_record(), second]
            )
            .is_none()
        );
        assert_eq!(entry.attempts.last().unwrap().outcome, "ambiguous");
    }

    #[test]
    fn explicit_ids_block_fallback_for_conflicts_missing_records_and_bad_metadata() {
        for suffix in [
            "PMID:999999",
            "PMCID:PMC999999",
            "PMID:123456 PMCID:PMC111111",
            "PMID:123456 PMID:999999",
        ] {
            let mut entry = query_entry();
            entry.raw.push(' ');
            entry.raw.push_str(suffix);
            entry.raw.push_str(" doi:10.1000/a");
            assert!(resolve_explicit_biomedical(&mut entry, |entry, id| {
                select_biomedical_record(entry, id, [biomedical_record()])
            }));
            assert!(entry.resolved.is_none());
            let mut outcome = Outcome::default();
            count_rejection(&entry.attempts, &mut outcome);
            assert_eq!(outcome.rejected, 1);
        }
        let mut entry = query_entry();
        entry.raw.push_str(" PMID:123456 doi:10.1000/a");
        assert!(resolve_explicit_biomedical(&mut entry, |entry, id| {
            select_biomedical_record(entry, id, [])
        }));
        assert!(entry.resolved.is_none());
        let mut outcome = Outcome::default();
        count_rejection(&entry.attempts, &mut outcome);
        assert_eq!(outcome.rejected, 0);
        let mut wrong = biomedical_record();
        wrong.title = "An unrelated clinical investigation".to_string();
        assert!(resolve_explicit_biomedical(&mut entry, |entry, id| {
            select_biomedical_record(entry, id, [wrong.clone()])
        }));
        count_rejection(&entry.attempts, &mut outcome);
        assert_eq!(outcome.rejected, 1);
        assert!(entry.resolved.is_none());
    }

    #[test]
    fn explicit_ids_accept_only_verified_joint_identity() {
        let mut entry = query_entry();
        assert!(!resolve_explicit_biomedical(&mut entry, |_, _| panic!(
            "no explicit IDs"
        )));
        entry.raw.push_str(" PMID:123456 PMCID:PMC7654321");
        assert!(resolve_explicit_biomedical(&mut entry, |entry, id| {
            select_biomedical_record(entry, id, [biomedical_record()])
        }));
        assert!(entry.resolved.is_some());
    }

    #[test]
    fn europepmc_mismatch_counts_even_after_not_found_or_transport_error() {
        for id in [
            pmc::Identifier::Pmid("123456".to_string()),
            pmc::Identifier::Doi("10.1000/a".to_string()),
        ] {
            let mut entry = query_entry();
            let mut wrong = biomedical_record();
            wrong.doi = Some("10.1000/a".to_string());
            wrong.title = "An unrelated clinical investigation".to_string();
            assert!(select_biomedical_record(&mut entry, &id, [wrong]).is_none());
            assert_eq!(entry.attempts.last().unwrap().outcome, "not_found");
            entry.attempts.push(Attempt {
                method: "query".to_string(),
                doi: None,
                outcome: "error".to_string(),
                detail: None,
            });
            let mut outcome = Outcome::default();
            count_rejection(&entry.attempts, &mut outcome);
            assert_eq!(outcome.rejected, 1);
            // A previous pass's mismatch must not contaminate a retry.
            count_rejection(&entry.attempts[entry.attempts.len()..], &mut outcome);
            assert_eq!(outcome.rejected, 1);
        }
    }

    #[test]
    fn old_resolved_json_remains_readable() {
        let old = r#"{"doi":"10.1000/a","title":"A title","authors":[],"year":2020,"venue":null,"source":"crossref","method":"printed","score":1.0}"#;
        let record: Resolved = serde_json::from_str(old).unwrap();
        assert_eq!(record.doi.as_deref(), Some("10.1000/a"));
        assert_eq!(record.pmid, None);
        assert_eq!(record.pmcid, None);
    }

    #[test]
    fn failed_registry_requests_are_visible_in_summary() {
        let resolver = Resolver::new(None).with_offline(true);
        let mut entries = [query_entry()];
        entries[0].raw.push_str(" PMID:123456 doi:10.1000/a");
        let outcome = resolver.resolve_entries(&mut entries);
        assert_eq!(outcome.errors, 1);
        assert_eq!(outcome.unresolved, 1);
        assert!(entries[0].attempts.iter().any(|a| a.method == "europepmc"));
        assert!(
            !entries[0]
                .attempts
                .iter()
                .any(|a| a.method == "printed" || a.method == "query")
        );
        assert_eq!(outcome.rejected, 0);
    }

    #[test]
    #[ignore = "requires live Europe PMC and Crossref access"]
    fn live_biomedical_exact_identifiers() {
        let raw = "Piwowar H, Priem J, Larivière V, et al. The state of OA: a large-scale analysis of the prevalence and impact of Open Access articles. PeerJ 2018.";
        for suffix in [
            "PMID:29456894",
            "PMCID:PMC5815332",
            "doi:10.7717/peerj.4375",
        ] {
            let mut entries = [ReferenceEntry {
                raw: format!("{raw} {suffix}"),
                ..ReferenceEntry::default()
            }];
            let outcome = Resolver::new(None).resolve_entries(&mut entries);
            assert_eq!(outcome.resolved, 1, "{:?}", entries[0].attempts);
            let record = entries[0].resolved.as_ref().unwrap();
            assert_eq!(record.doi.as_deref(), Some("10.7717/peerj.4375"));
            assert_eq!(record.pmid.as_deref(), Some("29456894"));
            assert_eq!(record.pmcid.as_deref(), Some("PMC5815332"));
        }
    }

    #[test]
    fn family_names_keep_particles() {
        assert_eq!(family_of("Adrian J. van der Kogel"), "der Kogel");
        assert_eq!(family_of("Jane Smith"), "Smith");
        assert!(author_agreement("van der Kogel, A.J.", "Adrian J. van der Kogel") >= 0.99);
        assert!(author_agreement("Obtułowicz K", "Krystyna Obtulowicz") >= 0.99);
        assert!(author_agreement("Smith, J.", "Jane Jones") < 0.8);
    }

    #[test]
    fn links_attach_to_the_entry_they_sit_on() {
        let mut page = PageText::new(3, 600.0, 800.0, 0);
        page.links.push(Link {
            bbox: Some(BBox {
                x0: 60.0,
                y0: 690.0,
                x1: 300.0,
                y1: 700.0,
            }),
            uri: "https://doi.org/10.1000/first".to_string(),
        });
        page.links.push(Link {
            bbox: Some(BBox {
                x0: 60.0,
                y0: 640.0,
                x1: 300.0,
                y1: 650.0,
            }),
            uri: "https://doi.org/10.1000/second".to_string(),
        });
        let entry = |index: u32, y0: f32| ReferenceEntry {
            index,
            page: 3,
            anchor: Some(BBox {
                x0: 50.0,
                y0,
                x1: 50.0,
                y1: y0 + 10.0,
            }),
            ..ReferenceEntry::default()
        };
        let mut entries = vec![entry(1, 710.0), entry(2, 660.0)];
        attach_links(&mut entries, &[page]);
        assert_eq!(entries[0].doi_link.as_deref(), Some("10.1000/first"));
        assert_eq!(entries[1].doi_link.as_deref(), Some("10.1000/second"));
    }

    #[test]
    fn verification_needs_author_and_year_agreement() {
        let record = PaperRecord {
            title: "A study of things".to_string(),
            authors: vec!["Jane Smith".to_string(), "Bob Jones".to_string()],
            year: Some(2020),
            venue: None,
            doi: Some("10.1000/x".to_string()),
            arxiv_id: None,
            pmid: None,
            pmcid: None,
            url: None,
            abstract_text: None,
            source: "crossref".to_string(),
            source_id: None,
        };
        let mut entry = ReferenceEntry {
            raw: "Smith, J., Jones, B. (2021). A study of things. J. Stuff 3, 1-9.".to_string(),
            ..ReferenceEntry::default()
        };
        assert!(verify(&record, &entry).is_ok());
        entry.raw = "Smith, J., Jones, B. (2015). A study of things. J. Stuff 3, 1-9.".to_string();
        assert!(verify(&record, &entry).unwrap_err().starts_with("year"));
        entry.raw = "Brown, T. (2020). A study of things. J. Stuff 3, 1-9.".to_string();
        assert!(
            verify(&record, &entry)
                .unwrap_err()
                .starts_with("first author")
        );
        entry.raw = "Smith, J. (2020). Something else entirely. J. Stuff 3, 1-9.".to_string();
        assert!(verify(&record, &entry).unwrap_err().starts_with("title"));
        entry.raw = "Ru Èhland K, Smith J. (2020). A study of things.".to_string();
        let record2 = PaperRecord {
            authors: vec!["K. M. Rühland".to_string()],
            ..record.clone()
        };
        assert!(verify(&record2, &entry).is_ok());
        entry.raw = "RowJR, Smith J. (2020). A study of things.".to_string();
        let record3 = PaperRecord {
            authors: vec!["Jeffrey R. Row".to_string()],
            ..record
        };
        assert!(verify(&record3, &entry).is_ok());
    }

    const CACHED_WORK: &str = r#"{"status":"ok","message":{"DOI":"10.7717/peerj.4375","type":"journal-article",
        "title":["The state of OA: a large-scale analysis of the prevalence and impact of Open Access articles"],
        "container-title":["PeerJ"],"publisher":"PeerJ","ISSN":["2167-8359"],"issn-type":[{"value":"2167-8359","type":"electronic"}],
        "license":[{"URL":"http://creativecommons.org/licenses/by/4.0/","delay-in-days":0,"content-version":"vor"}],
        "funder":[{"name":"Alfred P. Sloan Foundation","award":["G-2017-9851"]}],
        "issued":{"date-parts":[[2018,2,13]]},"URL":"https://doi.org/10.7717/peerj.4375",
        "author":[{"given":"Heather","family":"Piwowar"},{"given":"Jason","family":"Priem"}]}}"#;

    const CACHED_SUMMARY: &str = r#"{"result":{"uids":["29456894"],"29456894":{"uid":"29456894",
        "pubdate":"2018 Feb 13","source":"PeerJ","fulljournalname":"PeerJ",
        "authors":[{"name":"Piwowar H"},{"name":"Priem J"}],
        "title":"The state of OA: a large-scale analysis of the prevalence and impact of Open Access articles.",
        "essn":"2167-8359",
        "articleids":[{"idtype":"pubmed","value":"29456894"},{"idtype":"doi","value":"10.7717/peerj.4375"},{"idtype":"pmc","value":"PMC5815332"}]}}}"#;

    fn cached_resolver(dir: &std::path::Path) -> Resolver {
        let cache = tpe_biblio::DiskCache::new(dir, Duration::from_secs(600));
        cache
            .put("crossref", "10.7717/peerj.4375", 200, CACHED_WORK)
            .unwrap();
        cache
            .put("pubmed-esummary", "29456894", 200, CACHED_SUMMARY)
            .unwrap();
        cache.put("crossref", "10.1000/missing", 404, "").unwrap();
        cache.put("datacite", "10.1000/missing", 404, "").unwrap();
        cache
            .put("doi-org-csl", "10.1000/missing", 404, "")
            .unwrap();
        Resolver::new(None).with_cache_dir(dir).with_offline(true)
    }

    fn paper_meta() -> Metadata {
        Metadata {
            title: Some("The state of OA: a large-scale analysis of the prevalence and impact of Open Access articles".to_string()),
            authors: vec![crate::schema::Author {
                name: "Heather Piwowar".to_string(),
                affiliation: None,
                orcid: None,
                email: None,
            }],
            year: Some(2018),
            ..Metadata::default()
        }
    }

    #[test]
    fn paper_doi_resolves_from_the_cache_with_publisher_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = cached_resolver(dir.path());
        let mut meta = paper_meta();
        meta.doi = Some("https://doi.org/10.7717/PEERJ.4375".to_string());
        let report = resolver.resolve_paper_report(&meta, &[]);
        assert_eq!(report.status, "resolved", "{:?}", report.attempts);
        let accepted = report.resolved.as_ref().unwrap();
        assert_eq!(accepted.doi.as_deref(), Some("10.7717/peerj.4375"));
        assert_eq!(accepted.method, "metadata");
        assert_eq!(accepted.source, "crossref");
        assert_eq!(accepted.year, Some(2018));
        let publisher = report.publisher.as_ref().unwrap();
        assert_eq!(publisher.publisher.as_deref(), Some("PeerJ"));
        assert_eq!(publisher.issn, vec!["2167-8359"]);
        assert!(publisher.licenses[0].is_open());
        assert_eq!(publisher.funders[0].name, "Alfred P. Sloan Foundation");
        assert_eq!(report.attempts.len(), 1);
        assert_eq!(report.attempts[0].outcome, "verified");
        assert_eq!(report.identifiers.provenance["doi"], "metadata:doi");
        // The old entry point keeps its contract.
        assert_eq!(resolver.resolve_paper(&meta).unwrap().doi, accepted.doi);
    }

    #[test]
    fn paper_doi_record_must_agree_with_printed_title_and_issn() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = cached_resolver(dir.path());
        let mut meta = paper_meta();
        meta.doi = Some("10.7717/peerj.4375".to_string());
        meta.title = Some("A completely different manuscript about something else".to_string());
        let report = resolver.resolve_paper_report(&meta, &[]);
        assert!(report.resolved.is_none());
        assert_eq!(report.attempts[0].outcome, "mismatch");
        assert!(
            report.attempts[0]
                .detail
                .as_deref()
                .unwrap()
                .starts_with("title")
        );
        // The title query that follows cannot be sent offline: that is reported, not hidden.
        assert_eq!(report.status, "unresolved: offline");
        assert!(report.publisher.is_none());

        let mut meta = paper_meta();
        meta.doi = Some("10.7717/peerj.4375".to_string());
        let mut page = PageText::new(1, 600.0, 800.0, 0);
        page.text = "ISSN 0028-0836\nThe state of OA".to_string();
        let report = resolver.resolve_paper_report(&meta, &[page]);
        assert!(report.resolved.is_none());
        assert_eq!(report.identifiers.issns, vec!["0028-0836"]);
        assert!(
            report.attempts[0]
                .detail
                .as_deref()
                .unwrap()
                .starts_with("issn")
        );
    }

    #[test]
    fn paper_pmid_printed_on_page_one_resolves_through_pubmed() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = cached_resolver(dir.path());
        let mut page = PageText::new(1, 600.0, 800.0, 0);
        page.text = "PeerJ\nPMID: 29456894\nThe state of OA".to_string();
        let report = resolver.resolve_paper_report(&paper_meta(), &[page.clone()]);
        assert_eq!(report.status, "resolved", "{:?}", report.attempts);
        let accepted = report.resolved.as_ref().unwrap();
        assert_eq!(accepted.method, "pmid");
        assert_eq!(accepted.source, "pubmed");
        assert_eq!(accepted.pmid.as_deref(), Some("29456894"));
        assert_eq!(accepted.pmcid.as_deref(), Some("PMC5815332"));
        assert_eq!(accepted.doi.as_deref(), Some("10.7717/peerj.4375"));
        // The DOI the summary carries brings the cached publisher record along.
        assert_eq!(
            report.publisher.as_ref().unwrap().publisher.as_deref(),
            Some("PeerJ")
        );
        assert_eq!(report.identifiers.provenance["pmid"], "first_page");

        // A wrong first author or year rejects the PubMed record.
        let mut meta = paper_meta();
        meta.authors[0].name = "Someone Else".to_string();
        let report = resolver.resolve_paper_report(&meta, &[page.clone()]);
        assert!(report.resolved.is_none());
        assert!(
            report.attempts[0]
                .detail
                .as_deref()
                .unwrap()
                .starts_with("first author")
        );
        let mut meta = paper_meta();
        meta.year = Some(2011);
        let report = resolver.resolve_paper_report(&meta, &[page]);
        assert!(
            report.attempts[0]
                .detail
                .as_deref()
                .unwrap()
                .starts_with("year")
        );
    }

    #[test]
    fn offline_paper_resolution_is_reported_not_mistaken_for_absence() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = cached_resolver(dir.path());
        let mut meta = paper_meta();
        meta.doi = Some("10.1000/fresh".to_string());
        let report = resolver.resolve_paper_report(&meta, &[]);
        assert_eq!(report.status, "unresolved: offline");
        assert_eq!(report.attempts[0].outcome, "error");
        assert_eq!(report.attempts[0].detail.as_deref(), Some("offline"));
        assert_eq!(report.attempts[1].method, "query");
        assert_eq!(report.attempts[1].outcome, "error");
        // Every registry says no (cached 404s): not found, then the title query is offline.
        meta.doi = Some("10.1000/missing".to_string());
        let report = resolver.resolve_paper_report(&meta, &[]);
        assert_eq!(report.attempts[0].outcome, "not_found");
        assert_eq!(report.status, "unresolved: offline");
        // Nothing to go on at all.
        let report = resolver.resolve_paper_report(&Metadata::default(), &[]);
        assert_eq!(report.status, "unresolved: no identifier or title");
        assert!(report.attempts.is_empty());
        assert!(resolver.resolve_paper(&Metadata::default()).is_none());
    }

    #[test]
    fn reference_doi_lookup_uses_the_cache_and_reports_nothing_as_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = cached_resolver(dir.path());
        let mut entries = [ReferenceEntry {
            raw: "Piwowar H, Priem J. The state of OA: a large-scale analysis of the prevalence and impact of Open Access articles. PeerJ 2018. doi:10.7717/peerj.4375".to_string(),
            ..ReferenceEntry::default()
        }];
        let outcome = resolver.resolve_entries(&mut entries);
        assert_eq!(outcome.resolved, 1, "{:?}", entries[0].attempts);
        assert_eq!(outcome.by_method["printed"], 1);
        let accepted = entries[0].resolved.as_ref().unwrap();
        assert_eq!(accepted.doi.as_deref(), Some("10.7717/peerj.4375"));
        assert_eq!(accepted.venue.as_deref(), Some("PeerJ"));
        // Enrichment through Europe PMC is not cached, so it is logged as an error.
        assert_eq!(outcome.errors, 1);
        assert!(
            entries[0]
                .attempts
                .iter()
                .any(|a| a.method == "europepmc" && a.outcome == "error")
        );
        // A cached 404 is `not_found` without a request; the query is then offline.
        let mut entries = [ReferenceEntry {
            raw: "Nobody. Nothing at all. 2001. doi:10.1000/missing".to_string(),
            ..ReferenceEntry::default()
        }];
        let outcome = resolver.resolve_entries(&mut entries);
        assert_eq!(outcome.unresolved, 1);
        assert_eq!(entries[0].attempts[0].outcome, "not_found");
    }

    #[test]
    fn resolver_from_env_reads_only_the_documented_variables() {
        // No addresses are hard-coded: without `TPE_MAILTO` there is no mailto.
        let resolver = Resolver::new(None);
        assert_eq!(resolver.client.mailto(), None);
        assert!(resolver.client.user_agent().starts_with("tpe/"));
        assert!(!resolver.client.user_agent().contains('@'));
        let polite = Resolver::new(Some("someone@example.org"));
        assert!(
            polite
                .client
                .user_agent()
                .ends_with("(mailto:someone@example.org)")
        );
        assert!(polite.cache.is_none());
        assert_eq!(tpe_biblio::MAILTO_ENV, "TPE_MAILTO");
        assert_eq!(tpe_biblio::CACHE_DIR_ENV, "TPE_BIBLIO_CACHE_DIR");
    }
}
