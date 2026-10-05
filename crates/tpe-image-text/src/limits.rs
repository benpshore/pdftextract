//! Resource limits for an external OCR process, without `unsafe` and without
//! a shell. The controller re-executes its own binary as
//! `tpe-image-text exec-limited <cpu-seconds> <address-space-bytes> <program> <args>...`.
//! That helper lowers its own `RLIMIT_CORE`, `RLIMIT_CPU`, `RLIMIT_FSIZE` and
//! `RLIMIT_AS` with `rustix` (plain system calls on the calling process) and
//! then `exec`s the program, which inherits them. Nothing is interpolated into
//! a command line: every argument is passed as its own `OsString`.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;

/// First argument that selects the helper mode of the binary.
pub const EXEC_LIMITED: &str = "exec-limited";
/// Output files larger than this are refused to the child (bytes).
pub const MAX_OUTPUT_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// Exit status of the helper when the limits could not be installed.
pub const EXIT_LIMITS_FAILED: u8 = 125;
/// Exit status of the helper when `exec` itself failed.
pub const EXIT_EXEC_FAILED: u8 = 126;

/// Build the command for `program`: through `helper` with limits when one is
/// given, otherwise directly. The flag says whether limits will apply.
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
/// install the limits and replace this process. Returns only on failure, with
/// the exit code the caller should use.
pub fn exec_limited_main(rest: &[OsString]) -> u8 {
    let Some((cpu, bytes, program, args)) = parse_helper_args(rest) else {
        eprintln!(
            "usage: tpe-image-text {EXEC_LIMITED} <cpu-seconds> <address-space-bytes> <program> [args...]"
        );
        return EXIT_LIMITS_FAILED;
    };
    match install_limits(cpu, bytes) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("{EXEC_LIMITED}: could not install resource limits: {e}");
            return EXIT_LIMITS_FAILED;
        }
    }
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn install_limits(cpu_seconds: u64, address_space_bytes: u64) -> std::io::Result<()> {
    use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

    let zero = Rlimit {
        current: Some(0),
        maximum: Some(0),
    };
    setrlimit(Resource::Core, zero)?;
    let cpu = Rlimit {
        current: Some(cpu_seconds),
        maximum: Some(cpu_seconds.saturating_add(5)),
    };
    setrlimit(Resource::Cpu, cpu)?;
    let fsize = Rlimit {
        current: Some(MAX_OUTPUT_FILE_BYTES),
        maximum: Some(MAX_OUTPUT_FILE_BYTES),
    };
    setrlimit(Resource::Fsize, fsize)?;
    // Never widen an inherited address-space limit.
    let inherited = getrlimit(Resource::As);
    let effective = inherited
        .current
        .into_iter()
        .chain(inherited.maximum)
        .fold(address_space_bytes, u64::min);
    let address = Rlimit {
        current: Some(effective),
        maximum: Some(effective),
    };
    setrlimit(Resource::As, address)?;
    if getrlimit(Resource::As) != address {
        return Err(std::io::Error::other(
            "address-space limit was not installed",
        ));
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn install_limits(_cpu_seconds: u64, _address_space_bytes: u64) -> std::io::Result<()> {
    Err(std::io::Error::other("resource limits need Linux or macOS"))
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
}
