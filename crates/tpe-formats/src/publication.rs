//! Fixed-name staged publication, adapted from the reviewed image publisher.
//! Complete files, exclusive links and rollback on handled errors; cooperating
//! writers only. Visibility of multiple names and crash recovery are not atomic.

use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use same_file::Handle;
use tempfile::{NamedTempFile, TempDir};

use crate::FormatsError;

pub(crate) fn check_directory(directory: &Path, inputs: &[PathBuf]) -> Result<(), FormatsError> {
    // Resolve existing prefixes (including symlinks) before processing `..`.
    // No directories are created while checking an output that does not exist.
    let mut resolved = if directory.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir()?
    };
    for component in directory.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => {
                resolved.push(other.as_os_str());
                match fs::canonicalize(&resolved) {
                    Ok(path) => resolved = path,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
    }
    for input in inputs {
        if input.is_dir() && resolved.starts_with(fs::canonicalize(input)?) {
            return Err(FormatsError::Invalid(format!(
                "{}: output directory is inside input package {}",
                directory.display(),
                input.display()
            )));
        }
    }
    Ok(())
}

pub(crate) struct OutputSet {
    sources: Vec<(PathBuf, Handle)>,
    packages: Vec<PathBuf>,
    paths: Vec<PathBuf>,
    force: bool,
}

impl OutputSet {
    pub(crate) fn new(
        inputs: &[PathBuf],
        paths: Vec<PathBuf>,
        force: bool,
    ) -> Result<Self, FormatsError> {
        let mut sources = Vec::new();
        let mut packages = Vec::new();
        for path in inputs {
            match fs::metadata(path) {
                Ok(metadata) if metadata.is_dir() => {
                    packages.push(fs::canonicalize(path)?);
                    // A package is an input too: protect its existing files,
                    // including hard links reached through an outside name.
                    for entry in walkdir::WalkDir::new(path) {
                        let entry = entry.map_err(|e| FormatsError::Io(e.into()))?;
                        if entry.path().is_file() {
                            sources.push((
                                entry.path().to_path_buf(),
                                Handle::from_path(entry.path())?,
                            ));
                        }
                    }
                }
                Ok(_) => sources.push((path.clone(), Handle::from_path(path)?)),
                // A missing batch input still gets its own extraction error.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(Self {
            sources,
            packages,
            paths,
            force,
        })
    }

    fn check(&self) -> Result<Option<Vec<bool>>, FormatsError> {
        for (path, source) in &self.sources {
            if Handle::from_path(path)? != *source {
                return Err(FormatsError::Invalid(format!(
                    "{}: source identity changed during publication",
                    path.display()
                )));
            }
        }
        let mut exists = vec![false; self.paths.len()];
        let mut collision = false;
        for (index, path) in self.paths.iter().enumerate() {
            let parent = fs::canonicalize(path.parent().unwrap_or_else(|| Path::new(".")))?;
            if self
                .packages
                .iter()
                .any(|package| parent.starts_with(package))
            {
                return Err(FormatsError::Invalid(format!(
                    "{}: output is inside an input package",
                    path.display()
                )));
            }
            let metadata = match fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            exists[index] = true; // Includes dangling symlinks.
            if !self.force {
                collision = true;
                continue;
            }
            if !metadata.is_file() && !metadata.file_type().is_symlink() {
                return Err(FormatsError::Invalid(format!(
                    "{}: output must be a file or symlink",
                    path.display()
                )));
            }
            match fs::metadata(path) {
                Ok(target) if !target.is_file() => {
                    return Err(FormatsError::Invalid(format!(
                        "{}: output resolves to a non-file",
                        path.display()
                    )));
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            match Handle::from_path(path) {
                Ok(target) if self.sources.iter().any(|(_, source)| *source == target) => {
                    return Err(FormatsError::Invalid(format!(
                        "{}: output aliases an input (even with --force)",
                        path.display()
                    )));
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok((!collision).then_some(exists))
    }

    pub(crate) fn publish(self, bytes: &[&[u8]]) -> Result<bool, FormatsError> {
        self.publish_with(bytes, |from, to| fs::hard_link(from, to))
    }

    fn publish_with(
        self,
        bytes: &[&[u8]],
        mut link: impl FnMut(&Path, &Path) -> io::Result<()>,
    ) -> Result<bool, FormatsError> {
        if self.check()?.is_none() {
            return Ok(false);
        }
        let directory = self.paths[0].parent().unwrap_or_else(|| Path::new("."));
        let mut files = Vec::new();
        for contents in bytes {
            let mut builder = tempfile::Builder::new();
            builder.prefix(".tpe-formats-");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                builder.permissions(fs::Permissions::from_mode(0o666));
            }
            let mut file = builder.tempfile_in(directory)?;
            file.write_all(contents)?;
            file.as_file().sync_all()?;
            files.push(file);
        }
        let Some(exists) = self.check()? else {
            return Ok(false);
        };
        let backups = tempfile::Builder::new()
            .prefix(".tpe-formats-recovery-")
            .tempdir_in(directory)?;
        let mut transaction = Transaction {
            directory: directory.to_path_buf(),
            paths: self.paths.clone(),
            files,
            backups: Some(backups),
            saved: vec![false; self.paths.len()],
            published: vec![false; self.paths.len()],
        };
        let result: io::Result<()> = (|| {
            for (index, exists) in exists.into_iter().enumerate() {
                if exists {
                    // Move the entry, without following a symlink or truncating.
                    fs::rename(&self.paths[index], transaction.backup(index))?;
                    transaction.saved[index] = true;
                }
            }
            for (index, path) in self.paths.iter().enumerate() {
                link(transaction.files[index].path(), path)?;
                transaction.published[index] = true;
            }
            sync_directory(directory)
        })();
        if let Err(error) = result {
            return match transaction.rollback() {
                Ok(()) => Err(error.into()),
                Err(cleanup) => {
                    Err(io::Error::other(format!("{error}; rollback incomplete: {cleanup}")).into())
                }
            };
        }
        transaction.saved.fill(false);
        transaction.published.fill(false);
        Ok(true)
    }
}

struct Transaction {
    directory: PathBuf,
    paths: Vec<PathBuf>,
    files: Vec<NamedTempFile>,
    backups: Option<TempDir>,
    saved: Vec<bool>,
    published: Vec<bool>,
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
            for index in (0..self.paths.len()).rev() {
                let path = &self.paths[index];
                if self.published[index] {
                    let regular = match fs::symlink_metadata(path) {
                        Ok(metadata) => metadata.is_file(),
                        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                        Err(error) => return Err(error),
                    };
                    if regular
                        && Handle::from_path(path)?
                            == Handle::from_file(self.files[index].as_file().try_clone()?)?
                    {
                        fs::remove_file(path)?;
                    }
                    self.published[index] = false;
                }
                if self.saved[index] {
                    match fs::symlink_metadata(path) {
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                        Ok(_) => {
                            return Err(io::Error::other(format!(
                                "{} is occupied",
                                path.display()
                            )));
                        }
                    }
                    // Cooperating writers: absence-check/rename is not a
                    // hostile-directory concurrency guarantee.
                    fs::rename(self.backup(index), path)?;
                    self.saved[index] = false;
                }
            }
            sync_directory(&self.directory)
        })();
        if let Err(error) = result {
            let recovery = self.backups.take().expect("active transaction").keep();
            self.saved.fill(false);
            self.published.fill(false);
            return Err(io::Error::other(format!(
                "{error}; recovery: {} (numbered backups follow output order: {:?})",
                recovery.display(),
                self.paths
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

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_link_failure_restores_outputs_and_cleans_staging() {
        for force in [false, true] {
            for failure in [2, 3] {
                let dir = tempfile::tempdir().unwrap();
                let input = dir.path().join("source.txt");
                fs::write(&input, b"source").unwrap();
                let paths: Vec<_> = ["out.json", "out.txt", "out.preview.pdf"]
                    .iter()
                    .map(|name| dir.path().join(name))
                    .collect();
                if force {
                    for path in &paths {
                        fs::write(path, b"old output").unwrap();
                    }
                }
                let outputs =
                    OutputSet::new(std::slice::from_ref(&input), paths.clone(), force).unwrap();
                let mut calls = 0;
                let error = outputs
                    .publish_with(&[b"json", b"text", b"preview"], |from, to| {
                        calls += 1;
                        if calls == failure {
                            return Err(io::Error::other("injected link failure"));
                        }
                        fs::hard_link(from, to)
                    })
                    .unwrap_err();
                assert!(error.to_string().contains("injected link failure"));
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
                    if force { 4 } else { 1 }
                );
            }
        }
    }
}
