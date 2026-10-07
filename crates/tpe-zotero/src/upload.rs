//! Attachment file uploads (Zotero Web API v3, "File Uploads" page).
//!
//! Three steps for an `imported_file` attachment item that already exists:
//!
//! 1. **Authorisation**: `POST <prefix>/items/<key>/file` with a form body
//!    `md5=<hex>&filename=<name>&filesize=<bytes>&mtime=<ms>` and either
//!    `If-None-Match: *` (the item has no file yet) or `If-Match: <md5 of
//!    the file currently stored>`. The `200` body is `{"exists": 1}` when
//!    the server already holds an identical file (it is linked, nothing to
//!    upload), or an upload target: `{"url", "contentType", "prefix",
//!    "suffix", "uploadKey"}` for a full upload, or `{"url", "params":
//!    {...}, "uploadKey"}` for a `multipart/form-data` upload.
//! 2. **Upload**: `POST <url>` to the storage host with `Content-Type:
//!    <contentType>` and the body `<prefix><file bytes><suffix>`, or a
//!    multipart form with every `params` field followed by the `file` part.
//!    The storage host answers `201`. No Zotero headers go to that host.
//! 3. **Registration**: `POST <prefix>/items/<key>/file` with
//!    `upload=<uploadKey>` and the same `If-None-Match` / `If-Match` header.
//!    `204` on success; `412` when the file changed since step 1 (start
//!    again).
//!
//! `mtime` is in milliseconds. The item's `md5`, `mtime`, `filename` and
//! `contentType` properties are set when the attachment item is created
//! ([`crate::ZItemPatch::imported_file_attachment`]).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use md5::{Digest, Md5};
use serde_json::Value;

use crate::client::encode_component;
use crate::error::ZError;

/// What the server needs to know about a file before it hands out an
/// upload target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDescriptor {
    /// Lower-case hexadecimal MD5 of the file bytes.
    pub md5: String,
    /// File name as it should be stored (no directories).
    pub filename: String,
    /// Size in bytes.
    pub filesize: u64,
    /// Modification time in milliseconds since the Unix epoch.
    pub mtime_ms: u64,
}

impl FileDescriptor {
    /// Describe in-memory file bytes.
    pub fn from_bytes(bytes: &[u8], filename: &str, mtime_ms: u64) -> Self {
        Self {
            md5: md5_hex(bytes),
            filename: filename.to_string(),
            filesize: bytes.len() as u64,
            mtime_ms,
        }
    }

    /// The `application/x-www-form-urlencoded` body of the authorisation
    /// request (step 1).
    pub fn authorization_body(&self) -> String {
        form_encode(&[
            ("md5", &self.md5),
            ("filename", &self.filename),
            ("filesize", &self.filesize.to_string()),
            ("mtime", &self.mtime_ms.to_string()),
        ])
    }
}

/// Lower-case hexadecimal MD5 digest, the checksum Zotero uses for files.
pub fn md5_hex(bytes: &[u8]) -> String {
    let digest = Md5::digest(bytes);
    let mut out = String::with_capacity(32);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// `application/x-www-form-urlencoded` encoding of name/value pairs.
pub fn form_encode(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| format!("{}={}", encode_component(name), encode_component(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// The body that registers a finished upload (step 3).
pub fn registration_body(upload_key: &str) -> String {
    form_encode(&[("upload", upload_key)])
}

/// Where and how to send the bytes (step 2).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UploadTarget {
    /// Storage URL to `POST` to.
    pub url: String,
    /// `Content-Type` for a full upload (`None` for the multipart form).
    pub content_type: Option<String>,
    /// Bytes that go before the file in a full upload.
    pub prefix: String,
    /// Bytes that go after the file in a full upload.
    pub suffix: String,
    /// Token to pass back in step 3.
    pub upload_key: String,
    /// Form fields of a multipart upload (empty for a full upload).
    pub params: BTreeMap<String, String>,
}

impl UploadTarget {
    /// True when the server asked for a `multipart/form-data` upload.
    pub fn is_multipart(&self) -> bool {
        !self.params.is_empty()
    }

    /// Body of a full upload: `prefix`, file bytes, `suffix`.
    pub fn full_body(&self, file: &[u8]) -> Vec<u8> {
        let mut body = Vec::with_capacity(self.prefix.len() + file.len() + self.suffix.len());
        body.extend_from_slice(self.prefix.as_bytes());
        body.extend_from_slice(file);
        body.extend_from_slice(self.suffix.as_bytes());
        body
    }

    /// Body of a multipart upload: every `params` field, then the file as
    /// the `file` part. `boundary` must not occur in any field or the file.
    pub fn multipart_body(&self, file: &[u8], filename: &str, boundary: &str) -> Vec<u8> {
        let mut body = Vec::with_capacity(file.len() + 512);
        for (name, value) in &self.params {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{value}\r\n",
                    quote_form_name(name)
                )
                .as_bytes(),
            );
        }
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n",
                quote_form_name(filename)
            )
            .as_bytes(),
        );
        body.extend_from_slice(file);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        body
    }

    /// The `Content-Type` header and body for this target.
    pub fn request_body(&self, file: &[u8], filename: &str, boundary: &str) -> (String, Vec<u8>) {
        if self.is_multipart() {
            (
                format!("multipart/form-data; boundary={boundary}"),
                self.multipart_body(file, filename, boundary),
            )
        } else {
            (
                self.content_type
                    .clone()
                    .unwrap_or_else(|| "application/octet-stream".to_string()),
                self.full_body(file),
            )
        }
    }
}

/// Escape a multipart form name or file name for a quoted parameter.
fn quote_form_name(name: &str) -> String {
    name.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\r', '\n'], " ")
}

/// Answer to the authorisation request (step 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UploadAuthorization {
    /// `{"exists": 1}`: the server already had the file and linked it.
    Exists,
    /// An upload is needed.
    Upload(UploadTarget),
}

/// Parse the `200` body of the authorisation request.
pub fn parse_upload_authorization(body: &str) -> Result<UploadAuthorization, ZError> {
    let value: Value = serde_json::from_str(body)?;
    let object = value
        .as_object()
        .ok_or_else(|| ZError::Parse("upload authorisation is not a JSON object".to_string()))?;
    if object
        .get("exists")
        .is_some_and(|v| v.as_u64() == Some(1) || v.as_bool() == Some(true))
    {
        return Ok(UploadAuthorization::Exists);
    }
    let text = |name: &str| object.get(name).and_then(Value::as_str).map(str::to_string);
    let url = text("url")
        .ok_or_else(|| ZError::Parse("upload authorisation without a url".to_string()))?;
    let upload_key = text("uploadKey")
        .ok_or_else(|| ZError::Parse("upload authorisation without an uploadKey".to_string()))?;
    let params = object
        .get("params")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    Ok(UploadAuthorization::Upload(UploadTarget {
        url,
        content_type: text("contentType"),
        prefix: text("prefix").unwrap_or_default(),
        suffix: text("suffix").unwrap_or_default(),
        upload_key,
        params,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_rfc_1321_vectors() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5_hex(b"message digest"),
            "f96b697d7cb7938d525a2f31aaf161d0"
        );
    }

    #[test]
    fn descriptor_body_is_form_encoded() {
        let file = FileDescriptor::from_bytes(b"abc", "my paper & notes.pdf", 1_700_000_000_123);
        assert_eq!(file.filesize, 3);
        assert_eq!(
            file.authorization_body(),
            "md5=900150983cd24fb0d6963f7d28e17f72&filename=my%20paper%20%26%20notes.pdf\
             &filesize=3&mtime=1700000000123"
        );
        assert_eq!(registration_body("ABC123"), "upload=ABC123");
    }

    // Shapes from the "File Uploads" page: existing file, full upload, multipart.
    #[test]
    fn authorization_exists() {
        assert_eq!(
            parse_upload_authorization(r#"{"exists": 1}"#).unwrap(),
            UploadAuthorization::Exists
        );
    }

    #[test]
    fn authorization_full_upload() {
        let body = r#"{
          "url": "https://zoterofilestorage.s3.amazonaws.com/",
          "contentType": "multipart/form-data; boundary=A1B2C3",
          "prefix": "--A1B2C3\r\nContent-Disposition: form-data; name=\"key\"\r\n\r\nabc\r\n--A1B2C3\r\nContent-Disposition: form-data; name=\"file\"\r\n\r\n",
          "suffix": "\r\n--A1B2C3--",
          "uploadKey": "d8b9e4a1c0f3"
        }"#;
        let UploadAuthorization::Upload(target) = parse_upload_authorization(body).unwrap() else {
            panic!("expected an upload target");
        };
        assert!(!target.is_multipart());
        assert_eq!(target.upload_key, "d8b9e4a1c0f3");
        let (content_type, bytes) = target.request_body(b"%PDF-1.7", "a.pdf", "unused");
        assert_eq!(content_type, "multipart/form-data; boundary=A1B2C3");
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("--A1B2C3\r\n"));
        assert!(text.contains("\r\n\r\n%PDF-1.7\r\n--A1B2C3--"));
    }

    #[test]
    fn authorization_multipart_upload() {
        let body = r#"{
          "url": "https://storage.example/",
          "params": {"key": "files/abc", "acl": "private", "policy": "cG9saWN5"},
          "uploadKey": "k1"
        }"#;
        let UploadAuthorization::Upload(target) = parse_upload_authorization(body).unwrap() else {
            panic!("expected an upload target");
        };
        assert!(target.is_multipart());
        let (content_type, bytes) = target.request_body(b"xyz", "we\"ird.pdf", "B0UND");
        assert_eq!(content_type, "multipart/form-data; boundary=B0UND");
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("name=\"acl\"\r\n\r\nprivate\r\n"));
        assert!(text.contains("name=\"policy\"\r\n\r\ncG9saWN5\r\n"));
        assert!(text.contains("name=\"file\"; filename=\"we\\\"ird.pdf\""));
        assert!(text.ends_with("xyz\r\n--B0UND--\r\n"));
        // The file part comes after every parameter.
        assert!(text.find("name=\"file\"").unwrap() > text.find("name=\"policy\"").unwrap());
    }

    #[test]
    fn authorization_rejects_bad_shapes() {
        assert!(matches!(
            parse_upload_authorization(r#"{"uploadKey": "k"}"#),
            Err(ZError::Parse(_))
        ));
        assert!(matches!(
            parse_upload_authorization("[]"),
            Err(ZError::Parse(_))
        ));
        assert!(parse_upload_authorization("not json").is_err());
    }
}
