//! The service over real loopback sockets: token, `Host`/`Origin` checks,
//! CORS, limits, routing, scan failures without models, the worker boundary
//! and shutdown. Nothing here needs the docling models or `PDFium`; the
//! feature-gated scans use the engine's one-page probe PDF.

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use tpe_scan_service::multipart::{self, Part};
use tpe_scan_service::scan::{ErrorCode, Mode, ScanOptions};
use tpe_scan_service::server::{Config, Service};
use tpe_scan_service::worker::{WorkerLimits, run_in_worker};

/// A fixed test fixture value, not a real credential.
const TOKEN: &str = "test-fixture-token-0123456789abcdef";
const ORIGIN: &str = "http://localhost:5209";

fn config() -> Config {
    let mut config = Config::new(TOKEN.to_string());
    config.origins = vec![ORIGIN.to_string()];
    config.hash_models = false;
    config.worker_exe = PathBuf::from(env!("CARGO_BIN_EXE_tpe-scan-service"));
    config.scan_timeout = Duration::from_secs(90);
    config.max_body_bytes = 2 * 1024 * 1024;
    config
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|error| {
            panic!(
                "body is not JSON ({error}): {}",
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    fn error_code(&self) -> String {
        self.json()["error"]["code"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }
}

fn raw(addr: SocketAddr, request: &[u8]) -> Reply {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(120)))
        .unwrap();
    stream.write_all(request).unwrap();
    let mut buffer = Vec::new();
    // The server closes after one response; a reset after that is fine.
    let _ = stream.read_to_end(&mut buffer);
    let split = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap_or_else(|| panic!("no response head in {:?}", String::from_utf8_lossy(&buffer)));
    let head = String::from_utf8(buffer[..split].to_vec()).unwrap();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap();
    let status: u16 = status_line.split(' ').nth(1).unwrap().parse().unwrap();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .collect();
    Reply {
        status,
        headers,
        body: buffer[split + 4..].to_vec(),
    }
}

fn request(
    addr: SocketAddr,
    method: &str,
    target: &str,
    extra: &[(&str, &str)],
    body: &[u8],
) -> Reply {
    let mut head = format!(
        "{method} {target} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: {}\r\n",
        addr.port(),
        body.len()
    );
    if !extra
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case("authorization"))
    {
        let _ = write!(head, "Authorization: Bearer {TOKEN}\r\n");
    }
    for (key, value) in extra {
        let _ = write!(head, "{key}: {value}\r\n");
    }
    head.push_str("\r\n");
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body);
    raw(addr, &bytes)
}

fn probe_pdf() -> Vec<u8> {
    tpe::backend::probe_pdf().unwrap()
}

#[test]
fn capabilities_need_the_token_and_describe_the_build() {
    let service = Service::start(config()).unwrap();
    let addr = service.addr();
    let anonymous = raw(
        addr,
        format!(
            "GET /capabilities HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
            addr.port()
        )
        .as_bytes(),
    );
    assert_eq!(anonymous.status, 401);
    assert!(
        anonymous
            .header("WWW-Authenticate")
            .unwrap()
            .contains("Bearer")
    );
    let wrong = request(
        addr,
        "GET",
        "/capabilities",
        &[(
            "Authorization",
            "Bearer test-fixture-token-0123456789abcdee",
        )],
        b"",
    );
    assert_eq!(wrong.status, 401);
    let reply = request(addr, "GET", "/capabilities", &[], b"");
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    assert_eq!(reply.header("Content-Type"), Some("application/json"));
    assert_eq!(reply.header("Cache-Control"), Some("no-store"));
    let value = reply.json();
    assert_eq!(value["service"]["name"], "tpe-scan-service");
    assert_eq!(value["service"]["loopback_only"], true);
    assert_eq!(value["build"]["ocr_compiled"], cfg!(feature = "docling"));
    assert_eq!(
        value["build"]["text_layer_compiled"],
        cfg!(feature = "docling-text")
    );
    assert_eq!(value["limits"]["max_pages"], 50);
    assert_eq!(value["limits"]["max_body_bytes"], 2 * 1024 * 1024);
    assert_eq!(value["origins"][0], ORIGIN);
    assert_eq!(value["models"]["files"].as_array().unwrap().len(), 4);
    service.shutdown();
}

#[test]
fn host_and_origin_are_checked_and_cors_is_scoped_to_the_web_app() {
    let service = Service::start(config()).unwrap();
    let addr = service.addr();
    let bad_host = raw(
        addr,
        format!(
            "GET /capabilities HTTP/1.1\r\nHost: evil.example:{}\r\nAuthorization: Bearer {TOKEN}\r\n\r\n",
            addr.port()
        )
        .as_bytes(),
    );
    assert_eq!(bad_host.status, 403);
    assert_eq!(bad_host.error_code(), "host_not_allowed");
    let bad_origin = request(
        addr,
        "GET",
        "/capabilities",
        &[("Origin", "https://evil.example")],
        b"",
    );
    assert_eq!(bad_origin.status, 403);
    assert_eq!(bad_origin.error_code(), "origin_not_allowed");
    assert!(bad_origin.header("Access-Control-Allow-Origin").is_none());
    let null_origin = request(addr, "GET", "/capabilities", &[("Origin", "null")], b"");
    assert_eq!(null_origin.status, 403);
    let allowed = request(addr, "GET", "/capabilities", &[("Origin", ORIGIN)], b"");
    assert_eq!(allowed.status, 200);
    assert_eq!(allowed.header("Access-Control-Allow-Origin"), Some(ORIGIN));
    assert_eq!(allowed.header("Vary"), Some("Origin"));
    // Preflight: no token yet, origin required, private-network opt-in echoed.
    let preflight = raw(
        addr,
        format!(
            "OPTIONS /scan HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: {ORIGIN}\r\nAccess-Control-Request-Method: POST\r\nAccess-Control-Request-Private-Network: true\r\n\r\n",
            addr.port()
        )
        .as_bytes(),
    );
    assert_eq!(preflight.status, 204);
    assert_eq!(
        preflight.header("Access-Control-Allow-Methods"),
        Some("GET, POST, OPTIONS")
    );
    assert_eq!(
        preflight.header("Access-Control-Allow-Headers"),
        Some("Authorization, Content-Type")
    );
    assert_eq!(
        preflight.header("Access-Control-Allow-Private-Network"),
        Some("true")
    );
    let blind_preflight = raw(
        addr,
        format!(
            "OPTIONS /scan HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
            addr.port()
        )
        .as_bytes(),
    );
    assert_eq!(blind_preflight.status, 403);
    service.shutdown();
}

#[test]
fn limits_and_routing_answer_precisely() {
    let service = Service::start(config()).unwrap();
    let addr = service.addr();
    let too_large = raw(
        addr,
        format!(
            "POST /scan HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: {}\r\n\r\n",
            addr.port(),
            3 * 1024 * 1024
        )
        .as_bytes(),
    );
    assert_eq!(too_large.status, 413);
    assert_eq!(too_large.error_code(), "body_too_large");
    let chunked = raw(
        addr,
        format!(
            "POST /scan HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {TOKEN}\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
            addr.port()
        )
        .as_bytes(),
    );
    assert_eq!(chunked.status, 411);
    let huge_head = raw(
        addr,
        format!(
            "GET /capabilities HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nX-Pad: {}\r\n\r\n",
            addr.port(),
            "a".repeat(20_000)
        )
        .as_bytes(),
    );
    assert_eq!(huge_head.status, 431);
    assert_eq!(request(addr, "GET", "/scan", &[], b"").status, 405);
    assert_eq!(request(addr, "GET", "/nope", &[], b"").status, 404);
    assert_eq!(request(addr, "POST", "/capabilities", &[], b"").status, 405);
    let bad_mode = request(addr, "POST", "/scan?mode=bogus", &[], &probe_pdf());
    assert_eq!(
        (bad_mode.status, bad_mode.error_code()),
        (400, "bad_mode".to_string())
    );
    let bad_pages = request(addr, "POST", "/scan?pages=9-2", &[], &probe_pdf());
    assert_eq!(
        (bad_pages.status, bad_pages.error_code()),
        (400, "bad_pages".to_string())
    );
    let empty = request(addr, "POST", "/scan", &[], b"");
    assert_eq!(
        (empty.status, empty.error_code()),
        (400, "empty_body".to_string())
    );
    let text = request(
        addr,
        "POST",
        "/scan",
        &[("Content-Type", "text/plain")],
        b"hello",
    );
    assert_eq!(text.status, 415);
    assert!(
        text.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("declared text/plain")
    );
    let no_file = request(
        addr,
        "POST",
        "/scan",
        &[("Content-Type", "multipart/form-data; boundary=b")],
        b"--b--\r\n",
    );
    assert_eq!(
        (no_file.status, no_file.error_code()),
        (400, "no_file".to_string())
    );
    service.shutdown();
}

/// What `POST /scan` (OCR mode) must say on this machine: a precise 503 when
/// the backend is not compiled in or the models are absent.
fn expect_ocr_outcome(reply: &Reply) {
    if !cfg!(feature = "docling") {
        assert_eq!(
            reply.status,
            503,
            "{}",
            String::from_utf8_lossy(&reply.body)
        );
        assert_eq!(reply.error_code(), "not_compiled");
        assert!(
            reply.json()["error"]["message"]
                .as_str()
                .unwrap()
                .contains("--features docling"),
            "{}",
            String::from_utf8_lossy(&reply.body)
        );
        return;
    }
    let models =
        tpe_scan_service::models::probe(&tpe_scan_service::models::search_dirs(None), false);
    let pdfium = tpe_scan_service::models::pdfium();
    if tpe_scan_service::models::provisioning_problem(&models, &pdfium).is_some() {
        assert_eq!(
            reply.status,
            503,
            "{}",
            String::from_utf8_lossy(&reply.body)
        );
        assert_eq!(reply.error_code(), "models_not_provisioned");
        let message = reply.json()["error"]["message"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(message.contains("native/fetch.sh"), "{message}");
        assert!(
            message.contains("models not provisioned") || message.contains("PDFium"),
            "{message}"
        );
    } else {
        eprintln!("models provisioned here: asserting a real OCR result");
        assert_eq!(
            reply.status,
            200,
            "{}",
            String::from_utf8_lossy(&reply.body)
        );
        assert_eq!(reply.json()["backend"]["name"], "docling");
    }
}

#[test]
fn ocr_scan_without_models_is_a_precise_503_for_raw_and_multipart_uploads() {
    let service = Service::start(config()).unwrap();
    let addr = service.addr();
    let raw_reply = request(
        addr,
        "POST",
        "/scan",
        &[("Content-Type", "application/pdf"), ("Origin", ORIGIN)],
        &probe_pdf(),
    );
    expect_ocr_outcome(&raw_reply);
    assert_eq!(
        raw_reply.header("Access-Control-Allow-Origin"),
        Some(ORIGIN)
    );
    let body = multipart::encode(
        "fixture-boundary",
        &[Part {
            name: Some("file".to_string()),
            filename: Some("scan.pdf".to_string()),
            content_type: Some("application/pdf".to_string()),
            body: probe_pdf(),
        }],
    );
    let multipart_reply = request(
        addr,
        "POST",
        "/scan",
        &[(
            "Content-Type",
            "multipart/form-data; boundary=fixture-boundary",
        )],
        &body,
    );
    expect_ocr_outcome(&multipart_reply);
    service.shutdown();
}

#[cfg(feature = "docling-text")]
#[test]
fn text_mode_reads_a_pdf_and_wraps_a_png_through_the_worker() {
    let service = Service::start(config()).unwrap();
    let addr = service.addr();
    let reply = request(
        addr,
        "POST",
        "/scan?mode=text&pages=1-1",
        &[("Content-Type", "application/pdf")],
        &probe_pdf(),
    );
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let value = reply.json();
    assert_eq!(value["backend"]["name"], "docling-text");
    assert_eq!(value["mode"], "text");
    assert_eq!(value["input"], "pdf");
    assert_eq!(value["pages_total"], 1);
    assert!(
        value["pages"][0]["text"]
            .as_str()
            .unwrap()
            .contains("probe"),
        "{value}"
    );
    assert!(!value["pages"][0]["blocks"].as_array().unwrap().is_empty());
    assert!(value["pages"][0]["confidence"].is_null());

    let mut png = std::io::Cursor::new(Vec::new());
    image::GrayImage::from_pixel(40, 30, image::Luma([250]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let reply = request(
        addr,
        "POST",
        "/scan?mode=text",
        &[("Content-Type", "image/png")],
        &png.into_inner(),
    );
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let value = reply.json();
    assert_eq!(value["input"], "png");
    assert_eq!(value["pages"][0]["text"], "");
    assert!(
        value["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("40x30"))
    );
    let out_of_range = request(
        addr,
        "POST",
        "/scan?mode=text&pages=5",
        &[("Content-Type", "application/pdf")],
        &probe_pdf(),
    );
    assert_eq!(out_of_range.status, 400);
    assert_eq!(out_of_range.error_code(), "page_range");
    service.shutdown();
}

#[test]
fn worker_boundary_reports_or_scans_and_a_deadline_kills_the_child() {
    let options = ScanOptions {
        mode: Mode::Text,
        pages: None,
        max_pages: 10,
    };
    let limits = WorkerLimits {
        timeout: Duration::from_secs(90),
        memory_growth_mib: 2048,
        max_bytes: 1024 * 1024,
    };
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_tpe-scan-service"));
    let cancel = AtomicBool::new(false);
    let outcome = run_in_worker(&exe, probe_pdf(), &options, limits, &[], &cancel);
    if cfg!(feature = "docling-text") {
        let result = outcome.unwrap();
        assert!(result.pages[0].text.contains("probe"));
    } else {
        let error = outcome.unwrap_err();
        assert_eq!(error.code, ErrorCode::NotCompiled);
    }
    let too_big = run_in_worker(
        &exe,
        vec![b'x'; 2 * 1024 * 1024],
        &options,
        limits,
        &[],
        &cancel,
    );
    let error = too_big.unwrap_err();
    assert!(
        matches!(error.code, ErrorCode::Limit | ErrorCode::NotCompiled),
        "{error}"
    );
    let killed = run_in_worker(
        &exe,
        probe_pdf(),
        &options,
        WorkerLimits {
            timeout: Duration::ZERO,
            ..limits
        },
        &[],
        &cancel,
    );
    assert_eq!(killed.unwrap_err().code, ErrorCode::Timeout);
    let cancelled = AtomicBool::new(true);
    let stopped = run_in_worker(&exe, probe_pdf(), &options, limits, &[], &cancelled);
    assert_eq!(stopped.unwrap_err().code, ErrorCode::Timeout);
    let missing = run_in_worker(
        &PathBuf::from("/nonexistent/tpe-scan-service"),
        probe_pdf(),
        &options,
        limits,
        &[],
        &cancel,
    );
    assert_eq!(missing.unwrap_err().code, ErrorCode::Internal);
}

#[test]
fn shutdown_stops_the_listener() {
    let service = Service::start(config()).unwrap();
    let addr = service.addr();
    assert_eq!(request(addr, "GET", "/capabilities", &[], b"").status, 200);
    // `shutdown` joins the accept thread and closes the listening socket
    // before it returns, so the port must refuse connections from here on.
    // macOS keeps completing handshakes into the backlog until the socket is
    // closed, so probe with a short bounded retry rather than one attempt.
    service.shutdown();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
            Err(_) => break,
            Ok(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(_) => panic!("the port still accepts connections 5 s after shutdown returned"),
        }
    }
}

// Incomplete bodies are intentional: admission must answer from the head alone.
fn head_reply(addr: SocketAddr, head: &str) -> Reply {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream.write_all(head.as_bytes()).unwrap();
    reply_from_stream(&mut stream)
}

fn reply_from_stream(stream: &mut TcpStream) -> Reply {
    let mut buffer = Vec::new();
    let _ = stream.read_to_end(&mut buffer);
    let split = buffer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or_else(|| panic!("no prompt response: {:?}", String::from_utf8_lossy(&buffer)));
    let head = String::from_utf8_lossy(&buffer[..split]);
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    Reply {
        status,
        headers: lines
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
            .collect(),
        body: buffer[split + 4..].to_vec(),
    }
}

#[test]
fn unauthenticated_incomplete_or_oversized_bodies_are_refused_before_continue() {
    let service = Service::start(config()).unwrap();
    let addr = service.addr();
    for length in [1024, 1_u64 << 40] {
        for (host, extra, expected) in [
            ("127.0.0.1", "", 401),
            ("evil.example", "", 403),
            ("127.0.0.1", "Origin: https://evil.example\r\n", 403),
        ] {
            let reply = head_reply(
                addr,
                &format!(
                    "POST /scan HTTP/1.1\r\nHost: {host}:{}\r\n{extra}Content-Length: {length}\r\nExpect: 100-continue\r\n\r\n",
                    addr.port()
                ),
            );
            assert_eq!(reply.status, expected);
        }
    }
    // Authenticated GETs and preflights do not consume uploads either.
    let reply = head_reply(
        addr,
        &format!(
            "GET /capabilities HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: 1024\r\n\r\n",
            addr.port()
        ),
    );
    assert_eq!(reply.status, 400);
    service.shutdown();
}

#[test]
fn trickling_headers_and_bodies_cannot_renew_request_deadlines() {
    let mut cfg = config();
    cfg.header_timeout = Duration::from_millis(200);
    cfg.body_timeout = Duration::from_millis(200);
    let service = Service::start(cfg).unwrap();
    for body in [false, true] {
        let mut stream = TcpStream::connect(service.addr()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        if body {
            write!(stream, "POST /scan HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: 100\r\n\r\n", service.addr().port()).unwrap();
        } else {
            stream.write_all(b"POST /scan HTTP/1.1\r\nX-Pad: ").unwrap();
        }
        let mut sender = stream.try_clone().unwrap();
        let started = std::time::Instant::now();
        let trickle = std::thread::spawn(move || {
            for _ in 0..40 {
                if sender.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
        });
        let reply = reply_from_stream(&mut stream);
        assert_eq!(reply.status, 408);
        assert_eq!(reply.error_code(), "request_timeout");
        assert!(started.elapsed() < Duration::from_secs(1));
        trickle.join().unwrap();
    }
    service.shutdown();
}

#[test]
fn scan_admission_precedes_image_decode_and_upload_storage() {
    let service = Service::start(config()).unwrap();
    let addr = service.addr();
    let mut first = TcpStream::connect(addr).unwrap();
    first
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(first, "POST /scan HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: 100\r\nExpect: 100-continue\r\n\r\n", addr.port()).unwrap();
    let mut interim = [0_u8; 25];
    first.read_exact(&mut interim).unwrap();
    assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
    let threads: Vec<_> = (0..6).map(|_| std::thread::spawn(move || {
        let reply = request(addr, "POST", "/scan", &[("Content-Type", "image/png")], b"\x89PNG\r\n\x1a\nbroken");
        assert_eq!(reply.status, 429); // decoding would instead return 400
        let reply = head_reply(addr, &format!("POST /scan HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: 1048576\r\n\r\n", addr.port()));
        assert_eq!(reply.status, 429); // no body sent or reserved
    })).collect();
    for thread in threads {
        thread.join().unwrap();
    }
    drop(first);
    service.shutdown();
}

#[cfg(unix)]
fn fixture_worker(directory: &std::path::Path, delay: bool, flood: bool) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = directory.join("worker");
    // Each child writes its own PID marker. Fixtures only, no real input or token.
    let text = format!(
        "#!/bin/sh\necho $$ > '{}/started-'$$\ncat >/dev/null\n{}{}printf '%s' '{{\"err\":{{\"code\":\"malformed\",\"message\":\"fixture outcome\"}}}}'\n",
        directory.display(),
        if flood {
            "dd if=/dev/zero bs=65536 count=512 1>&2 2>/dev/null\nprintf 'FLOOD-END' >&2\n"
        } else {
            ""
        },
        if delay { "sleep 30\n" } else { "" }
    );
    std::fs::write(&script, text).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    script
}

#[cfg(unix)]
fn wait_for_workers(directory: &std::path::Path, count: usize) -> Vec<u32> {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let pids: Vec<_> = std::fs::read_dir(directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("started-"))
            .map(|entry| {
                std::fs::read_to_string(entry.path())
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap()
            })
            .collect();
        if pids.len() == count {
            return pids;
        }
        assert!(std::time::Instant::now() < deadline, "worker did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn worker_alive(pid: u32) -> bool {
    std::process::Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

#[cfg(unix)]
fn start_scan(addr: SocketAddr, id: &str) -> TcpStream {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let bytes = b"%PDF-1.4\nfixture";
    write!(stream, "POST /scan?id={id} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: {}\r\n\r\n", addr.port(), bytes.len()).unwrap();
    stream.write_all(bytes).unwrap();
    stream
}

#[cfg(unix)]
#[test]
fn disconnect_kills_worker_and_releases_capacity_for_retry() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config();
    cfg.worker_exe = fixture_worker(dir.path(), true, false);
    let service = Service::start(cfg).unwrap();
    let stream = start_scan(service.addr(), "disconnect-fixture-01");
    let pid = wait_for_workers(dir.path(), 1)[0];
    drop(stream);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while worker_alive(pid) {
        assert!(
            std::time::Instant::now() < deadline,
            "disconnected worker is alive"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // Admission may be released just after wait reaps the process.
    loop {
        let reply = request(service.addr(), "POST", "/scan", &[], b"junk");
        if reply.status == 415 {
            break;
        }
        assert_eq!(reply.status, 429);
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    service.shutdown();
}

#[cfg(unix)]
#[test]
fn authenticated_cancel_reaps_only_corresponding_worker_before_acknowledging() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config();
    cfg.max_concurrent = 2;
    cfg.worker_exe = fixture_worker(dir.path(), true, false);
    let service = Service::start(cfg).unwrap();
    let mut first = start_scan(service.addr(), "cancel-fixture-first");
    let first_pid = wait_for_workers(dir.path(), 1)[0];
    let mut second = start_scan(service.addr(), "cancel-fixture-second");
    let all = wait_for_workers(dir.path(), 2);
    let second_pid = *all.iter().find(|&&pid| pid != first_pid).unwrap();
    let unauthorized = request(
        service.addr(),
        "POST",
        "/cancel?id=cancel-fixture-first",
        &[("Authorization", "Bearer wrong-fixture-token")],
        b"",
    );
    assert_eq!(unauthorized.status, 401);
    assert!(worker_alive(first_pid));
    let cancelled = request(
        service.addr(),
        "POST",
        "/cancel?id=cancel-fixture-first",
        &[("Origin", ORIGIN)],
        b"",
    );
    assert_eq!(cancelled.status, 204);
    assert_eq!(
        cancelled.header("Access-Control-Allow-Origin"),
        Some(ORIGIN)
    );
    assert!(!worker_alive(first_pid));
    assert!(worker_alive(second_pid));
    assert_eq!(reply_from_stream(&mut first).error_code(), "timeout");
    // Slot is available immediately after the cancellation acknowledgement.
    assert_eq!(
        request(service.addr(), "POST", "/scan", &[], b"junk").status,
        415
    );
    assert_eq!(
        request(
            service.addr(),
            "POST",
            "/cancel?id=cancel-fixture-second",
            &[],
            b""
        )
        .status,
        204
    );
    assert!(!worker_alive(second_pid));
    assert_eq!(reply_from_stream(&mut second).error_code(), "timeout");
    service.shutdown();
}

#[cfg(unix)]
#[test]
fn stderr_flood_is_drained_and_cancelled_without_blocking_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config();
    cfg.worker_exe = fixture_worker(dir.path(), true, true);
    let service = Service::start(cfg).unwrap();
    let stream = start_scan(service.addr(), "stderr-flood-fixture");
    let pid = wait_for_workers(dir.path(), 1)[0];
    std::thread::sleep(Duration::from_millis(100));
    let reply = request(
        service.addr(),
        "POST",
        "/cancel?id=stderr-flood-fixture",
        &[],
        b"",
    );
    assert_eq!(reply.status, 204);
    assert!(!worker_alive(pid));
    drop(stream);
    service.shutdown();
}

#[test]
fn cancellation_before_admission_prevents_late_worker_start() {
    let service = Service::start(config()).unwrap();
    let addr = service.addr();
    // This models cancellation arriving on a separate browser connection first.
    assert_eq!(
        request(addr, "POST", "/cancel?id=cancel-overtakes-scan", &[], b"").status,
        204
    );
    let reply = head_reply(
        addr,
        &format!(
            "POST /scan?id=cancel-overtakes-scan HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: 1024\r\nExpect: 100-continue\r\n\r\n",
            addr.port()
        ),
    );
    assert_eq!(reply.status, 504);
    assert_eq!(reply.error_code(), "timeout");
    assert_eq!(request(addr, "POST", "/scan", &[], b"junk").status, 415);
    service.shutdown();
}

#[test]
fn cancelling_an_incomplete_upload_releases_admission_before_acknowledgement() {
    let service = Service::start(config()).unwrap();
    let addr = service.addr();
    let mut upload = TcpStream::connect(addr).unwrap();
    upload
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(upload, "POST /scan?id=cancel-during-upload HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: 1048576\r\nExpect: 100-continue\r\n\r\n", addr.port()).unwrap();
    let mut interim = [0_u8; 25];
    upload.read_exact(&mut interim).unwrap();
    assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
    assert_eq!(
        request(addr, "POST", "/cancel?id=cancel-during-upload", &[], b"").status,
        204
    );
    assert_eq!(request(addr, "POST", "/scan", &[], b"junk").status, 415);
    drop(upload);
    service.shutdown();
}

#[test]
fn service_shutdown_interrupts_incomplete_request_reads() {
    let service = Service::start(config()).unwrap();
    let mut upload = TcpStream::connect(service.addr()).unwrap();
    upload.write_all(b"POST /scan HTTP/1.1\r\n").unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let started = std::time::Instant::now();
    service.shutdown();
    assert!(started.elapsed() < Duration::from_secs(1));
}
