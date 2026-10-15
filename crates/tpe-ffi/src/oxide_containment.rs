//! Irreversible restrictions for disposable, already-loaded operation workers.
//!
//! This is Linux x86_64/aarch64 policy, not a portable sandbox claim. The caller
//! must load immutable inputs, close acquisition/request descriptors, install
//! resource limits, and retain only the stdin lease and stdout/stderr captures.
//! Never call these functions in the controller or an interactive process.
//!
//! Three policies are deliberately distinct: process/network seccomp only;
//! Landlock read allowlisting plus seccomp; and a seccomp syscall allowlist with
//! no path IO. Failure never enables `installed()`. Failure after an irreversible
//! step requires terminating the disposable worker, not continuing with parsing.
//!
//! ABI references: Linux v6.18 `include/uapi/linux/{landlock,seccomp,filter}.h`;
//! <https://docs.kernel.org/userspace-api/seccomp_filter.html>. Landlock ABI6 is
//! required; newer optional ABI semantics are not inferred. Seccomp uses TSYNC.

use std::path::PathBuf;

/// What this process actually installed, not a claim about PDF correctness.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Evidence {
    /// `seccomp_process`, `landlock_read_only`, or `seccomp_sealed`.
    pub mode: &'static str,
    /// Actual kernel ABI query, only for the Landlock policy.
    pub landlock_abi: Option<u32>,
    /// Reads are allowlisted, or all path IO is denied in sealed mode.
    pub filesystem_enforced: bool,
    /// Socket creation/use is denied; inherited sockets are rejected first.
    pub network_enforced: bool,
    /// Exec and process creation are denied; constrained threads remain allowed.
    pub process_enforced: bool,
    /// Seccomp synchronized successfully across all existing threads.
    pub seccomp_tsync: bool,
    /// Number of explicitly granted read-only paths; zero in sealed mode.
    pub read_only_paths: usize,
}

/// True only after a filesystem policy is enforced for the calling thread.
pub fn installed() -> bool {
    platform::installed()
}

/// Install network/process restrictions without claiming filesystem isolation.
///
/// This may be called with an existing lease thread because seccomp uses TSYNC.
/// Inputs and request files must already be closed. No caller-owned non-stdio
/// descriptors may remain, including sockets or directory capabilities.
pub fn install_process_policy() -> Result<Evidence, String> {
    platform::install_process_policy()
}

/// Deny all path IO using a syscall allowlist, with network/process restrictions.
///
/// This intentionally does not substitute for a nonempty read allowlist. It
/// permits memory-only operation and inherited stdio, never opens font/model
/// paths, and must be reported as `seccomp_sealed`, not Landlock.
pub fn install_sealed_process() -> Result<Evidence, String> {
    platform::install_sealed_process()
}

/// Install Landlock read-only allowlisting plus network/process restrictions.
///
/// Must run before any other thread. Kernel ABI6, all requested rights and the
/// complete seccomp filter must succeed; there is no best-effort fallback.
pub fn install(read_only_paths: &[PathBuf]) -> Result<Evidence, String> {
    platform::install(read_only_paths)
}

#[cfg(not(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64"),
    target_endian = "little"
)))]
mod platform {
    use super::{Evidence, PathBuf};

    pub(super) const fn installed() -> bool {
        false
    }
    pub(super) fn install_process_policy() -> Result<Evidence, String> {
        Err("oxide containment requires Linux little-endian x86_64/aarch64".into())
    }
    pub(super) fn install_sealed_process() -> Result<Evidence, String> {
        install_process_policy()
    }
    pub(super) fn install(_: &[PathBuf]) -> Result<Evidence, String> {
        install_process_policy()
    }
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64"),
    target_endian = "little"
))]
mod platform {
    use super::{Evidence, PathBuf};
    use std::cell::Cell;
    use std::fs::OpenOptions;
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
    use std::path::Component;
    use std::sync::atomic::{AtomicBool, Ordering};

    static PROCESS: AtomicBool = AtomicBool::new(false);
    static SEALED: AtomicBool = AtomicBool::new(false);
    thread_local! { static LANDLOCK: Cell<bool> = const { Cell::new(false) }; }

    const BPF_LOAD: u16 = 0x20; // BPF_LD | BPF_W | BPF_ABS
    const BPF_EQ: u16 = 0x15; // BPF_JMP | BPF_JEQ | BPF_K
    const BPF_AND: u16 = 0x54; // BPF_ALU | BPF_AND | BPF_K
    const BPF_JSET: u16 = 0x45; // BPF_JMP | BPF_JSET | BPF_K
    const BPF_RETURN: u16 = 0x06;
    const ALLOW: u32 = 0x7fff_0000;
    const KILL: u32 = 0x8000_0000;
    const DENY: u32 = 0x0005_0000 | libc::EPERM as u32;
    const UNAVAILABLE: u32 = 0x0005_0000 | libc::ENOSYS as u32;
    const ARCH_OFFSET: u32 = 4;
    const ARG0: u32 = 16;
    #[cfg(target_arch = "x86_64")]
    const AUDIT_ARCH: u32 = 0xc000_003e;
    #[cfg(target_arch = "aarch64")]
    const AUDIT_ARCH: u32 = 0xc000_00b7;

    // pthread-style clone only; no signal byte, process, namespace, untraced or
    // future unknown flags. clone3 cannot inspect pointed-to flags with cBPF;
    // ENOSYS requests libc's legacy clone fallback.
    const THREAD_REQUIRED: u32 = 0x100 | 0x800 | 0x1_0000; // VM, SIGHAND, THREAD
    const THREAD_ALLOWED: u32 = THREAD_REQUIRED
        | 0x200 // FS
        | 0x400 // FILES
        | 0x4_0000 // SYSVSEM
        | 0x8_0000 // SETTLS
        | 0x10_0000 // PARENT_SETTID
        | 0x20_0000 // CHILD_CLEARTID
        | 0x100_0000; // CHILD_SETTID

    #[repr(C)]
    struct Ruleset {
        handled_access_fs: u64,
        handled_access_net: u64,
        scoped: u64,
    }

    #[repr(C, packed)]
    struct PathRule {
        allowed_access: u64,
        parent_fd: i32,
    }

    pub(super) fn installed() -> bool {
        SEALED.load(Ordering::Acquire) || LANDLOCK.with(Cell::get)
    }

    fn evidence(mode: &'static str, fs: bool, abi: Option<u32>, paths: usize) -> Evidence {
        Evidence {
            mode,
            landlock_abi: abi,
            filesystem_enforced: fs,
            network_enforced: true,
            process_enforced: true,
            seccomp_tsync: true,
            read_only_paths: paths,
        }
    }

    fn error(action: &str) -> String {
        format!("oxide containment {action}: {}", io::Error::last_os_error())
    }

    fn check_descriptors() -> Result<(), String> {
        // Collect first, so read_dir's own descriptor is closed before checking.
        let entries = std::fs::read_dir("/proc/self/fd")
            .map_err(|e| format!("oxide containment cannot inspect inherited descriptors: {e}"))?;
        let mut fds = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("oxide containment descriptor entry: {e}"))?;
            let fd = entry
                .file_name()
                .to_string_lossy()
                .parse::<i32>()
                .map_err(|_| "oxide containment invalid descriptor entry".to_string())?;
            fds.push(fd);
            if fds.len() > 64 {
                return Err("oxide containment too many inherited descriptors".into());
            }
        }
        for fd in fds {
            let path = format!("/proc/self/fd/{fd}");
            let metadata = match std::fs::metadata(&path) {
                Ok(value) => value,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("oxide containment descriptor inspection: {e}")),
            };
            if fd > 2 {
                return Err(format!(
                    "oxide containment unexpected inherited descriptor {fd}"
                ));
            }
            let kind = metadata.file_type();
            let null_stdin = fd == 0
                && kind.is_char_device()
                && std::fs::read_link(path).ok().as_deref()
                    == Some(std::path::Path::new("/dev/null"));
            if !kind.is_fifo() && !(fd != 0 && kind.is_file()) && !null_stdin {
                return Err(format!(
                    "oxide containment stdio descriptor {fd} is not an approved pipe/capture"
                ));
            }
        }
        Ok(())
    }

    fn require_single_thread() -> Result<(), String> {
        let mut tasks = std::fs::read_dir("/proc/self/task")
            .map_err(|e| format!("oxide containment cannot inspect threads: {e}"))?;
        tasks
            .next()
            .ok_or("oxide containment cannot find current thread")?
            .map_err(|e| format!("oxide containment thread inspection: {e}"))?;
        if tasks.next().is_some() {
            return Err("Landlock must be installed before the lease/parser threads exist".into());
        }
        Ok(())
    }

    pub(super) fn install_process_policy() -> Result<Evidence, String> {
        if SEALED.load(Ordering::Acquire) {
            return Ok(evidence("seccomp_sealed", true, None, 0));
        }
        if !PROCESS.load(Ordering::Acquire) {
            check_descriptors()?;
            install_filter(false)?;
            PROCESS.store(true, Ordering::Release);
        }
        Ok(evidence("seccomp_process", false, None, 0))
    }

    pub(super) fn install_sealed_process() -> Result<Evidence, String> {
        if SEALED.load(Ordering::Acquire) {
            return Ok(evidence("seccomp_sealed", true, None, 0));
        }
        // Check again even if the process-only policy was installed earlier.
        check_descriptors()?;
        install_filter(true)?;
        PROCESS.store(true, Ordering::Release);
        SEALED.store(true, Ordering::Release);
        Ok(evidence("seccomp_sealed", true, None, 0))
    }

    pub(super) fn install(paths: &[PathBuf]) -> Result<Evidence, String> {
        if installed() {
            return Err(
                "filesystem containment is already installed; read allowlist cannot change".into(),
            );
        }
        require_single_thread()?;
        check_descriptors()?;
        if paths.len() > 16 {
            return Err("Landlock read allowlist exceeds 16 paths".into());
        }
        // SAFETY: version query supplies null/zero per the Linux Landlock UAPI.
        let abi = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<Ruleset>(),
                0usize,
                1u32,
            )
        };
        if abi < 0 {
            return Err(error("Landlock ABI query (no fallback)"));
        }
        if abi < 6 {
            return Err(format!("Landlock ABI {abi} is below required ABI6"));
        }
        let attr = Ruleset {
            handled_access_fs: 0xffff, // All ABI1..5 rights, including REFER/TRUNCATE/IOCTL_DEV.
            handled_access_net: 3,     // Bind/connect TCP, with no allowed ports.
            scoped: 3,                 // Abstract UNIX sockets and signals, ABI6.
        };
        // SAFETY: attr is the repr(C), initialized 24-byte ABI6 UAPI structure.
        let raw = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &raw const attr,
                std::mem::size_of::<Ruleset>(),
                0u32,
            )
        };
        if raw < 0 {
            return Err(error("Landlock create ruleset"));
        }
        let raw = i32::try_from(raw).map_err(|_| "Landlock returned invalid fd")?;
        // SAFETY: successful syscall returned a new, exclusively owned fd.
        let rules = unsafe { OwnedFd::from_raw_fd(raw) };
        for path in paths {
            if !path.is_absolute()
                || path.components().count() < 2
                || path
                    .components()
                    .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
                || path.as_os_str().len() > 4096
                || std::fs::canonicalize(path).as_ref().ok() != Some(path)
            {
                return Err("Landlock requires canonical, non-root absolute read paths".into());
            }
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_PATH | libc::O_CLOEXEC | libc::O_NOFOLLOW)
                .open(path)
                .map_err(|e| format!("Landlock open allowlist entry: {e}"))?;
            let kind = file
                .metadata()
                .map_err(|e| format!("Landlock entry metadata: {e}"))?
                .file_type();
            if !kind.is_dir()
                && !kind.is_file()
                && !(kind.is_char_device() && path == std::path::Path::new("/dev/urandom"))
            {
                return Err(
                    "Landlock read path must be a regular file/directory or /dev/urandom".into(),
                );
            }
            let rule = PathRule {
                allowed_access: if kind.is_dir() { 12 } else { 4 },
                parent_fd: file.as_raw_fd(),
            };
            // SAFETY: packed rule matches the initialized 12-byte UAPI structure;
            // both fds remain valid until the kernel copies the rule.
            let result = unsafe {
                libc::syscall(
                    libc::SYS_landlock_add_rule,
                    rules.as_raw_fd(),
                    1u32,
                    &raw const rule,
                    0u32,
                )
            };
            if result != 0 {
                return Err(error("Landlock add read rule"));
            }
        }
        no_new_privileges()?;
        // SAFETY: valid owned ruleset fd; flags0, current thread only as checked.
        if unsafe { libc::syscall(libc::SYS_landlock_restrict_self, rules.as_raw_fd(), 0u32) } != 0
        {
            return Err(error("Landlock restrict self"));
        }
        drop(rules);
        install_filter(false)?;
        PROCESS.store(true, Ordering::Release);
        LANDLOCK.with(|value| value.set(true));
        Ok(evidence(
            "landlock_read_only",
            true,
            Some(abi as u32),
            paths.len(),
        ))
    }

    fn no_new_privileges() -> Result<(), String> {
        // SAFETY: prctl scalar arguments documented by PR_SET_NO_NEW_PRIVS.
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
            return Err(error("PR_SET_NO_NEW_PRIVS"));
        }
        Ok(())
    }

    fn stmt(code: u16, k: u32) -> libc::sock_filter {
        libc::sock_filter {
            code,
            jt: 0,
            jf: 0,
            k,
        }
    }
    fn eq(k: u32, jt: u8, jf: u8) -> libc::sock_filter {
        libc::sock_filter {
            code: BPF_EQ,
            jt,
            jf,
            k,
        }
    }
    fn deny_syscall(filter: &mut Vec<libc::sock_filter>, nr: libc::c_long, verdict: u32) {
        filter.extend([eq(nr as u32, 0, 1), stmt(BPF_RETURN, verdict)]);
    }

    fn argument_choices(
        filter: &mut Vec<libc::sock_filter>,
        nr: libc::c_long,
        offset: u32,
        choices: &[u32],
    ) {
        // All matched paths return; nonmatching syscall preserves nr in A.
        let length = u8::try_from(2 * choices.len() + 5).expect("fixed small BPF block");
        filter.push(eq(nr as u32, 0, length));
        filter.extend([
            stmt(BPF_LOAD, offset + 4),
            eq(0, 1, 0),
            stmt(BPF_RETURN, DENY),
            stmt(BPF_LOAD, offset),
        ]);
        for value in choices {
            filter.extend([eq(*value, 0, 1), stmt(BPF_RETURN, ALLOW)]);
        }
        filter.push(stmt(BPF_RETURN, DENY));
    }

    fn policy(sealed: bool) -> Vec<libc::sock_filter> {
        let mut filter = vec![
            stmt(BPF_LOAD, ARCH_OFFSET),
            eq(AUDIT_ARCH, 1, 0),
            stmt(BPF_RETURN, KILL),
            stmt(BPF_LOAD, 0),
        ];
        #[cfg(target_arch = "x86_64")]
        filter.extend([
            libc::sock_filter {
                code: BPF_JSET,
                jt: 0,
                jf: 1,
                k: 0x4000_0000,
            },
            stmt(BPF_RETURN, KILL),
        ]);
        deny_syscall(&mut filter, libc::SYS_clone3, UNAVAILABLE);
        filter.extend([
            eq(libc::SYS_clone as u32, 0, 12),
            stmt(BPF_LOAD, ARG0 + 4),
            eq(0, 1, 0),
            stmt(BPF_RETURN, DENY),
            stmt(BPF_LOAD, ARG0),
            stmt(BPF_AND, !THREAD_ALLOWED),
            eq(0, 1, 0),
            stmt(BPF_RETURN, DENY),
            stmt(BPF_LOAD, ARG0),
            stmt(BPF_AND, THREAD_REQUIRED),
            eq(THREAD_REQUIRED, 0, 1),
            stmt(BPF_RETURN, ALLOW),
            stmt(BPF_RETURN, DENY),
        ]);
        let pid = std::process::id();
        for nr in [
            libc::SYS_kill,
            libc::SYS_tgkill,
            libc::SYS_rt_sigqueueinfo,
            libc::SYS_rt_tgsigqueueinfo,
        ] {
            argument_choices(&mut filter, nr, ARG0, &[pid]);
        }
        argument_choices(
            &mut filter,
            libc::SYS_prctl,
            ARG0,
            &[
                libc::PR_SET_NAME as u32,
                libc::PR_GET_NAME as u32,
                libc::PR_SET_NO_NEW_PRIVS as u32,
                libc::PR_GET_NO_NEW_PRIVS as u32,
                libc::PR_GET_SECCOMP as u32,
            ],
        );
        argument_choices(
            &mut filter,
            libc::SYS_fcntl,
            ARG0 + 8,
            &[
                libc::F_GETFD as u32,
                libc::F_SETFD as u32,
                libc::F_GETFL as u32,
                libc::F_SETFL as u32,
            ],
        );
        // prlimit64 may read this process's limits, never change any process's.
        filter.extend([
            eq(libc::SYS_prlimit64 as u32, 0, 12),
            stmt(BPF_LOAD, ARG0),
            eq(0, 2, 0),
            eq(pid, 1, 0),
            stmt(BPF_RETURN, DENY),
            stmt(BPF_LOAD, ARG0 + 16),
            eq(0, 1, 0),
            stmt(BPF_RETURN, DENY),
            stmt(BPF_LOAD, ARG0 + 20),
            eq(0, 0, 1),
            stmt(BPF_RETURN, ALLOW),
            stmt(BPF_RETURN, DENY),
            stmt(BPF_RETURN, DENY),
        ]);
        if sealed {
            // Default-deny avoids future/new filesystem syscalls escaping a
            // denylist. No path lookup, open, mutation, socket, exec, io_uring,
            // mount, IPC descriptor acquisition or file-handle calls are allowed.
            let allowed = [
                libc::SYS_read,
                libc::SYS_readv,
                libc::SYS_write,
                libc::SYS_writev,
                libc::SYS_close,
                libc::SYS_fstat,
                libc::SYS_lseek,
                libc::SYS_mmap,
                libc::SYS_mprotect,
                libc::SYS_munmap,
                libc::SYS_mremap,
                libc::SYS_madvise,
                libc::SYS_brk,
                libc::SYS_futex,
                libc::SYS_membarrier,
                libc::SYS_clock_gettime,
                libc::SYS_clock_getres,
                libc::SYS_clock_nanosleep,
                libc::SYS_nanosleep,
                libc::SYS_gettimeofday,
                libc::SYS_times,
                libc::SYS_rt_sigaction,
                libc::SYS_rt_sigprocmask,
                libc::SYS_rt_sigreturn,
                libc::SYS_sigaltstack,
                libc::SYS_exit,
                libc::SYS_exit_group,
                libc::SYS_sched_yield,
                libc::SYS_sched_getaffinity,
                libc::SYS_getcpu,
                libc::SYS_sysinfo,
                libc::SYS_uname,
                libc::SYS_getpid,
                libc::SYS_getppid,
                libc::SYS_gettid,
                libc::SYS_getuid,
                libc::SYS_geteuid,
                libc::SYS_getgid,
                libc::SYS_getegid,
                libc::SYS_getrandom,
                libc::SYS_set_tid_address,
                libc::SYS_set_robust_list,
                libc::SYS_rseq,
                libc::SYS_getrlimit,
                libc::SYS_getrusage,
                libc::SYS_restart_syscall,
                libc::SYS_seccomp,
            ];
            for nr in allowed {
                deny_syscall(&mut filter, nr, ALLOW);
            }
            #[cfg(target_arch = "x86_64")]
            deny_syscall(&mut filter, libc::SYS_arch_prctl, ALLOW);
            filter.push(stmt(BPF_RETURN, DENY));
        } else {
            let denied = [
                libc::SYS_socket,
                libc::SYS_socketpair,
                libc::SYS_connect,
                libc::SYS_bind,
                libc::SYS_listen,
                libc::SYS_accept,
                libc::SYS_accept4,
                libc::SYS_sendto,
                libc::SYS_sendmsg,
                libc::SYS_sendmmsg,
                libc::SYS_recvfrom,
                libc::SYS_recvmsg,
                libc::SYS_recvmmsg,
                libc::SYS_execve,
                libc::SYS_execveat,
                libc::SYS_ptrace,
                libc::SYS_process_vm_readv,
                libc::SYS_process_vm_writev,
                libc::SYS_pidfd_open,
                libc::SYS_pidfd_getfd,
                libc::SYS_pidfd_send_signal,
                libc::SYS_io_uring_setup,
                libc::SYS_io_uring_enter,
                libc::SYS_io_uring_register,
                libc::SYS_mount,
                libc::SYS_umount2,
                libc::SYS_pivot_root,
                libc::SYS_chroot,
                libc::SYS_unshare,
                libc::SYS_setns,
                libc::SYS_open_by_handle_at,
                libc::SYS_name_to_handle_at,
                libc::SYS_open_tree,
                libc::SYS_move_mount,
                libc::SYS_fsopen,
                libc::SYS_fsconfig,
                libc::SYS_fsmount,
                libc::SYS_fspick,
                libc::SYS_mount_setattr,
                libc::SYS_bpf,
                libc::SYS_userfaultfd,
                libc::SYS_perf_event_open,
                libc::SYS_add_key,
                libc::SYS_request_key,
                libc::SYS_keyctl,
                libc::SYS_kexec_load,
                libc::SYS_init_module,
                libc::SYS_finit_module,
                libc::SYS_delete_module,
                libc::SYS_reboot,
                libc::SYS_syslog,
                libc::SYS_acct,
                libc::SYS_swapon,
                libc::SYS_swapoff,
                libc::SYS_fchmod,
                libc::SYS_fchmodat,
                libc::SYS_fchown,
                libc::SYS_fchownat,
                libc::SYS_setxattr,
                libc::SYS_lsetxattr,
                libc::SYS_fsetxattr,
                libc::SYS_removexattr,
                libc::SYS_lremovexattr,
                libc::SYS_fremovexattr,
                libc::SYS_utimensat,
                libc::SYS_setrlimit,
                libc::SYS_tkill,
                libc::SYS_setuid,
                libc::SYS_setgid,
                libc::SYS_setreuid,
                libc::SYS_setregid,
                libc::SYS_setresuid,
                libc::SYS_setresgid,
                libc::SYS_setfsuid,
                libc::SYS_setfsgid,
                libc::SYS_setgroups,
                libc::SYS_capset,
            ];
            for nr in denied {
                deny_syscall(&mut filter, nr, DENY);
            }
            // New common-number ABI calls not present in older libc bindings.
            for nr in [452, 454, 456, 457, 458, 459, 460, 462] {
                deny_syscall(&mut filter, nr, DENY);
            }
            #[cfg(target_arch = "x86_64")]
            for nr in [
                libc::SYS_fork,
                libc::SYS_vfork,
                libc::SYS_chmod,
                libc::SYS_chown,
                libc::SYS_lchown,
                libc::SYS_utime,
                libc::SYS_utimes,
                libc::SYS_futimesat,
                libc::SYS_iopl,
                libc::SYS_ioperm,
                libc::SYS_modify_ldt,
            ] {
                deny_syscall(&mut filter, nr, DENY);
            }
            filter.push(stmt(BPF_RETURN, ALLOW));
        }
        filter
    }

    fn install_filter(sealed: bool) -> Result<(), String> {
        let mut instructions = policy(sealed);
        let program = libc::sock_fprog {
            len: u16::try_from(instructions.len()).map_err(|_| "oxide seccomp program too long")?,
            filter: instructions.as_mut_ptr(),
        };
        no_new_privileges()?;
        // SAFETY: the kernel copies this initialized cBPF vector synchronously;
        // program/instructions stay live, and no pointer is retained. TSYNC=1
        // makes success0 apply to all existing threads; positive TID is failure.
        let result = unsafe { libc::syscall(libc::SYS_seccomp, 1u32, 1u32, &raw const program) };
        if result < 0 {
            return Err(error("seccomp TSYNC installation"));
        }
        if result != 0 {
            return Err(format!("seccomp TSYNC could not restrict thread {result}"));
        }
        // SAFETY: scalar no-argument prctl queries, after successful filter install.
        let mode = unsafe { libc::prctl(libc::PR_GET_SECCOMP, 0, 0, 0, 0) };
        // SAFETY: PR_GET_NO_NEW_PRIVS requires zero in unused scalar arguments.
        let nnp = unsafe { libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) };
        if mode != 2 || nnp != 1 {
            return Err("seccomp/no-new-privileges verification failed; worker must exit".into());
        }
        Ok(())
    }
}
