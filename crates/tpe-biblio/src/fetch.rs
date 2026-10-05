//! A cached, retrying GET: the one path every resolution client goes
//! through. Order of business for `get_text(source, key, url)`:
//!
//! 1. a fresh cache entry answers without a request (`200` body, or `404`
//!    as `NotFound`);
//! 2. offline mode answers `Offline` without a request;
//! 3. otherwise the request runs under the retry policy (backoff, honouring
//!    `Retry-After`), and a `200` or `404` answer is stored in the cache.

use std::time::Duration;

use crate::cache::DiskCache;
use crate::client::Client;
use crate::error::BiblioError;
use crate::retry::RetryPolicy;

/// The client, the optional cache and the retry policy a source uses.
pub struct Fetcher<'a> {
    client: &'a Client,
    cache: Option<&'a DiskCache>,
    policy: RetryPolicy,
    sleep: fn(Duration),
}

impl<'a> Fetcher<'a> {
    /// A fetcher with the default retry policy and no cache.
    pub fn new(client: &'a Client) -> Self {
        Self {
            client,
            cache: None,
            policy: RetryPolicy::default(),
            sleep: std::thread::sleep,
        }
    }

    /// Use `cache` for responses.
    #[must_use]
    pub fn with_cache(mut self, cache: Option<&'a DiskCache>) -> Self {
        self.cache = cache;
        self
    }

    /// Replace the retry policy.
    #[must_use]
    pub fn with_policy(mut self, policy: RetryPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Replace the sleep used between retries (tests pass a no-op).
    #[must_use]
    pub fn with_sleep(mut self, sleep: fn(Duration)) -> Self {
        self.sleep = sleep;
        self
    }

    /// The underlying client.
    pub fn client(&self) -> &'a Client {
        self.client
    }

    /// The cache, if any.
    pub fn cache(&self) -> Option<&'a DiskCache> {
        self.cache
    }

    /// GET `url` for (`source`, `key`) through the cache and the retry policy.
    pub fn get_text(
        &self,
        source: &str,
        key: &str,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<String, BiblioError> {
        if let Some(cache) = self.cache
            && let Some(hit) = cache.get(source, key)
        {
            return match hit.status {
                404 => Err(BiblioError::NotFound),
                _ => Ok(hit.body),
            };
        }
        let result = self
            .policy
            .run(|| self.client.get_text(url, headers), self.sleep);
        if let Some(cache) = self.cache {
            match &result {
                Ok(body) => {
                    let _ = cache.put(source, key, 200, body);
                }
                Err(BiblioError::NotFound) => {
                    let _ = cache.put(source, key, 404, "");
                }
                Err(_) => {}
            }
        }
        result
    }

    /// Like [`Self::get_text`] but `Ok(None)` for a 404.
    pub fn get_optional(
        &self,
        source: &str,
        key: &str,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<Option<String>, BiblioError> {
        match self.get_text(source, key, url, headers) {
            Ok(body) => Ok(Some(body)),
            Err(BiblioError::NotFound) => Ok(None),
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_answers_before_offline_mode() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskCache::new(dir.path(), Duration::from_secs(3600));
        cache
            .put("crossref", "10.1000/x", 200, "{\"ok\":true}")
            .unwrap();
        cache.put("crossref", "10.1000/missing", 404, "").unwrap();
        let client = Client::new("t").with_offline(true);
        let fetcher = Fetcher::new(&client).with_cache(Some(&cache));
        assert_eq!(
            fetcher
                .get_text("crossref", "10.1000/x", "https://example.invalid/", &[])
                .unwrap(),
            "{\"ok\":true}"
        );
        assert_eq!(
            fetcher
                .get_optional(
                    "crossref",
                    "10.1000/missing",
                    "https://example.invalid/",
                    &[]
                )
                .unwrap(),
            None
        );
        // Not cached: offline mode is reported, with no retry.
        assert!(matches!(
            fetcher.get_text("crossref", "10.1000/other", "https://example.invalid/", &[]),
            Err(BiblioError::Offline)
        ));
        assert!(fetcher.cache().is_some());
        assert!(fetcher.client().is_offline());
    }

    #[test]
    fn offline_without_cache_is_offline() {
        let client = Client::new("t").with_offline(true);
        let fetcher = Fetcher::new(&client)
            .with_policy(RetryPolicy::NONE)
            .with_sleep(|_| {});
        assert!(matches!(
            fetcher.get_optional("datacite", "k", "https://example.invalid/", &[]),
            Err(BiblioError::Offline)
        ));
    }
}
