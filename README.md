# PDFTextract

PDFTextract extracts text and bibliographic evidence from academic PDFs. Its
Rust command-line engine, `tpe`, records source hashes, backend identity,
positioned text, inferred reading order, metadata, references and citation
markers. The repository also contains a separate browser extraction alpha and
a macOS application prototype.

**Early development:** extraction can be incomplete or wrong. `Complete` means
no supported failure or uncertainty was detected; it does not certify every
character, reading order or reference field. `Partial` results and warnings need
review against the original. Historical accuracy and timing reports do not
establish correctness or throughput for current releases or arbitrary PDFs.

## Install and try the CLI

[GitHub releases](https://github.com/benpshore/pdftextract/releases/latest)
provide `tpe` binaries for Apple Silicon macOS, Linux ARM64 and Linux x86-64,
with `SHA256SUMS`. These binaries use the pure-Rust `lopdf` backend; optional
native libraries and OCR models are not bundled.

With [mise](https://mise.jdx.dev):

```sh
mise use -g github:benpshore/pdftextract
tpe --version
tpe backends
```

For a manual install, download the matching release archive, verify it against
`SHA256SUMS`, and extract `tpe` to a directory on your `PATH`. To select a
specific release with mise, append `@VERSION` to the repository specifier.

```sh
tpe extract paper.pdf --db corpus.sqlite --out out/
tpe bibliography paper.pdf > bibliographies.jsonl
tpe stats --db corpus.sqlite
tpe --help
```

`extract` stores a run in SQLite and optionally exports structured JSON and
page text. `bibliography` searches backward for the final qualifying reference
list and emits one JSON record per PDF without requiring a ledger. It retains
raw entries alongside best-effort parsed fields. Its default `auto` route uses
the backends available in the build; `--resolve` explicitly enables Crossref
and Europe PMC lookups. Registry matches remain heuristic, not proof of an
exact citation. See [bibliography behavior and limits](docs/BIBLIOGRAPHY.md).

## Engine capabilities and boundaries

- `extract` runs disposable workers with document deadlines, memory-growth
  limits and one to four worker processes. Publication is serialized. Other
  commands and direct library calls do not inherit that worker boundary; it is
  not a complete security sandbox. See [CLI workers](docs/CLI-WORKERS.md) and
  [publication and recovery](docs/PUBLICATION.md).
- Optional builds provide PDFium, PDF Oxide, Docling text and LiteParse layout
  adapters. MuPDF and Poppler require separately built, explicitly configured
  provider libraries. Availability depends on build features and runtime
  artifacts; `tpe backends` reports what the installed binary can open.
- `auto` can select another whole backend pass while retaining usable native
  evidence when a candidate fails or regresses. MuPDF/Poppler fallback is an
  explicit opt-in. It does **not** fuse pages or regions across engines. See
  [native fallback](docs/NATIVE_FALLBACK.md) and the current
  [routing review](docs/EXTRACTION_ROUTING_REVIEW.md).
- Full Docling layout/OCR exists as an optional adapter for separately
  provisioned library/evaluation use. It is not admitted by supervised
  `extract`; tables are disabled in its current configuration. Docling text
  performs no OCR. Scans and unresolved character mappings can remain Partial.
  See [Docling](docs/DOCLING.md) and [native provisioning](docs/NATIVE.md).
- An optional, separately selected [GROBID client](docs/GROBID.md) retains TEI
  evidence from an explicitly configured server. It is not part of automatic
  extraction or the standard release binary.

## Browser alpha, app prototype and plans

[`web/`](web/README.md) contains the private browser alpha's application source:
PDF Oxide WASM extraction, HTML/feed capture, Office and archive import,
English still-image OCR, saved originals/results and read-only MCP access.
It runs separately from the native `tpe` engine. Complete scanned-PDF OCR,
audio/video transcription and large-batch capacity are not established. Source
in this repository is not proof of what is deployed; see the
[source and deployment boundary](docs/WEB_ALPHA.md).

The [macOS GPUI prototype](docs/APP.md) offers text and bibliography jobs plus
Finder integration. Build and unit-test evidence does not establish everyday
Mac usability or accessibility. The broader document workbench, embedded
research browser, page/region fusion and MLX acceleration remain plans or
unmerged prototypes.

Open draft PRs are not release features. In particular,
[#234](https://github.com/benpshore/pdftextract/pull/234) stages native/web
hardening and compatibility repairs; its checks and review apply to that
candidate, not the released engine or deployed Site.

## Development and evidence

Follow [AGENTS.md](AGENTS.md). The [Rust build contract](docs/RUST_BUILD.md)
documents the pinned toolchain, locked dependencies and release identity.
Python/uv tools support tests and evaluation; Python is not required to run
the native release binary. Web setup and checks live in [web/README.md](web/README.md).

See [evaluation](docs/EVAL.md), [dated analysis reports](docs/analysis/) and
the [engine repair record](docs/ENGINE_REPAIR_2026-10-03.md) for measurements
and their source identities. Historical reports retain their original scope;
they are not acceptance evidence for later code. M1 latency, sustained daily
throughput and error-free chunk targets remain unqualified.

Changes go through pull requests. Release automation publishes a new version
after a qualifying merge to `main`. The project is proprietary; see
[LICENSE](LICENSE). Upstream native-library and model licenses also apply.
