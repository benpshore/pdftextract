//! Bounded publisher transport-contract regressions using synthetic owned inputs.
//!
//! Expected identities are seeded from a real successful CLI fixture to isolate
//! the publisher contract. These tests do not independently qualify acquisition,
//! prove that a parser generated the right artifact, or establish PDF fidelity.
#![cfg(feature = "pdf-oxide")]

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tempfile::TempDir;

const OUTPUT_LIMIT: u64 = 1_048_576;
const SOURCE: &[u8] = b"Owned synthetic transport identity control.\n";
const SENTINEL: &[u8] = b"Unrelated caller output must remain unchanged.\n";

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn private_write(path: &Path, bytes: &[u8]) {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).unwrap().write_all(bytes).unwrap();
}

fn run(command: &mut Command, directory: &Path) -> Output {
    let stdout = directory.join("stdout");
    let stderr = directory.join("stderr");
    let mut child = ChildGuard(
        command
            .stdout(File::create_new(&stdout).unwrap())
            .stderr(File::create_new(&stderr).unwrap())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        let captured = fs::metadata(&stdout)
            .unwrap()
            .len()
            .saturating_add(fs::metadata(&stderr).unwrap().len());
        assert!(captured <= OUTPUT_LIMIT * 2, "test capture exceeded cap");
        assert!(Instant::now() < deadline, "owned child exceeded deadline");
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        fs::metadata(&stdout)
            .unwrap()
            .len()
            .saturating_add(fs::metadata(&stderr).unwrap().len())
            <= OUTPUT_LIMIT * 2,
        "completed test capture exceeded cap"
    );
    Output {
        status,
        stdout: fs::read(stdout).unwrap(),
        stderr: fs::read(stderr).unwrap(),
    }
}

fn successful_json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "status={}, stdout={}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

struct Fixture {
    root: TempDir,
    input: PathBuf,
    request: PathBuf,
    request_bytes: Vec<u8>,
    response: Value,
    expected: Value,
}

impl Fixture {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let input = root.path().join("source.txt");
        private_write(&input, SOURCE);
        let request = root.path().join("request.json");
        let request_bytes = serde_json::to_vec(&json!({
            "operation": "create.text", "inputs": [input], "params": {}
        }))
        .unwrap();
        private_write(&request, &request_bytes);
        let captures = root.path().join("captures");
        fs::create_dir(&captures).unwrap();
        let mut fixture = Self {
            root,
            input,
            request,
            request_bytes,
            response: Value::Null,
            expected: Value::Null,
        };
        let mut response = fixture.create_response("create");
        assert_eq!(response["operation"], "create.text");
        assert_eq!(
            response["sources"][0]["sha256"],
            tpe::schema::sha256_hex(SOURCE)
        );
        assert_eq!(response["sources"][0]["bytes"], SOURCE.len());
        assert_eq!(response["sources"].as_array().unwrap().len(), 1);
        assert!(!response["request_id"].as_str().unwrap().is_empty());
        assert_eq!(response["output"]["artifacts"].as_array().unwrap().len(), 1);
        assert!(!artifact_bytes(&response).is_empty());
        // Normal CLI output has already passed through a publisher. Reconstruct
        // the worker-to-publisher state without altering its actual artifacts.
        response["publication_limits"] = Value::Null;
        response["output_paths"] = json!([]);
        response["artifact_receipts"] = json!([]);
        fixture.expected = json!({
            "request_id": response["request_id"],
            "operation": response["operation"],
            "sources": response["sources"],
            "request_options": response["request_options"],
        });
        fixture.response = response;
        fixture.assert_preserved();
        fixture
    }

    fn create_response(&self, label: &str) -> Value {
        let directory = self.root.path().join(label);
        fs::create_dir(&directory).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_tpe"));
        command
            .args(["oxide", "--request"])
            .arg(&self.request)
            .args([
                "--timeout-ms",
                "10000",
                "--max-memory-growth-mib",
                "128",
                "--max-output-bytes",
                "1048576",
            ])
            .env("TMPDIR", self.root.path().join("captures"))
            .env("TMP", self.root.path().join("captures"));
        successful_json(&run(&mut command, &directory))
    }

    fn assert_preserved(&self) {
        assert_eq!(fs::read(&self.input).unwrap(), SOURCE);
        assert_eq!(fs::read(&self.request).unwrap(), self.request_bytes);
        assert_eq!(
            fs::read_dir(self.root.path().join("captures"))
                .unwrap()
                .count(),
            0,
            "supervisor captures leaked"
        );
    }

    fn publish(
        &self,
        label: &str,
        response: &Value,
        expected: &Value,
        capture_digest: Option<&str>,
    ) -> Probe {
        let directory = self.root.path().join(label);
        fs::create_dir(&directory).unwrap();
        let response_path = directory.join("response.json");
        let response_bytes = serde_json::to_vec(response).unwrap();
        assert!(response_bytes.len() as u64 <= OUTPUT_LIMIT);
        private_write(&response_path, &response_bytes);
        let receipt = directory.join("receipt.json");
        let out = directory.join("out");
        fs::create_dir(&out).unwrap();
        private_write(&out.join("sentinel"), SENTINEL);
        let envelope_path = directory.join("publication.json");
        let digest =
            capture_digest.map_or_else(|| tpe::schema::sha256_hex(&response_bytes), str::to_owned);
        let envelope_bytes = serde_json::to_vec(&json!({
            "response": response_path,
            "receipt": receipt,
            "out": out,
            "max_bytes": OUTPUT_LIMIT,
            "expected": expected,
            "response_sha256": digest,
        }))
        .unwrap();
        private_write(&envelope_path, &envelope_bytes);
        let mut command = Command::new(env!("CARGO_BIN_EXE_tpe"));
        command
            .arg("native-worker")
            .arg(&envelope_path)
            .args([
                "--phase",
                "oxide-publish",
                "--growth-bytes",
                "67108864",
                "--parent",
            ])
            .arg(std::process::id().to_string())
            .stdin(Stdio::piped());
        let output = run(&mut command, &directory);
        assert_eq!(fs::read(response_path).unwrap(), response_bytes);
        assert_eq!(fs::read(envelope_path).unwrap(), envelope_bytes);
        assert_eq!(fs::read(out.join("sentinel")).unwrap(), SENTINEL);
        self.assert_preserved();
        Probe {
            output,
            out,
            receipt,
        }
    }
}

struct Probe {
    output: Output,
    out: PathBuf,
    receipt: PathBuf,
}

impl Probe {
    fn assert_rejected(&self) {
        assert!(
            !self.output.status.success(),
            "mismatched response published"
        );
        assert!(
            String::from_utf8_lossy(&self.output.stderr).contains("operation response"),
            "unexpected rejection reason: {}",
            String::from_utf8_lossy(&self.output.stderr)
        );
        assert!(
            self.output.stdout.is_empty(),
            "rejected response was emitted"
        );
        assert!(!self.receipt.exists(), "rejected response created receipt");
        let entries: Vec<_> = fs::read_dir(&self.out)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("sentinel")]);
        assert_eq!(fs::read(self.out.join("sentinel")).unwrap(), SENTINEL);
    }
}

fn artifact_bytes(response: &Value) -> Vec<u8> {
    serde_json::from_value(response["output"]["artifacts"][0]["bytes"].clone()).unwrap()
}

#[test]
fn publisher_rejects_valid_but_unexpected_source_hash() {
    let fixture = Fixture::new();
    let mut response = fixture.response.clone();
    response["sources"][0]["sha256"] = json!(tpe::schema::sha256_hex(b"different owned input"));
    fixture
        .publish("hash", &response, &fixture.expected, None)
        .assert_rejected();
}

#[test]
fn publisher_rejects_source_size_and_count_mismatches() {
    let fixture = Fixture::new();
    let mut wrong_size = fixture.response.clone();
    wrong_size["sources"][0]["bytes"] = json!(SOURCE.len() + 1);
    let mut missing = fixture.response.clone();
    missing["sources"] = json!([]);
    let mut extra = fixture.response.clone();
    extra["sources"] = json!([
        fixture.response["sources"][0],
        fixture.response["sources"][0]
    ]);
    for (label, response) in [("size", wrong_size), ("missing", missing), ("extra", extra)] {
        fixture
            .publish(label, &response, &fixture.expected, None)
            .assert_rejected();
    }
}

#[test]
fn publisher_rejects_operation_and_public_request_option_swaps() {
    let fixture = Fixture::new();
    let mut operation = fixture.response.clone();
    operation["operation"] = json!("create.markdown");
    let mut pages = fixture.response.clone();
    pages["request_options"]["pages"] = json!([1, 1]);
    let mut parameters = fixture.response.clone();
    parameters["request_options"]["parameters_without_passwords"] =
        json!({"title": "Different request"});
    let mut limits = fixture.response.clone();
    limits["request_options"]["limits"]["max_pages"] = json!(1);
    for (label, response) in [
        ("operation", operation),
        ("pages", pages),
        ("parameters", parameters),
        ("limits", limits),
    ] {
        fixture
            .publish(label, &response, &fixture.expected, None)
            .assert_rejected();
    }
}

#[test]
fn publisher_rejects_an_old_response_for_a_fresh_identical_request() {
    let fixture = Fixture::new();
    let fresh = fixture.create_response("fresh-request");
    assert_eq!(fresh["operation"], fixture.response["operation"]);
    assert_eq!(fresh["sources"], fixture.response["sources"]);
    assert_eq!(
        fresh["request_options"],
        fixture.response["request_options"]
    );
    assert_ne!(fresh["request_id"], fixture.response["request_id"]);
    let mut fresh_expected = fixture.expected.clone();
    fresh_expected["request_id"] = fresh["request_id"].clone();
    fixture
        .publish("replay", &fixture.response, &fresh_expected, None)
        .assert_rejected();
}

#[test]
fn publisher_rejects_changed_artifact_bytes_with_the_old_capture_digest() {
    let fixture = Fixture::new();
    let original = serde_json::to_vec(&fixture.response).unwrap();
    let digest = tpe::schema::sha256_hex(&original);
    let mut response = fixture.response.clone();
    let bytes = response["output"]["artifacts"][0]["bytes"]
        .as_array_mut()
        .unwrap();
    let last = bytes.last_mut().unwrap();
    *last = json!(last.as_u64().unwrap() ^ 1);
    fixture
        .publish("artifact", &response, &fixture.expected, Some(&digest))
        .assert_rejected();
}

#[test]
fn publisher_rejects_wrong_build_and_already_published_state() {
    let fixture = Fixture::new();
    let mut backend = fixture.response.clone();
    backend["backend"]["config_digest"] = json!(tpe::schema::sha256_hex(b"other build"));
    let mut producer = fixture.response.clone();
    producer["producer"]["dependencies"]["pdf_oxide"] = json!("0.3.77");
    let mut paths = fixture.response.clone();
    paths["output_paths"] = json!([fixture.root.path().join("not-a-publication")]);
    let mut receipts = fixture.response.clone();
    receipts["artifact_receipts"] = json!([{
        "name": fixture.response["output"]["artifacts"][0]["name"],
        "mime": "application/pdf",
        "bytes": artifact_bytes(&fixture.response).len(),
        "sha256": tpe::schema::sha256_hex(&artifact_bytes(&fixture.response)),
    }]);
    let mut publication = fixture.response.clone();
    publication["publication_limits"] = fixture.response["extraction_limits"].clone();
    for (label, response) in [
        ("backend", backend),
        ("producer", producer),
        ("paths", paths),
        ("receipts", receipts),
        ("publication", publication),
    ] {
        fixture
            .publish(label, &response, &fixture.expected, None)
            .assert_rejected();
    }
}

#[test]
fn publisher_accepts_exact_identity_and_capture_with_unchanged_artifact_bytes() {
    let fixture = Fixture::new();
    let artifact = artifact_bytes(&fixture.response);
    let probe = fixture.publish("valid", &fixture.response, &fixture.expected, None);
    let published = successful_json(&probe.output);
    for name in ["request_id", "operation", "sources", "request_options"] {
        assert_eq!(published[name], fixture.expected[name]);
    }
    assert_eq!(published["backend"], fixture.response["backend"]);
    assert_eq!(published["producer"], fixture.response["producer"]);
    assert!(published["publication_limits"].is_object());
    let paths = published["output_paths"].as_array().unwrap();
    assert_eq!(paths.len(), 1);
    let path = Path::new(paths[0].as_str().unwrap());
    assert_eq!(path.parent(), Some(probe.out.as_path()));
    assert_eq!(fs::read(path).unwrap(), artifact);
    assert_eq!(published["artifact_receipts"].as_array().unwrap().len(), 1);
    assert_eq!(published["artifact_receipts"][0]["bytes"], artifact.len());
    assert_eq!(
        published["artifact_receipts"][0]["sha256"],
        tpe::schema::sha256_hex(&artifact)
    );
    assert_eq!(published["output"]["artifacts"][0]["bytes"], json!([]));
    assert_eq!(fs::read_dir(&probe.out).unwrap().count(), 2);
    let receipt: Value = serde_json::from_slice(&fs::read(&probe.receipt).unwrap()).unwrap();
    assert_eq!(receipt["version"], 1);
    assert_eq!(receipt["output_bytes"], probe.output.stdout.len());
    fixture.assert_preserved();
}
