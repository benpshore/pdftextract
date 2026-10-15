//! `doi.org`: content negotiation for CSL JSON (any registration agency),
//! and the registration-agency lookup (`/ra/{doi}`).
//!
//! Response shapes (public documentation; assumed, see crate docs):
//!
//! ```text
//! CSL JSON (Accept: application/vnd.citationstyles.csl+json):
//!   { "type", "DOI", "title", "container-title", "publisher",
//!     "ISSN": "..." | [..], "ISBN": "..." | [..],
//!     "author": [ { "given", "family" } | { "literal" } ],
//!     "issued": { "date-parts": [[y, m, d]] }, "volume", "issue", "page",
//!     "URL", "abstract" }
//! /ra/{doi}:  [ { "DOI": "10.1038/nature14539", "RA": "Crossref" } ]
//!             [ { "DOI": "10.1000/missing", "status": "DOI does not exist" } ]
//! ```

use serde_json::Value;
use tpe_common::{PaperRecord, normalize_doi};

use crate::client::Client;
use crate::error::BiblioError;
use crate::fetch::Fetcher;
use crate::util::{array, encode_path, str_field, strip_tags, year_of};

/// Resolver base URL.
pub const BASE: &str = "https://doi.org";
/// The `Accept` value asking a DOI resolver for CSL JSON.
pub const CSL_ACCEPT: &str = "application/vnd.citationstyles.csl+json";

/// `https://doi.org/<doi>` (used with `Accept: application/vnd.citationstyles.csl+json`).
pub fn csl_url(doi: &str) -> String {
    format!("{BASE}/{}", encode_path(doi))
}

/// `https://doi.org/ra/<doi>`.
pub fn ra_url(doi: &str) -> String {
    format!("{BASE}/ra/{}", encode_path(doi))
}

/// A CSL JSON item reduced to the fields the workbench uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CslItem {
    pub doi: Option<String>,
    /// CSL type: `article-journal`, `book`, `chapter`, `dataset`, ...
    pub item_type: Option<String>,
    pub title: Option<String>,
    pub container_title: Option<String>,
    pub publisher: Option<String>,
    pub issn: Vec<String>,
    pub isbn: Vec<String>,
    /// `Given Family` or the literal name.
    pub authors: Vec<String>,
    pub year: Option<u16>,
    pub volume: Option<String>,
    pub issue: Option<String>,
    pub page: Option<String>,
    pub url: Option<String>,
    pub abstract_text: Option<String>,
}

/// A string or an array of strings at `v[key]`.
fn string_or_list(v: &Value, key: &str) -> Vec<String> {
    match v.get(key) {
        Some(Value::String(s)) => crate::util::non_empty(s).into_iter().collect(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .filter_map(crate::util::non_empty)
            .collect(),
        _ => Vec::new(),
    }
}

fn csl_author(a: &Value) -> Option<String> {
    let given = str_field(a, "given");
    let family = str_field(a, "family");
    match (given, family) {
        (Some(g), Some(f)) => Some(format!("{g} {f}")),
        (None, Some(f)) => Some(f),
        (given, None) => str_field(a, "literal")
            .or_else(|| str_field(a, "name"))
            .or(given),
    }
}

fn csl_year(item: &Value) -> Option<u16> {
    ["issued", "published-online", "published-print", "published"]
        .iter()
        .find_map(|key| {
            let parts = item.get(*key)?.get("date-parts")?.get(0)?.get(0)?;
            year_of(parts)
        })
}

impl CslItem {
    /// Read one CSL JSON object.
    pub fn from_value(item: &Value) -> Self {
        Self {
            doi: str_field(item, "DOI").and_then(|d| normalize_doi(&d)),
            item_type: str_field(item, "type"),
            title: str_field(item, "title").map(|t| strip_tags(&t)),
            container_title: str_field(item, "container-title"),
            publisher: str_field(item, "publisher"),
            issn: string_or_list(item, "ISSN")
                .iter()
                .filter_map(|s| crate::identifiers::normalize_issn(s))
                .collect(),
            isbn: string_or_list(item, "ISBN")
                .iter()
                .filter_map(|s| crate::identifiers::normalize_isbn(s))
                .collect(),
            authors: array(item, "author")
                .iter()
                .filter_map(csl_author)
                .collect(),
            year: csl_year(item),
            volume: str_field(item, "volume"),
            issue: str_field(item, "issue"),
            page: str_field(item, "page"),
            url: str_field(item, "URL"),
            abstract_text: str_field(item, "abstract")
                .map(|a| strip_tags(&a))
                .and_then(|a| crate::util::non_empty(&a)),
        }
    }

    /// The normalised record (`source` is `doi.org`).
    pub fn to_record(&self) -> PaperRecord {
        PaperRecord {
            title: self.title.clone().unwrap_or_default(),
            authors: self.authors.clone(),
            year: self.year,
            venue: self.container_title.clone(),
            arxiv_id: self.doi.as_deref().and_then(crate::util::arxiv_from_doi),
            source_id: self.doi.clone(),
            doi: self.doi.clone(),
            url: self.url.clone(),
            abstract_text: self.abstract_text.clone(),
            source: "doi.org".to_string(),
            ..PaperRecord::default()
        }
    }
}

/// Parse a CSL JSON body.
pub fn parse_csl(json: &str) -> Result<CslItem, BiblioError> {
    let root: Value = serde_json::from_str(json)?;
    if !root.is_object() {
        return Err(BiblioError::Shape("CSL JSON: not an object".to_string()));
    }
    if root.get("DOI").is_none() && root.get("title").is_none() {
        return Err(BiblioError::Shape(
            "CSL JSON: neither `DOI` nor `title`".to_string(),
        ));
    }
    Ok(CslItem::from_value(&root))
}

/// Fetch CSL JSON for `doi` through content negotiation; `Ok(None)` on 404.
pub fn fetch_csl(client: &Client, doi: &str) -> Result<Option<CslItem>, BiblioError> {
    match client.get_text(&csl_url(doi), &[("Accept", CSL_ACCEPT)]) {
        Ok(body) => Ok(Some(parse_csl(&body)?)),
        Err(BiblioError::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}

/// [`fetch_csl`] through a cached, retrying [`Fetcher`].
pub fn fetch_csl_cached(fetcher: &Fetcher<'_>, doi: &str) -> Result<Option<CslItem>, BiblioError> {
    fetcher
        .get_optional("doi-org-csl", doi, &csl_url(doi), &[("Accept", CSL_ACCEPT)])?
        .map(|body| parse_csl(&body))
        .transpose()
}

/// The registration agency a DOI prefix belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Agency {
    Crossref,
    DataCite,
    /// Another agency, as `doi.org` names it (`mEDRA`, `JaLC`, `KISTI`, ...).
    Other(String),
}

impl Agency {
    /// Parse the `RA` value.
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "crossref" => Self::Crossref,
            "datacite" => Self::DataCite,
            _ => Self::Other(name.trim().to_string()),
        }
    }

    /// `crossref`, `datacite` or the agency's own name.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Crossref => "crossref",
            Self::DataCite => "datacite",
            Self::Other(name) => name,
        }
    }
}

/// One entry of a `/ra/` answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RaEntry {
    /// The DOI as echoed (normalised).
    pub doi: Option<String>,
    /// The agency, when the DOI is registered.
    pub agency: Option<Agency>,
    /// The status text otherwise (`DOI does not exist`, `Invalid DOI`).
    pub status: Option<String>,
}

/// Parse a `/ra/` answer.
pub fn parse_ra(json: &str) -> Result<Vec<RaEntry>, BiblioError> {
    let root: Value = serde_json::from_str(json)?;
    let Some(items) = root.as_array() else {
        return Err(BiblioError::Shape("doi.org/ra: not an array".to_string()));
    };
    Ok(items
        .iter()
        .map(|item| RaEntry {
            doi: str_field(item, "DOI").and_then(|d| normalize_doi(&d)),
            agency: str_field(item, "RA").map(|ra| Agency::from_name(&ra)),
            status: str_field(item, "status"),
        })
        .collect())
}

/// The registration agency of `doi`; `Ok(None)` when `doi.org` reports the
/// DOI as unregistered or invalid.
pub fn fetch_agency(fetcher: &Fetcher<'_>, doi: &str) -> Result<Option<Agency>, BiblioError> {
    let body = fetcher.get_text("doi-org-ra", doi, &ra_url(doi), &[])?;
    Ok(parse_ra(&body)?.into_iter().find_map(|entry| entry.agency))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSL: &str = r#"{
      "indexed": {"date-parts": [[2024, 1, 1]]},
      "type": "article-journal",
      "id": "10.1038/nature14539",
      "DOI": "10.1038/NATURE14539",
      "title": "Deep <i>learning</i>",
      "container-title": "Nature",
      "publisher": "Springer Science and Business Media LLC",
      "ISSN": ["0028-0836", "1476-4687"],
      "author": [{"given": "Yann", "family": "LeCun"}, {"given": "Yoshua", "family": "Bengio"}, {"literal": "Deep Learning Consortium"}],
      "issued": {"date-parts": [[2015, 5, 27]]},
      "volume": "521", "issue": "7553", "page": "436-444",
      "URL": "http://dx.doi.org/10.1038/nature14539",
      "abstract": "<jats:p>Deep learning allows computational models.</jats:p>"
    }"#;

    #[test]
    fn csl_item_and_record() {
        let item = parse_csl(CSL).unwrap();
        assert_eq!(item.doi.as_deref(), Some("10.1038/nature14539"));
        assert_eq!(item.item_type.as_deref(), Some("article-journal"));
        assert_eq!(item.title.as_deref(), Some("Deep learning"));
        assert_eq!(
            item.publisher.as_deref(),
            Some("Springer Science and Business Media LLC")
        );
        assert_eq!(item.issn, vec!["0028-0836", "1476-4687"]);
        assert_eq!(
            item.authors,
            vec!["Yann LeCun", "Yoshua Bengio", "Deep Learning Consortium"]
        );
        assert_eq!(item.year, Some(2015));
        assert_eq!(item.page.as_deref(), Some("436-444"));
        let record = item.to_record();
        assert_eq!(record.source, "doi.org");
        assert_eq!(record.venue.as_deref(), Some("Nature"));
        assert_eq!(
            record.abstract_text.as_deref(),
            Some("Deep learning allows computational models.")
        );
        // A single-string ISSN/ISBN is accepted too.
        let single = parse_csl(
            r#"{"DOI":"10.1000/b","type":"book","ISSN":"0028-0836","ISBN":"978-0-306-40615-7"}"#,
        )
        .unwrap();
        assert_eq!(single.issn, vec!["0028-0836"]);
        assert_eq!(single.isbn, vec!["9780306406157"]);
        assert!(matches!(parse_csl("[]"), Err(BiblioError::Shape(_))));
        assert!(matches!(
            parse_csl(r#"{"x":1}"#),
            Err(BiblioError::Shape(_))
        ));
    }

    #[test]
    fn registration_agency_lookup() {
        let entries = parse_ra(r#"[{"DOI":"10.1038/nature14539","RA":"Crossref"},{"DOI":"10.5281/zenodo.1","RA":"DataCite"},{"DOI":"10.9999/x","status":"DOI does not exist"},{"DOI":"10.1234/m","RA":"mEDRA"}]"#).unwrap();
        assert_eq!(entries[0].agency, Some(Agency::Crossref));
        assert_eq!(entries[1].agency, Some(Agency::DataCite));
        assert_eq!(entries[2].agency, None);
        assert_eq!(entries[2].status.as_deref(), Some("DOI does not exist"));
        assert_eq!(entries[3].agency, Some(Agency::Other("mEDRA".into())));
        assert_eq!(entries[3].agency.as_ref().unwrap().as_str(), "mEDRA");
        assert_eq!(Agency::from_name(" datacite ").as_str(), "datacite");
        assert!(matches!(parse_ra("{}"), Err(BiblioError::Shape(_))));
    }

    #[test]
    fn urls_and_offline() {
        assert_eq!(
            csl_url("10.1038/nature14539"),
            "https://doi.org/10.1038/nature14539"
        );
        assert_eq!(ra_url("10.1000/a b"), "https://doi.org/ra/10.1000/a%20b");
        let client = Client::new("t").with_offline(true);
        assert!(matches!(
            fetch_csl(&client, "10.1000/x"),
            Err(BiblioError::Offline)
        ));
        let fetcher = Fetcher::new(&client);
        assert!(matches!(
            fetch_csl_cached(&fetcher, "10.1000/x"),
            Err(BiblioError::Offline)
        ));
        assert!(matches!(
            fetch_agency(&fetcher, "10.1000/x"),
            Err(BiblioError::Offline)
        ));
    }

    #[test]
    fn cached_answers_work_offline() {
        let dir = tempfile::tempdir().unwrap();
        let cache = crate::cache::DiskCache::new(dir.path(), std::time::Duration::from_secs(60));
        cache
            .put("doi-org-csl", "10.1038/nature14539", 200, CSL)
            .unwrap();
        cache
            .put(
                "doi-org-ra",
                "10.1038/nature14539",
                200,
                r#"[{"DOI":"10.1038/nature14539","RA":"Crossref"}]"#,
            )
            .unwrap();
        cache
            .put(
                "doi-org-ra",
                "10.9999/x",
                200,
                r#"[{"DOI":"10.9999/x","status":"DOI does not exist"}]"#,
            )
            .unwrap();
        let client = Client::new("t").with_offline(true);
        let fetcher = Fetcher::new(&client).with_cache(Some(&cache));
        assert_eq!(
            fetch_csl_cached(&fetcher, "10.1038/nature14539")
                .unwrap()
                .unwrap()
                .title
                .as_deref(),
            Some("Deep learning")
        );
        assert_eq!(
            fetch_agency(&fetcher, "10.1038/nature14539").unwrap(),
            Some(Agency::Crossref)
        );
        assert_eq!(fetch_agency(&fetcher, "10.9999/x").unwrap(), None);
    }
}
