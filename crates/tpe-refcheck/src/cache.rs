//! On-disk cache of registry responses, one JSON file per request key in a
//! caller-given directory. Only final answers are stored (`200`, `404`);
//! rate limits and server errors are never cached. Writes go to a temporary
//! sibling first and are renamed into place, so a reader never sees a
//! partial file and concurrent writers of the same key both leave a whole one.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// A stored response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cached {
    /// The request key (`doi:<doi>` or `query:<text>`).
    pub key: String,
    /// The URL that was fetched (without any `mailto`).
    pub url: String,
    pub status: u16,
    pub body: String,
    /// Unix seconds at which the response was stored.
    pub fetched_at: u64,
}

/// The cache directory.
#[derive(Clone, Debug)]
pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    /// Open (creating it when needed) the cache directory `dir`.
    ///
    /// # Errors
    /// When the directory cannot be created.
    pub fn open(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    /// The directory this cache lives in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The key for a DOI lookup (`doi:<lower-case doi>`).
    pub fn doi_key(doi: &str) -> String {
        format!("doi:{}", doi.trim().to_lowercase())
    }

    /// The key for a bibliographic query (`query:<rows>:<text>`).
    pub fn query_key(text: &str, rows: u32) -> String {
        format!("query:{rows}:{}", text.trim())
    }

    /// The file a key is stored in: the SHA-256 of the key, so any text is
    /// a safe file name.
    pub fn path_for(&self, key: &str) -> PathBuf {
        self.dir
            .join(format!("{}.json", tpe::schema::sha256_hex(key.as_bytes())))
    }

    /// The stored response for `key`, if any. Unreadable or foreign files
    /// count as misses.
    pub fn get(&self, key: &str) -> Option<Cached> {
        let text = fs::read_to_string(self.path_for(key)).ok()?;
        let cached: Cached = serde_json::from_str(&text).ok()?;
        (cached.key == key).then_some(cached)
    }

    /// Store a response for `key`.
    ///
    /// # Errors
    /// When the file cannot be written or renamed into place.
    pub fn put(&self, key: &str, url: &str, status: u16, body: &str) -> io::Result<()> {
        let cached = Cached {
            key: key.to_string(),
            url: url.to_string(),
            status,
            body: body.to_string(),
            fetched_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        };
        let text = serde_json::to_string(&cached).map_err(io::Error::other)?;
        let target = self.path_for(key);
        let temp = self.dir.join(format!(
            ".{}.{}.tmp",
            target
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("entry"),
            std::process::id()
        ));
        fs::write(&temp, text)?;
        match fs::rename(&temp, &target) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = fs::remove_file(&temp);
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_ignores_foreign_files() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(&dir.path().join("nested")).unwrap();
        let key = Cache::doi_key("10.1038/Nature14539");
        assert_eq!(key, "doi:10.1038/nature14539");
        assert!(cache.get(&key).is_none());
        cache
            .put(&key, "https://doi.org/10.1038/nature14539", 200, "{}")
            .unwrap();
        let hit = cache.get(&key).unwrap();
        assert_eq!(hit.status, 200);
        assert_eq!(hit.body, "{}");
        assert!(hit.fetched_at > 0);
        // A file under the right name but for another key is a miss.
        let other = Cache::query_key("x", 5);
        fs::write(cache.path_for(&other), serde_json::to_string(&hit).unwrap()).unwrap();
        assert!(cache.get(&other).is_none());
        fs::write(cache.path_for(&other), "not json").unwrap();
        assert!(cache.get(&other).is_none());
        assert!(
            !dir.path()
                .join("nested")
                .read_dir()
                .unwrap()
                .any(|e| { e.unwrap().file_name().to_string_lossy().ends_with(".tmp") })
        );
    }
}
