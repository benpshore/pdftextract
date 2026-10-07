//! Source-preservation regressions for ledger reads and export publication.

use std::fs;
use std::path::Path;
use std::process::Command;

use rusqlite::{Connection, OpenFlags};
use tpe::ledger::Ledger;
use tpe::schema::ExtractionResult;
use tpe_export::input::{self, RunSelector};
use tpe_export::model::Export;
use tpe_export::{ExportError, Format, write_export_preserving};

const FIXTURE: &str = include_str!("fixtures/sample.json");

fn result() -> ExtractionResult {
    serde_json::from_str(FIXTURE).unwrap()
}

fn ledger(path: &Path, with_run: bool) -> Option<i64> {
    let run = {
        let mut ledger = Ledger::open(path).unwrap();
        with_run.then(|| ledger.write_result(&result()).unwrap())
    };
    // A closed, rollback-journal fixture makes exact main-file byte checks
    // independent of WAL checkpoint timing.
    Connection::open(path)
        .unwrap()
        .execute_batch("PRAGMA journal_mode = DELETE;")
        .unwrap();
    run
}

fn cli(input: &Path, output: &Path, format: Format, force: bool) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tpe-export"));
    command
        .args(["--format", format.name(), "--input"])
        .arg(input)
        .arg("--output")
        .arg(output);
    if force {
        command.arg("--force");
    }
    command.output().unwrap()
}

#[test]
fn v4_without_figures_exports_without_migration_or_initialization() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v4.sqlite");
    let run = ledger(&path, true).unwrap();
    Connection::open(&path).unwrap().execute_batch(
        "UPDATE schema_meta SET version = 4; UPDATE runs SET schema_version = 4; DROP TABLE figures;"
    ).unwrap();
    let before = fs::read(&path).unwrap();
    let expected = input::parse_json(FIXTURE.as_bytes()).unwrap();
    for selector in [
        RunSelector::Latest,
        RunSelector::RunId(run),
        RunSelector::HashPrefix("9F2C".into()),
    ] {
        assert_eq!(input::load(&path, &selector).unwrap(), expected);
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    for format in Format::ALL {
        let output = dir.path().join(format.name());
        let response = cli(&path, &output, format, false);
        assert!(
            response.status.success(),
            "{}",
            String::from_utf8_lossy(&response.stderr)
        );
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let version: u32 = conn
        .query_row("SELECT version FROM schema_meta", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 4);
    let figures: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='figures')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!figures);
}

#[test]
fn no_runs_and_unsupported_schema_are_refused_without_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.sqlite");
    ledger(&path, false);
    for version in [4, 999] {
        Connection::open(&path)
            .unwrap()
            .execute("UPDATE schema_meta SET version=?1", [version])
            .unwrap();
        let before = fs::read(&path).unwrap();
        let response = cli(&path, &dir.path().join("out.csv"), Format::Csv, false);
        assert!(!response.status.success());
        let stderr = String::from_utf8_lossy(&response.stderr);
        assert!(
            stderr.contains(if version == 4 {
                "no runs"
            } else {
                "schema version"
            }),
            "{stderr}"
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!dir.path().join("out.csv").exists());
    }
    Connection::open(&path)
        .unwrap()
        .execute("DELETE FROM schema_meta", [])
        .unwrap();
    let before = fs::read(&path).unwrap();
    assert!(input::load(&path, &RunSelector::Latest).is_err());
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "missing version is not initialized"
    );
}

#[test]
fn read_only_connection_cannot_write_or_create_a_missing_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.sqlite");
    let run = ledger(&path, true).unwrap();
    let before = fs::read(&path).unwrap();
    let mut reader = Ledger::open_read_only(&path).unwrap();
    let error = reader
        .update_timings(run, &tpe::schema::StageTimings::default())
        .unwrap_err();
    assert!(
        matches!(error, tpe::ledger::LedgerError::Sqlite(rusqlite::Error::SqliteFailure(code, _)) if code.code == rusqlite::ErrorCode::ReadOnly)
    );
    assert!(reader.write_result(&result()).is_err());
    drop(reader);
    assert_eq!(fs::read(&path).unwrap(), before);
    let missing = dir.path().join("missing.sqlite");
    assert!(Ledger::open_read_only(&missing).is_err());
    assert!(!missing.exists());
}

#[cfg(unix)]
#[test]
fn permission_read_only_ledger_is_exportable() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.sqlite");
    ledger(&path, true);
    let before = fs::read(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let response = cli(&path, &dir.path().join("out.csv"), Format::Csv, false);
    assert!(
        response.status.success(),
        "{}",
        String::from_utf8_lossy(&response.stderr)
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o444
    );
}

#[test]
fn live_wal_rows_are_read_and_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wal.sqlite");
    let mut writer = Ledger::open(&path).unwrap();
    let run = writer.write_result(&result()).unwrap();
    let before = fs::read(&path).unwrap();
    let wal = dir.path().join("wal.sqlite-wal");
    let wal_before = fs::read(&wal).unwrap();
    assert!(!wal_before.is_empty());
    assert_eq!(
        input::load(&path, &RunSelector::RunId(run)).unwrap(),
        input::parse_json(FIXTURE.as_bytes()).unwrap()
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(&wal).unwrap(), wal_before);
}

#[test]
fn cli_refuses_direct_and_hard_link_self_output_for_json_and_ledgers() {
    let dir = tempfile::tempdir().unwrap();
    for is_ledger in [false, true] {
        let source = dir.path().join(if is_ledger {
            "ledger.sqlite"
        } else {
            "source.json"
        });
        if is_ledger {
            ledger(&source, true);
        } else {
            fs::write(&source, FIXTURE).unwrap();
        }
        let alias = dir.path().join(if is_ledger {
            "ledger-alias"
        } else {
            "json-alias"
        });
        fs::hard_link(&source, &alias).unwrap();
        let before = fs::read(&source).unwrap();
        for output in [&source, &alias] {
            for format in Format::ALL {
                for force in [false, true] {
                    let response = cli(&source, output, format, force);
                    assert!(!response.status.success());
                    assert!(
                        String::from_utf8_lossy(&response.stderr)
                            .contains("output aliases the input")
                    );
                    assert_eq!(fs::read(&source).unwrap(), before);
                    assert_eq!(fs::read(&alias).unwrap(), before);
                    assert_eq!(
                        same_file::Handle::from_path(&source).unwrap(),
                        same_file::Handle::from_path(&alias).unwrap()
                    );
                }
            }
        }
    }
    assert!(fs::read_dir(dir.path()).unwrap().all(|e| {
        !e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tpe-export-")
    }));
}

#[cfg(unix)]
#[test]
fn cli_refuses_symlink_and_parent_symlink_self_output() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.json");
    fs::write(&source, FIXTURE).unwrap();
    let alias = dir.path().join("alias.json");
    symlink(&source, &alias).unwrap();
    let parent = dir.path().join("parent");
    symlink(dir.path(), &parent).unwrap();
    for output in [&alias, &source, &parent.join("source.json")] {
        for format in Format::ALL {
            let response = cli(&alias, output, format, true);
            assert!(!response.status.success());
            assert!(String::from_utf8_lossy(&response.stderr).contains("output aliases the input"));
            assert_eq!(fs::read_to_string(&source).unwrap(), FIXTURE);
            assert!(fs::symlink_metadata(&alias).unwrap().is_symlink());
        }
    }
}

#[test]
fn publication_rejects_alias_created_after_loading() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.json");
    fs::write(&source, FIXTURE).unwrap();
    let (article, identity) = input::load_preserving(&source, &RunSelector::Latest).unwrap();
    let output = dir.path().join("out.csv");
    fs::hard_link(&source, &output).unwrap();
    let error = write_export_preserving(
        &Export::from_article(&article),
        Format::Csv,
        &output,
        true,
        &identity,
    )
    .unwrap_err();
    assert!(matches!(error, ExportError::OutputIsInput(_)));
    assert_eq!(fs::read_to_string(&source).unwrap(), FIXTURE);
}

#[test]
fn publication_rejects_replaced_source_after_loading() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.json");
    fs::write(&source, FIXTURE).unwrap();
    let (article, identity) = input::load_preserving(&source, &RunSelector::Latest).unwrap();
    fs::rename(&source, dir.path().join("original.json")).unwrap();
    fs::write(&source, "replacement").unwrap();
    let output = dir.path().join("out.csv");
    fs::write(&output, "previous output").unwrap();
    let error = write_export_preserving(
        &Export::from_article(&article),
        Format::Csv,
        &output,
        true,
        &identity,
    )
    .unwrap_err();
    assert!(matches!(error, ExportError::InputChanged(_)));
    assert_eq!(fs::read_to_string(&source).unwrap(), "replacement");
    assert_eq!(fs::read_to_string(&output).unwrap(), "previous output");
    assert_eq!(
        fs::read_to_string(dir.path().join("original.json")).unwrap(),
        FIXTURE
    );
}
