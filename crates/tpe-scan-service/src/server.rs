//! The loopback HTTP service: listener, per-connection handling, the
//! request checks (`Host`, `Origin`, bearer token), CORS for the web app,
//! concurrency and size limits, and graceful shutdown.

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::capabilities;
use crate::http::{Request, Response, body_length, read_body, read_head};
use crate::image_pdf;
use crate::models::{self, MODELS_DIR_ENV, ModelReport, PdfiumReport};
use crate::multipart;
use crate::scan::{ErrorCode, Mode, ScanError, ScanOptions, parse_pages};
use crate::worker::{WorkerLimits, run_in_worker_cancellable};

/// Service configuration. `Config::new` holds the defaults.
#[derive(Clone, Debug)]
pub struct Config {
    /// Must be a loopback address; anything else is refused at start.
    pub bind: IpAddr,
    /// `0` picks a free port.
    pub port: u16,
    /// The per-launch bearer token (at least [`crate::auth::MIN_TOKEN_CHARS`] characters).
    pub token: String,
    /// Web-app origins allowed to call the service from a browser
    /// (`scheme://host[:port]`, normalised by [`normalize_origin`]).
    pub origins: Vec<String>,
    pub max_body_bytes: u64,
    pub max_pages: u32,
    pub max_concurrent: usize,
    pub scan_timeout: Duration,
    /// Absolute, non-renewable head and upload deadlines.
    pub header_timeout: Duration,
    pub body_timeout: Duration,
    pub worker_memory_growth_mib: u64,
    /// Explicit models directory (exported to the worker as `DOCLING_RS_MODELS_DIR`).
    pub models_dir: Option<PathBuf>,
    /// Hash the model files at start (about 82 MB to read when present).
    pub hash_models: bool,
    /// The binary to run scans in (normally this executable).
    pub worker_exe: PathBuf,
}

impl Config {
    /// Defaults: `127.0.0.1:0`, 64 MiB bodies, 50 pages, one scan at a time,
    /// 120 s per scan, 4 GiB of worker address-space growth.
    pub fn new(token: String) -> Self {
        Self {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            token,
            origins: Vec::new(),
            max_body_bytes: 64 * 1024 * 1024,
            max_pages: 50,
            max_concurrent: 1,
            scan_timeout: Duration::from_secs(120),
            header_timeout: HEADER_TIMEOUT,
            body_timeout: BODY_TIMEOUT,
            worker_memory_growth_mib: 4096,
            models_dir: None,
            hash_models: true,
            worker_exe: std::env::current_exe()
                .unwrap_or_else(|_| PathBuf::from("tpe-scan-service")),
        }
    }
}

/// Normalise an origin to `scheme://host[:port]` in lower case; `None` when
/// the value is not a plain origin.
pub fn normalize_origin(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches('/');
    let (scheme, rest) = value.split_once("://")?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return None;
    }
    if rest.is_empty() || rest.contains(['/', '?', '#', '@', ' ']) {
        return None;
    }
    Some(format!(
        "{}://{}",
        scheme.to_ascii_lowercase(),
        rest.to_ascii_lowercase()
    ))
}

/// Whether a `Host` header names this service's loopback address.
pub fn host_allowed(host: &str, port: u16) -> bool {
    let host = host.trim().to_ascii_lowercase();
    let names = ["127.0.0.1", "localhost", "[::1]"];
    names
        .iter()
        .any(|name| host == format!("{name}:{port}") || (port == 80 && host == *name))
}

/// A counting gate for concurrent scans.
#[derive(Debug, Default)]
pub struct Gate {
    in_flight: AtomicUsize,
}

/// A held slot; released on drop.
pub struct Slot<'a>(&'a Gate);

impl Gate {
    /// Take a slot when fewer than `limit` are held.
    pub fn try_acquire(&self, limit: usize) -> Option<Slot<'_>> {
        let mut current = self.in_flight.load(Ordering::SeqCst);
        loop {
            if current >= limit {
                return None;
            }
            match self.in_flight.compare_exchange(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return Some(Slot(self)),
                Err(actual) => current = actual,
            }
        }
    }

    /// Slots currently held.
    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::SeqCst)
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Shared {
    config: Config,
    port: u16,
    models: Mutex<ModelReport>,
    gate: Gate,
    shutdown: AtomicBool,
    handlers: AtomicUsize,
    jobs: Mutex<Jobs>,
}

/// A running service. Dropping it shuts it down.
pub struct Service {
    addr: SocketAddr,
    shared: Arc<Shared>,
    /// Shared with the accept thread; the last handle to go closes the port.
    listener: Option<Arc<TcpListener>>,
    accept_thread: Option<JoinHandle<()>>,
}

const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const BODY_TIMEOUT: Duration = Duration::from_secs(60);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

impl Service {
    /// Bind and start serving. Fails for a non-loopback address or a short token.
    pub fn start(config: Config) -> std::io::Result<Self> {
        if !config.bind.is_loopback() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "refusing to bind {}: the scan service only listens on a loopback address",
                    config.bind
                ),
            ));
        }
        if config.token.chars().count() < crate::auth::MIN_TOKEN_CHARS {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "the bearer token is too short",
            ));
        }
        let listener = Arc::new(TcpListener::bind(SocketAddr::new(
            config.bind,
            config.port,
        ))?);
        let addr = listener.local_addr()?;
        let dirs = models::search_dirs(config.models_dir.as_deref());
        let models = models::probe(&dirs, config.hash_models);
        let shared = Arc::new(Shared {
            port: addr.port(),
            models: Mutex::new(models),
            gate: Gate::default(),
            shutdown: AtomicBool::new(false),
            handlers: AtomicUsize::new(0),
            jobs: Mutex::new(Jobs::default()),
            config,
        });
        let accept_shared = Arc::clone(&shared);
        let accept_listener = Arc::clone(&listener);
        let accept_thread = std::thread::Builder::new()
            .name("tpe-scan-accept".to_string())
            .spawn(move || accept_loop(accept_listener, &accept_shared))?;
        Ok(Self {
            addr,
            shared,
            listener: Some(listener),
            accept_thread: Some(accept_thread),
        })
    }

    /// The bound loopback address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The current models report (re-probed cheaply).
    pub fn models(&self) -> ModelReport {
        refresh_models(&self.shared)
    }

    /// Stop accepting, close the listening socket, and join the handlers.
    /// Returns only once the port no longer accepts connections.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        if let Some(listener) = self.listener.take() {
            // From now on an accept returns `WouldBlock` instead of blocking,
            // so the loop notices the flag even if the wake-up below is lost.
            let _ = listener.set_nonblocking(true);
            // Release this handle first: the accept thread's handle is then
            // the last one, and it closes the socket the moment its loop ends.
            drop(listener);
        }
        // Wake the blocking accept with a throwaway connection.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_secs(1));
        // Joining guarantees the socket is closed before this returns. That
        // matters on macOS, where the kernel keeps completing handshakes into
        // the listen backlog for as long as the socket exists, so a flag alone
        // (or an unjoined drop on another thread) leaves the port observably
        // open for a while after "shutdown".
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service")
            .field("addr", &self.addr)
            .field("in_flight", &self.shared.gate.in_flight())
            .finish_non_exhaustive()
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        if self.accept_thread.is_some() {
            self.stop();
        }
    }
}

fn accept_loop(listener: Arc<TcpListener>, shared: &Arc<Shared>) {
    let mut handlers: Vec<JoinHandle<()>> = Vec::new();
    let max_handlers = shared.config.max_concurrent * 2 + 8;
    loop {
        if shared.shutdown.load(Ordering::SeqCst) {
            break;
        }
        let Ok((stream, _)) = listener.accept() else {
            // `WouldBlock` once shutdown made the socket non-blocking, or a
            // transient accept failure: re-check the flag, do not spin.
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        handlers.retain(|handle| !handle.is_finished());
        if shared.handlers.load(Ordering::SeqCst) >= max_handlers {
            let mut stream = stream;
            let _ = Response::error(503, "busy", "too many open connections; retry shortly")
                .header("Retry-After", "2")
                .write_to_before(&mut stream, Instant::now() + Duration::from_secs(2), || {
                    shared.shutdown.load(Ordering::SeqCst)
                });
            continue;
        }
        let accepted = Instant::now();
        shared.handlers.fetch_add(1, Ordering::SeqCst);
        let handler_shared = Arc::clone(shared);
        let spawned = std::thread::Builder::new()
            .name("tpe-scan-conn".to_string())
            .spawn(move || {
                handle_connection(stream, &handler_shared, accepted);
                handler_shared.handlers.fetch_sub(1, Ordering::SeqCst);
            });
        match spawned {
            Ok(handle) => handlers.push(handle),
            Err(_) => {
                shared.handlers.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }
    // Close the listening socket before waiting for open connections to
    // finish, so the port refuses new ones as soon as accepting stops.
    drop(listener);
    for handle in handlers {
        let _ = handle.join();
    }
}

// Cancellation can overtake a scan's headers on a separate HTTP connection.
// Fixed-count, short-lived tombstones prevent that late scan from starting.
const MAX_CANCEL_TOMBSTONES: usize = 64;
#[derive(Default)]
struct Jobs {
    active: HashMap<String, Arc<Job>>,
    cancelled: VecDeque<(String, Instant)>,
}
impl Jobs {
    fn expire(&mut self) {
        let now = Instant::now();
        self.cancelled.retain(|(_, expiry)| *expiry > now);
    }
}

#[derive(Default)]
struct Job {
    cancel: AtomicBool,
    finished: AtomicBool,
}

// Own the slot through upload, worker execution and pipe cleanup. Cancellation
// acknowledgement is published only after the slot is free.
struct Admission<'a> {
    shared: &'a Shared,
    slot: Option<Slot<'a>>,
    id: Option<String>,
    job: Arc<Job>,
}
impl Drop for Admission<'_> {
    fn drop(&mut self) {
        drop(self.slot.take());
        if let Some(id) = &self.id {
            self.shared
                .jobs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .active
                .remove(id);
        }
        self.job.finished.store(true, Ordering::SeqCst);
    }
}

fn busy() -> Response {
    scan_error(&ScanError::new(
        ErrorCode::Busy,
        "scan capacity is occupied; retry shortly",
    ))
}

fn handle_connection(mut stream: TcpStream, shared: &Arc<Shared>, accepted: Instant) {
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let _ = stream.set_nodelay(true);
    let response = match read_head(&mut stream, accepted + shared.config.header_timeout, || {
        shared.shutdown.load(Ordering::SeqCst)
    }) {
        Ok(mut request) => {
            let origin = allowed_origin(&request, &shared.config.origins);
            let response = handle_authenticated(&mut stream, &mut request, shared, accepted);
            with_cors(response, &request, origin.as_deref())
        }
        Err(error) => Response::error(error.status(), error.code(), &error.to_string()),
    };
    let _ = response.write_to_before(&mut stream, Instant::now() + WRITE_TIMEOUT, || {
        shared.shutdown.load(Ordering::SeqCst)
    });
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn handle_authenticated(
    stream: &mut TcpStream,
    request: &mut Request,
    shared: &Arc<Shared>,
    accepted: Instant,
) -> Response {
    if let Some(response) = authenticate(request, shared) {
        return response;
    }
    let length = match body_length(request, shared.config.max_body_bytes) {
        Ok(length) => length,
        Err(error) => return Response::error(error.status(), error.code(), &error.to_string()),
    };
    if request.method != "POST" || request.path != "/scan" {
        // Only scans accept bodies; never wait for irrelevant untrusted bytes.
        if length != 0 {
            return Response::error(400, "unexpected_body", "this route accepts no body");
        }
        return route(request, shared);
    }
    let Some(slot) = shared.gate.try_acquire(shared.config.max_concurrent) else {
        return busy();
    };
    let id = request.query_param("id");
    if id.as_ref().is_some_and(|id| !valid_job_id(id)) {
        return Response::error(
            400,
            "bad_id",
            "scan id must be 16-64 ASCII letters, digits or hyphens",
        );
    }
    let job = Arc::new(Job::default());
    if let Some(id) = &id {
        let mut jobs = shared
            .jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        jobs.expire();
        if jobs.cancelled.iter().any(|(cancelled, _)| cancelled == id) {
            return scan_error(&ScanError::new(
                ErrorCode::Timeout,
                "scan cancelled before admission",
            ));
        }
        if jobs.active.contains_key(id) {
            return Response::error(409, "duplicate_id", "scan id is already active");
        }
        jobs.active.insert(id.clone(), Arc::clone(&job));
    }
    let admission = Admission {
        shared,
        slot: Some(slot),
        id,
        job,
    };
    if let Err(error) = read_body(
        stream,
        request,
        length,
        accepted + shared.config.header_timeout + shared.config.body_timeout,
        || shared.shutdown.load(Ordering::SeqCst) || admission.job.cancel.load(Ordering::SeqCst),
    ) {
        return Response::error(error.status(), error.code(), &error.to_string());
    }
    if let Err(error) = stream.set_nonblocking(true) {
        return Response::error(500, "internal", &error.to_string());
    }
    let response = scan_response(request, shared, || {
        if shared.shutdown.load(Ordering::SeqCst) || admission.job.cancel.load(Ordering::SeqCst) {
            return true;
        }
        let mut probe = [0_u8; 1];
        match stream.peek(&mut probe) {
            Ok(_) => true, // EOF or extra data: one request per connection.
            Err(error) => !matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ),
        }
    });
    let _ = stream.set_nonblocking(false);
    response
}

fn valid_job_id(id: &str) -> bool {
    (16..=64).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn cancel_response(request: &Request, shared: &Shared) -> Response {
    let Some(id) = request.query_param("id").filter(|id| valid_job_id(id)) else {
        return Response::error(400, "bad_id", "a valid scan id is required");
    };
    let job = {
        let mut jobs = shared
            .jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        jobs.expire();
        // Preserve the tombstone even if a completed ID is seen again.
        if !jobs.cancelled.iter().any(|(cancelled, _)| cancelled == &id) {
            if jobs.cancelled.len() >= MAX_CANCEL_TOMBSTONES && !jobs.active.contains_key(&id) {
                return Response::error(
                    429,
                    "cancel_pending",
                    "cancellation tracking is full; retry shortly",
                );
            }
            if jobs.cancelled.len() < MAX_CANCEL_TOMBSTONES {
                jobs.cancelled.push_back((
                    id.clone(),
                    Instant::now() + shared.config.header_timeout + Duration::from_secs(1),
                ));
            }
        }
        jobs.active.get(&id).cloned()
    };
    let Some(job) = job else {
        return Response::empty(204);
    };
    job.cancel.store(true, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !job.finished.load(Ordering::SeqCst) {
        if Instant::now() >= deadline {
            return Response::error(503, "cancel_pending", "worker cleanup is still pending");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Response::empty(204)
}

/// The request's `Origin` when it is one of the configured origins.
fn allowed_origin(request: &Request, origins: &[String]) -> Option<String> {
    let origin = request.header("origin")?;
    let normalised = normalize_origin(origin)?;
    origins.contains(&normalised).then(|| origin.to_string())
}

fn with_cors(response: Response, request: &Request, origin: Option<&str>) -> Response {
    let Some(origin) = origin else {
        return response;
    };
    let mut response = response
        .header("Access-Control-Allow-Origin", origin)
        .header("Vary", "Origin")
        .header("Access-Control-Expose-Headers", "Retry-After");
    if request.method == "OPTIONS" {
        response = response
            .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
            .header(
                "Access-Control-Allow-Headers",
                "Authorization, Content-Type",
            )
            .header("Access-Control-Max-Age", "600");
        if request
            .header("access-control-request-private-network")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        {
            response = response.header("Access-Control-Allow-Private-Network", "true");
        }
    }
    response
}

fn authenticate(request: &Request, shared: &Shared) -> Option<Response> {
    let config = &shared.config;
    match request.header("host") {
        Some(host) if host_allowed(host, shared.port) => {}
        _ => {
            return Some(Response::error(
                403,
                "host_not_allowed",
                "the Host header must name this service's loopback address",
            ));
        }
    }
    if let Some(origin) = request.header("origin") {
        let allowed = normalize_origin(origin).is_some_and(|value| config.origins.contains(&value));
        if !allowed {
            return Some(Response::error(
                403,
                "origin_not_allowed",
                "this browser origin is not allowed; start the service with --origin <web app origin>",
            ));
        }
    }
    if request.method == "OPTIONS" {
        return Some(if request.header("origin").is_some() {
            Response::empty(204)
        } else {
            Response::error(403, "origin_required", "preflight without an Origin header")
        });
    }
    let presented = request
        .header("authorization")
        .and_then(crate::auth::bearer)
        .unwrap_or_default();
    if !crate::auth::token_matches(&config.token, presented) {
        return Some(
            Response::error(
                401,
                "unauthorized",
                "a valid bearer token is required (printed once when the service started)",
            )
            .header("WWW-Authenticate", "Bearer realm=\"tpe-scan-service\""),
        );
    }
    None
}

fn route(request: &Request, shared: &Arc<Shared>) -> Response {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/capabilities") => capabilities_response(shared),
        ("POST", "/cancel") => cancel_response(request, shared),
        ("GET" | "HEAD", "/scan") | ("POST", "/capabilities") => Response::error(
            405,
            "method_not_allowed",
            "method not allowed for this path",
        )
        .header(
            "Allow",
            if request.path == "/scan" {
                "POST, OPTIONS"
            } else {
                "GET, OPTIONS"
            },
        ),
        _ => Response::error(404, "not_found", "unknown path; use /capabilities or /scan"),
    }
}

fn refresh_models(shared: &Shared) -> ModelReport {
    let mut guard = shared
        .models
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let refreshed = models::refresh(&guard, shared.config.hash_models);
    *guard = refreshed.clone();
    refreshed
}

fn pdfium_report() -> PdfiumReport {
    models::pdfium()
}

fn capabilities_response(shared: &Arc<Shared>) -> Response {
    let models = refresh_models(shared);
    let document = capabilities::document(&shared.config, &models, &pdfium_report());
    Response::json(200, &document)
}

fn scan_error(error: &ScanError) -> Response {
    let response = Response::error(
        error.code.http_status(),
        &serde_json::to_value(error.code)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "internal".to_string()),
        &error.message,
    );
    if error.code == ErrorCode::Busy {
        response.header("Retry-After", "5")
    } else {
        response
    }
}

fn scan_response(
    request: &Request,
    shared: &Arc<Shared>,
    cancel: impl FnMut() -> bool,
) -> Response {
    let config = &shared.config;
    let mode = match request.query_param("mode") {
        None => Mode::Ocr,
        Some(value) => match Mode::parse(&value) {
            Some(mode) => mode,
            None => return Response::error(400, "bad_mode", "mode must be `ocr` or `text`"),
        },
    };
    let pages = match request.query_param("pages") {
        None => None,
        Some(value) => match parse_pages(&value) {
            Ok(range) => Some(range),
            Err(message) => return Response::error(400, "bad_pages", &message),
        },
    };
    let (bytes, declared) = match request.header("content-type").and_then(multipart::boundary) {
        Some(boundary) => match multipart::parse(&request.body, &boundary) {
            Ok(parts) => match multipart::file_part(parts) {
                Some(part) => (part.body, part.content_type),
                None => {
                    return Response::error(400, "no_file", "multipart body carries no file part");
                }
            },
            Err(message) => return Response::error(400, "bad_multipart", &message),
        },
        None => (
            request.body.clone(),
            request.header("content-type").map(str::to_string),
        ),
    };
    if bytes.is_empty() {
        return Response::error(
            400,
            "empty_body",
            "send the PDF, PNG or JPEG bytes as the body",
        );
    }
    if image_pdf::sniff(&bytes).is_none() {
        return Response::error(
            415,
            "unsupported_media_type",
            &format!(
                "the bytes are not a PDF, PNG or JPEG{}",
                declared
                    .map(|value| format!(" (declared {value})"))
                    .unwrap_or_default()
            ),
        );
    }
    let options = ScanOptions {
        mode,
        pages,
        max_pages: config.max_pages,
    };
    let limits = WorkerLimits {
        timeout: config.scan_timeout,
        memory_growth_mib: config.worker_memory_growth_mib,
        max_bytes: config.max_body_bytes,
    };
    let mut env = Vec::new();
    if let Some(dir) = &config.models_dir {
        env.push((MODELS_DIR_ENV.to_string(), dir.display().to_string()));
    }
    match run_in_worker_cancellable(&config.worker_exe, bytes, &options, limits, &env, cancel) {
        Ok(result) => Response::json(200, &serde_json::to_value(&result).unwrap_or_default()),
        Err(error) => scan_error(&error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_normalise_to_scheme_host_port() {
        assert_eq!(
            normalize_origin("HTTPS://App.Example.org/"),
            Some("https://app.example.org".to_string())
        );
        assert_eq!(
            normalize_origin("http://localhost:5209"),
            Some("http://localhost:5209".to_string())
        );
        assert_eq!(normalize_origin("null"), None);
        assert_eq!(normalize_origin("file://x"), None);
        assert_eq!(normalize_origin("http://a/b"), None);
        assert_eq!(normalize_origin("http://"), None);
    }

    #[test]
    fn host_must_be_loopback_with_the_bound_port() {
        assert!(host_allowed("127.0.0.1:5000", 5000));
        assert!(host_allowed("LocalHost:5000", 5000));
        assert!(host_allowed("[::1]:5000", 5000));
        assert!(!host_allowed("127.0.0.1:5001", 5000));
        assert!(!host_allowed("127.0.0.1", 5000));
        assert!(host_allowed("127.0.0.1", 80));
        assert!(!host_allowed("evil.example:5000", 5000));
        assert!(!host_allowed("127.0.0.1.evil.example:5000", 5000));
    }

    #[test]
    fn gate_counts_slots_and_releases_on_drop() {
        let gate = Gate::default();
        let first = gate.try_acquire(1).unwrap();
        assert!(gate.try_acquire(1).is_none());
        assert_eq!(gate.in_flight(), 1);
        drop(first);
        assert_eq!(gate.in_flight(), 0);
        let _a = gate.try_acquire(2).unwrap();
        let _b = gate.try_acquire(2).unwrap();
        assert!(gate.try_acquire(2).is_none());
        assert!(gate.try_acquire(0).is_none());
    }

    #[test]
    fn non_loopback_binds_and_short_tokens_are_refused() {
        let mut config = Config::new("0123456789abcdef0123456789abcdef".to_string());
        config.bind = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
        let error = Service::start(config).unwrap_err();
        assert!(error.to_string().contains("loopback"), "{error}");
        let short = Config::new("short".to_string());
        assert!(Service::start(short).is_err());
    }
}
