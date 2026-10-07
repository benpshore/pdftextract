//! Identifier extraction and normalisation from free text (extracted PDF
//! text, `/Info` strings, link URIs): DOI, `arXiv` id, PMID, PMCID, ISBN and
//! ISSN. Every function here is pure and never touches the network.
//!
//! Rules, so that nothing is invented:
//! - DOIs follow Crossref's recommended pattern `10.<4-9 digits>/<suffix>`
//!   with the suffix alphabet `-._;()/:A-Za-z0-9`; the older Wiley SICI
//!   form under `10.1002` may also contain `<>`. Trailing sentence
//!   punctuation and unbalanced closing brackets are removed, `doi.org`
//!   resolver prefixes (percent-encoded or not) are stripped and the result
//!   is lower-cased.
//! - `arXiv` ids are accepted in the new scheme (`YYMM.NNNNN`, `YYMM.NNNN`
//!   before 2015) and the old scheme (`archive[.SC]/YYMMNNN`). A bare
//!   new-style id without an `arXiv:` label or `arxiv.org` URL must have a
//!   plausible month; an old-style id without a label must name a known
//!   archive.
//! - PMIDs are taken only from a `PMID` label or a `PubMed` URL; a bare
//!   number is never a PMID. PMCIDs need the `PMC` prefix.
//! - ISBN-10/13 and ISSN values are accepted only when their check digit
//!   verifies. Each hit records whether it was labelled (`ISBN`, `ISSN`),
//!   so callers can keep only labelled values where a bare match would be
//!   too weak (a page range can look like an ISSN).

use std::sync::OnceLock;

use regex::Regex;
use tpe_common::{normalize_arxiv_id, normalize_doi};

/// A value found in text, with whether an explicit label introduced it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Hit {
    /// The normalised identifier.
    pub value: String,
    /// True when a label (`ISBN`, `ISSN`, `PMID`, `arXiv:`) or an
    /// identifier URL introduced it.
    pub labelled: bool,
}

/// Every identifier found in one text, deduplicated, in order of appearance.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identifiers {
    /// Normalised DOIs (lower-case, no resolver prefix).
    pub dois: Vec<String>,
    /// `arXiv` ids without version suffix.
    pub arxiv_ids: Vec<Hit>,
    /// `PubMed` ids (digits only); always labelled.
    pub pmids: Vec<String>,
    /// `PubMed` Central ids in the `PMC<digits>` form.
    pub pmcids: Vec<String>,
    /// ISBNs as 10 or 13 digits without separators, check digit verified.
    pub isbns: Vec<Hit>,
    /// ISSNs as `NNNN-NNNC`, check digit verified.
    pub issns: Vec<Hit>,
}

impl Identifiers {
    /// True when nothing was found.
    pub fn is_empty(&self) -> bool {
        self.dois.is_empty()
            && self.arxiv_ids.is_empty()
            && self.pmids.is_empty()
            && self.pmcids.is_empty()
            && self.isbns.is_empty()
            && self.issns.is_empty()
    }
}

/// Extract every identifier kind from `text`.
pub fn extract_identifiers(text: &str) -> Identifiers {
    Identifiers {
        dois: extract_dois(text),
        arxiv_ids: extract_arxiv_ids(text),
        pmids: extract_pmids(text),
        pmcids: extract_pmcids(text),
        isbns: extract_isbns(text),
        issns: extract_issns(text),
    }
}

fn push_unique<T: PartialEq>(out: &mut Vec<T>, value: T) {
    if !out.contains(&value) {
        out.push(value);
    }
}

// ---------------------------------------------------------------- DOI

fn doi_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Crossref's recommended class, plus `<>[]` so SICI suffixes are read
    // whole and bracketed citations can be trimmed afterwards.
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b10\.\d{4,9}/[-._;()/:a-z0-9<>\[\]]+").expect("valid regex")
    })
}

/// Decode `%XX` escapes that a `doi.org` URL may carry (`%2F` for `/`).
/// Invalid escapes are kept verbatim; the result is lossy for non-UTF-8.
pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = input.get(i + 1..i + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Remove trailing sentence punctuation and closing brackets that have no
/// opening partner inside the candidate.
fn trim_doi_tail(mut doi: &str) -> &str {
    loop {
        let before = doi;
        doi = doi.trim_end_matches(['.', ',', ';', ':']);
        for (open, close) in [('(', ')'), ('[', ']'), ('<', '>')] {
            while doi.ends_with(close) && doi.matches(close).count() > doi.matches(open).count() {
                doi = &doi[..doi.len() - 1];
            }
        }
        if doi == before {
            return doi;
        }
    }
}

/// Normalise one DOI candidate (a bare DOI, `doi:` form or `doi.org` URL)
/// to Crossref's canonical lower-case form. `None` when it is not a DOI.
pub fn normalize_doi_text(raw: &str) -> Option<String> {
    let decoded = percent_decode(raw.trim());
    let found = doi_re().find(&decoded)?;
    let mut candidate = trim_doi_tail(found.as_str());
    // Only the Wiley SICI family legitimately contains angle brackets.
    if !candidate.to_ascii_lowercase().starts_with("10.1002/")
        && let Some(cut) = candidate.find(['<', '>'])
    {
        candidate = trim_doi_tail(&candidate[..cut]);
    }
    normalize_doi(candidate)
}

/// Every distinct DOI in `text`, in order of appearance.
pub fn extract_dois(text: &str) -> Vec<String> {
    let decoded = percent_decode(text);
    let mut out: Vec<String> = Vec::new();
    for found in doi_re().find_iter(&decoded) {
        if let Some(doi) = normalize_doi_text(found.as_str()) {
            push_unique(&mut out, doi);
        }
    }
    out
}

/// True when `doi` is `arXiv`'s own DOI for a preprint (`10.48550/arxiv.<id>`).
pub fn is_arxiv_doi(doi: &str) -> bool {
    doi.to_ascii_lowercase().starts_with("10.48550/arxiv.")
}

/// DOI prefixes registered through `DataCite` that are common in reference
/// lists (Zenodo, figshare, Dryad, `arXiv`, OSF, `bioRxiv` is Crossref).
/// This is a hint for choosing a first registry to ask; the authoritative
/// answer is the `doi.org` registration-agency lookup.
pub const DATACITE_PREFIXES: [&str; 6] = [
    "10.5281", "10.6084", "10.5061", "10.48550", "10.17605", "10.5962",
];

/// True when the DOI's prefix is in [`DATACITE_PREFIXES`].
pub fn likely_datacite_doi(doi: &str) -> bool {
    doi.split_once('/')
        .is_some_and(|(prefix, _)| DATACITE_PREFIXES.contains(&prefix))
}

// -------------------------------------------------------------- arXiv

/// Archive names of the old `arXiv` scheme, so a bare `hep-th/9901001` is
/// recognised without an `arXiv:` label.
pub const ARXIV_ARCHIVES: [&str; 24] = [
    "astro-ph", "cond-mat", "gr-qc", "hep-ex", "hep-lat", "hep-ph", "hep-th", "math-ph", "nlin",
    "nucl-ex", "nucl-th", "physics", "quant-ph", "math", "cs", "q-bio", "q-fin", "stat", "eess",
    "econ", "chao-dyn", "solv-int", "patt-sol", "alg-geom",
];

fn arxiv_labelled_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:\barxiv\s*:\s*|arxiv\.org/(?:abs|pdf)/)(\d{4}\.\d{4,5}(?:v\d+)?|[a-z\-]+(?:\.[a-z]{2})?/\d{7}(?:v\d+)?)",
        )
        .expect("valid regex")
    })
}

fn arxiv_bare_new_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:^|[^\d.])(\d{4}\.\d{5})(?:v\d+)?\b").expect("valid regex"))
}

fn arxiv_bare_old_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b([a-z\-]+(?:\.[A-Z]{2})?/\d{7})(?:v\d+)?\b").expect("valid regex")
    })
}

/// A new-style id's `YYMM` must be a real month on or after 2007-04.
fn plausible_new_style(id: &str) -> bool {
    let Some((yymm, _)) = id.split_once('.') else {
        return false;
    };
    let Ok(yy) = yymm[..2].parse::<u16>() else {
        return false;
    };
    let Ok(mm) = yymm[2..].parse::<u16>() else {
        return false;
    };
    // The new scheme started with 0704.
    (1..=12).contains(&mm) && (yy > 7 || (yy == 7 && mm >= 4))
}

/// Every distinct `arXiv` id in `text` (version suffix removed).
pub fn extract_arxiv_ids(text: &str) -> Vec<Hit> {
    let mut out: Vec<Hit> = Vec::new();
    for caps in arxiv_labelled_re().captures_iter(text) {
        if let Some(id) = normalize_arxiv_id(&caps[1]) {
            push_unique(
                &mut out,
                Hit {
                    value: id,
                    labelled: true,
                },
            );
        }
    }
    for caps in arxiv_bare_new_re().captures_iter(text) {
        let raw = &caps[1];
        if plausible_new_style(raw)
            && let Some(id) = normalize_arxiv_id(raw)
            && !out.iter().any(|h| h.value == id)
        {
            out.push(Hit {
                value: id,
                labelled: false,
            });
        }
    }
    for caps in arxiv_bare_old_re().captures_iter(text) {
        let raw = &caps[1];
        let archive = raw.split(['/', '.']).next().unwrap_or("");
        if ARXIV_ARCHIVES.contains(&archive)
            && let Some(id) = normalize_arxiv_id(raw)
            && !out.iter().any(|h| h.value == id)
        {
            out.push(Hit {
                value: id,
                labelled: false,
            });
        }
    }
    out
}

// ------------------------------------------------------- PMID / PMCID

fn pmid_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:\bPMID\s*:?\s*|pubmed\.ncbi\.nlm\.nih\.gov/|ncbi\.nlm\.nih\.gov/pubmed/)(\d{1,12})\b",
        )
        .expect("valid regex")
    })
}

fn pmcid_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\bPMC\s?(\d{1,12})\b").expect("valid regex"))
}

/// Normalise a `PubMed` id: digits only, no leading zeros, at most nine digits.
pub fn normalize_pmid(raw: &str) -> Option<String> {
    let digits = raw
        .trim()
        .trim_start_matches("PMID")
        .trim_start_matches(':')
        .trim();
    let digits = digits.trim_start_matches('0');
    (!digits.is_empty() && digits.len() <= 9 && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.to_string())
}

/// Normalise a `PubMed` Central id to `PMC<digits>`.
pub fn normalize_pmcid(raw: &str) -> Option<String> {
    let t = raw.trim();
    let digits = t
        .strip_prefix("PMC")
        .or_else(|| t.strip_prefix("pmc"))
        .unwrap_or(t)
        .trim()
        .trim_start_matches('0');
    (!digits.is_empty() && digits.len() <= 9 && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| format!("PMC{digits}"))
}

/// Every labelled `PubMed` id in `text`. Bare numbers are never PMIDs.
pub fn extract_pmids(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for caps in pmid_re().captures_iter(text) {
        if let Some(id) = normalize_pmid(&caps[1]) {
            push_unique(&mut out, id);
        }
    }
    out
}

/// Every `PMC<digits>` id in `text`.
pub fn extract_pmcids(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for caps in pmcid_re().captures_iter(text) {
        if let Some(id) = normalize_pmcid(&caps[1]) {
            push_unique(&mut out, id);
        }
    }
    out
}

// --------------------------------------------------------------- ISBN

fn isbn_labelled_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\bISBN(?:-1[03])?\s*:?\s*([0-9][0-9 \-]{8,20}[0-9Xx])")
            .expect("valid regex")
    })
}

fn isbn_bare_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b(97[89][\- ]\d{1,5}[\- ]\d{1,7}[\- ]\d{1,7}[\- ][0-9Xx]|\d{1,5}[\- ]\d{1,7}[\- ]\d{1,7}[\- ][0-9Xx])\b")
            .expect("valid regex")
    })
}

/// Digits of an ISBN candidate without separators, upper-cased check digit.
fn isbn_digits(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// True when `digits` (10 or 13 characters, no separators) has a valid check digit.
pub fn isbn_is_valid(digits: &str) -> bool {
    let bytes = digits.as_bytes();
    match bytes.len() {
        10 => {
            if !bytes[..9].iter().all(u8::is_ascii_digit) {
                return false;
            }
            let sum: u32 = bytes[..9]
                .iter()
                .zip((2..=10).rev())
                .map(|(b, weight)| u32::from(b - b'0') * weight)
                .sum();
            let check = match bytes[9] {
                b'X' => 10,
                d if d.is_ascii_digit() => u32::from(d - b'0'),
                _ => return false,
            };
            (sum + check).is_multiple_of(11)
        }
        13 => {
            if !bytes.iter().all(u8::is_ascii_digit)
                || !(digits.starts_with("978") || digits.starts_with("979"))
            {
                return false;
            }
            let sum: u32 = bytes
                .iter()
                .enumerate()
                .map(|(i, b)| u32::from(b - b'0') * if i % 2 == 0 { 1 } else { 3 })
                .sum();
            sum.is_multiple_of(10)
        }
        _ => false,
    }
}

/// Normalise an ISBN (with or without separators or an `ISBN` label) to its
/// digits; `None` unless the check digit verifies.
pub fn normalize_isbn(raw: &str) -> Option<String> {
    let t = raw.trim();
    let t = t
        .strip_prefix("ISBN")
        .or_else(|| t.strip_prefix("isbn"))
        .unwrap_or(t);
    let t = t
        .strip_prefix("-13")
        .or_else(|| t.strip_prefix("-10"))
        .unwrap_or(t)
        .trim_start_matches([':', ' '])
        .trim();
    let digits = isbn_digits(t);
    isbn_is_valid(&digits).then_some(digits)
}

/// Convert a valid ISBN-10 to ISBN-13; an ISBN-13 is returned unchanged.
pub fn isbn13_of(digits: &str) -> Option<String> {
    if !isbn_is_valid(digits) {
        return None;
    }
    if digits.len() == 13 {
        return Some(digits.to_string());
    }
    let body = format!("978{}", &digits[..9]);
    let sum: u32 = body
        .bytes()
        .enumerate()
        .map(|(i, b)| u32::from(b - b'0') * if i % 2 == 0 { 1 } else { 3 })
        .sum();
    let check = (10 - sum % 10) % 10;
    Some(format!("{body}{check}"))
}

/// Every valid ISBN in `text`: labelled ones first, then hyphenated bare ones.
pub fn extract_isbns(text: &str) -> Vec<Hit> {
    let mut out: Vec<Hit> = Vec::new();
    for caps in isbn_labelled_re().captures_iter(text) {
        let digits = isbn_digits(&caps[1]);
        // A labelled run may carry a trailing year or page number: try the
        // longest valid prefix of 13 then 10 characters.
        for len in [13, 10] {
            if digits.len() >= len && isbn_is_valid(&digits[..len]) {
                push_unique(
                    &mut out,
                    Hit {
                        value: digits[..len].to_string(),
                        labelled: true,
                    },
                );
                break;
            }
        }
    }
    for caps in isbn_bare_re().captures_iter(text) {
        let digits = isbn_digits(&caps[1]);
        if isbn_is_valid(&digits) && !out.iter().any(|h| h.value == digits) {
            out.push(Hit {
                value: digits,
                labelled: false,
            });
        }
    }
    out
}

// --------------------------------------------------------------- ISSN

fn issn_labelled_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:e-?|p-?|print\s+|online\s+|electronic\s+)?ISSN\s*:?\s*(\d{4})\s?-?\s?(\d{3}[\dXx])\b")
            .expect("valid regex")
    })
}

fn issn_bare_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b(\d{4})-(\d{3}[\dXx])\b").expect("valid regex"))
}

/// True when `issn` (`NNNN-NNNC` or eight characters) has a valid check digit.
pub fn issn_is_valid(issn: &str) -> bool {
    let digits: Vec<u8> = issn.bytes().filter(|b| *b != b'-').collect();
    if digits.len() != 8 || !digits[..7].iter().all(u8::is_ascii_digit) {
        return false;
    }
    let sum: u32 = digits[..7]
        .iter()
        .zip((2..=8).rev())
        .map(|(b, weight)| u32::from(b - b'0') * weight)
        .sum();
    let check = match digits[7].to_ascii_uppercase() {
        b'X' => 10,
        d if d.is_ascii_digit() => u32::from(d - b'0'),
        _ => return false,
    };
    (sum + check).is_multiple_of(11)
}

/// Normalise an ISSN to `NNNN-NNNC` (upper-case `X`); `None` unless the
/// check digit verifies.
pub fn normalize_issn(raw: &str) -> Option<String> {
    let t = raw.trim();
    let t = t
        .strip_prefix("ISSN")
        .or_else(|| t.strip_prefix("issn"))
        .unwrap_or(t);
    let chars: String = t
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if chars.len() != 8 {
        return None;
    }
    let formatted = format!("{}-{}", &chars[..4], &chars[4..]);
    issn_is_valid(&formatted).then_some(formatted)
}

/// Every valid ISSN in `text`: labelled first, then bare `NNNN-NNNC` forms.
pub fn extract_issns(text: &str) -> Vec<Hit> {
    let mut out: Vec<Hit> = Vec::new();
    for caps in issn_labelled_re().captures_iter(text) {
        if let Some(issn) = normalize_issn(&format!("{}{}", &caps[1], &caps[2])) {
            push_unique(
                &mut out,
                Hit {
                    value: issn,
                    labelled: true,
                },
            );
        }
    }
    for caps in issn_bare_re().captures_iter(text) {
        if let Some(issn) = normalize_issn(&format!("{}{}", &caps[1], &caps[2]))
            && !out.iter().any(|h| h.value == issn)
        {
            out.push(Hit {
                value: issn,
                labelled: false,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hits(items: &[Hit]) -> Vec<(&str, bool)> {
        items
            .iter()
            .map(|h| (h.value.as_str(), h.labelled))
            .collect()
    }

    #[test]
    fn dois_are_normalised_and_trimmed() {
        assert_eq!(
            extract_dois("See https://doi.org/10.1038/NATURE14539. and doi:10.1000/x,"),
            vec!["10.1038/nature14539", "10.1000/x"]
        );
        assert_eq!(
            normalize_doi_text("http://dx.doi.org/10.1016/j.cell.2020.01.001)"),
            Some("10.1016/j.cell.2020.01.001".to_string())
        );
        assert_eq!(
            normalize_doi_text("[doi:10.1000/example]."),
            Some("10.1000/example".to_string())
        );
        assert_eq!(normalize_doi_text("10.1/short"), None);
        assert_eq!(normalize_doi_text("no doi"), None);
    }

    #[test]
    fn doi_org_urls_are_percent_decoded() {
        assert_eq!(
            extract_dois(
                "https://doi.org/10.1002%2F%28SICI%291097-0258%2819980815%2917%3A15%3C1741%3A%3AAID-SIM868%3E3.0.CO%3B2-8"
            ),
            vec!["10.1002/(sici)1097-0258(19980815)17:15<1741::aid-sim868>3.0.co;2-8"]
        );
        assert_eq!(percent_decode("a%2Fb%"), "a/b%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn sici_brackets_kept_only_for_wiley() {
        assert_eq!(
            extract_dois("10.1002/(SICI)1097-0258(19980815)17:15<1741::AID-SIM868>3.0.CO;2-8."),
            vec!["10.1002/(sici)1097-0258(19980815)17:15<1741::aid-sim868>3.0.co;2-8"]
        );
        assert_eq!(extract_dois("<a>10.1000/abc</a>"), vec!["10.1000/abc"]);
        assert_eq!(
            extract_dois("10.1016/S0140-6736(20)30183-5)"),
            vec!["10.1016/s0140-6736(20)30183-5"]
        );
    }

    #[test]
    fn duplicate_dois_collapse() {
        assert_eq!(
            extract_dois("doi:10.1000/ABC https://doi.org/10.1000/abc 10.1000/abc."),
            vec!["10.1000/abc"]
        );
        assert!(is_arxiv_doi("10.48550/arXiv.1706.03762"));
        assert!(likely_datacite_doi("10.5281/zenodo.123"));
        assert!(!likely_datacite_doi("10.1038/nature14539"));
    }

    #[test]
    fn arxiv_old_and_new_schemes() {
        let found = extract_arxiv_ids(
            "arXiv:1706.03762v5, https://arxiv.org/abs/2301.00001, hep-th/9901001 and math.GT/0309136v1 and bare 2105.12345v2",
        );
        assert_eq!(
            hits(&found),
            vec![
                ("1706.03762", true),
                ("2301.00001", true),
                ("2105.12345", false),
                ("hep-th/9901001", false),
                ("math.GT/0309136", false),
            ]
        );
        // Decimal numbers, prices and bad months are not ids.
        assert!(extract_arxiv_ids("costs 1234.56789 and 2013.13456 and 0.1234.56789").is_empty());
        // A four-digit suffix is accepted only with a label.
        assert!(extract_arxiv_ids("0704.0001").is_empty());
        assert_eq!(
            hits(&extract_arxiv_ids("arXiv: 0704.0001")),
            vec![("0704.0001", true)]
        );
        // Unknown archives need a label.
        assert!(extract_arxiv_ids("foo-bar/1234567").is_empty());
        assert_eq!(
            hits(&extract_arxiv_ids("arXiv:foo-bar/1234567")),
            vec![("foo-bar/1234567", true)]
        );
    }

    #[test]
    fn pmids_need_a_label_and_pmcids_a_prefix() {
        assert_eq!(
            extract_pmids(
                "PMID: 29456894; PMID:0029456894 https://pubmed.ncbi.nlm.nih.gov/12345/ 99999"
            ),
            vec!["29456894", "12345"]
        );
        assert!(extract_pmids("2020; 123456:7654321").is_empty());
        assert_eq!(
            extract_pmcids("PMCID: PMC5815332, pmc/articles/PMC0005815332/, PMC 42"),
            vec!["PMC5815332", "PMC42"]
        );
        assert_eq!(normalize_pmid("PMID:007"), Some("7".to_string()));
        assert_eq!(normalize_pmid("0"), None);
        assert_eq!(normalize_pmcid("5815332"), Some("PMC5815332".to_string()));
        assert_eq!(normalize_pmcid("abc"), None);
    }

    #[test]
    fn isbn_checksums() {
        assert!(isbn_is_valid("0306406152"));
        assert!(!isbn_is_valid("0306406153"));
        assert!(isbn_is_valid("9780306406157"));
        assert!(!isbn_is_valid("9780306406158"));
        assert!(isbn_is_valid("080442957X"));
        assert!(!isbn_is_valid("12345"));
        assert_eq!(isbn13_of("0306406152"), Some("9780306406157".to_string()));
        assert_eq!(
            isbn13_of("9780306406157"),
            Some("9780306406157".to_string())
        );
        assert_eq!(isbn13_of("0306406153"), None);
        assert_eq!(
            normalize_isbn("ISBN 978-0-306-40615-7"),
            Some("9780306406157".to_string())
        );
        assert_eq!(
            normalize_isbn("ISBN-10: 0-306-40615-2"),
            Some("0306406152".to_string())
        );
        assert_eq!(normalize_isbn("ISBN 978-0-306-40615-8"), None);
    }

    #[test]
    fn isbns_from_text() {
        let found = extract_isbns(
            "ISBN 978-0-306-40615-7 (print), ISBN: 0-8044-2957-X, isbn 9780306406158, and 978-3-16-148410-0 bare, 1-2345-6789-0 bad",
        );
        assert_eq!(
            hits(&found),
            vec![
                ("9780306406157", true),
                ("080442957X", true),
                ("9783161484100", false)
            ]
        );
        assert!(extract_isbns("978-1-7281-1234-5/20/$31.00").is_empty());
    }

    #[test]
    fn issn_checksums_and_extraction() {
        assert!(issn_is_valid("0378-5955"));
        assert!(issn_is_valid("2049-3630"));
        assert!(!issn_is_valid("0378-5956"));
        assert!(issn_is_valid("0317-8471"));
        assert_eq!(
            normalize_issn("issn 03785955"),
            Some("0378-5955".to_string())
        );
        assert_eq!(normalize_issn("2434-561x"), Some("2434-561X".to_string()));
        assert_eq!(normalize_issn("0378-5956"), None);
        let found = extract_issns(
            "ISSN 0378-5955, e-ISSN: 2049-3630, pages 1742-1750, 0317-8471 bare, 1234-5678",
        );
        assert_eq!(
            hits(&found),
            vec![
                ("0378-5955", true),
                ("2049-3630", true),
                ("0317-8471", false)
            ]
        );
    }

    #[test]
    fn everything_at_once() {
        let ids = extract_identifiers(
            "Smith (2020). Title. J Stuff. doi:10.1000/ABC PMID: 123 PMCID: PMC456 arXiv:2105.12345 ISSN 0378-5955 ISBN 0-306-40615-2",
        );
        assert_eq!(ids.dois, vec!["10.1000/abc"]);
        assert_eq!(ids.pmids, vec!["123"]);
        assert_eq!(ids.pmcids, vec!["PMC456"]);
        assert_eq!(hits(&ids.arxiv_ids), vec![("2105.12345", true)]);
        assert_eq!(hits(&ids.issns), vec![("0378-5955", true)]);
        assert_eq!(hits(&ids.isbns), vec![("0306406152", true)]);
        assert!(!ids.is_empty());
        assert!(extract_identifiers("nothing here").is_empty());
    }
}
