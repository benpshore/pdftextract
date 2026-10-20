//! `PubMed` through NCBI E-utilities (`db=pubmed`: `esearch`, `esummary`,
//! `efetch`) and the PMC ID converter (PMID, PMCID and DOI in any direction).
//!
//! Response shapes (public documentation; assumed, see crate docs):
//!
//! ```text
//! esearch:  { "esearchresult": { "count", "idlist": [..] } }
//! esummary: { "result": { "uids": [..], "<uid>": { "uid", "title", "authors": [ { "name" } ],
//!             "source", "fulljournalname", "pubdate", "epubdate", "volume", "issue",
//!             "pages", "issn", "essn", "lang": [..], "pubtype": [..],
//!             "articleids": [ { "idtype": "pubmed" | "doi" | "pmc" | "pii", "value" } ],
//!             "elocationid" } } }
//! efetch (retmode=xml): <PubmedArticleSet><PubmedArticle><MedlineCitation>
//!             <PMID>..</PMID><Article><Journal><ISSN>..</ISSN><JournalIssue><PubDate>
//!             <Year>..</Year></PubDate></JournalIssue><Title>..</Title></Journal>
//!             <ArticleTitle>..</ArticleTitle><Abstract><AbstractText>..</AbstractText>
//!             </Abstract><AuthorList><Author><LastName/><ForeName/><CollectiveName/>
//!             </Author></AuthorList><ELocationID EIdType="doi">..</ELocationID>
//!             </Article></MedlineCitation><PubmedData><ArticleIdList>
//!             <ArticleId IdType="doi|pubmed|pmc">..</ArticleId></ArticleIdList>
//!             </PubmedData></PubmedArticle></PubmedArticleSet>
//! idconv:   { "status": "ok", "records": [ { "pmcid", "pmid", "doi" } |
//!             { "<requested-type>": "<id>", "status": "error", "errmsg" } ] }
//! ```
//!
//! The `efetch` reader is a deliberate, minimal tag scanner for exactly the
//! elements above (entities decoded, nested markup stripped); it is not an
//! XML parser and ignores anything else.

use serde_json::Value;
use tpe_common::{PaperRecord, normalize_doi};

use crate::client::{Client, KEY_NCBI};
use crate::error::BiblioError;
use crate::fetch::Fetcher;
use crate::identifiers::{normalize_pmcid, normalize_pmid};
use crate::lookup::Lookup;
use crate::util::{array, decode_entities, str_field, strip_tags, with_query, year_prefix};

/// E-utilities base URL.
pub const EUTILS: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils";
/// PMC ID converter base URL.
pub const IDCONV: &str = "https://pmc.ncbi.nlm.nih.gov/tools/idconv/api/v1/articles/";
/// The `tool` name sent to NCBI, as its usage policy asks.
pub const TOOL: &str = "tpe";

fn with_key<'a>(pairs: &mut Vec<(&'a str, &'a str)>, api_key: Option<&'a str>) {
    if let Some(k) = api_key {
        pairs.push(("api_key", k));
    }
}

/// `esearch.fcgi?db=pubmed&term=<q>&retmode=json&retmax=<n>[&api_key=<k>]`.
pub fn esearch_url(term: &str, retmax: u32, api_key: Option<&str>) -> String {
    let n = retmax.clamp(1, 10_000).to_string();
    let mut pairs: Vec<(&str, &str)> = vec![
        ("db", "pubmed"),
        ("term", term),
        ("retmode", "json"),
        ("retmax", n.as_str()),
        ("tool", TOOL),
    ];
    with_key(&mut pairs, api_key);
    with_query(&format!("{EUTILS}/esearch.fcgi"), &pairs)
}

/// `esummary.fcgi?db=pubmed&id=<id,id>&retmode=json[&api_key=<k>]`.
pub fn esummary_url(ids: &[String], api_key: Option<&str>) -> String {
    let joined = ids.join(",");
    let mut pairs: Vec<(&str, &str)> = vec![
        ("db", "pubmed"),
        ("id", joined.as_str()),
        ("retmode", "json"),
        ("tool", TOOL),
    ];
    with_key(&mut pairs, api_key);
    with_query(&format!("{EUTILS}/esummary.fcgi"), &pairs)
}

/// `efetch.fcgi?db=pubmed&id=<id,id>&retmode=xml[&api_key=<k>]`.
pub fn efetch_url(ids: &[String], api_key: Option<&str>) -> String {
    let joined = ids.join(",");
    let mut pairs: Vec<(&str, &str)> = vec![
        ("db", "pubmed"),
        ("id", joined.as_str()),
        ("retmode", "xml"),
        ("tool", TOOL),
    ];
    with_key(&mut pairs, api_key);
    with_query(&format!("{EUTILS}/efetch.fcgi"), &pairs)
}

/// The ID converter URL for up to 200 ids (PMIDs, PMCIDs or DOIs, mixed).
/// `email` is the contact address NCBI asks for; it is never invented.
pub fn idconv_url(ids: &[String], email: Option<&str>) -> String {
    let joined = ids.join(",");
    let mut pairs: Vec<(&str, &str)> =
        vec![("ids", joined.as_str()), ("format", "json"), ("tool", TOOL)];
    if let Some(e) = email {
        pairs.push(("email", e));
    }
    with_query(IDCONV, &pairs)
}

/// `esearch` term for an exact DOI.
pub fn doi_term(doi: &str) -> String {
    format!("{doi}[DOI]")
}

/// The ids from an `esearch` JSON response (same shape for every database).
pub fn parse_esearch_ids(json: &str) -> Result<Vec<String>, BiblioError> {
    crate::pmc::parse_esearch_ids(json)
}

/// One `esummary` document, with the journal fields the summary carries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PubmedSummary {
    pub pmid: String,
    pub doi: Option<String>,
    pub pmcid: Option<String>,
    pub title: Option<String>,
    /// `Family Initials` as `PubMed` prints them.
    pub authors: Vec<String>,
    pub year: Option<u16>,
    pub journal: Option<String>,
    /// Medline abbreviation (`source`).
    pub journal_abbrev: Option<String>,
    pub volume: Option<String>,
    pub issue: Option<String>,
    pub pages: Option<String>,
    pub issn: Option<String>,
    pub essn: Option<String>,
    pub pub_types: Vec<String>,
}

fn summary_doc(uid: &str, doc: &Value) -> Option<PubmedSummary> {
    let pmid = normalize_pmid(uid)?;
    let mut summary = PubmedSummary {
        pmid,
        title: str_field(doc, "title").map(|t| strip_tags(&t)),
        authors: array(doc, "authors")
            .iter()
            .filter_map(|a| str_field(a, "name"))
            .collect(),
        year: str_field(doc, "pubdate")
            .and_then(|d| year_prefix(&d))
            .or_else(|| str_field(doc, "epubdate").and_then(|d| year_prefix(&d))),
        journal: str_field(doc, "fulljournalname"),
        journal_abbrev: str_field(doc, "source"),
        volume: str_field(doc, "volume"),
        issue: str_field(doc, "issue"),
        pages: str_field(doc, "pages"),
        issn: str_field(doc, "issn").and_then(|s| crate::identifiers::normalize_issn(&s)),
        essn: str_field(doc, "essn").and_then(|s| crate::identifiers::normalize_issn(&s)),
        pub_types: array(doc, "pubtype")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        ..PubmedSummary::default()
    };
    for id in array(doc, "articleids") {
        let Some(value) = str_field(id, "value") else {
            continue;
        };
        match str_field(id, "idtype").as_deref() {
            Some("doi") => summary.doi = summary.doi.take().or_else(|| normalize_doi(&value)),
            Some("pmc" | "pmcid") => {
                summary.pmcid = summary.pmcid.take().or_else(|| normalize_pmcid(&value));
            }
            _ => {}
        }
    }
    if summary.doi.is_none()
        && let Some(eloc) = str_field(doc, "elocationid")
        && let Some(doi) = crate::identifiers::normalize_doi_text(&eloc)
    {
        summary.doi = Some(doi);
    }
    Some(summary)
}

/// Turn `Family Initials` (`Piwowar H`) into `Initials Family` for the
/// given-name-first convention of the other sources.
pub fn given_first(name: &str) -> String {
    match name.rsplit_once(' ') {
        Some((family, initials))
            if !initials.is_empty()
                && initials.len() <= 5
                && initials.chars().all(|c| c.is_ascii_uppercase() || c == '.') =>
        {
            format!("{initials} {family}")
        }
        _ => name.to_string(),
    }
}

impl PubmedSummary {
    /// The normalised record (`source` is `pubmed`).
    pub fn to_record(&self) -> PaperRecord {
        PaperRecord {
            title: self.title.clone().unwrap_or_default(),
            authors: self.authors.iter().map(|a| given_first(a)).collect(),
            year: self.year,
            venue: self.journal.clone().or_else(|| self.journal_abbrev.clone()),
            doi: self.doi.clone(),
            arxiv_id: self.doi.as_deref().and_then(crate::util::arxiv_from_doi),
            pmid: Some(self.pmid.clone()),
            pmcid: self.pmcid.clone(),
            url: Some(format!("https://pubmed.ncbi.nlm.nih.gov/{}/", self.pmid)),
            source: "pubmed".to_string(),
            source_id: Some(self.pmid.clone()),
            ..PaperRecord::default()
        }
    }

    /// Both ISSNs the summary carries.
    pub fn issns(&self) -> Vec<String> {
        self.issn.iter().chain(self.essn.iter()).cloned().collect()
    }
}

/// Parse an `esummary` (`db=pubmed`) response, in `uids` order; documents
/// that carry an `error` are skipped.
pub fn parse_pubmed_esummary(json: &str) -> Result<Vec<PubmedSummary>, BiblioError> {
    let root: Value = serde_json::from_str(json)?;
    let Some(result) = root.get("result") else {
        return Err(BiblioError::Shape("esummary: no `result`".to_string()));
    };
    let mut out: Vec<PubmedSummary> = Vec::new();
    for uid in array(result, "uids").iter().filter_map(Value::as_str) {
        if let Some(doc) = result.get(uid)
            && doc.get("error").is_none()
            && let Some(summary) = summary_doc(uid, doc)
        {
            out.push(summary);
        }
    }
    Ok(out)
}

// ------------------------------------------------------------ efetch XML

/// The text of the first `<tag ...>...</tag>` in `xml`, with inner markup
/// removed and entities decoded; `None` when absent.
fn element_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut from = 0;
    loop {
        let start = xml[from..].find(&open)? + from;
        let after = start + open.len();
        // `<Title` must not match `<TitleX`.
        if !matches!(
            xml[after..].chars().next(),
            Some('>' | ' ' | '\n' | '\t' | '/')
        ) {
            from = after;
            continue;
        }
        let gt = xml[after..].find('>')? + after;
        if xml[..gt].ends_with('/') {
            return Some(String::new());
        }
        let end = xml[gt + 1..].find(&close)? + gt + 1;
        let text = strip_tags(&xml[gt + 1..end]);
        return crate::util::non_empty(&text);
    }
}

/// Every `<tag ...>...</tag>` block in `xml`, as raw inner text.
fn element_blocks<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(pos) = xml[from..].find(&open) {
        let start = pos + from;
        let after = start + open.len();
        if !matches!(xml[after..].chars().next(), Some('>' | ' ' | '\n' | '\t')) {
            from = after;
            continue;
        }
        let Some(gt) = xml[after..].find('>').map(|g| g + after) else {
            break;
        };
        let Some(end) = xml[gt + 1..].find(&close).map(|e| e + gt + 1) else {
            break;
        };
        out.push(&xml[gt + 1..end]);
        from = end + close.len();
    }
    out
}

/// The value of attribute `name` on the opening `<tag ...>` that starts `block`.
fn attribute_of(opening: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let start = opening.find(&key)? + key.len();
    let end = opening[start..].find('"')? + start;
    Some(decode_entities(&opening[start..end]))
}

/// Every `<tag attr="...">text</tag>` of `xml` as (attribute value, text).
fn typed_elements(xml: &str, tag: &str, attr: &str) -> Vec<(String, String)> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(pos) = xml[from..].find(&open) {
        let start = pos + from;
        let after = start + open.len();
        if !matches!(xml[after..].chars().next(), Some(' ' | '>' | '\n')) {
            from = after;
            continue;
        }
        let Some(gt) = xml[after..].find('>').map(|g| g + after) else {
            break;
        };
        let Some(end) = xml[gt + 1..].find(&close).map(|e| e + gt + 1) else {
            break;
        };
        let kind = attribute_of(&xml[start..gt], attr).unwrap_or_default();
        out.push((kind, strip_tags(&xml[gt + 1..end])));
        from = end + close.len();
    }
    out
}

/// One `PubmedArticle` from `efetch`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PubmedArticle {
    pub pmid: String,
    pub doi: Option<String>,
    pub pmcid: Option<String>,
    pub title: Option<String>,
    /// `Given Family`, or the collective name.
    pub authors: Vec<String>,
    pub year: Option<u16>,
    pub journal: Option<String>,
    pub issn: Vec<String>,
    pub abstract_text: Option<String>,
}

fn article(block: &str) -> Option<PubmedArticle> {
    let citation = element_blocks(block, "MedlineCitation")
        .into_iter()
        .next()?;
    let pmid = element_text(citation, "PMID").and_then(|p| normalize_pmid(&p))?;
    let mut doi = None;
    let mut central_id = None;
    for (kind, value) in typed_elements(block, "ArticleId", "IdType") {
        match kind.as_str() {
            "doi" => doi = doi.or_else(|| normalize_doi(&value)),
            "pmc" => central_id = central_id.or_else(|| normalize_pmcid(&value)),
            _ => {}
        }
    }
    for (kind, value) in typed_elements(citation, "ELocationID", "EIdType") {
        if kind == "doi" {
            doi = doi.or_else(|| normalize_doi(&value));
        }
    }
    let journal = element_blocks(citation, "Journal").into_iter().next();
    let authors = element_blocks(citation, "AuthorList")
        .into_iter()
        .next()
        .map(|list| {
            element_blocks(list, "Author")
                .into_iter()
                .filter_map(|a| {
                    let family = element_text(a, "LastName");
                    let given = element_text(a, "ForeName").or_else(|| element_text(a, "Initials"));
                    match (given, family) {
                        (Some(g), Some(f)) => Some(format!("{g} {f}")),
                        (None, Some(f)) => Some(f),
                        _ => element_text(a, "CollectiveName"),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let abstract_text = element_blocks(citation, "Abstract")
        .into_iter()
        .next()
        .map(|a| {
            element_blocks(a, "AbstractText")
                .into_iter()
                .map(strip_tags)
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .and_then(|t| crate::util::non_empty(&t));
    Some(PubmedArticle {
        pmid,
        doi,
        pmcid: central_id,
        title: element_text(citation, "ArticleTitle").map(|t| t.trim_end_matches('.').to_string()),
        authors,
        year: journal
            .and_then(|j| element_text(j, "Year"))
            .and_then(|y| year_prefix(&y))
            .or_else(|| element_text(citation, "Year").and_then(|y| year_prefix(&y))),
        journal: journal.and_then(|j| element_text(j, "Title")),
        issn: journal
            .map(|j| {
                typed_elements(j, "ISSN", "IssnType")
                    .into_iter()
                    .filter_map(|(_, v)| crate::identifiers::normalize_issn(&v))
                    .collect()
            })
            .unwrap_or_default(),
        abstract_text,
    })
}

impl PubmedArticle {
    /// The normalised record (`source` is `pubmed`).
    pub fn to_record(&self) -> PaperRecord {
        PaperRecord {
            title: self.title.clone().unwrap_or_default(),
            authors: self.authors.clone(),
            year: self.year,
            venue: self.journal.clone(),
            doi: self.doi.clone(),
            arxiv_id: self.doi.as_deref().and_then(crate::util::arxiv_from_doi),
            pmid: Some(self.pmid.clone()),
            pmcid: self.pmcid.clone(),
            url: Some(format!("https://pubmed.ncbi.nlm.nih.gov/{}/", self.pmid)),
            abstract_text: self.abstract_text.clone(),
            source: "pubmed".to_string(),
            source_id: Some(self.pmid.clone()),
        }
    }
}

/// Parse an `efetch` (`db=pubmed`, `retmode=xml`) response.
pub fn parse_pubmed_efetch(xml: &str) -> Result<Vec<PubmedArticle>, BiblioError> {
    if !xml.contains("<PubmedArticleSet") && !xml.contains("<PubmedArticle") {
        return Err(BiblioError::Shape(
            "efetch: no `PubmedArticleSet`".to_string(),
        ));
    }
    Ok(element_blocks(xml, "PubmedArticle")
        .into_iter()
        .filter_map(article)
        .collect())
}

// ------------------------------------------------------------- idconv

/// One ID converter record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdConversion {
    pub pmid: Option<String>,
    pub pmcid: Option<String>,
    pub doi: Option<String>,
    /// The converter's error message for an unknown id.
    pub error: Option<String>,
}

/// Parse an ID converter JSON response.
pub fn parse_idconv(json: &str) -> Result<Vec<IdConversion>, BiblioError> {
    let root: Value = serde_json::from_str(json)?;
    if str_field(&root, "status").as_deref() == Some("error") {
        return Err(BiblioError::Shape(format!(
            "idconv: {}",
            str_field(&root, "message").unwrap_or_else(|| "error".to_string())
        )));
    }
    let Some(records) = root.get("records").and_then(Value::as_array) else {
        return Err(BiblioError::Shape("idconv: no `records`".to_string()));
    };
    Ok(records
        .iter()
        .map(|r| IdConversion {
            pmid: str_field(r, "pmid").and_then(|p| normalize_pmid(&p)),
            pmcid: str_field(r, "pmcid").and_then(|p| normalize_pmcid(&p)),
            doi: str_field(r, "doi").and_then(|d| normalize_doi(&d)),
            error: (str_field(r, "status").as_deref() == Some("error"))
                .then(|| str_field(r, "errmsg").unwrap_or_else(|| "error".to_string())),
        })
        .collect())
}

// ------------------------------------------------------------- fetches

/// Summaries for `pmids` through `esummary`.
pub fn fetch_summaries(
    client: &Client,
    pmids: &[String],
) -> Result<Vec<PubmedSummary>, BiblioError> {
    if pmids.is_empty() {
        return Ok(Vec::new());
    }
    parse_pubmed_esummary(&client.get_text(&esummary_url(pmids, client.key(KEY_NCBI)), &[])?)
}

/// Full records for `pmids` through `efetch`.
pub fn fetch_articles(
    client: &Client,
    pmids: &[String],
) -> Result<Vec<PubmedArticle>, BiblioError> {
    if pmids.is_empty() {
        return Ok(Vec::new());
    }
    parse_pubmed_efetch(&client.get_text(&efetch_url(pmids, client.key(KEY_NCBI)), &[])?)
}

/// Search `PubMed` (`esearch` then `esummary`).
pub fn fetch_search(
    client: &Client,
    term: &str,
    retmax: u32,
) -> Result<Vec<PubmedSummary>, BiblioError> {
    let ids = parse_esearch_ids(
        &client.get_text(&esearch_url(term, retmax, client.key(KEY_NCBI)), &[])?,
    )?;
    fetch_summaries(client, &ids)
}

/// Convert ids (PMIDs, PMCIDs, DOIs) through the ID converter.
pub fn fetch_idconv(client: &Client, ids: &[String]) -> Result<Vec<IdConversion>, BiblioError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    parse_idconv(&client.get_text(&idconv_url(ids, client.mailto()), &[])?)
}

/// The summary for one exact PMID, cached under `pubmed-esummary`; the
/// returned record must carry that PMID, else it is `NotFound`.
pub fn resolve_pmid(fetcher: &Fetcher<'_>, pmid: &str) -> Lookup<PubmedSummary> {
    let Some(pmid) = normalize_pmid(pmid) else {
        return Lookup::Unresolved("not a PubMed id".to_string());
    };
    let client = fetcher.client();
    let url = esummary_url(std::slice::from_ref(&pmid), client.key(KEY_NCBI));
    let result = fetcher
        .get_optional("pubmed-esummary", &pmid, &url, &[])
        .and_then(|body| {
            body.map(|b| parse_pubmed_esummary(&b))
                .transpose()
                .map(|docs| docs.and_then(|docs| docs.into_iter().find(|d| d.pmid == pmid)))
        });
    Lookup::from_result(result)
}

/// The PMID and PMCID for one DOI through the ID converter, cached under
/// `pubmed-idconv`; `NotFound` when the converter does not know the DOI.
pub fn resolve_doi_ids(fetcher: &Fetcher<'_>, doi: &str) -> Lookup<IdConversion> {
    let Some(doi) = normalize_doi(doi) else {
        return Lookup::Unresolved("not a DOI".to_string());
    };
    let client = fetcher.client();
    let url = idconv_url(std::slice::from_ref(&doi), client.mailto());
    let result = fetcher
        .get_optional("pubmed-idconv", &doi, &url, &[])
        .and_then(|body| {
            body.map(|b| parse_idconv(&b)).transpose().map(|records| {
                records.and_then(|records| {
                    records
                        .into_iter()
                        .find(|r| r.error.is_none() && r.doi.as_deref() == Some(doi.as_str()))
                })
            })
        });
    Lookup::from_result(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ESUMMARY: &str = r#"{
      "header": {"type": "esummary", "version": "0.3"},
      "result": {
        "uids": ["29456894", "1"],
        "29456894": {
          "uid": "29456894",
          "pubdate": "2018 Feb 13", "epubdate": "2018 Feb 13",
          "source": "PeerJ", "fulljournalname": "PeerJ",
          "authors": [{"name": "Piwowar H", "authtype": "Author", "clusterid": ""}, {"name": "Priem J", "authtype": "Author", "clusterid": ""}],
          "title": "The state of OA: a large-scale analysis of the prevalence and impact of Open Access articles.",
          "volume": "6", "issue": "", "pages": "e4375",
          "lang": ["eng"], "issn": "", "essn": "2167-8359",
          "pubtype": ["Journal Article"],
          "articleids": [
            {"idtype": "pubmed", "idtypen": 1, "value": "29456894"},
            {"idtype": "doi", "idtypen": 3, "value": "10.7717/peerj.4375"},
            {"idtype": "pmc", "idtypen": 8, "value": "PMC5815332"},
            {"idtype": "pii", "idtypen": 4, "value": "4375"}
          ],
          "elocationid": "doi: 10.7717/peerj.4375",
          "sortpubdate": "2018/02/13 00:00"
        },
        "1": {"uid": "1", "error": "cannot get document summary"}
      }
    }"#;

    const EFETCH: &str = r#"<?xml version="1.0" ?>
<!DOCTYPE PubmedArticleSet PUBLIC "-//NLM//DTD PubMedArticle, 1st January 2024//EN" "https://dtd.nlm.nih.gov/ncbi/pubmed/out/pubmed_240101.dtd">
<PubmedArticleSet>
<PubmedArticle>
  <MedlineCitation Status="PubMed-not-MEDLINE" Owner="NLM">
    <PMID Version="1">29456894</PMID>
    <Article PubModel="Electronic-eCollection">
      <Journal>
        <ISSN IssnType="Electronic">2167-8359</ISSN>
        <JournalIssue CitedMedium="Internet"><Volume>6</Volume><PubDate><Year>2018</Year></PubDate></JournalIssue>
        <Title>PeerJ</Title>
        <ISOAbbreviation>PeerJ</ISOAbbreviation>
      </Journal>
      <ArticleTitle>The state of OA: a large-scale analysis of the prevalence &amp; impact of <i>Open Access</i> articles.</ArticleTitle>
      <ELocationID EIdType="doi" ValidYN="Y">10.7717/peerj.4375</ELocationID>
      <Abstract>
        <AbstractText Label="BACKGROUND">Despite growing interest in Open Access (OA).</AbstractText>
        <AbstractText Label="RESULTS">We find OA is growing.</AbstractText>
      </Abstract>
      <AuthorList CompleteYN="Y">
        <Author ValidYN="Y"><LastName>Piwowar</LastName><ForeName>Heather</ForeName><Initials>H</Initials></Author>
        <Author ValidYN="Y"><LastName>Priem</LastName><Initials>J</Initials></Author>
        <Author ValidYN="Y"><CollectiveName>OA Working Group</CollectiveName></Author>
      </AuthorList>
    </Article>
  </MedlineCitation>
  <PubmedData>
    <ArticleIdList>
      <ArticleId IdType="pubmed">29456894</ArticleId>
      <ArticleId IdType="doi">10.7717/PEERJ.4375</ArticleId>
      <ArticleId IdType="pmc">PMC5815332</ArticleId>
    </ArticleIdList>
  </PubmedData>
</PubmedArticle>
<PubmedArticle>
  <MedlineCitation Status="MEDLINE" Owner="NLM">
    <PMID Version="1">12345</PMID>
    <Article><Journal><Title>Old Journal</Title><JournalIssue><PubDate><MedlineDate>1998 Jan-Feb</MedlineDate></PubDate></JournalIssue></Journal>
    <ArticleTitle>Untitled work</ArticleTitle></Article>
  </MedlineCitation>
</PubmedArticle>
</PubmedArticleSet>"#;

    const IDCONV: &str = r#"{"status":"ok","responseDate":"2024-01-01 00:00:00","request":"ids=10.7717/peerj.4375,10.1000/missing;format=json",
      "records":[
        {"pmcid":"PMC5815332","pmid":"29456894","doi":"10.7717/peerj.4375","versions":[{"pmcid":"PMC5815332.1","current":"true"}]},
        {"doi":"10.1000/missing","status":"error","errmsg":"invalid article id"}
      ]}"#;

    #[test]
    fn esummary_documents() {
        let docs = parse_pubmed_esummary(ESUMMARY).unwrap();
        assert_eq!(docs.len(), 1);
        let d = &docs[0];
        assert_eq!(d.pmid, "29456894");
        assert_eq!(d.doi.as_deref(), Some("10.7717/peerj.4375"));
        assert_eq!(d.pmcid.as_deref(), Some("PMC5815332"));
        assert_eq!(d.year, Some(2018));
        assert_eq!(d.journal.as_deref(), Some("PeerJ"));
        assert_eq!(d.essn.as_deref(), Some("2167-8359"));
        assert_eq!(d.issn, None);
        assert_eq!(d.issns(), vec!["2167-8359"]);
        assert_eq!(d.pages.as_deref(), Some("e4375"));
        assert_eq!(d.pub_types, vec!["Journal Article"]);
        let record = d.to_record();
        assert_eq!(record.authors, vec!["H Piwowar", "J Priem"]);
        assert_eq!(record.pmid.as_deref(), Some("29456894"));
        assert_eq!(record.source, "pubmed");
        assert_eq!(
            record.url.as_deref(),
            Some("https://pubmed.ncbi.nlm.nih.gov/29456894/")
        );
        // The DOI falls back to `elocationid` when no article id carries it.
        let eloc_only = r#"{"result":{"uids":["7"],"7":{"uid":"7","title":"T","elocationid":"doi: 10.1000/ELOC.","articleids":[]}}}"#;
        assert_eq!(
            parse_pubmed_esummary(eloc_only).unwrap()[0].doi.as_deref(),
            Some("10.1000/eloc")
        );
        assert!(matches!(
            parse_pubmed_esummary("{}"),
            Err(BiblioError::Shape(_))
        ));
        assert_eq!(given_first("van der Kogel A"), "A van der Kogel");
        assert_eq!(given_first("Consortium"), "Consortium");
    }

    #[test]
    fn efetch_articles() {
        let articles = parse_pubmed_efetch(EFETCH).unwrap();
        assert_eq!(articles.len(), 2);
        let a = &articles[0];
        assert_eq!(a.pmid, "29456894");
        assert_eq!(a.doi.as_deref(), Some("10.7717/peerj.4375"));
        assert_eq!(a.pmcid.as_deref(), Some("PMC5815332"));
        assert_eq!(
            a.title.as_deref(),
            Some(
                "The state of OA: a large-scale analysis of the prevalence & impact of Open Access articles"
            )
        );
        assert_eq!(
            a.authors,
            vec!["Heather Piwowar", "J Priem", "OA Working Group"]
        );
        assert_eq!(a.year, Some(2018));
        assert_eq!(a.journal.as_deref(), Some("PeerJ"));
        assert_eq!(a.issn, vec!["2167-8359"]);
        assert_eq!(
            a.abstract_text.as_deref(),
            Some("Despite growing interest in Open Access (OA). We find OA is growing.")
        );
        assert_eq!(a.to_record().venue.as_deref(), Some("PeerJ"));
        let b = &articles[1];
        assert_eq!(b.pmid, "12345");
        assert_eq!(b.doi, None);
        assert_eq!(b.year, None);
        assert_eq!(b.journal.as_deref(), Some("Old Journal"));
        assert!(b.authors.is_empty());
        assert!(matches!(
            parse_pubmed_efetch("<html/>"),
            Err(BiblioError::Shape(_))
        ));
        assert!(
            parse_pubmed_efetch("<PubmedArticleSet></PubmedArticleSet>")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn idconv_records() {
        let records = parse_idconv(IDCONV).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].pmid.as_deref(), Some("29456894"));
        assert_eq!(records[0].pmcid.as_deref(), Some("PMC5815332"));
        assert_eq!(records[0].doi.as_deref(), Some("10.7717/peerj.4375"));
        assert_eq!(records[0].error, None);
        assert_eq!(records[1].error.as_deref(), Some("invalid article id"));
        assert_eq!(records[1].doi.as_deref(), Some("10.1000/missing"));
        assert!(matches!(
            parse_idconv(r#"{"status":"error","message":"bad"}"#),
            Err(BiblioError::Shape(_))
        ));
        assert!(matches!(parse_idconv("{}"), Err(BiblioError::Shape(_))));
    }

    #[test]
    fn urls_carry_tool_key_and_email_only_when_known() {
        assert_eq!(
            esearch_url("10.7717/peerj.4375[DOI]", 3, None),
            "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi?db=pubmed&term=10.7717%2Fpeerj.4375%5BDOI%5D&retmode=json&retmax=3&tool=tpe"
        );
        assert_eq!(doi_term("10.1000/x"), "10.1000/x[DOI]");
        let ids = vec!["1".to_string(), "2".to_string()];
        assert!(
            esummary_url(&ids, Some("K")).ends_with("id=1%2C2&retmode=json&tool=tpe&api_key=K")
        );
        assert!(
            efetch_url(&ids, None).contains("efetch.fcgi?db=pubmed&id=1%2C2&retmode=xml&tool=tpe")
        );
        assert_eq!(
            idconv_url(&ids, Some("me@x.org")),
            "https://pmc.ncbi.nlm.nih.gov/tools/idconv/api/v1/articles/?ids=1%2C2&format=json&tool=tpe&email=me%40x.org"
        );
        assert!(!idconv_url(&ids, None).contains("email"));
    }

    #[test]
    fn lookups_report_offline_and_use_the_cache() {
        let client = Client::new("t").with_offline(true);
        let fetcher = Fetcher::new(&client);
        assert_eq!(
            resolve_pmid(&fetcher, "29456894").status(),
            "unresolved: offline"
        );
        assert_eq!(
            resolve_doi_ids(&fetcher, "10.7717/peerj.4375").status(),
            "unresolved: offline"
        );
        assert_eq!(
            resolve_pmid(&fetcher, "abc").status(),
            "unresolved: not a PubMed id"
        );
        assert_eq!(
            resolve_doi_ids(&fetcher, "nope").status(),
            "unresolved: not a DOI"
        );
        assert!(matches!(
            fetch_search(&client, "x", 1),
            Err(BiblioError::Offline)
        ));
        assert!(fetch_summaries(&client, &[]).unwrap().is_empty());
        assert!(fetch_articles(&client, &[]).unwrap().is_empty());
        assert!(fetch_idconv(&client, &[]).unwrap().is_empty());

        let dir = tempfile::tempdir().unwrap();
        let cache = crate::cache::DiskCache::new(dir.path(), std::time::Duration::from_secs(60));
        cache
            .put("pubmed-esummary", "29456894", 200, ESUMMARY)
            .unwrap();
        cache.put("pubmed-esummary", "1", 200, ESUMMARY).unwrap();
        cache
            .put("pubmed-idconv", "10.7717/peerj.4375", 200, IDCONV)
            .unwrap();
        cache
            .put("pubmed-idconv", "10.1000/missing", 200, IDCONV)
            .unwrap();
        let fetcher = Fetcher::new(&client).with_cache(Some(&cache));
        let found = resolve_pmid(&fetcher, "29456894").found().unwrap();
        assert_eq!(found.doi.as_deref(), Some("10.7717/peerj.4375"));
        // The cached body does not carry a usable record for uid 1.
        assert_eq!(resolve_pmid(&fetcher, "1"), Lookup::NotFound);
        let ids = resolve_doi_ids(&fetcher, "10.7717/PEERJ.4375")
            .found()
            .unwrap();
        assert_eq!(ids.pmid.as_deref(), Some("29456894"));
        assert_eq!(
            resolve_doi_ids(&fetcher, "10.1000/missing"),
            Lookup::NotFound
        );
    }
}
