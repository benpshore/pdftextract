//! Regression cases using only the original #252 public API, so the same
//! fixtures can be run against the reviewed base as well as the correction.

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::tempdir;
use tpe_identify::rename::{apply, read_manifest, undo};
use tpe_identify::{LoadOptions, ScanOptions, scan};

const SOURCE: &[u8] = b"%PDF-1.4\nsynthetic immutable source canary\n";

fn report_must_refuse(source: &Path, output: &Path) {
    let result = Command::new(env!("CARGO_BIN_EXE_tpe-identify"))
        .args(["scan", "--no-extract", "--json"])
        .arg(output)
        .arg(source)
        .output()
        .unwrap();
    assert_eq!(fs::read(source).unwrap(), SOURCE, "source bytes changed");
    assert_eq!(fs::read(output).unwrap(), SOURCE, "alias bytes changed");
    assert!(
        !result.status.success(),
        "existing report destination was accepted"
    );
}

#[test]
fn json_report_refuses_input_path() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("source.pdf");
    fs::write(&source, SOURCE).unwrap();
    report_must_refuse(&source, &source);
}

#[test]
fn json_report_refuses_hard_link() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("source.pdf");
    let alias = dir.path().join("alias.json");
    fs::write(&source, SOURCE).unwrap();
    fs::hard_link(&source, &alias).unwrap();
    report_must_refuse(&source, &alias);
}

#[test]
#[cfg(unix)]
fn json_report_refuses_symlink() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("source.pdf");
    let alias = dir.path().join("alias.json");
    fs::write(&source, SOURCE).unwrap();
    std::os::unix::fs::symlink(&source, &alias).unwrap();
    report_must_refuse(&source, &alias);
}

#[test]
fn report_collision_prevents_apply() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("source.pdf");
    fs::write(&source, SOURCE).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_tpe-identify"))
        .args(["scan", "--no-extract", "--apply", "--rename-into"])
        .arg(dir.path())
        .arg("--json")
        .arg(&source)
        .arg(&source)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(fs::read(&source).unwrap(), SOURCE);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
#[cfg(unix)]
fn manifest_preflight_never_moves_without_recovery() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("source.pdf");
    fs::write(&source, SOURCE).unwrap();
    let occupied = dir.path().join("tpe-identify-manifest.json");
    let missing = dir.path().join("must-not-be-created.json");
    std::os::unix::fs::symlink(&missing, &occupied).unwrap();
    let report = scan(
        std::slice::from_ref(&source),
        &ScanOptions {
            load: LoadOptions::default(),
            rename_into: Some(dir.path().to_path_buf()),
            ..ScanOptions::default()
        },
    );
    match apply(report.rename.as_ref().unwrap(), dir.path()) {
        Ok((manifest, path)) => {
            assert_ne!(path, occupied);
            assert_eq!(read_manifest(&path).unwrap(), manifest);
            let reversed = undo(&manifest, false);
            assert_eq!(reversed[0].action, "restore");
        }
        Err(_) => assert!(source.exists(), "source moved without a recovery manifest"),
    }
    assert_eq!(fs::read(&source).unwrap(), SOURCE);
    assert_eq!(fs::read_link(&occupied).unwrap(), missing);
    assert!(!missing.exists());
}
