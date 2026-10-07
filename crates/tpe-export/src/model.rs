//! The export model: Zotero-shaped items built from the engine's records.
//!
//! Item 1 is the citing article; items 2.. are its reference entries in
//! list order. Field names follow Zotero's base fields so the writers can
//! treat every item type the same way (`docs/EXPORT.md`, "Field mapping").

use tpe::schema::{Author, ReferenceEntry, sha256_hex};

use crate::classify::{Classification, Hints, ItemType, classify, is_http_url, normalize_arxiv};
use crate::input::Article;
use crate::names::split_name;

/// Zotero's object-key alphabet (`Zotero.Utilities.generateObjectKey`).
const KEY_ALPHABET: &[u8] = b"23456789ABCDEFGHIJKLMNPQRSTUVWXYZ";

/// One creator; every creator the engine yields is an `author`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Creator {
    pub last_name: String,
    pub first_name: Option<String>,
}

impl Creator {
    /// `Last, First` or `Last`, as the CSV translator prints creators.
    pub fn display(&self) -> String {
        match &self.first_name {
            Some(first) => format!("{}, {first}", self.last_name),
            None => self.last_name.clone(),
        }
    }
}

/// One Zotero item. Every field is `None` or empty unless the engine
/// recorded it; nothing is guessed except the item type and the fields
/// `classify` derives from the venue string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// 1 for the citing article, `reference index + 1` for cited works.
    pub id: u32,
    /// Deterministic 8-character key in Zotero's key alphabet.
    pub key: String,
    pub item_type: ItemType,
    pub title: Option<String>,
    pub creators: Vec<Creator>,
    pub abstract_note: Option<String>,
    /// Zotero base field `publicationTitle`: journal, proceedings, book or
    /// website title depending on the type.
    pub publication_title: Option<String>,
    /// Zotero base field `publisher` (`university`, `institution`,
    /// `repository` for the respective types).
    pub publisher: Option<String>,
    /// Zotero base field `type` (`thesisType`, `reportType`, `genre`).
    pub type_field: Option<String>,
    /// Zotero base field `number` (`reportNumber`, `archiveID`).
    pub number: Option<String>,
    /// Publication date; the engine only knows the year, so `YYYY`.
    pub date: Option<String>,
    pub volume: Option<String>,
    pub issue: Option<String>,
    pub pages: Option<String>,
    pub doi: Option<String>,
    /// Never produced by the engine; carried so the writers cover Zotero's
    /// container identifiers completely.
    pub issn: Option<String>,
    pub isbn: Option<String>,
    pub url: Option<String>,
    pub pmid: Option<String>,
    pub pmcid: Option<String>,
    pub arxiv_id: Option<String>,
    /// Zotero `extra`: `Label: value` lines (see `extra_lines`).
    pub extra: Option<String>,
    /// Manual tags (the paper's keywords).
    pub tags: Vec<String>,
    /// The printed reference entry, verbatim.
    pub raw: Option<String>,
    /// 1-based position in the reference list; `None` for the article.
    pub reference_index: Option<u32>,
    /// Label as printed, e.g. `[12]`.
    pub label: Option<String>,
    /// Ids of related items (Zotero relations are bidirectional).
    pub related: Vec<u32>,
}

impl Item {
    /// The four-digit year of `date`, when it has one.
    pub fn year(&self) -> Option<u16> {
        let date = self.date.as_deref()?;
        let digits: String = date.chars().take(4).collect();
        if digits.len() == 4 && digits.chars().all(|c| c.is_ascii_digit()) {
            digits.parse().ok()
        } else {
            None
        }
    }
}

/// The citing article and the works it cites.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    /// SHA-256 of the source document when the input carried it.
    pub source_hash: Option<String>,
    /// `items[0]` is the citing article; the rest are its references.
    pub items: Vec<Item>,
}

impl Export {
    /// Maps an [`Article`] onto Zotero items.
    pub fn from_article(article: &Article) -> Self {
        let hash = article.source_hash.as_deref();
        let count = article.references.len();
        let mut items = Vec::with_capacity(count + 1);
        items.push(article_item(article, hash));
        for entry in &article.references {
            items.push(reference_item(entry, hash));
        }
        Self {
            source_hash: article.source_hash.clone(),
            items,
        }
    }

    /// The citing article.
    pub fn citing(&self) -> &Item {
        &self.items[0]
    }

    /// The cited works in reference-list order.
    pub fn cited(&self) -> &[Item] {
        &self.items[1..]
    }
}

/// A deterministic Zotero-style key: eight characters of the key alphabet
/// drawn from SHA-256 of the document hash and the item id.
pub fn item_key(source_hash: Option<&str>, id: u32) -> String {
    let seed = format!("{}\n{id}", source_hash.unwrap_or(""));
    let digest = sha256_hex(seed.as_bytes());
    let bytes = digest.as_bytes();
    (0..8)
        .map(|i| {
            let pair = std::str::from_utf8(&bytes[2 * i..2 * i + 2]).unwrap_or("00");
            let byte = u8::from_str_radix(pair, 16).unwrap_or(0);
            KEY_ALPHABET[usize::from(byte % 32)] as char
        })
        .collect()
}

fn article_item(article: &Article, hash: Option<&str>) -> Item {
    let meta = &article.metadata;
    let classification = classify(&Hints {
        venue: meta.venue.as_deref(),
        doi: meta.doi.as_deref(),
        arxiv_id: meta.arxiv_id.as_deref(),
        ..Hints::default()
    });
    let arxiv_id = meta.arxiv_id.as_deref().map(normalize_arxiv);
    let mut extra = Vec::new();
    if classification.item_type != ItemType::Preprint
        && let Some(id) = &arxiv_id
    {
        extra.push(format!("arXiv: {id}"));
    }
    if let Some(hash) = hash {
        extra.push(format!("Source SHA-256: {hash}"));
    }
    let related = (2..=u32::try_from(article.references.len() + 1).unwrap_or(u32::MAX)).collect();
    let mut item = base_item(1, hash, classification, related);
    item.title = clean(meta.title.as_deref());
    item.creators = meta.authors.iter().filter_map(author_creator).collect();
    item.abstract_note = clean(meta.abstract_text.as_deref());
    item.date = meta.year.map(|y| y.to_string());
    item.doi = clean(meta.doi.as_deref());
    item.arxiv_id = arxiv_id;
    item.extra = lines(&extra);
    item.tags = meta
        .keywords
        .iter()
        .filter_map(|k| clean(Some(k)))
        .collect();
    item
}

fn reference_item(entry: &ReferenceEntry, hash: Option<&str>) -> Item {
    let resolved = entry.resolved.as_ref();
    let doi = clean(entry.doi.as_deref())
        .or_else(|| clean(entry.doi_link.as_deref()))
        .or_else(|| resolved.and_then(|r| clean(r.doi.as_deref())));
    let title =
        clean(entry.title.as_deref()).or_else(|| resolved.and_then(|r| clean(r.title.as_deref())));
    let year = entry.year.or_else(|| resolved.and_then(|r| r.year));
    let venue =
        clean(entry.venue.as_deref()).or_else(|| resolved.and_then(|r| clean(r.venue.as_deref())));
    let printed_authors: Vec<&str> = entry.authors.iter().map(String::as_str).collect();
    let authors: Vec<&str> = if printed_authors.is_empty() {
        resolved
            .map(|r| r.authors.iter().map(String::as_str).collect())
            .unwrap_or_default()
    } else {
        printed_authors
    };
    let classification = classify(&Hints {
        venue: venue.as_deref(),
        volume: entry.volume.as_deref(),
        issue: entry.issue.as_deref(),
        pages: entry.pages.as_deref(),
        doi: doi.as_deref(),
        arxiv_id: entry.arxiv_id.as_deref(),
        url: entry.url.as_deref(),
        raw: Some(entry.raw.as_str()),
    });
    let arxiv_id = entry
        .arxiv_id
        .as_deref()
        .map(normalize_arxiv)
        .filter(|a| !a.is_empty());
    let pubmed_id = resolved.and_then(|r| clean(r.pmid.as_deref()));
    let pmc_id = resolved.and_then(|r| clean(r.pmcid.as_deref()));
    let mut extra = Vec::new();
    if classification.item_type != ItemType::Preprint
        && let Some(id) = &arxiv_id
    {
        extra.push(format!("arXiv: {id}"));
    }
    if classification.item_type != ItemType::JournalArticle {
        if let Some(id) = &pubmed_id {
            extra.push(format!("PMID: {id}"));
        }
        if let Some(id) = &pmc_id {
            extra.push(format!("PMCID: {id}"));
        }
    }
    extra.push(format!("Reference index: {}", entry.index));
    if let Some(label) = clean(entry.label.as_deref()) {
        extra.push(format!("Reference label: {label}"));
    }
    if let Some(raw) = clean(Some(&entry.raw)) {
        extra.push(format!("Reference text: {raw}"));
    }
    let id = entry.index.saturating_add(1);
    let mut item = base_item(id, hash, classification, vec![1]);
    item.title = title;
    item.creators = authors
        .iter()
        .filter_map(|name| split_name(name))
        .map(creator)
        .collect();
    item.date = year.map(|y| y.to_string());
    item.volume = clean(entry.volume.as_deref());
    item.issue = clean(entry.issue.as_deref());
    item.pages = clean(entry.pages.as_deref());
    item.doi = doi;
    item.url = clean(entry.url.as_deref()).filter(|u| is_http_url(u));
    item.pmid = pubmed_id;
    item.pmcid = pmc_id;
    item.arxiv_id = arxiv_id;
    item.extra = lines(&extra);
    item.raw = clean(Some(&entry.raw));
    item.reference_index = Some(entry.index);
    item.label = clean(entry.label.as_deref());
    item
}

fn base_item(
    id: u32,
    hash: Option<&str>,
    classification: Classification,
    related: Vec<u32>,
) -> Item {
    Item {
        id,
        key: item_key(hash, id),
        item_type: classification.item_type,
        title: None,
        creators: Vec::new(),
        abstract_note: None,
        publication_title: classification.publication_title,
        publisher: classification.publisher,
        type_field: classification.type_field,
        number: classification.number,
        date: None,
        volume: None,
        issue: None,
        pages: None,
        doi: None,
        issn: None,
        isbn: None,
        url: None,
        pmid: None,
        pmcid: None,
        arxiv_id: None,
        extra: None,
        tags: Vec::new(),
        raw: None,
        reference_index: None,
        label: None,
        related,
    }
}

fn author_creator(author: &Author) -> Option<Creator> {
    split_name(&author.name).map(creator)
}

fn creator(name: crate::names::SplitName) -> Creator {
    Creator {
        last_name: name.last,
        first_name: name.first,
    }
}

/// Trims and collapses whitespace; `None` when nothing is left.
fn clean(value: Option<&str>) -> Option<String> {
    let collapsed = value?.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}

fn lines(parts: &[String]) -> Option<String> {
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_deterministic_and_in_zoteros_alphabet() {
        let a = item_key(Some("abc"), 1);
        let b = item_key(Some("abc"), 1);
        assert_eq!(a, b);
        assert_ne!(a, item_key(Some("abc"), 2));
        assert_ne!(a, item_key(Some("abd"), 1));
        assert_eq!(a.len(), 8);
        assert!(a.bytes().all(|b| KEY_ALPHABET.contains(&b)));
    }

    #[test]
    fn year_reads_only_four_digit_dates() {
        let mut item = base_item(
            1,
            None,
            Classification {
                item_type: ItemType::Preprint,
                publication_title: None,
                publisher: None,
                type_field: None,
                number: None,
            },
            vec![],
        );
        assert_eq!(item.year(), None);
        item.date = Some("2019".into());
        assert_eq!(item.year(), Some(2019));
        item.date = Some("n.d.".into());
        assert_eq!(item.year(), None);
    }
}
