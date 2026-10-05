//! Minimal `multipart/form-data` reader (RFC 7578) for the one file part a
//! browser `FormData` upload carries. The whole body is already bounded by
//! the request-size limit before it gets here.

use std::fmt::Write;

/// One part of a multipart body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Part {
    pub name: Option<String>,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// The `boundary` parameter of a `multipart/form-data` content type.
pub fn boundary(content_type: &str) -> Option<String> {
    let mut parts = content_type.split(';');
    let media = parts.next()?.trim();
    if !media.eq_ignore_ascii_case("multipart/form-data") {
        return None;
    }
    parts
        .map(str::trim)
        .find_map(|parameter| {
            let (key, value) = parameter.split_once('=')?;
            key.trim()
                .eq_ignore_ascii_case("boundary")
                .then(|| value.trim().trim_matches('"').to_string())
        })
        .filter(|value| !value.is_empty() && value.len() <= 70)
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| offset + from)
}

/// Split `body` into its parts.
pub fn parse(body: &[u8], boundary: &str) -> Result<Vec<Part>, String> {
    let delimiter = format!("--{boundary}").into_bytes();
    let mut parts = Vec::new();
    let mut cursor = find(body, &delimiter, 0).ok_or("multipart body has no boundary")?;
    loop {
        cursor += delimiter.len();
        if body[cursor..].starts_with(b"--") {
            return Ok(parts);
        }
        // Tolerate transport padding after the delimiter; require the CRLF.
        while body
            .get(cursor)
            .is_some_and(|byte| *byte == b' ' || *byte == b'\t')
        {
            cursor += 1;
        }
        if !body[cursor..].starts_with(b"\r\n") {
            return Err("multipart boundary not followed by CRLF".to_string());
        }
        cursor += 2;
        let head_end = find(body, b"\r\n\r\n", cursor).ok_or("multipart part has no header end")?;
        let head = std::str::from_utf8(&body[cursor..head_end])
            .map_err(|_| "multipart part headers are not UTF-8".to_string())?;
        let mut part = Part::default();
        for line in head.split("\r\n") {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            if name.trim().eq_ignore_ascii_case("content-disposition") {
                for parameter in value.split(';').map(str::trim) {
                    if let Some((key, raw)) = parameter.split_once('=') {
                        let raw = raw.trim().trim_matches('"').to_string();
                        match key.trim().to_ascii_lowercase().as_str() {
                            "name" => part.name = Some(raw),
                            "filename" => part.filename = Some(raw),
                            _ => {}
                        }
                    }
                }
            } else if name.trim().eq_ignore_ascii_case("content-type") {
                part.content_type = Some(value.to_string());
            }
        }
        let body_start = head_end + 4;
        let mut closing = b"\r\n".to_vec();
        closing.extend_from_slice(&delimiter);
        let body_end =
            find(body, &closing, body_start).ok_or("multipart part is not terminated")?;
        part.body = body[body_start..body_end].to_vec();
        parts.push(part);
        cursor = body_end + 2;
    }
}

/// The uploaded file among `parts`: the part named `file`, else the first
/// part with a filename, else the first part.
pub fn file_part(parts: Vec<Part>) -> Option<Part> {
    let by_name = parts
        .iter()
        .position(|part| part.name.as_deref() == Some("file"));
    let by_filename = parts.iter().position(|part| part.filename.is_some());
    let index = by_name.or(by_filename).unwrap_or(0);
    parts.into_iter().nth(index)
}

/// Build a multipart body (used by tests and documentation examples).
pub fn encode(boundary: &str, parts: &[Part]) -> Vec<u8> {
    let mut out = Vec::new();
    for part in parts {
        out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        let mut disposition = String::from("Content-Disposition: form-data");
        if let Some(name) = &part.name {
            let _ = write!(disposition, "; name=\"{name}\"");
        }
        if let Some(filename) = &part.filename {
            let _ = write!(disposition, "; filename=\"{filename}\"");
        }
        out.extend_from_slice(disposition.as_bytes());
        out.extend_from_slice(b"\r\n");
        if let Some(content_type) = &part.content_type {
            out.extend_from_slice(format!("Content-Type: {content_type}\r\n").as_bytes());
        }
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(&part.body);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_parameter_is_found_with_or_without_quotes() {
        assert_eq!(
            boundary("multipart/form-data; boundary=----abc").as_deref(),
            Some("----abc")
        );
        assert_eq!(
            boundary("Multipart/Form-Data; charset=utf-8; boundary=\"x y\"").as_deref(),
            Some("x y")
        );
        assert_eq!(boundary("application/pdf"), None);
        assert_eq!(boundary("multipart/form-data"), None);
    }

    #[test]
    fn round_trip_keeps_binary_bodies_and_metadata() {
        let parts = vec![
            Part {
                name: Some("note".to_string()),
                filename: None,
                content_type: None,
                body: b"hello".to_vec(),
            },
            Part {
                name: Some("file".to_string()),
                filename: Some("scan.pdf".to_string()),
                content_type: Some("application/pdf".to_string()),
                body: b"%PDF-1.5\r\n--not-a-boundary\r\n\x00\xff".to_vec(),
            },
        ];
        let body = encode("boundary42", &parts);
        let parsed = parse(&body, "boundary42").unwrap();
        assert_eq!(parsed, parts);
        let file = file_part(parsed).unwrap();
        assert_eq!(file.filename.as_deref(), Some("scan.pdf"));
        assert!(file.body.starts_with(b"%PDF-1.5"));
    }

    #[test]
    fn malformed_bodies_are_rejected_not_panicked() {
        assert!(parse(b"", "b").is_err());
        assert!(
            parse(
                b"--b\r\nContent-Disposition: form-data; name=\"f\"\r\n\r\nunterminated",
                "b"
            )
            .is_err()
        );
        assert!(parse(b"--bXX\r\n\r\n\r\n--b--", "b").is_err());
        assert_eq!(parse(b"--b--\r\n", "b").unwrap(), Vec::new());
    }

    #[test]
    fn file_part_prefers_the_file_field_then_a_filename() {
        let named = |name: &str, filename: Option<&str>| Part {
            name: Some(name.to_string()),
            filename: filename.map(str::to_string),
            content_type: None,
            body: Vec::new(),
        };
        let chosen = file_part(vec![named("a", Some("a.png")), named("file", None)]).unwrap();
        assert_eq!(chosen.name.as_deref(), Some("file"));
        let chosen = file_part(vec![named("a", None), named("b", Some("b.png"))]).unwrap();
        assert_eq!(chosen.name.as_deref(), Some("b"));
        assert!(file_part(Vec::new()).is_none());
    }
}
