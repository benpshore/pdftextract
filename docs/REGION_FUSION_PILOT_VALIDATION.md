# Reviewed region fusion pilot — 2026-10-04

Tracking: [#196](https://github.com/benpshore/pdftextract/issues/196), engine epic
[#181](https://github.com/benpshore/pdftextract/issues/181), service epic
[#182](https://github.com/benpshore/pdftextract/issues/182). This increment is
stacked on frozen [PR202](https://github.com/benpshore/pdftextract/pull/202),
`fb623bec2c8c68341a45e1f220ff0f46245dc039`. It adds a separate experiment and
dedicated qualification workflow; it does not modify PR202 or production files.

## Demonstrated behavior

A real lopdf pass runs first and durably saves its original JSON before the
explicitly requested PDFium pass begins. Actual extraction of the frozen positive
fixture produces a correct neighboring baseline span and an unresolved target
of 18 replacement characters. The PDFium candidate supplies `RECOVER ALPHA 2026`.
A bounded geometric scan finds the unique pair inside the reviewed region and
selects the candidate's existing text for that span alone. The other span retains
its baseline text and identity. The complete original source, baseline and
candidate artifacts remain available with exact hashes.

This is a **source/render-review-assisted selection pilot**. An externally
accepted review supplies source-bound transcription and frame evidence. The
expected review digest is an explicit caller policy input, never computed from
an arbitrary received file as self-approval. The fixture truth was declared by
the deterministic generator before extraction and corroborated by a separate
MuPDF render; visual reviewers were AI agents shown the expected strings. This
demonstration is not blind annotation, held-out accuracy, automatic semantic
adjudication or general extraction recovery.

The output is a separate span projection, not a rewritten schema-5 extraction.
Original `PageText.text`, geometry, lines, metadata, warnings and Partial status
are retained in the unchanged evidence. The nested foundation
`evidence.decision` continues to describe that contract's baseline-retaining
artifact record; the new outer `regions` and `pages` describe the independently
reviewed selection. Neither field promotes the original extraction status.
The runner exits 1 if either completed extraction is Partial, even after the
reviewed target is recovered.

## Source, native runtime and toolchain

The production extractor used locally was built from unchanged main
`e27e1fb28a5b40dbca7f517c6fe396e6f11ed4ec`, with its lockfile and PDFium feature.
It reports `tpe git-e27e1fb`; binary SHA-256 is
`2674dbc4712d65b7f41ddee341086fb3ee1be8a5497fd4c5b18c07272b5ec498`.
This does not qualify PR175's different native/provider dependency graph.

The existing Linux x64 runtime is `chromium/8066`, library SHA-256
`7670b3c597b02dfa3f98b23b49c3bb52536312f1ea686b739321731b6011f5a9`, distributed
in an archive with SHA-256
`0b43f405477cf2cfc4dbff06905093c3309756c6bca1fb9da99234a2ca97fed2`.
The Rust wrapper remains 0.8.37; its selected `pdfium_latest` API feature resolves
to 7543. These are distinct pins. The workflow verifies the existing manifest's
archive and library hashes before building its test executable. No dependency,
runtime, Rust or Clippy pin is upgraded. Read the
[compatibility audit](PDFIUM_COMPATIBILITY_AUDIT.md) for upstream/distributor
separation and the outstanding platform/ABI regression matrix.

Local checks use rustc and Cargo 1.98.1, Clippy 0.1.98, rustfmt 1.9.0 and uv-managed
Python 3.14.7. Exact compiler commit identities and earlier unchanged production
checks are in [foundation validation](REGION_FOUNDATION_VALIDATION.md).

## Reproducible checks

```sh
cargo fmt --manifest-path experiments/region-fusion/Cargo.toml --check
cargo clippy --locked --manifest-path experiments/region-fusion/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path experiments/region-fusion/Cargo.toml
uv run --locked ruff format --check
uv run --locked ruff check
uv run --locked pytest
uv run --locked pytest experiments/region-fusion/test_runner.py

# Required explicit local executables and existing pinned native runtime:
REGION_TPE_BIN=/absolute/path/to/tpe \
REGION_FUSION_BIN=/absolute/path/to/fuse_artifacts \
PDFIUM_DYNAMIC_LIB_PATH=/absolute/path/to/libpdfium.so \
uv run --locked pytest experiments/region-fusion/test_real_extraction.py
```

The real-engine suite fails when its executable/runtime prerequisites are
missing; it does not silently skip them. Its six tests exercise actual fresh
lopdf/PDFium recovery, baseline-only opt-in behavior, unapproved review rejection,
a deliberately injected candidate-controller crash after real baseline
extraction, and longer-wrong/duplicate-geometry mutations of real candidate
artifacts. The injected crash is not a demonstrated crash inside PDFium.

The runner's 22 separate fault tests launch real Linux processes with fake tools
to test process supervision, deadlines, output limits, failed or missing results,
source preservation, separate-session descendant cleanup and cancellation during
final journal fsync. Those tests measure runner behavior, not native correctness.
Rust tests cover the retained captured engine artifacts, unique geometric
correspondence, trust/source/page/render rejection, ambiguous/crossing geometry,
competing attempts, longer/equal text, all declared selector budgets and rollback
after a proposal. The CLI also has six bounded I/O and no-clobber tests.

Local frozen-source validation passed: 16 selector tests, six CLI tests, 22 runner
tests and six real-engine integration tests (50 new passing outcomes); strict
Clippy, Rust formatting and repository-wide Ruff also passed. The unchanged
Python baseline passed all 92 tests. The prior production Rust baseline remains
the 809 passing outcomes recorded in foundation validation; it was not represented
as a new production release build. Swift/CMake are unavailable in this Linux
executor, and the local OSV connection remains a separate audit limitation.

The dedicated [pilot workflow](../.github/workflows/region-fusion.yml) repeats
the standalone Rust and runner checks, verifies the pinned runtime, builds
actual test executables from the reviewed tree, and runs the required real-engine
suite. Its debug executable build is test qualification, not the separate #201
production/release owner's build. Hosted run URLs and exact head SHA belong in
the draft PR's validation record after the frozen commit runs.

## Resource and cancellation semantics

The Linux runner has one shared monotonic deadline across source snapshot,
hashing, all passes and validation (default 60 seconds; caller range 1–300,000 ms).
Source copying is streamed; there is no new default PDF size ceiling. An optional
caller-selected source cap is checked while reading. Exact snapshot size is
passed to the existing extractor's byte limit. Review input is capped at 1 MiB,
render input at 32 MiB, and tool identity reads at 1 GiB. Diagnostic capture and
individual output files are bounded by the caller output cap, default 64 MiB.

The CLI additionally caps extraction artifacts at 16 MiB each and review/render
bytes at 1/32 MiB. The selector defaults to 128 reviews, eight candidate attempts,
64 MiB total candidate/render bytes, 100,000 span comparisons and 100,000 projected
spans. Library limits are caller-owned; records cannot raise them. Exceeding a
selection work budget or cooperative cancellation rolls back every proposed
selection. The original baseline artifact remains the fallback even when a
projection cannot fit its span budget.

The runner bounds controller address space, CPU and individual file size, uses
the existing native worker's growth limit, drains stdout/stderr incrementally,
and kills/reaps owned child processes. The Linux `prctl` call is typed explicitly;
this is still an FFI boundary. These controls are not a hostile-executable
sandbox, aggregate disk/RSS guarantee or hard wall-time guarantee for blocking
filesystem operations, fsync or uninterruptible kernel I/O.

Consumers must await runner termination and then require the final completed
journal and matching fused artifact hash. Live observation of a filename or
tentative completed journal is not an irrevocable commit: cancellation during
final persistence can demote it. SIGKILL cannot update the last durable receipt.
The source snapshot is protected against ordinary changes, not against a
malicious process with the same user privileges.

## Independent review and next gates

The [specialist implementation review](REGION_FUSION_IMPLEMENTATION_REVIEW.md)
challenged trust authority, provenance, ABI argument widths, ownership,
correspondence ambiguity, cancellation and publication semantics. Concrete
pre-publication issues were corrected; this review is separate from #200's
standardized security workflow and does not replace it.

Before production integration, obtain rendered ground truth that the selector
cannot read as selection input; test hidden/ActualText and misleading Unicode,
rotated/cropped pages and native frame transformations, multilingual and scanned
regions, repeated/overlapping content, split spans, large documents and actual
native worker failures. Establish a source-mapping or independently adjudicated
selection policy with calibrated evidence, then test multiple real engines and
resource/cancellation behavior on each supported platform. No confidence numbers
or general accuracy claim are supplied by this experiment.

CPU OCR/GROBID work remains with #197/#198 under #199. PR175 release readiness,
PR192 web integration, #201 production builds and #200 security qualification
retain their separate owners. Native ARM/macOS, CMake Objective-C++/Foundation,
Docling/model and JS/WASM runtime qualification remain outside this Linux pilot.
No merge, release, Site deployment, personal-Mac access, credentials or live
security settings were requested or changed.
