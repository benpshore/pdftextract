//! The disposable worker boundary. Every scan runs in a child process of
//! this same binary (`tpe-scan-service worker <json>`): the parent streams
//! the input over stdin, reads one JSON line from stdout, and kills the child
//! at the deadline or on shutdown. The child installs a hard address-space
//! limit before it reads a byte, mirroring the CLI's extraction workers, so
//! a runaway conversion fails inside the child instead of the service.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::image_pdf::{self, ImageError, InputKind};
use crate::scan::{ErrorCode, ScanError, ScanOptions, ScanResult, scan_bytes};

/// Subcommand name the child is started with.
pub const SUBCOMMAND: &str = "worker";
/// Largest JSON result the parent reads back (OCR text of a 50-page window
/// with spans and blocks stays far below this).
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024 * 1024;
/// Stderr kept from a failed child, for the error message.
const STDERR_TAIL_BYTES: usize = 4096;

/// What the parent asks a child to do; passed as one JSON argument.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerRequest {
    pub options: ScanOptions,
    /// Most input bytes the child reads from stdin.
    pub max_bytes: u64,
    /// Address-space growth the child allows itself above its start-up mappings.
    pub memory_growth_mib: u64,
    /// The parent's PID; the child refuses to run for anyone else.
    pub parent_pid: u32,
}

/// What the child prints: exactly one of these, as JSON on stdout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerOutput {
    Ok(Box<ScanResult>),
    Err(ScanError),
}

/// Parent-side limits for one child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkerLimits {
    pub timeout: Duration,
    pub memory_growth_mib: u64,
    pub max_bytes: u64,
}

/// Child entry point: parse the request, install limits, read stdin, scan,
/// print the outcome. Returns the process exit code.
pub fn run_worker(request_json: &str) -> i32 {
    let output = worker_output(request_json);
    let mut stdout = std::io::stdout().lock();
    let written = serde_json::to_writer(&mut stdout, &output)
        .map_err(|error| error.to_string())
        .and_then(|()| stdout.write_all(b"\n").map_err(|error| error.to_string()))
        .and_then(|()| stdout.flush().map_err(|error| error.to_string()));
    match written {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("worker could not write its result: {error}");
            3
        }
    }
}

fn worker_output(request_json: &str) -> WorkerOutput {
    let request: WorkerRequest = match serde_json::from_str(request_json) {
        Ok(request) => request,
        Err(error) => {
            return WorkerOutput::Err(ScanError::new(
                ErrorCode::Internal,
                format!("worker request is not valid JSON: {error}"),
            ));
        }
    };
    if let Err(error) = limits::install(request.memory_growth_mib, request.parent_pid) {
        return WorkerOutput::Err(ScanError::new(
            ErrorCode::Internal,
            format!("worker limits could not be installed: {error}"),
        ));
    }
    let input = match read_stdin(request.max_bytes) {
        Ok(input) => input,
        Err(error) => return WorkerOutput::Err(error),
    };
    match scan_input(&input, &request.options) {
        Ok(result) => WorkerOutput::Ok(Box::new(result)),
        Err(error) => WorkerOutput::Err(error),
    }
}

// Called only in the disposable worker after hard limits are installed.
fn scan_input(input: &[u8], options: &ScanOptions) -> Result<ScanResult, ScanError> {
    let started = Instant::now();
    let kind = image_pdf::sniff(input)
        .ok_or_else(|| ScanError::new(ErrorCode::Malformed, "unsupported input"))?;
    let wrapped = if kind == InputKind::Pdf {
        None
    } else {
        Some(image_pdf::wrap_image(input, kind).map_err(|error| {
            ScanError::new(
                if matches!(error, ImageError::TooLarge { .. }) {
                    ErrorCode::Limit
                } else {
                    ErrorCode::Malformed
                },
                error.to_string(),
            )
        })?)
    };
    let pdf = wrapped
        .as_ref()
        .map_or(input, |wrapped| wrapped.pdf.as_slice());
    let mut result = scan_bytes(pdf, options)?;
    result.input = kind;
    if let Some(wrapped) = wrapped {
        result.warnings.push(format!(
            "{} image of {}x{} px placed on one page at an assumed {} dpi",
            kind.as_str(),
            wrapped.width_px,
            wrapped.height_px,
            image_pdf::DEFAULT_DPI
        ));
    }
    result.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(result)
}

/// Drain stderr with fixed storage; retain only the last 4 KiB while reading.
fn stderr_tail(mut stream: impl Read) -> String {
    let mut ring = [0_u8; STDERR_TAIL_BYTES];
    let mut chunk = [0_u8; STDERR_TAIL_BYTES];
    let mut position = 0;
    let mut kept = 0;
    loop {
        let read = match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        for &byte in &chunk[..read] {
            ring[position] = byte;
            position = (position + 1) % STDERR_TAIL_BYTES;
        }
        kept = (kept + read).min(STDERR_TAIL_BYTES);
    }
    let mut ordered = Vec::with_capacity(kept);
    let start = if kept == STDERR_TAIL_BYTES {
        position
    } else {
        0
    };
    for index in 0..kept {
        ordered.push(ring[(start + index) % STDERR_TAIL_BYTES]);
    }
    String::from_utf8_lossy(&ordered).into_owned()
}

fn read_stdin(max_bytes: u64) -> Result<Vec<u8>, ScanError> {
    let mut input = Vec::new();
    let mut stdin = std::io::stdin().lock();
    let read = stdin
        .by_ref()
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut input)
        .map_err(|error| ScanError::new(ErrorCode::Internal, format!("reading input: {error}")))?;
    if u64::try_from(read).unwrap_or(u64::MAX) > max_bytes {
        return Err(ScanError::new(
            ErrorCode::Limit,
            format!("input exceeds {max_bytes} bytes"),
        ));
    }
    if input.is_empty() {
        return Err(ScanError::new(ErrorCode::Malformed, "empty input"));
    }
    Ok(input)
}

/// Kill-on-drop guard so an abandoned child never outlives the request.
struct Reaped(Child, bool);
impl Reaped {
    fn kill(&mut self) {
        if self.1 {
            return;
        }
        self.1 = true;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if let Ok(raw) = i32::try_from(self.0.id())
            && let Some(pid) = rustix::process::Pid::from_raw(raw)
        {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for Reaped {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Run one scan in a child process of `executable`, with `env` added to the
/// child's environment. `cancel` (the service's shutdown flag) kills the
/// child early.
pub fn run_in_worker(
    executable: &Path,
    pdf: Vec<u8>,
    options: &ScanOptions,
    limits: WorkerLimits,
    env: &[(String, String)],
    cancel: &AtomicBool,
) -> Result<ScanResult, ScanError> {
    run_in_worker_cancellable(executable, pdf, options, limits, env, || {
        cancel.load(Ordering::SeqCst)
    })
}

/// Supervise the request's shutdown, disconnect and explicit cancellation.
pub fn run_in_worker_cancellable(
    executable: &Path,
    input: Vec<u8>,
    options: &ScanOptions,
    limits: WorkerLimits,
    env: &[(String, String)],
    mut cancelled: impl FnMut() -> bool,
) -> Result<ScanResult, ScanError> {
    let deadline = Instant::now() + limits.timeout;
    if cancelled() || Instant::now() >= deadline {
        return Err(ScanError::new(
            ErrorCode::Timeout,
            "scan cancelled or deadline exceeded",
        ));
    }
    let request = WorkerRequest {
        options: options.clone(),
        max_bytes: limits.max_bytes,
        memory_growth_mib: limits.memory_growth_mib,
        parent_pid: std::process::id(),
    };
    let request_json = serde_json::to_string(&request)
        .map_err(|error| ScanError::new(ErrorCode::Internal, error.to_string()))?;
    let mut command = Command::new(executable);
    command
        .arg(SUBCOMMAND)
        .arg(request_json)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = Reaped(
        command.spawn().map_err(|error| {
            ScanError::new(
                ErrorCode::Internal,
                format!(
                    "could not start the scan worker {}: {error}",
                    executable.display()
                ),
            )
        })?,
        false,
    );
    let Some(mut stdin) = child.0.stdin.take() else {
        return Err(ScanError::new(ErrorCode::Internal, "worker stdin missing"));
    };
    let Some(mut stdout) = child.0.stdout.take() else {
        return Err(ScanError::new(ErrorCode::Internal, "worker stdout missing"));
    };
    let Some(stderr) = child.0.stderr.take() else {
        return Err(ScanError::new(ErrorCode::Internal, "worker stderr missing"));
    };
    // A child that exits early closes its pipe; a broken pipe is then
    // reported through its JSON/exit status, not here.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
        drop(stdin);
    });
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let outcome = stdout
            .by_ref()
            .take(u64::try_from(MAX_OUTPUT_BYTES).unwrap_or(u64::MAX))
            .read_to_end(&mut output)
            .map(|_| output);
        let _ = sender.send(outcome);
    });
    let errors = std::thread::spawn(move || stderr_tail(stderr));
    let outcome = loop {
        if cancelled() {
            break Err(ScanError::new(
                ErrorCode::Timeout,
                "scan cancelled; its worker was killed",
            ));
        }
        if Instant::now() >= deadline {
            break Err(ScanError::new(
                ErrorCode::Timeout,
                "scan deadline exceeded; its worker was killed",
            ));
        }
        match child.0.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(error) => {
                break Err(ScanError::new(
                    ErrorCode::Internal,
                    format!("waiting for worker: {error}"),
                ));
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // Kill the entire private process group, including any descendants holding
    // pipes open, and join all drains before returning/releasing admission.
    child.kill();
    let _ = writer.join();
    let _ = reader.join();
    let stderr_tail = errors.join().unwrap_or_default();
    let status = outcome?;
    let output = receiver
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| ScanError::new(ErrorCode::Internal, "worker output was not delivered"))?
        .map_err(|error| {
            ScanError::new(
                ErrorCode::Internal,
                format!("reading worker output: {error}"),
            )
        })?;
    match serde_json::from_slice::<WorkerOutput>(&output) {
        Ok(WorkerOutput::Ok(result)) => Ok(*result),
        Ok(WorkerOutput::Err(error)) => Err(error),
        Err(_) => Err(ScanError::new(
            ErrorCode::Internal,
            format!(
                "scan worker exited with {status} without a result{}",
                if stderr_tail.trim().is_empty() {
                    String::new()
                } else {
                    format!(": {}", stderr_tail.trim())
                }
            ),
        )),
    }
}

/// Hard process limits for the child, installed before any input is read.
pub mod limits {
    /// Install the address-space growth limit, disable core dumps and (on
    /// Linux) ask the kernel to kill the worker when the parent dies.
    /// Returns the effective address-space limit in bytes.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn install(growth_mib: u64, expected_parent: u32) -> Result<u64, String> {
        use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

        if growth_mib == 0 {
            return Err("worker address-space growth must be positive".to_string());
        }
        check_parent(expected_parent)?;
        let baseline = virtual_bytes()?;
        if baseline == 0 {
            return Err("worker start-up virtual size is unavailable".to_string());
        }
        let desired = growth_mib
            .checked_mul(1024 * 1024)
            .and_then(|growth| baseline.checked_add(growth))
            .ok_or("worker address-space limit overflow")?;
        let inherited = getrlimit(Resource::As);
        let effective = inherited
            .current
            .into_iter()
            .chain(inherited.maximum)
            .fold(desired, u64::min);
        if effective <= baseline {
            return Err("inherited address-space limit leaves no worker headroom".to_string());
        }
        let zero = Rlimit {
            current: Some(0),
            maximum: Some(0),
        };
        setrlimit(Resource::Core, zero)
            .map_err(|error| format!("disabling core dumps: {error}"))?;
        parent_protection(expected_parent)?;
        let limit = Rlimit {
            current: Some(effective),
            maximum: Some(effective),
        };
        setrlimit(Resource::As, limit)
            .map_err(|error| format!("installing the address-space limit: {error}"))?;
        if getrlimit(Resource::As) != limit {
            return Err("the address-space limit was not installed".to_string());
        }
        check_parent(expected_parent)?;
        Ok(effective)
    }

    /// Other platforms have no supported hard limit; the worker refuses to run.
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub fn install(_growth_mib: u64, _expected_parent: u32) -> Result<u64, String> {
        Err("hard worker limits require Linux or macOS".to_string())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn check_parent(expected_parent: u32) -> Result<(), String> {
        let expected = i32::try_from(expected_parent).map_err(|_| "invalid parent PID")?;
        if expected <= 0 {
            return Err("invalid parent PID".to_string());
        }
        if rustix::process::getppid().is_some_and(|pid| pid.as_raw_pid() == expected) {
            Ok(())
        } else {
            Err("the scan service exited or changed before the worker started".to_string())
        }
    }

    #[cfg(target_os = "linux")]
    fn parent_protection(expected_parent: u32) -> Result<(), String> {
        use rustix::process::{DumpableBehavior, Signal};

        rustix::process::set_dumpable_behavior(DumpableBehavior::NotDumpable)
            .map_err(|error| format!("disabling dumpability: {error}"))?;
        rustix::process::set_parent_process_death_signal(Some(Signal::KILL))
            .map_err(|error| format!("installing the parent-death signal: {error}"))?;
        check_parent(expected_parent)
    }

    #[cfg(target_os = "macos")]
    fn parent_protection(expected_parent: u32) -> Result<(), String> {
        // No Darwin counterpart to PR_SET_PDEATHSIG; the parent's kill-on-drop
        // guard and the deadline poll own the child's lifetime instead.
        check_parent(expected_parent)
    }

    #[cfg(target_os = "linux")]
    fn virtual_bytes() -> Result<u64, String> {
        use std::io::Read;

        let mut buffer = [0_u8; 256];
        let count = std::fs::File::open("/proc/self/statm")
            .and_then(|mut file| file.read(&mut buffer))
            .map_err(|error| format!("reading /proc/self/statm: {error}"))?;
        let text = std::str::from_utf8(&buffer[..count]).map_err(|error| error.to_string())?;
        let pages: u64 = text
            .split_whitespace()
            .next()
            .ok_or("missing virtual-memory page count")?
            .parse()
            .map_err(|_| "invalid virtual-memory page count")?;
        let page_size = u64::try_from(rustix::param::page_size()).map_err(|_| "page size")?;
        pages
            .checked_mul(page_size)
            .ok_or_else(|| "virtual-memory accounting overflow".to_string())
    }

    #[cfg(target_os = "macos")]
    fn virtual_bytes() -> Result<u64, String> {
        tpe_ffi::process::virtual_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stderr_storage_stays_fixed_while_a_large_stream_is_read() {
        struct Flood {
            remaining: usize,
            marker: bool,
        }
        impl Read for Flood {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                // Catch a growing read_to_end buffer, while streaming 32 MiB
                // without allocating the synthetic flood in the test itself.
                assert!(buffer.len() <= STDERR_TAIL_BYTES);
                if self.remaining > 0 {
                    let count = self.remaining.min(buffer.len());
                    buffer[..count].fill(b'x');
                    self.remaining -= count;
                    return Ok(count);
                }
                if !self.marker {
                    self.marker = true;
                    buffer[..9].copy_from_slice(b"FLOOD-END");
                    return Ok(9);
                }
                Ok(0)
            }
        }
        let tail = stderr_tail(Flood {
            remaining: 32 * 1024 * 1024,
            marker: false,
        });
        assert_eq!(tail.len(), STDERR_TAIL_BYTES);
        assert!(tail.ends_with("FLOOD-END"));
        assert!(tail[..tail.len() - 9].bytes().all(|byte| byte == b'x'));
    }

    #[test]
    fn worker_output_json_shapes_are_stable() {
        let error = WorkerOutput::Err(ScanError::new(ErrorCode::Limit, "too big"));
        let json = serde_json::to_string(&error).unwrap();
        assert_eq!(json, r#"{"err":{"code":"limit","message":"too big"}}"#);
        assert_eq!(serde_json::from_str::<WorkerOutput>(&json).unwrap(), error);
    }

    #[test]
    fn a_bad_request_is_reported_not_panicked() {
        let output = worker_output("{not json");
        let WorkerOutput::Err(error) = output else {
            panic!("expected an error");
        };
        assert_eq!(error.code, ErrorCode::Internal);
        assert!(error.message.contains("not valid JSON"));
    }
}
