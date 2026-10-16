# Disposable-worker containment

The implementation in `crates/tpe-ffi/src/oxide_containment.rs` supplies three
distinct Linux policies. It is an irreversible worker boundary, not a PDF safety,
extraction fidelity, kernel isolation or general RCE certificate. It must never
run in the controller or an interactive process.

| API | Evidence mode | Filesystem behavior |
| --- | --- | --- |
| `install_process_policy()` | `seccomp_process` | Filesystem access remains available; `installed()` is false. |
| `install_sealed_process()` | `seccomp_sealed` | Default-deny syscall allowlist; no path opens, reads, metadata queries, directory enumeration or mutations. Zero read paths are granted. |
| `install(&[PathBuf])` | `landlock_read_only` | Landlock ABI >= 6 read-only file/directory-content allowlist plus the process policy. Other pathname metadata is not fully hidden by Landlock. |

The sealed policy never silently replaces a requested nonempty Landlock allowlist.
It supports already-loaded memory operations and inherited stdio. Dependencies
that require opening fonts, models or files can fail under this policy. A guard
allows attempting an operation; it is not evidence that the operation is correct.

`Evidence` records the actual mode, queried Landlock ABI where applicable,
filesystem/socket/process enforcement, successful seccomp TSYNC, and the count
of read-only paths. `installed()` only becomes true after an enforced filesystem
policy. For Landlock it conservatively reports only the installing thread; future
threads inherit kernel restrictions but do not inherit that thread-local marker.
Sealed mode reports enforcement across existing and future threads.

## Installation contract

The trusted worker startup must first load immutable request/input/model bytes,
close acquisition descriptors and install memory/time/output limits. Parser and
dependency work must not race installation. An existing lease thread is allowed
for seccomp if it only reads its inherited stdin pipe and does not open files or
create descriptors during setup.

Before installation, `/proc/self/fd` is inspected. Non-stdio descriptors are
rejected. Stdio must be pipes/capture files; `/dev/null` is additionally accepted
for test stdin. Regular stdin files, directory descriptors and socket/device
capabilities are rejected. The caller owns the identity and output budget of
stdout/stderr captures. No private inputs should already exist in worker memory.

Landlock additionally requires exactly one existing thread. It accepts at most
16 canonical, non-root absolute read paths, each a regular file/directory or
`/dev/urandom`. It handles all ABI 1–5 filesystem rights, grants only content-read
and directory-read rights, denies TCP bind/connect and sets both ABI 6 scopes.
No writes are granted. There is no best-effort fallback on unsupported kernels.

Every policy sets `no_new_privs` and installs its seccomp filter using TSYNC.
The kernel return value, seccomp mode and `no_new_privs` are checked before the
success marker is set. Failure after an irreversible step requires the disposable
worker to exit; the caller must not continue parsing. Installation is supported
only on little-endian Linux `x86_64` and `aarch64`; other platforms return errors.

## Scope and limits

Both seccomp variants reject other architectures, and the x86 policy rejects the
x32 ABI. Socket creation and communication, exec, fork/vfork and process clones
are denied. `clone3` returns `ENOSYS`; legacy clone permits only a constrained
pthread-style flag set requiring `CLONE_THREAD`, `CLONE_VM` and `CLONE_SIGHAND`.
Ordinary Rust thread creation remains functional in the observed tests. Signals
are limited to the current process. Privilege, mount, file-handle, ptrace,
cross-process memory, io_uring and related acquisition paths are blocked.

The sealed policy denies unknown syscalls. It permits memory allocation,
synchronization, selected thread/signal/time primitives and stdio IO. It does not
prevent native code from executing inside the worker or protect data already in
memory. The process-only policy uses a narrower denylist and leaves filesystem
access and some other kernel interfaces available; it must not be presented as
filesystem or complete IPC isolation. Landlock cannot hide all pathname metadata.

There is **no separate PID, cgroup or thread-count quota**. Threads still consume
resources, so the caller's address-space/CPU/deadline limits and kill/reap logic
remain necessary. This work supplies finite process-creation denial evidence,
not a fork-bomb stress test. It does not establish broad SSRF/RCE resistance,
security against kernel vulnerabilities, absence of covert channels, OCR parity
or document correctness. It changes no host settings and requires no privilege
escalation, persistent registration or private paths.

The source review uses the pinned [Linux 6.18 Landlock UAPI](https://raw.githubusercontent.com/torvalds/linux/v6.18/include/uapi/linux/landlock.h),
[x86 syscall table](https://raw.githubusercontent.com/torvalds/linux/v6.18/arch/x86/entry/syscalls/syscall_64.tbl)
and [common syscall table](https://raw.githubusercontent.com/torvalds/linux/v6.18/scripts/syscall.tbl),
with the [kernel seccomp filter documentation](https://docs.kernel.org/userspace-api/seccomp_filter.html).
New metadata/mount variants `setxattrat`, `removexattrat`, `open_tree_attr` and
`file_setattr` are explicitly denied by the process policy as well as implicitly
denied by sealed mode. Current documentation for newer Landlock ABIs must not be
mistaken for features available on this kernel.

## Observed evidence

[containment-evidence.json](containment-evidence.json) records exact source and
linked dependency hashes, compiler version, every command, output and log hash.
Compilation used the existing pinned toolchain and locked dependency rlibs
directly, without concurrent Cargo execution. These are isolated module checks;
the integration owner must attach exact combined Cargo-build/test evidence.

- Production module, private FFI tests and the harness executable passed direct
  Clippy compilation with the repository's pedantic lint configuration.
- Five bounded child scenarios passed: process policy, sealed policy, unexpected
  inherited descriptor rejection, multithreaded Landlock rejection and explicit
  Landlock availability handling. Each child had a ten-second kill/reap deadline.
- Sealed children rejected file reads/writes, metadata, listings, symlink reads,
  removal, rename, chmod, truncation and symlink creation. Sentinels stayed intact.
  The same checks passed in a thread created before TSYNC and a future thread.
- TCP/UDP/UNIX socket creation and exec were denied, while threads, arithmetic and
  captured output worked. No outbound request was attempted.
- Two private FFI test workers made one direct fork, vfork and process-clone call
  each on x86; all returned `EPERM`. `clone3` returned `ENOSYS`, and a normal thread
  still worked. Null-argument probes for all four new metadata/mount variants
  returned `EPERM`. An unexpected child would exit directly in assembly without
  touching Rust state; ignored SIGCHLD guarantees kernel auto-reaping. The outer
  worker is separately bounded and reaped. There is no public unsafe probe API.
- This VM's Landlock ABI query returned `ENOSYS`. **Positive Landlock read-allowlist
  enforcement was not executed.** Aarch64 has source review only, with no compile
  or runtime receipt from this agent. Mac jobs remain paused.

The Cargo entry points for the integration owner's serialized run are:

```sh
cargo test -p tpe-ffi --lib oxide_containment::tests::bounded_raw_process_denials -- --nocapture
cargo test --features pdf-oxide --test oxide_containment
```
