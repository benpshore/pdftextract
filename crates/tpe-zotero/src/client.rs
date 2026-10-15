//! Zotero Web API v3 client (blocking, over `ureq`).
//!
//! Verified against the Zotero Web API documentation (basics and write
//! requests pages, last updated 2026-07-29):
//! * base URL `https://api.zotero.org`, library prefix `/users/<id>` or
//!   `/groups/<id>`;
//! * `Zotero-API-Version: 3` request header, key in `Zotero-API-Key`;
//! * multi-object reads return `Total-Results`, `Last-Modified-Version` and a
//!   `Link` header with `rel="next"`; `limit` is 1–100 (default 25);
//! * `POST <prefix>/items` takes up to 50 objects and an optional
//!   32-character `Zotero-Write-Token` for unversioned writes; the 200
//!   response has `successful` / `success`, `unchanged` and `failed` maps
//!   keyed by the index in the uploaded array;
//! * `PATCH <prefix>/items/<key>` with `If-Unmodified-Since-Version`
//!   returns 204, or 412 when the item changed;
//! * 403 = bad key or privileges, 412/428 = version or write-token problems,
//!   429 = rate limited (`Retry-After`), `Backoff` may appear on any response.

use std::collections::BTreeMap;
use std::collections::hash_map::RandomState;
use std::fmt;
use std::fmt::Write as _;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::error::ZError;
use crate::headers::{link_next, parse_u64};
use crate::item::{ZItem, ZItemPatch};
use crate::retry::{RetryPolicy, backoff_delay, retry_delay};
use crate::schema::{
    NamedEntry, ZGroup, ZKeyInfo, ZTag, collection_body, parse_groups, parse_key_info,
    parse_named_list, parse_tags, parse_template,
};
use crate::upload::{
    FileDescriptor, UploadAuthorization, UploadTarget, parse_upload_authorization,
    registration_body,
};

/// Default API base URL.
pub const DEFAULT_BASE: &str = "https://api.zotero.org";
/// API version requested with every call.
pub const API_VERSION: &str = "3";
/// Largest `limit` the Web API accepts for multi-object reads.
pub const MAX_LIMIT: u32 = 100;
/// Longest pause honoured for a `Backoff` header before the next request.
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// Most objects one write request may carry.
pub const MAX_WRITE_ITEMS: usize = 50;
/// `User-Agent` sent by this client.
pub const USER_AGENT: &str = "text-processing-engine-zotero/0.1";

/// Which library a client talks to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Library {
    /// A user library (numeric user id, not the username).
    User(u64),
    /// A group library (numeric group id).
    Group(u64),
}

impl Library {
    /// URL prefix: `/users/<id>` or `/groups/<id>`.
    pub fn prefix(self) -> String {
        match self {
            Self::User(id) => format!("/users/{id}"),
            Self::Group(id) => format!("/groups/{id}"),
        }
    }
}

/// A Zotero API key. `Debug` never prints the value.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wrap a key obtained from the credential store.
    pub fn new(value: &str) -> Self {
        Self(value.trim().to_string())
    }

    /// The raw key, for the request header only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(***)")
    }
}

/// Parameters of an items read. `None` fields are not sent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemQuery {
    /// Quick search (`q`), titles and creators by default.
    pub q: Option<String>,
    /// `itemType` search syntax, e.g. `journalArticle || preprint` or `-attachment`.
    pub item_type: Option<String>,
    /// Only objects modified after this library version (`since`).
    pub since: Option<u64>,
    /// Page size (clamped to 1..=100).
    pub limit: Option<u32>,
    /// Index of the first result (`start`).
    pub start: Option<u32>,
    /// Sort field (`dateModified`, `title`, `date`, ...).
    pub sort: Option<String>,
    /// Read `/items/top` (no child notes or attachments) instead of `/items`.
    pub top: bool,
}

/// One page of a multi-object read.
#[derive(Clone, Debug, PartialEq)]
pub struct Page<T> {
    /// The objects on this page.
    pub items: Vec<T>,
    /// `Total-Results` header.
    pub total_results: Option<u64>,
    /// `Last-Modified-Version` header (library version).
    pub last_modified_version: Option<u64>,
    /// URL of the next page from the `Link` header.
    pub next: Option<String>,
    /// `Backoff` header in seconds, when the server asked clients to slow down.
    pub backoff_secs: Option<u64>,
}

/// A collection in the library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZCollection {
    /// Collection key.
    pub key: String,
    /// Collection version.
    pub version: u64,
    /// Display name.
    pub name: String,
    /// Parent collection key (`None` for top-level collections).
    pub parent: Option<String>,
}

/// One object the server refused in a write request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteFailure {
    /// Object key, when the server echoed one.
    pub key: Option<String>,
    /// Per-object HTTP-style code.
    pub code: u16,
    /// Server message.
    pub message: String,
}

/// Outcome of a multi-object write, keyed by index in the uploaded array.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteResult {
    /// Index -> key of objects created or modified.
    pub successful: BTreeMap<usize, String>,
    /// Index -> key of objects that were already identical.
    pub unchanged: BTreeMap<usize, String>,
    /// Index -> failure details.
    pub failed: BTreeMap<usize, WriteFailure>,
    /// Library version assigned to the successful objects.
    pub last_modified_version: Option<u64>,
}

impl WriteResult {
    /// The key of the object uploaded at `index` (created, modified or
    /// unchanged), or the server's refusal as [`ZError::WriteFailed`].
    pub fn key_at(&self, index: usize) -> Result<String, ZError> {
        if let Some(key) = self
            .successful
            .get(&index)
            .or_else(|| self.unchanged.get(&index))
        {
            return Ok(key.clone());
        }
        match self.failed.get(&index) {
            Some(failure) => Err(ZError::WriteFailed {
                code: failure.code,
                message: failure.message.clone(),
            }),
            None => Err(ZError::Parse(format!(
                "write response has no entry for object {index}"
            ))),
        }
    }
}

/// A response reduced to what the parsers need; lets them be tested on
/// recorded data without a network.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawResponse {
    /// HTTP status code.
    pub status: u16,
    /// Header names and values in received order.
    pub headers: Vec<(String, String)>,
    /// Body text (empty for 204).
    pub body: String,
}

impl RawResponse {
    /// First header with this name (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Convert a `ureq` response, reading the whole body.
    fn from_http(response: ureq::http::Response<ureq::Body>) -> Result<Self, ZError> {
        let (parts, mut body) = response.into_parts();
        let headers: Vec<(String, String)> = parts
            .headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.as_str().to_string(), v.to_string()))
            })
            .collect();
        let text = body.read_to_string()?;
        Ok(Self {
            status: parts.status.as_u16(),
            headers,
            body: text,
        })
    }
}

/// Map error statuses to [`ZError`]; 2xx and 304 pass through.
pub fn check_status(raw: RawResponse) -> Result<RawResponse, ZError> {
    match raw.status {
        200..=299 | 304 => Ok(raw),
        400 => Err(ZError::BadRequest(truncate(&raw.body, 300))),
        403 => Err(ZError::Forbidden),
        404 => Err(ZError::NotFound(truncate(&raw.body, 300))),
        409 => Err(ZError::Conflict),
        412 => Err(ZError::PreconditionFailed),
        413 => Err(ZError::TooLarge(truncate(&raw.body, 300))),
        428 => Err(ZError::PreconditionRequired),
        429 => Err(ZError::RateLimited {
            retry_after_secs: parse_u64(raw.header("Retry-After")),
        }),
        code => Err(ZError::Status {
            code,
            message: truncate(&raw.body, 300),
        }),
    }
}

/// Parse a multi-object items response (body + paging headers).
pub fn parse_items_page(raw: &RawResponse) -> Result<Page<ZItem>, ZError> {
    let items = if raw.status == 304 {
        Vec::new()
    } else {
        ZItem::parse_many(&raw.body)?
    };
    Ok(page_from(raw, items))
}

/// Parse a multi-object collections response body.
pub fn parse_collections(body: &str) -> Result<Vec<ZCollection>, ZError> {
    let value: Value = serde_json::from_str(body)?;
    let array = value
        .as_array()
        .ok_or_else(|| ZError::Parse("expected a JSON array of collections".to_string()))?;
    let mut out = Vec::with_capacity(array.len());
    for entry in array {
        let data = entry
            .get("data")
            .and_then(Value::as_object)
            .ok_or_else(|| ZError::Parse("collection without a data object".to_string()))?;
        let key = entry
            .get("key")
            .and_then(Value::as_str)
            .or_else(|| data.get("key").and_then(Value::as_str))
            .ok_or_else(|| ZError::Parse("collection without a key".to_string()))?;
        out.push(ZCollection {
            key: key.to_string(),
            version: entry.get("version").and_then(Value::as_u64).unwrap_or(0),
            name: data
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            // `parentCollection` is a key, or `false` for top-level collections.
            parent: data
                .get("parentCollection")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }
    Ok(out)
}

/// Parse the 200 response of a multi-object write. Accepts both
/// `success` (index -> key) and `successful` (index -> saved object).
pub fn parse_write_result(
    body: &str,
    last_modified_version: Option<u64>,
) -> Result<WriteResult, ZError> {
    let value: Value = serde_json::from_str(body)?;
    let object = value
        .as_object()
        .ok_or_else(|| ZError::Parse("write response is not a JSON object".to_string()))?;
    let mut result = WriteResult {
        last_modified_version,
        ..WriteResult::default()
    };
    if let Some(map) = object.get("success").and_then(Value::as_object) {
        for (index, entry) in map {
            if let (Ok(i), Some(key)) = (index.parse::<usize>(), entry.as_str()) {
                result.successful.insert(i, key.to_string());
            }
        }
    }
    if let Some(map) = object.get("successful").and_then(Value::as_object) {
        for (index, entry) in map {
            let key = entry
                .get("key")
                .and_then(Value::as_str)
                .or_else(|| entry.as_str());
            if let (Ok(i), Some(key)) = (index.parse::<usize>(), key) {
                result
                    .successful
                    .entry(i)
                    .or_insert_with(|| key.to_string());
            }
        }
    }
    if let Some(map) = object.get("unchanged").and_then(Value::as_object) {
        for (index, entry) in map {
            if let (Ok(i), Some(key)) = (index.parse::<usize>(), entry.as_str()) {
                result.unchanged.insert(i, key.to_string());
            }
        }
    }
    if let Some(map) = object.get("failed").and_then(Value::as_object) {
        for (index, entry) in map {
            let Ok(i) = index.parse::<usize>() else {
                continue;
            };
            let code = entry
                .get("code")
                .and_then(Value::as_u64)
                .and_then(|c| u16::try_from(c).ok())
                .unwrap_or(0);
            result.failed.insert(
                i,
                WriteFailure {
                    key: entry.get("key").and_then(Value::as_str).map(str::to_string),
                    code,
                    message: entry
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                },
            );
        }
    }
    Ok(result)
}

/// JSON body for `POST <prefix>/items`; refuses more than 50 objects.
pub fn write_items_body(items: &[ZItemPatch]) -> Result<String, ZError> {
    if items.len() > MAX_WRITE_ITEMS {
        return Err(ZError::TooMany { count: items.len() });
    }
    let array: Vec<Value> = items.iter().map(ZItemPatch::to_json).collect();
    Ok(serde_json::to_string(&Value::Array(array))?)
}

/// Check that an object key is 8 ASCII alphanumerics, so it is safe to put in
/// a URL path. (Server-generated keys use `[23456789ABCDEFGHIJKLMNPQRSTUVWXYZ]`.)
pub fn validate_key(key: &str) -> Result<(), ZError> {
    if key.len() == 8 && key.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(())
    } else {
        Err(ZError::InvalidKey(key.to_string()))
    }
}

/// Percent-encode a query-string value (RFC 3986 unreserved characters kept).
pub fn encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// A client-generated 32-character hexadecimal `Zotero-Write-Token`. It only
/// has to be unique per request (the server uses it to drop duplicate
/// submissions), so the standard library's randomly keyed hasher suffices.
pub fn new_write_token() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let state = RandomState::new();
    let mut first = state.build_hasher();
    first.write_u128(nanos);
    first.write_u64(count);
    let high = first.finish();
    let mut second = state.build_hasher();
    second.write_u64(high);
    second.write_u64(count ^ 0x9E37_79B9_7F4A_7C15);
    let low = second.finish();
    format!("{high:016x}{low:016x}")
}

fn page_from<T>(raw: &RawResponse, items: Vec<T>) -> Page<T> {
    Page {
        items,
        total_results: parse_u64(raw.header("Total-Results")),
        last_modified_version: parse_u64(raw.header("Last-Modified-Version")),
        next: raw.header("Link").and_then(link_next),
        backoff_secs: parse_u64(raw.header("Backoff")),
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn build_url(base: &str, params: &[(&str, String)]) -> String {
    if params.is_empty() {
        return base.to_string();
    }
    let joined: Vec<String> = params
        .iter()
        .map(|(name, value)| format!("{name}={}", encode_component(value)))
        .collect();
    format!("{base}?{}", joined.join("&"))
}

/// HTTP methods that carry a JSON body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriteMethod {
    Post,
    Patch,
}

/// Blocking Zotero Web API v3 client for one library.
///
/// Every request first waits out a pending `Backoff` (any earlier response
/// may carry one), and `429` / `5xx` / transport failures are retried
/// according to the [`RetryPolicy`] (default: three attempts).
pub struct ZoteroClient {
    base: String,
    library: Library,
    key: Option<ApiKey>,
    agent: ureq::Agent,
    offline: bool,
    retry: RetryPolicy,
    backoff_until: Mutex<Option<Instant>>,
}

impl fmt::Debug for ZoteroClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZoteroClient")
            .field("base", &self.base)
            .field("library", &self.library)
            .field("key", &self.key)
            .field("offline", &self.offline)
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

impl ZoteroClient {
    /// A client for `library` at [`DEFAULT_BASE`]. Status codes are handled
    /// by this crate (the agent does not turn 4xx/5xx into errors); the
    /// global timeout is 60 s.
    pub fn new(library: Library, key: Option<ApiKey>) -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent(USER_AGENT)
            .build();
        Self {
            base: DEFAULT_BASE.to_string(),
            library,
            key,
            agent: ureq::Agent::new_with_config(config),
            offline: false,
            retry: RetryPolicy::default(),
            backoff_until: Mutex::new(None),
        }
    }

    /// Use another base URL (for example a test server); trailing `/` removed.
    #[must_use]
    pub fn with_base(mut self, base: &str) -> Self {
        self.base = base.trim_end_matches('/').to_string();
        self
    }

    /// Change how `429`, `5xx` and transport failures are retried.
    #[must_use]
    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry = policy;
        self
    }

    /// The retry policy in force.
    pub fn retry_policy(&self) -> RetryPolicy {
        self.retry
    }

    /// Absolute URL of a path outside any library (`/itemTypes`,
    /// `/items/new`, `/keys/current`, ...).
    pub fn api_url(&self, path: &str) -> String {
        let base = &self.base;
        format!("{base}{path}")
    }

    /// When `true`, every request fails with [`ZError::Offline`] before any
    /// network access.
    #[must_use]
    pub fn offline(mut self, offline: bool) -> Self {
        self.offline = offline;
        self
    }

    /// The library this client reads and writes.
    pub fn library(&self) -> Library {
        self.library
    }

    /// Absolute URL of a path inside the library (`path` starts with `/`).
    pub fn library_url(&self, path: &str) -> String {
        let base = &self.base;
        let prefix = self.library.prefix();
        format!("{base}{prefix}{path}")
    }

    /// URL of an items read with the given query.
    pub fn items_url(&self, query: &ItemQuery) -> String {
        let path = if query.top { "/items/top" } else { "/items" };
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(q) = &query.q {
            params.push(("q", q.clone()));
        }
        if let Some(item_type) = &query.item_type {
            params.push(("itemType", item_type.clone()));
        }
        if let Some(since) = query.since {
            params.push(("since", since.to_string()));
        }
        if let Some(limit) = query.limit {
            params.push(("limit", limit.clamp(1, MAX_LIMIT).to_string()));
        }
        if let Some(start) = query.start {
            params.push(("start", start.to_string()));
        }
        if let Some(sort) = &query.sort {
            params.push(("sort", sort.clone()));
        }
        build_url(&self.library_url(path), &params)
    }

    /// URL of an attachment's file (`/items/<key>/file`). Downloading it
    /// needs the same `Zotero-API-Key` header; the API redirects to storage.
    pub fn attachment_file_url(&self, key: &str) -> Result<String, ZError> {
        validate_key(key)?;
        Ok(self.library_url(&format!("/items/{key}/file")))
    }

    /// One page of items.
    pub fn items(&self, query: &ItemQuery) -> Result<Page<ZItem>, ZError> {
        let raw = self.get(&self.items_url(query))?;
        parse_items_page(&raw)
    }

    /// The page after `page`, following its `Link: rel="next"` URL; `None`
    /// on the last page. Links outside the API base are refused so the key
    /// is never sent elsewhere.
    pub fn next_page(&self, page: &Page<ZItem>) -> Result<Option<Page<ZItem>>, ZError> {
        let Some(next) = &page.next else {
            return Ok(None);
        };
        self.check_same_origin(next)?;
        let raw = self.get(next)?;
        parse_items_page(&raw).map(Some)
    }

    /// One item by key.
    pub fn item(&self, key: &str) -> Result<ZItem, ZError> {
        validate_key(key)?;
        let raw = self.get(&self.library_url(&format!("/items/{key}")))?;
        ZItem::parse_one(&raw.body)
    }

    /// Every item matching `query` (all pages, following `Link: rel="next"`;
    /// `start` and `limit` in the query only position the first page).
    pub fn all_items(&self, query: &ItemQuery) -> Result<Vec<ZItem>, ZError> {
        self.collect_pages(self.items_url(query), ZItem::parse_many)
    }

    /// Child notes and attachments of an item (all pages).
    pub fn children(&self, key: &str) -> Result<Vec<ZItem>, ZError> {
        validate_key(key)?;
        let url = build_url(
            &self.library_url(&format!("/items/{key}/children")),
            &[("limit", MAX_LIMIT.to_string())],
        );
        self.collect_pages(url, ZItem::parse_many)
    }

    /// All collections in the library (all pages).
    pub fn collections(&self) -> Result<Vec<ZCollection>, ZError> {
        let url = build_url(
            &self.library_url("/collections"),
            &[("limit", MAX_LIMIT.to_string())],
        );
        self.collect_pages(url, parse_collections)
    }

    /// All tags in the library (all pages).
    pub fn tags(&self) -> Result<Vec<ZTag>, ZError> {
        let url = build_url(
            &self.library_url("/tags"),
            &[("limit", MAX_LIMIT.to_string())],
        );
        self.collect_pages(url, parse_tags)
    }

    /// Follow `Link: rel="next"` from `first_url` until the last page,
    /// refusing links that leave the API base. `Backoff` headers are
    /// honoured between pages like between any two requests.
    fn collect_pages<T>(
        &self,
        first_url: String,
        parse: fn(&str) -> Result<Vec<T>, ZError>,
    ) -> Result<Vec<T>, ZError> {
        let mut out = Vec::new();
        let mut next = Some(first_url);
        while let Some(url) = next {
            self.check_same_origin(&url)?;
            let raw = self.get(&url)?;
            out.extend(parse(&raw.body)?);
            next = raw.header("Link").and_then(link_next);
        }
        Ok(out)
    }

    /// The empty template for a new item of `item_type` (`GET
    /// /items/new?itemType=…`); `link_mode` is required for attachments.
    pub fn item_template(
        &self,
        item_type: &str,
        link_mode: Option<&str>,
    ) -> Result<Map<String, Value>, ZError> {
        let mut params = vec![("itemType", item_type.to_string())];
        if let Some(mode) = link_mode {
            params.push(("linkMode", mode.to_string()));
        }
        let raw = self.get(&build_url(&self.api_url("/items/new"), &params))?;
        parse_template(&raw.body)
    }

    /// All item types (`GET /itemTypes`).
    pub fn item_types(&self) -> Result<Vec<NamedEntry>, ZError> {
        let raw = self.get(&self.api_url("/itemTypes"))?;
        parse_named_list(&raw.body)
    }

    /// The valid fields of an item type (`GET /itemTypeFields?itemType=…`).
    pub fn item_type_fields(&self, item_type: &str) -> Result<Vec<NamedEntry>, ZError> {
        let url = build_url(
            &self.api_url("/itemTypeFields"),
            &[("itemType", item_type.to_string())],
        );
        let raw = self.get(&url)?;
        parse_named_list(&raw.body)
    }

    /// The valid creator types of an item type
    /// (`GET /itemTypeCreatorTypes?itemType=…`).
    pub fn item_type_creator_types(&self, item_type: &str) -> Result<Vec<NamedEntry>, ZError> {
        let url = build_url(
            &self.api_url("/itemTypeCreatorTypes"),
            &[("itemType", item_type.to_string())],
        );
        let raw = self.get(&url)?;
        parse_named_list(&raw.body)
    }

    /// The user id and permissions behind the configured key
    /// (`GET /keys/current`; the key travels in the header, not the URL).
    pub fn key_info(&self) -> Result<ZKeyInfo, ZError> {
        if self.key.is_none() {
            return Err(ZError::MissingKey);
        }
        let raw = self.get(&self.api_url("/keys/current"))?;
        parse_key_info(&raw.body)
    }

    /// The groups `user_id` belongs to (`GET /users/<id>/groups`, all pages).
    pub fn groups(&self, user_id: u64) -> Result<Vec<ZGroup>, ZError> {
        let url = build_url(
            &self.api_url(&format!("/users/{user_id}/groups")),
            &[("limit", MAX_LIMIT.to_string())],
        );
        self.collect_pages(url, parse_groups)
    }

    /// Create a collection (`POST <prefix>/collections`, write-token
    /// guarded). `parent` is the key of the parent collection.
    pub fn create_collection(
        &self,
        name: &str,
        parent: Option<&str>,
    ) -> Result<WriteResult, ZError> {
        if let Some(parent) = parent {
            validate_key(parent)?;
        }
        let body = serde_json::to_string(&Value::Array(vec![collection_body(name, parent)]))?;
        let token = new_write_token();
        let raw = self.send_json(
            WriteMethod::Post,
            &self.library_url("/collections"),
            &body,
            &[("Zotero-Write-Token", token)],
        )?;
        parse_write_result(&raw.body, parse_u64(raw.header("Last-Modified-Version")))
    }

    /// Rename or move a collection (`PATCH <prefix>/collections/<key>`) if
    /// it is still at `version`.
    pub fn update_collection(
        &self,
        key: &str,
        version: u64,
        name: &str,
        parent: Option<&str>,
    ) -> Result<Option<u64>, ZError> {
        validate_key(key)?;
        if let Some(parent) = parent {
            validate_key(parent)?;
        }
        let mut body = collection_body(name, parent);
        if let Some(object) = body.as_object_mut() {
            object.remove("relations");
        }
        let raw = self.send_json(
            WriteMethod::Patch,
            &self.library_url(&format!("/collections/{key}")),
            &serde_json::to_string(&body)?,
            &[("If-Unmodified-Since-Version", version.to_string())],
        )?;
        Ok(parse_u64(raw.header("Last-Modified-Version")))
    }

    /// Delete an item (`DELETE <prefix>/items/<key>`) if it is still at
    /// `version`. (Trashing instead is a `PATCH` with `deleted: 1`, see
    /// [`Self::update_item`].)
    pub fn delete_item(&self, key: &str, version: u64) -> Result<(), ZError> {
        validate_key(key)?;
        self.ensure_online()?;
        let Some(api_key) = &self.key else {
            return Err(ZError::MissingKey);
        };
        let url = self.library_url(&format!("/items/{key}"));
        self.with_retries(|| {
            self.wait_for_backoff();
            let response = self
                .agent
                .delete(&url)
                .header("Zotero-API-Version", API_VERSION)
                .header("Zotero-API-Key", api_key.expose())
                .header("If-Unmodified-Since-Version", version.to_string())
                .call()?;
            let raw = RawResponse::from_http(response)?;
            self.note_backoff(&raw);
            check_status(raw)
        })
        .map(|_| ())
    }

    /// Step 1 of a file upload: ask where to send `file` for the attachment
    /// item `key`. `existing_md5` is the digest of the file the item holds
    /// now (`If-Match`), or `None` for an item without a file
    /// (`If-None-Match: *`).
    pub fn authorize_upload(
        &self,
        key: &str,
        file: &FileDescriptor,
        existing_md5: Option<&str>,
    ) -> Result<UploadAuthorization, ZError> {
        validate_key(key)?;
        let raw = self.send_form(
            &self.library_url(&format!("/items/{key}/file")),
            &file.authorization_body(),
            existing_md5,
        )?;
        parse_upload_authorization(&raw.body)
    }

    /// Step 2 of a file upload: send the bytes to the storage host named by
    /// `target`. No Zotero headers are sent to that host.
    pub fn upload_file(
        &self,
        target: &UploadTarget,
        file: &[u8],
        filename: &str,
    ) -> Result<(), ZError> {
        self.ensure_online()?;
        let boundary = format!("tpe{}", new_write_token());
        let (content_type, body) = target.request_body(file, filename, &boundary);
        self.with_retries(|| {
            let response = self
                .agent
                .post(&target.url)
                .content_type(&content_type)
                .send(body.as_slice())?;
            check_status(RawResponse::from_http(response)?)
        })
        .map(|_| ())
    }

    /// Step 3 of a file upload: tell the API the upload named by
    /// `upload_key` is complete (`204`).
    pub fn register_upload(
        &self,
        key: &str,
        upload_key: &str,
        existing_md5: Option<&str>,
    ) -> Result<(), ZError> {
        validate_key(key)?;
        self.send_form(
            &self.library_url(&format!("/items/{key}/file")),
            &registration_body(upload_key),
            existing_md5,
        )
        .map(|_| ())
    }

    /// Create an `imported_file` attachment under `parent` and upload
    /// `file` for it (all three upload steps). Returns the attachment key.
    pub fn attach_file(
        &self,
        parent: &str,
        file: &[u8],
        filename: &str,
        content_type: &str,
        title: &str,
    ) -> Result<String, ZError> {
        validate_key(parent)?;
        let mtime_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        let descriptor = FileDescriptor::from_bytes(file, filename, mtime_ms);
        let patch = ZItemPatch::imported_file_attachment(
            parent,
            title,
            filename,
            content_type,
            &descriptor.md5,
            mtime_ms,
        );
        let key = self.write_items(&[patch])?.key_at(0)?;
        if let UploadAuthorization::Upload(target) =
            self.authorize_upload(&key, &descriptor, None)?
        {
            self.upload_file(&target, file, filename)?;
            self.register_upload(&key, &target.upload_key, None)?;
        }
        Ok(key)
    }

    /// Create (or, with `key` + `version` properties, update) up to 50
    /// items in one unversioned request guarded by a fresh
    /// `Zotero-Write-Token`. An empty slice makes no request.
    pub fn write_items(&self, items: &[ZItemPatch]) -> Result<WriteResult, ZError> {
        let body = write_items_body(items)?;
        if items.is_empty() {
            return Ok(WriteResult::default());
        }
        let token = new_write_token();
        let raw = self.send_json(
            WriteMethod::Post,
            &self.library_url("/items"),
            &body,
            &[("Zotero-Write-Token", token)],
        )?;
        parse_write_result(&raw.body, parse_u64(raw.header("Last-Modified-Version")))
    }

    /// Partially update one item (`PATCH`) if it is still at `version`.
    /// Returns the new `Last-Modified-Version` when the server sends one;
    /// a changed item yields [`ZError::PreconditionFailed`].
    pub fn update_item(
        &self,
        key: &str,
        version: u64,
        patch: &ZItemPatch,
    ) -> Result<Option<u64>, ZError> {
        validate_key(key)?;
        let body = serde_json::to_string(&patch.to_json())?;
        let raw = self.send_json(
            WriteMethod::Patch,
            &self.library_url(&format!("/items/{key}")),
            &body,
            &[("If-Unmodified-Since-Version", version.to_string())],
        )?;
        Ok(parse_u64(raw.header("Last-Modified-Version")))
    }

    /// Add an HTML child note to `parent`.
    pub fn create_note(&self, parent: &str, html: &str) -> Result<WriteResult, ZError> {
        validate_key(parent)?;
        self.write_items(&[ZItemPatch::note(parent, html)])
    }

    /// Add a `linked_url` attachment (a link, no file upload) to `parent`.
    pub fn add_attachment_link(
        &self,
        parent: &str,
        url: &str,
        title: &str,
    ) -> Result<WriteResult, ZError> {
        validate_key(parent)?;
        self.write_items(&[ZItemPatch::linked_url_attachment(parent, url, title)])
    }

    fn check_same_origin(&self, url: &str) -> Result<(), ZError> {
        let base = &self.base;
        if url.starts_with(&format!("{base}/")) {
            Ok(())
        } else {
            Err(ZError::ForeignLink(url.to_string()))
        }
    }

    fn ensure_online(&self) -> Result<(), ZError> {
        if self.offline {
            Err(ZError::Offline)
        } else {
            Ok(())
        }
    }

    /// Sleep until a previously announced `Backoff` period is over.
    fn wait_for_backoff(&self) {
        let until = self
            .backoff_until
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(until) = until {
            let now = Instant::now();
            if until > now {
                std::thread::sleep(until - now);
            }
        }
    }

    /// Remember a `Backoff` header for the next request.
    fn note_backoff(&self, raw: &RawResponse) {
        if let Some(delay) = backoff_delay(parse_u64(raw.header("Backoff")), MAX_BACKOFF) {
            *self
                .backoff_until
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now() + delay);
        }
    }

    /// Run `attempt` until it succeeds or the retry policy gives up,
    /// sleeping for the computed delay between attempts.
    fn with_retries(
        &self,
        mut attempt: impl FnMut() -> Result<RawResponse, ZError>,
    ) -> Result<RawResponse, ZError> {
        let mut attempts = 0;
        loop {
            attempts += 1;
            match attempt() {
                Ok(raw) => return Ok(raw),
                Err(error) => match retry_delay(&error, attempts, &self.retry) {
                    Some(delay) => std::thread::sleep(delay),
                    None => return Err(error),
                },
            }
        }
    }

    fn get(&self, url: &str) -> Result<RawResponse, ZError> {
        self.ensure_online()?;
        self.with_retries(|| {
            self.wait_for_backoff();
            let mut request = self
                .agent
                .get(url)
                .header("Zotero-API-Version", API_VERSION);
            if let Some(key) = &self.key {
                request = request.header("Zotero-API-Key", key.expose());
            }
            let raw = RawResponse::from_http(request.call()?)?;
            self.note_backoff(&raw);
            check_status(raw)
        })
    }

    fn send_json(
        &self,
        method: WriteMethod,
        url: &str,
        body: &str,
        extra_headers: &[(&str, String)],
    ) -> Result<RawResponse, ZError> {
        self.send_body(method, url, "application/json", body, extra_headers)
    }

    /// `POST` an `application/x-www-form-urlencoded` body with the
    /// `If-None-Match: *` / `If-Match: <md5>` header of the upload flow.
    fn send_form(
        &self,
        url: &str,
        body: &str,
        existing_md5: Option<&str>,
    ) -> Result<RawResponse, ZError> {
        let precondition = match existing_md5 {
            Some(md5) => ("If-Match", md5.to_string()),
            None => ("If-None-Match", "*".to_string()),
        };
        self.send_body(
            WriteMethod::Post,
            url,
            "application/x-www-form-urlencoded",
            body,
            &[precondition],
        )
    }

    fn send_body(
        &self,
        method: WriteMethod,
        url: &str,
        content_type: &str,
        body: &str,
        extra_headers: &[(&str, String)],
    ) -> Result<RawResponse, ZError> {
        self.ensure_online()?;
        let Some(key) = &self.key else {
            return Err(ZError::MissingKey);
        };
        self.with_retries(|| {
            self.wait_for_backoff();
            let mut request = match method {
                WriteMethod::Post => self.agent.post(url),
                WriteMethod::Patch => self.agent.patch(url),
            };
            request = request
                .header("Zotero-API-Version", API_VERSION)
                .header("Zotero-API-Key", key.expose())
                .content_type(content_type);
            for (name, value) in extra_headers {
                request = request.header(*name, value.as_str());
            }
            let raw = RawResponse::from_http(request.send(body)?)?;
            self.note_backoff(&raw);
            check_status(raw)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw(status: u16, headers: &[(&str, &str)], body: &str) -> RawResponse {
        RawResponse {
            status,
            headers: headers
                .iter()
                .map(|(n, v)| ((*n).to_string(), (*v).to_string()))
                .collect(),
            body: body.to_string(),
        }
    }

    const TWO_ITEMS: &str = r#"[
      {"key": "AAAA2222", "version": 5, "data": {"key": "AAAA2222", "version": 5,
        "itemType": "book", "title": "A Book", "publisher": "Press",
        "creators": [{"creatorType": "author", "firstName": "B", "lastName": "Author"}],
        "date": "1999", "tags": [], "collections": [], "relations": {}}},
      {"key": "BBBB3333", "version": 6, "data": {"key": "BBBB3333", "version": 6,
        "itemType": "attachment", "parentItem": "AAAA2222", "linkMode": "imported_file",
        "title": "Full Text PDF", "contentType": "application/pdf", "filename": "a.pdf",
        "tags": [], "relations": {}}}
    ]"#;

    #[test]
    fn items_page_reads_paging_headers() {
        let response = raw(
            200,
            &[
                ("total-results", "5040"),
                ("Last-Modified-Version", "1234"),
                (
                    "Link",
                    "<https://api.zotero.org/users/1/items?limit=2&start=2>; rel=\"next\", <https://api.zotero.org/users/1/items?limit=2&start=5038>; rel=\"last\"",
                ),
                ("Backoff", "30"),
            ],
            TWO_ITEMS,
        );
        let page = parse_items_page(&response).unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.total_results, Some(5040));
        assert_eq!(page.last_modified_version, Some(1234));
        assert_eq!(page.backoff_secs, Some(30));
        assert_eq!(
            page.next.as_deref(),
            Some("https://api.zotero.org/users/1/items?limit=2&start=2")
        );
        let book = page.items[0].to_record();
        assert_eq!(book.venue.as_deref(), Some("Press"));
        assert_eq!(book.authors, vec!["B Author"]);
        assert_eq!(book.year, Some(1999));
        assert_eq!(page.items[1].parent_item(), Some("AAAA2222"));
    }

    #[test]
    fn not_modified_page_is_empty() {
        let page = parse_items_page(&raw(304, &[("Last-Modified-Version", "9")], "")).unwrap();
        assert!(page.items.is_empty());
        assert_eq!(page.last_modified_version, Some(9));
    }

    #[test]
    fn status_mapping() {
        assert!(matches!(
            check_status(raw(403, &[], "Forbidden")),
            Err(ZError::Forbidden)
        ));
        assert!(matches!(
            check_status(raw(412, &[], "")),
            Err(ZError::PreconditionFailed)
        ));
        assert!(matches!(
            check_status(raw(429, &[("Retry-After", "7")], "")),
            Err(ZError::RateLimited {
                retry_after_secs: Some(7)
            })
        ));
        assert!(matches!(
            check_status(raw(500, &[], "boom")),
            Err(ZError::Status { code: 500, .. })
        ));
        assert!(check_status(raw(204, &[], "")).is_ok());
    }

    #[test]
    fn urls_are_built_from_the_query() {
        let client = ZoteroClient::new(Library::Group(42), None).offline(true);
        let query = ItemQuery {
            q: Some("deep learning".to_string()),
            item_type: Some("journalArticle || preprint".to_string()),
            since: Some(10),
            limit: Some(500),
            start: Some(100),
            sort: Some("dateModified".to_string()),
            top: true,
        };
        assert_eq!(
            client.items_url(&query),
            "https://api.zotero.org/groups/42/items/top?q=deep%20learning\
             &itemType=journalArticle%20%7C%7C%20preprint&since=10&limit=100&start=100\
             &sort=dateModified"
        );
        assert_eq!(
            client.attachment_file_url("ABCD2345").unwrap(),
            "https://api.zotero.org/groups/42/items/ABCD2345/file"
        );
        assert!(matches!(
            client.attachment_file_url("../x"),
            Err(ZError::InvalidKey(_))
        ));
        assert_eq!(
            ZoteroClient::new(Library::User(7), None).items_url(&ItemQuery::default()),
            "https://api.zotero.org/users/7/items"
        );
    }

    #[test]
    fn offline_client_never_touches_the_network() {
        let client = ZoteroClient::new(Library::User(1), Some(ApiKey::new("secret"))).offline(true);
        assert!(matches!(
            client.items(&ItemQuery::default()),
            Err(ZError::Offline)
        ));
        assert!(matches!(client.item("ABCD2345"), Err(ZError::Offline)));
        assert!(matches!(client.collections(), Err(ZError::Offline)));
        assert!(matches!(
            client.create_note("ABCD2345", "<p>x</p>"),
            Err(ZError::Offline)
        ));
        assert!(matches!(
            client.update_item("ABCD2345", 3, &ZItemPatch::new()),
            Err(ZError::Offline)
        ));
    }

    #[test]
    fn writes_need_a_key_and_at_most_fifty_items() {
        let client = ZoteroClient::new(Library::User(1), None);
        // No key: refused before any request is built.
        assert!(matches!(
            client.create_note("ABCD2345", "x"),
            Err(ZError::MissingKey)
        ));
        let many = vec![ZItemPatch::new(); 51];
        assert!(matches!(
            write_items_body(&many),
            Err(ZError::TooMany { count: 51 })
        ));
        assert_eq!(write_items_body(&[]).unwrap(), "[]");
        assert!(client.write_items(&[]).unwrap().successful.is_empty());
    }

    #[test]
    fn write_body_is_a_json_array_of_patches() {
        let body = write_items_body(&[
            ZItemPatch::note("ABCD2345", "<p>n</p>"),
            ZItemPatch::linked_url_attachment("ABCD2345", "https://x.org/a.pdf", "A"),
        ])
        .unwrap();
        let value: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value[0]["itemType"], json!("note"));
        assert_eq!(value[1]["linkMode"], json!("linked_url"));
    }

    #[test]
    fn write_result_parses_documented_shapes() {
        // Shape from "Creating Multiple Objects" plus the `success` map.
        let body = r#"{
          "successful": {"0": {"key": "AAAA2222", "version": 10, "data": {}},
                         "2": {"key": "CCCC4444", "version": 10, "data": {}}},
          "success": {"0": "AAAA2222", "2": "CCCC4444"},
          "unchanged": {"4": "EEEE6666"},
          "failed": {"1": {"key": "BBBB3333", "code": 400, "message": "Invalid field"},
                     "3": {"code": 413, "message": "Too large"}}
        }"#;
        let result = parse_write_result(body, Some(10)).unwrap();
        assert_eq!(
            result.successful.get(&0).map(String::as_str),
            Some("AAAA2222")
        );
        assert_eq!(
            result.successful.get(&2).map(String::as_str),
            Some("CCCC4444")
        );
        assert_eq!(
            result.unchanged.get(&4).map(String::as_str),
            Some("EEEE6666")
        );
        assert_eq!(result.failed[&1].code, 400);
        assert_eq!(result.failed[&1].key.as_deref(), Some("BBBB3333"));
        assert_eq!(result.failed[&3].key, None);
        assert_eq!(result.last_modified_version, Some(10));
    }

    #[test]
    fn collections_parse_parent_false_and_key() {
        let body = r#"[
          {"key": "COLL2345", "version": 3, "data": {"key": "COLL2345", "version": 3,
            "name": "Reading", "parentCollection": false, "relations": {}}},
          {"key": "SUBC2345", "version": 4, "data": {"key": "SUBC2345", "version": 4,
            "name": "Sub", "parentCollection": "COLL2345", "relations": {}}}
        ]"#;
        let collections = parse_collections(body).unwrap();
        assert_eq!(collections[0].parent, None);
        assert_eq!(collections[1].parent.as_deref(), Some("COLL2345"));
        assert_eq!(collections[1].name, "Sub");
    }

    #[test]
    fn write_tokens_are_32_hex_and_distinct() {
        let a = new_write_token();
        let b = new_write_token();
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn api_key_debug_is_redacted() {
        let client = ZoteroClient::new(
            Library::User(1),
            Some(ApiKey::new("P9NiFoyLeZu2bZNvvuQPDWsd")),
        );
        let text = format!("{client:?}");
        assert!(text.contains("ApiKey(***)"));
        assert!(!text.contains("P9Ni"));
    }

    #[test]
    fn foreign_next_links_are_refused() {
        let client = ZoteroClient::new(Library::User(1), None);
        let page = Page::<ZItem> {
            items: Vec::new(),
            total_results: None,
            last_modified_version: None,
            next: Some("https://evil.example/users/1/items".to_string()),
            backoff_secs: None,
        };
        assert!(matches!(
            client.next_page(&page),
            Err(ZError::ForeignLink(_))
        ));
    }

    #[test]
    fn more_status_mappings() {
        assert!(matches!(
            check_status(raw(400, &[], "Invalid value")),
            Err(ZError::BadRequest(m)) if m == "Invalid value"
        ));
        assert!(matches!(
            check_status(raw(409, &[], "")),
            Err(ZError::Conflict)
        ));
        assert!(matches!(
            check_status(raw(413, &[], "big")),
            Err(ZError::TooLarge(_))
        ));
        assert!(matches!(
            check_status(raw(428, &[], "")),
            Err(ZError::PreconditionRequired)
        ));
    }

    #[test]
    fn key_at_reports_each_outcome() {
        let body = r#"{"success": {"0": "AAAA2222"}, "unchanged": {"1": "BBBB3333"},
                       "failed": {"2": {"code": 400, "message": "Invalid field"}}}"#;
        let result = parse_write_result(body, None).unwrap();
        assert_eq!(result.key_at(0).unwrap(), "AAAA2222");
        assert_eq!(result.key_at(1).unwrap(), "BBBB3333");
        assert!(matches!(
            result.key_at(2),
            Err(ZError::WriteFailed { code: 400, message }) if message == "Invalid field"
        ));
        assert!(matches!(result.key_at(3), Err(ZError::Parse(_))));
    }

    #[test]
    fn schema_and_key_urls_are_outside_the_library_prefix() {
        let client = ZoteroClient::new(Library::User(7), None);
        assert_eq!(
            client.api_url("/keys/current"),
            "https://api.zotero.org/keys/current"
        );
        assert_eq!(
            build_url(
                &client.api_url("/items/new"),
                &[
                    ("itemType", "attachment".to_string()),
                    ("linkMode", "imported_file".to_string())
                ]
            ),
            "https://api.zotero.org/items/new?itemType=attachment&linkMode=imported_file"
        );
        assert_eq!(
            client.library_url("/items/ABCD2345/file"),
            "https://api.zotero.org/users/7/items/ABCD2345/file"
        );
    }

    #[test]
    fn new_endpoints_respect_offline_and_missing_key() {
        let offline = ZoteroClient::new(Library::User(1), Some(ApiKey::new("k"))).offline(true);
        assert!(matches!(
            offline.all_items(&ItemQuery::default()),
            Err(ZError::Offline)
        ));
        assert!(matches!(offline.tags(), Err(ZError::Offline)));
        assert!(matches!(offline.item_types(), Err(ZError::Offline)));
        assert!(matches!(
            offline.item_template("book", None),
            Err(ZError::Offline)
        ));
        assert!(matches!(offline.key_info(), Err(ZError::Offline)));
        assert!(matches!(offline.groups(1), Err(ZError::Offline)));
        assert!(matches!(
            offline.create_collection("x", None),
            Err(ZError::Offline)
        ));
        assert!(matches!(
            offline.delete_item("ABCD2345", 1),
            Err(ZError::Offline)
        ));
        assert!(matches!(
            offline.attach_file("ABCD2345", b"x", "x.pdf", "application/pdf", "PDF"),
            Err(ZError::Offline)
        ));
        let target = UploadTarget {
            url: "https://storage.example/".to_string(),
            upload_key: "k".to_string(),
            ..UploadTarget::default()
        };
        assert!(matches!(
            offline.upload_file(&target, b"x", "x.pdf"),
            Err(ZError::Offline)
        ));
        let keyless = ZoteroClient::new(Library::User(1), None);
        assert!(matches!(keyless.key_info(), Err(ZError::MissingKey)));
        assert!(matches!(
            keyless.delete_item("ABCD2345", 1),
            Err(ZError::MissingKey)
        ));
        assert!(matches!(
            keyless.register_upload("ABCD2345", "k", None),
            Err(ZError::MissingKey)
        ));
        assert!(matches!(
            keyless.update_collection("bad key", 1, "n", None),
            Err(ZError::InvalidKey(_))
        ));
        assert!(matches!(
            keyless.create_collection("n", Some("../x")),
            Err(ZError::InvalidKey(_))
        ));
    }

    #[test]
    fn backoff_is_remembered_for_the_next_request() {
        let client = ZoteroClient::new(Library::User(1), None);
        client.note_backoff(&raw(200, &[("Backoff", "0")], "[]"));
        assert!(client.backoff_until.lock().unwrap().is_none());
        client.note_backoff(&raw(200, &[("Backoff", "30")], "[]"));
        let until = client.backoff_until.lock().unwrap().unwrap();
        let remaining = until.saturating_duration_since(Instant::now());
        assert!(remaining <= Duration::from_secs(30));
        assert!(remaining > Duration::from_secs(25));
        // Waiting takes the pending pause, so a zero pause is instant.
        *client.backoff_until.lock().unwrap() = Some(Instant::now());
        client.wait_for_backoff();
        assert!(client.backoff_until.lock().unwrap().is_none());
    }

    #[test]
    fn retries_give_up_after_the_policy() {
        let client = ZoteroClient::new(Library::User(1), None).with_retry_policy(RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
        });
        let mut calls = 0;
        let result = client.with_retries(|| {
            calls += 1;
            Err(ZError::Status {
                code: 503,
                message: String::new(),
            })
        });
        assert!(matches!(result, Err(ZError::Status { code: 503, .. })));
        assert_eq!(calls, 3);
        let mut calls = 0;
        let result = client.with_retries(|| {
            calls += 1;
            Err(ZError::Forbidden)
        });
        assert!(matches!(result, Err(ZError::Forbidden)));
        assert_eq!(calls, 1);
        let mut calls = 0;
        let ok = client.with_retries(|| {
            calls += 1;
            if calls < 2 {
                Err(ZError::Transport("reset".to_string()))
            } else {
                Ok(raw(204, &[], ""))
            }
        });
        assert_eq!(ok.unwrap().status, 204);
        assert_eq!(calls, 2);
    }
}
