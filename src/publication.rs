//! Complete-file, no-clobber publication with rollback on handled errors.
//! SQLite commits last. This is not a cross-filesystem crash transaction;
//! see docs/PUBLICATION.md for the observable boundary and recovery limits.
//!
//! Outputs may be staged gzip-compressed ([`Compression::Gzip`]): the bytes
//! are encoded while they stream into the staging file, which is then synced
//! and linked exactly like an uncompressed one, under the usual name plus
//! `.gz`. Nothing here touches the SQLite ledger, which stays uncompressed.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

use flate2::write::GzEncoder;
use tempfile::NamedTempFile;

/// A gzip (deflate) compression level between 1 (fastest) and 9 (smallest).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GzipLevel(u8);

impl GzipLevel {
    /// The lowest accepted level.
    pub const MIN: u8 = 1;
    /// The highest accepted level.
    pub const MAX: u8 = 9;
    /// zlib's default trade-off, used when no level is chosen.
    pub const DEFAULT: Self = Self(6);

    /// Validate a level; `0` (store only) and anything above 9 are rejected.
    ///
    /// # Errors
    /// `InvalidInput` when `level` is outside `1..=9`.
    pub fn new(level: u8) -> io::Result<Self> {
        if (Self::MIN..=Self::MAX).contains(&level) {
            Ok(Self(level))
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "gzip level {level} is outside {}..={}",
                    Self::MIN,
                    Self::MAX
                ),
            ))
        }
    }

    /// The numeric level.
    #[must_use]
    pub fn get(self) -> u8 {
        self.0
    }
}

impl Default for GzipLevel {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// How staged bytes are encoded before they are synced and linked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Compression {
    /// Bytes are written as given.
    #[default]
    None,
    /// One RFC 1952 gzip member per file; the final name gains `.gz`.
    Gzip(GzipLevel),
}

impl Compression {
    /// Gzip at `level`, validated by [`GzipLevel::new`].
    ///
    /// # Errors
    /// `InvalidInput` when `level` is outside `1..=9`.
    pub fn gzip(level: u8) -> io::Result<Self> {
        GzipLevel::new(level).map(Self::Gzip)
    }

    /// What is appended to every published name: `""` or `".gz"`.
    #[must_use]
    pub fn suffix(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Gzip(_) => ".gz",
        }
    }

    /// The gzip level, or `None` when bytes are written as given.
    #[must_use]
    pub fn gzip_level(self) -> Option<GzipLevel> {
        match self {
            Self::None => None,
            Self::Gzip(level) => Some(level),
        }
    }
}

/// `0` means [`Compression::None`]; `1..=9` is a gzip level.
static PROCESS_DEFAULT: AtomicU8 = AtomicU8::new(0);

/// Set what [`StagedOutputs::stage`] uses for the rest of this process. The
/// `tpe` publish worker adopts its controller's `--gzip` choice this way;
/// library callers that want an explicit choice use [`StagedOutputs::stage_with`].
pub fn set_process_default(compression: Compression) {
    let level = compression.gzip_level().map_or(0, GzipLevel::get);
    PROCESS_DEFAULT.store(level, Ordering::Release);
}

/// The compression [`StagedOutputs::stage`] applies: [`Compression::None`]
/// unless [`set_process_default`] changed it.
#[must_use]
pub fn process_default() -> Compression {
    match PROCESS_DEFAULT.load(Ordering::Acquire) {
        0 => Compression::None,
        level => Compression::Gzip(GzipLevel(level)),
    }
}

/// Complete staged files, published without overwriting existing paths.
pub struct StagedOutputs {
    directory: PathBuf,
    base: String,
    files: Vec<(String, NamedTempFile)>,
    linked: Vec<(usize, PathBuf)>,
}

impl StagedOutputs {
    /// Write and sync each temporary sibling before making any final name visible.
    ///
    /// Bytes are encoded with [`process_default`]; see [`Self::stage_with`].
    pub fn stage(source: &Path, files: &[(&str, Vec<u8>)]) -> io::Result<Self> {
        Self::stage_with(source, files, process_default())
    }

    /// As [`Self::stage`], with an explicit [`Compression`]. Each `suffix`
    /// (such as `.json`) gains [`Compression::suffix`] in every published name.
    pub fn stage_with(
        source: &Path,
        files: &[(&str, Vec<u8>)],
        compression: Compression,
    ) -> io::Result<Self> {
        let mut readers: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(suffix, bytes)| (*suffix, bytes.as_slice()))
            .collect();
        let mut streams: Vec<(&str, &mut dyn Read)> = readers
            .iter_mut()
            .map(|(suffix, bytes)| (*suffix, bytes as &mut dyn Read))
            .collect();
        Self::stage_streams(source, &mut streams, compression)
    }

    /// Stream each reader into its staging file, encoding on the way, and
    /// sync it before making any final name visible. No output is held in
    /// memory as a whole: the gzip encoder keeps a small fixed buffer.
    pub fn stage_streams(
        source: &Path,
        files: &mut [(&str, &mut dyn Read)],
        compression: Compression,
    ) -> io::Result<Self> {
        let directory = source
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let mut staged = Self {
            directory,
            base: source
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            files: Vec::new(),
            linked: Vec::new(),
        };
        for (suffix, bytes) in files {
            let mut builder = tempfile::Builder::new();
            builder.prefix(".pdftextract-");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // Applied by open(2), so the caller's umask can only narrow
                // permissions. Hard-link publication retains this mode. Do
                // not chmod afterwards or read/change the process-wide umask.
                builder.permissions(fs::Permissions::from_mode(0o666));
            }
            let file = builder.tempfile_in(&staged.directory)?;
            encode(&mut **bytes, file.as_file(), compression)?;
            file.as_file().sync_all()?;
            staged
                .files
                .push((format!("{suffix}{}", compression.suffix()), file));
        }
        Ok(staged)
    }

    /// Publish complete files, commit last, and roll back on a handled error.
    pub fn publish_then(
        &mut self,
        commit: impl FnOnce() -> Result<(), String>,
    ) -> Result<Vec<PathBuf>, String> {
        self.publish_then_with_paths(|_| commit())
    }

    /// As [`Self::publish_then`], with the chosen no-clobber paths available
    /// before commit, so receipts can be serialized while rollback is possible.
    pub fn publish_then_with_paths(
        &mut self,
        commit: impl FnOnce(&[PathBuf]) -> Result<(), String>,
    ) -> Result<Vec<PathBuf>, String> {
        self.publish_with_paths(|from, to| fs::hard_link(from, to), commit)
    }

    #[cfg(test)]
    fn publish_with(
        &mut self,
        link: impl FnMut(&Path, &Path) -> io::Result<()>,
        commit: impl FnOnce() -> Result<(), String>,
    ) -> Result<Vec<PathBuf>, String> {
        self.publish_with_paths(link, |_| commit())
    }

    fn publish_with_paths(
        &mut self,
        mut link: impl FnMut(&Path, &Path) -> io::Result<()>,
        commit: impl FnOnce(&[PathBuf]) -> Result<(), String>,
    ) -> Result<Vec<PathBuf>, String> {
        let mut generation = 1u64;
        loop {
            let mut collision = false;
            for (index, (suffix, staged)) in self.files.iter().enumerate() {
                let name = if generation == 1 {
                    format!("{}{suffix}", self.base)
                } else {
                    format!("{} {generation}{suffix}", self.base)
                };
                let target = self.directory.join(name);
                if let Err(error) = link(staged.path(), &target) {
                    let cleanup = self.rollback();
                    if let Err(cleanup) = cleanup {
                        return Err(format!(
                            "publishing {}: {error}; {cleanup}",
                            target.display()
                        ));
                    }
                    if error.kind() == io::ErrorKind::AlreadyExists {
                        collision = true;
                        break;
                    }
                    return Err(format!("publishing {}: {error}", target.display()));
                }
                self.linked.push((index, target));
            }
            if !collision {
                break;
            }
            generation = generation
                .checked_add(1)
                .ok_or("output generation exhausted")?;
        }
        let paths: Vec<PathBuf> = self.linked.iter().map(|(_, path)| path.clone()).collect();
        // Sync the directory while rollback is still possible. There is no
        // fallback that exposes a partially written final file.
        let result = sync_directory(&self.directory)
            .map_err(|e| format!("syncing outputs: {e}"))
            .and_then(|()| commit(&paths));
        if let Err(error) = result {
            return match self.rollback() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(format!("{error}; {cleanup}")),
            };
        }
        self.linked.clear(); // The ledger commit succeeded; never unlink these now.
        Ok(paths)
    }

    fn rollback(&mut self) -> Result<(), String> {
        let mut failures = Vec::new();
        self.linked.retain(|(index, path)| {
            let staged = self.files[*index].1.as_file();
            let result = match fs::symlink_metadata(path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
                Ok(metadata) => staged.metadata().and_then(|ours| {
                    if same_file(&ours, &metadata)? {
                        fs::remove_file(path)
                    } else {
                        // Another owner replaced our name; leave their file alone.
                        Ok(())
                    }
                }),
            };
            if let Err(error) = result {
                failures.push(format!("{}: {error}", path.display()));
                true
            } else {
                false
            }
        });
        let sync = sync_directory(&self.directory);
        if !failures.is_empty() {
            return Err(format!(
                "cleanup incomplete; retained output paths: {}",
                failures.join(", ")
            ));
        }
        sync.map_err(|e| format!("cleanup directory sync: {e}"))
    }
}

impl Drop for StagedOutputs {
    fn drop(&mut self) {
        if !self.linked.is_empty() {
            let _ = self.rollback();
        }
        // NamedTempFile removes each staging name, including on unwind.
    }
}

#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)] // The non-Unix implementation can fail.
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(a.dev() == b.dev() && a.ino() == b.ino())
}

#[cfg(not(unix))]
fn same_file(_a: &fs::Metadata, _b: &fs::Metadata) -> io::Result<bool> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "output rollback requires file identity support",
    ))
}

fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

/// Copy `reader` into `file`, gzip-encoding it when asked. Both paths stream
/// through `io::copy`'s fixed buffer; `finish` writes the gzip trailer
/// (CRC-32 and length) so a short file is detectable by any decoder.
fn encode(reader: &mut dyn Read, mut file: &fs::File, compression: Compression) -> io::Result<()> {
    match compression {
        Compression::None => {
            io::copy(reader, &mut file)?;
        }
        Compression::Gzip(level) => {
            let mut encoder =
                GzEncoder::new(file, flate2::Compression::new(u32::from(level.get())));
            io::copy(reader, &mut encoder)?;
            encoder.finish()?;
        }
    }
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(source: &Path) -> StagedOutputs {
        StagedOutputs::stage(
            source,
            &[
                (".references.json", b"json".to_vec()),
                (".references.txt", b"text".to_vec()),
            ],
        )
        .unwrap()
    }

    #[test]
    #[cfg(unix)]
    fn published_outputs_respect_caller_umask() {
        // Only each child shell changes its umask. Other tests and worker
        // threads keep their inherited creation policy.
        for (mask, expected) in [
            ("000", "666"),
            ("002", "664"),
            ("022", "644"),
            ("027", "640"),
            ("077", "600"),
            ("777", "000"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let output = std::process::Command::new("sh")
                .args(["-c", "umask \"$1\"; exec \"$2\" --exact publication::tests::publication_permissions_child --nocapture", "publication-test", mask])
                .arg(std::env::current_exe().unwrap())
                .env("TPE_TEST_PUBLICATION_DIRECTORY", directory.path())
                .env("TPE_TEST_PUBLICATION_MODE", expected)
                .output().unwrap();
            assert!(
                output.status.success(),
                "umask {mask}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn publication_permissions_child() {
        use std::os::unix::fs::PermissionsExt;
        let Some(directory) = std::env::var_os("TPE_TEST_PUBLICATION_DIRECTORY") else {
            return;
        };
        let expected =
            u32::from_str_radix(&std::env::var("TPE_TEST_PUBLICATION_MODE").unwrap(), 8).unwrap();
        let source = Path::new(&directory).join("paper.pdf");
        let mut outputs = StagedOutputs::stage(
            &source,
            &[
                (".txt", b"full text".to_vec()),
                (".references.txt", b"references".to_vec()),
                (".references.json", b"[]".to_vec()),
            ],
        )
        .unwrap();
        for (_, staged) in &outputs.files {
            assert_eq!(
                staged.as_file().metadata().unwrap().permissions().mode() & 0o777,
                expected
            );
        }
        let published = outputs.publish_then(|| Ok(())).unwrap();
        assert_eq!(published.len(), 3);
        for path in published {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                expected
            );
        }
    }

    #[test]
    fn failure_after_first_link_removes_it_and_does_not_commit() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("paper.pdf");
        let mut staged = pair(&source);
        let mut calls = 0;
        let result = staged.publish_with(
            |from, to| {
                calls += 1;
                if calls == 2 {
                    return Err(io::Error::other("injected disk failure"));
                }
                fs::hard_link(from, to)
            },
            || panic!("must not commit"),
        );
        assert!(result.unwrap_err().contains("injected disk failure"));
        drop(staged);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn failed_commit_removes_all_published_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut staged = pair(&dir.path().join("paper.pdf"));
        let result = staged.publish_then(|| Err("injected commit failure".into()));
        assert_eq!(result.unwrap_err(), "injected commit failure");
        drop(staged);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn collision_during_second_link_retries_both_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("paper.pdf");
        let collision = dir.path().join("paper.references.txt");
        let mut staged = pair(&source);
        let mut calls = 0;
        let paths = staged
            .publish_with(
                |from, to| {
                    calls += 1;
                    if calls == 2 {
                        fs::write(&collision, b"other owner")?;
                    }
                    fs::hard_link(from, to)
                },
                || Ok(()),
            )
            .unwrap();
        assert_eq!(
            paths,
            [
                dir.path().join("paper 2.references.json"),
                dir.path().join("paper 2.references.txt")
            ]
        );
        assert_eq!(fs::read(&collision).unwrap(), b"other owner");
        assert!(!dir.path().join("paper.references.json").exists());
        assert_eq!(fs::read(&paths[0]).unwrap(), b"json");
        assert_eq!(fs::read(&paths[1]).unwrap(), b"text");
    }

    #[test]
    fn unsupported_links_fail_without_writing_a_partial_final_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut staged = pair(&dir.path().join("paper.pdf"));
        assert!(
            staged
                .publish_with(
                    |_, _| Err(io::Error::from(io::ErrorKind::Unsupported)),
                    || panic!("must not commit")
                )
                .is_err()
        );
        drop(staged);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn concurrent_publishers_get_distinct_complete_pairs() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("paper.pdf");
        let barrier = std::sync::Barrier::new(8);
        let outputs = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        let mut staged = pair(&source);
                        barrier.wait();
                        staged.publish_then(|| Ok(())).unwrap()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        let distinct: std::collections::HashSet<_> = outputs.iter().collect();
        assert_eq!(distinct.len(), 16);
        for pair in outputs.as_chunks::<2>().0 {
            assert_eq!(fs::read(&pair[0]).unwrap(), b"json");
            assert_eq!(fs::read(&pair[1]).unwrap(), b"text");
        }
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 16);
    }

    #[test]
    fn rollback_does_not_remove_a_replacement_from_another_owner() {
        let dir = tempfile::tempdir().unwrap();
        let mut staged = pair(&dir.path().join("paper.pdf"));
        let target = dir.path().join("paper.references.json");
        let result = staged.publish_then(|| {
            fs::remove_file(&target).unwrap();
            fs::write(&target, b"replacement").unwrap();
            Err("commit failed".into())
        });
        assert!(result.is_err());
        assert_eq!(fs::read(target).unwrap(), b"replacement");
    }
}
