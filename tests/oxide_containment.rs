//! A harness=false executable: only disposable children install restrictions.
//! Probes are finite single operations, never a fork bomb or outbound request.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn denied<T>(result: io::Result<T>, operation: &str) {
    let error = match result {
        Ok(_) => panic!("{operation} unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.raw_os_error(), Some(1), "{operation}: {error}");
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn network_and_process_probes() {
    use std::os::unix::process::CommandExt;
    denied(
        std::net::TcpListener::bind("127.0.0.1:0"),
        "TCP socket creation",
    );
    denied(
        std::net::UdpSocket::bind("127.0.0.1:0"),
        "UDP socket creation",
    );
    denied(
        std::os::unix::net::UnixStream::pair(),
        "UNIX socketpair creation",
    );
    let absent = "/tpe-containment-deliberately-nonexistent-executable";
    denied(Command::new(absent).spawn(), "process creation");
    // Nonexistent path cannot replace even a faulty test worker; EPERM instead
    // of ENOENT demonstrates exec denial without executing an external program.
    let result = Command::new(absent).exec();
    assert_eq!(result.raw_os_error(), Some(1), "exec: {result}");
    assert_eq!(std::thread::spawn(|| 21 * 2).join().unwrap(), 42);
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn filesystem_probes(root: &Path) {
    denied(std::fs::read(root.join("sentinel")), "read existing file");
    denied(
        std::fs::write(root.join("new-output"), b"forbidden"),
        "create/write file",
    );
    denied(std::fs::metadata(root.join("sentinel")), "path metadata");
    denied(std::fs::read_dir(root), "directory listing");
    denied(std::fs::read_link(root.join("symlink")), "read symlink");
    denied(
        std::fs::remove_file(root.join("sentinel")),
        "remove existing file",
    );
    denied(
        std::fs::rename(root.join("sentinel"), root.join("renamed")),
        "rename file",
    );
    denied(
        std::fs::set_permissions(
            root.join("sentinel"),
            std::os::unix::fs::PermissionsExt::from_mode(0o777),
        ),
        "chmod file",
    );
    denied(
        std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(root.join("sentinel")),
        "truncate file",
    );
    denied(
        std::os::unix::fs::symlink(root.join("sentinel"), root.join("new-symlink")),
        "create symlink",
    );
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn probe(name: &str, root: &Path) {
    use tpe_ffi::oxide_containment as guard;
    assert!(!guard::installed());
    match name {
        "process" | "sealed" => {
            // This thread deliberately predates seccomp. Its probes confirm
            // TSYNC; future thread creation/inheritance is checked separately.
            let (send, receive) = std::sync::mpsc::channel();
            let existing_root = root.to_path_buf();
            let sealed = name == "sealed";
            let existing = std::thread::spawn(move || {
                receive.recv_timeout(Duration::from_secs(5)).unwrap();
                denied(
                    std::net::TcpListener::bind("127.0.0.1:0"),
                    "existing thread socket",
                );
                if sealed {
                    filesystem_probes(&existing_root);
                }
            });
            if name == "sealed" {
                let evidence = guard::install_sealed_process().expect("sealed install");
                assert_eq!(evidence.mode, "seccomp_sealed");
                assert_eq!(evidence.landlock_abi, None);
                assert!(evidence.filesystem_enforced && guard::installed());
                filesystem_probes(root);
                assert!(guard::install(&[root.to_path_buf()]).is_err());
            } else {
                let evidence = guard::install_process_policy().expect("process-only install");
                assert_eq!(evidence.mode, "seccomp_process");
                assert!(!evidence.filesystem_enforced && !guard::installed());
                assert_eq!(std::fs::read(root.join("sentinel")).unwrap(), b"unchanged");
            }
            send.send(()).unwrap();
            existing.join().unwrap();
            network_and_process_probes();
            if sealed {
                let future_root = root.to_path_buf();
                std::thread::spawn(move || {
                    assert!(guard::installed());
                    filesystem_probes(&future_root);
                })
                .join()
                .unwrap();
            }
            let sum = (0..10_000_u64).map(|n| n * n).sum::<u64>();
            assert_eq!(sum, 333_283_335_000);
            println!("{name}: filesystem/network/process probes passed; threads and memory work");
        }
        "inherited_fd" => {
            let file = std::fs::File::open(root.join("sentinel")).unwrap();
            let error = guard::install_sealed_process().unwrap_err();
            assert!(error.contains("unexpected inherited descriptor"), "{error}");
            assert!(!guard::installed());
            drop(file);
            assert_eq!(std::fs::read(root.join("sentinel")).unwrap(), b"unchanged");
            println!("unexpected inherited descriptor rejected before restriction");
        }
        "landlock_threads" => {
            let (send, receive) = std::sync::mpsc::channel::<()>();
            let other = std::thread::spawn(move || {
                let _ = receive.recv_timeout(Duration::from_secs(5));
            });
            let error = guard::install(&[]).unwrap_err();
            assert!(error.contains("before the lease/parser threads"), "{error}");
            assert!(!guard::installed());
            send.send(()).unwrap();
            other.join().unwrap();
            println!("multithreaded Landlock installation rejected");
        }
        "landlock" => match guard::install(&[root.join("sentinel")]) {
            Ok(evidence) => {
                assert_eq!(evidence.mode, "landlock_read_only");
                assert!(evidence.landlock_abi.unwrap() >= 6);
                assert!(evidence.filesystem_enforced && guard::installed());
                assert_eq!(std::fs::read(root.join("sentinel")).unwrap(), b"unchanged");
                assert!(std::fs::read(root.join("outside")).is_err());
                assert!(std::fs::write(root.join("new-output"), b"forbidden").is_err());
                network_and_process_probes();
                println!(
                    "Landlock allowlist enforced at ABI {:?}",
                    evidence.landlock_abi
                );
            }
            Err(error) => {
                assert!(!guard::installed());
                assert!(
                    error.contains("Landlock ABI") || error.contains("Landlock create"),
                    "{error}"
                );
                println!(
                    "LANDLOCK_UNAVAILABLE: {error}; positive filesystem allowlist probe not executed"
                );
            }
        },
        _ => panic!("unknown bounded containment probe"),
    }
}

fn main() {
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        let arguments: Vec<_> = std::env::args().collect();
        if arguments.get(1).map(String::as_str) == Some("--probe") {
            assert_eq!(arguments.len(), 4);
            probe(&arguments[2], Path::new(&arguments[3]));
            return;
        }
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("sentinel"), b"unchanged").unwrap();
        std::fs::write(root.path().join("outside"), b"outside").unwrap();
        std::os::unix::fs::symlink(root.path().join("sentinel"), root.path().join("symlink"))
            .unwrap();
        let executable = std::env::current_exe().unwrap();
        for case in [
            "process",
            "sealed",
            "inherited_fd",
            "landlock_threads",
            "landlock",
        ] {
            let mut child = Command::new(&executable)
                .arg("--probe")
                .arg(case)
                .arg(root.path())
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if child.try_wait().unwrap().is_some() {
                    break;
                }
                if Instant::now() >= deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("containment probe {case} exceeded 10 seconds; killed/reaped");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{case}: status {:?}\n{}\n{}",
                output.status.code(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            print!("{}", String::from_utf8_lossy(&output.stdout));
            assert_eq!(
                std::fs::read(root.path().join("sentinel")).unwrap(),
                b"unchanged"
            );
            assert!(!root.path().join("new-output").exists());
            assert!(!root.path().join("new-symlink").exists());
            assert!(!root.path().join("renamed").exists());
        }
        println!(
            "test result: ok. 5 bounded subprocess scenarios; see explicit Landlock availability above"
        );
    }
    #[cfg(not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        assert!(!tpe_ffi::oxide_containment::installed());
        assert!(tpe_ffi::oxide_containment::install_process_policy().is_err());
        assert!(tpe_ffi::oxide_containment::install_sealed_process().is_err());
        assert!(tpe_ffi::oxide_containment::install(&[]).is_err());
        println!("containment unavailable on this platform; fail-closed APIs verified");
    }
}
