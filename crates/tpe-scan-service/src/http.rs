//! A deliberately small HTTP/1.1 request reader and response writer over a
//! `TcpStream`: one request per connection, `Content-Length` bodies only,
//! bounded header and body sizes, read timeouts. Enough for a loopback
//! service that a browser `fetch` talks to; nothing more.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// Largest request head (request line plus headers) accepted.
pub const MAX_HEADER_BYTES: usize = 16 * 1024;
/// Most headers accepted on one request.
pub const MAX_HEADERS: usize = 64;

/// One parsed request. Header names are lower-cased.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    /// Path without the query string.
    pub path: String,
    /// Query string without the leading `?` (empty when absent).
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// First value of header `name` (any case), trimmed.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.trim())
    }

    /// Percent-decoded value of query parameter `key` (first occurrence).
    pub fn query_param(&self, key: &str) -> Option<String> {
        self.query
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| pair.split_once('=').unwrap_or((pair, "")))
            .find(|(name, _)| percent_decode(name) == key)
            .map(|(_, value)| percent_decode(value))
    }
}

/// Decode `%XX` escapes and `+` as space; invalid escapes are kept verbatim.
pub fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => out.push(b' '),
            b'%' if index + 3 <= bytes.len() => {
                let pair = std::str::from_utf8(&bytes[index + 1..index + 3])
                    .ok()
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok());
                if let Some(byte) = pair {
                    out.push(byte);
                    index += 3;
                    continue;
                }
                out.push(b'%');
            }
            byte => out.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Why a request could not be read; each maps to an HTTP status.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("request deadline exceeded")]
    Deadline,
    #[error("request headers exceed {MAX_HEADER_BYTES} bytes")]
    HeadersTooLarge,
    #[error("malformed request: {0}")]
    Malformed(String),
    #[error("request body of {length} bytes exceeds the limit of {limit} bytes")]
    BodyTooLarge { length: u64, limit: u64 },
    #[error("chunked or unknown transfer encodings are not accepted; send Content-Length")]
    LengthRequired,
    #[error("the connection closed before the request was complete")]
    Truncated,
    #[error("i/o while reading the request: {0}")]
    Io(#[from] io::Error),
}

impl ReadError {
    /// The HTTP status that reports this error.
    pub fn status(&self) -> u16 {
        match self {
            Self::Deadline => 408,
            Self::HeadersTooLarge => 431,
            Self::Malformed(_) | Self::Truncated | Self::Io(_) => 400,
            Self::BodyTooLarge { .. } => 413,
            Self::LengthRequired => 411,
        }
    }

    /// A short machine-readable code for the JSON error body.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Deadline => "request_timeout",
            Self::HeadersTooLarge => "headers_too_large",
            Self::Malformed(_) => "malformed_request",
            Self::Truncated | Self::Io(_) => "incomplete_request",
            Self::BodyTooLarge { .. } => "body_too_large",
            Self::LengthRequired => "length_required",
        }
    }
}

/// Read only a bounded head and at most one fixed-size chunk of body prefix.
/// Authentication and admission must happen before `read_body`.
pub fn read_head(
    stream: &mut TcpStream,
    deadline: Instant,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Request, ReadError> {
    let mut buffer = Vec::with_capacity(4096);
    let mut chunk = [0_u8; 4096];
    loop {
        let read = read_before(stream, &mut chunk, deadline, &mut cancelled)?;
        if read == 0 {
            return Err(ReadError::Truncated);
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some((mut request, consumed)) = parse_head(&buffer)? {
            if consumed > MAX_HEADER_BYTES {
                return Err(ReadError::HeadersTooLarge);
            }
            request.body = buffer.split_off(consumed);
            return Ok(request);
        }
        if buffer.len() >= MAX_HEADER_BYTES {
            return Err(ReadError::HeadersTooLarge);
        }
    }
}

/// Length validation is intentionally separate from header authentication.
pub fn body_length(request: &Request, max_body: u64) -> Result<usize, ReadError> {
    if request
        .header("transfer-encoding")
        .is_some_and(|value| !value.is_empty())
    {
        return Err(ReadError::LengthRequired);
    }
    let length: u64 = match request.header("content-length") {
        None => 0,
        Some(value) => value
            .parse()
            .map_err(|_| ReadError::Malformed("invalid Content-Length".into()))?,
    };
    if length > max_body {
        return Err(ReadError::BodyTooLarge {
            length,
            limit: max_body,
        });
    }
    usize::try_from(length).map_err(|_| ReadError::Malformed("Content-Length too large".into()))
}

/// Grow storage only for bytes actually received, after authentication and
/// admission. Recompute remaining time before every read; progress never
/// renews the absolute deadline. Short polling also permits prompt shutdown.
pub fn read_body(
    stream: &mut TcpStream,
    request: &mut Request,
    length: usize,
    deadline: Instant,
    mut cancelled: impl FnMut() -> bool,
) -> Result<(), ReadError> {
    if request.body.len() > length {
        return Err(ReadError::Malformed("bytes after the declared body".into()));
    }
    if request.body.len() < length
        && request
            .header("expect")
            .is_some_and(|value| value.eq_ignore_ascii_case("100-continue"))
    {
        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
    }
    let mut chunk = [0_u8; 8192];
    while request.body.len() < length {
        let remaining = (length - request.body.len()).min(chunk.len());
        let read = read_before(stream, &mut chunk[..remaining], deadline, &mut cancelled)?;
        if read == 0 {
            return Err(ReadError::Truncated);
        }
        request.body.extend_from_slice(&chunk[..read]);
    }
    if cancelled() {
        return Err(ReadError::Truncated);
    }
    Ok(())
}

fn read_before(
    stream: &mut TcpStream,
    buffer: &mut [u8],
    deadline: Instant,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<usize, ReadError> {
    loop {
        if cancelled() {
            return Err(ReadError::Truncated);
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(ReadError::Deadline)?;
        stream.set_read_timeout(Some(remaining.min(Duration::from_millis(50))))?;
        match stream.read(buffer) {
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            result => return result.map_err(ReadError::Io),
        }
    }
}

/// Parse the request head from `buffer`; `None` while it is incomplete.
pub fn parse_head(buffer: &[u8]) -> Result<Option<(Request, usize)>, ReadError> {
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut parsed = httparse::Request::new(&mut headers);
    let consumed = match parsed.parse(buffer) {
        Ok(httparse::Status::Complete(consumed)) => consumed,
        Ok(httparse::Status::Partial) => return Ok(None),
        Err(httparse::Error::TooManyHeaders) => return Err(ReadError::HeadersTooLarge),
        Err(error) => return Err(ReadError::Malformed(error.to_string())),
    };
    let method = parsed
        .method
        .ok_or_else(|| ReadError::Malformed("missing method".to_string()))?
        .to_string();
    let target = parsed
        .path
        .ok_or_else(|| ReadError::Malformed("missing request target".to_string()))?;
    if parsed.version != Some(1) {
        return Err(ReadError::Malformed("HTTP/1.1 required".to_string()));
    }
    let (path, query) = target
        .split_once('?')
        .map_or((target, ""), |(path, query)| (path, query));
    let mut collected = Vec::with_capacity(parsed.headers.len());
    for header in parsed.headers.iter() {
        let value = std::str::from_utf8(header.value)
            .map_err(|_| ReadError::Malformed(format!("header {} is not UTF-8", header.name)))?;
        let name = header.name.to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "host" | "origin" | "authorization" | "content-length" | "transfer-encoding"
        ) && collected.iter().any(|(key, _)| key == &name)
        {
            return Err(ReadError::Malformed(format!("duplicate {name} header")));
        }
        collected.push((name, value.to_string()));
    }
    Ok(Some((
        Request {
            method,
            path: path.to_string(),
            query: query.to_string(),
            headers: collected,
            body: Vec::new(),
        },
        consumed,
    )))
}

/// A response to write back; always `Connection: close`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    /// A response with `body` and no extra headers.
    pub fn new(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body,
        }
    }

    /// An empty response.
    pub fn empty(status: u16) -> Self {
        Self::new(status, Vec::new())
    }

    /// A JSON response.
    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        Self::new(status, serde_json::to_vec(value).unwrap_or_default())
            .header("Content-Type", "application/json")
    }

    /// The service's JSON error shape: `{"error":{"code","message"}}`.
    pub fn error(status: u16, code: &str, message: &str) -> Self {
        Self::json(
            status,
            &serde_json::json!({"error": {"code": code, "message": message}}),
        )
    }

    /// Add a header.
    #[must_use]
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// Serialise the response (status line, headers, body).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 {} {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Length: {}\r\n",
            self.status,
            reason(self.status),
            self.body.len()
        )
        .into_bytes();
        for (name, value) in &self.headers {
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(b": ");
            out.extend_from_slice(value.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(&self.body);
        out
    }

    /// Write the response to `stream`.
    pub fn write_to(&self, stream: &mut impl Write) -> io::Result<()> {
        stream.write_all(&self.to_bytes())?;
        stream.flush()
    }
}

/// Reason phrase for the statuses this service uses.
pub fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_parses_method_path_query_and_lowercases_headers() {
        let raw = b"POST /scan?pages=1-3&mode=text HTTP/1.1\r\nHost: 127.0.0.1:9\r\nContent-Length: 2\r\n\r\nok";
        let (request, consumed) = parse_head(raw).unwrap().unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/scan");
        assert_eq!(request.query, "pages=1-3&mode=text");
        assert_eq!(request.header("HOST"), Some("127.0.0.1:9"));
        assert_eq!(request.query_param("pages").as_deref(), Some("1-3"));
        assert_eq!(request.query_param("mode").as_deref(), Some("text"));
        assert_eq!(request.query_param("missing"), None);
        assert_eq!(&raw[consumed..], b"ok");
    }

    #[test]
    fn partial_head_waits_and_bad_head_fails() {
        assert!(
            parse_head(b"GET /capabilities HTTP/1.1\r\nHost: x")
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            parse_head(b"GET /x HTTP/1.0\r\n\r\n"),
            Err(ReadError::Malformed(_))
        ));
        assert!(matches!(
            parse_head(b"\x00\x01 garbage\r\n\r\n"),
            Err(ReadError::Malformed(_))
        ));
    }

    #[test]
    fn percent_decoding_handles_escapes_plus_and_junk() {
        assert_eq!(percent_decode("a%20b+c"), "a b c");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("caf%C3%A9"), "café");
    }

    #[test]
    fn responses_carry_length_close_and_nosniff() {
        let text = String::from_utf8(Response::error(404, "not_found", "no").to_bytes()).unwrap();
        assert!(text.starts_with("HTTP/1.1 404 Not Found\r\n"));
        assert!(text.contains("Connection: close\r\n"));
        assert!(text.contains("X-Content-Type-Options: nosniff\r\n"));
        assert!(text.contains("Content-Type: application/json\r\n"));
        assert!(text.ends_with("{\"error\":{\"code\":\"not_found\",\"message\":\"no\"}}"));
        assert_eq!(ReadError::LengthRequired.status(), 411);
        assert_eq!(ReadError::HeadersTooLarge.status(), 431);
    }
}
