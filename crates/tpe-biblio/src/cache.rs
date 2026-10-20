//! On-disk cache of registry responses under a caller-provided directory.
//!
//! One file per (source, key) holds the status, the body and when it was
//! stored, as JSON. Successful bodies and `404` answers are cached (a DOI
//! that does not exist today rarely exists tomorrow); nothing else is.
//! Entries older than the TTL are ignored. Writes are atomic (temporary
//! file then rename) so a crash never leaves a half-written entry. The
//! cache never leaves the directory it was given and never deletes files.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

/// Default time to live for cached responses: seven days.
// `Duration::from_days` is not a stable const fn on this toolchain.
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// A cached registry response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedResponse {
    /// HTTP status when stored (`200` or `404`).
    pub status: u16,
    /// Response body.
    pub body: String,
    /// Seconds since the Unix epoch when stored.
    pub stored_at: u64,
}

/// The cache: a directory, a TTL and a clock.
#[derive(Clone, Debug)]
pub struct DiskCache {
    dir: PathBuf,
    ttl: Duration,
    now: fn() -> u64,
}

/// FNV-1a 64-bit hash, so a long or unusual key still gives a short file name.
fn fnv1a(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in input.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// A file name for `key`: a readable prefix (ASCII letters, digits, `.` and
/// `-`, at most 40 characters) plus the key's hash, so distinct keys never
/// collide on a sanitised prefix.
pub fn file_name_for(key: &str) -> String {
    let readable: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    format!("{readable}-{:016x}.json", fnv1a(key))
}

/// A directory name for `source` (`crossref`, `datacite`, ...): letters,
/// digits and `-` only.
fn dir_name_for(source: &str) -> String {
    let name: String = source
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() {
        "unknown".to_string()
    } else {
        name
    }
}

impl DiskCache {
    /// A cache under `dir` (created on first write) with the given TTL.
    pub fn new(dir: impl Into<PathBuf>, ttl: Duration) -> Self {
        Self {
            dir: dir.into(),
            ttl,
            now: crate::client::unix_now,
        }
    }

    /// Replace the clock (tests freeze it).
    #[must_use]
    pub fn with_clock(mut self, now: fn() -> u64) -> Self {
        self.now = now;
        self
    }

    /// The cache directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The TTL.
    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Where the entry for (`source`, `key`) lives.
    pub fn path_for(&self, source: &str, key: &str) -> PathBuf {
        self.dir.join(dir_name_for(source)).join(file_name_for(key))
    }

    /// The fresh entry for (`source`, `key`), or `None` when missing, expired
    /// or unreadable. A mismatched stored key (hash collision) is a miss.
    pub fn get(&self, source: &str, key: &str) -> Option<CachedResponse> {
        let text = fs::read_to_string(self.path_for(source, key)).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        if value.get("key").and_then(Value::as_str) != Some(key) {
            return None;
        }
        let stored_at = value.get("stored_at")?.as_u64()?;
        let status = u16::try_from(value.get("status")?.as_u64()?).ok()?;
        let body = value.get("body")?.as_str()?.to_string();
        let now = (self.now)();
        if now.saturating_sub(stored_at) > self.ttl.as_secs() {
            return None;
        }
        Some(CachedResponse {
            status,
            body,
            stored_at,
        })
    }

    /// Store `body` for (`source`, `key`) atomically.
    pub fn put(&self, source: &str, key: &str, status: u16, body: &str) -> io::Result<()> {
        let path = self.path_for(source, key);
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("cache path has no parent"))?;
        fs::create_dir_all(parent)?;
        let entry = json!({
            "source": source,
            "key": key,
            "status": status,
            "stored_at": (self.now)(),
            "body": body,
        });
        let tmp = parent.join(format!(
            ".{}.{}.tmp",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("entry"),
            std::process::id()
        ));
        fs::write(&tmp, serde_json::to_vec(&entry)?)?;
        match fs::rename(&tmp, &path) {
            Ok(()) => Ok(()),
            Err(err) => {
                let _ = fs::remove_file(&tmp);
                Err(err)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frozen() -> u64 {
        1_000_000
    }

    fn later() -> u64 {
        1_000_200
    }

    #[test]
    fn file_names_are_safe_and_distinct() {
        let a = file_name_for("10.1038/nature14539");
        assert!(a.starts_with("10.1038_nature14539-"));
        assert!(
            std::path::Path::new(&a)
                .extension()
                .is_some_and(|e| e == "json")
        );
        assert!(!a.contains('/'));
        assert_ne!(a, file_name_for("10.1038/NATURE14539"));
        let long = file_name_for(&"x".repeat(500));
        assert!(long.len() < 80);
        assert_eq!(dir_name_for("Cross/ref"), "cross_ref");
        assert_eq!(dir_name_for(""), "unknown");
    }

    #[test]
    fn round_trip_expiry_and_misses() {
        let dir = tempfile::tempdir().unwrap();
        let cache =
            DiskCache::new(dir.path().join("sub"), Duration::from_secs(100)).with_clock(frozen);
        assert_eq!(cache.get("crossref", "10.1000/x"), None);
        cache
            .put("crossref", "10.1000/x", 200, "{\"a\":1}")
            .unwrap();
        assert_eq!(
            cache.get("crossref", "10.1000/x"),
            Some(CachedResponse {
                status: 200,
                body: "{\"a\":1}".to_string(),
                stored_at: 1_000_000,
            })
        );
        assert!(
            cache
                .path_for("crossref", "10.1000/x")
                .starts_with(dir.path())
        );
        // Another source or key is a miss.
        assert_eq!(cache.get("datacite", "10.1000/x"), None);
        assert_eq!(cache.get("crossref", "10.1000/y"), None);
        // Expired entries are misses.
        let old = DiskCache::new(dir.path().join("sub"), Duration::from_secs(0)).with_clock(frozen);
        assert!(old.get("crossref", "10.1000/x").is_some());
        let later_cache =
            DiskCache::new(dir.path().join("sub"), Duration::from_secs(100)).with_clock(later);
        assert_eq!(later_cache.get("crossref", "10.1000/x"), None);
        // Garbage files are misses, never errors.
        fs::write(cache.path_for("crossref", "bad"), b"not json").unwrap();
        assert_eq!(cache.get("crossref", "bad"), None);
        // A stored entry for another key under the same file name is a miss.
        let path = cache.path_for("crossref", "10.1000/z");
        fs::write(
            &path,
            serde_json::to_vec(
                &json!({"key": "other", "status": 200, "stored_at": 1_000_000, "body": ""}),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(cache.get("crossref", "10.1000/z"), None);
        // No temporary files are left behind.
        let leftovers: Vec<_> = fs::read_dir(dir.path().join("sub").join("crossref"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }
}
