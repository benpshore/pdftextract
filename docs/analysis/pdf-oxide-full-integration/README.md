# Pinned PDF Oxide integration inventory

This is a source inventory and verification plan, not a completion report. It
records **84 capability obligations** across the full pinned surface, rather than
equating text extraction with integration. Every capability initially has empty
implementation/test evidence. The integration owner must attach exact commit,
feature, fixture and command evidence before changing that state.

The separate [integration-status.json](integration-status.json) maps the current
adapters and explicit gaps back to all 84 obligations without rewriting the
original inventory. It records 66 partial adapters, six explicit unsupported
capabilities, eleven source-only obligations and one parent-owned Python parity
obligation. No row is qualified; registry names and passing tests do not establish
complete surface coverage or fidelity.

The source index covers all 467 Rust files shipped under the two crates' `src/`
directories. It records 9,678 lexical public declarations, public fields and
public-trait members, together with adjacent attributes and source locations.
This intentionally includes conditional bindings, support binaries and items in
private modules that may be reexported. It is not compiler-expanded rustdoc;
macro-generated items and effective visibility are not inferred. Counts describe
the index, not supported user operations or completed tests.

| Pinned package | Root public modules | Rust files | Indexed declarations | Features |
| --- | ---: | ---: | ---: | ---: |
| `pdf_oxide 0.3.78` | 50 | 374 | 8,063 | 27 |
| `office_oxide 0.1.9` | 14 | 93 | 1,615 | 5 |

Both packages declare `MIT OR Apache-2.0` and Rust 1.88. The cached registry
archives match their existing Cargo.lock checksums:

| Package | Archive SHA-256 | Registry VCS commit |
| --- | --- | --- |
| PDF Oxide | `95b1ecc1a60a314fbd1aea5fe8f700ec32af1efa70fef5c6879162444da3437d` | `ad49c4cb3638dc950a29350ef882161372fde473` |
| Office Oxide | `e07e156a6bc62caaa66fc698daad7405dd62e86f5292fd22898c1a55f154748d` | `1b8c6438905814169616c7b4f5a0f67b55b1d3fa` |

The selected-source aggregate hashes are respectively
`223fb9ee47a16ffd40d101e045670b8ea0b3d5463a15481bf9ee855ce3482ac3` and
`3e26d4fafea0ea2e2ba8663120a1c1b7957bb288314540d9cc25a566430ceaa9`.
The exact hash recipe, every source-file hash, manifests, features and dependency
declarations are in [source-inventory.json](source-inventory.json). This does not
assert a signed release attestation. Upstream copyright, licenses and NOTICE
must remain with the dependency notices; this project’s license stays unchanged.

[capability-matrix.json](capability-matrix.json) contains source references and
API declarations for each obligation. [capability-spec.json](capability-spec.json)
is the reviewable source of that matrix. It covers intake, extraction, reading
order, structured regions/tables/paths, search, metadata, cleanup, images,
conversion, creation, editing, forms, annotations/attachments, encryption,
redaction/sanitization, signatures, compliance, rendering/color, OCR/ML, Office,
batch/caches, C/Python/WASM bindings, and the separately packaged CLI/MCP surface.

Implementation ownership is divided by operation group. The root owner handles
the existing engine contracts, supervised worker, extraction and combined tests.
The creation/Office worker handles creation and conversion; editing/forms handles
document mutation, forms, annotations and metadata queries; rendering/OCR handles
optional image generation/runtime paths; security handles encryption, redaction,
sanitization, compliance and signatures. Query operations are assigned separately
to the query worker. Python work remains parent-coordinated, using frozen fixture
identities and distinct evidence; this branch does not write that implementation.

[feature-build-matrix.json](feature-build-matrix.json) proposes 19 build/test
profiles and five platform rows. No profile is marked passed by this inventory
worker. The existing `--features pdf-oxide` metadata snapshot enables no upstream
PDF/Office features and its dependency closure’s maximum declared Rust version is
1.88.0. That is metadata evidence only: the minimum toolchain and optional feature
graphs need actual builds. In particular, `fips` and `legacy-crypto` deliberately
conflict, so `--all-features` is not a positive acceptance configuration.

Source findings that affect implementation:

- `PdfDocument::open_with_config` ignores its argument. Public `ParserOptions`
  limit-looking fields are not evidence of effective document resource limits.
- The pinned `PdfImage` has `to_png_bytes`, `save_as_png` and `save_as_jpeg`, but
  no `to_jpeg_bytes`. Owned image vectors and inline decoding need worker limits
  even when no disk IO is requested. Empty extraction can conceal upstream failure.
- Header/footer removal mutates document-local masks and scans all pages. Keep
  original and cleaned instances separate. Tagged early returns, repeated prose,
  table headers and citations need false-positive fixtures. Masks are not redaction.
- HTML image paths are interpolated into quoted attributes by one upstream path;
  application-owned wrappers/paths and sanitization remain required. The HTML/CSS
  creation callback accepts data URIs and can silently omit undecodable images.
- `Pdf::to_bytes` and rendering `flatten_to_images` use PID-named temporary paths.
  `Pdf::into_bytes` can hide serialization failure. Prefer verified fallible byte
  APIs and existing atomic artifact publication.
- Office ZIP reads preallocate declared size and use `read_to_end`; nonempty CRC
  mismatch data can be accepted. XML helpers disable end-name/comment checks.
  Adapter preflight must supply ZIP/XML/decompression budgets and strict rejection
  of unsupported active/external content. External relationship resolution rejects
  targets but hyperlink data may remain. This is not a full Office security audit.
- Model prefetch (`ocr`) and timestamp acquisition (`tsa-client`) contain network
  clients with whole-body reads. Keep them disabled; no credentials or model
  download is authorized. Rendering can read host fonts and Office can use fixed
  font fallbacks, so font provenance and confinement need separate evidence.
- Upstream parallel/batch helpers collect whole-document/corpus vectors. Retain
  the existing bounded queue, backpressure and per-file failures. The optional
  Office parsing thread’s 16 MiB stack also counts toward process resources.
- The library’s CLI and MCP companions are not included in these crate archives.
  Their installation, persistent registration, filesystem grants and write-capable
  tools remain a separate pinned-source review; no `@latest` command is part of this
  work.

The generator ran successfully under Python 3.14.7 with a 1 GiB address-space cap
and 120-second deadline, validating archive hashes, source references and named
APIs. That is inventory validation, not Python engine parity or a PDF test result.

[containment.md](containment.md) documents the worker policy, its fail-closed
Landlock path, the distinct sealed seccomp path and bounded syscall probes.
[containment-evidence.json](containment-evidence.json) binds those local probe
results to exact source hashes. This separate evidence does not mark upstream
capability obligations complete; the integration owner attaches combined results.

To reproduce, use the existing locked metadata snapshot and extracted registry
sources; the script does not download or compile anything:

```sh
uv run --no-project --offline python \
  docs/analysis/pdf-oxide-full-integration/build_inventory.py \
  --pdf-root /path/to/pdf_oxide-0.3.78 \
  --office-root /path/to/office_oxide-0.1.9 \
  --cargo-home /path/to/cargo-home \
  --lock Cargo.lock \
  --metadata /path/to/cargo-metadata.json
```

Exact version source takes precedence over current website examples. The source
index is from the registry archive; upstream entry points include
[PDF Oxide’s tagged source](https://github.com/yfedoseev/pdf_oxide/tree/v0.3.78)
and [Office Oxide’s recorded source commit](https://github.com/yfedoseev/office_oxide/tree/1b8c6438905814169616c7b4f5a0f67b55b1d3fa).
