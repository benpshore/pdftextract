//! `DataCite` REST API: `/dois/{doi}` for DOIs registered through `DataCite`
//! (Zenodo, figshare, Dryad, `arXiv`'s own DOIs, institutional repositories).
//!
//! Response shape (JSON:API, public documentation; assumed, see crate docs):
//!
//! ```text
//! { "data": { "id": "10.5281/zenodo.123", "type": "dois", "attributes": {
//!     "doi", "prefix", "url", "publisher": "Zenodo" | { "name": "Zenodo" },
//!     "publicationYear": 2020, "version",
//!     "titles": [ { "title", "titleType" } ],
//!     "creators": [ { "name", "givenName", "familyName", "nameType",
//!                     "nameIdentifiers": [ { "nameIdentifier", "nameIdentifierScheme" } ] } ],
//!     "types": { "resourceTypeGeneral", "resourceType", "citeproc" },
//!     "rightsList": [ { "rights", "rightsUri", "rightsIdentifier" } ],
//!     "fundingReferences": [ { "funderName", "funderIdentifier", "awardNumber", "awardTitle" } ],
//!     "descriptions": [ { "description", "descriptionType" } ],
//!     "container": { "type", "title", "identifier", "identifierType" },
//!     "relatedIdentifiers": [ { "relatedIdentifier", "relatedIdentifierType", "relationType" } ],
//!     "dates": [ { "date", "dateType" } ] } } }
//! ```

use serde_json::Value;
use tpe_common::{PaperRecord, normalize_doi};

use crate::client::Client;
use crate::error::BiblioError;
use crate::fetch::Fetcher;
use crate::util::{array, encode_path, str_field, strip_tags, year_of};

/// API base URL.
pub const BASE: &str = "https://api.datacite.org";

/// `GET /dois/<doi>`.
pub fn doi_url(doi: &str) -> String {
    format!("{BASE}/dois/{}", encode_path(doi))
}

/// One creator.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Creator {
    /// `Family, Given` or an organisation name, as registered.
    pub name: String,
    pub given: Option<String>,
    pub family: Option<String>,
    /// `Personal` or `Organizational`.
    pub name_type: Option<String>,
    /// ORCID identifier (URL form), when registered.
    pub orcid: Option<String>,
}

impl Creator {
    /// `Given Family` when both are known, else the registered name with a
    /// `Family, Given` form turned around.
    pub fn display_name(&self) -> String {
        match (&self.given, &self.family) {
            (Some(g), Some(f)) => format!("{g} {f}"),
            _ => match self.name.split_once(", ") {
                Some((family, given))
                    if self.name_type.as_deref() != Some("Organizational") && !given.is_empty() =>
                {
                    format!("{given} {family}")
                }
                _ => self.name.clone(),
            },
        }
    }
}

/// One `rightsList` entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rights {
    pub rights: Option<String>,
    pub uri: Option<String>,
    /// SPDX identifier such as `cc-by-4.0`.
    pub identifier: Option<String>,
}

/// One `fundingReferences` entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FundingReference {
    pub funder_name: String,
    pub funder_identifier: Option<String>,
    pub award_number: Option<String>,
    pub award_title: Option<String>,
}

/// One `relatedIdentifiers` entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RelatedIdentifier {
    pub identifier: String,
    /// `DOI`, `URL`, `arXiv`, ...
    pub identifier_type: String,
    /// `IsVersionOf`, `IsSupplementTo`, `IsIdenticalTo`, ...
    pub relation_type: String,
}

/// A `DataCite` DOI record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DataCiteRecord {
    pub doi: Option<String>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub creators: Vec<Creator>,
    pub publisher: Option<String>,
    pub publication_year: Option<u16>,
    /// `Dataset`, `Software`, `Text`, `Preprint`, ...
    pub resource_type_general: Option<String>,
    pub resource_type: Option<String>,
    pub version: Option<String>,
    pub rights: Vec<Rights>,
    pub funding: Vec<FundingReference>,
    pub abstract_text: Option<String>,
    pub container_title: Option<String>,
    pub related: Vec<RelatedIdentifier>,
}

fn creator(c: &Value) -> Option<Creator> {
    let name = str_field(c, "name").or_else(|| {
        match (str_field(c, "givenName"), str_field(c, "familyName")) {
            (Some(g), Some(f)) => Some(format!("{f}, {g}")),
            (None, Some(f)) => Some(f),
            (Some(g), None) => Some(g),
            (None, None) => None,
        }
    })?;
    let orcid = array(c, "nameIdentifiers").iter().find_map(|id| {
        (str_field(id, "nameIdentifierScheme").is_some_and(|s| s.eq_ignore_ascii_case("orcid")))
            .then(|| str_field(id, "nameIdentifier"))
            .flatten()
    });
    Some(Creator {
        name,
        given: str_field(c, "givenName"),
        family: str_field(c, "familyName"),
        name_type: str_field(c, "nameType"),
        orcid,
    })
}

impl DataCiteRecord {
    /// Read the `attributes` object of one DOI.
    pub fn from_attributes(attrs: &Value) -> Self {
        let publisher = match attrs.get("publisher") {
            Some(Value::String(s)) => crate::util::non_empty(s),
            Some(obj @ Value::Object(_)) => str_field(obj, "name"),
            _ => None,
        };
        let types = attrs.get("types");
        Self {
            doi: str_field(attrs, "doi").and_then(|d| normalize_doi(&d)),
            url: str_field(attrs, "url"),
            title: array(attrs, "titles")
                .iter()
                .find(|t| t.get("titleType").is_none())
                .or_else(|| array(attrs, "titles").first())
                .and_then(|t| str_field(t, "title"))
                .map(|t| strip_tags(&t)),
            creators: array(attrs, "creators")
                .iter()
                .filter_map(creator)
                .collect(),
            publisher,
            publication_year: attrs.get("publicationYear").and_then(year_of),
            resource_type_general: types.and_then(|t| str_field(t, "resourceTypeGeneral")),
            resource_type: types.and_then(|t| str_field(t, "resourceType")),
            version: str_field(attrs, "version"),
            rights: array(attrs, "rightsList")
                .iter()
                .map(|r| Rights {
                    rights: str_field(r, "rights"),
                    uri: str_field(r, "rightsUri"),
                    identifier: str_field(r, "rightsIdentifier"),
                })
                .filter(|r| r.rights.is_some() || r.uri.is_some() || r.identifier.is_some())
                .collect(),
            funding: array(attrs, "fundingReferences")
                .iter()
                .filter_map(|f| {
                    Some(FundingReference {
                        funder_name: str_field(f, "funderName")?,
                        funder_identifier: str_field(f, "funderIdentifier"),
                        award_number: str_field(f, "awardNumber"),
                        award_title: str_field(f, "awardTitle"),
                    })
                })
                .collect(),
            abstract_text: array(attrs, "descriptions")
                .iter()
                .find(|d| str_field(d, "descriptionType").as_deref() == Some("Abstract"))
                .and_then(|d| str_field(d, "description"))
                .map(|d| strip_tags(&d)),
            container_title: attrs.get("container").and_then(|c| str_field(c, "title")),
            related: array(attrs, "relatedIdentifiers")
                .iter()
                .filter_map(|r| {
                    Some(RelatedIdentifier {
                        identifier: str_field(r, "relatedIdentifier")?,
                        identifier_type: str_field(r, "relatedIdentifierType")?,
                        relation_type: str_field(r, "relationType").unwrap_or_default(),
                    })
                })
                .collect(),
        }
    }

    /// The normalised record (`source` is `datacite`).
    pub fn to_record(&self) -> PaperRecord {
        PaperRecord {
            title: self.title.clone().unwrap_or_default(),
            authors: self.creators.iter().map(Creator::display_name).collect(),
            year: self.publication_year,
            venue: self
                .container_title
                .clone()
                .or_else(|| self.publisher.clone()),
            arxiv_id: self.doi.as_deref().and_then(crate::util::arxiv_from_doi),
            source_id: self.doi.clone(),
            doi: self.doi.clone(),
            url: self.url.clone(),
            abstract_text: self.abstract_text.clone(),
            source: "datacite".to_string(),
            ..PaperRecord::default()
        }
    }

    /// True when a rights entry is a Creative Commons licence.
    pub fn has_open_license(&self) -> bool {
        self.rights.iter().any(|r| {
            r.identifier
                .as_deref()
                .is_some_and(|id| id.to_ascii_lowercase().starts_with("cc"))
                || r.uri
                    .as_deref()
                    .is_some_and(|u| crate::util::host_of(u).ends_with("creativecommons.org"))
        })
    }
}

/// Parse a `/dois/{doi}` response.
pub fn parse_datacite(json: &str) -> Result<DataCiteRecord, BiblioError> {
    let root: Value = serde_json::from_str(json)?;
    let Some(data) = root.get("data") else {
        return Err(BiblioError::Shape("DataCite: no `data`".to_string()));
    };
    let Some(attrs) = data.get("attributes") else {
        return Err(BiblioError::Shape("DataCite: no `attributes`".to_string()));
    };
    let mut record = DataCiteRecord::from_attributes(attrs);
    if record.doi.is_none() {
        record.doi = str_field(data, "id").and_then(|d| normalize_doi(&d));
    }
    Ok(record)
}

/// Look up one DOI; `Ok(None)` when `DataCite` answers 404.
pub fn fetch_by_doi(client: &Client, doi: &str) -> Result<Option<DataCiteRecord>, BiblioError> {
    match client.get_text(&doi_url(doi), &[("Accept", "application/vnd.api+json")]) {
        Ok(body) => Ok(Some(parse_datacite(&body)?)),
        Err(BiblioError::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}

/// [`fetch_by_doi`] through a cached, retrying [`Fetcher`].
pub fn fetch_by_doi_cached(
    fetcher: &Fetcher<'_>,
    doi: &str,
) -> Result<Option<DataCiteRecord>, BiblioError> {
    fetcher
        .get_optional(
            "datacite",
            doi,
            &doi_url(doi),
            &[("Accept", "application/vnd.api+json")],
        )?
        .map(|body| parse_datacite(&body))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOI: &str = r#"{"data":{"id":"10.5281/zenodo.1234567","type":"dois","attributes":{
      "doi":"10.5281/ZENODO.1234567","prefix":"10.5281","url":"https://zenodo.org/record/1234567",
      "publisher":"Zenodo","publicationYear":2020,"version":"1.0.0",
      "titles":[{"title":"Example <b>dataset</b> of measurements"},{"title":"Subtitle here","titleType":"Subtitle"}],
      "creators":[
        {"name":"Doe, Jane","givenName":"Jane","familyName":"Doe","nameType":"Personal","nameIdentifiers":[{"nameIdentifier":"https://orcid.org/0000-0002-1825-0097","nameIdentifierScheme":"ORCID"}]},
        {"name":"Example Consortium","nameType":"Organizational","nameIdentifiers":[]},
        {"name":"Roe, Richard"}
      ],
      "types":{"resourceTypeGeneral":"Dataset","resourceType":"Measurements","citeproc":"dataset"},
      "rightsList":[{"rights":"Creative Commons Attribution 4.0 International","rightsUri":"https://creativecommons.org/licenses/by/4.0/legalcode","rightsIdentifier":"cc-by-4.0"}],
      "fundingReferences":[{"funderName":"European Commission","funderIdentifier":"https://doi.org/10.13039/501100000780","awardNumber":"101000000","awardTitle":"EXAMPLE"}],
      "descriptions":[{"description":"<p>Measurements taken in 2019.</p>","descriptionType":"Abstract"}],
      "container":{"type":"Series","title":"Example Series"},
      "relatedIdentifiers":[{"relatedIdentifier":"10.5281/zenodo.1234566","relatedIdentifierType":"DOI","relationType":"IsVersionOf"}],
      "dates":[{"date":"2020-03-01","dateType":"Issued"}]
    }}}"#;

    #[test]
    fn dataset_record() {
        let record = parse_datacite(DOI).unwrap();
        assert_eq!(record.doi.as_deref(), Some("10.5281/zenodo.1234567"));
        assert_eq!(
            record.title.as_deref(),
            Some("Example dataset of measurements")
        );
        assert_eq!(record.publisher.as_deref(), Some("Zenodo"));
        assert_eq!(record.publication_year, Some(2020));
        assert_eq!(record.resource_type_general.as_deref(), Some("Dataset"));
        assert_eq!(record.version.as_deref(), Some("1.0.0"));
        assert_eq!(record.creators.len(), 3);
        assert_eq!(
            record.creators[0].orcid.as_deref(),
            Some("https://orcid.org/0000-0002-1825-0097")
        );
        assert_eq!(record.creators[0].display_name(), "Jane Doe");
        assert_eq!(record.creators[1].display_name(), "Example Consortium");
        assert_eq!(record.creators[2].display_name(), "Richard Roe");
        assert_eq!(record.rights[0].identifier.as_deref(), Some("cc-by-4.0"));
        assert!(record.has_open_license());
        assert_eq!(record.funding[0].award_number.as_deref(), Some("101000000"));
        assert_eq!(
            record.abstract_text.as_deref(),
            Some("Measurements taken in 2019.")
        );
        assert_eq!(record.container_title.as_deref(), Some("Example Series"));
        assert_eq!(record.related[0].relation_type, "IsVersionOf");
        let paper = record.to_record();
        assert_eq!(paper.source, "datacite");
        assert_eq!(
            paper.authors,
            vec!["Jane Doe", "Example Consortium", "Richard Roe"]
        );
        assert_eq!(paper.venue.as_deref(), Some("Example Series"));
        assert_eq!(
            paper.url.as_deref(),
            Some("https://zenodo.org/record/1234567")
        );
    }

    #[test]
    fn publisher_object_arxiv_doi_and_bad_shapes() {
        let arxiv = r#"{"data":{"id":"10.48550/arxiv.1706.03762","attributes":{"publisher":{"name":"arXiv"},"publicationYear":"2017","titles":[{"title":"Attention Is All You Need"}],"creators":[{"givenName":"Ashish","familyName":"Vaswani"}],"types":{"resourceTypeGeneral":"Preprint"}}}}"#;
        let record = parse_datacite(arxiv).unwrap();
        assert_eq!(record.doi.as_deref(), Some("10.48550/arxiv.1706.03762"));
        assert_eq!(record.publisher.as_deref(), Some("arXiv"));
        assert_eq!(record.publication_year, Some(2017));
        assert_eq!(record.creators[0].name, "Vaswani, Ashish");
        let paper = record.to_record();
        assert_eq!(paper.arxiv_id.as_deref(), Some("1706.03762"));
        assert_eq!(paper.venue.as_deref(), Some("arXiv"));
        assert!(!record.has_open_license());
        assert!(matches!(
            parse_datacite(r#"{"errors":[{"status":"404"}]}"#),
            Err(BiblioError::Shape(_))
        ));
        assert!(matches!(
            parse_datacite(r#"{"data":{"id":"x"}}"#),
            Err(BiblioError::Shape(_))
        ));
    }

    #[test]
    fn urls_offline_and_cache() {
        assert_eq!(
            doi_url("10.5281/zenodo.1"),
            "https://api.datacite.org/dois/10.5281/zenodo.1"
        );
        let client = Client::new("t").with_offline(true);
        assert!(matches!(
            fetch_by_doi(&client, "10.5281/zenodo.1"),
            Err(BiblioError::Offline)
        ));
        let dir = tempfile::tempdir().unwrap();
        let cache = crate::cache::DiskCache::new(dir.path(), std::time::Duration::from_secs(60));
        cache
            .put("datacite", "10.5281/zenodo.1234567", 200, DOI)
            .unwrap();
        let fetcher = Fetcher::new(&client).with_cache(Some(&cache));
        assert_eq!(
            fetch_by_doi_cached(&fetcher, "10.5281/zenodo.1234567")
                .unwrap()
                .unwrap()
                .publisher
                .as_deref(),
            Some("Zenodo")
        );
        assert!(matches!(
            fetch_by_doi_cached(&fetcher, "10.5281/zenodo.2"),
            Err(BiblioError::Offline)
        ));
    }
}
