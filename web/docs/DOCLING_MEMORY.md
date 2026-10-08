# CPU/WASM memory assessment

The browser PDF path still needs substantial RAM. The measured changes reduce
specific allocations and simultaneous ownership; they do not establish a mobile
memory budget or prevent an upstream/browser out-of-memory failure.

[Comparison evidence](evidence/docling-memory-comparison.json) records two runs
in opposite orders for each experiment, each profile in a fresh owned Chromium
151 process tree. Every run executed real native, scanned and same-page mixed
PDFs and checked complete, independently expected text. Models and provider
stayed pinned. No garbage collection was forced. The earlier
[full browser baseline](evidence/docling-browser-columns-memory-baseline.json)
also includes six-page and column-order cases.

## What is measured

The harness samples Linux `/proc` every 100 ms for descendants of the browser it
launched. It records VmRSS, proportional-set size (PSS), private resident bytes,
process type, phase peaks and the process breakdown at the aggregate RSS peak.
It excludes the Node fixture server and unrelated processes. RSS counts shared
mappings in multiple processes; PSS apportions those pages. Peaks shorter than
the sampling interval can be missed. Browser process types come from Chromium's
`--type` argument. They do not reliably map one renderer process to one JS worker.

A separate **test-only** wrapper observes WebAssembly constructors and instance
exports inside the inference worker. It identifies Docling's exported memory
and ONNX's constructor location and reports their current linear capacities.
These capacities are neither live allocations nor resident pages. **Do not add
them to RSS/PSS.** A zero-byte ONNX feature-probe memory is recorded too; it is
not a third live application heap. The production app does not install this
wrapper. CI enables it only in the test harness.

At one baseline scan peak, the owned processes were:

| Process group | RSS MiB | PSS MiB | Private MiB |
| --- | ---: | ---: | ---: |
| Largest renderer | 770.4 | 736.8 | 721.2 |
| Other two renderers | 206.0 | 150.7 | 129.0 |
| Browser | 127.4 | 88.8 | 74.4 |
| GPU process used by browser rendering | 38.7 | 23.0 | 13.3 |
| Two utility processes | 85.3 | 38.0 | 23.9 |
| Two zygotes | 96.8 | 27.3 | 1.4 |
| Total | 1324.7 | 1064.5 | 963.2 |

The GPU process is browser infrastructure, **not a GPU inference provider**.
All ONNX sessions use one-thread CPU/WASM. After disabling prepacking, the
largest renderer in the corresponding scan run was 721.9 MiB RSS / 690.1 MiB
PSS. The other process groups were broadly similar. Heap attribution below
comes from the worker probe, independently of these process measurements.

## Models, heaps and raster lifetimes

Models total 87,592,123 serialized bytes (83.5 MiB): layout 65.5 MiB, recognition
8.6 MiB, detector 9.5 MiB, plus the small dictionary. A verified download buffer
is copied into ORT's linear heap for session creation. Decoded weights,
initialization workspace and activations occupy additional storage. The probe
cannot separately account for each native allocation inside ORT; serialized
model size is not model resident size.

| Measured stage | Docling linear MiB | ORT default linear MiB | Prepacking disabled linear MiB |
| --- | ---: | ---: | ---: |
| Layout session ready, before inference | 3.6 | 185.0 | 185.0 |
| First digital layout completed | 34.6 | 222.0 | 185.0 |
| Scan detector completed, all three sessions loaded | 64.5 | 460.5 | 400.1 |
| Recognition completed | 64.5 | 460.5 | 400.1 |

The largest ORT capacity increase occurs during detector inference, not during
the small dictionary or recognition model download. The data distinguish two
application WASM heaps; there is no additional PDF Oxide instance in Docling
jobs. ORT's heap holds all its sessions. Docling's heap holds its Rust parser,
image preprocessing and postprocessing allocations. Their high-water capacities
do not shrink when a document or temporary tensor is freed.

The letter fixture's transferred page RGBA is 7.4 MiB. The tall same-page mixed
fixture uses 14.8 MiB RGBA plus at most one 7.4 MiB OCR crop. Rust also copies
WASM-bindgen input slices and creates RGB/normalized image arrays; ORT copies
input tensors into its heap. Those copies are included within the measured
heaps/process totals. A page canvas exists while obtaining ImageData, then its
backing dimensions are reset before inference. Native PDF bytes are snapshotted
once, copied for PDF.js when needed, and transferred once for at most one Rust
parse. No PDF Oxide parse occurs on this path.

## Applied changes and observed effects

1. **Disable ONNX weight prepacking.** The supported
   [`session.disable_prepacking` option](https://github.com/microsoft/onnxruntime/blob/v1.24.3/include/onnxruntime/core/session/onnxruntime_session_options_config_keys.h)
   avoids packed weight copies. Across both run orders, digital ORT capacity
   fell from 222.0 to 185.0 MiB and the OCR high-water capacity from 460.5 to
   400.1 MiB (about 60 MiB / 13%). Scan aggregate peak RSS fell by 43–54 MiB;
   its PSS fell by 41–52 MiB. Warm digital RSS fell by about 193–197 MiB in this
   comparison, but this exceeds the heap-capacity change and includes JS/GC
   timing; it must not be attributed entirely to packed weights. Scan time was
   6.53–6.57 s versus 6.58 s in the two baseline runs. Tensor fingerprints change
   with kernel selection, so this is not a claim of bitwise equivalence. Every
   independent text assertion passed, and the complete PDF suite validates the
   production setting. Broader model accuracy remains unqualified.
2. **Release PDF.js before final-page inference.** Text cells, image bounds,
   links and raster pixels are collected first; the renderer is then destroyed
   and unneeded PDF bytes released. The one-page mixed witness's peak RSS fell
   by 21–47 MiB and the scan's by 7–34 MiB in the paired experiments. Digital RSS
   moved in both directions with collection timing. The change reduces owned
   simultaneous resources; it does not promise a corresponding immediate OS
   reduction. Earlier pages of a multi-page job still retain the one PDF.js
   document, avoiding repeated parsing.
3. **Load OCR only for selected content that needs it.** A digital page without
   raster regions executes layout only. Raster crops use the existing detector
   and recognizer, sequentially, with native text positions masked. Successful
   jobs reuse sessions; cancellation/faults terminate the worker. Explicit
   disposal or 30-second idle expiry removes the runtime owner. After explicit
   disposal the benchmark process tree retained roughly 376–398 MiB PSS of
   browser/test infrastructure, rather than dropping to zero.

The final peak remains around a gigabyte of PSS on these workloads. Phones,
Safari, large scans and arbitrary layouts have not been qualified. The API's
byte/page/pixel/time/output limits bound requests and returned data, not internal
allocations by PDF.js, Rust, ONNX or the browser. A Worker is not OS process
isolation, and Rust's guarantees do not certify foreign-runtime memory safety.

## Reproduce

Stage the pinned assets and install the test tools as described in
[the API guide](DOCLING_PDF_API.md). From `web`:

```sh
MEMORY_PROFILES=default,noPrepack \
PLAYWRIGHT_MODULE=./browser-tools/node_modules/playwright/index.mjs \
PDF_LIB_MODULE=./browser-tools/node_modules/pdf-lib/cjs/index.js \
EVIDENCE_PATH=/tmp/memory-options.json node scripts/benchmark-docling-memory.mjs

MEMORY_PROFILES=noPrepackKeepRenderer,noPrepack \
PLAYWRIGHT_MODULE=./browser-tools/node_modules/playwright/index.mjs \
PDF_LIB_MODULE=./browser-tools/node_modules/pdf-lib/cjs/index.js \
EVIDENCE_PATH=/tmp/memory-lifetime.json node scripts/benchmark-docling-memory.mjs
```

`default` means ORT's library session defaults, not the app's new production
setting. `noPrepack` uses the production setting; `noPrepackKeepRenderer` changes
only the test's renderer lifetime. Repeat in reverse profile order. The harness
injects those experiments into its local worker/API responses, preserving the
app files and model assets. `CHROMIUM_PATH` can select an installed Chromium.
Raw tensor fingerprints, shapes, page text, session settings, process samples
and heap observations remain in the evidence; no real user documents are used.
