//! Local inventory first; no extractor runs while discovering source facts.
//! Sources are opened read-only. Archives are opaque files, never expanded.
//! SQLite is a separate, explicitly selected owned destination, with one writer.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, ensure};
use clap::Args;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{ExtractArgs, cli_worker};

const APPLICATION_ID: i64 = 0x5450_4549;
const MAX_ENTRIES: usize = 1_000_000;
const MAX_DEPTH: usize = 64;
const BUFFER: usize = 64 * 1024;

#[derive(Args)]
pub(super) struct InventoryArgs {
    /// Explicit copies to inventory; directories are traversed without following symlinks.
    #[arg(required = true)]
    paths: Vec<PathBuf>,
    /// Separate inventory database. Existing files must be a tpe inventory.
    #[arg(long)]
    db: PathBuf,
    /// List selected roots only; no contents, database, API, or permission checks.
    #[arg(long, short = 'n')]
    dry_run: bool,
    /// Bound directory entries per invocation (a limited scan is not promoted).
    #[arg(long, default_value_t = 100_000)]
    max_entries: usize,
    /// Bound bytes hashed per file; larger files receive a failure record.
    #[arg(long, default_value_t = 1_073_741_824)]
    max_bytes: u64,
    /// Bound elapsed time for each file hash.
    #[arg(long, default_value_t = 60_000)]
    timeout_ms: u64,
    /// Rehash unchanged files during an explicit integrity reconciliation.
    #[arg(long)]
    rehash: bool,
}

#[derive(Args)]
pub(super) struct BatchArgs {
    /// A completed inventory database, created before any extraction.
    #[arg(long)]
    db: PathBuf,
    /// Separate output directory, never inside an inventoried root.
    #[arg(long)]
    out: PathBuf,
    /// Plan from read-only inventory facts without hashing, extracting or writing.
    #[arg(long, short = 'n')]
    dry_run: bool,
    /// Maximum new attempts this invocation; rerun to drain more of the queue.
    #[arg(long, default_value_t = 100)]
    limit: usize,
    /// Retry failed/partial attempts. Verified complete results are always reused.
    #[arg(long)]
    retry: bool,
    #[arg(long, default_value_t = 60_000)]
    timeout_ms: u64,
    #[arg(long, default_value_t = 512)]
    max_memory_growth_mib: u64,
    #[arg(long, default_value_t = 67_108_864)]
    max_output_bytes: u64,
}

/// A descriptor identity, not a promise that metadata alone proves byte equality.
/// ctime catches ordinary same-size rewrites with restored mtime on Unix. Every
/// PDF worker additionally verifies its actual snapshot against the saved hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Identity {
    size: u64,
    modified_ns: u128,
    device: u64,
    inode: u64,
    changed_s: i64,
    changed_ns: i64,
}

fn identity(meta: &fs::Metadata) -> anyhow::Result<Identity> {
    ensure!(meta.is_file(), "not a regular file");
    let modified_ns = meta.modified()?.duration_since(UNIX_EPOCH)?.as_nanos();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(Identity {
            size: meta.len(),
            modified_ns,
            device: meta.dev(),
            inode: meta.ino(),
            changed_s: meta.ctime(),
            changed_ns: meta.ctime_nsec(),
        })
    }
    #[cfg(not(unix))]
    anyhow::bail!("inventory identity currently requires macOS or Linux")
}

fn read_identity(path: &Path) -> anyhow::Result<Identity> {
    identity(&fs::symlink_metadata(path)?)
}

/// No parser, whole-file buffer, decompressor or global input cache is involved.
/// A single fixed buffer and owned descriptor die on every return path.
fn hash_file(
    path: &Path,
    max_bytes: u64,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> anyhow::Result<(Identity, String)> {
    let before = read_identity(path)?;
    ensure!(
        before.size <= max_bytes,
        "file exceeds inventory byte limit"
    );
    #[cfg(unix)]
    let mut file = {
        use rustix::fs::{Mode, OFlags, open};
        File::from(open(
            path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::empty(),
        )?)
    };
    #[cfg(not(unix))]
    let mut file = File::open(path)?;
    ensure!(
        identity(&file.metadata()?)? == before,
        "source changed before read"
    );
    let mut digest = Sha256::new();
    let mut buffer = vec![0; BUFFER].into_boxed_slice();
    let mut total = 0_u64;
    loop {
        ensure!(!cancelled.load(Ordering::Relaxed), "cancelled");
        ensure!(
            Instant::now() < deadline,
            "inventory hash deadline exceeded"
        );
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total = total.checked_add(n as u64).context("size overflow")?;
        ensure!(
            total <= max_bytes && total <= before.size,
            "source grew during read"
        );
        digest.update(&buffer[..n]);
    }
    ensure!(
        total == before.size
            && identity(&file.metadata()?)? == before
            && read_identity(path)? == before,
        "source changed during read"
    );
    Ok((before, hex::encode(digest.finalize())))
}

fn absolute(path: &Path) -> anyhow::Result<PathBuf> {
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(parent
        .canonicalize()?
        .join(path.file_name().context("missing filename")?))
}

fn separate(destination: &Path, roots: &[PathBuf]) -> anyhow::Result<PathBuf> {
    ensure!(
        fs::symlink_metadata(destination).map_or(true, |m| !m.file_type().is_symlink()),
        "destination may not be a symlink"
    );
    let path = absolute(destination)?;
    for root in roots {
        let root = absolute(root)?;
        ensure!(
            !path.starts_with(&root),
            "destination must be outside source roots"
        );
        if let (Ok(a), Ok(b)) = (read_identity(&path), read_identity(&root)) {
            ensure!(
                a.device != b.device || a.inode != b.inode,
                "destination aliases source"
            );
        }
    }
    Ok(path)
}

fn open_owned(path: &Path, create: bool) -> anyhow::Result<(File, Connection)> {
    ensure!(
        fs::symlink_metadata(path).map_or(true, |m| m.is_file() && !m.file_type().is_symlink()),
        "inventory database must be a regular non-symlink file"
    );
    let fresh = !path.exists();
    ensure!(create || !fresh, "inventory database does not exist");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(fresh)
        .open(path)?;
    #[cfg(unix)]
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .context("another inventory/batch writer owns this database")?;
    if !fresh {
        // Validate immutably before opening read-write: SQLite may otherwise
        // recover a hot journal belonging to an unrelated existing database.
        open_readonly(path)?;
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    if fresh {
        conn.execute_batch("PRAGMA application_id=1414546761; PRAGMA user_version=1;
            PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
            CREATE TABLE scans(id INTEGER PRIMARY KEY, roots TEXT NOT NULL, status TEXT NOT NULL, started TEXT NOT NULL);
            CREATE TABLE observations(id INTEGER PRIMARY KEY, scan INTEGER NOT NULL, path TEXT NOT NULL,
                identity TEXT, sha256 TEXT, kind TEXT NOT NULL, status TEXT NOT NULL, detail TEXT NOT NULL);
            CREATE INDEX observation_path ON observations(path,id);
            CREATE TABLE attempts(id INTEGER PRIMARY KEY, observation INTEGER NOT NULL, processing TEXT NOT NULL,
                status TEXT NOT NULL, directory TEXT NOT NULL, artifacts TEXT, detail TEXT NOT NULL);
            CREATE INDEX attempt_source ON attempts(observation,processing,id);")?;
    }
    validate_db(&conn)?;
    if fresh {
        File::open(path.parent().unwrap_or(Path::new(".")))?.sync_all()?;
    }
    Ok((lock, conn))
}

fn validate_db(conn: &Connection) -> anyhow::Result<()> {
    let app: i64 = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(
        app == APPLICATION_ID && version == 1,
        "not a supported tpe inventory database"
    );
    Ok(())
}

fn open_readonly(path: &Path) -> anyhow::Result<Connection> {
    ensure!(path.is_file(), "inventory database does not exist");
    let path = path.canonicalize()?;
    let text = path.to_str().context("inventory path must be UTF-8")?;
    let uri = format!(
        "file:{}?immutable=1",
        text.replace('%', "%25")
            .replace('?', "%3F")
            .replace('#', "%23")
    );
    let conn = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    validate_db(&conn)?;
    Ok(conn)
}

fn kind(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => "pdf",
        "txt" | "md" => "text",
        "zip" | "gz" | "tgz" | "tar" | "docx" | "pptx" | "xlsx" | "pages" | "numbers" | "key" => {
            "container_unsupported"
        }
        "jpg" | "jpeg" | "png" | "tif" | "tiff" | "heic" | "heif" => "image_unsupported",
        _ => "unsupported",
    }
}

fn report(value: &serde_json::Value) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, &value)?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

pub(super) fn inventory(args: &InventoryArgs) -> anyhow::Result<ExitCode> {
    ensure!(
        (1..=MAX_ENTRIES).contains(&args.max_entries),
        "--max-entries must be 1..=1000000"
    );
    ensure!(
        args.max_bytes > 0 && (1..=300_000).contains(&args.timeout_ms),
        "invalid inventory limits"
    );
    if args.dry_run {
        for path in &args.paths {
            report(
                &serde_json::json!({"status":"planned","root":path,"db":args.db,
                "writes":false,"network":false,"unchecked":["contents/checksums","inventory database","Photos/Zotero authorization","destination write access"]}),
            )?;
        }
        return Ok(ExitCode::SUCCESS);
    }
    let roots: Vec<PathBuf> = args
        .paths
        .iter()
        .map(|p| {
            ensure!(
                !fs::symlink_metadata(p)?.file_type().is_symlink(),
                "source root may not be a symlink"
            );
            Ok(p.canonicalize()?)
        })
        .collect::<anyhow::Result<_>>()?;
    let db = separate(&args.db, &roots)?;
    let (_lock, conn) = open_owned(&db, true)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let _signals = cli_worker::CancelSignals::install(&cancelled)?;
    conn.execute(
        "INSERT INTO scans(roots,status,started) VALUES (?1,'running',?2)",
        params![
            serde_json::to_string(&roots)?,
            format!("{:?}", SystemTime::now())
        ],
    )?;
    let scan = conn.last_insert_rowid();
    let mut count = 0;
    let result = (|| -> anyhow::Result<()> {
        for root in &roots {
            visit(root, 0, &mut count, args, &cancelled, &mut |path| {
                observe(&conn, scan, path, args, &cancelled)
            })?;
        }
        ensure!(!cancelled.load(Ordering::Relaxed), "cancelled");
        Ok(())
    })();
    let status = if result.is_ok() {
        "complete"
    } else {
        "interrupted"
    };
    conn.execute(
        "UPDATE scans SET status=?1 WHERE id=?2",
        params![status, scan],
    )?;
    result?;
    conn.execute("INSERT INTO observations(scan,path,kind,status,detail)
        SELECT ?1,o.path,o.kind,'missing','absent during completed namespace reconciliation'
        FROM observations o WHERE o.scan=(SELECT max(id) FROM scans WHERE id<?1 AND roots=?2 AND status='complete')
        AND o.status!='missing' AND NOT EXISTS(SELECT 1 FROM observations n WHERE n.scan=?1 AND n.path=o.path)",
        params![scan,serde_json::to_string(&roots)?])?;
    let failed: i64 = conn.query_row(
        "SELECT count(*) FROM observations WHERE scan=?1 AND status='failed'",
        [scan],
        |r| r.get(0),
    )?;
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn observe(
    conn: &Connection,
    scan: i64,
    path: &Path,
    args: &InventoryArgs,
    cancelled: &AtomicBool,
) -> anyhow::Result<()> {
    let text = path.to_str().context("source path must be UTF-8")?;
    let previous: Option<(String, String)> = conn.query_row(
                    "SELECT identity,sha256 FROM observations WHERE path=?1 AND status='inventoried' ORDER BY id DESC LIMIT 1",
                    [text], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
    let observed = (|| -> anyhow::Result<(Identity, String, bool)> {
        let current = read_identity(path)?;
        if let Some((old, hash)) = previous {
            let old: Identity = serde_json::from_str(&old)?;
            if !args.rehash && old == current {
                return Ok((current, hash, false));
            }
        }
        let (id, hash) = hash_file(
            path,
            args.max_bytes,
            Instant::now() + Duration::from_millis(args.timeout_ms),
            cancelled,
        )?;
        Ok((id, hash, true))
    })();
    let (id, hash, status, detail) = match observed {
        Ok((id, hash, hashed)) => (
            Some(serde_json::to_string(&id)?),
            Some(hash),
            "inventoried",
            if hashed {
                "hashed".to_string()
            } else {
                "metadata_unchanged; checksum reused, not reverified".to_string()
            },
        ),
        Err(error) => (None, None, "failed", format!("{error:#}")),
    };
    conn.execute("INSERT INTO observations(scan,path,identity,sha256,kind,status,detail) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    params![scan,text,id,hash,kind(path),status,detail])?;
    report(
        &serde_json::json!({"scan":scan,"path":path,"status":status,"sha256":hash,"kind":kind(path),"detail":detail}),
    )
}

/// A depth-first iterator keeps at most 64 directory handles and one path;
/// it never collects a million-entry directory or follows directory symlinks.
fn visit(
    path: &Path,
    depth: usize,
    count: &mut usize,
    args: &InventoryArgs,
    cancelled: &AtomicBool,
    each: &mut impl FnMut(&Path) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    ensure!(!cancelled.load(Ordering::Relaxed), "cancelled");
    *count += 1;
    ensure!(
        *count <= args.max_entries && depth <= MAX_DEPTH,
        "inventory traversal limit reached; scan not promoted"
    );
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            visit(&entry?.path(), depth + 1, count, args, cancelled, each)?;
        }
    } else {
        each(path)?;
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Artifact {
    path: PathBuf,
    sha256: String,
    size: u64,
}

pub(super) fn batch(args: &BatchArgs) -> anyhow::Result<ExitCode> {
    ensure!(
        (1..=10_000).contains(&args.limit),
        "--limit must be 1..=10000"
    );
    ensure!(
        (1..=300_000).contains(&args.timeout_ms),
        "--timeout-ms must be 1..=300000"
    );
    ensure!(
        (32..=4096).contains(&args.max_memory_growth_mib),
        "memory allowance must be 32..=4096 MiB"
    );
    ensure!(
        args.max_output_bytes >= 1024,
        "output limit must be at least 1024"
    );
    // Dry-run takes the strictly read-only branch before locks, mkdir or updates.
    let owned = if args.dry_run {
        None
    } else {
        Some(open_owned(&args.db, false)?)
    };
    let read = if args.dry_run {
        Some(open_readonly(&args.db)?)
    } else {
        None
    };
    let conn = owned
        .as_ref()
        .map_or_else(|| read.as_ref().unwrap(), |(_, conn)| conn);
    let (scan, roots): (i64, String) = conn
        .query_row(
            "SELECT id,roots FROM scans WHERE status='complete' ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .context("a complete inventory scan is required before extraction")?;
    let roots: Vec<PathBuf> = serde_json::from_str(&roots)?;
    let out = separate(&args.out, &roots)?;
    ensure!(
        !args.db.canonicalize()?.starts_with(&out),
        "output directory may not contain inventory database"
    );
    let cancelled = Arc::new(AtomicBool::new(false));
    let _signals = if args.dry_run {
        None
    } else {
        Some(cli_worker::CancelSignals::install(&cancelled)?)
    };
    let executable_hash = if args.dry_run {
        "unchecked".to_string()
    } else {
        hash_file(
            &std::env::current_exe()?,
            u64::MAX,
            Instant::now() + Duration::from_millis(args.timeout_ms),
            &cancelled,
        )?
        .1
    };
    let processing = format!(
        "lopdf:{executable_hash}:pages=all:memory={}:timeout={}:output={}",
        args.max_memory_growth_mib, args.timeout_ms, args.max_output_bytes
    );
    drain(conn, scan, &out, &processing, args, &cancelled)
}

fn drain(
    conn: &Connection,
    scan: i64,
    out: &Path,
    processing: &str,
    args: &BatchArgs,
    cancelled: &AtomicBool,
) -> anyhow::Result<ExitCode> {
    let mut errors = conn.prepare(
        "SELECT path,status,detail FROM observations WHERE scan=?1 AND status!='inventoried'",
    )?;
    let mut rows = errors.query([scan])?;
    let mut failed = false;
    while let Some(row) = rows.next()? {
        report(
            &serde_json::json!({"path":row.get::<_,String>(0)?,"status":row.get::<_,String>(1)?,"detail":row.get::<_,String>(2)?}),
        )?;
        failed = true;
    }
    let mut cursor = 0_i64;
    let mut attempts = 0;
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Ok(ExitCode::FAILURE);
        }
        let row: Option<(i64,String,String,String,String)> = conn.query_row(
            "SELECT id,path,identity,sha256,kind FROM observations WHERE scan=?1 AND status='inventoried' AND id>?2 ORDER BY id LIMIT 1",
            params![scan,cursor], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let Some((id, path, identity_json, hash, kind)) = row else {
            break;
        };
        cursor = id;
        if kind != "pdf" {
            report(
                &serde_json::json!({"path":path,"status":"unsupported","kind":kind,"detail":"inventory only; no parser or archive expansion invoked"}),
            )?;
            failed = true;
            continue;
        }
        let previous: Option<(String,Option<String>)> = conn.query_row(
            "SELECT a.status,a.artifacts FROM attempts a JOIN observations o ON a.observation=o.id WHERE o.path=?1 AND o.sha256=?2 AND a.processing=?3 ORDER BY a.id DESC LIMIT 1",
            params![path,hash,processing], |r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if args.dry_run {
            report(
                &serde_json::json!({"path":path,"status":"planned","prior_status":previous.as_ref().map(|p|&p.0),
                "writes":false,"network":false,"unchecked":["changed since inventory","output integrity","PDF validity","runtime availability"]}),
            )?;
            continue;
        }
        let expected: Identity = serde_json::from_str(&identity_json)?;
        if read_identity(Path::new(&path)).ok().as_ref() != Some(&expected) {
            report(
                &serde_json::json!({"path":path,"status":"changed_since_inventory","detail":"run inventory again; no extraction performed"}),
            )?;
            failed = true;
            continue;
        }
        if let Some((status, artifacts)) = &previous {
            if status == "complete"
                && artifacts
                    .as_ref()
                    .is_some_and(|a| verify_artifacts(a, args, cancelled).is_ok())
            {
                report(&serde_json::json!({"path":path,"status":"reused","sha256":hash}))?;
                continue;
            }
            if status != "complete" && status != "running" && status != "interrupted" && !args.retry
            {
                report(
                    &serde_json::json!({"path":path,"status":status,"detail":"retained; use --retry for a new attempt"}),
                )?;
                failed = true;
                continue;
            }
        }
        if attempts >= args.limit {
            report(
                &serde_json::json!({"status":"pending","detail":"attempt limit reached; rerun batch to continue","next_path":path}),
            )?;
            failed = true;
            break;
        }
        attempts += 1;
        failed |= !attempt(
            conn, id, &path, &hash, &expected, out, processing, args, cancelled,
        )?;
    }
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

// The worker and publisher terminate before this scope records a durable result.
#[allow(clippy::too_many_arguments)]
fn attempt(
    conn: &Connection,
    id: i64,
    path: &str,
    hash: &str,
    expected: &Identity,
    out: &Path,
    processing: &str,
    args: &BatchArgs,
    cancelled: &AtomicBool,
) -> anyhow::Result<bool> {
    fs::create_dir_all(out)?;
    conn.execute("INSERT INTO attempts(observation,processing,status,directory,detail) VALUES (?1,?2,'running','','')", params![id,processing])?;
    let attempt = conn.last_insert_rowid();
    // Never adopt a directory left by an interrupted publisher. Fresh
    // create_dir plus no-clobber publication preserves orphan evidence.
    let directory = out.join(format!("attempt-{attempt}"));
    fs::create_dir(&directory).context("attempt destination exists; preserved, not overwritten")?;
    File::open(out)?.sync_all()?;
    conn.execute(
        "UPDATE attempts SET directory=?1 WHERE id=?2",
        params![directory.to_str(), attempt],
    )?;
    report(&serde_json::json!({"path":path,"status":"running","attempt":attempt}))?;
    let extraction = ExtractArgs {
        dry_run: false,
        paths: vec![PathBuf::from(&path)],
        db: directory.join("extraction.sqlite"),
        backend: "lopdf".to_string(),
        out: Some(directory.clone()),
        json: false,
        password: None,
        pages: None,
        jobs: 1,
        max_bytes: Some(expected.size.max(1)),
        timeout_ms: args.timeout_ms,
        max_memory_growth_mib: args.max_memory_growth_mib,
        max_output_bytes: Some(args.max_output_bytes),
        max_files: 1,
        figures_dir: None,
        progress: false,
    };
    let result =
        cli_worker::inventory_job(&extraction, hash, cancelled).and_then(|(status, paths)| {
            finish(path, expected, &directory, status, &paths, args, cancelled)
        });
    let (status, artifacts, detail) = match result {
        Ok((status, artifacts)) => (status, Some(artifacts), String::new()),
        Err(error) => (
            if cancelled.load(Ordering::Relaxed) {
                "interrupted"
            } else {
                "failed"
            }
            .to_string(),
            None,
            format!("{error:#}"),
        ),
    };
    conn.execute(
        "UPDATE attempts SET status=?1,artifacts=?2,detail=?3 WHERE id=?4",
        params![status, artifacts, detail, attempt],
    )?;

    report(
        &serde_json::json!({"path":path,"status":status,"attempt":attempt,"detail":detail,"directory":directory}),
    )?;
    Ok(status == "complete")
}

fn finish(
    source: &str,
    expected: &Identity,
    directory: &Path,
    status: tpe::schema::Status,
    paths: &[PathBuf],
    args: &BatchArgs,
    cancelled: &AtomicBool,
) -> anyhow::Result<(String, String)> {
    ensure!(
        read_identity(Path::new(source))? == *expected,
        "source changed during job; outputs retained but not complete"
    );
    let artifacts = paths
        .iter()
        .map(|path| {
            let (id, hash) = hash_file(
                path,
                args.max_output_bytes,
                Instant::now() + Duration::from_millis(args.timeout_ms),
                cancelled,
            )?;
            File::open(path)?.sync_all()?;
            Ok(Artifact {
                path: path.clone(),
                sha256: hash,
                size: id.size,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    File::open(directory)?.sync_all()?;
    Ok((
        status.as_str().to_string(),
        serde_json::to_string(&artifacts)?,
    ))
}

fn verify_artifacts(json: &str, args: &BatchArgs, cancelled: &AtomicBool) -> anyhow::Result<()> {
    let artifacts: Vec<Artifact> = serde_json::from_str(json)?;
    ensure!(artifacts.len() == 2, "missing output pair");
    for artifact in artifacts {
        let (id, hash) = hash_file(
            &artifact.path,
            args.max_output_bytes,
            Instant::now() + Duration::from_millis(args.timeout_ms),
            cancelled,
        )?;
        ensure!(
            id.size == artifact.size && hash == artifact.sha256,
            "output incomplete or changed"
        );
    }
    Ok(())
}
