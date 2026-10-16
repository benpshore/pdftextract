# Combined PR273/native recovery checkpoint

**For Ada's review; all promotion and qualification holds remain.** Work is on
`codex/pr273-native-combined-checkpoint-20261016`. No PR was created, closed,
merged, or advanced. PR273 remains the sole tracked integration task and its
observed head remains `88822fddfca1c84f81bb3e9c2cd0bdd3ce8fbb35`.

## Exact sources and resulting changes

| Commit | Purpose |
| --- | --- |
| `c2c87b22eb23da415e960088bbe4a784a5d76d94` | Two-parent integration of PR273 checkpoint `88822fddfca1c84f81bb3e9c2cd0bdd3ce8fbb35` and native checkpoint `4e60d3949b12f9d3baeb36067e7804ac67b39c6e`. Preserves both complete histories. |
| `ad9fd7532e9534de5ac39aef43c9f8806c5be182` | Repair the parent-death test's readiness race under a finite inherited address-space limit. |
| `4514dc1410851a2a186048be7bd9c64d7a30c0f3` | Reject precision-damaged flatten transforms and separately drawn Link borders that the pinned writer loses. **Exact tested code commit.** |

The tested tree is `36fe1e2bf4536bcac48f33c2563ab91957292d30`.
The preserved binary is `tpe git-4514dc1`, SHA-256
`03c2fa604500e1db468f0c0734f1cd21d24506b05defe0a8874aaaeafb630ccf`.
This handoff and its receipts are an evidence-only follow-up to that code commit.

The original branches overlap in four files. The automatic merge lost the
`allow_empty` option's scope after PR281 introduced the nonblocking acquisition
helper; the compiler rejected it. The integration passes that option explicitly,
tests both FIFO acquisition modes, and preserves document empty/size rejection
while permitting bounded empty auxiliary passwords. Native operation supervision
explicitly initializes PR273's `dry_run` option to false. Offline planning, font
advances, column order, uncertainty, strict diagnostics, and native worker/engine
guards are retained.

No dependency refresh was performed. PR273's lockfile equals main's; the combined
lockfile is byte-identical to the native checkpoint. It adds 113 package identities
without removing old identities or changing shared package checksums. PR281's
`rustix` filesystem feature is combined with the native feature additions.

## Failures preserved and repaired

The first clean combined run at `c2c87b2` had one real test failure:
`kernel_parent_death_signal_terminates_even_a_stopped_parser` reported a surviving
stopped parser. Its readiness check accepted the runner's inherited finite 8 GiB
limit before worker initialization. Production installs and verifies `PDEATHSIG`
before lowering the worker limit. The test now runs in a disposable child with a
finite inherited cap, waits for a strictly lower installed limit, confirms the
actual stopped state, then requires death after its controller is killed. Runtime
containment was not weakened. The failed clean source, binary, full log, and receipt
remain preserved; the test does not claim protection before initialization.

Ada's source-derived flatten precision concern reproduced for both annotation and
form flattening. With a 100-point destination, `BBox` extent 1e9 produced zero scale
and collapsed all four corners. Extent 8e7 produced a nonzero rounded scale with
about **20 points** of corner displacement. All four damaged outputs were admitted
by the prior wrapper. The new shared admission models the pinned f32 arithmetic,
six-decimal overlay, and five-decimal saved bounds, then compares transformed
corners in page units. It rejects degeneracy or error above 0.001 points, including
saved clipping-bound drift. Nonidentity appearance matrices remain unqualified
and are explicitly rejected; their placement is not repaired or certified here.

Independent Poppler rendering exposed another failure: the original annotation
positive controls lost a separately drawn Link border despite correct corners.
That failed render receipt is retained. Link flattening now requires explicit zero
external border width. The original bordered fixture remains, with negative tests
for `Border`, default border, and `BS`; all three upstream outputs independently
render differently. Separate border-free precision controls retain their intended
test purpose. No old receipt or exception was rewritten to hide this finding.

## Exact validation

Linux x86_64, Rust 1.98.1, CPython 3.14.7. Cargo used two build jobs and two test
threads, 8 GiB process address-space ceilings, and finite outer deadlines. Runtime
controls used private disposable workers, smaller limits, fixed synthetic or
approved repository fixtures, and explicit kill/reap timeouts.

| Check at `4514dc1` | Outcome |
| --- | --- |
| Format and strict workspace/all-target Clippy with `pdf-oxide-rendering,pdf-oxide-barcodes,pdf-oxide-signatures,pdf-oxide-ocr,docling-text,pdfium,pdf-extract,grobid` | Passed. |
| Exact clean workspace tests with those features | **1,365 top-level passes**, five additional supervised child-probe passes, zero failures, 15 ignored entries; five additional finite containment scenarios. Includes all 44 editing/form tests and actual PDFium Unicode regressions with the rehashed pinned library. |
| Ruff and Python tooling tests | Passed; **241** tests on Python 3.14.7. |
| Provider ABI Clippy/tests | Passed on x86_64; nine tests, two ignored entries. ARM64 is not inferred. |
| Web source tests, source verification, local package build | Passed; 28 web tests. Local build artifacts were not published. |
| Real validator CLI with synthetic retained evidence | Three expected rejections: historical manifest drift for Partial and Complete, and wrong producer commit. Zero historical exceptions applied. |
| Eight frozen extraction fixtures | Expected Partial results for all eight, 29 pages, unchanged inputs, correct source identities, clean captures, sealed workers; three table candidates remain explicitly unverified. |
| Frozen synthetic offline OCR | Passed bounded plumbing checks, two deterministic image placements, unchanged input/model bytes, no native-text replacement. Constant graphs ignore image content; no OCR accuracy claim. |
| Four prior flatten-carrier negative controls | Correct rejection, unchanged source, no publication, clean captures. |
| Eight prior widget-state controls | Four expected rejections and four successful direct-state cases; matching receipts and independent first-page renders. |
| Eleven precision/border CLI controls | Seven expected rejections without publication; four successful ordinary/exact-tiny-scale controls with matching artifact receipts and independent renders. |

All 32 supervised CLI invocations had their expected outcomes. Source commit/tree
and clean status matched before and after verification; the saved binary digest
also remained unchanged. These checks cover concrete backpressure, publication
rollback, cancellation, finite memory limits, immutable acquisition, process/network
denials and loopback non-access cases. They do not establish broad SSRF/RCE safety,
unbounded fork-bomb containment, shared RSS/thread-count bounds, crash-atomic
visibility of output pairs, or general extraction/rendering/Office/OCR fidelity.
Parent-death probes prove termination; orphan zombies can await PID 1 reaping.

## Unresolved gates and unchanged holds

- Full combined **Docling** check failed before tests: pinned `ort-sys` download
  from `cdn.pyke.io` returned CONNECT proxy HTTP 403. No pin or build bypass.
- `uv audit` could not connect to `api.osv.dev`. Swift, CMake and CTest commands are
  absent in this VM; their attempted failures are preserved. Mac jobs stay paused.
- Landlock returned ENOSYS; its positive filesystem allowlist probe did not run.
  Finite seccomp probes are not a substitute for all sandbox/platform guarantees.
- Fresh exact-source ARM64 Native evaluation and review of real retained outcomes
  are still required. The historical Partial exception policy remains ineligible
  for these changed source/dependency identities. No exception hashes were renewed.
- The Python package remains a scaffold; Python engine/API parity and the exhaustive
  toolkit parity program remain subsequent work. All **84** native capability
  obligations remain unqualified: 66 partial adapters, six unsupported, 11
  source-only, one parent-owned parity obligation; 70 operation names are registered.
- PR226's global hold, PR163, PR135/136/138/141/143, PR275/280/281 refresh holds, and
  all other parent coordination constraints remain. No hold blocks the explicitly
  authorized nonpublishing combination itself; none was revoked.

## Preservation and proposed next action

[Backlog source index](../backlog-2026-10-15/sources.json) and every original native
inventory, capability specification, matrix and evidence file are preserved
byte-for-byte. The index is a dated snapshot of **119 exact PR heads**, not a claim
about today's open-PR count. All 119 archive refs still match. Its complete-history
bundle remains verified at `/workspace/tpe-integration-evidence/open-pr-sources-20261015.bundle`,
SHA-256 `ac4494d1762d90667d9fc8ef306c01c446a2dbfefa1aeace181ad23dc8573085`.

The original PR273 `7cabac188b5d89c5f0a9127392f5213df7d349a1`, PR280
`bae267aff0c999c452ede13c5dea079c5243350a`, PR281
`059436c46b4e352a38d62a7922dca05ad0c927cf`, main `b3730aa`, and both requested
checkpoints are ancestors. The held refreshed PR280 `e17c2eaa2af4e9f7b5fc19799d3409d41ab8026d`
and PR281 `0c4533dd413c3e50949b7e4b68c2991ca2c93a43` were not substituted.
Before/after remote snapshots show unchanged main and PR273/280/281 heads.

PR279's provider implementation, all PR273 workflows including PR274's Mac pause,
LICENSE, strict validator, historical exceptions, and corrected model pins retain
their expected hashes. Existing synthetic failures and raw license/log whitespace
are intentionally retained. Only rebuildable Cargo incremental cache was removed;
source, saved binaries, fixtures, and evidence were preserved. No Mac/Documents,
private imports, credential discovery, new architecture, repository creation,
visibility/license changes, protected merge, release or artifact publication occurred.

The competing 54→55 and 251→263 intake designs remain separate. Old 84/92 are not
wholesale imported. PR64 remains preserved for Ada; prior closures of
155/156/161/170/240 are recorded without further closures.

Proposed action for Ada: review this separate recovery branch and its new guards
as the candidate continuation of existing PR273. It is ancestry-compatible with a
future fast-forward, but **no PR-head update or promotion is performed here**.
Resolve the stated exact-source/platform/Native gates under parent coordination.

[combined-status.json](combined-status.json) provides the machine-readable overlay;
[evidence-manifest.json](evidence-manifest.json) hashes the copied receipts, runners,
and unmodified logs. Full raw outputs, preserved binaries and rendered synthetic
fixtures remain under `/workspace/tpe-combined-integration-evidence-20261016/`.
