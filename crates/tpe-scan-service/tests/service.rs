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
    assert!(value["pages"][0]["blocks"].as_array().unwrap().len() >= 1);
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
    service.shutdown();
    let refused = TcpStream::connect_timeout(&addr, Duration::from_secs(2));
    assert!(refused.is_err(), "the port still accepts connections");
}
