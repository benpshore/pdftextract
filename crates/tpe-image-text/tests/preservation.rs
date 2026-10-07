//! Real filesystem tests through the same `process_file` entry point as the CLI.
//! The synthetic engine avoids requiring OCR executables, models or a network.
use std::fs;
use std::path::{Path, PathBuf};

use image::GrayImage;
use tpe_image_text::{
    Block, Engine, ImageTextError, PreprocessOptions, Recognition, RunOptions, process_file,
};

struct Synthetic {
    block_json: Option<PathBuf>,
    alias_json: Option<(PathBuf, PathBuf)>,
}

impl Engine for Synthetic {
    fn name(&self) -> &'static str {
        "synthetic-publication-test"
    }
    fn version(&self) -> String {
        "fixture".into()
    }
    fn recognize(&self, _: &GrayImage, _: &str) -> Result<Recognition, ImageTextError> {
        if let Some(path) = &self.block_json {
            fs::create_dir(path).unwrap();
        }
        if let Some((input, output)) = &self.alias_json {
            fs::hard_link(input, output).unwrap();
        }
        Ok(Recognition {
            blocks: vec![Block {
                text: "synthetic text".into(),
                confidence: None,
                bbox: None,
            }],
            ..Recognition::default()
        })
    }
}

fn engine() -> Synthetic {
    Synthetic {
        block_json: None,
        alias_json: None,
    }
}

fn image(path: &Path) -> Vec<u8> {
    GrayImage::from_pixel(8, 8, image::Luma([255]))
        .save_with_format(path, image::ImageFormat::Png)
        .unwrap();
    fs::read(path).unwrap()
}

fn options(directory: &Path, force: bool) -> RunOptions {
    RunOptions {
        out_dir: directory.to_path_buf(),
        force,
        lang: "eng".into(),
        preprocess: PreprocessOptions {
            auto_contrast: false,
            upscale_small_text: false,
            deskew: false,
        },
    }
}

fn no_staging(directory: &Path) {
    assert!(
        fs::read_dir(directory).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".tpe-image-text-")
        }),
        "staging/recovery debris left behind"
    );
}

#[test]
fn rejects_input_path_even_with_force() {
    for extension in ["txt", "json"] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join(format!("scan.{extension}"));
        let before = image(&input);
        let result = process_file(&input, &engine(), &options(dir.path(), true));
        assert!(result.is_err(), "source path must be refused");
        assert_eq!(fs::read(&input).unwrap(), before);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[test]
fn rejects_hard_link_even_with_force() {
    for extension in ["txt", "json"] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("scan.png");
        let before = image(&input);
        let output = dir.path().join(format!("scan.{extension}"));
        fs::hard_link(&input, &output).unwrap();
        let result = process_file(&input, &engine(), &options(dir.path(), true));
        assert!(result.is_err(), "hard-link alias must be refused");
        assert_eq!(fs::read(&input).unwrap(), before);
        assert_eq!(fs::read(&output).unwrap(), before);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}

#[test]
#[cfg(unix)]
fn rejects_symlink_even_with_force() {
    for extension in ["txt", "json"] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("scan.png");
        let before = image(&input);
        let output = dir.path().join(format!("scan.{extension}"));
        std::os::unix::fs::symlink(&input, &output).unwrap();
        let result = process_file(&input, &engine(), &options(dir.path(), true));
        assert!(result.is_err(), "symlink alias must be refused");
        assert_eq!(fs::read(&input).unwrap(), before);
        assert_eq!(fs::read_link(&output).unwrap(), input);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}

#[test]
fn rejects_alias_created_during_recognition() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("scan.png");
    let before = image(&input);
    let output = dir.path().join("scan.json");
    let engine = Synthetic {
        block_json: None,
        alias_json: Some((input.clone(), output)),
    };
    assert!(process_file(&input, &engine, &options(dir.path(), true)).is_err());
    assert_eq!(fs::read(&input).unwrap(), before);
    assert!(!dir.path().join("scan.txt").exists());
    no_staging(dir.path());
}

#[test]
fn rejects_another_batch_input_as_output() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("scan.png");
    let second = dir.path().join("scan.txt");
    let first_bytes = image(&first);
    let second_bytes = image(&second);
    let outcomes = tpe_image_text::run_batch(
        &[first.clone(), second.clone()],
        &engine(),
        &options(dir.path(), true),
    );
    assert!(outcomes.iter().all(|outcome| !outcome.ok));
    assert_eq!(fs::read(&first).unwrap(), first_bytes);
    assert_eq!(fs::read(&second).unwrap(), second_bytes);
    assert!(!dir.path().join("scan.json").exists());
    no_staging(dir.path());
}

#[test]
#[cfg(unix)]
fn no_force_dangling_second_output_keeps_pair_absent() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("scan.png");
    let before = image(&input);
    let missing = dir.path().join("missing.json");
    let output = dir.path().join("scan.json");
    std::os::unix::fs::symlink(&missing, &output).unwrap();
    assert!(process_file(&input, &engine(), &options(dir.path(), false)).is_err());
    assert!(
        !dir.path().join("scan.txt").exists(),
        "no partial output pair"
    );
    assert!(!missing.exists());
    assert_eq!(fs::read_link(&output).unwrap(), missing);
    assert_eq!(fs::read(&input).unwrap(), before);
    no_staging(dir.path());
}

#[test]
#[cfg(unix)]
fn force_replaces_dangling_entry_without_following() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("scan.png");
    let before = image(&input);
    let missing = dir.path().join("missing.txt");
    let output = dir.path().join("scan.txt");
    std::os::unix::fs::symlink(&missing, &output).unwrap();
    process_file(&input, &engine(), &options(dir.path(), true)).unwrap();
    assert!(!missing.exists(), "force must not write a symlink referent");
    assert!(fs::symlink_metadata(&output).unwrap().is_file());
    assert_eq!(fs::read(&input).unwrap(), before);
    no_staging(dir.path());
}

#[test]
fn no_force_existing_output_keeps_pair_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("scan.png");
    let before = image(&input);
    let output = dir.path().join("scan.json");
    fs::write(&output, b"old JSON").unwrap();
    assert!(process_file(&input, &engine(), &options(dir.path(), false)).is_err());
    assert_eq!(fs::read(&output).unwrap(), b"old JSON");
    assert!(!dir.path().join("scan.txt").exists());
    assert_eq!(fs::read(&input).unwrap(), before);
    no_staging(dir.path());
}

#[test]
fn normal_new_output_publishes_complete_pair() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("scan.png");
    let before = image(&input);
    let report = process_file(&input, &engine(), &options(dir.path(), false)).unwrap();
    assert_eq!(fs::read(&report.outputs.text).unwrap(), b"synthetic text\n");
    let saved: tpe_image_text::FileReport =
        serde_json::from_slice(&fs::read(&report.outputs.json).unwrap()).unwrap();
    assert_eq!(saved.input_sha256, report.input_sha256);
    assert_eq!(saved.text_chars, report.text_chars);
    assert_eq!(fs::read(&input).unwrap(), before);
    no_staging(dir.path());
}

#[test]
fn force_replaces_legitimate_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("scan.png");
    let before = image(&input);
    for extension in ["txt", "json"] {
        fs::write(dir.path().join(format!("scan.{extension}")), b"old").unwrap();
    }
    let report = process_file(&input, &engine(), &options(dir.path(), true)).unwrap();
    assert_eq!(fs::read(&report.outputs.text).unwrap(), b"synthetic text\n");
    let _: tpe_image_text::FileReport =
        serde_json::from_slice(&fs::read(&report.outputs.json).unwrap()).unwrap();
    assert_eq!(fs::read(&input).unwrap(), before);
    no_staging(dir.path());
}

#[test]
fn failed_pair_keeps_new_output_absent() {
    failed_pair(false);
}

#[test]
fn failed_pair_preserves_existing_output() {
    failed_pair(true);
}

fn failed_pair(force: bool) {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("scan.png");
    let before = image(&input);
    let text = dir.path().join("scan.txt");
    if force {
        fs::write(&text, b"old text").unwrap();
    }
    let engine = Synthetic {
        block_json: Some(dir.path().join("scan.json")),
        alias_json: None,
    };
    assert!(process_file(&input, &engine, &options(dir.path(), force)).is_err());
    if force {
        assert_eq!(fs::read(&text).unwrap(), b"old text");
    } else {
        assert!(
            !text.exists(),
            "failed second output must not leave the first"
        );
    }
    assert_eq!(fs::read(&input).unwrap(), before);
    no_staging(dir.path());
}
