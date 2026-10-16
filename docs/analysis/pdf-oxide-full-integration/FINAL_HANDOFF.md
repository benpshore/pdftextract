# Native recovery checkpoint for Ada — 2026-10-16

**The native checkpoint, including the subsequent flatten carrier guard, passes its scoped Linux checks. It remains separate from
PR273's backlog checkpoint and is held for parent integration review.** Existing
PR273 remains the integration task; this work opened, closed, merged or updated no
PR. No capability is fully qualified, and this is not the exhaustive toolkit or
Python binding parity program.

| Identity | Value |
| --- | --- |
| Native branch | `codex/pdf-oxide-rust-recovery-20261015` |
| Tested native source | `a39af2b312a39f36f384031d7f81bb22d22daf59` |
| Tested source tree | `ed4aa34eca5fc11e0a1785e759548b95328eb465` |
| Native branch base | `b3730aa958d947fcbf6ea45b0efef07fcfa20f29` |
| Preserved binary version | `tpe git-a39af2b` |
| Binary SHA-256 | `ba82387554bb695840f0a18c13f9624005daab923b18cc9ec50ed1f8efa69295` |
| Separate PR273 checkpoint | `88822fddfca1c84f81bb3e9c2cd0bdd3ce8fbb35` |

The source tree was clean and unchanged throughout the complete workspace run,
shared-fixture evaluation and OCR control. The later documentation-only commit
archives those original receipts; the binary does not claim that later commit's
identity. [The manifest](evidence/exact-a39af2b-manifest.json) indexes exact bytes,
commands, hashes, local evidence paths and the archived execution scripts.

The runtime changes bind publication to a fresh invocation, ordered immutable
staged input digests/sizes, operation, public request options, canonical producer
and exact response capture. Acquisition occurs in a bounded disposable worker;
private request/password digests stay private. Seven transport tests cover
valid-looking source substitutions, replay, option/operation swaps, changed
artifact capture, build identity and already-published state, with an exact-byte
positive control. These checks establish transport consistency for those cases;
they do not establish semantic correctness of parser output.

Known data-loss paths now have explicit admission checks. Text-consuming adapters
reject the pinned shallow-subtype Form omission and bound nested resource work.
Conservative text-work checks reject demonstrated silent native operator caps;
native warning provenance and missing page identities remain distinct from
adapter context. Query/conversion completeness is unverified or explicitly
truncated. Valid PDFs can be rejected by these deliberately conservative checks.

Redaction requires bounded explicit targets, rejects unsupported ActualText,
saved text state and carrier/page-tree structures, and independently verifies
target presence before and absence after in decoded objects and text. It makes
no universal erasure claim. Form value/style changes with stale appearances,
insertion into an existing AcroForm, and unqualified appearance regeneration are
rejected. Annotation flattening requires a supported normal appearance and
verifies selected removal/nonselected preservation. The missing-appearance proof
distinguishes retained widget values from static page painting; native regeneration
has a separate positive control, so no-AP alone is not treated as universal loss.

Flattening also rejects affected selected pages without supported explicit
`/Contents` or a local `/Resources` dictionary. The raw page-tree check avoids
the normal API's inherited-resource projection. The original no-Contents form
becomes blank in Poppler; the inherited-resource form loses body text and image
while retaining the appearance. Positive local direct/indirect-resource controls
verify original content operations, resource bindings and appearance invocation,
including explicit empty content arrays. These are concrete admission guards,
not general visual fidelity qualification.

Earlier repaired native behaviors and explicit gaps remain recorded in
[CHECKPOINT.md](CHECKPOINT.md) and [NATIVE_API.md](NATIVE_API.md): bounded Office
preflight, Unicode annotation reading, form geometry/properties, independent
encryption credentials, source-preserving repair provenance, offline OCR budgets,
and publication/receipt rollback. Known defective table creation, HTML/CSS layout
and text replacement paths remain unsupported. Table extraction exposes unverified
candidates; cleanup is a separate preview that preserves the baseline. Original
failing runs and independent Poppler observations have not been overwritten.

| Exact check | Outcome and limit |
| --- | --- |
| Rust 1.98.1, Linux x86_64; workspace formatting and strict all-target Clippy | Passed with rendering, barcodes, signatures and offline OCR enabled together. |
| Workspace tests with those four features | 1,293 top-level passes plus 3 nested child-probe passes; zero failures; 10 ignored. Five additional bounded containment harness scenarios ran. |
| Python 3.14.7 and Ruff | 210 tooling/validator tests passed; formatting and lint passed. This does not test a functional Python engine binding. |
| Eight frozen approved shared fixtures | All returned `partial` / exit 1, covering 29 expected pages with matching source identities, unchanged source bytes, reported parser sealing and cleaned captures. All three detected tables remained unverified candidates. This is not an extraction-accuracy pass. |
| Frozen offline OCR control | Passed: all four input digests preserved, two separate image placements, deterministic `AB`, 10,056 processed pixels within 16,384, actual sealed worker, no native-text replacement/artifacts and cleaned captures. Constant graphs ignore image content; this is not trained OCR accuracy. |
| Four preserved flatten carrier defects through the clean supervised CLI | All explicitly rejected with expected reasons, no output publication, unchanged inputs and cleaned private captures. |
| Dependency audit | Unavailable: OSV tunnel/connect failure after retries. |
| Swift/CMake/CTest | Commands unavailable in this VM. No Mac job was enabled or run. |

The native test command was:

```sh
cargo test --locked --workspace \
  --features pdf-oxide-rendering,pdf-oxide-barcodes,pdf-oxide-signatures,pdf-oxide-ocr \
  --no-fail-fast
```

The preserved runner applied an outer 8 GiB address-space ceiling, core limit 0,
600-second timeout, and two build/test threads. Shared-fixture subprocesses used
an outer 2 GiB ceiling and 20-second timeout, plus CLI deadlines, 256 MiB memory
growth and 64 MiB output bounds. OCR used the corresponding 15-second outer /
5-second CLI deadline and 1 MiB output bound. These bounds and synthetic fixtures
do not constitute broad resource or hostile-input qualification.

Exact receipts: [workspace](evidence/exact-a39af2b-receipt.json),
[shared fixtures](evidence/shared-exact-a39af2b-receipt.json),
[OCR control](evidence/ocr-exact-a39af2b-receipt.json). Raw logs are committed
alongside them. The preserved compiled binary, large shared-fixture responses and
generated ledgers remain under `/workspace/tpe-pdf-oxide-evidence-20261015/` and
are indexed by hash; no database or release binary is committed.

Failure-path checks include stopped operation-worker timeout/INT/TERM kill/reap,
parent lease/death cleanup, full stdout backpressure, memory/output limits,
publication collisions and rollback, loopback SSRF/action negative controls,
and finite process/path/socket denial probes. The operation worker reports actual
`seccomp_sealed` restrictions with thread synchronization. The acquisition worker,
controller and publisher retain their intended file access. Landlock returns
ENOSYS, so its positive filesystem allowlist probe did not execute. Threads remain
allowed; no separate thread-count, PID/cgroup or shared-RSS guarantee is claimed.
No live fork bomb or unbounded host test was performed.

The coverage overlay still records 84 unique obligations: 66 partial adapters,
6 explicit unsupported capabilities, 11 source-only obligations and one
parent-owned Python parity obligation. Seventy operation names are registered;
registry breadth is not completeness. [integration-status.json](integration-status.json)
keeps the exact native receipts separate from the still-null PR273 combined-source
qualification. All-platform, free-threaded/binding, independent fidelity, broader
frozen-corpus, trained OCR and full CLI/MCP coverage remain outstanding.

Backlog preservation is unchanged. The separate PR273 branch retains the full
histories of head `7cabac188b5d89c5f0a9127392f5213df7d349a1`, main b3730aa,
PR280 `bae267aff0c999c452ede13c5dea079c5243350a` and PR281
`059436c46b4e352a38d62a7922dca05ad0c927cf` through integration commit
`3787073d4b54a5bb515f50d75b4de158698d99db`. Its repairs are tested at
`de1396bf956ebefdd77052796df8311daa71579f`; checkpoint 88822fd carries the
durable 119-head source/dependency index in `docs/analysis/backlog-2026-10-15/`.
The verified complete-history bundle is
`/workspace/tpe-integration-evidence/open-pr-sources-20261015.bundle`, SHA-256
`ac4494d1762d90667d9fc8ef306c01c446a2dbfefa1aeace181ad23dc8573085`.
Competing intake designs 54→55 and 251→263, excluded wholesale 84/92 imports,
PR64, and parent-owned holds retain their obligations. No proposal was closed.

The original Native provenance rejection remains an evidence gate. The historical
Partial policy refers to a different native manifest and Cargo.lock; its hashes
and validator were not restamped or weakened. Download of the original ARM64
artifact returned HTTP 403, and the pinned full Docling runtime download was also
blocked by HTTP 403. Original dumps and a fresh exact combined-source ARM64 Native
run still require review. Synthetic validator rejection tests do not replace them.
The PR273 handoff in the preserved backlog checkpoint records the exact identities.

Ada's next integration step is to review and reconcile this native history with
the separately preserved PR273 repairs, retain all unique source obligations,
and rerun the exact combined-source checks before advancing the existing PR273.
No new PR is needed. Native source a39af2b has no diff from main b3730aa in
`LICENSE`, `.github/`, or `crates/tpe-ffi/src/provider.rs`; the ARM64 repair and
Mac-job pause remain intact. No private code/data, Mac/Documents access, real
credentials/signing, persistent MCP, new queue/database architecture, protected
merge, release or repository visibility/license change occurred in this work.

The flatten carrier follow-up is indexed in
[evidence/flatten-carriers-manifest.json](evidence/flatten-carriers-manifest.json).
Its four original failures, failed harness binary and independent Poppler
observations remain preserved. The complete a39af2b run includes all 39 editing/
form tests. Four additional [supervised rejection controls](evidence/flatten-carriers-exact-a39af2b-receipt.json)
returned the expected carrier errors, retained all source bytes, published no
output and cleaned every private capture. The earlier 3251660 receipts are still
preserved under their original names and hashes.
