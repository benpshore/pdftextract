//! Crossref REST API: `/works` search and DOI lookup.
//!
//! Response shape (public Crossref REST API; assumed, see crate docs):
//!
//! ```text
//! list:   { "status": "ok", "message-type": "work-list", "message": { "items": [work...] } }
//! single: { "status": "ok", "message-type": "work", "message": work }
//! work:   { "title": [..], "author": [ { "given", "family", "name" } ],
//!           "issued": { "date-parts": [[y, m, d]] }, "published", "published-print",
//!           "published-online", "container-title": [..], "DOI", "URL",
//!           "abstract" (JATS markup), "link": [ { "URL", "content-type" } ],
//!           "license": [ { "URL", "delay-in-days", "content-version" } ] }
//! ```
//!
//! A Crossref `link` of type `application/pdf` is often a publisher URL that
//! needs a subscription, so `requires_session` is derived conservatively: it is
//! `false` only when the work is known to be open access, that is when a
//! `license` entry points at a Creative Commons URL with no embargo
//! (`delay-in-days` absent or zero), or when the link's host is a known
//! open-access host ([`OPEN_ACCESS_HOSTS`] or a subdomain of one). Every other
//! link is marked `true`.

use serde_json::Value;
use tpe_common::{PaperRecord, normalize_doi};

use crate::client::Client;
use crate::error::BiblioError;
use crate::fetch::Fetcher;
use crate::util::{
    array, arxiv_from_doi, encode_path, first_str, host_of, str_field, strip_tags, with_query,
    year_of,
};
use crate::{CandidateKind, Found, FullTextCandidate, push_unique};

/// API base URL.
pub const BASE: &str = "https://api.crossref.org";

/// Hosts (and their subdomains) that serve full text without a subscription.
pub const OPEN_ACCESS_HOSTS: [&str; 3] = ["arxiv.org", "europepmc.org", "ncbi.nlm.nih.gov"];

/// `GET /works?query=<q>&rows=<n>&mailto=<m>`.
pub fn search_url(query: &str, rows: u32, mailto: Option<&str>) -> String {
    let n = rows.clamp(1, 1000).to_string();
    let mut pairs: Vec<(&str, &str)> = vec![("query", query), ("rows", n.as_str())];
    if let Some(m) = mailto {
        pairs.push(("mailto", m));
    }
    with_query(&format!("{BASE}/works"), &pairs)
}

/// `GET /works/<doi>?mailto=<m>`.
pub fn doi_url(doi: &str, mailto: Option<&str>) -> String {
    let path = format!("{BASE}/works/{}", encode_path(doi));
    if let Some(m) = mailto {
        with_query(&path, &[("mailto", m)])
    } else {
        path
    }
}

/// Parse a work list or a single work into records.
pub fn parse_crossref(json: &str) -> Result<Vec<PaperRecord>, BiblioError> {
    Ok(parse_crossref_found(json)?
        .into_iter()
        .map(|f| f.record)
        .collect())
}

/// Like [`parse_crossref`] but keeps `link[]` entries of type `application/pdf`.
pub fn parse_crossref_found(json: &str) -> Result<Vec<Found>, BiblioError> {
    let root: Value = serde_json::from_str(json)?;
    let Some(message) = root.get("message") else {
        return Err(BiblioError::Shape("Crossref: no `message`".to_string()));
    };
    if let Some(items) = message.get("items").and_then(Value::as_array) {
        return Ok(items.iter().map(item_to_found).collect());
    }
    if message.get("DOI").is_some() {
        return Ok(vec![item_to_found(message)]);
    }
    Err(BiblioError::Shape(
        "Crossref: `message` has neither `items` nor `DOI`".to_string(),
    ))
}

fn author_name(a: &Value) -> Option<String> {
    let given = str_field(a, "given");
    let family = str_field(a, "family");
    match (given, family) {
        (Some(g), Some(f)) => Some(format!("{g} {f}")),
        (None, Some(f)) => Some(f),
        (given, None) => str_field(a, "name").or(given),
    }
}

fn year(item: &Value) -> Option<u16> {
    ["issued", "published", "published-print", "published-online"]
        .iter()
        .find_map(|key| {
            let parts = item.get(*key)?.get("date-parts")?.get(0)?.get(0)?;
            year_of(parts)
        })
}

fn abstract_text(item: &Value) -> Option<String> {
    let raw = str_field(item, "abstract")?;
    let text = strip_tags(&raw);
    let body = text.strip_prefix("Abstract ").unwrap_or(&text);
    crate::util::non_empty(body)
}

/// True when `host` is `domain` or a subdomain of it.
fn host_is(host: &str, domain: &str) -> bool {
    host.strip_suffix(domain)
        .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('.'))
}

/// True when a `license` entry is a Creative Commons licence with no embargo.
fn has_open_license(item: &Value) -> bool {
    array(item, "license").iter().any(|license| {
        let delay = license
            .get("delay-in-days")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        delay <= 0
            && str_field(license, "URL")
                .is_some_and(|url| host_is(&host_of(&url), "creativecommons.org"))
    })
}

/// Conservative `requires_session`: `false` only for known open access (see module docs).
fn requires_session(url: &str, open_license: bool) -> bool {
    if open_license {
        return false;
    }
    let host = host_of(url);
    !OPEN_ACCESS_HOSTS
        .iter()
        .any(|domain| host_is(&host, domain))
}

fn pdf_links(item: &Value) -> Vec<FullTextCandidate> {
    let open_license = has_open_license(item);
    let mut out: Vec<FullTextCandidate> = Vec::new();
    for link in array(item, "link") {
        if str_field(link, "content-type").as_deref() != Some("application/pdf") {
            continue;
        }
        if let Some(url) = str_field(link, "URL") {
            let session = requires_session(&url, open_license);
            push_unique(
                &mut out,
                FullTextCandidate::new(&url, "crossref", CandidateKind::Pdf, session),
            );
        }
    }
    out
}

fn item_to_found(item: &Value) -> Found {
    let doi = str_field(item, "DOI").and_then(|d| normalize_doi(&d));
    let record = PaperRecord {
        title: first_str(item, "title").unwrap_or_default(),
        authors: array(item, "author")
            .iter()
            .filter_map(author_name)
            .collect(),
        year: year(item),
        venue: first_str(item, "container-title"),
        arxiv_id: doi.as_deref().and_then(arxiv_from_doi),
        source_id: doi.clone(),
        doi,
        url: str_field(item, "URL"),
        abstract_text: abstract_text(item),
        source: "crossref".to_string(),
        ..PaperRecord::default()
    };
    Found {
        record,
        candidates: pdf_links(item),
    }
}

/// Search works by free text (`query`).
pub fn fetch_search(client: &Client, query: &str, rows: u32) -> Result<Vec<Found>, BiblioError> {
    let url = search_url(query, rows, client.mailto());
    parse_crossref_found(&client.get_text(&url, &[])?)
}

/// Look up one DOI; `Ok(None)` when Crossref answers 404.
pub fn fetch_by_doi(client: &Client, doi: &str) -> Result<Option<Found>, BiblioError> {
    let url = doi_url(doi, client.mailto());
    match client.get_text(&url, &[]) {
        Ok(body) => Ok(parse_crossref_found(&body)?.into_iter().next()),
        Err(BiblioError::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}

/// `GET /works?query.bibliographic=<text>&rows=<n>&mailto=<m>`: the query
/// field meant for whole citation strings (author, title, venue and year
/// weighed together), unlike the plain `query`.
pub fn bibliographic_url(text: &str, rows: u32, mailto: Option<&str>) -> String {
    let n = rows.clamp(1, 1000).to_string();
    let mut pairs: Vec<(&str, &str)> = vec![("query.bibliographic", text), ("rows", n.as_str())];
    if let Some(m) = mailto {
        pairs.push(("mailto", m));
    }
    with_query(&format!("{BASE}/works"), &pairs)
}

/// Search works by a whole citation string (`query.bibliographic`).
pub fn fetch_bibliographic(
    client: &Client,
    text: &str,
    rows: u32,
) -> Result<Vec<Found>, BiblioError> {
    let url = bibliographic_url(text, rows, client.mailto());
    parse_crossref_found(&client.get_text(&url, &[])?)
}

/// A date Crossref gives as `date-parts`: year, optional month and day.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PartialDate {
    pub year: u16,
    pub month: Option<u8>,
    pub day: Option<u8>,
}

impl PartialDate {
    /// `YYYY`, `YYYY-MM` or `YYYY-MM-DD`.
    pub fn to_iso(self) -> String {
        match (self.month, self.day) {
            (Some(m), Some(d)) => format!("{:04}-{m:02}-{d:02}", self.year),
            (Some(m), None) => format!("{:04}-{m:02}", self.year),
            _ => format!("{:04}", self.year),
        }
    }
}

/// One `author` entry as Crossref records it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contributor {
    pub given: Option<String>,
    pub family: Option<String>,
    /// Organisation name, for group authors.
    pub name: Option<String>,
    /// ORCID URL as given (`https://orcid.org/0000-...`).
    pub orcid: Option<String>,
    /// `first` or `additional`.
    pub sequence: Option<String>,
    pub affiliations: Vec<String>,
}

impl Contributor {
    /// `Given Family`, or the family or organisation name alone.
    pub fn display_name(&self) -> Option<String> {
        match (&self.given, &self.family) {
            (Some(g), Some(f)) => Some(format!("{g} {f}")),
            (None, Some(f)) => Some(f.clone()),
            (given, None) => self.name.clone().or_else(|| given.clone()),
        }
    }
}

/// One `license` entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct License {
    pub url: String,
    pub start: Option<PartialDate>,
    pub delay_in_days: Option<i64>,
    /// `vor`, `am`, `tdm` or `unspecified`.
    pub content_version: Option<String>,
}

impl License {
    /// True for a Creative Commons licence with no embargo.
    pub fn is_open(&self) -> bool {
        self.delay_in_days.unwrap_or(0) <= 0 && host_is(&host_of(&self.url), "creativecommons.org")
    }
}

/// One `funder` entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Funder {
    pub name: String,
    /// Open Funder Registry DOI (`10.13039/...`), when asserted.
    pub doi: Option<String>,
    pub awards: Vec<String>,
}

/// An ISSN with the kind Crossref's `issn-type` gives it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Issn {
    pub value: String,
    /// `print`, `electronic` or `unknown`.
    pub kind: String,
}

/// The fields of a Crossref work that describe the publication and its
/// publisher, beyond the normalised [`PaperRecord`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CrossrefWork {
    pub doi: Option<String>,
    /// `journal-article`, `book-chapter`, `proceedings-article`, `posted-content`, ...
    pub work_type: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub authors: Vec<Contributor>,
    pub container_title: Option<String>,
    pub short_container_title: Option<String>,
    pub publisher: Option<String>,
    /// Crossref member id of the publisher.
    pub member: Option<String>,
    pub issn: Vec<Issn>,
    pub isbn: Vec<String>,
    pub volume: Option<String>,
    pub issue: Option<String>,
    pub page: Option<String>,
    pub article_number: Option<String>,
    pub issued: Option<PartialDate>,
    pub published_online: Option<PartialDate>,
    pub published_print: Option<PartialDate>,
    pub licenses: Vec<License>,
    pub funders: Vec<Funder>,
    pub url: Option<String>,
    pub language: Option<String>,
    pub subjects: Vec<String>,
    pub references_count: Option<u64>,
    pub is_referenced_by_count: Option<u64>,
    pub abstract_text: Option<String>,
}

fn partial_date(v: &Value) -> Option<PartialDate> {
    let parts = v.get("date-parts")?.get(0)?.as_array()?;
    let year = parts.first().and_then(year_of)?;
    let small = |i: usize, max: u8| {
        parts
            .get(i)
            .and_then(Value::as_u64)
            .and_then(|n| u8::try_from(n).ok())
            .filter(|n| (1..=max).contains(n))
    };
    Some(PartialDate {
        year,
        month: small(1, 12),
        day: small(2, 31),
    })
}

fn contributor(a: &Value) -> Contributor {
    Contributor {
        given: str_field(a, "given"),
        family: str_field(a, "family"),
        name: str_field(a, "name"),
        orcid: str_field(a, "ORCID"),
        sequence: str_field(a, "sequence"),
        affiliations: array(a, "affiliation")
            .iter()
            .filter_map(|x| str_field(x, "name"))
            .collect(),
    }
}

fn issns(item: &Value) -> Vec<Issn> {
    let mut out: Vec<Issn> = Vec::new();
    for typed in array(item, "issn-type") {
        if let Some(value) = str_field(typed, "value")
            && let Some(value) = crate::identifiers::normalize_issn(&value)
            && !out.iter().any(|i| i.value == value)
        {
            out.push(Issn {
                value,
                kind: str_field(typed, "type").unwrap_or_else(|| "unknown".to_string()),
            });
        }
    }
    for value in array(item, "ISSN").iter().filter_map(Value::as_str) {
        if let Some(value) = crate::identifiers::normalize_issn(value)
            && !out.iter().any(|i| i.value == value)
        {
            out.push(Issn {
                value,
                kind: "unknown".to_string(),
            });
        }
    }
    out
}

impl CrossrefWork {
    /// Read one work object (the `message` of `/works/{doi}` or an item of a list).
    pub fn from_item(item: &Value) -> Self {
        let doi = str_field(item, "DOI").and_then(|d| normalize_doi(&d));
        Self {
            doi,
            work_type: str_field(item, "type"),
            title: first_str(item, "title"),
            subtitle: first_str(item, "subtitle"),
            authors: array(item, "author").iter().map(contributor).collect(),
            container_title: first_str(item, "container-title"),
            short_container_title: first_str(item, "short-container-title"),
            publisher: str_field(item, "publisher"),
            member: str_field(item, "member"),
            issn: issns(item),
            isbn: array(item, "ISBN")
                .iter()
                .filter_map(Value::as_str)
                .filter_map(crate::identifiers::normalize_isbn)
                .collect(),
            volume: str_field(item, "volume"),
            issue: str_field(item, "issue"),
            page: str_field(item, "page"),
            article_number: str_field(item, "article-number"),
            issued: item.get("issued").and_then(partial_date),
            published_online: item.get("published-online").and_then(partial_date),
            published_print: item.get("published-print").and_then(partial_date),
            licenses: array(item, "license")
                .iter()
                .filter_map(|l| {
                    Some(License {
                        url: str_field(l, "URL")?,
                        start: l.get("start").and_then(partial_date),
                        delay_in_days: l.get("delay-in-days").and_then(Value::as_i64),
                        content_version: str_field(l, "content-version"),
                    })
                })
                .collect(),
            funders: array(item, "funder")
                .iter()
                .filter_map(|f| {
                    Some(Funder {
                        name: str_field(f, "name")?,
                        doi: str_field(f, "DOI").and_then(|d| normalize_doi(&d)),
                        awards: array(f, "award")
                            .iter()
                            .filter_map(Value::as_str)
                            .filter_map(crate::util::non_empty)
                            .collect(),
                    })
                })
                .collect(),
            url: str_field(item, "URL"),
            language: str_field(item, "language"),
            subjects: array(item, "subject")
                .iter()
                .filter_map(Value::as_str)
                .filter_map(crate::util::non_empty)
                .collect(),
            references_count: item.get("references-count").and_then(Value::as_u64),
            is_referenced_by_count: item.get("is-referenced-by-count").and_then(Value::as_u64),
            abstract_text: abstract_text(item),
        }
    }

    /// The year the work was issued (falling back to the online/print dates).
    pub fn year(&self) -> Option<u16> {
        self.issued
            .or(self.published_print)
            .or(self.published_online)
            .map(|d| d.year)
    }

    /// The normalised record (same mapping as [`parse_crossref`]).
    pub fn to_record(&self) -> PaperRecord {
        PaperRecord {
            title: self.title.clone().unwrap_or_default(),
            authors: self
                .authors
                .iter()
                .filter_map(Contributor::display_name)
                .collect(),
            year: self.year(),
            venue: self.container_title.clone(),
            arxiv_id: self.doi.as_deref().and_then(arxiv_from_doi),
            source_id: self.doi.clone(),
            doi: self.doi.clone(),
            url: self.url.clone(),
            abstract_text: self.abstract_text.clone(),
            source: "crossref".to_string(),
            ..PaperRecord::default()
        }
    }

    /// True when any licence is a Creative Commons licence without embargo.
    pub fn has_open_license(&self) -> bool {
        self.licenses.iter().any(License::is_open)
    }
}

/// Parse a single-work response (`/works/{doi}`) into a [`CrossrefWork`].
pub fn parse_crossref_work(json: &str) -> Result<CrossrefWork, BiblioError> {
    let root: Value = serde_json::from_str(json)?;
    let Some(message) = root.get("message") else {
        return Err(BiblioError::Shape("Crossref: no `message`".to_string()));
    };
    if message.get("DOI").is_none() {
        return Err(BiblioError::Shape(
            "Crossref: `message` has no `DOI`".to_string(),
        ));
    }
    Ok(CrossrefWork::from_item(message))
}

/// Parse a work-list response into [`CrossrefWork`]s.
pub fn parse_crossref_works(json: &str) -> Result<Vec<CrossrefWork>, BiblioError> {
    let root: Value = serde_json::from_str(json)?;
    let Some(message) = root.get("message") else {
        return Err(BiblioError::Shape("Crossref: no `message`".to_string()));
    };
    if let Some(items) = message.get("items").and_then(Value::as_array) {
        return Ok(items.iter().map(CrossrefWork::from_item).collect());
    }
    if message.get("DOI").is_some() {
        return Ok(vec![CrossrefWork::from_item(message)]);
    }
    Err(BiblioError::Shape(
        "Crossref: `message` has neither `items` nor `DOI`".to_string(),
    ))
}

/// Look up one DOI's full work record; `Ok(None)` when Crossref answers 404.
pub fn fetch_work(client: &Client, doi: &str) -> Result<Option<CrossrefWork>, BiblioError> {
    match client.get_text(&doi_url(doi, client.mailto()), &[]) {
        Ok(body) => Ok(Some(parse_crossref_work(&body)?)),
        Err(BiblioError::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}

/// [`fetch_work`] through a cached, retrying [`Fetcher`].
pub fn fetch_work_cached(
    fetcher: &Fetcher<'_>,
    doi: &str,
) -> Result<Option<CrossrefWork>, BiblioError> {
    let client = fetcher.client();
    let url = doi_url(doi, client.mailto());
    fetcher
        .get_optional("crossref", doi, &url, &[])?
        .map(|body| parse_crossref_work(&body))
        .transpose()
}

#[cfg(test)]
mod work_tests {
    use super::*;

    const WORK: &str = r#"{"status":"ok","message-type":"work","message":{
      "DOI":"10.7717/PEERJ.4375","type":"journal-article","URL":"https://doi.org/10.7717/peerj.4375",
      "title":["The state of OA: a large-scale analysis of the prevalence and impact of Open Access articles"],
      "subtitle":[],
      "container-title":["PeerJ"],"short-container-title":["PeerJ"],
      "publisher":"PeerJ","member":"4443","language":"en",
      "ISSN":["2167-8359"],"issn-type":[{"value":"2167-8359","type":"electronic"}],
      "ISBN":["978-0-306-40615-7"],
      "volume":"6","page":"e4375","article-number":"e4375",
      "issued":{"date-parts":[[2018,2,13]]},"published-online":{"date-parts":[[2018,2,13]]},
      "license":[{"URL":"http://creativecommons.org/licenses/by/4.0/","start":{"date-parts":[[2018,2,13]]},"delay-in-days":0,"content-version":"unspecified"}],
      "funder":[{"DOI":"10.13039/100000001","name":"National Science Foundation","doi-asserted-by":"publisher","award":["1546338"]},{"name":"Unnamed Foundation"}],
      "author":[{"given":"Heather","family":"Piwowar","sequence":"first","ORCID":"https://orcid.org/0000-0003-2117-3717","affiliation":[{"name":"Impactstory"}]},
                {"given":"Jason","family":"Priem","sequence":"additional","affiliation":[]},
                {"name":"The OA Consortium","sequence":"additional","affiliation":[]}],
      "subject":["General Agricultural and Biological Sciences"],
      "references-count":64,"is-referenced-by-count":1200,
      "abstract":"<jats:p>Despite growing interest in Open Access.</jats:p>"
    }}"#;

    #[test]
    fn work_carries_publisher_metadata() {
        let work = parse_crossref_work(WORK).unwrap();
        assert_eq!(work.doi.as_deref(), Some("10.7717/peerj.4375"));
        assert_eq!(work.work_type.as_deref(), Some("journal-article"));
        assert_eq!(work.publisher.as_deref(), Some("PeerJ"));
        assert_eq!(work.member.as_deref(), Some("4443"));
        assert_eq!(work.container_title.as_deref(), Some("PeerJ"));
        assert_eq!(work.subtitle, None);
        assert_eq!(
            work.issn,
            vec![Issn {
                value: "2167-8359".into(),
                kind: "electronic".into()
            }]
        );
        assert_eq!(work.isbn, vec!["9780306406157"]);
        assert_eq!(
            work.issued,
            Some(PartialDate {
                year: 2018,
                month: Some(2),
                day: Some(13)
            })
        );
        assert_eq!(work.issued.unwrap().to_iso(), "2018-02-13");
        assert_eq!(work.year(), Some(2018));
        assert_eq!(work.licenses.len(), 1);
        assert!(work.licenses[0].is_open());
        assert!(work.has_open_license());
        assert_eq!(
            work.licenses[0].content_version.as_deref(),
            Some("unspecified")
        );
        assert_eq!(work.funders.len(), 2);
        assert_eq!(work.funders[0].doi.as_deref(), Some("10.13039/100000001"));
        assert_eq!(work.funders[0].awards, vec!["1546338"]);
        assert_eq!(work.funders[1].doi, None);
        assert_eq!(work.authors.len(), 3);
        assert_eq!(
            work.authors[0].orcid.as_deref(),
            Some("https://orcid.org/0000-0003-2117-3717")
        );
        assert_eq!(work.authors[0].affiliations, vec!["Impactstory"]);
        assert_eq!(
            work.authors[2].display_name().as_deref(),
            Some("The OA Consortium")
        );
        assert_eq!(work.references_count, Some(64));
        assert_eq!(work.is_referenced_by_count, Some(1200));
        assert_eq!(work.language.as_deref(), Some("en"));
        assert_eq!(work.subjects.len(), 1);
        let record = work.to_record();
        assert_eq!(
            record.authors,
            vec!["Heather Piwowar", "Jason Priem", "The OA Consortium"]
        );
        assert_eq!(record.year, Some(2018));
        assert_eq!(record.venue.as_deref(), Some("PeerJ"));
        assert_eq!(
            record.abstract_text.as_deref(),
            Some("Despite growing interest in Open Access.")
        );
        assert_eq!(record, parse_crossref(WORK).unwrap().remove(0));
    }

    #[test]
    fn work_without_optional_fields_and_bad_shapes() {
        let minimal = r#"{"status":"ok","message":{"DOI":"10.1000/x","issued":{"date-parts":[[null]]},"published-print":{"date-parts":[[2009,13]]},"ISSN":["1234-5678"]}}"#;
        let work = parse_crossref_work(minimal).unwrap();
        assert_eq!(work.title, None);
        assert_eq!(work.year(), Some(2009));
        // Month 13 is dropped; an ISSN failing its check digit is dropped.
        assert_eq!(work.published_print.unwrap().month, None);
        assert!(work.issn.is_empty());
        assert!(!work.has_open_license());
        assert!(matches!(
            parse_crossref_work(r#"{"status":"ok","message":{}}"#),
            Err(BiblioError::Shape(_))
        ));
        assert!(matches!(
            parse_crossref_work("{}"),
            Err(BiblioError::Shape(_))
        ));
        assert_eq!(parse_crossref_works(WORK).unwrap().len(), 1);
        assert!(matches!(
            parse_crossref_works(r#"{"message":{}}"#),
            Err(BiblioError::Shape(_))
        ));
    }

    #[test]
    fn bibliographic_query_url_and_offline() {
        assert_eq!(
            bibliographic_url("Smith 2020 Things", 5, Some("me@x.org")),
            "https://api.crossref.org/works?query.bibliographic=Smith%202020%20Things&rows=5&mailto=me%40x.org"
        );
        let client = Client::new("t").with_offline(true);
        assert!(matches!(
            fetch_bibliographic(&client, "x", 1),
            Err(BiblioError::Offline)
        ));
        assert!(matches!(
            fetch_work(&client, "10.1000/x"),
            Err(BiblioError::Offline)
        ));
        let fetcher = Fetcher::new(&client);
        assert!(matches!(
            fetch_work_cached(&fetcher, "10.1000/x"),
            Err(BiblioError::Offline)
        ));
    }

    #[test]
    fn cached_work_needs_no_request() {
        let dir = tempfile::tempdir().unwrap();
        let cache = crate::cache::DiskCache::new(dir.path(), std::time::Duration::from_secs(60));
        cache
            .put("crossref", "10.7717/peerj.4375", 200, WORK)
            .unwrap();
        cache.put("crossref", "10.1000/missing", 404, "").unwrap();
        let client = Client::new("t").with_offline(true);
        let fetcher = Fetcher::new(&client).with_cache(Some(&cache));
        let work = fetch_work_cached(&fetcher, "10.7717/peerj.4375")
            .unwrap()
            .unwrap();
        assert_eq!(work.publisher.as_deref(), Some("PeerJ"));
        assert_eq!(
            fetch_work_cached(&fetcher, "10.1000/missing").unwrap(),
            None
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"{
      "status": "ok",
      "message-type": "work-list",
      "message-version": "1.0.0",
      "message": {
        "facets": {},
        "total-results": 2,
        "items": [
          {
            "DOI": "10.1038/NATURE14539",
            "URL": "https://doi.org/10.1038/nature14539",
            "type": "journal-article",
            "title": ["Deep learning"],
            "container-title": ["Nature"],
            "volume": "521", "issue": "7553", "page": "436-444",
            "author": [
              {"given": "Yann", "family": "LeCun", "sequence": "first", "affiliation": []},
              {"given": "Yoshua", "family": "Bengio", "sequence": "additional", "affiliation": []},
              {"name": "The Deep Learning Consortium", "sequence": "additional", "affiliation": []}
            ],
            "issued": {"date-parts": [[2015, 5, 27]]},
            "abstract": "<jats:title>Abstract</jats:title><jats:p>Deep learning allows computational models to learn.</jats:p>",
            "link": [
              {"URL": "https://www.nature.com/articles/nature14539.pdf", "content-type": "application/pdf", "content-version": "vor", "intended-application": "text-mining"},
              {"URL": "https://www.nature.com/articles/nature14539", "content-type": "text/html", "content-version": "vor", "intended-application": "text-mining"}
            ]
          },
          {
            "DOI": "10.5555/12345678",
            "URL": "https://doi.org/10.5555/12345678",
            "title": [],
            "container-title": [],
            "author": [{"family": "Onlyfamily"}],
            "issued": {"date-parts": [[null]]},
            "published-print": {"date-parts": [[2009]]}
          }
        ],
        "items-per-page": 2
      }
    }"#;

    #[test]
    fn parses_work_list() {
        let found = parse_crossref_found(LIST).unwrap();
        assert_eq!(found.len(), 2);
        let r = &found[0].record;
        assert_eq!(r.title, "Deep learning");
        assert_eq!(
            r.authors,
            vec![
                "Yann LeCun",
                "Yoshua Bengio",
                "The Deep Learning Consortium"
            ]
        );
        assert_eq!(r.year, Some(2015));
        assert_eq!(r.venue.as_deref(), Some("Nature"));
        assert_eq!(r.doi.as_deref(), Some("10.1038/nature14539"));
        assert_eq!(
            r.url.as_deref(),
            Some("https://doi.org/10.1038/nature14539")
        );
        assert_eq!(
            r.abstract_text.as_deref(),
            Some("Deep learning allows computational models to learn.")
        );
        assert_eq!(r.source, "crossref");
        assert_eq!(found[0].candidates.len(), 1);
        assert_eq!(
            found[0].candidates[0].url,
            "https://www.nature.com/articles/nature14539.pdf"
        );
        // Publisher PDF with no open licence: assume a subscription is needed.
        assert!(found[0].candidates[0].requires_session);
    }

    #[test]
    fn handles_empty_and_null_fields() {
        let found = parse_crossref_found(LIST).unwrap();
        let r = &found[1].record;
        assert_eq!(r.title, "");
        assert_eq!(r.venue, None);
        assert_eq!(r.authors, vec!["Onlyfamily"]);
        // issued is [[null]]: falls back to published-print.
        assert_eq!(r.year, Some(2009));
        assert!(found[1].candidates.is_empty());
    }

    #[test]
    fn single_work_and_bad_shape() {
        let single =
            r#"{"status":"ok","message-type":"work","message":{"DOI":"10.1234/A","title":["X"]}}"#;
        let recs = parse_crossref(single).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].doi.as_deref(), Some("10.1234/a"));
        // A DOI without the `10.<4-9 digits>/` shape is not invented into a record DOI.
        let invalid =
            r#"{"status":"ok","message-type":"work","message":{"DOI":"10.1/A","title":["X"]}}"#;
        assert_eq!(parse_crossref(invalid).unwrap()[0].doi, None);
        assert!(matches!(
            parse_crossref(r#"{"status":"failed"}"#),
            Err(BiblioError::Shape(_))
        ));
    }

    #[test]
    fn closed_access_pdf_link_requires_session() {
        let closed = r#"{"status":"ok","message":{"DOI":"10.1016/j.cell.2020.01.001",
            "license":[{"URL":"https://www.elsevier.com/tdm/userlicense/1.0/","delay-in-days":0}],
            "link":[{"URL":"https://api.elsevier.com/content/article/PII:X?httpAccept=text/pdf",
                     "content-type":"application/pdf"}]}}"#;
        let found = parse_crossref_found(closed).unwrap();
        assert_eq!(found[0].candidates.len(), 1);
        assert!(found[0].candidates[0].requires_session);

        // A Creative Commons licence under embargo is not open yet.
        let embargoed = r#"{"status":"ok","message":{"DOI":"10.1016/j.cell.2020.01.002",
            "license":[{"URL":"https://creativecommons.org/licenses/by/4.0/","delay-in-days":365}],
            "link":[{"URL":"https://example.com/a.pdf","content-type":"application/pdf"}]}}"#;
        assert!(parse_crossref_found(embargoed).unwrap()[0].candidates[0].requires_session);
    }

    #[test]
    fn open_access_pdf_links_need_no_session() {
        let licensed = r#"{"status":"ok","message":{"DOI":"10.1371/journal.pone.0000001",
            "license":[{"URL":"http://creativecommons.org/licenses/by/4.0/","delay-in-days":0}],
            "link":[{"URL":"https://journals.plos.org/x.pdf","content-type":"application/pdf"}]}}"#;
        assert!(!parse_crossref_found(licensed).unwrap()[0].candidates[0].requires_session);

        let oa_host = r#"{"status":"ok","message":{"DOI":"10.48550/arXiv.1706.03762",
            "link":[{"URL":"https://arxiv.org/pdf/1706.03762","content-type":"application/pdf"},
                    {"URL":"https://www.ncbi.nlm.nih.gov/pmc/articles/PMC1/pdf","content-type":"application/pdf"},
                    {"URL":"https://notarxiv.org/x.pdf","content-type":"application/pdf"}]}}"#;
        let found = parse_crossref_found(oa_host).unwrap();
        assert_eq!(found[0].candidates.len(), 3);
        assert!(!found[0].candidates[0].requires_session);
        assert!(!found[0].candidates[1].requires_session);
        // Only exact hosts and true subdomains count.
        assert!(found[0].candidates[2].requires_session);
    }

    #[test]
    fn urls() {
        assert_eq!(
            search_url("deep learning", 2, Some("me@x.org")),
            "https://api.crossref.org/works?query=deep%20learning&rows=2&mailto=me%40x.org"
        );
        assert_eq!(
            doi_url("10.1038/nature14539", None),
            "https://api.crossref.org/works/10.1038/nature14539"
        );
    }

    #[test]
    fn offline_fetch_errors() {
        let client = Client::new("t").with_offline(true);
        assert!(matches!(
            fetch_by_doi(&client, "10.1/x"),
            Err(BiblioError::Offline)
        ));
    }
}
