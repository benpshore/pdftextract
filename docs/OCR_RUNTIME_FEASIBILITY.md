# OCR runtime and acceleration qualification

Reviewed 2026-10-04 against TPE source `baafb472873750e7e84d32c3c8e6fa7a680e37db`,
the PR #175 integration baseline. This work supports [OCR #197](https://github.com/benpshore/pdftextract/issues/197)
and the [dedicated OCR epic #199](https://github.com/benpshore/pdftextract/issues/199),
linked to [#180](https://github.com/benpshore/pdftextract/issues/180) and deployment
[#182](https://github.com/benpshore/pdftextract/issues/182). This is source and
local VM evidence; it does not qualify an iPhone, a browser deployment, or a
production service. Machine-readable artifact identities and observations are
in [docling-runtime-audit.json](validation/docling-runtime-audit.json).

## Exact upstream and capability boundary

TPE selects `docling-pdf = "=1.69.2"` with `default-features = false`, and
`docling-core = "=1.69.2"`. Downloaded crates matched the SHA-256 checksums in
`Cargo.lock`. The published PDF crate's `.cargo_vcs_info.json` identifies
`docling-project/docling.rs` commit
`908f080d5def736bbc094beeaf5b4a04fe504dd3`. This is the Rust implementation;
Python Docling releases and newer Rust documentation do not establish this
version's behavior. [Published crate](https://docs.rs/crate/docling-pdf/1.69.2/source/),
[exact upstream manifest](https://github.com/docling-project/docling.rs/blob/908f080d5def736bbc094beeaf5b4a04fe504dd3/crates/docling-pdf/Cargo.toml).

| Path | Actual work | Qualification boundary |
| --- | --- | --- |
| TPE `docling-text` | Pure Rust PDF content-stream/text-layer parser; retained page parser and native evidence reconciliation. | No raster recognition, model, PDFium or ONNX dependency. Empty scans do not become OCR successes. |
| TPE optional `docling` | Enables upstream `ml`: PDFium rasterization, layout model, PP-OCR recognition and optional structural models. TPE explicitly chooses PP-OCR and defaults tables off. | The supervised CLI blocks this backend pending an explicit model/runtime contract; library conversion alone does not prove completeness. |
| Upstream native PP-OCR | PP-OCRv3 English `ocr_rec_en.onnx` + `en_dict.txt` or multilingual Chinese pair; line preparation and CTC decoding; optional PP-OCRv6 text detector. | Native CPU inference is real OCR. Recognition/model/language provenance must stay distinct from embedded PDF text. |
| Upstream `docling-wasm` | Separate browser crate shares Rust OCR preparation, layout/table postprocessing; JavaScript ONNX Runtime Web runs inference. `ocr_image`, `ScannedConverter` and `DigitalConverter` are separate from text-only `convert`. | Not part of the current TPE browser integration. A Rust text parser's WASM build does not include the native `ml` stack. |

Sources: [OCR implementation](https://github.com/docling-project/docling.rs/blob/908f080d5def736bbc094beeaf5b4a04fe504dd3/crates/docling-pdf/src/ocr.rs),
[detector](https://github.com/docling-project/docling.rs/blob/908f080d5def736bbc094beeaf5b4a04fe504dd3/crates/docling-pdf/src/ocr_det.rs),
[browser crate manifest](https://github.com/docling-project/docling.rs/blob/908f080d5def736bbc094beeaf5b4a04fe504dd3/crates/docling-wasm/Cargo.toml),
[browser API](https://github.com/docling-project/docling.rs/blob/908f080d5def736bbc094beeaf5b4a04fe504dd3/crates/docling-wasm/README.md).

Upstream asset resolution checks the current directory before configured model
fallbacks. Recognition load failure can warn and continue without OCR unless
full-page OCR was explicitly required. English model absence can select the
Chinese pair with weaker Latin spacing. Production preflight must validate
absolute per-file paths and hashes, explicitly select language/mode/CPU, and
fail an OCR-required job when its models are unavailable. Preserve the warning
and incomplete regions instead of inferring success from nonempty output.
[Pinned resolver and pipeline](https://github.com/docling-project/docling.rs/blob/908f080d5def736bbc094beeaf5b4a04fe504dd3/crates/docling-pdf/src/lib.rs),
[recognition selection](https://github.com/docling-project/docling.rs/blob/908f080d5def736bbc094beeaf5b4a04fe504dd3/crates/docling-pdf/src/ocr.rs).

## Pins versus observed runtimes

No Cargo or toolchain pin was changed by this audit. Version output verifies
installation; it does not by itself verify every optional feature combination.
Refer to the workstream's test report for actual build/test results.
The coordinated workstream passed locked workspace Clippy (`--all-targets`)
and tests with `--features docling-text,grobid`; that feature set does not
exercise Docling `ml` or a WASM target. See [CPU runtime handoff](CPU_RUNTIME_HANDOFF.md).

| Component | Selected identity | Evidence on this Linux VM |
| --- | --- | --- |
| rustup | Repository does not pin manager; provisioned `1.28.2` (`e4f3ad6f8`) | Version command observed; manager is distinct from compiler. |
| rustc / host / LLVM | `rust-toolchain.toml`: `1.98.1`; `x86_64-unknown-linux-gnu`; LLVM `22.1.8` | `rustc -Vv`: commit `48a229ceaefd4985c50990b14116b6d856af0985`. |
| Cargo | Bundled `1.98.1` (`797e8a9bc`) | `cargo -V` observed; use `--locked`. |
| Clippy / rustfmt | Toolchain components; Clippy `0.1.98` (`48a229ceae`) | Clippy version observed; no inference that all features passed lint. |
| Docling Rust | `docling-{pdf,core,onnx}` `1.69.2` | Published PDF/ONNX source hashes verified. PDF crate declares Rust `1.88`; this is not a measured whole-TPE MSRV. |
| Rust ONNX wrapper | `ort` / `ort-sys` `2.0.0-rc.13` | Published `ort-sys` checksum verified; declares Rust `1.88`. |
| ONNX native prebuilt selected by that wrapper | `ort-sys` distribution table names ONNX Runtime `1.28.0`, per-target URL and SHA-256 | Source-verified selection; actual TPE linked runtime still needs build/runtime identity. The upstream Docling CLI is a separate artifact. |
| PDFium | `chromium/8066` Linux x64 | Archive and extracted library downloaded and verified against `native/manifest.json`. |
| Native OCR models | Heron INT8 layout, PP-OCRv6 detector, PP-OCRv3 English recognizer/dictionary | All four downloads matched `native/manifest.json`; exact byte sizes/hashes in JSON. |
| Rust WASM toolchain | Root lock has `wasm-bindgen 0.2.129`, futures `0.4.79` | Only host target installed at audit time. No `wasm32` TPE build or matching wasm-bindgen CLI validation claimed. |
| Current browser OCR | `tesseract.js 7.0.0`, core `7.0.0`, English data `1.0.0` | Repository JS package/lock pins; generated same-origin asset manifest is a separate build artifact. |
| Current browser PDF engine | `pdf-oxide-wasm 0.3.77` in pnpm lock | Distinct published WASM artifact; not the native TPE optional `pdf_oxide 0.3.78`. |
| Prospective Docling browser OCR | `docling.rs-wasm` plus ONNX Runtime Web and model files | No exact integrated browser runtime/package lock or device qualification in TPE. Upstream demo imports an unversioned ORT CDN URL and mutable model URLs; do not copy those into a reproducible deployment. |

The `ort-sys` source table provides a stronger identity than the earlier
unresolved note in [NATIVE.md](NATIVE.md): Linux x64 CPU archive
`https://cdn.pyke.io/0/pyke:ort-rs/ms@1.28.0/x86_64-unknown-linux-gnu.tar.lzma2`,
SHA-256 `e454f710f8a49f53aa5b4ff51e3454ae1835777e431c6c35c5255ce6f205fd68`.
Its source revision is `002f41a8e175eac7f6695ff361d2e51a50874c48`.
An overridden library path still needs its own identity and compatibility test.
[Exact distribution table](https://github.com/pykeio/ort/blob/002f41a8e175eac7f6695ff361d2e51a50874c48/ort-sys/build/download/dist.tsv),
[upstream browser inference wiring](https://github.com/docling-project/docling.rs/blob/908f080d5def736bbc094beeaf5b4a04fe504dd3/crates/docling-wasm/www/pipeline.js).

The [upstream v1.69.2 release](https://github.com/docling-project/docling.rs/releases/tag/v1.69.2)
lists CLI and FFI prebuilts for Linux x64/arm64 and Windows x64; no macOS asset
was listed. The Linux x64 CLI archive downloaded with its GitHub-published
SHA-256 `2fc8c150ea31291eb7758c923953cff46537966006b3c5950169332f669eb70b`;
the extracted binary reports `docling-rs 1.69.2 (chunking)`. Its archive contains
the executable, not the model/PDFium bundle. `ldd` showed standard C/C++ runtime
dependencies and no separately loaded ONNX library; that observation does not
identify the embedded ONNX version. This audit does not substitute the CLI
binary for the Rust crate dependency.

The upstream [model notice at the same commit](https://github.com/docling-project/docling.rs/blob/908f080d5def736bbc094beeaf5b4a04fe504dd3/docs/MODELS_NOTICE.md)
attributes the Heron layout and PP-OCR model/dictionary files to Apache-2.0
sources. Retain their notices with redistributed artifacts. The repository's
manifest deliberately excludes TableFormer, multilingual recognition and
enrichment assets; do not silently enable those stages or mutable fallback
downloads.

## Local upstream OCR proof and reproduction

The verified precompiled CLI recognized the existing benign
`web/scripts/fixtures/ocr-still.png` (1500 × 340 pixels), with all three expected
lines exact: `AUTONOMOUS OCR TEST`, `Reference 10.1000/example`, and
`Local pixels stay on this device.` Both outputs have SHA-256
`86aa03b93a299672ccb5ec1cf5ebdfa73316b9205f916320dd8025bc8998b824`.
The [retained raw JSON](validation/docling-ocr-synthetic.json) contains page 1,
image dimensions and upstream region rectangles. Its reported origin MIME is
`text/plain` despite the PNG input; retain the independently hashed original
source identity. These image coordinates have not been qualified as PDF page
coordinates.

| Run | Wall time including process/model startup | Observed peak RSS |
| --- | --- | --- |
| First process | 1,478.751 ms | 324,808 KiB |
| Repeated fresh process | 1,426.387 ms | 315,124 KiB |

The second run benefits from already touched filesystem data; neither run is
a reused-session warm benchmark, and no disk-cache flush was performed. RSS
uses 10 ms `/proc` high-water sampling and can miss an exit-time peak. The CPU
window was coordinated with the other runtime/build agents. Both jobs had one
OCR session, one page worker, one inference thread, a 4 GiB address-space
ceiling, a 60-second CPU ceiling and a 65-second supervisor deadline. These
operational limits qualify a small probe; they are not document admission
limits. Original input hash was unchanged.

Reproduce on Linux x64 with Node 22+ and `tar`/`prlimit` installed:

```sh
node scripts/docling_cpu_probe.mjs \
  --cache /tmp/tpe-docling-cpu-assets \
  --output /tmp/tpe-docling-cpu-result.json
```

The script downloads only the explicit
[artifact pins](validation/docling-runtime-pins.json), verifies archive and
extracted hashes, uses absolute model/PDFium paths, creates a fresh working
directory and runs only the hashed benign fixture. It reserves new report/raw
output files exclusively; select a new output name for another run. Reports
retain failures, stderr, hashes, observed RSS and exact text checks. Cancellation
kills the process group or aborts the active download. No user document is
uploaded. This establishes available upstream CPU OCR; it does not remove the
TPE supervised model-path deployment gate or qualify scholarly/layout accuracy.
The maintained script was also executed against the verified artifact cache:
1,437.083 ms and 1,453.421 ms, all three parsed text fields exact, original hash
unchanged. The audit JSON retains this separate validation and the script hash.

## CPU, browser GPU, and native Neural Engine are separate paths

| Path | Compatible candidate | What must be established |
| --- | --- | --- |
| CPU native | Pinned native PP-OCR + PDFium + ONNX CPU, or separately identified local Tesseract | Actual scanned fixture recognition; source transforms; warning propagation; explicit resource controls and cancellation; release artifact identity. |
| CPU browser | Existing Tesseract.js/core with matching `eng.traineddata`; or a separately pinned Docling WASM + ORT Web PP-OCR path | Worker execution, exact asset integrity, scalar/SIMD fallback, raster transform, offline model reuse and cancel/restart on target browsers. |
| Safari WebGPU GPU | ONNX Runtime Web WebGPU provider with a separately qualified ONNX detector/recognizer | API/adapter creation, every model operator and shape, actual GPU placement, memory failure/device loss and CPU fallback; no assumed iPhone speedup. |
| Native CoreML GPU/ANE | Native ONNX Runtime CoreML provider or a CoreML-converted OCR model | Apple target binary, operator/shape support, actual compute placement, CPU-equivalent recognition and measured cold/warm latency. |

WebKit explicitly records WebGPU shipping in Safari 26.0. That exposes GPU
compute; it is not evidence of a Safari-to-Apple-Neural-Engine bridge. The
reviewed ONNX Runtime Web table still marks Safari WebGPU unsupported while
showing WASM CPU support. Treat that runtime/API difference as an unqualified
combination until the exact version and model run on the actual device.
[WebKit release evidence](https://webkit.org/blog/17640/webkit-features-for-safari-26-2/),
[ONNX Runtime browser matrix](https://onnxruntime.ai/docs/get-started/with-javascript/web.html).

Apple's native `MLComputeUnits` and ONNX Runtime's CoreML provider expose CPU,
GPU and Neural Engine choices. Selecting eligible compute units does not prove
that all operators run on ANE; provider partitioning and device profiling are
required. ORT documents native iOS/macOS packages and model-format/shape
constraints. [Apple API](https://developer.apple.com/documentation/coreml/mlcomputeunits),
[CoreML execution provider](https://onnxruntime.ai/docs/execution-providers/CoreML-ExecutionProvider.html).

There is a concrete reason to keep CPU first for this pin: `docling-onnx 1.69.2`
defaults CoreML to `MLProgram`, static-shaped partitions, and CPU+GPU. Its
source records M4 Max layout-model dynamic-shape aborts and ANE logit corruption.
These are upstream reports, not measurements repeated here, and are not claims
that all OCR models fail on ANE. The CPU-calibrated INT8 model path is also not
the upstream default for GPU providers. TPE currently exposes no CoreML Cargo
feature. Enabling it or changing models needs a coordinated dependency change
and correctness comparison, not an environment-only acceleration claim.
[Pinned provider source](https://github.com/docling-project/docling.rs/blob/908f080d5def736bbc094beeaf5b4a04fe504dd3/crates/docling-onnx/src/lib.rs).

## Browser execution requirements

Use a bounded dedicated worker queue, initially one active OCR page. A worker
moves work off the UI event loop; it does not itself add SIMD or shared-memory
threads. Preserve the original file, page/region identifiers, raster-to-source
transform, recognized text, model identity and uncertainty. Release tensors and
rasters between pages, persist completed results, and terminate/recreate the
worker for hard cancellation if inference cannot cooperate.

Tesseract.js 7's actual loader probes both ordinary and relaxed WASM SIMD and
chooses a compatible core variant. Serve the matching worker/core/language set
from versioned same-origin paths. SIMD and a single worker do not require
`SharedArrayBuffer`; a scalar fallback remains distinct from a threaded build.
The current Tesseract path provides no WebGPU or ANE execution evidence.
[Versioned core-selection source](https://github.com/naptha/tesseract.js/blob/v7.0.0/src/worker-script/browser/getCore.js),
[worker and local asset configuration](https://github.com/naptha/tesseract.js/blob/v7.0.0/docs/local-installation.md).

For prospective ORT Web inference, keep the JS and WASM files from the same
build. Start with `env.wasm.numThreads = 1`; additional WASM threads require
browser support for shared memory/threads and successful cross-origin
isolation. Probe `crossOriginIsolated` and `SharedArrayBuffer`, and handle
initialization failure. Bound thread count independently of reported hardware
concurrency. The ORT proxy-worker mechanism cannot be used with its WebGPU
provider; create that session directly inside a dedicated worker instead.
[ORT runtime options](https://onnxruntime.ai/docs/tutorials/web/env-flags-and-session-options.html).

The server-side isolation contract normally uses COOP `same-origin` and COEP
`require-corp`, with compatible worker/subresource CORS or CORP and the relevant
Permissions Policy/embedding context. Check the resulting capability in the
real page. Those headers affect cross-origin assets and popup authentication;
they require application deployment testing, not changes to user browser
settings. WebGPU additionally requires a secure context and an available adapter/device in the
execution context and successful model inference. No Safari flags, security
settings, network settings or Site deployment were changed by this audit.
[HTML isolation capability](https://html.spec.whatwg.org/multipage/webappapis.html#dom-crossoriginisolated),
[browser-vendor isolation guide](https://web.dev/articles/cross-origin-isolation-guide),
[WebGPU API requirements](https://gpuweb.github.io/gpuweb/#navigator-gpu),
[ORT WebGPU usage](https://onnxruntime.ai/docs/tutorials/web/ep-webgpu.html).

Acceptance remains: real iPhone and iPad mini runs in supported Safari and
Chrome contexts; versions and model hashes; recognition errors on benign scan,
photo, rotation and mixed text/image cases; operator placement; model-load and
steady-state timings; memory/resource failures; cancel/resume and tab lifecycle.
Desktop Chromium or a Linux precompiled binary cannot satisfy that acceptance.
