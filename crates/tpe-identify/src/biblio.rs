//! Bibliographic keys: a DOI or `arXiv` id when the engine found one, else
//! a normalised title plus first-author surname plus year. Every field is
//! sanity-checked because PDF metadata is often garbage (`Microsoft Word -
//! draft.docx`, `untitled`, an author field holding an e-mail address).

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use tpe::schema::Metadata;
use tpe_common::{normalize_arxiv_id, normalize_doi};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Whether an input looks like a preprint, a publisher version, or neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Variant {
    /// An `arXiv`/`bioRxiv`/`medRxiv`/SSRN/OSF/Research Square/`ChemRxiv`
    /// record, or page text that announces itself as a preprint.
    Preprint,
    /// A DOI outside the preprint registrants.
    Published,
    /// No evidence either way.
    Unknown,
}

impl Variant {
    /// Rank used when choosing a group's canonical member (lower is better).
    pub fn rank(self) -> u8 {
        match self {
            Self::Published => 0,
            Self::Preprint => 1,
            Self::Unknown => 2,
        }
    }
}

/// Where an identifier came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdSource {
    /// The engine's `metadata` block.
    Metadata,
    /// A scan of the first page's text (weaker: could be a cited DOI).
    Text,
}

/// The identifiers that link one input to a work.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BiblioKey {
    /// Normalised DOI (`10.xxxx/...`, lower-case).
    pub doi: Option<String>,
    pub doi_source: Option<IdSource>,
    /// Normalised `arXiv` id without version.
    pub arxiv_id: Option<String>,
    pub arxiv_source: Option<IdSource>,
    /// Title as the engine reported it, when it passed the plausibility check.
    pub title: Option<String>,
    /// Lower-case alphanumeric title tokens joined by spaces.
    pub title_key: Option<String>,
    /// First author's name as reported.
    pub first_author: Option<String>,
    /// ASCII-folded lower-case surname of the first author.
    pub surname: Option<String>,
    pub year: Option<u16>,
    /// Why a field was rejected or substituted.
    pub notes: Vec<String>,
}

impl BiblioKey {
    /// A key with no usable fields.
    pub fn is_empty(&self) -> bool {
        self.doi.is_none() && self.arxiv_id.is_none() && self.title_key.is_none()
    }
}

/// Fold to ASCII-ish: NFKD, drop combining marks, map a few letters that
/// have no decomposition (`ß`, `ø`, `æ`, `ł`, `đ`), keep everything else.
pub fn fold_ascii(text: &str) -> String {
    text.nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(|c| match c {
            'ß' => "ss".chars().collect::<Vec<_>>(),
            'ø' => vec!['o'],
            'Ø' => vec!['O'],
            'æ' => "ae".chars().collect(),
            'Æ' => "AE".chars().collect(),
            'œ' => "oe".chars().collect(),
            'Œ' => "OE".chars().collect(),
            'ł' => vec!['l'],
            'Ł' => vec!['L'],
            'đ' => vec!['d'],
            'Đ' => vec!['D'],
            'þ' => "th".chars().collect(),
            'Þ' => "Th".chars().collect(),
            'ı' => vec!['i'],
            other => vec![other],
        })
        .collect()
}

/// Lower-case alphanumeric tokens of `text` joined by single spaces, after
/// ASCII folding.
pub fn normalize_title(text: &str) -> String {
    let mut out = String::new();
    let mut token = String::new();
    for ch in fold_ascii(text).chars().chain(std::iter::once(' ')) {
        if ch.is_alphanumeric() {
            token.extend(ch.to_lowercase());
        } else if !token.is_empty() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&token);
            token.clear();
        }
    }
    out
}

/// Metadata titles that are really a file name, a template placeholder or
/// an application banner. Checked against the lower-cased, trimmed title.
const JUNK_TITLE_PREFIXES: &[&str] = &[
    "microsoft word",
    "microsoft powerpoint",
    "powerpoint presentation",
    "untitled",
    "document",
    "draft",
    "manuscript",
    "slide 1",
    "title",
    "paper",
    "final",
    "revised",
    "untitled document",
    "pdf",
    "print",
    "layout",
    "template",
    "author guidelines",
    "doi",
];

const JUNK_TITLE_SUFFIXES: &[&str] = &[
    ".doc", ".docx", ".tex", ".pdf", ".dvi", ".ps", ".odt", ".rtf", ".indd", ".qxd", ".pptx",
];

/// Whether `title` can serve as an identity: at least two tokens with
/// letters, at least ten letters in total, more letters than other
/// characters, not a file name or an application banner, and not a single
/// repeated character.
pub fn title_is_plausible(title: &str) -> bool {
    let trimmed = title.trim();
    let lower = trimmed.to_lowercase();
    if trimmed.chars().count() > 400 {
        return false;
    }
    if JUNK_TITLE_PREFIXES.iter().any(|junk| {
        lower == *junk
            || lower.starts_with(&format!("{junk} "))
            || lower.starts_with(&format!("{junk}-"))
            || lower.starts_with(&format!("{junk}:"))
    }) {
        return false;
    }
    if JUNK_TITLE_SUFFIXES
        .iter()
        .any(|suffix| lower.ends_with(suffix))
    {
        return false;
    }
    let letters = trimmed.chars().filter(|c| c.is_alphabetic()).count();
    let others = trimmed
        .chars()
        .filter(|c| !c.is_alphabetic() && !c.is_whitespace())
        .count();
    if letters < 10 || others > letters {
        return false;
    }
    let words_with_letters = trimmed
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphabetic))
        .count();
    if words_with_letters < 2 {
        return false;
    }
    let distinct: std::collections::HashSet<char> =
        lower.chars().filter(|c| c.is_alphanumeric()).collect();
    distinct.len() >= 4
}

/// Tokens that are not surnames even when they end an author string.
const NOT_SURNAMES: &[&str] = &[
    "al",
    "et",
    "and",
    "others",
    "anonymous",
    "author",
    "authors",
    "university",
    "department",
    "institute",
    "laboratory",
    "group",
    "team",
    "consortium",
    "collaboration",
    "inc",
    "ltd",
    "unknown",
    "user",
    "admin",
    "administrator",
    "owner",
    "editor",
    "editors",
    "committee",
];

/// Generational and academic suffixes that follow the surname.
const NAME_SUFFIXES: &[&str] = &["jr", "sr", "ii", "iii", "iv", "phd", "md", "dr", "prof"];

/// The ASCII-folded, lower-case surname of an author string, or `None`
/// when nothing in it looks like one. Accepts `Given Surname`, `Surname,
/// Given`, `G. Surname Jr.` and `SURNAME Given`; rejects e-mail addresses,
/// `et al.` fragments and institution words.
pub fn surname_of(author: &str) -> Option<String> {
    let folded = fold_ascii(author);
    let cleaned: String = folded
        .chars()
        .map(|c| {
            if c.is_alphabetic() || c == ',' || c == '-' || c == '\'' {
                c
            } else {
                ' '
            }
        })
        .collect();
    if folded.contains('@') {
        return None;
    }
    let candidate = if let Some((before, _)) = cleaned.split_once(',') {
        // `Surname, Given`: the surname is the last word before the comma.
        before.split_whitespace().last().map(str::to_string)
    } else {
        let mut words: Vec<&str> = cleaned.split_whitespace().collect();
        while words
            .last()
            .is_some_and(|w| NAME_SUFFIXES.contains(&w.to_lowercase().trim_matches('-')))
        {
            words.pop();
        }
        // All-caps word among mixed-case words is a `SURNAME Given` style.
        let caps = words
            .iter()
            .find(|w| w.len() > 1 && w.chars().all(|c| !c.is_lowercase()));
        if words.len() > 1
            && caps.is_some()
            && words.iter().any(|w| w.chars().any(char::is_lowercase))
        {
            caps.map(|w| (*w).to_string())
        } else {
            words.last().map(|w| (*w).to_string())
        }
    }?;
    let surname: String = candidate
        .chars()
        .filter(|c| c.is_alphabetic())
        .flat_map(char::to_lowercase)
        .collect();
    if surname.chars().count() < 2 || NOT_SURNAMES.contains(&surname.as_str()) {
        return None;
    }
    Some(surname)
}

fn doi_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?i)\b10\.\d{4,9}/[^\s"<>]+"#).expect("valid regex"))
}

fn arxiv_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)arxiv:\s*(\d{4}\.\d{4,5}(?:v\d+)?|[a-z\-]+(?:\.[A-Z]{2})?/\d{7}(?:v\d+)?)")
            .expect("valid regex")
    })
}

/// First DOI printed in `text`, normalised.
pub fn doi_in_text(text: &str) -> Option<String> {
    doi_regex()
        .find_iter(text)
        .find_map(|m| normalize_doi(m.as_str()))
}

/// First `arXiv:` identifier printed in `text`, normalised.
pub fn arxiv_in_text(text: &str) -> Option<String> {
    arxiv_regex()
        .captures_iter(text)
        .find_map(|c| normalize_arxiv_id(c.get(1)?.as_str()))
}

/// DOI prefixes of preprint servers.
const PREPRINT_DOI_PREFIXES: &[&str] = &[
    "10.48550/", // arXiv
    "10.1101/",  // bioRxiv, medRxiv
    "10.2139/",  // SSRN
    "10.31219/", // OSF preprints
    "10.21203/", // Research Square
    "10.26434/", // ChemRxiv
    "10.20944/", // Preprints.org
    "10.22541/", // Authorea
    "10.31234/", // PsyArXiv
    "10.32942/", // EcoEvoRxiv
    "10.36227/", // TechRxiv
];

/// Phrases on page 1 that mark a preprint.
const PREPRINT_PHRASES: &[&str] = &[
    "arxiv:",
    "biorxiv preprint",
    "medrxiv preprint",
    "this is a preprint",
    "preprint not peer reviewed",
    "not peer-reviewed",
    "not been peer reviewed",
    "not been peer-reviewed",
    "preprint doi",
    "ssrn electronic journal",
    "research square",
    "preprint submitted to",
    "under review",
    "working paper",
];

/// Classify an input from its identifiers and the first page's text.
pub fn classify_variant(key: &BiblioKey, page1_text: &str) -> Variant {
    if key.arxiv_id.is_some() {
        return Variant::Preprint;
    }
    if let Some(doi) = &key.doi
        && PREPRINT_DOI_PREFIXES.iter().any(|p| doi.starts_with(p))
    {
        return Variant::Preprint;
    }
    let head: String = page1_text
        .chars()
        .take(4000)
        .collect::<String>()
        .to_lowercase();
    if PREPRINT_PHRASES.iter().any(|phrase| head.contains(phrase)) {
        return Variant::Preprint;
    }
    if key.doi.is_some() {
        return Variant::Published;
    }
    Variant::Unknown
}

/// Build the key for one document from the engine's metadata and the text
/// of its first page (used only to fill identifiers the metadata lacks).
pub fn key_of(metadata: &Metadata, page1_text: &str) -> BiblioKey {
    let mut key = BiblioKey::default();
    if let Some(doi) = metadata.doi.as_deref().and_then(normalize_doi) {
        key.doi = Some(doi);
        key.doi_source = Some(IdSource::Metadata);
    } else {
        if let Some(raw) = &metadata.doi {
            key.notes.push(format!("metadata doi rejected: {raw:?}"));
        }
        if let Some(doi) = doi_in_text(page1_text) {
            key.doi = Some(doi);
            key.doi_source = Some(IdSource::Text);
        }
    }
    if let Some(id) = metadata.arxiv_id.as_deref().and_then(normalize_arxiv_id) {
        key.arxiv_id = Some(id);
        key.arxiv_source = Some(IdSource::Metadata);
    } else if let Some(id) = arxiv_in_text(page1_text) {
        key.arxiv_id = Some(id);
        key.arxiv_source = Some(IdSource::Text);
    }
    if let Some(title) = metadata.title.as_deref().map(str::trim) {
        if title_is_plausible(title) {
            let normalized = normalize_title(title);
            if !normalized.is_empty() {
                key.title = Some(title.to_string());
                key.title_key = Some(normalized);
            }
        } else if !title.is_empty() {
            key.notes
                .push(format!("metadata title rejected: {title:?}"));
        }
    }
    if let Some(author) = metadata.authors.first() {
        let name = author.name.trim();
        match surname_of(name) {
            Some(surname) => {
                key.first_author = Some(name.to_string());
                key.surname = Some(surname);
            }
            None if !name.is_empty() => {
                key.notes.push(format!("first author rejected: {name:?}"));
            }
            None => {}
        }
    }
    key.year = metadata.year.filter(|y| (1800..=2100).contains(y));
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpe::schema::Author;

    #[test]
    fn folds_accents_and_special_letters() {
        assert_eq!(
            fold_ascii("Ångström Müller Łukasz Straße Søren"),
            "Angstrom Muller Lukasz Strasse Soren"
        );
    }

    #[test]
    fn title_normalisation_is_case_and_punctuation_insensitive() {
        assert_eq!(
            normalize_title("Deep   Learning: A Survey (2nd ed.)"),
            normalize_title("deep learning a survey 2nd ed")
        );
        assert_eq!(normalize_title("Élan—vital!"), "elan vital");
    }

    #[test]
    fn junk_titles_are_rejected() {
        for junk in [
            "Microsoft Word - paper_final.docx",
            "untitled",
            "Untitled Document",
            "draft.tex",
            "paper.pdf",
            "PowerPoint Presentation",
            "Title",
            "aaaa aaaa aaaa",
            "12345 67890",
            "A B",
            "Slide 1",
        ] {
            assert!(!title_is_plausible(junk), "{junk}");
        }
        assert!(title_is_plausible("Attention Is All You Need"));
        assert!(title_is_plausible(
            "On the Electrodynamics of Moving Bodies"
        ));
    }

    #[test]
    fn surname_parsing_tolerates_common_shapes() {
        assert_eq!(surname_of("Ada Lovelace").as_deref(), Some("lovelace"));
        assert_eq!(surname_of("Lovelace, Ada").as_deref(), Some("lovelace"));
        assert_eq!(
            surname_of("A. B. Lovelace Jr.").as_deref(),
            Some("lovelace")
        );
        assert_eq!(surname_of("LOVELACE Ada").as_deref(), Some("lovelace"));
        assert_eq!(surname_of("Jean-Luc Picard").as_deref(), Some("picard"));
        assert_eq!(surname_of("Müller, J.").as_deref(), Some("muller"));
        assert_eq!(surname_of("ada@example.org"), None);
        assert_eq!(surname_of("et al."), None);
        assert_eq!(surname_of("Stanford University"), None);
        assert_eq!(surname_of(""), None);
        assert_eq!(surname_of("X"), None);
    }

    #[test]
    fn identifiers_are_found_in_text() {
        let text = "Journal of Things 12(3) 2020\nhttps://doi.org/10.1234/ABC.567; arXiv:2101.00001v2 [cs.LG]";
        assert_eq!(doi_in_text(text).as_deref(), Some("10.1234/abc.567"));
        assert_eq!(arxiv_in_text(text).as_deref(), Some("2101.00001"));
        assert_eq!(doi_in_text("no identifiers here"), None);
        assert_eq!(arxiv_in_text("no identifiers here"), None);
    }

    fn metadata(title: &str, author: &str, year: Option<u16>, doi: Option<&str>) -> Metadata {
        Metadata {
            title: Some(title.to_string()),
            authors: vec![Author {
                name: author.to_string(),
                ..Author::default()
            }],
            doi: doi.map(str::to_string),
            year,
            ..Metadata::default()
        }
    }

    #[test]
    fn key_prefers_metadata_and_falls_back_to_text() {
        let key = key_of(
            &metadata(
                "A Study of Things",
                "Jane Q. Public",
                Some(2021),
                Some("https://doi.org/10.1000/XYZ"),
            ),
            "",
        );
        assert_eq!(key.doi.as_deref(), Some("10.1000/xyz"));
        assert_eq!(key.doi_source, Some(IdSource::Metadata));
        assert_eq!(key.surname.as_deref(), Some("public"));
        assert_eq!(key.title_key.as_deref(), Some("a study of things"));
        assert_eq!(key.year, Some(2021));

        let key = key_of(
            &metadata("Microsoft Word - final.docx", "admin", Some(1200), None),
            "Preprint. doi:10.1101/2020.01.01.123456 ...",
        );
        assert_eq!(key.doi.as_deref(), Some("10.1101/2020.01.01.123456"));
        assert_eq!(key.doi_source, Some(IdSource::Text));
        assert_eq!(key.title_key, None);
        assert_eq!(key.surname, None);
        assert_eq!(key.year, None);
        assert_eq!(key.notes.len(), 2, "{:?}", key.notes);
        assert_eq!(classify_variant(&key, ""), Variant::Preprint);
    }

    #[test]
    fn variant_classification() {
        let published = key_of(
            &metadata(
                "Some Real Paper Title",
                "A. Author",
                Some(2020),
                Some("10.1000/j.1"),
            ),
            "",
        );
        assert_eq!(
            classify_variant(&published, "Journal of X"),
            Variant::Published
        );
        let arxiv = BiblioKey {
            arxiv_id: Some("2101.00001".to_string()),
            ..BiblioKey::default()
        };
        assert_eq!(classify_variant(&arxiv, ""), Variant::Preprint);
        assert_eq!(
            classify_variant(&BiblioKey::default(), "bioRxiv preprint doi: ..."),
            Variant::Preprint
        );
        assert_eq!(
            classify_variant(&BiblioKey::default(), "plain text"),
            Variant::Unknown
        );
    }
}
