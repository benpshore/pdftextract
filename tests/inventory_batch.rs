//! Exercise the inventory and queue through the shipped process boundary.
use rusqlite::Connection;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

struct Work {
    root: TempDir,
    copies: PathBuf,
    db: PathBuf,
    out: PathBuf,
}
impl Work {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let copies = root.path().join("copies");
        fs::create_dir(&copies).unwrap();
        Self {
            db: root.path().join("inventory.sqlite"),
            out: root.path().join("outputs"),
            copies,
            root,
        }
    }
    fn pdf(&self, name: &str) -> PathBuf {
        let path = self.copies.join(name);
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native-worker/native.pdf"),
            &path,
        )
        .unwrap();
        path
    }
    fn inventory(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_tpe"));
        c.arg("inventory")
            .arg(&self.copies)
            .arg("--db")
            .arg(&self.db);
        c
    }
    fn batch(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_tpe"));
        c.arg("batch")
            .arg("--db")
            .arg(&self.db)
            .arg("--out")
            .arg(&self.out);
        c
    }
    fn attempts(&self) -> i64 {
        Connection::open(&self.db)
            .unwrap()
            .query_row("SELECT count(*) FROM attempts", [], |r| r.get(0))
            .unwrap()
    }
}
fn records(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn status(output: &Output, value: &str) -> bool {
    records(output).iter().any(|r| r["status"] == value)
}

#[test]
fn inventory_precedes_jobs_reconciliation_reuses_hashes_and_reports_changes() {
    let w = Work::new();
    let a = w.pdf("a.pdf");
    assert!(!w.batch().output().unwrap().status.success());
    let first = w.inventory().output().unwrap();
    success(&first);
    assert_eq!(records(&first)[0]["detail"], "hashed");
    assert_eq!(w.attempts(), 0);
    assert!(!w.out.exists());
    let again = w.inventory().output().unwrap();
    success(&again);
    assert!(
        records(&again)[0]["detail"]
            .as_str()
            .unwrap()
            .starts_with("metadata_unchanged")
    );
    let original = fs::read(&a).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&a)
        .unwrap()
        .write_all(b"\n% changed\n")
        .unwrap();
    let stale = w.batch().output().unwrap();
    assert!(status(&stale, "changed_since_inventory"));
    assert_eq!(w.attempts(), 0);
    let changed = w.inventory().output().unwrap();
    success(&changed);
    assert_eq!(records(&changed)[0]["detail"], "hashed");
    assert_ne!(records(&first)[0]["sha256"], records(&changed)[0]["sha256"]);
    let forced = w.inventory().arg("--rehash").output().unwrap();
    success(&forced);
    assert_eq!(records(&forced)[0]["detail"], "hashed");
    fs::rename(&a, w.copies.join("renamed.pdf")).unwrap();
    success(&w.inventory().output().unwrap());
    let db = Connection::open(&w.db).unwrap();
    let missing: i64 = db.query_row("SELECT count(*) FROM observations WHERE scan=(SELECT max(id) FROM scans) AND status='missing'", [], |r|r.get(0)).unwrap();
    assert_eq!(missing, 1);
    assert_eq!(
        &fs::read(w.copies.join("renamed.pdf")).unwrap()[..original.len()],
        &original
    );
}

#[test]
fn bounded_resume_reuses_complete_verifies_artifacts_and_preserves_sources() {
    let w = Work::new();
    let a = w.pdf("a.pdf");
    w.pdf("b.pdf");
    let before = fs::read(&a).unwrap();
    success(&w.inventory().output().unwrap());
    let first = w.batch().args(["--limit", "1"]).output().unwrap();
    assert!(status(&first, "complete") && status(&first, "pending"));
    assert_eq!(w.attempts(), 1);
    let second = w.batch().output().unwrap();
    success(&second);
    assert!(status(&second, "reused") && status(&second, "complete"));
    assert_eq!(w.attempts(), 2);
    let again = w.batch().output().unwrap();
    success(&again);
    assert_eq!(
        records(&again)
            .iter()
            .filter(|r| r["status"] == "reused")
            .count(),
        2
    );
    assert_eq!(w.attempts(), 2);
    let db = Connection::open(&w.db).unwrap();
    let artifacts: String = db
        .query_row("SELECT artifacts FROM attempts WHERE id=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    let parsed: Value = serde_json::from_str(&artifacts).unwrap();
    let artifact = PathBuf::from(parsed[0]["path"].as_str().unwrap());
    fs::write(&artifact, b"interrupted or corrupted export").unwrap();
    let repair = w.batch().output().unwrap();
    success(&repair);
    assert!(status(&repair, "complete"));
    assert_eq!(w.attempts(), 3);
    assert_eq!(
        fs::read(artifact).unwrap(),
        b"interrupted or corrupted export"
    );
    assert_eq!(fs::read(a).unwrap(), before);
}

#[test]
fn partial_failures_and_opaque_archives_are_never_done() {
    let w = Work::new();
    let a = w.pdf("partial.pdf");
    let mut pdf = lopdf::Document::load(&a).unwrap();
    for object in pdf.objects.values_mut() {
        if let Ok(font) = object.as_dict_mut() {
            if font.has(b"BaseFont") {
                font.set("BaseFont", lopdf::Object::Name(b"UnknownFont".to_vec()));
            }
        }
    }
    pdf.save(&a).unwrap();
    fs::write(w.copies.join("bad.pdf"), b"not a PDF").unwrap();
    fs::write(
        w.copies.join("archive.zip"),
        b"opaque archive; no member enumeration",
    )
    .unwrap();
    success(&w.inventory().output().unwrap());
    let first = w.batch().output().unwrap();
    assert!(!first.status.success());
    for state in ["partial", "failed", "unsupported"] {
        assert!(status(&first, state), "{state}: {:?}", records(&first));
    }
    assert_eq!(w.attempts(), 2);
    let second = w.batch().output().unwrap();
    assert!(!second.status.success());
    assert_eq!(w.attempts(), 2);
    assert!(!status(&second, "reused"));
    let retry = w.batch().arg("--retry").output().unwrap();
    assert!(!retry.status.success());
    assert_eq!(w.attempts(), 4);
}

#[test]
fn both_dry_run_spellings_leave_sources_db_outputs_and_temp_unchanged() {
    let w = Work::new();
    let input = w.pdf("native.pdf");
    let source = fs::read(&input).unwrap();
    for flag in ["-n", "--dry-run"] {
        success(&w.inventory().arg(flag).output().unwrap());
        assert!(!w.db.exists());
        let output = Command::new(env!("CARGO_BIN_EXE_tpe"))
            .arg("extract")
            .arg(flag)
            .arg(&input)
            .arg("--db")
            .arg(&w.db)
            .arg("--out")
            .arg(&w.out)
            .output()
            .unwrap();
        success(&output);
        assert_eq!(records(&output)[0]["network"], false);
        assert!(!w.db.exists() && !w.out.exists());
    }
    success(&w.inventory().output().unwrap());
    let database = fs::read(&w.db).unwrap();
    let entries = fs::read_dir(w.root.path()).unwrap().count();
    for flag in ["-n", "--dry-run"] {
        let output = w.batch().arg(flag).output().unwrap();
        success(&output);
        assert!(status(&output, "planned"));
        assert_eq!(fs::read(&w.db).unwrap(), database);
        assert_eq!(fs::read(&input).unwrap(), source);
        assert!(!w.out.exists());
        assert_eq!(fs::read_dir(w.root.path()).unwrap().count(), entries);
    }
}

#[test]
fn limited_inventory_and_unrelated_database_do_not_authorize_extraction() {
    let w = Work::new();
    w.pdf("a.pdf");
    w.pdf("b.pdf");
    let limited = w.inventory().args(["--max-entries", "1"]).output().unwrap();
    assert!(!limited.status.success());
    assert!(!w.batch().output().unwrap().status.success());
    assert!(!w.out.exists());
    let other = Work::new();
    other.pdf("a.pdf");
    let conn = Connection::open(&other.db).unwrap();
    conn.execute_batch(
        "CREATE TABLE important(value TEXT); INSERT INTO important VALUES ('keep');",
    )
    .unwrap();
    drop(conn);
    let bytes = fs::read(&other.db).unwrap();
    assert!(!other.inventory().output().unwrap().status.success());
    assert_eq!(fs::read(&other.db).unwrap(), bytes);
}

#[cfg(unix)]
#[test]
fn real_worker_interruption_releases_ownership_and_resumes_in_new_attempt() {
    let w = Work::new();
    let a = w.pdf("many.pdf");
    let mut pdf = lopdf::Document::load(&a).unwrap();
    let template = pdf.get_object(pdf.get_pages()[&1]).unwrap().clone();
    let parent = template
        .as_dict()
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    let kids: Vec<lopdf::Object> = (0..2000)
        .map(|_| pdf.add_object(template.clone()).into())
        .collect();
    let pages = pdf.get_object_mut(parent).unwrap().as_dict_mut().unwrap();
    pages.set("Kids", kids);
    pages.set("Count", 2000);
    pdf.save(&a).unwrap();
    drop(pdf);
    let before = fs::read(&a).unwrap();
    success(&w.inventory().output().unwrap());
    let log = w.root.path().join("interrupted.jsonl");
    let mut child = w
        .batch()
        .stdout(fs::File::create(&log).unwrap())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let worker = loop {
        let ps = Command::new("ps")
            .args(["-axo", "pid=,ppid="])
            .output()
            .unwrap();
        let found = String::from_utf8(ps.stdout)
            .unwrap()
            .lines()
            .find_map(|line| {
                let mut fields = line.split_whitespace();
                let pid = fields.next()?;
                (fields.next()? == child.id().to_string()).then(|| pid.to_string())
            });
        if let Some(pid) = found {
            break pid;
        }
        if Instant::now() > deadline || child.try_wait().unwrap().is_some() {
            let _ = child.kill();
            let _ = child.wait();
            panic!("no worker became observable");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(
        Command::new("kill")
            .args(["-STOP", &worker])
            .status()
            .unwrap()
            .success()
    );
    let concurrent = w.batch().output().unwrap();
    assert!(!concurrent.status.success());
    assert!(String::from_utf8_lossy(&concurrent.stderr).contains("another inventory/batch writer"));
    assert!(
        Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("controller failed to exit");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !Command::new("kill")
            .args(["-0", &worker])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    assert!(fs::read_to_string(log).unwrap().contains("interrupted"));
    let resumed = w.batch().output().unwrap();
    success(&resumed);
    assert!(status(&resumed, "complete"));
    assert_eq!(w.attempts(), 2);
    assert_eq!(fs::read(a).unwrap(), before);
}
