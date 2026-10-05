# Upstream integration and update policy

Research checked **2026-09-28**. These are upstream findings, not results measured in this repository. Recheck the selected revisions during implementation. Research snapshot SHAs are not validated release pins.

## Components

| Component | Primary sources | Integration consequence |
| --- | --- | --- |
| Poppler | [Project](https://poppler.freedesktop.org/), [source](https://gitlab.freedesktop.org/poppler/poppler), [text/layout API](https://poppler.freedesktop.org/api/cpp/classpoppler_1_1page.html) | Benchmark native utilities first; evaluate a small C++ adapter for sustained document sessions. Inspect the exact GPL license and notices before shipping. |
| PDFium | [Source/build](https://pdfium.googlesource.com/pdfium/+/HEAD/README.md), [text API](https://pdfium.googlesource.com/pdfium/+/main/public/fpdf_text.h), [license](https://pdfium.googlesource.com/pdfium/+/HEAD/LICENSE) | Own a pinned native build recipe. Start with V8/XFA disabled unless fixtures require them. |
| PDFium binding | [pdfium-render](https://github.com/ajrcarey/pdfium-render), [documentation](https://docs.rs/crate/pdfium-render/latest/source/README.md) | Pin the crate, explicit API feature, and native binary together. The wrapper does not supply/build PDFium; its mutex does not provide parallel PDFium throughput. |
| Bootstrap binaries | [PDFium binary publisher/build recipes](https://github.com/bblanchon/pdfium-binaries) | macOS/Linux arm64 artifacts are a possible initial baseline. Identify them as third-party builds, verify hashes/provenance, and measure source-build feasibility. |
| Docling Rust | [Official project](https://github.com/docling-project/docling.rs), [PDF conformance](https://github.com/docling-project/docling.rs/blob/master/docs/PDF_CONFORMANCE.md), [migration](https://github.com/docling-project/docling.rs/blob/master/docs/MIGRATION.md) | Use this native implementation rather than a similarly named HTTP client. Pin PDFium, ONNX Runtime, weights, dictionaries, preprocessing, and provider settings. |
| MLX | [MLX](https://github.com/ml-explore/mlx), [build support](https://ml-explore.github.io/mlx/build/html/install.html), [official C API](https://github.com/ml-explore/mlx-c), [community Rust binding](https://github.com/oxiglade/mlx-rs) | Test an existing Rust binding or narrow C boundary with one model before building a broad custom binding layer. |
| GPUI | [Framework](https://github.com/zed-industries/zed/tree/main/crates/gpui), [manifest/license](https://github.com/zed-industries/zed/blob/main/crates/gpui/Cargo.toml) | Later GUI-only dependency; pre-1.0 API changes require pinning. |
| Runners | [GitHub reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) | Initial candidates: `macos-15` and `ubuntu-24.04-arm`, subject to SDK/runtime requirements. Record image/architecture and probe device support. |

## Constraints that change the design

**PDFium:** exposes Unicode, geometry, and mapping-error information. Some text APIs omit or cannot map characters; test non-BMP symbols and crop-box visibility explicitly. Preserve diagnostics. `pdfium-render` serializes native calls within a process, so bounded process-level concurrency is the baseline. Any alternative must establish the exact library's guarantees.

**Poppler:** positioned text and layout modes are useful comparisons, but its C++ documentation does not promise correct ordering for all scripts. GPL obligations differ from the MIT application scaffold and PDFium's BSD-style principal license. A subprocess boundary is not an automatic licensing exemption. Review exact source, third-party notices, and packaging before distributing linked or bundled components.

**Docling Rust:** the inspected PDF pipeline defaults to Rust `textparse.rs` on `lopdf`; `DOCLING_PDFIUM_TEXT=1` selects PDFium text fallback. PDFium remains a renderer. Its PDF conformance page reports 9/17 strict and 10/17 whitespace-normalized matches against Python Docling fixtures. These are upstream parity results, not independently checked human truth or this project's accuracy. General all-format compatibility claims must not become perfect-PDF claims.

The [CoreML section](https://github.com/docling-project/docling.rs/blob/master/README.md#gpu-execution-providers-optional-off-by-default) records incorrect boxes for some operations, numerical issues, and session startup costs. Its cited hardware measurements used M4 Max. Establish a CPU reference; measure M1 provider correctness, precision, setup, batches, and total throughput independently. Record actual provider selection and fallback. CoreML, Metal, and MLX are not interchangeable runtimes.

Docling Rust already exposes page selection and a streaming conversion callback. However, the inspected [PDF backend](https://github.com/docling-project/docling.rs/blob/908f080d5def736bbc094beeaf5b4a04fe504dd3/crates/docling-pdf/src/pdfium_backend.rs) opens parser state for each conversion: keeping model sessions warm does not keep the same document open across separate page-window conversions. Benchmark one streaming conversion per large document against repeated window calls. Preserve callback backpressure; treat resumable parser sessions and yielding a live document between jobs as capabilities to verify or implement, not existing guarantees.

The inspected [pipeline API](https://github.com/docling-project/docling.rs/blob/908f080d5def736bbc094beeaf5b4a04fe504dd3/crates/docling-pdf/src/lib.rs) also has `process_pages(Vec<PdfPage>)` for supplied page cells/images. Verify page-number preservation, outline/heading context, and coordinate compatibility before using it to avoid duplicate extraction. Streaming and buffered paths differ in confidence/heading handling at this revision; compare the requested structural fields, not just concatenated text, before treating streaming as equivalent.

**MLX:** current upstream supports Apple Metal and Linux CPU/CUDA configurations. This does not establish our Linux aarch64 binding/model packaging. ONNX models need a tested implementation/conversion for MLX, including operators and preprocessing; a Cargo feature cannot supply that conversion.

Reviewed research snapshots: Docling Rust `908f080d5def736bbc094beeaf5b4a04fe504dd3`; MLX `a32f0c3d46ecd4864cd7eae44552ed160d480dfb`; Zed/GPUI `4f70d91bda7ec5f5600a000a0dc34391d9f1e96f`. These identify reviewed state only. Reproduce claims at the selected implementation pins.

## Planned artifact lock

Add a small machine-readable native/model manifest alongside Cargo.lock. Record source URL/immutable revision, artifact URL/SHA256, target and deployment/glibc baseline, build-recipe digest, compiler/SDK/features, compatible binding/API, and license/notice locations. Models also need weight/dictionary/tokenizer hashes, preprocessing, precision, runtime/provider, and conversion-script identity.

Provision dependencies explicitly. Accepted artifacts must support offline processing; absent dependencies fail clearly. Contain upstream first-use download behavior so ordinary jobs cannot fetch moving libraries or alternate weights.

## Planned candidate-update workflow

1. Check releases and relevant heads on a daily schedule, plus manual trigger. Record a cursor; unchanged upstreams do not rebuild. Urgent candidates can be requested on demand.
2. Build changed candidates outside the release lane. Keep one update PR per coupled dependency group rather than per upstream commit. Measure PDFium's Chromium-tooling disk/time requirements before assuming a small runner can rebuild it.
3. Cache by full source/target/compiler/SDK/flags/recipe/dependency identity. Store accepted artifacts durably with provenance: disposable CI caches are not the release source of truth.
4. Run both native target checks, fixed-corpus quality comparisons, resource limits, model parity, and relevant large-document regression cases. Keep physical M1 performance evidence distinct from hosted timing noise.
5. Attach resolved pins, upstream changes, licenses, install results, quality diffs, and cost measurements to the PR. Initially no automatic merge. Failed candidates leave the accepted set usable.
6. Adoption creates a new processing identity; old results retain their original provenance. Rollback selects the previous manifest without overwriting evidence or rewriting Git history.

Prefer unmodified upstream releases. Keep required patches small, explicit, and covered by fixtures, with upstream issue/PR references. If a fork is needed, separate its tracking branch from accepted application pins. Do not continuously rebase whole upstream source trees into the application.

These workflows are planned, not active yet. GitHub builds use public/synthetic fixtures. Bulk extraction runs on explicitly selected machines; do not make a personal Mac a public PR runner or upload private corpora as a side effect of implementation.

## Crate dependency review (2026-10-05)

Produced with `sh scripts/deps-report.sh --online` at commit `1bc92cc`
(toolchain 1.98.1, host `x86_64-unknown-linux-gnu`, default features;
`Cargo.lock` SHA-256 `c903547b…b89f365`). These are proposals: nothing below
was applied, `cargo update` was not run and no manifest or lockfile changed.
Re-run the report before acting; crates.io moves.

### Outdated direct dependencies (proposals)

| crate | declared | locked | newest | proposal |
| --- | --- | --- | --- | --- |
| `sha2` (tpe-search) | `0.10` | 0.10.9 | 0.11.0 | Align with the root crate's `sha2 = "0.11"`: one `sha2` removes the `digest`, `block-buffer`, `crypto-common`, `cpufeatures` and `rand_core` duplicates listed below. Own PR for `crates/tpe-search`; verify FTS/embedding hashes in its tests. |
| `signal-hook` (root) | `0.3` | 0.3.18 | 0.4.5 | Major bump touching the worker signal path; check the 0.4 API against `src/` signal handling in a dedicated PR with the worker-startup and CLI-containment tests. |
| `pdfium-render` (optional `pdfium`) | `0.8` | not resolved on Linux by default | 0.9.4 | Keep 0.8 while `docling-pdf 1.69.2` pins the same major, so one `pdfium-render` links libpdfium (root `Cargo.toml` comment). Moves together with the docling pins. |
| `docling-core`, `docling-pdf` (optional) | `=1.69.2` | not resolved by default | 1.96.1 | Deliberate exact pins; a bump is a native-backend candidate update (PDFium, models, ONNX Runtime together), not a lockfile refresh. |
| `office_oxide` (optional) | `=0.1.9` | not resolved by default | 0.1.13 | Held back on purpose: `pdf_oxide 0.3.78` breaks against 0.1.13 (root `Cargo.toml` comment). |
| `roxmltree` (optional `grobid`) | `=0.20.0` | not resolved by default | 0.21.1 | Exact pin; revalidate the GROBID TEI tests before moving. |
| `libloading` (tpe-ffi `provider`) | `0.8.9` | not resolved by default | 0.9.0 | `pdfium-render` already pulls 0.9.0 under `pdfium`; moving `tpe-ffi` to 0.9 would leave one copy in native builds. |
| `libc` (tpe-ffi, macOS) | `0.2` | not on Linux | 1.0.0-alpha.5 | Prerelease; stay on 0.2. |
| `ruff` (uv dev group) | `>=0.14` | 0.16.9 | 0.16.10 | `uv lock --upgrade-package ruff` in a Python-tooling PR. |

Every other direct dependency in the default Linux scope (anyhow, argon2,
chacha20poly1305, clap, flate2, futures, hex, lopdf, regex, rusqlite, rustix,
serde, serde_json, tar, tempfile, thiserror, unicode-normalization, ureq,
zeroize) is at its newest crates.io release.

### Duplicate crate versions (default Linux scope)

- `sha2` 0.10.9 (tpe-search) and 0.11.0 (root, lopdf): the only duplicate a
  workspace manifest controls; it drags `digest` 0.10.7/0.11.3, `block-buffer`
  0.10.4/0.12.1, `crypto-common` 0.1.7/0.2.2, `cpufeatures` 0.2.17/0.3.1 and
  `rand_core` 0.6.4/0.10.1.
- `getrandom` 0.2.17 (ring 0.17, rand_core 0.6) and 0.4.3 (lopdf, rand 0.10,
  tempfile, crypto-common 0.2): upstream; resolves when ring moves to
  getrandom 0.4.
- `syn` 2.0.119 (zerocopy-derive, zeroize_derive) and 3.0.6 (clap_derive,
  serde_derive, thiserror-impl, futures-macro): proc-macro only, build time.

### Licenses and native code (default Linux scope, 179 crates outside the workspace)

All declared licenses are permissive: MIT and/or Apache-2.0 alone for 156 crates,
plus BSD-3-Clause (3), ISC, Zlib, Unlicense, 0BSD, BSL-1.0, Unicode-3.0 and
CDLA-Permissive-2.0 variants. No GPL, LGPL, AGPL or undeclared license in the
default graph. The GPL/AGPL engines (Poppler, MuPDF) are never resolved by
Cargo: they are user-built providers (`native/README.md`). Crates compiling
native code by default: `cc` for `libsqlite3-sys` 0.38.2 (bundled SQLite) and
`ring` 0.17.14; `pkg-config`/`vcpkg` are probed, not required. Optional
features add `cxx` (usearch), `aws-lc-sys` with `cmake` fallback (reqwest via
liteparse, fastembed), `ort-sys` (ONNX Runtime download), `bindgen`/libclang
and `freetype-sys` (GPUI, macOS), `cef-dll-sys` (tpe-browser) and `zstd-sys`
(kokoro-tts).

### Python audit

`uv audit --preview-features audit-command` needs a uv release with the
`audit` subcommand (CI installs the latest uv through `astral-sh/setup-uv`).
On the 2026-10-05 review host, uv 0.8.17 has no such subcommand and a current
uv run through `uvx --from 'uv>=0.9' uv audit --preview-features audit-command`
(uv 0.12.23) could not reach `api.osv.dev` through that host's egress, so the
result there is "not run"; the required `ci` check's python job is the
authoritative audit run. The uv project has two dev dependencies (pytest 9.1.1,
ruff 0.16.9) and no runtime dependency.
