//! Zotero item-type inference from the engine's parsed reference fields.
//!
//! The engine records a venue string plus volume, issue, pages and
//! identifiers; it does not say whether the venue is a journal, a
//! proceedings volume or a publisher. The rules here are deliberately
//! conservative and ordered: thesis, report and preprint cues in the venue
//! first, then "In:" / editor cues (book section), conference cues,
//! publisher cues (book), and finally journal article. Without a venue, an
//! arXiv id makes a preprint, a bare URL a web page, publisher cues in the
//! raw entry a book, and anything else falls back to `preprint`, Zotero's
//! type for unreviewed works. The raw entry is always kept alongside, so a
//! wrong guess loses nothing.

use std::sync::LazyLock;

use regex::Regex;

/// The Zotero item types an export can produce.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ItemType {
    JournalArticle,
    Book,
    BookSection,
    ConferencePaper,
    Thesis,
    Report,
    Webpage,
    Preprint,
}

impl ItemType {
    /// The `itemType` name of the Zotero schema.
    pub fn zotero_name(self) -> &'static str {
        match self {
            Self::JournalArticle => "journalArticle",
            Self::Book => "book",
            Self::BookSection => "bookSection",
            Self::ConferencePaper => "conferencePaper",
            Self::Thesis => "thesis",
            Self::Report => "report",
            Self::Webpage => "webpage",
            Self::Preprint => "preprint",
        }
    }

    /// Every type, for documentation and tests.
    pub const ALL: [ItemType; 8] = [
        Self::JournalArticle,
        Self::Book,
        Self::BookSection,
        Self::ConferencePaper,
        Self::Thesis,
        Self::Report,
        Self::Webpage,
        Self::Preprint,
    ];
}

/// The engine fields the classifier looks at.
#[derive(Clone, Copy, Debug, Default)]
pub struct Hints<'a> {
    pub venue: Option<&'a str>,
    pub volume: Option<&'a str>,
    pub issue: Option<&'a str>,
    pub pages: Option<&'a str>,
    pub doi: Option<&'a str>,
    pub arxiv_id: Option<&'a str>,
    pub url: Option<&'a str>,
    pub raw: Option<&'a str>,
}

/// The inferred type and the type-specific fields derived from the venue.
/// Field names follow Zotero's base fields: `publication_title` is
/// `publicationTitle` (journal, proceedings, book or website title),
/// `publisher` covers `publisher`, `university`, `institution` and
/// `repository`, `type_field` covers `thesisType`, `reportType` and
/// `genre`, and `number` covers `reportNumber` and `archiveID`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Classification {
    pub item_type: ItemType,
    pub publication_title: Option<String>,
    pub publisher: Option<String>,
    pub type_field: Option<String>,
    pub number: Option<String>,
}

impl Classification {
    fn plain(item_type: ItemType) -> Self {
        Self {
            item_type,
            publication_title: None,
            publisher: None,
            type_field: None,
            number: None,
        }
    }
}

static THESIS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:(?:ph\.?\s?d\.?|d\.?\s?phil\.?|doctoral|doctor's|master'?s?|m\.?\s?sc\.?|m\.?\s?a\.?|m\.?\s?s\.?|bachelor'?s?|b\.?\s?sc\.?|b\.?\s?a\.?|diploma|habilitation|honou?rs)\s+)?(?:thesis|dissertation)\b",
    )
    .expect("thesis regex")
});

static REPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:tech(?:nical)?\.?\s+rep(?:ort)?\.?|research\s+report|white\s+paper|working\s+paper|internal\s+report|report\s+(?:no|number)\b\.?|rfc\s*\d+)",
    )
    .expect("report regex")
});

static REPORT_NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:tech(?:nical)?\.?\s+rep(?:ort)?\.?|report)\s*(?:no\.?|number|#)?\s*:?\s*([A-Za-z]*[-/]?\d[A-Za-z0-9.\-/]*)",
    )
    .expect("report number regex")
});

static PREPRINT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:arxiv|preprint|biorxiv|medrxiv|ssrn|chemrxiv|psyarxiv|eartharxiv|hal)\b")
        .expect("preprint regex")
});

static BOOK_SECTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^in:?\s|\((?:eds?\.?|editors?)\)|\beds?\.\s|\beditors?\b|\bedited by\b")
        .expect("book section regex")
});

static CONFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\bproceedings\b|\bproc\b|\bconference\b|\bconf\b|\bsymposium\b|\bworkshop\b|\bmeeting\b|\bcongress\b|\bcolloquium\b|\bconvention\b",
    )
    .expect("conference regex")
});

static CONFERENCE_ACRONYM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b(?:NeurIPS|NIPS|ICML|ICLR|CVPR|ICCV|ECCV|AAAI|IJCAI|ACL|EMNLP|NAACL|COLING|SIGIR|KDD|WWW|CHI|UIST|SIGGRAPH|SIGMOD|VLDB|ICDE|SOSP|OSDI|NSDI|USENIX|ICSE|PLDI|POPL|OOPSLA|ICRA|IROS|INTERSPEECH|ICASSP|AISTATS|UAI|COLT|STOC|FOCS|SODA|CRYPTO|EUROCRYPT|CCS|NDSS)\b",
    )
    .expect("conference acronym regex")
});

static PUBLISHER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:press|publishers?|publishing|publications|verlag|springer|wiley|elsevier|pergamon|mcgraw|prentice|addison|o'reilly|routledge|palgrave|sage|blackwell|birkh[äa]user|de gruyter|crc|penguin|dover|morgan kaufmann|north-holland|kluwer|plenum|butterworth|chapman)\b|\b(?:\d+(?:st|nd|rd|th)|first|second|third|fourth|fifth|revised|new)\s+ed(?:ition|\.)",
    )
    .expect("publisher regex")
});

/// Infers the Zotero item type and venue-derived fields for one entry.
pub fn classify(hints: &Hints<'_>) -> Classification {
    let venue = hints.venue.map(str::trim).filter(|v| !v.is_empty());
    let arxiv = hints
        .arxiv_id
        .map(normalize_arxiv)
        .filter(|a| !a.is_empty());
    if let Some(venue) = venue {
        if let Some(m) = THESIS.find(venue) {
            return Classification {
                item_type: ItemType::Thesis,
                publication_title: None,
                publisher: remainder(venue, m.range()),
                type_field: Some(m.as_str().to_string()),
                number: None,
            };
        }
        if let Some(m) = REPORT.find(venue) {
            let number = REPORT_NUMBER
                .captures(venue)
                .map(|c| c[1].to_string())
                .filter(|n| !n.is_empty());
            let mut publisher = remainder(venue, m.range());
            if let (Some(number), Some(rest)) = (&number, &publisher) {
                publisher = strip_token(rest, number);
            }
            return Classification {
                item_type: ItemType::Report,
                publication_title: None,
                publisher,
                type_field: Some(m.as_str().to_string()),
                number,
            };
        }
        if PREPRINT.is_match(venue) {
            return preprint(arxiv.as_deref());
        }
        if BOOK_SECTION.is_match(venue) {
            let title = venue
                .strip_prefix("In:")
                .or_else(|| venue.strip_prefix("in:"))
                .or_else(|| venue.strip_prefix("In "))
                .or_else(|| venue.strip_prefix("in "))
                .unwrap_or(venue)
                .trim();
            return Classification {
                item_type: ItemType::BookSection,
                publication_title: Some(title.to_string()),
                publisher: None,
                type_field: None,
                number: None,
            };
        }
        if CONFERENCE.is_match(venue) || CONFERENCE_ACRONYM.is_match(venue) {
            return Classification {
                item_type: ItemType::ConferencePaper,
                publication_title: Some(venue.to_string()),
                publisher: None,
                type_field: None,
                number: None,
            };
        }
        if PUBLISHER.is_match(venue) && hints.volume.is_none() && hints.issue.is_none() {
            return Classification {
                item_type: ItemType::Book,
                publication_title: None,
                publisher: Some(venue.to_string()),
                type_field: None,
                number: None,
            };
        }
        return Classification {
            item_type: ItemType::JournalArticle,
            publication_title: Some(venue.to_string()),
            publisher: None,
            type_field: None,
            number: None,
        };
    }
    if arxiv.is_some() {
        return preprint(arxiv.as_deref());
    }
    if hints.doi.is_none() && hints.url.is_some_and(is_http_url) {
        return Classification::plain(ItemType::Webpage);
    }
    if let Some(raw) = hints.raw {
        if let Some(m) = THESIS.find(raw) {
            let mut thesis = Classification::plain(ItemType::Thesis);
            thesis.type_field = Some(m.as_str().to_string());
            return thesis;
        }
        if let Some(m) = REPORT.find(raw) {
            let mut report = Classification::plain(ItemType::Report);
            report.type_field = Some(m.as_str().to_string());
            return report;
        }
        if PUBLISHER.is_match(raw) {
            return Classification::plain(ItemType::Book);
        }
    }
    Classification::plain(ItemType::Preprint)
}

fn preprint(arxiv: Option<&str>) -> Classification {
    Classification {
        item_type: ItemType::Preprint,
        publication_title: None,
        publisher: arxiv.map(|_| "arXiv".to_string()),
        type_field: None,
        number: arxiv.map(|id| format!("arXiv:{id}")),
    }
}

/// `arXiv:2101.00001v2` → `2101.00001v2`; the engine stores ids without the
/// scheme but printed entries sometimes carry it.
pub fn normalize_arxiv(id: &str) -> String {
    let trimmed = id.trim();
    let lower = trimmed.to_ascii_lowercase();
    let rest = if lower.starts_with("arxiv:") {
        &trimmed[6..]
    } else {
        trimmed
    };
    rest.trim().to_string()
}

/// Whether `url` is an absolute http(s) URL without whitespace or XML
/// delimiters, so it can stand as an RDF resource identifier.
pub fn is_http_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://"))
        && url.len() > 8
        && !url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '"'))
}

/// `venue` without the matched phrase, trimmed of separators; `None` when
/// nothing is left.
fn remainder(venue: &str, range: std::ops::Range<usize>) -> Option<String> {
    let mut rest = String::new();
    rest.push_str(&venue[..range.start]);
    rest.push(' ');
    rest.push_str(&venue[range.end..]);
    let trimmed = rest.trim_matches(|c: char| {
        c.is_whitespace() || matches!(c, ',' | '.' | ';' | ':' | '-' | '(' | ')')
    });
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.split_whitespace().collect::<Vec<_>>().join(" "))
    }
}

fn strip_token(text: &str, token: &str) -> Option<String> {
    let stripped = text.replacen(token, "", 1);
    let trimmed = stripped.trim_matches(|c: char| {
        c.is_whitespace() || matches!(c, ',' | '.' | ';' | ':' | '-' | '#')
    });
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.split_whitespace().collect::<Vec<_>>().join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_venue(venue: &str) -> Classification {
        classify(&Hints {
            venue: Some(venue),
            ..Hints::default()
        })
    }

    #[test]
    fn journal_is_the_default_for_a_venue() {
        let c = with_venue("Journal of Important Results");
        assert_eq!(c.item_type, ItemType::JournalArticle);
        assert_eq!(
            c.publication_title.as_deref(),
            Some("Journal of Important Results")
        );
    }

    #[test]
    fn thesis_and_report_split_type_and_institution() {
        let c = with_venue("PhD thesis, Massachusetts Institute of Technology");
        assert_eq!(c.item_type, ItemType::Thesis);
        assert_eq!(c.type_field.as_deref(), Some("PhD thesis"));
        assert_eq!(
            c.publisher.as_deref(),
            Some("Massachusetts Institute of Technology")
        );

        let c = with_venue("Technical Report TR-2020-07, Example University");
        assert_eq!(c.item_type, ItemType::Report);
        assert_eq!(c.type_field.as_deref(), Some("Technical Report"));
        assert_eq!(c.number.as_deref(), Some("TR-2020-07"));
        assert_eq!(c.publisher.as_deref(), Some("Example University"));
    }

    #[test]
    fn section_conference_book_and_preprint_cues() {
        let c = with_venue("In: Handbook of Things, Smith (Ed.)");
        assert_eq!(c.item_type, ItemType::BookSection);
        assert_eq!(
            c.publication_title.as_deref(),
            Some("Handbook of Things, Smith (Ed.)")
        );

        let c = with_venue("Proceedings of the 10th Conference on Examples");
        assert_eq!(c.item_type, ItemType::ConferencePaper);
        assert_eq!(with_venue("NeurIPS").item_type, ItemType::ConferencePaper);

        let c = with_venue("Cambridge University Press");
        assert_eq!(c.item_type, ItemType::Book);
        assert_eq!(c.publisher.as_deref(), Some("Cambridge University Press"));

        let c = classify(&Hints {
            venue: Some("arXiv preprint"),
            arxiv_id: Some("arXiv:2101.00001"),
            ..Hints::default()
        });
        assert_eq!(c.item_type, ItemType::Preprint);
        assert_eq!(c.number.as_deref(), Some("arXiv:2101.00001"));
        assert_eq!(c.publisher.as_deref(), Some("arXiv"));
    }

    #[test]
    fn without_a_venue() {
        let c = classify(&Hints {
            arxiv_id: Some("2101.00001"),
            ..Hints::default()
        });
        assert_eq!(c.item_type, ItemType::Preprint);
        let c = classify(&Hints {
            url: Some("https://example.org/page"),
            ..Hints::default()
        });
        assert_eq!(c.item_type, ItemType::Webpage);
        let c = classify(&Hints {
            raw: Some("Knuth, D. The Art of Computer Programming. Addison-Wesley, 1968."),
            ..Hints::default()
        });
        assert_eq!(c.item_type, ItemType::Book);
        let c = classify(&Hints {
            raw: Some("Doe, J. Some title. 2001."),
            ..Hints::default()
        });
        assert_eq!(c.item_type, ItemType::Preprint);
        let c = classify(&Hints {
            doi: Some("10.1000/x"),
            url: Some("https://doi.org/10.1000/x"),
            ..Hints::default()
        });
        assert_eq!(c.item_type, ItemType::Preprint);
    }

    #[test]
    fn url_check_rejects_unsafe_values() {
        assert!(is_http_url("https://example.org/a?b=c&d=e"));
        assert!(!is_http_url("example.org"));
        assert!(!is_http_url("https://example.org/a b"));
        assert!(!is_http_url("https://example.org/\"x"));
    }
}
