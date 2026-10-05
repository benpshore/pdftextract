//! Optional, explicitly configured native providers. The ABI is in
//! `native/provider.h`; engine-specific exception handling stays in C/C++.
//! CLI calls execute inside the existing disposable worker. Providers never
//! spawn children. Library callers must supply their own process boundary.
//!
//! Provider/runtime files are trusted deployment code, not PDF inputs. Both
//! absolute files are bounded and fingerprinted before loading; their bytes
//! must remain unchanged during a job. Transitive system dependencies are not
//! snapshotted. No default loader search or current-directory lookup is used.

use std::collections::BTreeMap;
use std::ffi::CString;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use tpe_ffi::provider::{OpenError, ProviderDocument, ProviderLibrary};
use unicode_normalization::UnicodeNormalization;

use super::{BackendError, DocumentSession, EncryptionProblem, Extractor};
use crate::schema::{BBox, BackendIdentity, Figure, Link, PageText, Span, config_digest};

const ABI: u32 = tpe_ffi::provider::ABI;
const MAX_LIBRARY: u64 = 256 * 1024 * 1024;
const MAX_OUTPUT: usize = 16 * 1024 * 1024;
const MAX_CHARACTERS: usize = 1_000_000;

/// Only these built-in engine specifications can select a provider.
#[derive(Clone, Copy, Debug)]
pub enum Engine {
    #[cfg(feature = "mupdf")]
    MuPdf,
    #[cfg(feature = "poppler")]
    Poppler,
}
impl Engine {
    fn names(self) -> (&'static str, &'static str, &'static str) {
        match self {
            #[cfg(feature = "mupdf")]
            Self::MuPdf => ("mupdf", "TPE_MUPDF_PROVIDER_PATH", "MUPDF_DYNAMIC_LIB_PATH"),
            #[cfg(feature = "poppler")]
            Self::Poppler => (
                "poppler",
                "TPE_POPPLER_PROVIDER_PATH",
                "POPPLER_DYNAMIC_LIB_PATH",
            ),
        }
    }
}

#[derive(Clone, Debug)]
struct Configuration {
    provider: PathBuf,
    runtime: PathBuf,
    provider_sha: String,
    runtime_sha: String,
}
impl Configuration {
    fn read(engine: Engine) -> Result<Self, String> {
        let (_, provider, runtime) = engine.names();
        let path = |name| -> Result<PathBuf, String> {
            let value = std::env::var_os(name)
                .ok_or_else(|| format!("set {name} to an absolute library file"))?;
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                return Err(format!("{name} must name an absolute library file"));
            }
            path.canonicalize().map_err(|e| format!("{name}: {e}"))
        };
        let provider = path(provider)?;
        let runtime = path(runtime)?;
        Ok(Self {
            provider_sha: fingerprint(&provider)?,
            runtime_sha: fingerprint(&runtime)?,
            provider,
            runtime,
        })
    }
    fn verify(&self) -> Result<(), String> {
        if fingerprint(&self.provider)? != self.provider_sha
            || fingerprint(&self.runtime)? != self.runtime_sha
        {
            return Err(
                "configured native provider/runtime changed after fingerprinting".to_string(),
            );
        }
        Ok(())
    }
}

fn fingerprint(path: &Path) -> Result<String, String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_LIBRARY {
        return Err("native library must be a regular file no larger than 256 MiB".to_string());
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut bytes = [0; 16 * 1024];
    let mut length = 0_u64;
    loop {
        let count = file.read(&mut bytes).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        length += count as u64;
        if length > MAX_LIBRARY {
            return Err("native library exceeded 256 MiB while hashing".to_string());
        }
        digest.update(&bytes[..count]);
    }
    Ok(hex::encode(digest.finalize()))
}

/// The same safe Rust adapter is used for every optional C ABI provider.
#[derive(Clone, Debug)]
pub struct NativeProviderBackend {
    engine: Engine,
    configuration: Result<Configuration, String>,
}
impl NativeProviderBackend {
    pub fn new(engine: Engine) -> Self {
        Self {
            engine,
            configuration: Configuration::read(engine),
        }
    }
}
impl Extractor for NativeProviderBackend {
    fn identity(&self) -> BackendIdentity {
        let mut config = BTreeMap::new();
        config.insert("abi".to_string(), ABI.to_string());
        config.insert("page_output_limit".to_string(), MAX_OUTPUT.to_string());
        match &self.configuration {
            Ok(c) => {
                config.insert(
                    "provider_path".to_string(),
                    c.provider.to_string_lossy().into_owned(),
                );
                config.insert(
                    "runtime_path".to_string(),
                    c.runtime.to_string_lossy().into_owned(),
                );
                config.insert("provider_sha256".to_string(), c.provider_sha.clone());
                config.insert("runtime_sha256".to_string(), c.runtime_sha.clone());
            }
            Err(e) => {
                config.insert("configuration_error".to_string(), e.clone());
            }
        }
        BackendIdentity {
            name: self.engine.names().0.to_string(),
            version: "native-provider-abi-1".to_string(),
            config_digest: config_digest(&config),
        }
    }
    fn open(
        &self,
        bytes: &[u8],
        password: Option<&str>,
    ) -> Result<Box<dyn DocumentSession>, BackendError> {
        let config = self
            .configuration
            .as_ref()
            .map_err(|e| BackendError::Unsupported(e.clone()))?;
        let api =
            Arc::new(load_api(config, self.engine.names().0).map_err(BackendError::Unsupported)?);
        let bytes = bytes.to_vec().into_boxed_slice();
        let password = password
            .map(CString::new)
            .transpose()
            .map_err(|_| BackendError::Unsupported("password contains NUL".to_string()))?;
        let document = api
            .open(bytes, password.as_deref())
            .map_err(|error| match error {
                OpenError::PasswordRequired => {
                    BackendError::Encrypted(EncryptionProblem::PasswordRequired)
                }
                OpenError::WrongPassword => {
                    BackendError::Encrypted(EncryptionProblem::WrongPassword)
                }
                OpenError::Failed(message) => BackendError::Malformed(format!(
                    "{} provider open: {message}",
                    self.engine.names().0
                )),
            })?;
        Ok(Box::new(Session { document }))
    }
}

/// Load the configured provider/runtime pair, re-fingerprinting both files
/// before and after symbol resolution (`Configuration::verify`).
fn load_api(config: &Configuration, expected_engine: &str) -> Result<ProviderLibrary, String> {
    ProviderLibrary::load(&config.provider, &config.runtime, expected_engine, &|| {
        config.verify()
    })
}

struct Session {
    document: ProviderDocument,
}
impl DocumentSession for Session {
    fn page_count(&self) -> u32 {
        self.document.page_count()
    }
    fn info(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }
    fn page_text(&mut self, page: u32) -> Result<PageText, BackendError> {
        let pages = self.document.page_count();
        if page == 0 || page > pages {
            return Err(BackendError::PageRange { page, count: pages });
        }
        let fail = |message| BackendError::Page { page, message };
        let bytes = self.document.page(page, MAX_OUTPUT).map_err(fail)?;
        let mut result = parse_page(&bytes, page).map_err(fail)?;
        result.warnings.push(format!(
            "native_provider: {} {}",
            self.document.library().engine(),
            self.document.library().version()
        ));
        Ok(result)
    }
}

#[derive(Deserialize)]
struct NativePage {
    abi: u32,
    page: u32,
    bounds: [f32; 4],
    to_pdf: [f32; 6],
    page_size: [f32; 2],
    rotation: i32,
    characters: usize,
    unmapped: usize,
    warnings: u32,
    structured: Structured,
    links: Vec<NativeLink>,
}
#[derive(Deserialize)]
struct Structured {
    blocks: Vec<Block>,
}
#[derive(Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    bbox: Rect,
    #[serde(default)]
    lines: Vec<NativeLine>,
}
#[derive(Deserialize)]
struct NativeLine {
    bbox: Rect,
    font: Font,
    text: String,
}
#[derive(Deserialize)]
struct Font {
    name: String,
    size: f32,
}
#[derive(Clone, Copy, Deserialize)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}
#[derive(Deserialize)]
struct NativeLink {
    uri: String,
    bounds: Option<[f32; 4]>,
}

fn checked_bounds(b: [f32; 4]) -> Result<[f32; 4], String> {
    if b.iter().all(|v| v.is_finite() && v.abs() <= 1.0e8) && b[0] <= b[2] && b[1] <= b[3] {
        Ok(b)
    } else {
        Err("provider returned invalid geometry".to_string())
    }
}
fn convert(b: [f32; 4], transform: [f32; 6]) -> Result<BBox, String> {
    let b = checked_bounds(b)?;
    let [xx, yx, xy, yy, tx, ty] = transform;
    let corners = [(b[0], b[1]), (b[0], b[3]), (b[2], b[1]), (b[2], b[3])];
    let mut result = BBox {
        x0: f32::INFINITY,
        y0: f32::INFINITY,
        x1: f32::NEG_INFINITY,
        y1: f32::NEG_INFINITY,
    };
    for (x, y) in corners {
        let (x, y) = (xx * x + xy * y + tx, yx * x + yy * y + ty);
        result.x0 = result.x0.min(x);
        result.y0 = result.y0.min(y);
        result.x1 = result.x1.max(x);
        result.y1 = result.y1.max(y);
    }
    checked_bounds([result.x0, result.y0, result.x1, result.y1])?;
    Ok(result)
}
impl Rect {
    fn convert(self, transform: [f32; 6]) -> Result<BBox, String> {
        convert(
            [self.x, self.y, self.x + self.w, self.y + self.h],
            transform,
        )
    }
}
fn parse_page(bytes: &[u8], requested: u32) -> Result<PageText, String> {
    let raw: NativePage =
        serde_json::from_slice(bytes).map_err(|e| format!("invalid provider JSON: {e}"))?;
    let bounds = checked_bounds(raw.bounds)?;
    if raw.abi != ABI
        || raw.page != requested
        || bounds[2] <= bounds[0]
        || bounds[3] <= bounds[1]
        || raw.characters > MAX_CHARACTERS
        || raw.unmapped > raw.characters
        || raw.links.len() > 100_000
    {
        return Err("provider page identity, bounds, or coverage is invalid".to_string());
    }
    if !matches!(raw.rotation, 0 | 90 | 180 | 270) {
        return Err("provider page rotation is invalid".to_string());
    }
    let size = raw.page_size;
    if size
        .iter()
        .any(|v| !v.is_finite() || *v <= 0.0 || *v > 1.0e8)
    {
        return Err("provider unrotated page size is invalid".to_string());
    }
    let [xx, yx, xy, yy, tx, ty] = raw.to_pdf;
    if [xx, yx, xy, yy, tx, ty]
        .iter()
        .any(|v| !v.is_finite() || v.abs() > 1.0e8)
    {
        return Err("provider PDF coordinate transform is invalid".to_string());
    }
    let font_scale = (xx * yy - yx * xy).abs().sqrt();
    if !font_scale.is_finite() || font_scale == 0.0 {
        return Err("provider PDF coordinate transform is singular".to_string());
    }
    let mut page = PageText::new(requested, size[0], size[1], raw.rotation);
    let mut characters = 0;
    let mut replacements = 0;
    for block in raw.structured.blocks {
        let bbox = block.bbox.convert(raw.to_pdf)?;
        match block.kind.as_str() {
            "text" => {
                for line in block.lines {
                    let bbox = line.bbox.convert(raw.to_pdf)?;
                    if !line.font.size.is_finite()
                        || line.font.size < 0.0
                        || line.font.size * font_scale > 1.0e8
                    {
                        return Err("provider returned invalid font size".to_string());
                    }
                    characters += line.text.chars().count();
                    replacements += line
                        .text
                        .chars()
                        .filter(|c| *c == '\u{fffd}' || *c == '\0')
                        .count();
                    if characters > MAX_CHARACTERS || page.spans.len() >= MAX_CHARACTERS {
                        return Err("provider page exceeds character limit".to_string());
                    }
                    page.spans.push(Span {
                        text: line.text.nfc().collect(),
                        bbox: Some(bbox),
                        font: Some(line.font.name),
                        size: Some(line.font.size * font_scale),
                        seq: u32::try_from(page.spans.len()).map_err(|e| e.to_string())?,
                    });
                }
            }
            "image" => page.figures.push(Figure {
                index: u32::try_from(page.figures.len()).map_err(|e| e.to_string())?,
                bbox: Some(bbox),
                kind: "raster".to_string(),
                mime: None,
                width_px: None,
                height_px: None,
                sha256: None,
                file: None,
                caption: None,
            }),
            _ => return Err(format!("unsupported provider block type: {}", block.kind)),
        }
    }
    if characters != raw.characters {
        return Err("provider text coverage count differs from output".to_string());
    }
    if raw.unmapped > 0 || replacements > 0 {
        page.warnings.push(format!(
            "unicode_mapping: native provider reported {} unresolved characters",
            raw.unmapped.max(replacements)
        ));
    }
    if raw.warnings > 0 {
        page.warnings.push(format!(
            "extraction_incomplete: native provider reported {} parser warnings",
            raw.warnings
        ));
    }
    for link in raw.links {
        page.links.push(Link {
            bbox: link
                .bounds
                .map(|rect| convert(rect, raw.to_pdf))
                .transpose()?,
            uri: link.uri,
        });
    }
    crate::router::mark_incomplete(&mut page);
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        serde_json::json!({"abi":1,"page":2,"bounds":[10,20,210,320],"rotation":0,"page_size":[200,300],"to_pdf":[1,0,0,-1,-10,320],"characters":3,"unmapped":0,"warnings":0,"structured":{"blocks":[{"type":"text","bbox":{"x":20,"y":30,"w":40,"h":15},"lines":[{"bbox":{"x":20,"y":30,"w":40,"h":15},"font":{"name":"Times","size":12},"text":"abc"}]}]},"links":[{"uri":"https://doi.org/10.1234/a?x=\"b\"","bounds":[20,30,60,45]}]})
    }
    #[test]
    fn preserves_real_geometry_fonts_and_raw_uri() {
        let result = parse_page(&serde_json::to_vec(&fixture()).unwrap(), 2).unwrap();
        assert_eq!((result.width, result.height), (200.0, 300.0));
        assert_eq!(
            result.spans[0].bbox,
            Some(BBox {
                x0: 10.0,
                y0: 275.0,
                x1: 50.0,
                y1: 290.0
            })
        );
        assert_eq!(result.spans[0].font.as_deref(), Some("Times"));
        assert_eq!(result.links[0].uri, "https://doi.org/10.1234/a?x=\"b\"");
        assert_eq!(result.extraction_status(), crate::schema::Status::Complete);
    }
    #[test]
    fn rejects_mismatched_coverage_page_geometry_and_missing_links() {
        for (field, value) in [
            ("characters", serde_json::json!(2)),
            ("page", serde_json::json!(1)),
            ("bounds", serde_json::json!([10, 20, 10, 30])),
        ] {
            let mut data = fixture();
            data[field] = value;
            assert!(parse_page(&serde_json::to_vec(&data).unwrap(), 2).is_err());
        }
        let mut data = fixture();
        data.as_object_mut().unwrap().remove("links");
        assert!(parse_page(&serde_json::to_vec(&data).unwrap(), 2).is_err());
    }
    #[test]
    fn reports_unknown_characters_and_native_parser_warnings_as_partial() {
        for field in ["unmapped", "warnings"] {
            let mut data = fixture();
            data[field] = serde_json::json!(1);
            assert_eq!(
                parse_page(&serde_json::to_vec(&data).unwrap(), 2)
                    .unwrap()
                    .extraction_status(),
                crate::schema::Status::Partial
            );
        }
    }
    #[cfg(feature = "mupdf")]
    #[test]
    #[ignore = "requires separately licensed MuPDF provider and runtime paths"]
    fn native_output_limit_fails_closed_and_releases_the_document() {
        let config = Configuration::read(Engine::MuPdf).unwrap();
        let api = Arc::new(load_api(&config, "mupdf").unwrap());
        let bytes = super::super::probe_pdf().unwrap().into_boxed_slice();
        let document = api.open(bytes, None).unwrap();
        let error = document.page(1, 32).unwrap_err();
        assert!(error.contains("output limit"), "{error}");
        // The provider contract: no output buffer is handed back on failure.
        assert!(
            !error.contains("provider returned output on failure"),
            "{error}"
        );
        let output = document.page(1, MAX_OUTPUT).unwrap();
        assert!(output.len() > 32 && output.len() < MAX_OUTPUT);
        drop(document);
    }

    #[test]
    fn library_fingerprint_is_bounded_and_content_based() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime");
        std::fs::write(&path, b"first").unwrap();
        let first = fingerprint(&path).unwrap();
        std::fs::write(&path, b"other").unwrap();
        assert_ne!(first, fingerprint(&path).unwrap());
        File::create(&path)
            .unwrap()
            .set_len(MAX_LIBRARY + 1)
            .unwrap();
        assert!(fingerprint(&path).is_err());
    }
}
