# Scan-service boundary repair — 2026-10-07

Reviewed input: PR #258 at `e529eb2e763af9b1175b44cad0314f7c378dca15`.
The parent PR is unchanged; this correction is stacked on that exact head.

## Corrected behavior

- The bounded head is authenticated before length validation, body accumulation,
  `100 Continue`, or worker admission. At most a fixed 4 KiB chunk may include
  a body prefix during header acquisition. Body storage grows with received
  bytes only. The scan slot covers upload and execution.
- Header and upload deadlines are absolute from socket acceptance. Progress
  does not renew either deadline; polling permits prompt shutdown/cancellation.
- PNG/JPEG decoding, color conversion, compression and PDF wrapping occur in
  the gated child, after installation of its address-space limit, under the
  same worker deadline as extraction. Result input kind, warnings and timing
  now come from that worker.
- Disconnect/shutdown/explicit cancellation kills the operation's private
  worker process group and joins all pipe threads before releasing its slot.
  `/cancel?id=...` uses the same authentication and origin policy and accepts
  no body. It acknowledges only after admission is released. A fixed cap of
  64 expiring cancellation IDs covers cancellation overtaking scan headers.
- The browser generates a per-operation UUID, aborts transport and sends a
  separate authenticated cancellation request with a 6 s deadline. Its
  promise waits for cleanup acknowledgement; failure is `cancel_pending`.
  Abort listeners are removed, and late parsing cannot report success.
- Stderr is continuously drained through fixed 4 KiB buffers; only the last
  4 KiB is retained, including during flood and cancellation.

## Local validation

Linux x86-64, Rust 1.98.1, Node 24.19.0. Rust debug information and incremental
compilation were disabled after this shared workspace ran out of disk; only
this task's build outputs were cleaned before rerunning.

| Check | Result |
| --- | --- |
| Default scan service | 33 unit + 15 loopback integration, passed |
| `docling-text` scan service | 33 unit + 16 integration, passed; real PDF text extraction and PNG wrapping in child |
| Scan-service `docling-text` strict Clippy | Passed |
| Workspace/all-targets default strict Clippy | Passed |
| Rust formatting | Passed |
| Browser client Node suite | 51 checks passed, including default XHR and fetch cancellation |
| Full web TypeScript and focused ESLint | Passed |
| Web portable source verification | All 156 files verified; manifest regenerated |
| Ruff format/check | Passed |
| Python tests | 210 passed |
| Locked Python dependency audit | Passed, no vulnerability reported |
| Full default workspace Rust tests | Root 753 unit + CLI 14 unit passed; run stopped at three CLI containment failures described below |
| Swift/CMake | Unavailable executables on this Linux host; attempted and recorded |

Seven API-compatible loopback regressions were copied into a detached control
worktree of the reviewed head. Existing six integration tests passed; all
seven new witnesses failed: pre-auth body admission, gated image intake,
disconnect, targeted cancellation, stderr-flood cancellation, cancellation
before headers, and incomplete-upload cancellation. No control change was
published. The new trickling-deadline regression and fixed-buffer 32 MiB
stderr reader probe pass on the correction.

The same three root CLI containment tests also fail on the reviewed head,
with the same `worker did not become observable` assertion at
`tests/cli_containment.rs:408`: `deadline_and_cancellation_kill_and_reap_a_stopped_worker`,
`kernel_parent_death_signal_terminates_even_a_stopped_parser`, and
`cancellation_does_not_launch_relays_for_the_whole_pending_batch`.
The control CLI suite reports 15 passed / 3 failed. These existing root
process-observation failures are retained, not changed or waived by this repair.

The focused service workflow runs default and `docling-text` tests plus
strict Clippy on Ubuntu and macOS. Web CI now executes the client suite and
focused lint. Hosted results belong to the exact published commit and must
be assessed separately from these local checks.

## Boundary and remaining qualification

No full `docling` OCR/model qualification, full corpus extraction, real
browser-to-service CORS/LNA execution, Safari/iOS runtime, UI mounting,
deployment, merge or release is claimed. The browser regressions use synthetic
XHR/fetch fixtures; the service regressions use real loopback sockets and
synthetic disposable process fixtures, plus real `docling-text` extraction.
The service remains a library/unmounted client feature at this checkpoint.

This is an implementation and regression report, not independent review.
Keep the correction draft pending independent review and terminal hosted CI.
