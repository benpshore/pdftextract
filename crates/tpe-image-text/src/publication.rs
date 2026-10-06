//! Fixed-name adaptation of the root staged publisher: complete files,
//! exclusive links and rollback on handled errors. Unlike that publisher,
//! this CLI must preserve fixed names and an explicit force operation.
//! Cooperating writers only; not a two-file crash transaction.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use same_file::Handle;
use tempfile::{NamedTempFile, TempDir};

use crate::ImageTextError;

pub(crate) struct OutputPair {
    input: PathBuf,
    source: Handle,
    batch_sources: Vec<Handle>,
    paths: [PathBuf; 2],
    force: bool,
}

impl OutputPair {
    pub(crate) fn new(
        input: &Path,
        paths: [&Path; 2],
        force: bool,
        inputs: &[PathBuf],
    ) -> Result<Self, ImageTextError> {
        let pair = Self {
            input: input.to_path_buf(),
            source: Handle::from_path(input).map_err(|e| io_error(input, e))?,
            // Unreadable/missing inputs still receive their own normal per-file
            // error. An unreadable output also fails its identity check below.
            batch_sources: inputs
                .iter()
                .filter_map(|path| Handle::from_path(path).ok())
                .collect(),
            paths: paths.map(Path::to_path_buf),
            force,
        };
        pair.check()?;
        Ok(pair)
    }

    fn check(&self) -> Result<[bool; 2], ImageTextError> {
        if Handle::from_path(&self.input).map_err(|e| io_error(&self.input, e))? != self.source {
            return Err(ImageTextError::InvalidInput(format!(
                "{}: source identity changed during processing",
                self.input.display()
            )));
        }
        let mut exists = [false; 2];
        for (index, path) in self.paths.iter().enumerate() {
            let metadata = match fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(io_error(path, e)),
            };
            exists[index] = true; // Includes dangling symlinks.
            if !self.force {
                return Err(ImageTextError::OutputExists(path.display().to_string()));
            }
            if !metadata.is_file() && !metadata.file_type().is_symlink() {
                return Err(ImageTextError::InvalidInput(format!(
                    "{}: output must be a file or symlink",
                    path.display()
                )));
            }
            match fs::metadata(path) {
                Ok(target) if !target.is_file() => {
                    return Err(ImageTextError::InvalidInput(format!(
                        "{}: output resolves to a non-file",
                        path.display()
                    )));
                }
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(io_error(path, e)),
            }
            match Handle::from_path(path) {
                Ok(target) if target == self.source || self.batch_sources.contains(&target) => {
                    return Err(ImageTextError::InvalidInput(format!(
                        "{}: output aliases an input while processing {} (even with --force)",
                        path.display(),
                        self.input.display()
                    )));
                }
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(io_error(path, e)),
            }
        }
        Ok(exists)
    }

    pub(crate) fn publish(self, bytes: [&[u8]; 2]) -> Result<(), ImageTextError> {
        self.publish_with(bytes, |from, to| fs::hard_link(from, to))
    }

    fn publish_with(
        self,
        bytes: [&[u8]; 2],
        mut link: impl FnMut(&Path, &Path) -> io::Result<()>,
    ) -> Result<(), ImageTextError> {
        let directory = self.paths[0].parent().unwrap_or_else(|| Path::new("."));
        // Every byte, including serialized JSON, is staged before mutation.
        let mut files = Vec::new();
        for contents in bytes {
            let mut builder = tempfile::Builder::new();
            builder.prefix(".tpe-image-text-");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                builder.permissions(fs::Permissions::from_mode(0o666));
            }
            let mut file = builder
                .tempfile_in(directory)
                .map_err(|e| io_error(directory, e))?;
            file.write_all(contents)
                .map_err(|e| io_error(directory, e))?;
            file.as_file()
                .sync_all()
                .map_err(|e| io_error(directory, e))?;
            files.push(file);
        }
        let exists = self.check()?; // Recheck after OCR and staging.
        let backups = tempfile::Builder::new()
            .prefix(".tpe-image-text-recovery-")
            .tempdir_in(directory)
            .map_err(|e| io_error(directory, e))?;
        let mut transaction = Transaction {
            directory: directory.to_path_buf(),
            paths: self.paths.clone(),
            files,
            backups: Some(backups),
            saved: [false; 2],
            published: [false; 2],
        };
        let result = (|| {
            for (index, exists) in exists.into_iter().enumerate() {
                if exists {
                    // Move the entry itself; never open an output for writing or
                    // follow its symlink. Keep it until both new files succeed.
                    fs::rename(&self.paths[index], transaction.backup(index))
                        .map_err(|e| io_error(&self.paths[index], e))?;
                    transaction.saved[index] = true;
                }
            }
            for (index, path) in self.paths.iter().enumerate() {
                link(transaction.files[index].path(), path).map_err(|e| io_error(path, e))?;
                transaction.published[index] = true;
            }
            sync_directory(directory).map_err(|e| io_error(directory, e))
        })();
        if let Err(error) = result {
            return match transaction.rollback() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(ImageTextError::Io(
                    format!("{error}; rollback incomplete: {cleanup}"),
                    io::Error::other("inspect retained recovery directory"),
                )),
            };
        }
        transaction.saved = [false; 2];
        transaction.published = [false; 2];
        Ok(())
    }
}

struct Transaction {
    directory: PathBuf,
    paths: [PathBuf; 2],
    files: Vec<NamedTempFile>,
    backups: Option<TempDir>,
    saved: [bool; 2],
    published: [bool; 2],
}

impl Transaction {
    fn backup(&self, index: usize) -> PathBuf {
        self.backups
            .as_ref()
            .expect("active transaction")
            .path()
            .join(index.to_string())
    }

    fn rollback(&mut self) -> io::Result<()> {
        let result = (|| {
            for index in (0..2).rev() {
                let path = &self.paths[index];
                if self.published[index] {
                    // Do not unlink a replacement installed by another owner.
                    if fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
                        && Handle::from_path(path)?
                            == Handle::from_file(self.files[index].as_file().try_clone()?)?
                    {
                        fs::remove_file(path)?;
                    }
                    self.published[index] = false;
                }
                if self.saved[index] {
                    match fs::symlink_metadata(path) {
                        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e),
                        Ok(_) => {
                            return Err(io::Error::other(format!(
                                "{} is occupied",
                                path.display()
                            )));
                        }
                    }
                    // Cooperating-writer boundary: the absence check and rename
                    // are not a hostile-directory concurrency guarantee.
                    fs::rename(self.backup(index), path)?;
                    self.saved[index] = false;
                }
            }
            sync_directory(&self.directory)
        })();
        if let Err(error) = result {
            let recovery = self.backups.take().expect("active transaction").keep();
            self.saved = [false; 2];
            self.published = [false; 2];
            return Err(io::Error::other(format!(
                "{error}; recovery: {} (0=text, 1=JSON)",
                recovery.display()
            )));
        }
        Ok(())
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        if self.saved.iter().any(|saved| *saved)
            || self.published.iter().any(|published| *published)
        {
            let _ = self.rollback();
        }
    }
}

fn io_error(path: &Path, error: io::Error) -> ImageTextError {
    if error.kind() == io::ErrorKind::AlreadyExists {
        ImageTextError::OutputExists(path.display().to_string())
    } else {
        ImageTextError::Io(path.display().to_string(), error)
    }
}

fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_link_failure_restores_old_pair_and_cleans_staging() {
        for force in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let input = dir.path().join("input.png");
            fs::write(&input, b"source").unwrap();
            let paths = [dir.path().join("input.txt"), dir.path().join("input.json")];
            if force {
                for path in &paths {
                    fs::write(path, b"old output").unwrap();
                }
            }
            let pair = OutputPair::new(&input, [&paths[0], &paths[1]], force, &[]).unwrap();
            let mut calls = 0;
            let error = pair
                .publish_with([b"new text", b"new JSON"], |from, to| {
                    calls += 1;
                    if calls == 2 {
                        return Err(io::Error::other("injected second-link failure"));
                    }
                    fs::hard_link(from, to)
                })
                .unwrap_err();
            assert!(error.to_string().contains("injected second-link failure"));
            assert_eq!(fs::read(&input).unwrap(), b"source");
            for path in &paths {
                if force {
                    assert_eq!(fs::read(path).unwrap(), b"old output");
                } else {
                    assert!(!path.exists());
                }
            }
            assert_eq!(
                fs::read_dir(dir.path()).unwrap().count(),
                if force { 3 } else { 1 }
            );
        }
    }
}
