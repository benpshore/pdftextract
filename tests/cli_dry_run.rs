//! Dry-run must stay before source decoding, worker creation and publication.
use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn dry_run_and_n_preserve_invalid_source_existing_ledger_and_destinations() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("broken.pdf");
    let ledger = root.path().join("ledger.sqlite");
    let output = root.path().join("absent-output");
    let captures = root.path().join("captures");
    fs::create_dir(&captures).unwrap();
    fs::write(
        &input,
        b"deliberately invalid PDF; planning must not decode this",
    )
    .unwrap();
    fs::write(&ledger, b"existing non-SQLite bytes must never be touched").unwrap();
    let source = fs::read(&input).unwrap();
    let database = fs::read(&ledger).unwrap();
    for flag in ["--dry-run", "-n"] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_tpe"));
        cmd.args(["extract", flag])
            .arg(&input)
            .arg("--db")
            .arg(&ledger)
            .arg("--out")
            .arg(&output)
            .env("TMPDIR", &captures);
        let result = cmd.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(value["status"], "planned");
        assert_eq!(value["network"], false);
        assert_eq!(value["writes"], false);
        assert!(
            value["unchecked"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "source contents/checksum")
        );
        assert_eq!(fs::read(&input).unwrap(), source);
        assert_eq!(fs::read(&ledger).unwrap(), database);
        assert!(!output.exists());
        assert_eq!(fs::read_dir(&captures).unwrap().count(), 0);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 3);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_planning_succeeds_with_network_and_file_writes_denied() {
    let root = tempfile::tempdir().unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native-worker/native.pdf");
    for flag in ["-n", "--dry-run"] {
        let result = Command::new("/usr/bin/sandbox-exec")
            .args([
                "-p",
                "(version 1) (allow default) (deny file-write*) (deny network*)",
            ])
            .arg(env!("CARGO_BIN_EXE_tpe"))
            .args(["extract", flag])
            .arg(&fixture)
            .arg("--db")
            .arg(root.path().join("absent.sqlite"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
}

#[test]
fn ordinary_extraction_still_rejects_invalid_input() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("broken.pdf");
    fs::write(&input, b"not a PDF").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_tpe"))
        .arg("extract")
        .arg("--json")
        .arg(&input)
        .arg("--db")
        .arg(root.path().join("ledger.sqlite"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["status"], "failed");
    assert!(!Path::new(&root.path().join("ledger.sqlite")).exists());
}
