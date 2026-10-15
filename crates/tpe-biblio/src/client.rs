//! The shared HTTP client: one `ureq` agent, per-host rate limiting, offline mode.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::error::BiblioError;
use crate::retry::parse_retry_after;
use crate::util::host_of;

/// Environment variable holding the contact address for polite pools
/// (Crossref, `OpenAlex`, Unpaywall, NCBI). Never hard-code an address: the
/// person running the tool supplies it.
pub const MAILTO_ENV: &str = "TPE_MAILTO";
/// Environment variable holding the on-disk cache directory for registry
/// responses; unset means no cache.
pub const CACHE_DIR_ENV: &str = "TPE_BIBLIO_CACHE_DIR";

/// The contact address from [`MAILTO_ENV`], trimmed; `None` when unset or empty.
pub fn mailto_from_env() -> Option<String> {
    std::env::var(MAILTO_ENV)
        .ok()
        .and_then(|v| crate::util::non_empty(&v))
}

/// The cache directory from [`CACHE_DIR_ENV`]; `None` when unset or empty.
pub fn cache_dir_from_env() -> Option<std::path::PathBuf> {
    std::env::var(CACHE_DIR_ENV)
        .ok()
        .and_then(|v| crate::util::non_empty(&v))
        .map(std::path::PathBuf::from)
}

/// A polite-pool `User-Agent`: `product/version (mailto:address)` when an
/// address is known, else `product/version`. Crossref and `OpenAlex` route
/// requests carrying a `mailto:` to their steadier polite pools.
pub fn polite_user_agent(product: &str, version: &str, mailto: Option<&str>) -> String {
    match mailto.and_then(crate::util::non_empty) {
        Some(m) => format!("{product}/{version} (mailto:{m})"),
        None => format!("{product}/{version}"),
    }
}

/// One HTTP response as the sources see it: the status, the `Retry-After`
/// header when present, and the body as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    /// HTTP status code.
    pub status: u16,
    /// Raw `Retry-After` header value, if any.
    pub retry_after: Option<String>,
    /// Response body as text.
    pub body: String,
}

impl Response {
    /// `Ok(body)` for a 2xx status; otherwise the same errors [`Client::get_text`]
    /// reports (404 is `NotFound`, 429 is `RateLimited` or, with a usable
    /// `Retry-After`, `RetryAfter`; 503 with `Retry-After` is `RetryAfter`).
    pub fn into_text(self, now_unix: u64) -> Result<String, BiblioError> {
        match self.status {
            200..=299 => Ok(self.body),
            404 => Err(BiblioError::NotFound),
            status @ (429 | 503) => {
                let after = self
                    .retry_after
                    .as_deref()
                    .and_then(|v| parse_retry_after(v, now_unix));
                match (status, after) {
                    (_, Some(after)) => Err(BiblioError::RetryAfter { status, after }),
                    (429, None) => Err(BiblioError::RateLimited),
                    (status, None) => Err(BiblioError::Status(status)),
                }
            }
            status => Err(BiblioError::Status(status)),
        }
    }
}

/// Seconds since the Unix epoch, or 0 before it.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Key name for the `OpenAlex` API key (sent as the `api_key` query parameter).
pub const KEY_OPENALEX: &str = "openalex";
/// Key name for the Semantic Scholar API key (sent as the `x-api-key` header).
pub const KEY_SEMANTIC_SCHOLAR: &str = "semantic_scholar";
/// Key name for the NCBI E-utilities API key (sent as the `api_key` query parameter).
pub const KEY_NCBI: &str = "ncbi";

/// Default minimum spacing between two requests to the same host.
pub const DEFAULT_MIN_INTERVAL: Duration = Duration::from_millis(100);

/// Per-host request spacing. Pure: the caller supplies the current time, so it
/// can be tested with synthetic instants.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    default_interval: Duration,
    per_host: HashMap<String, Duration>,
    next_free: HashMap<String, Instant>,
}

impl RateLimiter {
    /// A limiter spacing requests to every host by `default_interval`.
    pub fn new(default_interval: Duration) -> Self {
        Self {
            default_interval,
            per_host: HashMap::new(),
            next_free: HashMap::new(),
        }
    }

    /// Override the spacing for one host (lower-case host name).
    pub fn set_host_interval(&mut self, host: &str, interval: Duration) {
        self.per_host.insert(host.to_ascii_lowercase(), interval);
    }

    /// The spacing that applies to `host`.
    pub fn interval_for(&self, host: &str) -> Duration {
        self.per_host
            .get(host)
            .copied()
            .unwrap_or(self.default_interval)
    }

    /// Reserve the next request slot for `host` and return how long the caller
    /// must wait (from `now`) before sending.
    pub fn reserve(&mut self, host: &str, now: Instant) -> Duration {
        let slot = match self.next_free.get(host) {
            Some(&next) if next > now => next,
            _ => now,
        };
        let interval = self.interval_for(host);
        self.next_free.insert(host.to_string(), slot + interval);
        slot.saturating_duration_since(now)
    }
}

/// HTTP client shared by all bibliographic sources.
pub struct Client {
    agent: ureq::Agent,
    user_agent: String,
    mailto: Option<String>,
    keys: HashMap<&'static str, String>,
    offline: bool,
    link_resolver: Option<String>,
    limiter: Mutex<RateLimiter>,
    now: fn() -> Instant,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut key_names: Vec<&str> = self.keys.keys().copied().collect();
        key_names.sort_unstable();
        f.debug_struct("Client")
            .field("user_agent", &self.user_agent)
            .field("mailto", &self.mailto)
            .field("keys", &key_names)
            .field("offline", &self.offline)
            .field("link_resolver", &self.link_resolver)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// A client with the given `User-Agent` (include a contact address, as
    /// Crossref and `OpenAlex` ask), 30 s timeout, and default host spacing.
    pub fn new(user_agent: &str) -> Self {
        let config = ureq::Agent::config_builder()
            .user_agent(user_agent)
            .timeout_global(Some(Duration::from_secs(30)))
            // Statuses are read by `Response::into_text`, so `Retry-After` survives.
            .http_status_as_error(false)
            .build();
        let agent: ureq::Agent = config.into();
        let mut limiter = RateLimiter::new(DEFAULT_MIN_INTERVAL);
        // Semantic Scholar allows about 1 request/s; E-utilities 3/s without a key.
        limiter.set_host_interval("api.semanticscholar.org", Duration::from_millis(1100));
        limiter.set_host_interval("eutils.ncbi.nlm.nih.gov", Duration::from_millis(350));
        Self {
            agent,
            user_agent: user_agent.to_string(),
            mailto: None,
            keys: HashMap::new(),
            offline: false,
            link_resolver: None,
            limiter: Mutex::new(limiter),
            now: Instant::now,
        }
    }

    /// A polite client for `product`/`version`: the `User-Agent` carries
    /// `mailto:` when an address is given and the same address is sent as
    /// the `mailto` query parameter. Read the address with [`mailto_from_env`].
    pub fn polite(product: &str, version: &str, mailto: Option<&str>) -> Self {
        let mut client = Self::new(&polite_user_agent(product, version, mailto));
        if let Some(m) = mailto {
            client = client.with_mailto(m);
        }
        client
    }

    /// The configured `User-Agent`.
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// Set the contact e-mail sent as `mailto` / `email` (polite pools, Unpaywall).
    #[must_use]
    pub fn with_mailto(mut self, mailto: &str) -> Self {
        self.mailto = crate::util::non_empty(mailto);
        self
    }

    /// Store an API key under one of the `KEY_*` names.
    #[must_use]
    pub fn with_key(mut self, name: &'static str, value: &str) -> Self {
        self.keys.insert(name, value.to_string());
        self
    }

    /// Enable or disable offline mode (every request fails with `BiblioError::Offline`).
    #[must_use]
    pub fn with_offline(mut self, offline: bool) -> Self {
        self.offline = offline;
        self
    }

    /// Set the base URL of the library's `OpenURL` link resolver.
    #[must_use]
    pub fn with_link_resolver(mut self, base_url: &str) -> Self {
        self.link_resolver = crate::util::non_empty(base_url);
        self
    }

    /// Override the minimum spacing for one host.
    #[must_use]
    pub fn with_host_interval(self, host: &str, interval: Duration) -> Self {
        self.limiter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_host_interval(host, interval);
        self
    }

    /// Replace the clock used by the rate limiter (tests use a frozen clock).
    #[must_use]
    pub fn with_clock(mut self, now: fn() -> Instant) -> Self {
        self.now = now;
        self
    }

    /// The configured contact e-mail.
    pub fn mailto(&self) -> Option<&str> {
        self.mailto.as_deref()
    }

    /// The API key stored under `name`, if any.
    pub fn key(&self, name: &str) -> Option<&str> {
        self.keys.get(name).map(String::as_str)
    }

    /// Whether offline mode is on.
    pub fn is_offline(&self) -> bool {
        self.offline
    }

    /// The configured link-resolver base URL.
    pub fn link_resolver(&self) -> Option<&str> {
        self.link_resolver.as_deref()
    }

    /// Reserve a request slot for `host`; returns the delay to wait before sending.
    pub fn reserve_slot(&self, host: &str) -> Duration {
        let now = (self.now)();
        self.limiter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reserve(&host.to_ascii_lowercase(), now)
    }

    /// GET `url` with extra `headers` and return the body as text. Respects
    /// offline mode and the per-host rate limit. Errors never contain the URL.
    pub fn get_text(&self, url: &str, headers: &[(&str, &str)]) -> Result<String, BiblioError> {
        self.get(url, headers)?.into_text(unix_now())
    }

    /// GET `url` and return the status, `Retry-After` and body, so a caller
    /// can honour the server's delay. Transport failures are still errors.
    pub fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response, BiblioError> {
        if self.offline {
            return Err(BiblioError::Offline);
        }
        let wait = self.reserve_slot(&host_of(url));
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        let mut request = self.agent.get(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let mut response = request.call().map_err(|e| BiblioError::from_ureq(&e))?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| BiblioError::from_ureq(&e))?;
        Ok(Response {
            status,
            retry_after,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::*;

    #[test]
    fn limiter_spaces_requests_per_host() {
        let t0 = Instant::now();
        let mut limiter = RateLimiter::new(Duration::from_millis(100));
        assert_eq!(limiter.reserve("a", t0), Duration::ZERO);
        assert_eq!(limiter.reserve("a", t0), Duration::from_millis(100));
        assert_eq!(limiter.reserve("a", t0), Duration::from_millis(200));
        assert_eq!(limiter.reserve("b", t0), Duration::ZERO);
        // Well after the reserved slots: no wait.
        let later = t0 + Duration::from_secs(5);
        assert_eq!(limiter.reserve("a", later), Duration::ZERO);
        // Partly elapsed: wait only for the remainder.
        let mid = later + Duration::from_millis(40);
        assert_eq!(limiter.reserve("a", mid), Duration::from_millis(60));
    }

    #[test]
    fn limiter_honours_host_override() {
        let t0 = Instant::now();
        let mut limiter = RateLimiter::new(Duration::from_millis(100));
        limiter.set_host_interval("slow.example", Duration::from_secs(1));
        assert_eq!(limiter.reserve("slow.example", t0), Duration::ZERO);
        assert_eq!(limiter.reserve("slow.example", t0), Duration::from_secs(1));
    }

    #[test]
    fn client_uses_injected_clock() {
        fn frozen() -> Instant {
            static T: OnceLock<Instant> = OnceLock::new();
            *T.get_or_init(Instant::now)
        }
        let client = Client::new("tpe-biblio-test")
            .with_host_interval("api.example.org", Duration::from_millis(250))
            .with_clock(frozen);
        assert_eq!(client.reserve_slot("api.example.org"), Duration::ZERO);
        assert_eq!(
            client.reserve_slot("API.example.org"),
            Duration::from_millis(250)
        );
        assert_eq!(
            client.reserve_slot("api.example.org"),
            Duration::from_millis(500)
        );
    }

    #[test]
    fn offline_client_refuses_requests() {
        let client = Client::new("tpe-biblio-test").with_offline(true);
        let result = client.get_text("https://api.openalex.org/works", &[]);
        assert!(matches!(result, Err(BiblioError::Offline)));
    }

    #[test]
    fn polite_user_agent_and_env_placeholder() {
        assert_eq!(
            polite_user_agent("tpe", "1.0", Some("someone@example.org")),
            "tpe/1.0 (mailto:someone@example.org)"
        );
        assert_eq!(polite_user_agent("tpe", "1.0", None), "tpe/1.0");
        assert_eq!(polite_user_agent("tpe", "1.0", Some("  ")), "tpe/1.0");
        let client = Client::polite("tpe", "1.0", Some("someone@example.org"));
        assert_eq!(client.user_agent(), "tpe/1.0 (mailto:someone@example.org)");
        assert_eq!(client.mailto(), Some("someone@example.org"));
        assert_eq!(MAILTO_ENV, "TPE_MAILTO");
    }

    #[test]
    fn response_status_mapping_honours_retry_after() {
        let ok = Response {
            status: 200,
            retry_after: None,
            body: "x".into(),
        };
        assert_eq!(ok.into_text(0).unwrap(), "x");
        let missing = Response {
            status: 404,
            retry_after: None,
            body: String::new(),
        };
        assert!(matches!(missing.into_text(0), Err(BiblioError::NotFound)));
        let limited = Response {
            status: 429,
            retry_after: None,
            body: String::new(),
        };
        assert!(matches!(
            limited.into_text(0),
            Err(BiblioError::RateLimited)
        ));
        let later = Response {
            status: 429,
            retry_after: Some("7".into()),
            body: String::new(),
        };
        assert!(matches!(
            later.into_text(0),
            Err(BiblioError::RetryAfter { status: 429, after }) if after == Duration::from_secs(7)
        ));
        let busy = Response {
            status: 503,
            retry_after: Some("garbage".into()),
            body: String::new(),
        };
        assert!(matches!(busy.into_text(0), Err(BiblioError::Status(503))));
        let other = Response {
            status: 500,
            retry_after: None,
            body: String::new(),
        };
        assert!(matches!(other.into_text(0), Err(BiblioError::Status(500))));
    }

    #[test]
    fn debug_hides_key_values() {
        let client = Client::new("ua").with_key(KEY_OPENALEX, "SECRET-KEY-VALUE");
        let shown = format!("{client:?}");
        assert!(shown.contains("openalex"));
        assert!(!shown.contains("SECRET-KEY-VALUE"));
        assert_eq!(client.key(KEY_OPENALEX), Some("SECRET-KEY-VALUE"));
    }
}
