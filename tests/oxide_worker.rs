//! Bounded end-to-end tests of the shipped PDF Oxide subprocess controller.
//! All inputs and attempted action targets are synthetic and disposable.
#![cfg(feature = "pdf-oxide")]

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

use lopdf::{Document, Object, Stream, dictionary};
use serde_json::{Value, json};
use tempfile::TempDir;

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn wait_bounded(child: &mut Child, timeout: Duration) -> ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("owned test child exceeded its deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn command(root: &Path, timeout_ms: u64) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tpe"));
    command
        .args(["oxide", "--request"])
        .arg(root.join("request.json"))
        .arg("--out")
        .arg(root.join("out"))
        .arg("--timeout-ms")
        .arg(timeout_ms.to_string());
    let captures = root.join("captures");
    fs::create_dir_all(&captures).unwrap();
    command.env("TMPDIR", &captures).env("TMP", captures);
    command
}

fn request(root: &Path, value: &Value) -> PathBuf {
    let path = root.join("request.json");
    private_write(&path, &serde_json::to_vec(value).unwrap());
    path
}

fn private_write(path: &Path, bytes: &[u8]) {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).unwrap().write_all(bytes).unwrap();
}

fn run(mut command: Command, root: &Path) -> Output {
    let stdout = root.join("controller.stdout");
    let stderr = root.join("controller.stderr");
    let mut child = ChildGuard(
        command
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap())
            .spawn()
            .unwrap(),
    );
    let status = wait_bounded(&mut child.0, Duration::from_secs(20));
    Output {
        status,
        stdout: fs::read(stdout).unwrap(),
        stderr: fs::read(stderr).unwrap(),
    }
}

fn record(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON {error}; status={}, stdout={}, stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn success(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    record(output)
}

fn empty_publication(root: &Path) {
    let out = root.join("out");
    assert!(!out.exists() || fs::read_dir(out).unwrap().next().is_none());
}

fn clean_captures(root: &Path) {
    assert_eq!(
        fs::read_dir(root.join("captures")).unwrap().count(),
        0,
        "private captures leaked"
    );
}

fn fixture(root: &Path) -> (PathBuf, Vec<u8>) {
    let bytes = tpe::backend::probe_pdf().unwrap();
    let input = root.join("input.pdf");
    fs::write(&input, &bytes).unwrap();
    (input, bytes)
}

fn save(mut document: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn assert_limit_evidence(result: &Value) {
    for phase in ["extraction_limits", "publication_limits"] {
        let evidence = &result[phase];
        assert_eq!(evidence["kind"], "address_space_growth");
        let startup = evidence["startup_virtual_bytes"].as_u64().unwrap();
        let growth = evidence["requested_growth_bytes"].as_u64().unwrap();
        let effective = evidence["effective_address_space_bytes"].as_u64().unwrap();
        assert!(startup > 0 && effective > startup && effective <= startup + growth);
        assert_eq!(evidence["core_dump_bytes"], 0);
    }
}

#[test]
fn creation_then_query_uses_workers_and_preserves_exact_source_provenance() {
    let root = TempDir::new().unwrap();
    let text = b"Synthetic worker integration control.\nSecond stable line.";
    let input = root.path().join("source.txt");
    fs::write(&input, text).unwrap();
    request(
        root.path(),
        &json!({"operation":"create.text","inputs":[input],"params":{}}),
    );
    let created = success(&run(command(root.path(), 10_000), root.path()));
    assert_eq!(created["operation"], "create.text");
    assert_eq!(created["backend"]["version"], "0.3.78");
    assert_eq!(
        created["sources"][0]["sha256"],
        tpe::schema::sha256_hex(text)
    );
    assert_eq!(created["sources"][0]["bytes"], text.len());
    assert_limit_evidence(&created);
    let path = PathBuf::from(created["output_paths"][0].as_str().unwrap());
    let pdf = fs::read(&path).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    assert_eq!(created["artifact_receipts"][0]["bytes"], pdf.len());
    assert_eq!(
        created["artifact_receipts"][0]["sha256"],
        tpe::schema::sha256_hex(&pdf)
    );
    assert_eq!(created["artifact_receipts"][0]["mime"], "application/pdf");
    assert!(
        Document::load_mem(&pdf)
            .unwrap()
            .extract_text(&[1])
            .unwrap()
            .contains("Synthetic worker integration control")
    );
    request(
        root.path(),
        &json!({"operation":"query.text","inputs":[path],"params":{}}),
    );
    let queried = success(&run(command(root.path(), 10_000), root.path()));
    assert_eq!(
        queried["sources"][0]["sha256"],
        tpe::schema::sha256_hex(&pdf)
    );
    assert!(
        queried["output"]["data"]["result"][0]["value"]
            .as_str()
            .unwrap()
            .contains("Second stable line")
    );
    assert_eq!(queried["output"]["data"]["completeness"], "unverified");
    assert_limit_evidence(&queried);
    assert_eq!(fs::read(input).unwrap(), text);
    assert_eq!(fs::read(path).unwrap(), pdf);
    clean_captures(root.path());
}

#[test]
fn atomic_publication_preserves_existing_files_and_repeated_outputs() {
    let root = TempDir::new().unwrap();
    let (input, original) = fixture(root.path());
    request(
        root.path(),
        &json!({"operation":"security.decrypt","inputs":[input],"params":{}}),
    );
    fs::create_dir(root.path().join("out")).unwrap();
    let sentinel = root.path().join("out").join("sentinel.pdf");
    fs::write(&sentinel, b"caller-owned sentinel").unwrap();
    let first = success(&run(command(root.path(), 10_000), root.path()));
    let first_path = first["output_paths"][0].as_str().unwrap();
    let first_bytes = fs::read(first_path).unwrap();
    let second = success(&run(command(root.path(), 10_000), root.path()));
    assert_ne!(first["output_paths"], second["output_paths"]);
    assert_eq!(fs::read(first_path).unwrap(), first_bytes);
    assert_eq!(fs::read(&sentinel).unwrap(), b"caller-owned sentinel");
    assert_eq!(fs::read(input).unwrap(), original);
    assert_eq!(fs::read_dir(root.path().join("out")).unwrap().count(), 3);
    clean_captures(root.path());
}

#[test]
fn publisher_failure_leaves_source_and_existing_target_unchanged() {
    let root = TempDir::new().unwrap();
    let (input, original) = fixture(root.path());
    request(
        root.path(),
        &json!({"operation":"security.decrypt","inputs":[input],"params":{}}),
    );
    // An ordinary existing file at the requested directory is a bounded
    // publication fault; no permission changes or external paths are needed.
    fs::write(root.path().join("out"), b"existing caller file").unwrap();
    let output = run(command(root.path(), 10_000), root.path());
    assert!(!output.status.success());
    assert_eq!(record(&output)["status"], "publication_unknown");
    assert_eq!(
        fs::read(root.path().join("out")).unwrap(),
        b"existing caller file"
    );
    assert_eq!(fs::read(input).unwrap(), original);
    clean_captures(root.path());
}

#[test]
fn receipt_write_failure_rolls_back_published_artifacts() {
    let root = TempDir::new().unwrap();
    let source = b"Synthetic atomic publication receipt control.";
    let input = root.path().join("source.txt");
    fs::write(&input, source).unwrap();
    let request_path = request(
        root.path(),
        &json!({"operation":"create.text","inputs":[input],"params":{}}),
    );
    let mut create = Command::new(env!("CARGO_BIN_EXE_tpe"));
    create.args(["oxide", "--request"]).arg(request_path);
    create.args(["--timeout-ms", "10000"]);
    let response = success(&run(create, root.path()));
    assert!(
        response["output"]["artifacts"][0]["bytes"]
            .as_array()
            .unwrap()
            .len()
            > 100
    );
    let response_path = root.path().join("response.json");
    private_write(&response_path, &serde_json::to_vec(&response).unwrap());
    let receipt = root.path().join("receipt-is-a-directory");
    fs::create_dir(&receipt).unwrap();
    fs::write(receipt.join("sentinel"), b"caller-owned receipt sentinel").unwrap();
    let publication = root.path().join("publication.json");
    private_write(
        &publication,
        &serde_json::to_vec(&json!({
            "response": response_path, "receipt": receipt,
            "out": root.path().join("out"), "max_bytes": 1_048_576
        }))
        .unwrap(),
    );
    let mut publish = Command::new(env!("CARGO_BIN_EXE_tpe"));
    publish
        .arg("native-worker")
        .arg(publication)
        .args([
            "--phase",
            "oxide-publish",
            "--growth-bytes",
            "67108864",
            "--parent",
        ])
        .arg(std::process::id().to_string())
        .stdin(Stdio::piped());
    let result = run(publish, root.path());
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("directory"));
    empty_publication(root.path());
    assert_eq!(
        fs::read(receipt.join("sentinel")).unwrap(),
        b"caller-owned receipt sentinel"
    );
    assert_eq!(fs::read(input).unwrap(), source);
}

#[test]
fn unconfined_routed_extraction_fails_before_opening_ledger_or_outputs() {
    let root = TempDir::new().unwrap();
    let (input, original) = fixture(root.path());
    let mut extract = Command::new(env!("CARGO_BIN_EXE_tpe"));
    extract
        .args([
            "extract",
            "--backend",
            "routed",
            "--timeout-ms",
            "1000",
            "--db",
        ])
        .arg(root.path().join("ledger.sqlite"))
        .arg("--out")
        .arg(root.path().join("out"))
        .arg(&input);
    let result = run(extract, root.path());
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("no qualified filesystem containment path")
    );
    assert!(!root.path().join("ledger.sqlite").exists());
    empty_publication(root.path());
    assert_eq!(fs::read(input).unwrap(), original);
}

#[test]
fn malformed_unknown_and_out_of_range_requests_fail_without_publication() {
    for case in 0..5 {
        let root = TempDir::new().unwrap();
        let (input, original) = fixture(root.path());
        let value = match case {
            0 => json!({"operation":"not.an.operation","inputs":[input],"params":{}}),
            1 => {
                json!({"operation":"query.text","inputs":[input],"params":{"output_path":"unapproved"}})
            }
            2 => json!({"operation":"query.text","inputs":[input],"pages":[2,2],"params":{}}),
            3 => {
                json!({"operation":"query.text","inputs":[input],"pages":[1,u32::MAX],"params":{}})
            }
            _ => {
                fs::write(&input, b"malformed synthetic input").unwrap();
                json!({"operation":"query.text","inputs":[input],"params":{}})
            }
        };
        request(root.path(), &value);
        let output = run(command(root.path(), 10_000), root.path());
        assert!(!output.status.success(), "case {case}");
        assert_eq!(record(&output)["status"], "failed");
        empty_publication(root.path());
        clean_captures(root.path());
        if case != 4 {
            assert_eq!(fs::read(input).unwrap(), original);
        }
    }
}

#[test]
fn output_overflow_fails_before_publication_and_removes_captures() {
    let root = TempDir::new().unwrap();
    let (input, original) = fixture(root.path());
    request(
        root.path(),
        &json!({"operation":"security.decrypt","inputs":[input],"params":{}}),
    );
    let mut cmd = command(root.path(), 10_000);
    cmd.args(["--max-output-bytes", "1024"]);
    let output = run(cmd, root.path());
    assert!(!output.status.success());
    assert_eq!(record(&output)["status"], "failed");
    empty_publication(root.path());
    clean_captures(root.path());
    assert_eq!(fs::read(input).unwrap(), original);
}

#[test]
fn passwords_use_private_request_and_file_channels_not_argv_or_reports() {
    let root = TempDir::new().unwrap();
    let (input, original) = fixture(root.path());
    let user_password = "synthetic-private-user";
    let owner_password = "synthetic-private-owner";
    request(
        root.path(),
        &json!({"operation":"security.encrypt","inputs":[input],"params":{"user_password":user_password,"owner_password":owner_password}}),
    );
    let output = run(command(root.path(), 10_000), root.path());
    let created = success(&output);
    for bytes in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(bytes);
        assert!(!text.contains(user_password) && !text.contains(owner_password));
    }
    let encrypted = PathBuf::from(created["output_paths"][0].as_str().unwrap());
    let password_file = root.path().join("password.txt");
    private_write(&password_file, user_password.as_bytes());
    request(
        root.path(),
        &json!({"operation":"security.decrypt","inputs":[encrypted],"params":{},"password_file":password_file}),
    );
    let output = run(command(root.path(), 10_000), root.path());
    let decrypted = success(&output);
    let recovered = fs::read(decrypted["output_paths"][0].as_str().unwrap()).unwrap();
    let pdf = Document::load_mem(&recovered).unwrap();
    assert!(!pdf.is_encrypted());
    assert!(pdf.extract_text(&[1]).unwrap().contains("probe"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains(user_password));
    assert_eq!(fs::read(input).unwrap(), original);
    clean_captures(root.path());
}

fn pressure_pdf() -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    // Construct compressed 128 MiB comments without retaining the decoded
    // bytes in the test host. Only the 32 MiB-budget child decodes the stream.
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    const BLOCK_BYTES: usize = 64 * 1024;
    let mut block = [b' '; BLOCK_BYTES];
    block[0] = b'%';
    block[BLOCK_BYTES - 1] = b'\n';
    for _ in 0..2048 {
        encoder.write_all(&block).unwrap();
    }
    let compressed = encoder.finish().unwrap();
    let mut pdf = Document::load_mem(&tpe::backend::probe_pdf().unwrap()).unwrap();
    let stream = pdf.add_object(Stream::new(
        dictionary! {"Filter"=>"FlateDecode"},
        compressed,
    ));
    let page = pdf.get_pages()[&1];
    pdf.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Contents", stream);
    save(pdf)
}

#[test]
fn bounded_memory_failure_does_not_poison_the_next_operation() {
    let root = TempDir::new().unwrap();
    let input = root.path().join("pressure.pdf");
    let bytes = pressure_pdf();
    assert!(bytes.len() < 2 * 1024 * 1024);
    fs::write(&input, &bytes).unwrap();
    request(
        root.path(),
        &json!({"operation":"query.content","inputs":[input],"params":{}}),
    );
    let mut cmd = command(root.path(), 8_000);
    cmd.args(["--max-memory-growth-mib", "32"]);
    let failed = run(cmd, root.path());
    assert!(!failed.status.success());
    assert_eq!(record(&failed)["status"], "failed");
    empty_publication(root.path());
    clean_captures(root.path());
    assert_eq!(fs::read(&input).unwrap(), bytes);
    let (next, _) = fixture(root.path());
    request(
        root.path(),
        &json!({"operation":"query.text","inputs":[next],"params":{}}),
    );
    let result = success(&run(command(root.path(), 10_000), root.path()));
    assert!(
        result["output"]["data"]["result"][0]["value"]
            .as_str()
            .unwrap()
            .contains("probe")
    );
    clean_captures(root.path());
}

#[cfg(unix)]
fn child_pid(controller: &mut Child) -> String {
    let expected_parent = controller.id().to_string();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let result = Command::new("ps")
            .args(["-axo", "pid=,ppid="])
            .output()
            .unwrap();
        if let Some(pid) = String::from_utf8_lossy(&result.stdout)
            .lines()
            .find_map(|line| {
                let mut fields = line.split_whitespace();
                let pid = fields.next()?;
                (fields.next()? == expected_parent).then(|| pid.to_owned())
            })
        {
            return pid;
        }
        assert!(
            controller.try_wait().unwrap().is_none(),
            "controller exited before worker observation"
        );
        assert!(
            Instant::now() < deadline,
            "worker did not become observable"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(unix)]
fn signal(pid: &str, name: &str) {
    assert!(
        Command::new("kill")
            .args([name, pid])
            .status()
            .unwrap()
            .success()
    );
}

#[cfg(unix)]
fn not_running(pid: &str) -> bool {
    !Command::new("kill")
        .args(["-0", pid])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

#[cfg(unix)]
fn bounded_many_pages() -> Vec<u8> {
    let mut pdf = Document::load_mem(&tpe::backend::probe_pdf().unwrap()).unwrap();
    let template = pdf.get_object(pdf.get_pages()[&1]).unwrap().clone();
    let tree = template
        .as_dict()
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    let children: Vec<Object> = (0..256)
        .map(|_| pdf.add_object(template.clone()).into())
        .collect();
    let pages = pdf.get_object_mut(tree).unwrap().as_dict_mut().unwrap();
    pages.set("Kids", children);
    pages.set("Count", 256);
    save(pdf)
}

#[cfg(unix)]
#[test]
fn deadline_and_cancellation_kill_reap_and_clean_a_stopped_worker() {
    for cancellation in [None, Some("-INT"), Some("-TERM")] {
        let root = TempDir::new().unwrap();
        let bytes = bounded_many_pages();
        let input = root.path().join("many.pdf");
        fs::write(&input, &bytes).unwrap();
        request(
            root.path(),
            &json!({"operation":"query.text","inputs":[input],"params":{}}),
        );
        let stdout = root.path().join("controller.stdout");
        let mut child = ChildGuard(
            command(root.path(), 1_500)
                .stdout(File::create(&stdout).unwrap())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let worker = child_pid(&mut child.0);
        signal(&worker, "-STOP");
        if let Some(name) = cancellation {
            signal(&child.0.id().to_string(), name);
        }
        assert!(!wait_bounded(&mut child.0, Duration::from_secs(5)).success());
        let value: Value = serde_json::from_slice(&fs::read(stdout).unwrap()).unwrap();
        assert_eq!(value["status"], "failed");
        assert!(
            value["error"]
                .as_str()
                .unwrap()
                .contains(if cancellation.is_some() {
                    "cancelled"
                } else {
                    "timed out"
                })
        );
        assert!(not_running(&worker), "stopped worker was not reaped");
        empty_publication(root.path());
        clean_captures(root.path());
        assert_eq!(fs::read(input).unwrap(), bytes);
    }
}

#[cfg(unix)]
#[test]
fn stdout_backpressure_obeys_deadline_without_changing_pipe_flags() {
    use std::os::fd::AsFd;
    let root = TempDir::new().unwrap();
    let mut pdf = Document::load_mem(&tpe::backend::probe_pdf().unwrap()).unwrap();
    let info = pdf.add_object(
        dictionary! {"Producer"=>Object::string_literal("SYNTHETIC_BACKPRESSURE_".repeat(8192))},
    );
    pdf.trailer.set("Info", info);
    let bytes = save(pdf);
    let input = root.path().join("metadata.pdf");
    fs::write(&input, &bytes).unwrap();
    request(
        root.path(),
        &json!({"operation":"query.document","inputs":[input],"params":{}}),
    );
    // Only the duplicated stdout may reach the controller. Extra inherited
    // pipe ends correctly fail the sealed worker's descriptor admission.
    let (read_end, write_end) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).unwrap();
    let before = rustix::fs::fcntl_getfl(&write_end).unwrap();
    let started = Instant::now();
    let stderr = root.path().join("controller.stderr");
    let mut child = ChildGuard(
        command(root.path(), 3_000)
            .stdout(Stdio::from(write_end.as_fd().try_clone_to_owned().unwrap()))
            .stderr(File::create(&stderr).unwrap())
            .spawn()
            .unwrap(),
    );
    assert!(!wait_bounded(&mut child.0, Duration::from_secs(5)).success());
    let elapsed = started.elapsed();
    let after = rustix::fs::fcntl_getfl(&write_end).unwrap();
    let changed = (before ^ after).bits();
    #[cfg(target_os = "macos")]
    let changed = changed & !0x0001_0000;
    assert_eq!(changed, 0, "controller changed caller pipe flags");
    drop(write_end);
    let mut output = Vec::new();
    std::io::Read::read_to_end(&mut File::from(read_end), &mut output).unwrap();
    assert!(
        elapsed >= Duration::from_secs(2),
        "controller failed before backpressure reached its deadline: stdout={} stderr={}",
        String::from_utf8_lossy(&output),
        fs::read_to_string(&stderr).unwrap(),
    );
    assert!(output.len() > 1024, "test did not fill the output pipe");
    assert!(
        String::from_utf8_lossy(&output).contains("\"backend\"")
            && String::from_utf8_lossy(&output).contains("SYNTHETIC_BACKPRESSURE_"),
        "partial output did not contain the actual operation report: {}",
        String::from_utf8_lossy(&output),
    );
    clean_captures(root.path());
    assert_eq!(fs::read(input).unwrap(), bytes);
}

#[test]
fn document_links_and_actions_are_data_and_do_not_trigger_loopback_requests() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "http://{}/synthetic-no-fetch",
        listener.local_addr().unwrap()
    );
    let root = TempDir::new().unwrap();
    let marker = root.path().join("action-was-executed");
    let script = root.path().join("synthetic-action.sh");
    // A disposable marker is the only effect even if an erroneous launch
    // occurs. No external command/data/service is targeted by this fixture.
    private_write(
        &script,
        format!("#!/bin/sh\nprintf synthetic > '{}'\n", marker.display()).as_bytes(),
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut pdf = Document::load_mem(&tpe::backend::probe_pdf().unwrap()).unwrap();
    let uri = pdf.add_object(dictionary! {"S"=>"URI","URI"=>Object::string_literal(url.as_str())});
    let launch = pdf.add_object(dictionary!{"S"=>"Launch","F"=>Object::string_literal(script.to_string_lossy().as_bytes())});
    let js = pdf.add_object(dictionary!{"S"=>"JavaScript","JS"=>Object::string_literal(format!("app.launchURL('{url}', false);"))});
    let link = pdf.add_object(dictionary!{"Type"=>"Annot","Subtype"=>"Link","A"=>uri,"Rect"=>vec![0.into(),0.into(),10.into(),10.into()]});
    let page = pdf.get_pages()[&1];
    pdf.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", vec![link.into()]);
    let catalog = pdf.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let catalog = pdf.get_object_mut(catalog).unwrap().as_dict_mut().unwrap();
    catalog.set("OpenAction", launch);
    catalog.set("AA", dictionary! {"WC"=>js});
    let original = save(pdf);
    let input = root.path().join("actions.pdf");
    fs::write(&input, &original).unwrap();
    request(
        root.path(),
        &json!({"operation":"query.text","inputs":[input],"params":{}}),
    );
    let result = success(&run(command(root.path(), 10_000), root.path()));
    assert!(
        result["output"]["data"]["result"][0]["value"]
            .as_str()
            .unwrap()
            .contains("probe")
    );
    assert!(!marker.exists(), "PDF action executed");
    assert!(
        matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock),
        "unexpected loopback connection"
    );
    assert_eq!(fs::read(input).unwrap(), original);
    clean_captures(root.path());
}
