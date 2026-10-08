# Independent scan-service audit — 2026-10-08 UTC

Reviewed PR #265 at exactly `21624ad1bb9096cf8f0cc95d9b0beeb4ed353192`,
tree `351ee713a46dd22680748ed86e50f1f29e700536`.

**Verdict: rework.** Two authenticated resource/cleanup defects remain at
that head. This follow-up fixes them on a separate branch; #265 remains
unchanged. Do not merge or deploy either draft on the strength of this report.

## Demonstrated defects

1. **Multipart metadata amplification in the parent.** `scan_response` calls
   `multipart::parse` in the service process. That parser creates every part
   and copies its metadata without a part-count or part-header budget. An
   exact-source standalone witness with 100,000 tiny parts accepted a
   1,500,007-byte body and allocated at least 12,582,912 bytes for the `Part`
   vector alone on this 64-bit host. Payloads, metadata strings and the
   original upload are additional. Scaling the same encoding to the default
   64 MiB body ceiling permits millions of records and roughly 0.8 GB of
   vector capacity alone; this extrapolation is not an OOM experiment.
   Authentication and scan admission apply, but worker address-space limits
   do not protect this allocation. The correction rejects more than 64 parts
   and bounds each part's header section, including its terminator, to 16 KiB
   before decoding/copying metadata. Ordinary single-file uploads still pass.

2. **Response delivery blocks shutdown and has a renewable timeout.** After
   worker cleanup, `handle_connection` calls blocking `Response::write_to`.
   The socket's 30-second write timeout applies to individual writes, and
   `write_all` can renew it after partial progress. Shutdown joins that
   handler without interrupting its write. A disposable worker returning a
   valid 16 MiB error result demonstrated that a client which reads just the
   response prefix then stops prevents shutdown from completing within two
   seconds; closing that client releases the old handler. The correction
   uses a non-renewable response deadline with at most 50 ms socket waits and
   shutdown polling, including the accept-loop overload response.

## Control and reproduction

The original, unmodified service's 33 unit and 15 integration tests passed
locally. A detached worktree of the exact reviewed head, with only this
branch's `tests/service.rs` copied into it, returned **17 passed / 2 failed**:

- `oversized_multipart_metadata_is_rejected_and_admission_is_released`: old
  result `empty_body`, expected `bad_multipart`; it parsed all 65 empty parts.
- `shutdown_interrupts_a_response_to_a_nonreading_client`: shutdown blocked.

Two standalone rejection witnesses compiled against the exact original
`multipart.rs` also failed: 100,000 accepted parts and a 16 KiB `name` field
accepted as part metadata. No control implementation was changed.

To repeat the service control, from this follow-up checkout:

```sh
git worktree add --detach /tmp/tpe-scan-control 21624ad1bb9096cf8f0cc95d9b0beeb4ed353192
cp crates/tpe-scan-service/tests/service.rs /tmp/tpe-scan-control/crates/tpe-scan-service/tests/service.rs
cd /tmp/tpe-scan-control
cargo test --locked -p tpe-scan-service --test service
```

The subsequently added stopped-worker probe should also pass on the control;
the two defect witnesses above should still fail. On the corrected checkout:

```sh
cargo test --locked -p tpe-scan-service
cargo test --locked -p tpe-scan-service --features docling-text
cargo clippy --locked -p tpe-scan-service --all-targets --features docling-text -- -D warnings
node web/tests/scan-service.test.mjs
```

## Other boundaries inspected and exercised

| Boundary | Source/control-flow evidence and exercised cases |
| --- | --- |
| Pre-authentication | Bounded 16 KiB request head, 64 headers, at most one 4 KiB body-prefix read; duplicate security/length headers rejected. Host/Origin/token checks precede length validation, admission, body growth and `100 Continue`. Existing incomplete/oversized unauthenticated request witnesses passed. |
| Read deadlines | Header deadline is acceptance plus header budget; upload deadline is acceptance plus header and body budgets. `read_before` recalculates remaining time and polls cancellation. Trickle and incomplete-read shutdown tests passed. |
| Admission | Atomic gate acquired before upload and parent multipart processing, held through image worker execution and pipe joins. Busy intake, upload cancellation and new multipart rejection/retry tests passed. |
| Cancellation races | Registration/tombstone lookup share the jobs mutex. Cancellation flags are polled during intake and worker supervision; acknowledgement waits for `finished` after slot release. Overtaking headers, targeted cancellation, registry saturation with explicit rejection, and expiry/retry tests passed. IDs are per-operation, not durable idempotency records. |
| Worker lifetime | Private process group established at spawn; kill-on-drop sends group SIGKILL, then kills/waits the direct child. Writer/stdout/stderr threads are joined before returning. Deadline, disconnect, targeted cancellation, exited-worker descendants holding pipes, and a stopped worker with a blocked 1 MiB stdin writer passed. |
| Stderr | Separate fixed 4 KiB read chunk and 4 KiB tail ring; no accumulating stderr vector. The 32 MiB synthetic reader and flood/cancellation regressions passed. UTF-8 rendering can expand the final diagnostic, but remains bounded. |
| Failure cleanup | Spawn errors return before pipe setup; later exits use the reap guard. Malformed image/scan, upload abort, multipart rejection, worker exit and timeout release capacity. New response shutdown/deadline probes pass after the fix. |

## Final local checks

Linux x86-64, Rust 1.98.1. Default service: **36 unit + 20 integration passed**.
`docling-text`: **36 unit + 21 integration passed**. Browser client: **51 checks
passed**. Formatting, diff whitespace checks, default repository/all-target
strict Clippy and feature service strict Clippy passed. Ruff format/check,
**210 Python tests**, and locked Python dependency audit passed.

The required root `cargo test` attempt passed its 753 library units and 14 CLI
units, then stopped at the same three CLI process-observation assertions
listed in #265's prior validation: `tests/cli_containment.rs:408`,
`worker did not become observable`. That suite returned 15 passed / 3 failed.
This follow-up does not change those root CLI implementation or test files;
this is not a passing full repository test claim. Swift and CMake commands
were attempted and are unavailable on this host.

## Qualification boundary

No full Docling OCR/model qualification, real browser-to-service CORS/LNA,
Safari/iOS, deployment, merge or release is claimed. The transport witnesses
use real loopback sockets and disposable worker fixtures. Real PDF extraction
and PNG conversion run through `docling-text`. Private process groups assume
descendants retain group membership; this is not a hostile-code process-tree
sandbox. New macOS behavior and hosted CI belong to the follow-up commit and
must be checked separately. The added repair is agent-authored and should
remain draft for review of its final head.
