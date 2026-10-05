//! One DOI, one answer: the registry record with its publisher metadata,
//! from Crossref, `DataCite` or `doi.org` content negotiation, whichever
//! knows the DOI.
//!
//! Order of sources: Crossref first (it registers most scholarly articles),
//! `DataCite` first for prefixes known to be `DataCite`'s, and `doi.org`
//! CSL JSON last (it answers for every agency but with fewer fields). The
//! first source that returns a record wins; a record that does not carry the
//! requested DOI is a mismatch and never accepted. When every source says
//! "not found" the answer is `NotFound`; when any source could not answer
//! and none found the DOI, the answer is `Unresolved` with the first reason
//! (`offline`, `network: ...`, `HTTP status ...`), never "not found".

use tpe_common::{PaperRecord, normalize_doi};

use crate::crossref::{self, CrossrefWork, Funder, License};
use crate::datacite::{self, DataCiteRecord};
use crate::doi_org::{self, Agency, CslItem};
use crate::fetch::Fetcher;
use crate::identifiers::likely_datacite_doi;
use crate::lookup::{Lookup, unresolved_reason};

/// Publication and publisher facts about a DOI, as its registry records them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PublisherMetadata {
    pub publisher: Option<String>,
    /// Journal, book or series title.
    pub container_title: Option<String>,
    /// Registry work type (`journal-article`, `Dataset`, `article-journal`, ...).
    pub work_type: Option<String>,
    pub issn: Vec<String>,
    pub isbn: Vec<String>,
    pub licenses: Vec<License>,
    pub funders: Vec<Funder>,
    pub volume: Option<String>,
    pub issue: Option<String>,
    pub page: Option<String>,
    /// The landing page the registry points at.
    pub url: Option<String>,
    /// Crossref member id, when known.
    pub member: Option<String>,
}

/// The result of resolving one DOI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoiMetadata {
    /// The normalised DOI that was asked for (and that the record carries).
    pub doi: String,
    /// The normalised bibliographic record.
    pub record: PaperRecord,
    /// Publisher and publication facts.
    pub publisher: PublisherMetadata,
    /// Which registry answered: `crossref`, `datacite` or `doi.org`.
    pub source: String,
    /// The registration agency, when it is known from the answering source.
    pub agency: Option<Agency>,
}

impl DoiMetadata {
    /// From a Crossref work.
    pub fn from_crossref(work: &CrossrefWork) -> Option<Self> {
        let doi = work.doi.clone()?;
        Some(Self {
            doi,
            record: work.to_record(),
            publisher: PublisherMetadata {
                publisher: work.publisher.clone(),
                container_title: work.container_title.clone(),
                work_type: work.work_type.clone(),
                issn: work.issn.iter().map(|i| i.value.clone()).collect(),
                isbn: work.isbn.clone(),
                licenses: work.licenses.clone(),
                funders: work.funders.clone(),
                volume: work.volume.clone(),
                issue: work.issue.clone(),
                page: work.page.clone().or_else(|| work.article_number.clone()),
                url: work.url.clone(),
                member: work.member.clone(),
            },
            source: "crossref".to_string(),
            agency: Some(Agency::Crossref),
        })
    }

    /// From a `DataCite` record.
    pub fn from_datacite(record: &DataCiteRecord) -> Option<Self> {
        let doi = record.doi.clone()?;
        Some(Self {
            doi,
            record: record.to_record(),
            publisher: PublisherMetadata {
                publisher: record.publisher.clone(),
                container_title: record.container_title.clone(),
                work_type: record.resource_type_general.clone(),
                licenses: record
                    .rights
                    .iter()
                    .filter_map(|r| {
                        Some(License {
                            url: r.uri.clone()?,
                            start: None,
                            delay_in_days: None,
                            content_version: r.identifier.clone(),
                        })
                    })
                    .collect(),
                funders: record
                    .funding
                    .iter()
                    .map(|f| Funder {
                        name: f.funder_name.clone(),
                        doi: f.funder_identifier.as_deref().and_then(normalize_doi),
                        awards: f.award_number.iter().cloned().collect(),
                    })
                    .collect(),
                url: record.url.clone(),
                ..PublisherMetadata::default()
            },
            source: "datacite".to_string(),
            agency: Some(Agency::DataCite),
        })
    }

    /// From a CSL JSON item (`doi.org`).
    pub fn from_csl(item: &CslItem) -> Option<Self> {
        let doi = item.doi.clone()?;
        Some(Self {
            doi,
            record: item.to_record(),
            publisher: PublisherMetadata {
                publisher: item.publisher.clone(),
                container_title: item.container_title.clone(),
                work_type: item.item_type.clone(),
                issn: item.issn.clone(),
                isbn: item.isbn.clone(),
                volume: item.volume.clone(),
                issue: item.issue.clone(),
                page: item.page.clone(),
                url: item.url.clone(),
                ..PublisherMetadata::default()
            },
            source: "doi.org".to_string(),
            agency: None,
        })
    }
}

/// The registries, in the order they are asked for `doi`.
fn source_order(doi: &str) -> [&'static str; 3] {
    if likely_datacite_doi(doi) {
        ["datacite", "crossref", "doi.org"]
    } else {
        ["crossref", "datacite", "doi.org"]
    }
}

/// Resolve `doi` to its registry metadata (see the module docs for the
/// order and the meaning of each outcome).
pub fn resolve_doi(fetcher: &Fetcher<'_>, doi: &str) -> Lookup<DoiMetadata> {
    let Some(doi) = normalize_doi(doi) else {
        return Lookup::Unresolved("not a DOI".to_string());
    };
    let mut first_failure: Option<String> = None;
    for source in source_order(&doi) {
        let result = match source {
            "crossref" => crossref::fetch_work_cached(fetcher, &doi)
                .map(|w| w.as_ref().and_then(DoiMetadata::from_crossref)),
            "datacite" => datacite::fetch_by_doi_cached(fetcher, &doi)
                .map(|r| r.as_ref().and_then(DoiMetadata::from_datacite)),
            _ => doi_org::fetch_csl_cached(fetcher, &doi)
                .map(|i| i.as_ref().and_then(DoiMetadata::from_csl)),
        };
        match result {
            Ok(Some(found)) if found.doi == doi => return Lookup::Found(found),
            Ok(Some(found)) => {
                first_failure.get_or_insert(format!(
                    "{source} answered with a different DOI ({})",
                    found.doi
                ));
            }
            Ok(None) => {}
            Err(err) => {
                first_failure.get_or_insert(unresolved_reason(&err));
            }
        }
    }
    match first_failure {
        Some(reason) => Lookup::Unresolved(reason),
        None => Lookup::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::cache::DiskCache;
    use crate::client::Client;

    const CROSSREF: &str = r#"{"status":"ok","message":{"DOI":"10.1038/nature14539","type":"journal-article",
        "title":["Deep learning"],"container-title":["Nature"],"publisher":"Springer Nature","member":"297",
        "ISSN":["0028-0836","1476-4687"],"volume":"521","issue":"7553","page":"436-444",
        "issued":{"date-parts":[[2015,5,27]]},"URL":"https://doi.org/10.1038/nature14539",
        "license":[{"URL":"https://www.springer.com/tdm","delay-in-days":0,"content-version":"tdm"}],
        "funder":[{"name":"CIFAR","award":["X1"]}],
        "author":[{"given":"Yann","family":"LeCun"},{"given":"Yoshua","family":"Bengio"},{"given":"Geoffrey","family":"Hinton"}]}}"#;
    const DATACITE: &str = r#"{"data":{"id":"10.5281/zenodo.1234567","attributes":{"doi":"10.5281/zenodo.1234567","url":"https://zenodo.org/record/1234567","publisher":"Zenodo","publicationYear":2020,"titles":[{"title":"A dataset"}],"creators":[{"name":"Doe, Jane"}],"types":{"resourceTypeGeneral":"Dataset"},"rightsList":[{"rightsUri":"https://creativecommons.org/licenses/by/4.0/","rightsIdentifier":"cc-by-4.0"}],"fundingReferences":[{"funderName":"EC","funderIdentifier":"https://doi.org/10.13039/501100000780","awardNumber":"1"}]}}}"#;
    const CSL: &str = r#"{"type":"article-journal","DOI":"10.9999/other","title":"Elsewhere","container-title":"Other Journal","publisher":"Other House","ISSN":"0028-0836","author":[{"given":"A","family":"B"}],"issued":{"date-parts":[[2001]]},"URL":"https://example.org/x"}"#;

    fn cached_fetcher(dir: &std::path::Path) -> DiskCache {
        DiskCache::new(dir, Duration::from_secs(600))
    }

    #[test]
    fn crossref_record_wins_with_publisher_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cached_fetcher(dir.path());
        cache
            .put("crossref", "10.1038/nature14539", 200, CROSSREF)
            .unwrap();
        let client = Client::new("t").with_offline(true);
        let fetcher = Fetcher::new(&client).with_cache(Some(&cache));
        let found = resolve_doi(&fetcher, "https://doi.org/10.1038/NATURE14539")
            .found()
            .unwrap();
        assert_eq!(found.source, "crossref");
        assert_eq!(found.agency, Some(Agency::Crossref));
        assert_eq!(found.record.title, "Deep learning");
        assert_eq!(found.record.authors.len(), 3);
        assert_eq!(
            found.publisher.publisher.as_deref(),
            Some("Springer Nature")
        );
        assert_eq!(found.publisher.container_title.as_deref(), Some("Nature"));
        assert_eq!(found.publisher.issn, vec!["0028-0836", "1476-4687"]);
        assert_eq!(
            found.publisher.licenses[0].content_version.as_deref(),
            Some("tdm")
        );
        assert_eq!(found.publisher.funders[0].name, "CIFAR");
        assert_eq!(found.publisher.page.as_deref(), Some("436-444"));
        assert_eq!(found.publisher.member.as_deref(), Some("297"));
    }

    #[test]
    fn datacite_prefix_is_asked_first_and_maps_rights_and_funding() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cached_fetcher(dir.path());
        cache
            .put("datacite", "10.5281/zenodo.1234567", 200, DATACITE)
            .unwrap();
        let client = Client::new("t").with_offline(true);
        let fetcher = Fetcher::new(&client).with_cache(Some(&cache));
        assert_eq!(source_order("10.5281/zenodo.1234567")[0], "datacite");
        let found = resolve_doi(&fetcher, "10.5281/zenodo.1234567")
            .found()
            .unwrap();
        assert_eq!(found.source, "datacite");
        assert_eq!(found.publisher.work_type.as_deref(), Some("Dataset"));
        assert_eq!(
            found.publisher.licenses[0].url,
            "https://creativecommons.org/licenses/by/4.0/"
        );
        assert!(found.publisher.licenses[0].is_open());
        assert_eq!(
            found.publisher.funders[0].doi.as_deref(),
            Some("10.13039/501100000780")
        );
        assert_eq!(found.publisher.funders[0].awards, vec!["1"]);
        assert_eq!(found.record.authors, vec!["Jane Doe"]);
    }

    #[test]
    fn falls_through_to_doi_org_and_reports_not_found_only_when_all_agree() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cached_fetcher(dir.path());
        cache.put("crossref", "10.9999/other", 404, "").unwrap();
        cache.put("datacite", "10.9999/other", 404, "").unwrap();
        cache.put("doi-org-csl", "10.9999/other", 200, CSL).unwrap();
        let client = Client::new("t").with_offline(true);
        let fetcher = Fetcher::new(&client).with_cache(Some(&cache));
        let found = resolve_doi(&fetcher, "10.9999/other").found().unwrap();
        assert_eq!(found.source, "doi.org");
        assert_eq!(found.agency, None);
        assert_eq!(found.publisher.publisher.as_deref(), Some("Other House"));
        assert_eq!(found.publisher.issn, vec!["0028-0836"]);

        cache.put("crossref", "10.9999/none", 404, "").unwrap();
        cache.put("datacite", "10.9999/none", 404, "").unwrap();
        cache.put("doi-org-csl", "10.9999/none", 404, "").unwrap();
        assert_eq!(resolve_doi(&fetcher, "10.9999/none"), Lookup::NotFound);

        // Two registries say no, the third could not be asked: unresolved, not "not found".
        cache.put("crossref", "10.9999/unknown", 404, "").unwrap();
        cache.put("datacite", "10.9999/unknown", 404, "").unwrap();
        assert_eq!(
            resolve_doi(&fetcher, "10.9999/unknown").status(),
            "unresolved: offline"
        );
        assert_eq!(
            resolve_doi(&fetcher, "10.9999/fresh").status(),
            "unresolved: offline"
        );
        assert_eq!(
            resolve_doi(&fetcher, "garbage").status(),
            "unresolved: not a DOI"
        );
    }

    #[test]
    fn a_record_for_another_doi_is_never_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cached_fetcher(dir.path());
        // Crossref's cached body is for nature14539 but was stored under another key.
        cache
            .put("crossref", "10.1038/other", 200, CROSSREF)
            .unwrap();
        cache.put("datacite", "10.1038/other", 404, "").unwrap();
        cache.put("doi-org-csl", "10.1038/other", 404, "").unwrap();
        let client = Client::new("t").with_offline(true);
        let fetcher = Fetcher::new(&client).with_cache(Some(&cache));
        assert_eq!(
            resolve_doi(&fetcher, "10.1038/other").status(),
            "unresolved: crossref answered with a different DOI (10.1038/nature14539)"
        );
    }
}
