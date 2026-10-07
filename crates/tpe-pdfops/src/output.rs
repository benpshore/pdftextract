//! Writing outputs without ever touching an input.
//!
//! Every operation ends here: the document is serialised to memory, parsed
//! again as a self-check, and then written to a path that is not an input and
//! that does not exist yet (unless `force` is set).

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use lopdf::Document;

use crate::error::{PdfOpsError, Result};

/// Where an operation writes and whether an existing file may be replaced.
#[derive(Debug, Clone)]
pub struct Output {
    /// The output file (or directory for `paginate`).
    pub path: PathBuf,
    /// Replace an existing output. Inputs are refused even with `force`.
    pub force: bool,
}

impl Output {
    /// An output at `path` that refuses to replace an existing file.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            force: false,
        }
    }

    /// Allow replacing an existing output file.
    #[must_use]
    pub fn force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }
}

/// Refuse an output that names one of the inputs, including hard links.
pub fn ensure_not_input(output: &Path, inputs: &[&Path]) -> Result<()> {
    let handles = input_handles(inputs)?;
    check_output(output, inputs, &handles)
}

fn input_handles(inputs: &[&Path]) -> Result<Vec<same_file::Handle>> {
    inputs
        .iter()
        .map(|input| same_file::Handle::from_path(input).map_err(|e| PdfOpsError::io(input, e)))
        .collect()
}

fn check_output(output: &Path, inputs: &[&Path], handles: &[same_file::Handle]) -> Result<()> {
    let output_abs = absolute(output);
    if inputs.iter().any(|input| output_abs == absolute(input)) {
        return Err(PdfOpsError::OutputIsInput(output.to_path_buf()));
    }
    match same_file::Handle::from_path(output) {
        Ok(handle) if handles.contains(&handle) => {
            Err(PdfOpsError::OutputIsInput(output.to_path_buf()))
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(PdfOpsError::io(output, e)),
    }
}

fn absolute(path: &Path) -> PathBuf {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()))
}

/// Stage a complete new inode, sync it, then publish it. Existing files are
/// never opened for writing, even with `force`. For input alias rejection,
/// use [`write_bytes_with_inputs`].
pub fn write_bytes(output: &Output, bytes: &[u8]) -> Result<()> {
    write_bytes_with_inputs(output, bytes, &[])
}

/// Write with input identity checks before staging and before publication.
pub fn write_bytes_with_inputs(output: &Output, bytes: &[u8], inputs: &[&Path]) -> Result<()> {
    stage_and_publish(output, inputs, |file| {
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()
    })
}

// The callback also permits deterministic handled-I/O and race regressions.
fn stage_and_publish(
    output: &Output,
    inputs: &[&Path],
    stage: impl FnOnce(&mut File) -> std::io::Result<()>,
) -> Result<()> {
    let path = &output.path;
    // Hold handles so a replacement of an input pathname does not discard
    // the identity we originally protected.
    let handles = input_handles(inputs)?;
    check_output(path, inputs, &handles)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| PdfOpsError::io(path, e))?;
    stage(temp.as_file_mut()).map_err(|e| PdfOpsError::io(path, e))?;
    check_output(path, inputs, &handles)?;
    ensure_not_input(path, inputs)?;
    let result = if output.force {
        // Rename replaces the directory entry; it never follows a destination
        // symlink or writes through a hard link introduced after the check.
        temp.persist(path)
    } else {
        temp.persist_noclobber(path)
    };
    match result {
        Ok(_) => Ok(()),
        Err(e) if !output.force && e.error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(PdfOpsError::OutputExists(path.clone()))
        }
        Err(e) => Err(PdfOpsError::io(path, e.error)),
    }
}

/// Serialise `doc`, parse the bytes again (page count must survive), then
/// write them with [`write_bytes`]. Returns the number of pages written.
pub fn save_document(doc: &mut Document, output: &Output) -> Result<usize> {
    save_document_with_inputs(doc, output, &[])
}

/// Serialise and self-check, protecting the supplied input identities.
pub fn save_document_with_inputs(
    doc: &mut Document,
    output: &Output,
    inputs: &[&Path],
) -> Result<usize> {
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes)
        .map_err(|e| PdfOpsError::io(&output.path, e))?;
    let expected = doc.get_pages().len();
    let reparsed = Document::load_mem(&bytes).map_err(|e| PdfOpsError::pdf(&output.path, e))?;
    let found = reparsed.get_pages().len();
    if found != expected {
        return Err(PdfOpsError::Invalid(format!(
            "self-check failed: serialised document has {found} pages, expected {expected}"
        )));
    }
    write_bytes_with_inputs(output, &bytes, inputs)?;
    Ok(found)
}

/// Load a document read-only. An encrypted document is refused with a hint to
/// run `unlock` first, unless `allow_encrypted` is set.
pub fn load_input(path: &Path, allow_encrypted: bool) -> Result<Document> {
    let bytes = std::fs::read(path).map_err(|e| PdfOpsError::io(path, e))?;
    let doc = Document::load_mem(&bytes).map_err(|e| PdfOpsError::pdf(path, e))?;
    if doc.is_encrypted() && !allow_encrypted {
        return Err(PdfOpsError::Invalid(format!(
            "{}: document is encrypted; run `tpe-pdfops unlock --password ...` first",
            path.display()
        )));
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_link_alias_is_refused_with_force() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.pdf");
        let alias = dir.path().join("alias.pdf");
        std::fs::write(&input, b"original").unwrap();
        std::fs::hard_link(&input, &alias).unwrap();
        assert!(matches!(
            write_bytes_with_inputs(&Output::new(&alias).force(true), b"new", &[&input]),
            Err(PdfOpsError::OutputIsInput(_))
        ));
        assert_eq!(std::fs::read(&input).unwrap(), b"original");
        assert_eq!(std::fs::read(&alias).unwrap(), b"original");
    }

    #[test]
    fn replacement_during_staging_is_rechecked() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.pdf");
        let output = dir.path().join("output.pdf");
        std::fs::write(&input, b"original").unwrap();
        std::fs::write(&output, b"old output").unwrap();
        let result = stage_and_publish(&Output::new(&output).force(true), &[&input], |file| {
            file.write_all(b"complete")?;
            std::fs::remove_file(&output)?;
            std::fs::hard_link(&input, &output)
        });
        assert!(matches!(result, Err(PdfOpsError::OutputIsInput(_))));
        assert_eq!(std::fs::read(&input).unwrap(), b"original");
        assert_eq!(std::fs::read(&output).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn retained_input_identity_survives_input_path_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.pdf");
        let saved = dir.path().join("saved.pdf");
        let output = dir.path().join("output.pdf");
        std::fs::write(&input, b"original").unwrap();
        let result = stage_and_publish(&Output::new(&output).force(true), &[&input], |file| {
            file.write_all(b"complete")?;
            std::fs::rename(&input, &saved)?;
            std::fs::write(&input, b"replacement")?;
            std::fs::hard_link(&saved, &output)
        });
        assert!(matches!(result, Err(PdfOpsError::OutputIsInput(_))));
        assert_eq!(std::fs::read(&saved).unwrap(), b"original");
        assert_eq!(std::fs::read(&input).unwrap(), b"replacement");
    }

    #[test]
    fn staged_publication_never_writes_through_a_destination_link() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.pdf");
        let output = dir.path().join("output.pdf");
        std::fs::write(&input, b"original").unwrap();
        std::fs::hard_link(&input, &output).unwrap();
        // The low-level writer has no input list. Even if a link appears after
        // the last identity check, replacing it cannot truncate its target.
        write_bytes(&Output::new(&output).force(true), b"complete").unwrap();
        assert_eq!(std::fs::read(&input).unwrap(), b"original");
        assert_eq!(std::fs::read(&output).unwrap(), b"complete");
        assert!(!same_file::is_same_file(&input, &output).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_publication_never_follows_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.pdf");
        let output = dir.path().join("output.pdf");
        std::fs::write(&input, b"original").unwrap();
        std::os::unix::fs::symlink(&input, &output).unwrap();
        assert!(ensure_not_input(&output, &[&input]).is_err());
        write_bytes(&Output::new(&output).force(true), b"complete").unwrap();
        assert_eq!(std::fs::read(&input).unwrap(), b"original");
        assert!(
            !std::fs::symlink_metadata(&output)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn handled_partial_write_failure_leaves_old_or_absent_output() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("output.pdf");
        for existing in [false, true] {
            if existing {
                std::fs::write(&output, b"old output").unwrap();
            }
            let result = stage_and_publish(&Output::new(&output).force(true), &[], |file| {
                file.write_all(b"partial")?;
                Err(std::io::Error::other("injected write/flush/sync failure"))
            });
            assert!(matches!(result, Err(PdfOpsError::Io { .. })));
            if existing {
                assert_eq!(std::fs::read(&output).unwrap(), b"old output");
            } else {
                assert!(!output.exists());
            }
            assert_eq!(
                std::fs::read_dir(dir.path()).unwrap().count(),
                usize::from(existing)
            );
        }
    }

    #[test]
    fn no_force_race_and_publication_failure_preserve_destination() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("output.pdf");
        let result = stage_and_publish(&Output::new(&output), &[], |file| {
            file.write_all(b"complete")?;
            std::fs::write(&output, b"racing output")
        });
        assert!(matches!(result, Err(PdfOpsError::OutputExists(_))));
        assert_eq!(std::fs::read(&output).unwrap(), b"racing output");
        std::fs::remove_file(&output).unwrap();
        std::fs::create_dir(&output).unwrap();
        std::fs::write(output.join("sentinel"), b"keep").unwrap();
        assert!(write_bytes(&Output::new(&output).force(true), b"new").is_err());
        assert_eq!(std::fs::read(output.join("sentinel")).unwrap(), b"keep");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
