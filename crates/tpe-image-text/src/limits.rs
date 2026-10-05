//! Resource limits for an external OCR process, without `unsafe` and without
//! a shell. The controller re-executes its own binary as
//! `tpe-image-text exec-limited <cpu-seconds> <address-space-bytes> <program> <args>...`.
//! That helper lowers its own `RLIMIT_CORE`, `RLIMIT_CPU`, `RLIMIT_FSIZE` and
//! `RLIMIT_AS` with `rustix` (plain system calls on the calling process), one
//! at a time, and then `exec`s the program, which inherits them. A limit the
//! kernel refuses (macOS answers `EINVAL` for `RLIMIT_AS`, for instance) is
//! skipped, not fatal: the helper prints one marker line on stderr naming the
//! limits it applied and the ones it skipped with the reason, then runs the
//! program anyway, because the controller's wall-clock kill is the guarantee
//! and the kernel limits are defence in depth. Nothing is interpolated into a
//! command line: every argument is passed as its own `OsString`.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;

/// First argument that selects the helper mode of the binary.
pub const EXEC_LIMITED: &str = "exec-limited";
/// Output files larger than this are refused to the child (bytes).
pub const MAX_OUTPUT_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// Exit status of the helper when its arguments are unusable.
pub const EXIT_LIMITS_FAILED: u8 = 125;
/// Exit status of the helper when `exec` itself failed.
pub const EXIT_EXEC_FAILED: u8 = 126;
/// Start of the stderr line in which the helper reports what it applied.
pub const MARKER_PREFIX: &str = "exec-limited: applied ";
const MARKER_SKIPPED: &str = "; skipped ";
const MARKER_NONE: &str = "none";
const MARKER_SKIPPED_SEPARATOR: &str = " | ";

/// Names of the limits the helper tries, in the order it tries them.
pub const LIMIT_NAMES: [&str; 4] = ["core", "cpu", "fsize", "as"];

/// What the helper managed to install before `exec`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LimitsOutcome {
    /// Limits that are in force in the engine process (`core`, `cpu`, `fsize`, `as`).
    pub applied: Vec<String>,
    /// `<name>: <reason>` for each limit the kernel refused or did not keep.
    pub skipped: Vec<String>,
}

impl LimitsOutcome {
    /// The marker line the helper prints on stderr before `exec`.
    #[must_use]
    pub fn marker_line(&self) -> String {
        let applied = if self.applied.is_empty() {
            MARKER_NONE.to_string()
        } else {
            self.applied.join(",")
        };
        let skipped = if self.skipped.is_empty() {
            MARKER_NONE.to_string()
        } else {
            self.skipped.join(MARKER_SKIPPED_SEPARATOR)
        };
        format!("{MARKER_PREFIX}{applied}{MARKER_SKIPPED}{skipped}")
    }

    /// Parse one line; `None` when it is not a marker line.
    #[must_use]
    pub fn parse_marker_line(line: &str) -> Option<Self> {
        let rest = line.trim().strip_prefix(MARKER_PREFIX)?;
        let (applied, skipped) = rest.split_once(MARKER_SKIPPED)?;
        let split = |s: &str, sep: &str| -> Vec<String> {
            if s == MARKER_NONE {
                Vec::new()
            } else {
                s.split(sep)
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(str::to_string)
                    .collect()
            }
        };
        Some(Self {
            applied: split(applied, ","),
            skipped: split(skipped, MARKER_SKIPPED_SEPARATOR),
        })
    }
}

/// Take the helper's marker line out of an engine's stderr. Returns the
/// outcome (if the helper reported one) and stderr without that line.
#[must_use]
pub fn split_marker(stderr: &str) -> (Option<LimitsOutcome>, String) {
    let mut outcome = None;
    let mut rest = String::with_capacity(stderr.len());
    for line in stderr.lines() {
        if let (true, Some(found)) = (outcome.is_none(), LimitsOutcome::parse_marker_line(line)) {
            outcome = Some(found);
        } else {
            rest.push_str(line);
            rest.push('\n');
        }
    }
    (outcome, rest)
}

/// Build the command for `program`: through `helper` with limits when one is
/// given, otherwise directly. The flag says whether the helper is used.
pub fn limited_command(
    helper: Option<&Path>,
    program: &Path,
    cpu_seconds: u64,
    address_space_bytes: u64,
) -> (Command, bool) {
    match helper {
        Some(helper) if cfg!(any(target_os = "linux", target_os = "macos")) => {
            let mut cmd = Command::new(helper);
            cmd.arg(EXEC_LIMITED)
                .arg(cpu_seconds.to_string())
                .arg(address_space_bytes.to_string())
                .arg(program);
            (cmd, true)
        }
        _ => (Command::new(program), false),
    }
}

/// Entry point of the helper mode: parse `<cpu> <bytes> <program> <args>...`,
/// install what the kernel accepts, report it on stderr and replace this
/// process. Returns only on failure, with the exit code the caller should use.
pub fn exec_limited_main(rest: &[OsString]) -> u8 {
    let Some((cpu, bytes, program, args)) = parse_helper_args(rest) else {
        eprintln!(
            "usage: tpe-image-text {EXEC_LIMITED} <cpu-seconds> <address-space-bytes> <program> [args...]"
        );
        return EXIT_LIMITS_FAILED;
    };
    let outcome = install_limits(cpu, bytes);
    eprintln!("{}", outcome.marker_line());
    let err = exec(program, args);
    eprintln!(
        "{EXEC_LIMITED}: exec {}: {err}",
        Path::new(program).display()
    );
    EXIT_EXEC_FAILED
}

fn parse_helper_args(rest: &[OsString]) -> Option<(u64, u64, &OsStr, &[OsString])> {
    let cpu = rest.first()?.to_str()?.parse::<u64>().ok()?;
    let bytes = rest.get(1)?.to_str()?.parse::<u64>().ok()?;
    let program = rest.get(2)?.as_os_str();
    if cpu == 0 || bytes == 0 {
        return None;
    }
    Some((cpu, bytes, program, &rest[3..]))
}

/// Lower each limit on this process, independently, never above what was
/// inherited. A refused or unkept limit is recorded, never fatal: on macOS the
/// kernel answers `EINVAL` for `RLIMIT_AS`, and the engine must still run
/// under the controller's wall-clock kill.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn install_limits(cpu_seconds: u64, address_space_bytes: u64) -> LimitsOutcome {
    use rustix::process::Resource;

    let wanted = [
        (LIMIT_NAMES[0], Resource::Core, 0, 0),
        (
            LIMIT_NAMES[1],
            Resource::Cpu,
            cpu_seconds,
            cpu_seconds.saturating_add(5),
        ),
        (
            LIMIT_NAMES[2],
            Resource::Fsize,
            MAX_OUTPUT_FILE_BYTES,
            MAX_OUTPUT_FILE_BYTES,
        ),
        (
            LIMIT_NAMES[3],
            Resource::As,
            address_space_bytes,
            address_space_bytes,
        ),
    ];
    let mut outcome = LimitsOutcome::default();
    for (name, resource, current, maximum) in wanted {
        match lower(resource, current, maximum) {
            Ok(()) => outcome.applied.push(name.to_string()),
            Err(reason) => outcome.skipped.push(format!("{name}: {reason}")),
        }
    }
    outcome
}

/// Set one limit to `min(wanted, inherited)` and confirm the kernel kept it.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn lower(resource: rustix::process::Resource, current: u64, maximum: u64) -> Result<(), String> {
    use rustix::process::{Rlimit, getrlimit, setrlimit};

    let cap = |value: u64, limit: Option<u64>| limit.map_or(value, |l| value.min(l));
    let inherited = getrlimit(resource);
    let maximum = cap(maximum, inherited.maximum);
    let current = cap(cap(current, inherited.current), Some(maximum));
    let wanted = Rlimit {
        current: Some(current),
        maximum: Some(maximum),
    };
    match setrlimit(resource, wanted) {
        Ok(()) => {}
        Err(e @ (rustix::io::Errno::INVAL | rustix::io::Errno::PERM)) => {
            return Err(format!(
                "not supported on this platform ({})",
                std::io::Error::from(e)
            ));
        }
        Err(e) => return Err(std::io::Error::from(e).to_string()),
    }
    let kept = getrlimit(resource);
    if kept == wanted {
        Ok(())
    } else {
        Err(format!(
            "kernel kept {}/{} instead of {current}/{maximum}",
            kept.current
                .map_or("unlimited".to_string(), |v| v.to_string()),
            kept.maximum
                .map_or("unlimited".to_string(), |v| v.to_string()),
        ))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn install_limits(_cpu_seconds: u64, _address_space_bytes: u64) -> LimitsOutcome {
    LimitsOutcome {
        applied: Vec::new(),
        skipped: LIMIT_NAMES
            .iter()
            .map(|n| format!("{n}: resource limits need Linux or macOS"))
            .collect(),
    }
}

#[cfg(unix)]
fn exec(program: &OsStr, args: &[OsString]) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    Command::new(program).args(args).exec()
}

#[cfg(not(unix))]
fn exec(program: &OsStr, args: &[OsString]) -> std::io::Error {
    let _ = (program, args);
    std::io::Error::other("exec is only available on Unix")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_args_are_validated() {
        let ok: Vec<OsString> = ["5", "1024", "prog", "a", "b"]
            .iter()
            .map(OsString::from)
            .collect();
        let (cpu, bytes, program, args) = parse_helper_args(&ok).expect("valid");
        assert_eq!((cpu, bytes), (5, 1024));
        assert_eq!(program, OsStr::new("prog"));
        assert_eq!(args.len(), 2);
        let zero: Vec<OsString> = ["0", "1024", "prog"].iter().map(OsString::from).collect();
        assert!(parse_helper_args(&zero).is_none());
        let short: Vec<OsString> = ["5", "1024"].iter().map(OsString::from).collect();
        assert!(parse_helper_args(&short).is_none());
        let junk: Vec<OsString> = ["five", "1024", "prog"]
            .iter()
            .map(OsString::from)
            .collect();
        assert!(parse_helper_args(&junk).is_none());
    }

    #[test]
    fn direct_command_without_helper_reports_no_limits() {
        let (cmd, limited) = limited_command(None, Path::new("/bin/true"), 1, 1 << 30);
        assert!(!limited);
        assert_eq!(cmd.get_program(), OsStr::new("/bin/true"));
        let (cmd, limited) = limited_command(
            Some(Path::new("/x/helper")),
            Path::new("/bin/true"),
            7,
            4096,
        );
        assert_eq!(limited, cfg!(any(target_os = "linux", target_os = "macos")));
        if limited {
            let args: Vec<&OsStr> = cmd.get_args().collect();
            assert_eq!(
                args,
                ["exec-limited", "7", "4096", "/bin/true"].map(OsStr::new)
            );
        }
    }

    #[test]
    fn marker_line_round_trips_every_shape() {
        let full = LimitsOutcome {
            applied: LIMIT_NAMES.iter().map(ToString::to_string).collect(),
            skipped: Vec::new(),
        };
        assert_eq!(
            full.marker_line(),
            "exec-limited: applied core,cpu,fsize,as; skipped none"
        );
        assert_eq!(
            LimitsOutcome::parse_marker_line(&full.marker_line()),
            Some(full)
        );
        let partial = LimitsOutcome {
            applied: vec!["core".into(), "cpu".into(), "fsize".into()],
            skipped: vec![
                "as: not supported on this platform (Invalid argument (os error 22))".into(),
            ],
        };
        assert_eq!(
            LimitsOutcome::parse_marker_line(&partial.marker_line()),
            Some(partial.clone())
        );
        let nothing = LimitsOutcome {
            applied: Vec::new(),
            skipped: vec!["core: x".into(), "cpu: y".into()],
        };
        assert_eq!(
            nothing.marker_line(),
            "exec-limited: applied none; skipped core: x | cpu: y"
        );
        assert_eq!(
            LimitsOutcome::parse_marker_line(&nothing.marker_line()),
            Some(nothing)
        );
        assert_eq!(LimitsOutcome::parse_marker_line("tesseract: warning"), None);
        assert_eq!(
            LimitsOutcome::parse_marker_line("exec-limited: exec x"),
            None
        );

        let stderr = format!(
            "{}\nWarning: Invalid resolution 0 dpi.\n",
            partial.marker_line()
        );
        let (outcome, rest) = split_marker(&stderr);
        assert_eq!(outcome, Some(partial));
        assert_eq!(rest, "Warning: Invalid resolution 0 dpi.\n");
        let (none, rest) = split_marker("plain\n");
        assert_eq!(none, None);
        assert_eq!(rest, "plain\n");
    }
}
