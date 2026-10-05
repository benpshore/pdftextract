//! Polite registry access: one `User-Agent` carrying the contact address,
//! per-host request spacing, retries with backoff on `429`/`5xx` that honour
//! `Retry-After`, request timeouts, and the on-disk cache in front of it all.
//!
//! The transport is a trait so every test runs against scripted responses;
//! only [`UreqTransport`] touches the network.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use percent_encoding::{AsciiSet, CONTROLS, NON_ALPHANUMERIC, utf8_percent_encode};

use crate::cache::Cache;
use crate::record::{Record, parse_crossref_work, parse_crossref_works, parse_csl_json};

/// The content type asked of `doi.org`.
pub const CSL_JSON: &str = "application/vnd.citationstyles.csl+json";
/// DOI resolver host.
pub const DOI_HOST: &str = "doi.org";
/// Crossref REST API host.
pub const CROSSREF_HOST: &str = "api.crossref.org";
/// Environment variable holding the contact address for the polite pool.
pub const MAILTO_ENV: &str = "TPE_MAILTO";

/// Characters escaped inside a URL path segment (a DOI suffix may hold
/// parentheses, semicolons and angle brackets, which are kept).
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'[')
    .add(b']')
    .add(b'\\')
    .add(b'^')
    .add(b'|');

/// An HTTP response as the transport saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: String,
    /// The raw `Retry-After` header, if any.
    pub retry_after: Option<String>,
}

/// Something that performs a GET. Returns `Err` only when no response came
/// back (DNS, TLS, timeout); every HTTP status is an `Ok` response.
pub trait Transport: Send + Sync {
    /// GET `url` with `headers`.
    ///
    /// # Errors
    /// When the request produced no response.
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response, String>;
}

/// The network transport.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    /// An agent identifying as `user_agent`, with `timeout` for the whole request.
    pub fn new(user_agent: &str, timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .user_agent(user_agent)
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl Transport for UreqTransport {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response, String> {
        let mut request = self.agent.get(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let mut response = request.call().map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| e.to_string())?;
        Ok(Response {
            status,
            body,
            retry_after,
        })
    }
}

/// How the client behaves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientConfig {
    /// Contact address for the polite pool (`TPE_MAILTO`); never hard-coded.
    pub mailto: Option<String>,
    /// Whole-request timeout.
    pub timeout: Duration,
    /// Least spacing between two requests to the same host.
    pub interval: Duration,
    /// Retries after `429`, `5xx` or a transport failure.
    pub retries: u32,
    /// First backoff; doubled on each retry unless `Retry-After` says otherwise.
    pub backoff: Duration,
    /// Longest wait honoured for one retry.
    pub max_backoff: Duration,
    /// Answer from the cache only; every miss is an error.
    pub offline: bool,
}

impl ClientConfig {
    /// Defaults: 30 s timeout, 200 ms spacing, 4 retries from 1.5 s, 60 s cap.
    pub fn new(mailto: Option<String>) -> Self {
        Self {
            mailto: mailto.filter(|m| !m.trim().is_empty()),
            timeout: Duration::from_secs(30),
            interval: Duration::from_millis(200),
            retries: 4,
            backoff: Duration::from_millis(1500),
            max_backoff: Duration::from_secs(60),
            offline: false,
        }
    }

    /// The `User-Agent` sent: the tool, its version, the project URL and
    /// the contact address when one is configured.
    pub fn user_agent(&self) -> String {
        let base = format!(
            "tpe-refcheck/{} (https://github.com/benpshore/pdftextract",
            env!("CARGO_PKG_VERSION")
        );
        match &self.mailto {
            Some(m) => format!("{base}; mailto:{m})"),
            None => format!("{base})"),
        }
    }
}

/// Request accounting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub requests: u64,
    pub cache_hits: u64,
    pub retries: u64,
}

/// A request that produced no final answer.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FetchError {
    /// Offline mode and the cache had no answer.
    #[error("offline: not in cache")]
    Offline,
    /// No response after the retries.
    #[error("transport: {0}")]
    Transport(String),
    /// Still `429`/`5xx` after the retries.
    #[error("http {0} after retries")]
    Status(u16),
}

/// What a DOI resolved to.
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup {
    /// A record came back.
    Found(Box<Record>),
    /// Both `doi.org` and Crossref say the DOI is not registered.
    NotFound,
    /// The lookup could not be completed.
    Error(String),
}

/// The registry client.
pub struct Client {
    config: ClientConfig,
    transport: Box<dyn Transport>,
    cache: Option<Cache>,
    next_slot: Mutex<HashMap<String, Instant>>,
    sleep: Box<dyn Fn(Duration) + Send + Sync>,
    requests: AtomicU64,
    cache_hits: AtomicU64,
    retries: AtomicU64,
}

/// The host of `url`, lower-cased.
pub fn host_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    rest.split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from((month + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The wait a `Retry-After` header asks for: delay seconds, or an HTTP
/// date (`Sun, 06 Nov 1994 08:49:37 GMT`) relative to `now`. A date in the
/// past is no wait; anything unreadable is `None`.
pub fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let parts: Vec<&str> = value.split_whitespace().collect();
    let [_, day, month, year, time, _] = parts.as_slice() else {
        return None;
    };
    let months = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let month = months
        .iter()
        .position(|m| m.eq_ignore_ascii_case(month))
        .map(|p| u32::try_from(p + 1).ok())??;
    let day: u32 = day.parse().ok()?;
    let year: i64 = year.parse().ok()?;
    let mut hms = time.split(':').map(|p| p.parse::<i64>().ok());
    let (h, m, s) = (hms.next()??, hms.next()??, hms.next()??);
    let secs = days_from_civil(year, month, day) * 86_400 + h * 3600 + m * 60 + s;
    let target = u64::try_from(secs).ok()?;
    let now_secs = now.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(Duration::from_secs(target.saturating_sub(now_secs)))
}

impl Client {
    /// A client on the network, with `cache` in front of it when given.
    pub fn new(config: ClientConfig, cache: Option<Cache>) -> Self {
        let transport = UreqTransport::new(&config.user_agent(), config.timeout);
        Self::with_transport(config, cache, Box::new(transport))
    }

    /// A client on `transport` (tests script one).
    pub fn with_transport(
        config: ClientConfig,
        cache: Option<Cache>,
        transport: Box<dyn Transport>,
    ) -> Self {
        Self {
            config,
            transport,
            cache,
            next_slot: Mutex::new(HashMap::new()),
            sleep: Box::new(std::thread::sleep),
            requests: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            retries: AtomicU64::new(0),
        }
    }

    /// Replace the sleep used for spacing and backoff (tests record it).
    #[must_use]
    pub fn with_sleep(mut self, sleep: Box<dyn Fn(Duration) + Send + Sync>) -> Self {
        self.sleep = sleep;
        self
    }

    /// The configuration.
    pub fn config(&self) -> &ClientConfig {
        &self.config
    }

    /// Requests sent, cache hits and retries so far.
    pub fn stats(&self) -> Stats {
        Stats {
            requests: self.requests.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            retries: self.retries.load(Ordering::Relaxed),
        }
    }

    fn wait_for_slot(&self, host: &str) {
        let now = Instant::now();
        let wait = {
            let mut slots = self
                .next_slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let slot = match slots.get(host) {
                Some(&next) if next > now => next,
                _ => now,
            };
            slots.insert(host.to_string(), slot + self.config.interval);
            slot.saturating_duration_since(now)
        };
        if !wait.is_zero() {
            (self.sleep)(wait);
        }
    }

    fn backoff_for(&self, attempt: u32, retry_after: Option<&str>) -> Duration {
        let asked = retry_after.and_then(|v| parse_retry_after(v, SystemTime::now()));
        let delay = asked.unwrap_or_else(|| {
            self.config
                .backoff
                .saturating_mul(2_u32.saturating_pow(attempt))
        });
        delay.min(self.config.max_backoff)
    }

    /// GET `url` (cache first, under `key`), retrying `429`/`5xx` and
    /// transport failures. Final answers (`2xx`, `404`, `410`) are cached.
    ///
    /// # Errors
    /// Offline without a cached answer, or no final answer after the retries.
    pub fn fetch(
        &self,
        key: &str,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<Response, FetchError> {
        if let Some(hit) = self.cache.as_ref().and_then(|c| c.get(key)) {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(Response {
                status: hit.status,
                body: hit.body,
                retry_after: None,
            });
        }
        if self.config.offline {
            return Err(FetchError::Offline);
        }
        let host = host_of(url);
        let mut attempt = 0_u32;
        loop {
            self.wait_for_slot(&host);
            self.requests.fetch_add(1, Ordering::Relaxed);
            let outcome = self.transport.get(url, headers);
            let (retryable, retry_after, result) = match outcome {
                Ok(resp) if resp.status == 429 || resp.status >= 500 => {
                    let status = resp.status;
                    (true, resp.retry_after, Err(FetchError::Status(status)))
                }
                Ok(resp) => {
                    if matches!(resp.status, 200..=299 | 404 | 410)
                        && let Some(cache) = &self.cache
                        && let Err(e) = cache.put(key, url, resp.status, &resp.body)
                    {
                        eprintln!("tpe-refcheck: cache write failed: {e}");
                    }
                    (false, None, Ok(resp))
                }
                Err(e) => (true, None, Err(FetchError::Transport(e))),
            };
            if !retryable || attempt >= self.config.retries {
                return result;
            }
            let delay = self.backoff_for(attempt, retry_after.as_deref());
            self.retries.fetch_add(1, Ordering::Relaxed);
            (self.sleep)(delay);
            attempt += 1;
        }
    }

    fn crossref_url(&self, path: &str, pairs: &[(&str, &str)]) -> String {
        let mut url = format!("https://{CROSSREF_HOST}{path}");
        let mut first = true;
        let mailto = self.config.mailto.clone();
        let extra: Vec<(&str, &str)> = pairs
            .iter()
            .copied()
            .chain(mailto.as_deref().map(|m| ("mailto", m)))
            .collect();
        for (k, v) in extra {
            url.push(if first { '?' } else { '&' });
            first = false;
            url.push_str(k);
            url.push('=');
            url.push_str(&utf8_percent_encode(v, NON_ALPHANUMERIC).to_string());
        }
        url
    }

    /// Resolve `doi` (already normalised): `doi.org` content negotiation
    /// first, then Crossref `/works/{doi}`.
    pub fn resolve_doi(&self, doi: &str) -> Lookup {
        let encoded = utf8_percent_encode(doi, PATH_SEGMENT).to_string();
        let doi_url = format!("https://{DOI_HOST}/{encoded}");
        match self.fetch(&Cache::doi_key(doi), &doi_url, &[("Accept", CSL_JSON)]) {
            Ok(resp) if resp.status == 200 => {
                if let Ok(record) = parse_csl_json(&resp.body) {
                    return Lookup::Found(Box::new(record));
                }
            }
            Ok(_) => {}
            Err(FetchError::Offline) => return Lookup::Error("offline".to_string()),
            Err(e) => return Lookup::Error(format!("doi.org: {e}")),
        }
        let key = format!("crossref:{}", Cache::doi_key(doi));
        let url = self.crossref_url(&format!("/works/{encoded}"), &[]);
        match self.fetch(&key, &url, &[("Accept", "application/json")]) {
            Ok(resp) if resp.status == 200 => match parse_crossref_work(&resp.body) {
                Ok(record) => Lookup::Found(Box::new(record)),
                Err(e) => Lookup::Error(format!("crossref: {e}")),
            },
            Ok(resp) if resp.status == 404 || resp.status == 410 => Lookup::NotFound,
            Ok(resp) => Lookup::Error(format!("crossref: http {}", resp.status)),
            Err(FetchError::Offline) => Lookup::Error("offline".to_string()),
            Err(e) => Lookup::Error(format!("crossref: {e}")),
        }
    }

    /// Crossref `works?query.bibliographic=<text>&rows=<rows>`.
    ///
    /// # Errors
    /// When the request failed or the response was not a work list.
    pub fn query(&self, text: &str, rows: u32) -> Result<Vec<Record>, String> {
        let rows_text = rows.to_string();
        let url = self.crossref_url(
            "/works",
            &[("query.bibliographic", text), ("rows", &rows_text)],
        );
        let key = Cache::query_key(text, rows);
        match self.fetch(&key, &url, &[("Accept", "application/json")]) {
            Ok(resp) if resp.status == 200 => {
                parse_crossref_works(&resp.body).map_err(|e| format!("crossref: {e}"))
            }
            Ok(resp) => Err(format!("crossref: http {}", resp.status)),
            Err(FetchError::Offline) => Err("offline".to_string()),
            Err(e) => Err(format!("crossref: {e}")),
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use std::sync::Arc;

    use super::*;

    /// Scripted responses, consumed in order; records every URL asked.
    pub struct Script {
        responses: Mutex<Vec<Result<Response, String>>>,
        pub urls: Mutex<Vec<String>>,
    }

    impl Script {
        pub fn new(responses: Vec<Result<Response, String>>) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(responses),
                urls: Mutex::new(Vec::new()),
            })
        }
    }

    impl Transport for Arc<Script> {
        fn get(&self, url: &str, _headers: &[(&str, &str)]) -> Result<Response, String> {
            self.urls.lock().unwrap().push(url.to_string());
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Err("script exhausted".to_string());
            }
            responses.remove(0)
        }
    }

    #[allow(clippy::unnecessary_wraps)]
    fn ok(body: &str) -> Result<Response, String> {
        Ok(Response {
            status: 200,
            body: body.to_string(),
            retry_after: None,
        })
    }

    #[allow(clippy::unnecessary_wraps)]
    fn status(code: u16, retry_after: Option<&str>) -> Result<Response, String> {
        Ok(Response {
            status: code,
            body: String::new(),
            retry_after: retry_after.map(str::to_string),
        })
    }

    fn make_client(
        script: &Arc<Script>,
        cache: Option<Cache>,
    ) -> (Client, Arc<Mutex<Vec<Duration>>>) {
        let sleeps = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&sleeps);
        let mut config = ClientConfig::new(Some("ci@example.org".to_string()));
        config.retries = 2;
        config.backoff = Duration::from_millis(100);
        config.max_backoff = Duration::from_secs(5);
        config.interval = Duration::from_millis(50);
        let client = Client::with_transport(config, cache, Box::new(Arc::clone(script)))
            .with_sleep(Box::new(move |d| recorder.lock().unwrap().push(d)));
        (client, sleeps)
    }

    #[test]
    fn user_agent_carries_the_contact_only_when_configured() {
        let polite = ClientConfig::new(Some("ci@example.org".to_string()));
        assert!(polite.user_agent().ends_with("; mailto:ci@example.org)"));
        assert!(polite.user_agent().starts_with("tpe-refcheck/"));
        let anonymous = ClientConfig::new(Some("  ".to_string()));
        assert!(!anonymous.user_agent().contains("mailto"));
        assert_eq!(anonymous.mailto, None);
    }

    #[test]
    fn retry_after_parses_seconds_and_http_dates() {
        let now = UNIX_EPOCH + Duration::from_secs(784_111_777); // 1994-11-06 08:49:37 UTC
        assert_eq!(parse_retry_after("12", now), Some(Duration::from_secs(12)));
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 08:49:47 GMT", now),
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 08:00:00 GMT", now),
            Some(Duration::ZERO)
        );
        assert_eq!(parse_retry_after("soon", now), None);
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    }

    #[test]
    fn backoff_honours_retry_after_doubles_and_caps() {
        let script = Script::new(vec![
            status(429, Some("3")),
            status(503, None),
            status(500, Some("9999")),
        ]);
        let (client, sleeps) = make_client(&script, None);
        let err = client
            .fetch("doi:x", "https://api.crossref.org/works/x", &[])
            .unwrap_err();
        assert_eq!(err, FetchError::Status(500));
        let sleeps = sleeps.lock().unwrap();
        // Three requests: first is free, the next two wait the host interval;
        // two retries sleep 3 s (Retry-After) and 200 ms (100 ms doubled).
        let backoffs: Vec<Duration> = sleeps
            .iter()
            .copied()
            .filter(|d| *d >= Duration::from_millis(150))
            .collect();
        assert_eq!(
            backoffs,
            vec![Duration::from_secs(3), Duration::from_millis(200)]
        );
        assert_eq!(client.stats().requests, 3);
        assert_eq!(client.stats().retries, 2);
        // Retry-After beyond the cap is clamped.
        assert_eq!(client.backoff_for(0, Some("9999")), Duration::from_secs(5));
    }

    #[test]
    fn transport_failures_retry_then_report() {
        let script = Script::new(vec![Err("dns".to_string()), ok("{}")]);
        let (client, _) = make_client(&script, None);
        let resp = client.fetch("k", "https://doi.org/10.1/x", &[]).unwrap();
        assert_eq!(resp.status, 200);
        let script = Script::new(vec![Err("a".into()), Err("b".into()), Err("c".into())]);
        let (client, _) = make_client(&script, None);
        assert_eq!(
            client
                .fetch("k", "https://doi.org/10.1/x", &[])
                .unwrap_err(),
            FetchError::Transport("c".to_string())
        );
    }

    #[test]
    fn cache_serves_repeat_and_offline_requests_but_not_rate_limits() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path()).unwrap();
        let script = Script::new(vec![
            status(404, None),
            status(429, None),
            status(429, None),
            status(429, None),
        ]);
        let (client, _) = make_client(&script, Some(cache.clone()));
        let miss = client.fetch("doi:a", "https://doi.org/a", &[]).unwrap();
        assert_eq!(miss.status, 404);
        assert_eq!(
            client
                .fetch("doi:a", "https://doi.org/a", &[])
                .unwrap()
                .status,
            404
        );
        assert_eq!(client.stats().cache_hits, 1);
        assert_eq!(client.stats().requests, 1);
        assert_eq!(
            client.fetch("doi:b", "https://doi.org/b", &[]).unwrap_err(),
            FetchError::Status(429)
        );
        assert!(cache.get("doi:b").is_none(), "429 is never cached");

        let offline_script = Script::new(vec![]);
        let mut config = ClientConfig::new(None);
        config.offline = true;
        let offline = Client::with_transport(config, Some(cache), Box::new(offline_script));
        assert_eq!(
            offline
                .fetch("doi:a", "https://doi.org/a", &[])
                .unwrap()
                .status,
            404
        );
        assert_eq!(
            offline
                .fetch("doi:c", "https://doi.org/c", &[])
                .unwrap_err(),
            FetchError::Offline
        );
    }

    #[test]
    fn doi_resolution_falls_back_to_crossref_and_urls_are_polite() {
        let csl = r#"{"DOI":"10.1/a","title":"A","author":[{"family":"Ab"}],"issued":{"date-parts":[[2020]]}}"#;
        let work =
            r#"{"status":"ok","message-type":"work","message":{"DOI":"10.1/b","title":["B"]}}"#;
        let script = Script::new(vec![
            ok(csl),
            status(404, None),
            ok(work),
            status(406, None),
            status(404, None),
        ]);
        let (client, _) = make_client(&script, None);
        assert!(matches!(client.resolve_doi("10.1/a"), Lookup::Found(r) if r.source == "doi.org"));
        assert!(matches!(client.resolve_doi("10.1/b"), Lookup::Found(r) if r.source == "crossref"));
        assert_eq!(client.resolve_doi("10.1/c(d)"), Lookup::NotFound);
        let urls = script.urls.lock().unwrap();
        assert_eq!(urls[0], "https://doi.org/10.1/a");
        assert_eq!(
            urls[2],
            "https://api.crossref.org/works/10.1/b?mailto=ci%40example%2Eorg"
        );
        assert_eq!(
            urls[4],
            "https://api.crossref.org/works/10.1/c(d)?mailto=ci%40example%2Eorg"
        );
    }

    #[test]
    fn query_url_and_errors() {
        let list = r#"{"status":"ok","message-type":"work-list","message":{"items":[{"DOI":"10.1/q","title":["Q"]}]}}"#;
        let script = Script::new(vec![ok(list), status(400, None)]);
        let (client, _) = make_client(&script, None);
        let records = client.query("Smith 2020 A title", 5).unwrap();
        assert_eq!(records[0].doi.as_deref(), Some("10.1/q"));
        assert_eq!(
            script.urls.lock().unwrap()[0],
            "https://api.crossref.org/works?query.bibliographic=Smith%202020%20A%20title&rows=5&mailto=ci%40example%2Eorg"
        );
        assert_eq!(client.query("x", 5).unwrap_err(), "crossref: http 400");
        assert_eq!(
            host_of("https://API.crossref.org/works?x=1"),
            "api.crossref.org"
        );
    }
}
