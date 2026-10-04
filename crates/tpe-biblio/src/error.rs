//! Error type shared by every client and parser in this crate.

use thiserror::Error;

/// Errors from the bibliographic clients and parsers.
///
/// Messages never contain the request URL, because some URLs carry API keys.
#[derive(Debug, Error)]
pub enum BiblioError {
    /// The client is in offline mode; no request was made.
    #[error("offline mode: network requests are disabled")]
    Offline,
    /// HTTP transport was excluded from this build; runtime settings cannot enable it.
    #[error("network capability is disabled in this build")]
    NetworkDisabled,
    /// The server answered 404.
    #[error("not found")]
    NotFound,
    /// The server answered 429 (rate limited).
    #[error("rate limited by the server (HTTP 429)")]
    RateLimited,
    /// Any other non-success HTTP status.
    #[error("HTTP status {0}")]
    Status(u16),
    /// A transport-level failure (DNS, TLS, timeout, ...), sanitised.
    #[error("transport error: {0}")]
    Transport(String),
    /// The response body was not valid JSON.
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// A required setting (such as the contact e-mail Unpaywall needs) is missing.
    #[error("missing configuration: {0}")]
    MissingConfig(&'static str),
    /// The JSON was valid but did not have the expected shape.
    #[error("unexpected response shape: {0}")]
    Shape(String),
}

impl BiblioError {
    /// Map a `ureq` error to a sanitised `BiblioError` (no URL in the message).
    #[cfg(feature = "network")]
    pub fn from_ureq(err: &ureq::Error) -> Self {
        match err {
            ureq::Error::StatusCode(404) => Self::NotFound,
            ureq::Error::StatusCode(429) => Self::RateLimited,
            ureq::Error::StatusCode(code) => Self::Status(*code),
            ureq::Error::Timeout(_) => Self::Transport("timeout".to_string()),
            ureq::Error::HostNotFound => Self::Transport("host not found".to_string()),
            ureq::Error::ConnectionFailed => Self::Transport("connection failed".to_string()),
            ureq::Error::Io(io) => Self::Transport(format!("io: {}", io.kind())),
            ureq::Error::BodyExceedsLimit(limit) => {
                Self::Transport(format!("response body exceeds {limit} bytes"))
            }
            ureq::Error::TooManyRedirects => Self::Transport("too many redirects".to_string()),
            ureq::Error::BadUri(_) => Self::Transport("invalid request URI".to_string()),
            // Classify failures without formatting their messages or source chains:
            // TLS/proxy diagnostics can carry URLs, credentials or peer-supplied data.
            ureq::Error::Tls(_) | ureq::Error::Rustls(_) => {
                Self::Transport("TLS handshake or certificate validation failed".to_string())
            }
            ureq::Error::Pem(_) => Self::Transport("invalid TLS certificate data".to_string()),
            ureq::Error::TlsRequired => Self::Transport("TLS transport unavailable".to_string()),
            ureq::Error::ConnectProxyFailed(_) => {
                Self::Transport("CONNECT proxy negotiation failed".to_string())
            }
            ureq::Error::InvalidProxyUrl => {
                Self::Transport("invalid proxy configuration".to_string())
            }
            ureq::Error::Protocol(_) => Self::Transport("HTTP protocol error".to_string()),
            ureq::Error::Http(_) => Self::Transport("HTTP request error".to_string()),
            ureq::Error::RequireHttpsOnly(_) => {
                Self::Transport("request requires HTTPS".to_string())
            }
            _ => Self::Transport("request failed".to_string()),
        }
    }
}

#[cfg(all(test, feature = "network"))]
mod tests {
    use super::*;

    #[test]
    fn status_errors_never_echo_the_uri() {
        let secret_uri = "https://example.org/x?api_key=SECRET123".to_string();
        let err = BiblioError::from_ureq(&ureq::Error::BadUri(secret_uri));
        assert!(!err.to_string().contains("SECRET123"));
        let err = BiblioError::from_ureq(&ureq::Error::StatusCode(503));
        assert_eq!(err.to_string(), "HTTP status 503");
        assert!(matches!(
            BiblioError::from_ureq(&ureq::Error::StatusCode(404)),
            BiblioError::NotFound
        ));
    }

    #[test]
    fn transport_categories_drop_untrusted_error_payloads() {
        const SECRET: &str = "https://USER:PASSWORD@example.org/?api_key=SECRET123";
        let errors = [
            (
                ureq::Error::Tls(SECRET),
                "TLS handshake or certificate validation failed",
            ),
            (
                ureq::Error::ConnectProxyFailed(SECRET.to_string()),
                "CONNECT proxy negotiation failed",
            ),
            (ureq::Error::InvalidProxyUrl, "invalid proxy configuration"),
            (ureq::Error::TlsRequired, "TLS transport unavailable"),
            (
                ureq::Error::RequireHttpsOnly(SECRET.to_string()),
                "request requires HTTPS",
            ),
            (
                ureq::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    SECRET,
                )),
                "io: permission denied",
            ),
            (
                ureq::Error::Other(Box::new(std::io::Error::other(SECRET))),
                "request failed",
            ),
        ];
        for (error, expected) in errors {
            let mapped = BiblioError::from_ureq(&error);
            assert!(matches!(&mapped, BiblioError::Transport(detail) if detail == expected));
            assert_eq!(mapped.to_string(), format!("transport error: {expected}"));
            let debug = format!("{mapped:?}");
            for marker in ["example.org", "USER", "PASSWORD", "SECRET123"] {
                assert!(!mapped.to_string().contains(marker));
                assert!(!debug.contains(marker));
            }
        }
    }

    #[test]
    fn malformed_certificate_and_http_request_keep_only_categories() {
        let pem = b"-----BEGIN CERTIFICATE-----\nSECRET123!?\n-----END CERTIFICATE-----\n";
        let error = ureq::tls::Certificate::from_pem(pem).unwrap_err();
        assert!(matches!(&error, ureq::Error::Pem(_)));
        assert_eq!(
            BiblioError::from_ureq(&error).to_string(),
            "transport error: invalid TLS certificate data"
        );

        let error = ureq::http::Request::builder()
            .header("x-secret", "SECRET123\r\n")
            .body(())
            .unwrap_err();
        assert_eq!(
            BiblioError::from_ureq(&ureq::Error::Http(error)).to_string(),
            "transport error: HTTP request error"
        );
    }
}
