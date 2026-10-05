//! Pinned, hash-verified provisioning and discovery of the `PDFium` shared
//! library. This module does not need the `pdfium` feature: it is plain
//! download, digest and archive handling, exposed as `tpe pdfium fetch|status|path`
//! (and the stand-alone `tpe-pdfium` binary) for any build.
//!
//! `native/manifest.json` is compiled into the binary. [`fetch`] downloads the
//! pinned `bblanchon/pdfium-binaries` archive for the host platform, verifies
//! the archive and the extracted library against the pinned SHA-256 digests,
//! and installs the library under the per-user data directory:
//!
//! * Linux: `$XDG_DATA_HOME/tpe/pdfium/<release>/<platform>/` (default
//!   `~/.local/share/...`)
//! * macOS: `~/Library/Application Support/tpe/pdfium/<release>/<platform>/`
//! * Windows: `%LOCALAPPDATA%\tpe\pdfium\<release>\<platform>\`
//!
//! `TPE_DATA_DIR` (absolute) replaces the platform data directory when set.
//!
//! [`installed_library`] reports that file only while its digest still matches
//! the pin, so the `pdfium` backend can load it without an environment
//! variable and without ever trusting the working directory. Everything is
//! bounded: archives and libraries over 64 MiB are refused before any byte is
//! kept, the archive member is extracted into memory from a decoder (no paths
//! from the archive touch the file system), and the install is an atomic
//! rename of a verified temporary file.

use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use ureq::tls::{Certificate, PemItem, RootCerts, TlsConfig, parse_pem};

use crate::schema::sha256_hex;

/// The pinned artifact manifest, compiled in so a binary cannot be pointed at
/// a different pin by a file next to it.
pub const MANIFEST_JSON: &str = include_str!("../native/manifest.json");
/// Environment variable that replaces the platform data directory.
pub const ENV_DATA_DIR: &str = "TPE_DATA_DIR";
/// Environment variable the `pdfium` backend reads for an explicit library.
pub const ENV_LIBRARY_PATH: &str = "PDFIUM_DYNAMIC_LIB_PATH";
/// Refuse archives larger than this before keeping any byte.
pub const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
/// Refuse libraries larger than this (the backend fingerprints up to 64 MiB).
pub const MAX_LIBRARY_BYTES: u64 = 64 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(60);
const RETRY_DELAY: Duration = Duration::from_secs(2);

#[derive(Deserialize)]
struct Manifest {
    pdfium_release: String,
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    kind: String,
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    member: Option<String>,
    #[serde(default)]
    archive_sha256: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
}

/// One platform's pinned library: where it comes from and what it must hash to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    /// Upstream release tag, e.g. `chromium/8066`.
    pub release: String,
    /// Manifest platform key, e.g. `linux-x64`.
    pub platform: String,
    /// Archive URL (HTTPS, from the compiled-in manifest).
    pub url: String,
    /// Archive member holding the library, e.g. `lib/libpdfium.so`.
    pub member: String,
    /// SHA-256 of the whole archive.
    pub archive_sha256: String,
    /// SHA-256 of the extracted library file.
    pub library_sha256: String,
}

impl Pin {
    /// File name of the library inside the install directory.
    #[must_use]
    pub fn library_file_name(&self) -> &str {
        self.member.rsplit('/').next().unwrap_or(&self.member)
    }

    /// `chromium/8066` → `chromium-8066`, safe as one path component.
    #[must_use]
    pub fn release_slug(&self) -> String {
        self.release
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' {
                    c
                } else {
                    '-'
                }
            })
            .collect()
    }
}

/// Why provisioning or discovery failed.
#[derive(Debug)]
pub enum ProvisionError {
    /// No pinned library for this operating system and architecture.
    UnsupportedPlatform(String),
    /// The compiled-in manifest is malformed or the entry is not fully pinned.
    Manifest(String),
    /// Download failure (after one retry for transient errors).
    Http(String),
    /// File-system failure.
    Io(io::Error),
    /// The downloaded archive does not hash to the pin.
    ArchiveDigest {
        /// Pinned digest.
        expected: String,
        /// Observed digest.
        actual: String,
    },
    /// The extracted library does not hash to the pin.
    LibraryDigest {
        /// Pinned digest.
        expected: String,
        /// Observed digest.
        actual: String,
    },
    /// The archive has no member with the pinned name.
    MemberMissing(String),
    /// The archive or library exceeds the size bound.
    TooLarge(u64),
    /// No per-user data directory could be determined.
    NoDataDir(String),
}

impl fmt::Display for ProvisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform(detail) => {
                write!(f, "no pinned PDFium library for this platform ({detail})")
            }
            Self::Manifest(detail) => write!(f, "native/manifest.json: {detail}"),
            Self::Http(detail) => write!(f, "download failed: {detail}"),
            Self::Io(err) => write!(f, "file system: {err}"),
            Self::ArchiveDigest { expected, actual } => write!(
                f,
                "archive sha256 {actual} does not match the pinned {expected}; nothing installed"
            ),
            Self::LibraryDigest { expected, actual } => write!(
                f,
                "library sha256 {actual} does not match the pinned {expected}; nothing installed"
            ),
            Self::MemberMissing(member) => write!(f, "archive has no member {member}"),
            Self::TooLarge(bytes) => write!(f, "refusing {bytes} bytes (bound is 64 MiB)"),
            Self::NoDataDir(detail) => write!(f, "no data directory: {detail}"),
        }
    }
}

impl std::error::Error for ProvisionError {}

impl From<io::Error> for ProvisionError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// Manifest platform key for the running binary.
///
/// # Errors
/// When the OS/architecture pair has no manifest key at all.
pub fn host_platform() -> Result<&'static str, ProvisionError> {
    platform_key(std::env::consts::OS, std::env::consts::ARCH)
}

/// Manifest platform key for an OS/architecture pair.
///
/// # Errors
/// When the pair is not one `bblanchon/pdfium-binaries` publishes.
pub fn platform_key(os: &str, arch: &str) -> Result<&'static str, ProvisionError> {
    match (os, arch) {
        ("linux", "x86_64") => Ok("linux-x64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        ("macos", "aarch64") => Ok("mac-arm64"),
        ("macos", "x86_64") => Ok("mac-x64"),
        ("windows", "x86_64") => Ok("win-x64"),
        ("windows", "aarch64") => Ok("win-arm64"),
        _ => Err(ProvisionError::UnsupportedPlatform(format!("{os}/{arch}"))),
    }
}

/// The compiled-in release tag.
///
/// # Errors
/// When the manifest cannot be parsed.
pub fn pinned_release() -> Result<String, ProvisionError> {
    Ok(parse_manifest(MANIFEST_JSON)?.pdfium_release)
}

/// The pin for `platform` from the compiled-in manifest.
///
/// # Errors
/// When there is no entry, more than one, or the entry is not fully pinned.
pub fn pin_for(platform: &str) -> Result<Pin, ProvisionError> {
    pin_from(MANIFEST_JSON, platform)
}

fn parse_manifest(json: &str) -> Result<Manifest, ProvisionError> {
    serde_json::from_str(json).map_err(|err| ProvisionError::Manifest(err.to_string()))
}

fn pin_from(json: &str, platform: &str) -> Result<Pin, ProvisionError> {
    let manifest = parse_manifest(json)?;
    let mut matches = manifest
        .entries
        .into_iter()
        .filter(|entry| entry.kind == "pdfium" && entry.platform.as_deref() == Some(platform));
    let Some(entry) = matches.next() else {
        return Err(ProvisionError::UnsupportedPlatform(format!(
            "no pdfium entry for {platform}"
        )));
    };
    if matches.next().is_some() {
        return Err(ProvisionError::Manifest(format!(
            "more than one pdfium entry for {platform}"
        )));
    }
    let field = |name: &str, value: Option<String>| {
        value.filter(|v| !v.is_empty()).ok_or_else(|| {
            ProvisionError::Manifest(format!("pdfium entry for {platform} lacks {name}"))
        })
    };
    let url = field("url", entry.url)?;
    if !url.starts_with("https://") {
        return Err(ProvisionError::Manifest(format!("{url} is not https")));
    }
    let member = field("member", entry.member)?;
    if member.starts_with('/') || member.split('/').any(|part| part == "..") {
        return Err(ProvisionError::Manifest(format!(
            "member {member} is not a plain path"
        )));
    }
    let archive_sha256 = digest_field("archive_sha256", entry.archive_sha256, platform)?;
    let library_sha256 = digest_field("sha256", entry.sha256, platform)?;
    Ok(Pin {
        release: manifest.pdfium_release,
        platform: platform.to_string(),
        url,
        member,
        archive_sha256,
        library_sha256,
    })
}

fn digest_field(
    name: &str,
    value: Option<String>,
    platform: &str,
) -> Result<String, ProvisionError> {
    let value = value.unwrap_or_default();
    let well_formed = value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if well_formed {
        Ok(value)
    } else {
        Err(ProvisionError::Manifest(format!(
            "pdfium entry for {platform}: {name} is not a pinned lowercase sha256"
        )))
    }
}

/// The per-user root for installed libraries, `<data dir>/tpe/pdfium`.
///
/// # Errors
/// When neither `TPE_DATA_DIR` nor the platform's data directory can be found.
pub fn data_root() -> Result<PathBuf, ProvisionError> {
    data_root_from(std::env::consts::OS, |name| {
        std::env::var_os(name).map(PathBuf::from)
    })
}

/// [`data_root`] with the OS and environment lookups injected (pure; tested).
///
/// # Errors
/// As [`data_root`].
pub fn data_root_from(
    os: &str,
    var: impl Fn(&str) -> Option<PathBuf>,
) -> Result<PathBuf, ProvisionError> {
    let absolute = |name: &str| var(name).filter(|p| p.is_absolute() && !p.as_os_str().is_empty());
    if let Some(dir) = absolute(ENV_DATA_DIR) {
        return Ok(dir.join("pdfium"));
    }
    let base = match os {
        "macos" => absolute("HOME").map(|home| home.join("Library").join("Application Support")),
        "windows" => absolute("LOCALAPPDATA"),
        _ => absolute("XDG_DATA_HOME")
            .or_else(|| absolute("HOME").map(|home| home.join(".local").join("share"))),
    };
    base.map(|dir| dir.join("tpe").join("pdfium")).ok_or_else(|| {
        ProvisionError::NoDataDir(format!(
            "set {ENV_DATA_DIR} to an absolute directory (HOME/XDG_DATA_HOME/LOCALAPPDATA unset)"
        ))
    })
}

/// Directory holding `pin`'s library under `root`.
#[must_use]
pub fn install_dir(root: &Path, pin: &Pin) -> PathBuf {
    root.join(pin.release_slug()).join(&pin.platform)
}

/// Full path of `pin`'s library under `root`.
#[must_use]
pub fn install_path(root: &Path, pin: &Pin) -> PathBuf {
    install_dir(root, pin).join(pin.library_file_name())
}

/// A library file whose digest matched its pin when it was inspected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledLibrary {
    /// Absolute path of the library file.
    pub path: PathBuf,
    /// Its SHA-256 (equal to the pin).
    pub sha256: String,
    /// Upstream release tag.
    pub release: String,
    /// Manifest platform key.
    pub platform: String,
}

/// The pinned library installed for this host, only while it verifies.
///
/// `None` when the platform has no pin, no data directory exists, the file
/// is absent, oversized, unreadable, or its digest differs from the pin.
#[must_use]
pub fn installed_library() -> Option<InstalledLibrary> {
    let pin = pin_for(host_platform().ok()?).ok()?;
    let root = data_root().ok()?;
    verified_library_at(&install_path(&root, &pin), &pin)
}

/// `path` as an [`InstalledLibrary`] when it is a regular file within the size
/// bound whose SHA-256 equals `pin.library_sha256`.
#[must_use]
pub fn verified_library_at(path: &Path, pin: &Pin) -> Option<InstalledLibrary> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_LIBRARY_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    let sha256 = sha256_hex(&bytes);
    (sha256 == pin.library_sha256).then(|| InstalledLibrary {
        path: path.to_path_buf(),
        sha256,
        release: pin.release.clone(),
        platform: pin.platform.clone(),
    })
}

/// What [`fetch`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchAction {
    /// A verified copy was already installed; nothing was downloaded.
    AlreadyInstalled,
    /// Downloaded, verified and installed (first install or replaced a
    /// missing/mismatching file).
    Installed,
}

/// Result of [`fetch`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchOutcome {
    /// The verified library now on disk.
    pub library: InstalledLibrary,
    /// Whether a download happened.
    pub action: FetchAction,
}

/// Download, verify and install the pinned library for this host into the
/// per-user data directory. With `force`, re-download even when a verified
/// copy exists.
///
/// # Errors
/// Any [`ProvisionError`]; on a digest mismatch nothing is installed and any
/// previously verified copy is left untouched.
pub fn fetch(user_agent: &str, force: bool) -> Result<FetchOutcome, ProvisionError> {
    let pin = pin_for(host_platform()?)?;
    let root = data_root()?;
    fetch_into(&root, &pin, user_agent, force, |url| {
        download(url, user_agent)
    })
}

/// [`fetch`] with the root, pin and downloader injected (tested offline).
///
/// # Errors
/// As [`fetch`].
pub fn fetch_into(
    root: &Path,
    pin: &Pin,
    user_agent: &str,
    force: bool,
    downloader: impl FnOnce(&str) -> Result<Vec<u8>, ProvisionError>,
) -> Result<FetchOutcome, ProvisionError> {
    let _ = user_agent;
    let target = install_path(root, pin);
    if !force && let Some(library) = verified_library_at(&target, pin) {
        return Ok(FetchOutcome {
            library,
            action: FetchAction::AlreadyInstalled,
        });
    }
    let archive = downloader(&pin.url)?;
    if archive.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err(ProvisionError::TooLarge(archive.len() as u64));
    }
    let actual = sha256_hex(&archive);
    if actual != pin.archive_sha256 {
        return Err(ProvisionError::ArchiveDigest {
            expected: pin.archive_sha256.clone(),
            actual,
        });
    }
    let library = extract_member(&archive, &pin.member, MAX_LIBRARY_BYTES)?;
    let actual = sha256_hex(&library);
    if actual != pin.library_sha256 {
        return Err(ProvisionError::LibraryDigest {
            expected: pin.library_sha256.clone(),
            actual,
        });
    }
    let dir = install_dir(root, pin);
    fs::create_dir_all(&dir)?;
    let mut temp = tempfile::Builder::new()
        .prefix(".tpe-pdfium-")
        .tempfile_in(&dir)?;
    temp.write_all(&library)?;
    temp.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o755))?;
    }
    temp.persist(&target).map_err(|err| err.error)?;
    let library = verified_library_at(&target, pin).ok_or_else(|| {
        ProvisionError::Io(io::Error::other(
            "installed file does not verify after rename",
        ))
    })?;
    Ok(FetchOutcome {
        library,
        action: FetchAction::Installed,
    })
}

/// Read the single archive member `member` (gzip-compressed tar) into memory,
/// refusing members larger than `cap`. Archive paths never touch the file
/// system.
///
/// # Errors
/// Decoding failures, a missing member, or an oversized member.
pub fn extract_member(archive: &[u8], member: &str, cap: u64) -> Result<Vec<u8>, ProvisionError> {
    let decoder = flate2::read::GzDecoder::new(archive);
    let mut tar = tar::Archive::new(decoder);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        let name = path.to_string_lossy();
        let name = name.strip_prefix("./").unwrap_or(&name);
        if name != member {
            continue;
        }
        let size = entry.header().size()?;
        if size > cap {
            return Err(ProvisionError::TooLarge(size));
        }
        let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
        entry.by_ref().take(cap + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > cap {
            return Err(ProvisionError::TooLarge(bytes.len() as u64));
        }
        return Ok(bytes);
    }
    Err(ProvisionError::MemberMissing(member.to_string()))
}

/// Trust store for outbound HTTPS: the certificates in `$SSL_CERT_FILE` when
/// that variable names a readable PEM bundle (the convention curl, OpenSSL and
/// Python follow, and how TLS-inspecting corporate proxies are trusted), else
/// `None` for ureq's built-in Mozilla roots.
#[must_use]
pub fn trust_store_from_env() -> Option<TlsConfig> {
    let path = std::env::var_os("SSL_CERT_FILE")?;
    let pem = fs::read(path).ok()?;
    trust_store_from_pem(&pem)
}

/// [`trust_store_from_env`] for an in-memory PEM bundle; `None` when it holds
/// no certificate.
#[must_use]
pub fn trust_store_from_pem(pem: &[u8]) -> Option<TlsConfig> {
    let certs: Vec<Certificate<'static>> = parse_pem(pem)
        .filter_map(|item| match item {
            Ok(PemItem::Certificate(cert)) => Some(cert),
            _ => None,
        })
        .collect();
    if certs.is_empty() {
        return None;
    }
    Some(
        TlsConfig::builder()
            .root_certs(RootCerts::new_with_certs(&certs))
            .build(),
    )
}

fn download(url: &str, user_agent: &str) -> Result<Vec<u8>, ProvisionError> {
    let mut builder = ureq::Agent::config_builder()
        .user_agent(user_agent)
        .http_status_as_error(true)
        .max_redirects(10)
        .timeout_resolve(Some(TIMEOUT))
        .timeout_connect(Some(TIMEOUT))
        .timeout_send_request(Some(TIMEOUT))
        .timeout_recv_response(Some(TIMEOUT))
        .timeout_recv_body(Some(TIMEOUT));
    if let Some(tls) = trust_store_from_env() {
        builder = builder.tls_config(tls);
    }
    let agent = ureq::Agent::new_with_config(builder.build());
    match download_once(&agent, url) {
        Ok(bytes) => Ok(bytes),
        Err(first) if is_transient(&first) => {
            std::thread::sleep(RETRY_DELAY);
            download_once(&agent, url).map_err(|err| ProvisionError::Http(format!("{url}: {err}")))
        }
        Err(err) => Err(ProvisionError::Http(format!("{url}: {err}"))),
    }
}

fn download_once(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, ureq::Error> {
    let mut response = agent.get(url).call()?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_ARCHIVE_BYTES)
        .read_to_vec()
}

fn is_transient(err: &ureq::Error) -> bool {
    match err {
        ureq::Error::StatusCode(code) => *code == 429 || *code >= 500,
        ureq::Error::Io(_)
        | ureq::Error::Timeout(_)
        | ureq::Error::Protocol(_)
        | ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::BodyStalled => true,
        _ => false,
    }
}

/// Where the `pdfium` backend would load its library from, for `tpe-pdfium status`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effective {
    /// `PDFIUM_DYNAMIC_LIB_PATH` is set (not verified against the pin by design).
    Environment(String),
    /// The verified per-user install.
    Installed(PathBuf),
    /// Nothing usable; the backend reports `pdfium library not found`.
    None,
}

/// Snapshot of the provisioning state for this host.
#[derive(Clone, Debug)]
pub struct Status {
    /// Host platform key, or why there is none.
    pub platform: Result<String, String>,
    /// The pin for that platform, or why there is none.
    pub pin: Result<Pin, String>,
    /// Per-user root, or why there is none.
    pub root: Result<PathBuf, String>,
    /// Expected install path when platform, pin and root are known.
    pub install_path: Option<PathBuf>,
    /// Verified install, if any.
    pub installed: Option<InstalledLibrary>,
    /// `PDFIUM_DYNAMIC_LIB_PATH`, if set and non-empty.
    pub environment: Option<String>,
    /// What the backend would use, following its search order.
    pub effective: Effective,
}

impl Status {
    /// Inspect this host.
    #[must_use]
    pub fn inspect() -> Self {
        let platform = host_platform()
            .map(str::to_string)
            .map_err(|e| e.to_string());
        let pin = platform
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|p| pin_for(p).map_err(|e| e.to_string()));
        let root = data_root().map_err(|e| e.to_string());
        let install_path = match (&pin, &root) {
            (Ok(pin), Ok(root)) => Some(install_path(root, pin)),
            _ => None,
        };
        let installed = match (&pin, &install_path) {
            (Ok(pin), Some(path)) => verified_library_at(path, pin),
            _ => None,
        };
        let environment = std::env::var(ENV_LIBRARY_PATH)
            .ok()
            .filter(|value| !value.is_empty());
        let effective = match (&environment, &installed) {
            (Some(env), _) => Effective::Environment(env.clone()),
            (None, Some(lib)) => Effective::Installed(lib.path.clone()),
            (None, None) => Effective::None,
        };
        Self {
            platform,
            pin,
            root,
            install_path,
            installed,
            environment,
            effective,
        }
    }

    /// Multi-line `key: value` report.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let line = |out: &mut String, key: &str, value: String| {
            out.push_str(key);
            out.push_str(": ");
            out.push_str(&value);
            out.push('\n');
        };
        line(
            &mut out,
            "platform",
            self.platform
                .clone()
                .unwrap_or_else(|e| format!("none ({e})")),
        );
        match &self.pin {
            Ok(pin) => {
                line(&mut out, "release", pin.release.clone());
                line(&mut out, "url", pin.url.clone());
                line(
                    &mut out,
                    "pinned_library_sha256",
                    pin.library_sha256.clone(),
                );
            }
            Err(err) => line(&mut out, "pin", format!("none ({err})")),
        }
        line(
            &mut out,
            "data_root",
            self.root
                .clone()
                .map_or_else(|e| format!("none ({e})"), |p| p.display().to_string()),
        );
        line(
            &mut out,
            "install_path",
            self.install_path
                .as_ref()
                .map_or_else(|| "none".to_string(), |p| p.display().to_string()),
        );
        line(
            &mut out,
            "installed",
            self.installed.as_ref().map_or_else(
                || "no (run `tpe pdfium fetch`)".to_string(),
                |l| format!("yes, sha256 {} verified", l.sha256),
            ),
        );
        line(
            &mut out,
            ENV_LIBRARY_PATH,
            self.environment
                .clone()
                .unwrap_or_else(|| "unset".to_string()),
        );
        let effective = match &self.effective {
            Effective::Environment(path) => {
                format!("environment {path} (not checked against the pin)")
            }
            Effective::Installed(path) => format!("installed {}", path.display()),
            Effective::None => {
                "none: the pdfium backend will report `pdfium library not found`".to_string()
            }
        };
        line(&mut out, "effective", effective);
        out
    }
}

/// The `tpe pdfium ...` / `tpe-pdfium ...` command line, shared by both
/// entry points so release archives (which ship only `tpe`) carry it.
pub mod cli {
    use std::process::ExitCode;

    use clap::Subcommand;

    use super::{Effective, FetchAction, Status, fetch, installed_library};

    const USER_AGENT: &str =
        "text-processing-engine tpe-pdfium (+https://github.com/benpshore/pdftextract)";

    /// Provisioning subcommands.
    #[derive(Clone, Debug, Subcommand)]
    pub enum Command {
        /// Download, verify (SHA-256 of archive and library) and install the
        /// pinned library for this platform under the per-user data directory.
        Fetch {
            /// Re-download even when a verified copy is installed.
            #[arg(long)]
            force: bool,
        },
        /// Show the pin, the install location, and what the backend would load
        /// (exit 1 when nothing is loadable).
        Status,
        /// Print the directory holding the verified library (exit 1 when absent),
        /// for `export PDFIUM_DYNAMIC_LIB_PATH="$(tpe pdfium path)"`.
        Path,
    }

    /// Run one subcommand, printing to stdout/stderr like a CLI.
    #[must_use]
    pub fn run(command: &Command) -> ExitCode {
        match command {
            Command::Fetch { force } => match fetch(USER_AGENT, *force) {
                Ok(outcome) => {
                    let verb = match outcome.action {
                        FetchAction::AlreadyInstalled => "already installed",
                        FetchAction::Installed => "installed",
                    };
                    println!(
                        "{verb}: {} ({} {}, sha256 {} verified)",
                        outcome.library.path.display(),
                        outcome.library.release,
                        outcome.library.platform,
                        outcome.library.sha256
                    );
                    ExitCode::SUCCESS
                }
                Err(err) => {
                    eprintln!("tpe pdfium fetch: {err}");
                    ExitCode::from(2)
                }
            },
            Command::Status => {
                let status = Status::inspect();
                print!("{}", status.render());
                if matches!(status.effective, Effective::None) {
                    ExitCode::from(1)
                } else {
                    ExitCode::SUCCESS
                }
            }
            Command::Path => {
                if let Some(library) = installed_library() {
                    let dir = library.path.parent().map_or_else(
                        || library.path.display().to_string(),
                        |dir| dir.display().to_string(),
                    );
                    println!("{dir}");
                    ExitCode::SUCCESS
                } else {
                    eprintln!(
                        "tpe pdfium path: no verified library installed; run `tpe pdfium fetch`"
                    );
                    ExitCode::from(1)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gz_tar(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, bytes) in members {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, *bytes).unwrap();
        }
        let tar_bytes = builder.into_inner().unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn test_pin(archive: &[u8], library: &[u8]) -> Pin {
        Pin {
            release: "chromium/8066".to_string(),
            platform: "linux-x64".to_string(),
            url: "https://example.invalid/pdfium.tgz".to_string(),
            member: "lib/libpdfium.so".to_string(),
            archive_sha256: sha256_hex(archive),
            library_sha256: sha256_hex(library),
        }
    }

    #[test]
    fn manifest_pins_every_published_platform_fully() {
        for platform in ["linux-x64", "linux-arm64", "mac-arm64"] {
            let pin = pin_for(platform).unwrap();
            assert_eq!(pin.release, pinned_release().unwrap());
            assert!(
                pin.url
                    .starts_with("https://github.com/bblanchon/pdfium-binaries/")
            );
            assert!(pin.member.starts_with("lib/libpdfium."));
            assert_eq!(pin.archive_sha256.len(), 64);
            assert_eq!(pin.library_sha256.len(), 64);
        }
        assert!(matches!(
            pin_for("plan9-mips"),
            Err(ProvisionError::UnsupportedPlatform(_))
        ));
        assert_eq!(platform_key("linux", "x86_64").unwrap(), "linux-x64");
        assert_eq!(platform_key("macos", "aarch64").unwrap(), "mac-arm64");
        assert!(platform_key("haiku", "x86_64").is_err());
    }

    #[test]
    fn unpinned_or_unsafe_entries_are_rejected() {
        let json = r#"{"pdfium_release":"chromium/1","entries":[
          {"kind":"pdfium","platform":"a","url":"https://x/y.tgz","member":"lib/l.so","archive_sha256":null,"sha256":"00"},
          {"kind":"pdfium","platform":"b","url":"http://x/y.tgz","member":"lib/l.so","archive_sha256":"aa","sha256":"bb"},
          {"kind":"pdfium","platform":"c","url":"https://x/y.tgz","member":"../l.so","archive_sha256":"aa","sha256":"bb"}]}"#;
        assert!(matches!(
            pin_from(json, "a"),
            Err(ProvisionError::Manifest(_))
        ));
        assert!(matches!(
            pin_from(json, "b"),
            Err(ProvisionError::Manifest(_))
        ));
        assert!(matches!(
            pin_from(json, "c"),
            Err(ProvisionError::Manifest(_))
        ));
        assert!(matches!(
            pin_from("{", "a"),
            Err(ProvisionError::Manifest(_))
        ));
    }

    #[test]
    fn data_root_follows_platform_conventions_and_override() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| PathBuf::from(v))
            }
        };
        assert_eq!(
            data_root_from("linux", env(&[("HOME", "/home/u")])).unwrap(),
            PathBuf::from("/home/u/.local/share/tpe/pdfium")
        );
        assert_eq!(
            data_root_from(
                "linux",
                env(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "/data")])
            )
            .unwrap(),
            PathBuf::from("/data/tpe/pdfium")
        );
        assert_eq!(
            data_root_from("macos", env(&[("HOME", "/Users/u")])).unwrap(),
            PathBuf::from("/Users/u/Library/Application Support/tpe/pdfium")
        );
        assert_eq!(
            data_root_from(
                "windows",
                env(&[("LOCALAPPDATA", "/C/Users/u/AppData/Local")])
            )
            .unwrap(),
            PathBuf::from("/C/Users/u/AppData/Local/tpe/pdfium")
        );
        assert_eq!(
            data_root_from(
                "linux",
                env(&[("HOME", "/home/u"), (ENV_DATA_DIR, "/srv/tpe")])
            )
            .unwrap(),
            PathBuf::from("/srv/tpe/pdfium")
        );
        // Relative overrides are ignored, not trusted.
        assert_eq!(
            data_root_from(
                "linux",
                env(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "rel")])
            )
            .unwrap(),
            PathBuf::from("/home/u/.local/share/tpe/pdfium")
        );
        assert!(matches!(
            data_root_from("linux", env(&[])),
            Err(ProvisionError::NoDataDir(_))
        ));
    }

    #[test]
    fn install_layout_is_release_and_platform_scoped() {
        let pin = test_pin(b"", b"");
        assert_eq!(pin.release_slug(), "chromium-8066");
        assert_eq!(pin.library_file_name(), "libpdfium.so");
        assert_eq!(
            install_path(Path::new("/root"), &pin),
            PathBuf::from("/root/chromium-8066/linux-x64/libpdfium.so")
        );
    }

    #[test]
    fn extract_member_reads_only_the_pinned_member_within_bounds() {
        let archive = gz_tar(&[
            ("lib/other.txt", b"x"),
            ("./lib/libpdfium.so", b"library bytes"),
        ]);
        assert_eq!(
            extract_member(&archive, "lib/libpdfium.so", 1024).unwrap(),
            b"library bytes"
        );
        assert!(matches!(
            extract_member(&archive, "lib/missing.so", 1024),
            Err(ProvisionError::MemberMissing(_))
        ));
        assert!(matches!(
            extract_member(&archive, "lib/libpdfium.so", 4),
            Err(ProvisionError::TooLarge(_))
        ));
        assert!(extract_member(b"not gzip", "lib/libpdfium.so", 1024).is_err());
    }

    #[test]
    fn fetch_into_verifies_installs_atomically_and_refuses_mismatches() {
        let library = b"pretend libpdfium".to_vec();
        let archive = gz_tar(&[("lib/libpdfium.so", &library)]);
        let pin = test_pin(&archive, &library);
        let root = tempfile::tempdir().unwrap();
        let outcome = fetch_into(root.path(), &pin, "test", false, |url| {
            assert_eq!(url, pin.url);
            Ok(archive.clone())
        })
        .unwrap();
        assert_eq!(outcome.action, FetchAction::Installed);
        let path = install_path(root.path(), &pin);
        assert_eq!(outcome.library.path, path);
        assert_eq!(fs::read(&path).unwrap(), library);
        assert_eq!(
            verified_library_at(&path, &pin).unwrap().sha256,
            pin.library_sha256
        );
        // No temporary file is left behind.
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, vec![std::ffi::OsString::from("libpdfium.so")]);

        // A verified copy short-circuits without downloading.
        let outcome = fetch_into(root.path(), &pin, "test", false, |_| {
            panic!("must not download")
        })
        .unwrap();
        assert_eq!(outcome.action, FetchAction::AlreadyInstalled);

        // A tampered archive is refused and the good copy stays.
        let bad = gz_tar(&[("lib/libpdfium.so", b"evil")]);
        let err = fetch_into(root.path(), &pin, "test", true, |_| Ok(bad.clone())).unwrap_err();
        assert!(matches!(err, ProvisionError::ArchiveDigest { .. }), "{err}");
        assert_eq!(fs::read(&path).unwrap(), library);

        // A matching archive whose member hashes differently is refused too.
        let mut wrong_pin = pin.clone();
        wrong_pin.library_sha256 = sha256_hex(b"something else");
        let err = fetch_into(root.path(), &wrong_pin, "test", true, |_| {
            Ok(archive.clone())
        })
        .unwrap_err();
        assert!(matches!(err, ProvisionError::LibraryDigest { .. }), "{err}");
        assert_eq!(fs::read(&path).unwrap(), library);

        // Corruption on disk makes the install invisible.
        fs::write(&path, b"corrupted").unwrap();
        assert!(verified_library_at(&path, &pin).is_none());
        let err = fetch_into(root.path(), &pin, "test", false, |_| {
            Err(ProvisionError::Http("offline".into()))
        })
        .unwrap_err();
        assert!(matches!(err, ProvisionError::Http(_)));
    }

    #[test]
    fn trust_store_comes_only_from_pem_certificates() {
        assert!(trust_store_from_pem(b"").is_none());
        assert!(
            trust_store_from_pem(b"-----BEGIN PRIVATE KEY-----\nAA==\n-----END PRIVATE KEY-----\n")
                .is_none()
        );
        let system = Path::new("/etc/ssl/certs/ca-certificates.crt");
        if let Ok(bundle) = fs::read(system) {
            assert!(
                trust_store_from_pem(&bundle).is_some(),
                "{}",
                system.display()
            );
        } else {
            eprintln!("skipped: no system bundle at {}", system.display());
        }
    }

    #[test]
    fn status_renders_every_field() {
        let status = Status::inspect();
        let text = status.render();
        for key in [
            "platform",
            "data_root",
            "install_path",
            "installed",
            ENV_LIBRARY_PATH,
            "effective",
        ] {
            assert!(text.contains(&format!("{key}: ")), "{text}");
        }
    }
}
