//! End-to-end tests of the `tpe-image-text` binary. The plumbing tests use a
//! committed stand-in for tesseract (`tests/fixtures/fake-tesseract.sh`), so
//! they run everywhere; tests that need a real engine print why they skipped.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use sha2::{Digest, Sha256};

const BIN: &str = env!("CARGO_BIN_EXE_tpe-image-text");

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fake_tesseract() -> PathBuf {
    fixtures().join("fake-tesseract.sh")
}

fn sha256_of(path: &Path) -> String {
    hex::encode(Sha256::digest(fs::read(path).expect("read")))
}

/// Run the binary with the fake tesseract and return (code, stdout, stderr).
fn run(args: &[&str], envs: &[(&str, &str)]) -> (i32, String, String) {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .env("TPE_TESSERACT_BIN", fake_tesseract())
        .env("TPE_OCRS_MODELS_DIR", "/nonexistent/ocrs-models")
        .env("DOCLING_RS_MODELS_DIR", "/nonexistent/docling-models");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run binary");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn exits_2_with_a_precise_message_when_no_engine_is_available() {
    let out = tempfile::tempdir().expect("tempdir");
    let hello = fixtures().join("hello.png");
    let (code, _, stderr) = run(
        &[
            hello.to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ],
        &[("TPE_TESSERACT_BIN", "/nonexistent/tesseract")],
    );
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("no OCR engine available"), "{stderr}");
    assert!(
        stderr.contains(
            "tesseract: configured tesseract executable does not exist: /nonexistent/tesseract"
        ),
        "{stderr}"
    );
    assert!(
        stderr.contains("ocrs:") && stderr.contains("/nonexistent/ocrs-models"),
        "{stderr}"
    );
    assert!(
        stderr.contains("docling:") && stderr.contains("not runnable"),
        "{stderr}"
    );
    assert_eq!(
        fs::read_dir(out.path()).expect("read_dir").count(),
        0,
        "no outputs written"
    );
}

#[test]
fn list_engines_reports_each_engine_honestly() {
    let (code, stdout, _) = run(&["--list-engines", "--json"], &[]);
    assert_eq!(code, 0);
    let probes: Vec<serde_json::Value> = serde_json::from_str(&stdout).expect("json");
    let names: Vec<&str> = probes
        .iter()
        .map(|p| p["engine"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["tesseract", "ocrs", "docling"]);
    assert_eq!(probes[0]["available"], true);
    assert_eq!(probes[0]["version"], "5.9.9-fake");
    assert_eq!(probes[1]["available"], false);
    assert_eq!(probes[2]["available"], false);
    assert!(
        probes[2]["detail"]
            .as_str()
            .unwrap()
            .contains("not provisioned")
    );
}

#[test]
fn writes_text_and_json_per_input_without_touching_inputs() {
    let out = tempfile::tempdir().expect("tempdir");
    let hello = fixtures().join("hello.png");
    let before = sha256_of(&hello);
    let (code, stdout, stderr) = run(
        &[
            hello.to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--json",
        ],
        &[],
    );
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(sha256_of(&hello), before, "input unchanged");
    let summary: serde_json::Value = serde_json::from_str(&stdout).expect("json summary");
    assert_eq!(summary["written"], 1);
    assert_eq!(summary["failed"], 0);
    assert_eq!(summary["engine"]["name"], "tesseract");
    let text = fs::read_to_string(out.path().join("hello.txt")).expect("txt");
    assert_eq!(text, "Hello OCR 42\n");
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.path().join("hello.json")).expect("json"))
            .expect("parse");
    assert_eq!(report["engine"]["name"], "tesseract");
    assert_eq!(report["engine"]["version"], "5.9.9-fake");
    assert_eq!(report["engine"]["lang"], "eng");
    assert_eq!(
        report["engine"]["resource_limits_applied"],
        cfg!(any(target_os = "linux", target_os = "macos"))
    );
    assert_eq!(report["image"]["format"], "png");
    assert_eq!(report["image"]["width"], 420);
    assert_eq!(report["image"]["height"], 90);
    assert_eq!(report["input_sha256"], before);
    assert_eq!(report["blocks"].as_array().unwrap().len(), 1);
    let conf = report["blocks"][0]["confidence"].as_f64().unwrap();
    assert!((conf - 0.92).abs() < 0.001, "{conf}");
    assert_eq!(report["blocks"][0]["bbox"]["x"], 90);
    assert!(report["timing_ms"]["total"].is_u64());
    assert!(
        report["preprocessing"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "grayscale")
    );
    let warnings = report["warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap().contains("Invalid resolution")),
        "{warnings:?}"
    );
}

#[test]
fn refuses_to_overwrite_unless_forced() {
    let out = tempfile::tempdir().expect("tempdir");
    let hello = fixtures().join("hello.png");
    let args = [
        hello.to_str().unwrap(),
        "--out",
        out.path().to_str().unwrap(),
    ];
    let (code, _, _) = run(&args, &[]);
    assert_eq!(code, 0);
    fs::write(out.path().join("hello.txt"), "keep me").expect("write");
    let (code, stdout, _) = run(&args, &[]);
    assert_eq!(code, 1);
    assert!(
        stdout.contains("output exists (use --force to overwrite)"),
        "{stdout}"
    );
    assert_eq!(
        fs::read_to_string(out.path().join("hello.txt")).unwrap(),
        "keep me"
    );
    let (code, _, _) = run(&[&args[..], &["--force"]].concat(), &[]);
    assert_eq!(code, 0);
    assert_eq!(
        fs::read_to_string(out.path().join("hello.txt")).unwrap(),
        "Hello OCR 42\n"
    );
}

#[test]
fn directories_are_walked_and_every_format_is_decoded() {
    let out = tempfile::tempdir().expect("tempdir");
    let input = tempfile::tempdir().expect("tempdir");
    let nested = input.path().join("nested");
    fs::create_dir(&nested).expect("mkdir");
    // Distinct stems so every format gets its own outputs.
    for (fixture, name) in [
        ("hello.png", "a-png.png"),
        ("hello.jpg", "b-jpeg.jpg"),
        ("hello.bmp", "c-bmp.bmp"),
        ("hello.webp", "d-webp.webp"),
        ("anim.gif", "e-gif.gif"),
        ("fake.heic", "f-heic.heic"),
        ("garbage.png", "g-garbage.png"),
        ("hello.tif", "nested/h-tiff.tif"),
    ] {
        fs::copy(fixtures().join(fixture), input.path().join(name)).expect("copy");
    }
    fs::write(input.path().join("readme.txt"), "not an image").expect("write");
    let (code, stdout, stderr) = run(
        &[
            input.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--recursive",
            "--json",
        ],
        &[],
    );
    assert_eq!(code, 1, "heic and garbage fail: {stderr}");
    let summary: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(summary["inputs"], 8, "{stdout}");
    assert_eq!(summary["written"], 6, "{stdout}");
    assert_eq!(summary["failed"], 2, "{stdout}");
    let files = summary["files"].as_array().unwrap();
    let by_name = |n: &str| {
        files
            .iter()
            .find(|f| f["input"].as_str().unwrap().ends_with(n))
            .unwrap()
    };
    assert!(
        by_name("f-heic.heic")["error"]
            .as_str()
            .unwrap()
            .contains("HEIC/HEIF is not decodable")
    );
    assert!(
        by_name("g-garbage.png")["error"]
            .as_str()
            .unwrap()
            .contains("not a recognised image")
    );
    assert!(
        by_name("e-gif.gif")["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("first frame"))
    );
    for (stem, format) in [
        ("a-png", "png"),
        ("b-jpeg", "jpeg"),
        ("c-bmp", "bmp"),
        ("d-webp", "webp"),
        ("e-gif", "gif"),
        ("h-tiff", "tiff"),
    ] {
        assert_eq!(
            fs::read_to_string(out.path().join(format!("{stem}.txt"))).unwrap(),
            "Hello OCR 42\n"
        );
        let report: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(out.path().join(format!("{stem}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(report["image"]["format"], format, "{stem}");
        assert_eq!(report["image"]["width"], 420, "{stem}");
    }
    assert!(!out.path().join("f-heic.txt").exists());
    assert!(!out.path().join("g-garbage.txt").exists());
}

#[test]
fn inputs_sharing_a_stem_never_overwrite_each_other() {
    let out = tempfile::tempdir().expect("tempdir");
    let input = tempfile::tempdir().expect("tempdir");
    fs::create_dir(input.path().join("x")).expect("mkdir");
    fs::create_dir(input.path().join("y")).expect("mkdir");
    fs::copy(
        fixtures().join("hello.png"),
        input.path().join("x/scan.png"),
    )
    .expect("copy");
    fs::copy(
        fixtures().join("hello.jpg"),
        input.path().join("y/scan.jpg"),
    )
    .expect("copy");
    let (code, stdout, _) = run(
        &[
            input.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--recursive",
            "--json",
        ],
        &[],
    );
    assert_eq!(code, 1);
    let summary: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(summary["written"], 1);
    assert_eq!(summary["failed"], 1);
    let second = &summary["files"][1];
    assert!(second["input"].as_str().unwrap().ends_with("y/scan.jpg"));
    assert!(
        second["error"].as_str().unwrap().contains("output exists"),
        "{second}"
    );
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.path().join("scan.json")).unwrap()).unwrap();
    assert!(
        report["input"].as_str().unwrap().ends_with("x/scan.png"),
        "first input kept its outputs"
    );
}

#[test]
fn skipped_non_recursive_directories_are_noted() {
    let out = tempfile::tempdir().expect("tempdir");
    let input = tempfile::tempdir().expect("tempdir");
    fs::create_dir(input.path().join("sub")).expect("mkdir");
    fs::copy(fixtures().join("hello.png"), input.path().join("a.png")).expect("copy");
    let (code, stdout, _) = run(
        &[
            input.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--json",
        ],
        &[],
    );
    assert_eq!(code, 0);
    let summary: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert!(
        summary["notes"][0]
            .as_str()
            .unwrap()
            .contains("no --recursive")
    );
}

#[test]
fn engine_timeout_kills_the_process_and_fails_the_file() {
    let out = tempfile::tempdir().expect("tempdir");
    let hello = fixtures().join("hello.png");
    let started = Instant::now();
    let (code, stdout, _) = run(
        &[
            hello.to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--timeout-secs",
            "1",
        ],
        &[("FAKE_TESSERACT_MODE", "sleep")],
    );
    assert_eq!(code, 1);
    assert!(started.elapsed().as_secs() < 10, "killed promptly");
    assert!(stdout.contains("timed out after 1s"), "{stdout}");
    assert!(!out.path().join("hello.txt").exists());
}

#[test]
fn engine_failure_quotes_its_stderr() {
    let out = tempfile::tempdir().expect("tempdir");
    let hello = fixtures().join("hello.png");
    let (code, stdout, _) = run(
        &[
            hello.to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ],
        &[("FAKE_TESSERACT_MODE", "fail")],
    );
    assert_eq!(code, 1);
    assert!(
        stdout.contains("tesseract exited exit status: 1: Error opening data file"),
        "{stdout}"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn kernel_limits_reach_the_engine_process() {
    let out = tempfile::tempdir().expect("tempdir");
    let hello = fixtures().join("hello.png");
    let (code, _, stderr) = run(
        &[
            hello.to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--max-memory-mib",
            "512",
            "--timeout-secs",
            "7",
        ],
        &[("FAKE_TESSERACT_MODE", "limits")],
    );
    assert_eq!(code, 0, "{stderr}");
    let text = fs::read_to_string(out.path().join("hello.txt")).unwrap();
    assert_eq!(
        text, "as=524288 cpu=7\n",
        "ulimit -v is in KiB, -t in seconds"
    );
}

#[test]
fn bad_language_codes_are_rejected_before_spawning() {
    let out = tempfile::tempdir().expect("tempdir");
    let hello = fixtures().join("hello.png");
    let (code, stdout, _) = run(
        &[
            hello.to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--lang=--psm",
        ],
        &[],
    );
    assert_eq!(code, 1);
    assert!(stdout.contains("not a tesseract language code"), "{stdout}");
}

#[test]
fn exec_limited_helper_rejects_bad_arguments() {
    let out = Command::new(BIN)
        .arg("exec-limited")
        .arg("0")
        .arg("1")
        .arg("/bin/true")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(125));
    let out = Command::new(BIN).arg("exec-limited").output().unwrap();
    assert_eq!(out.status.code(), Some(125));
}

/// Needs a real `tesseract` on PATH; skips with a reason otherwise.
#[test]
fn real_tesseract_reads_the_fixture() {
    let Ok(bin) = tpe_image_text::engine::tesseract::find_binary(None) else {
        eprintln!("skipped: tesseract is not installed on PATH");
        return;
    };
    let out = tempfile::tempdir().expect("tempdir");
    let hello = fixtures().join("hello.png");
    let output = Command::new(BIN)
        .args([
            hello.to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--engine",
            "tesseract",
        ])
        .env("TPE_TESSERACT_BIN", &bin)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = fs::read_to_string(out.path().join("hello.txt")).unwrap();
    assert!(text.contains("Hello"), "{text}");
}

/// Needs the ocrs models (`TPE_OCRS_MODELS_DIR` or `<repo>/.models/ocrs`); skips otherwise.
#[cfg(feature = "ocrs")]
#[test]
fn real_ocrs_reads_the_fixtures() {
    let dir = std::env::var_os("TPE_OCRS_MODELS_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.models/ocrs"),
        PathBuf::from,
    );
    if let Err(reason) = tpe_image_text::engine::ocrs::verify_models(&dir) {
        eprintln!("skipped: {reason}");
        return;
    }
    let out = tempfile::tempdir().expect("tempdir");
    let hello = fixtures().join("hello.png");
    let skew = fixtures().join("skew.png");
    let output = Command::new(BIN)
        .args([
            hello.to_str().unwrap(),
            skew.to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--engine",
            "ocrs",
            "--json",
        ])
        .env("TPE_OCRS_MODELS_DIR", &dir)
        .env("TPE_TESSERACT_BIN", "/nonexistent/tesseract")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = fs::read_to_string(out.path().join("hello.txt")).unwrap();
    assert!(text.contains("Hello") && text.contains("OCR"), "{text}");
    let skew_text = fs::read_to_string(out.path().join("skew.txt")).unwrap();
    assert!(skew_text.to_lowercase().contains("skewed"), "{skew_text}");
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out.path().join("skew.json")).unwrap()).unwrap();
    assert_eq!(report["engine"]["name"], "ocrs");
    assert!(
        report["blocks"][0]["confidence"].is_null(),
        "ocrs has no confidence"
    );
    assert!(
        report["preprocessing"]["deskew_degrees"]
            .as_f64()
            .unwrap()
            .abs()
            >= 2.0,
        "{}",
        report["preprocessing"]
    );
    let (code, stdout, _) = run(
        &[
            hello.to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--engine",
            "ocrs",
            "--lang",
            "deu",
            "--force",
        ],
        &[("TPE_OCRS_MODELS_DIR", dir.to_str().unwrap())],
    );
    assert_eq!(code, 1);
    assert!(stdout.contains("ocrs has only the English"), "{stdout}");
}
