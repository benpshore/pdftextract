//! Synthetic fixtures through the public extractor/writer and the shipped CLI.
//! Original-source controls use only this API, with huge-allocation tests excluded.
use std::fmt::Write as _;
use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use std::process::Command;

use tpe_formats::{FormatsError, FormatsResult, Options, extract_path, write_outputs};
use zip::write::SimpleFileOptions;

fn zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, contents) in entries {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(contents.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn source(dir: &Path) -> FormatsResult {
    let path = dir.join("source.md");
    fs::write(&path, "source").unwrap();
    extract_path(&path, &Options::default()).unwrap()
}

fn workbook(rows: &str, sheet_count: usize) -> Vec<u8> {
    let mut sheets = String::new();
    for i in 1..=sheet_count {
        write!(sheets, "<sheet name=\"Sheet{i}\" sheetId=\"{i}\" r:id=\"rId1\"/>").unwrap();
    }
    let workbook = format!(
        "<workbook xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets>{sheets}</sheets></workbook>"
    );
    let sheet = format!("<worksheet><sheetData>{rows}</sheetData></worksheet>");
    zip(&[
        ("xl/workbook.xml", &workbook),
        (
            "xl/_rels/workbook.xml.rels",
            "<Relationships><Relationship Id=\"rId1\" Target=\"worksheets/sheet1.xml\"/></Relationships>",
        ),
        ("xl/worksheets/sheet1.xml", &sheet),
    ])
}

fn extract_xlsx(rows: &str, sheet_count: usize) -> Result<FormatsResult, FormatsError> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.xlsx");
    let bytes = workbook(rows, sheet_count);
    fs::write(&path, &bytes).unwrap();
    let result = extract_path(&path, &Options::default());
    assert_eq!(fs::read(path).unwrap(), bytes);
    result
}

#[test]
fn lone_json_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let result = source(dir.path());
    fs::write(dir.path().join("out.json"), b"old JSON").unwrap();
    assert!(
        write_outputs(&result, dir.path(), "out", false)
            .unwrap()
            .is_none()
    );
    assert_eq!(fs::read(dir.path().join("out.json")).unwrap(), b"old JSON");
    assert!(!dir.path().join("out.txt").exists());
}

#[test]
fn lone_text_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let result = source(dir.path());
    fs::write(dir.path().join("out.txt"), b"old text").unwrap();
    assert!(
        write_outputs(&result, dir.path(), "out", false)
            .unwrap()
            .is_none()
    );
    assert_eq!(fs::read(dir.path().join("out.txt")).unwrap(), b"old text");
    assert!(!dir.path().join("out.json").exists());
}

#[test]
fn input_text_without_companion_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.txt");
    fs::write(&input, b"source").unwrap();
    let result = extract_path(&input, &Options::default()).unwrap();
    assert!(
        write_outputs(&result, dir.path(), "source", false)
            .unwrap()
            .is_none()
    );
    assert_eq!(fs::read(input).unwrap(), b"source");
    assert!(!dir.path().join("source.json").exists());
}

#[test]
fn forced_input_path_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.txt");
    fs::write(&input, b"source").unwrap();
    let result = extract_path(&input, &Options::default()).unwrap();
    assert!(write_outputs(&result, dir.path(), "source", true).is_err());
    assert_eq!(fs::read(input).unwrap(), b"source");
    assert!(!dir.path().join("source.json").exists());
}

#[test]
fn forced_hard_link_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let result = source(dir.path());
    fs::hard_link(dir.path().join("source.md"), dir.path().join("out.json")).unwrap();
    assert!(write_outputs(&result, dir.path(), "out", true).is_err());
    assert_eq!(fs::read(dir.path().join("source.md")).unwrap(), b"source");
    assert!(!dir.path().join("out.txt").exists());
}

#[cfg(unix)]
#[test]
fn forced_symlink_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let result = source(dir.path());
    std::os::unix::fs::symlink("source.md", dir.path().join("out.txt")).unwrap();
    assert!(write_outputs(&result, dir.path(), "out", true).is_err());
    assert_eq!(fs::read(dir.path().join("source.md")).unwrap(), b"source");
    assert!(!dir.path().join("out.json").exists());
}

#[test]
fn cli_protects_another_batch_input() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("a.md");
    let second = dir.path().join("a.txt");
    fs::write(&first, b"first").unwrap();
    fs::write(&second, b"second").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tpe-formats"))
        .args([first.as_os_str(), second.as_os_str()])
        .arg("--out")
        .arg(dir.path())
        .args(["--force", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let reports: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reports[0]["status"], "failed");
    assert_eq!(fs::read(first).unwrap(), b"first");
    assert_eq!(fs::read(second).unwrap(), b"second");
    assert!(!dir.path().join("a.json").exists());
    assert!(dir.path().join("a-2.txt").exists());
}

#[cfg(unix)]
#[test]
fn dangling_output_blocks_unforced_pair() {
    let dir = tempfile::tempdir().unwrap();
    let result = source(dir.path());
    std::os::unix::fs::symlink("absent", dir.path().join("out.txt")).unwrap();
    assert!(
        write_outputs(&result, dir.path(), "out", false)
            .unwrap()
            .is_none()
    );
    assert!(!dir.path().join("absent").exists());
    assert!(!dir.path().join("out.json").exists());
}

#[cfg(unix)]
#[test]
fn force_replaces_dangling_entry_without_following() {
    let dir = tempfile::tempdir().unwrap();
    let result = source(dir.path());
    std::os::unix::fs::symlink("absent", dir.path().join("out.txt")).unwrap();
    write_outputs(&result, dir.path(), "out", true)
        .unwrap()
        .unwrap();
    assert!(!dir.path().join("absent").exists());
    assert!(
        fs::symlink_metadata(dir.path().join("out.txt"))
            .unwrap()
            .is_file()
    );
}

#[test]
fn failed_pair_leaves_new_json_absent() {
    let dir = tempfile::tempdir().unwrap();
    let result = source(dir.path());
    fs::create_dir(dir.path().join("out.txt")).unwrap();
    assert!(write_outputs(&result, dir.path(), "out", true).is_err());
    assert!(!dir.path().join("out.json").exists());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn failed_pair_preserves_old_json() {
    let dir = tempfile::tempdir().unwrap();
    let result = source(dir.path());
    fs::write(dir.path().join("out.json"), b"old JSON").unwrap();
    fs::create_dir(dir.path().join("out.txt")).unwrap();
    assert!(write_outputs(&result, dir.path(), "out", true).is_err());
    assert_eq!(fs::read(dir.path().join("out.json")).unwrap(), b"old JSON");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
}

#[test]
fn preview_alias_is_rejected_before_pair_publication() {
    let dir = tempfile::tempdir().unwrap();
    let mut result = source(dir.path());
    result.extra_files.push(tpe_formats::ExtraFile {
        suffix: "preview.pdf".into(),
        bytes: b"preview".to_vec(),
    });
    fs::hard_link(
        dir.path().join("source.md"),
        dir.path().join("out.preview.pdf"),
    )
    .unwrap();
    assert!(write_outputs(&result, dir.path(), "out", true).is_err());
    assert_eq!(fs::read(dir.path().join("source.md")).unwrap(), b"source");
    assert!(!dir.path().join("out.json").exists());
}

#[test]
fn package_inputs_reject_internal_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.pages");
    fs::create_dir(&input).unwrap();
    fs::write(input.join("preview.pdf"), b"synthetic preview").unwrap();
    let result = extract_path(&input, &Options::default()).unwrap();
    assert!(write_outputs(&result, &input, "out", true).is_err());
    assert!(write_outputs(&result, &input.join("new"), "out", true).is_err());
    assert_eq!(
        fs::read(input.join("preview.pdf")).unwrap(),
        b"synthetic preview"
    );
    assert_eq!(fs::read_dir(input).unwrap().count(), 1);
}

#[test]
fn zero_row_returns_typed_error() {
    let result =
        std::panic::catch_unwind(|| extract_xlsx("<row r=\"0\"><c r=\"A0\"><v>1</v></c></row>", 1));
    assert!(matches!(result, Ok(Err(FormatsError::Invalid(_)))));
}

#[test]
fn malformed_coordinates_return_typed_errors() {
    for rows in [
        "<row r=\"oops\"><c r=\"A1\"><v>1</v></c></row>",
        "<row r=\"-1\"/>",
        "<row r=\"+1\"/>",
        "<row r=\"\"/>",
        "<row r=\"1\"><c r=\"A\"><v>1</v></c></row>",
        "<row r=\"1\"><c r=\"A0\"><v>1</v></c></row>",
        "<row r=\"1\"><c r=\"A2\"><v>1</v></c></row>",
        "<row r=\"1\"><c r=\"A1junk\"><v>1</v></c></row>",
        "<row r=\"1\"><c r=\"1\"><v>1</v></c></row>",
        "<row r=\"1\"><c r=\"$A$1\"><v>1</v></c></row>",
        "<row r=\"1\"><c r=\"\"><v>1</v></c></row>",
    ] {
        assert!(
            matches!(extract_xlsx(rows, 1), Err(FormatsError::Invalid(_))),
            "{rows}"
        );
    }
}

#[test]
fn overflowing_column_returns_typed_error() {
    let rows = format!(
        "<row r=\"1\"><c r=\"{}1\"><v>1</v></c></row>",
        "A".repeat(80)
    );
    let result = std::panic::catch_unwind(|| extract_xlsx(&rows, 1));
    assert!(matches!(result, Ok(Err(FormatsError::Invalid(_)))));
}

#[test]
fn cli_continues_after_zero_row() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.xlsx");
    let good = dir.path().join("good.md");
    fs::write(&bad, workbook("<row r=\"0\"/>", 1)).unwrap();
    fs::write(&good, b"good").unwrap();
    let out = dir.path().join("out");
    let output = Command::new(env!("CARGO_BIN_EXE_tpe-formats"))
        .args([bad.as_os_str(), good.as_os_str()])
        .arg("--out")
        .arg(&out)
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let reports: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reports[0]["status"], "failed");
    assert_eq!(reports[1]["status"], "complete");
    assert!(out.join("good.txt").exists());
    assert!(!out.join("bad.json").exists());
}

#[test]
fn valid_outputs_control() {
    let dir = tempfile::tempdir().unwrap();
    let result = source(dir.path());
    let outputs = write_outputs(&result, dir.path(), "out", false)
        .unwrap()
        .unwrap();
    assert_eq!(
        fs::read_to_string(outputs.txt.unwrap()).unwrap(),
        result.text
    );
    let json: serde_json::Value = serde_json::from_slice(&fs::read(outputs.json).unwrap()).unwrap();
    assert_eq!(json["text"], result.text);
    assert_eq!(fs::read(dir.path().join("source.md")).unwrap(), b"source");
}

#[test]
fn existing_pair_control() {
    let dir = tempfile::tempdir().unwrap();
    let result = source(dir.path());
    for name in ["out.json", "out.txt"] {
        fs::write(dir.path().join(name), b"old").unwrap();
    }
    assert!(
        write_outputs(&result, dir.path(), "out", false)
            .unwrap()
            .is_none()
    );
    for name in ["out.json", "out.txt"] {
        assert_eq!(fs::read(dir.path().join(name)).unwrap(), b"old");
    }
}

#[test]
fn sparse_and_inferred_coordinates_control() {
    let result = extract_xlsx(
        "<row r=\"2\"><c r=\"C2\"><v>7</v></c><c><v>8</v></c></row><row><c><v>9</v></c></row>",
        1,
    )
    .unwrap();
    let rows = result.sections[0].blocks[0].rows.as_ref().unwrap();
    assert_eq!(rows, &vec![vec![], vec!["", "", "7", "8"], vec!["9"]]);
}

// These cases deliberately never run against the unbounded original reader.
#[test]
fn corrected_only_huge_coordinates_are_bounded() {
    for rows in [
        "<row r=\"18446744073709551615\"/>",
        "<row r=\"9999999999999999999999999999999999999999999999999\"/>",
        "<row r=\"1048576\"><c r=\"A1048576\"><v>1</v></c></row>",
        "<row r=\"1\"><c r=\"XFE1\"><v>1</v></c></row>",
    ] {
        assert!(matches!(
            extract_xlsx(rows, 1),
            Err(FormatsError::Invalid(_))
        ));
    }
}

#[test]
fn corrected_only_workbook_row_budget_is_shared() {
    assert!(matches!(
        extract_xlsx("<row r=\"60000\"><c r=\"A60000\"><v>1</v></c></row>", 2),
        Err(FormatsError::Invalid(_))
    ));
}

#[test]
fn corrected_only_workbook_cell_budget_is_shared() {
    let mut rows = String::new();
    for i in 1..=31 {
        write!(rows, "<row r=\"{i}\"><c r=\"XFD{i}\"><v>1</v></c></row>").unwrap();
    }
    assert!(matches!(
        extract_xlsx(&rows, 2),
        Err(FormatsError::Invalid(_))
    ));
}

#[test]
fn corrected_only_repeated_coordinates_consume_budget() {
    let rows = "<row r=\"1\"/>".repeat(100_001);
    assert!(matches!(
        extract_xlsx(&rows, 1),
        Err(FormatsError::Invalid(_))
    ));
}
