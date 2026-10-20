# PR273 integration checkpoint — 2026-10-15

**Held for Ada's review; not qualified for promotion yet.** The existing PR273
remains the sole integration task. Its head is still
`7cabac188b5d89c5f0a9127392f5213df7d349a1`. No issue/PR was opened, closed or
merged. No release, deployment, repository/license/visibility change, private
import, or Mac/Documents access occurred.

Checkpoint branch: `codex/pr273-backlog-checkpoint-20261015`.

| Checkpoint | Purpose |
| --- | --- |
| `3787073d4b54a5bb515f50d75b4de158698d99db` | Four-parent integration preserving the full histories of PR273, main `b3730aa958d947fcbf6ea45b0efef07fcfa20f29`, PR280 `bae267aff0c999c452ede13c5dea079c5243350a`, and PR281 `059436c46b4e352a38d62a7922dca05ad0c927cf`. |
| `de1396bf956ebefdd77052796df8311daa71579f` | Repair independent tight-heading failures and preserve a source/dependency index for all 119 open PRs. Tested code tree: `c66bb7ff89ac8d4283de0cfa094c598851da3d26`. |

## Resulting behavior

PR273's standard font advances, honest width/order uncertainty and offline
planning behavior are retained. Main's ARM64 provider implementation from PR279
is byte-identical, and PR274's Mac job pause remains in place. Cargo.lock,
LICENSE and the historical Partial policy are unchanged from main.

PR280's diagnostic reporting and PR281's nonblocking, descriptor-verified file
acquisition are compatible and integrated in full. Historical model drift still
fails the validator. Diagnostic reference/page baselines never grant Partial
exceptions. FIFO replacement probes remain bounded by a supervisor that kills
and reaps on timeout; regular-file symlinks and original bytes are preserved.

Two independently authored synthetic PDF cases from PR269 failed both on main
and on the initially combined sources: two columns interleaved under a tight
heading, and a three-column heading appeared after the left column. The repair
recognizes a top heading centered over a proven body gutter before choosing a
column cut. It adds linear work per region within the existing line/depth
bounds. Both original handwritten expectations now pass, with a negative control
keeping a right-column heading attached to that column. Raw spans remain intact,
without missing or duplicated membership. Only those fixtures/test were selected;
PR269's companion physical-layout API and other fixtures remain preserved work.

## Validation and limits

Cloud host: Linux x86_64, Rust 1.98.1, CPython 3.14.7. Test commands used two build
jobs/test threads, bounded timeouts and an outer 8 GiB address-space ceiling;
the existing production worker tests additionally install their own smaller
limits and kill/reap deadlines. Approved repository/synthetic fixtures were used.
No public corpus or layout-witness PDFs were downloaded for these tests.

| Check | Outcome |
| --- | --- |
| Default workspace strict Clippy / tests, repaired precommit content | Passed; 1,136 tests, 11 ignored. |
| Exact clean `de1396b` workspace strict Clippy / tests with `docling-text,pdfium,pdf-extract,grobid` | Passed; 1,188 tests, 13 ignored. Source commit/tree matched before and after. The ASR feature-disabled smoke also reports a skip. |
| Ruff format/check and Python 3.14.7 suite at `de1396b` | Passed; 241 tests. These are tooling/validator tests, not Python engine/API parity. |
| Real validator CLI, synthetic retained evidence, clean `de1396b` | Three expected rejections verified: model drift with Partial, model drift with Complete, wrong producer commit. Zero historical exceptions; comparable baseline retained; separate current diagnostics emitted. No monkeypatch of source provenance. |
| Pinned native provisioning | PDFium and all four model assets verified, including layout SHA-256 `87ac925d4183b4f4745fb48f6ff216fe88ce244d9ab635725e6d514fe490fc2a`. |
| Full `docling,pdfium,pdf-extract,grobid` build/check | Blocked before tests: ort-sys could not download pinned ONNX Runtime from `cdn.pyke.io` (CONNECT HTTP 403). No dependency/pin/validator bypass. |
| Exact clean `de1396b` optimized worker/CLI/column checks | Passed; 51 tests, 2 supervised-only ignored probes. Source commit/tree matched before and after. |
| uv audit | Blocked by `api.osv.dev` tunnel/connect failure. |
| Swift/CMake | Commands unavailable in this VM; Mac jobs remain intentionally paused. |

Passing failure-path tests include stopped-worker deadline/SIGINT/SIGTERM
cleanup, parent-death/lease cleanup, full stdout backpressure, malformed-input
continuation, input/output/ledger alias protection, allocation pressure, bounded
publisher JSON expansion, failed publication/commit rollback and publication
collisions. They do not establish a security sandbox, crash-atomic visibility
of output pairs, shared RSS bounds, fork-bomb containment, comprehensive SSRF/RCE
defenses or general extraction fidelity. bwrap could not create its user namespace
because the UID map is read-only; no fork-bomb experiment was attempted.

The Python package entry point is still a Hello scaffold. Functional Python
engine/API parity and the exhaustive toolkit parity program remain subsequent
work; this checkpoint does not claim that acceptance is finished.

## Native failure diagnosis

Original [Native run 38169034277](https://github.com/benpshore/pdftextract/actions/runs/38169034277)
at PR273 head `7cabac1` completed provisioning, compilation, tests and all four
evaluation processes. Job `114557539328` then rejected every backend with
`reviewed-partial provenance mismatch: native_manifest_sha256`.

| Identity | Historical policy | Combined source |
| --- | --- | --- |
| Native manifest SHA-256 | `194f06dd0a127a2faea3e2c815c9278e61d8cada1b7477f3bafddddee27fb156` | `8209444469c506439c8d6af65944c756da78199ff4be6ce3d60846942ac5c782` |
| Cargo.lock SHA-256 | `ae0367f2f9391ec98fd0a125bc516e0179419b311e4f1c7693ad3052b3b7f8c0` | `c903547b84464b062fc3dd67dc45892aac0ae51fe940b247dee0f9c95b89f365` |

Corpus, metric-source and truth-source hashes still match. PR273's separately
investigated model pin fixes main's stale provisioning expectation; its downloaded
bytes verified again here. This does **not** renew the historical Partial review.
The policy SHA-256 remains
`ff92be145cb8fa57e0948f984088c21079c9ad5a85bdbdb27b53d5ceed33e234`.

Artifact `11714096388` is listed as unexpired and bound to the original ARM64
producer/head, with archive digest
`5a07d859b562ed4b8451320229ceb45af4cc6c1e41a87d0cc466fca36e2122c0`.
The authorized connector returned a file reference, but downloading its URL from
this VM returned HTTP 403. Original dumps were not read or revalidated, and the
synthetic replay must not be substituted for them. Fresh, exact-source ARM64
Native evidence and review of actual remaining outcomes are still required.

## Preservation and proposed next action

[README.md](README.md) and [sources.json](sources.json) record all 119 exact heads,
their trees/base identities, dependency groups, contained PR heads, changed paths,
same-as-main blobs and feature/evidence obligations. Every head has a local
`archive/20261015-prN` ref. The verified complete-history cloud bundle is
`/workspace/tpe-integration-evidence/open-pr-sources-20261015.bundle`, SHA-256
`ac4494d1762d90667d9fc8ef306c01c446a2dbfefa1aeace181ad23dc8573085`.

The index keeps 54→55 and 251→263 as competing intake designs, bars wholesale
imports of the replaced 84/92 pipeline, preserves 64 pending Ada's coordination,
and records Ada's prior closures of 155/156/161/170/240. Main already contains many
PR173 ports; ancestry alone is not a supersession decision. PR101's LICENSE
deletion is explicitly excluded. No new queue/database architecture is introduced.

Proposed eventual update: fast-forward the existing PR273 source branch to the
accepted checkpoint after Ada reviews it and the outstanding exact-source gates
are resolved. The ancestry permits this without rewriting history. PR280/281 are
fully preserved as parents, but remain open until Ada coordinates any closure.
The remaining PRs require source/evidence review using the index before closure.

Raw local logs and the bounded replay are preserved under
`/workspace/tpe-integration-evidence/`; validation-summary.json records hashes of
the principal test logs. No claim of full Native success, security clearance or
extraction accuracy follows from these passing checks.
