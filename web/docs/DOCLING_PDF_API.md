# Local Docling PDF API

`/pdf-api/v1/api.js` is a browser ES-module API. The workspace and
`/pdf-api/v1/index.html` call the same implementation. It is not an HTTP server,
WASI runtime, native Docling service or deployment. No PDF is uploaded by the
standalone UI; the existing authenticated workspace still saves originals.

```js
import {discoverPdf, createPdfJob, getPdfJob, cancelPdfJob, disposePdfRuntime}
  from '/pdf-api/v1/api.js';
console.log(await discoverPdf());
const job = createPdfJob(file, {ocr: 'auto', pages: [1, 2], onProgress: console.log});
const result = await job.result; // or cancelPdfJob(job.id)
disposePdfRuntime();            // optional immediate release of idle model memory
```

The versioned exports also include `API_VERSION` and serializable control/result
`schemas`. Input is File, Blob, Uint8Array or ArrayBuffer. The job exposes `id`,
`status`, `progress`, `subscribe(listener)`, `cancel()`, and a `result` promise.
The terminal statuses are `partial`, `failed`, `cancelled`. A successful model
run remains partial because inference does not establish complete recovery.
Invalid controls and concurrent jobs throw coded errors before claiming the
active slot. Processing failures resolve a result with coded diagnostics.
`signal` and `onProgress` are JavaScript lifecycle controls outside JSON schemas.
The API remembers at most eight job handles. Callers own any results they retain.

Default `engine: 'docling'` runs layout and supports English `ocr: 'auto'`,
`'always'`, or `'off'`. Automatic routing uses the selected page's PDF.js text
cells; fewer than 20 non-whitespace characters routes to OCR. This is a stated
heuristic, not a guarantee that an existing text layer is correct. `always`
allows explicit raster recognition. Automatic digital-page OCR uses PDF.js's
rendered image-operation bounds, including clipping and transforms, to crop
raster regions independently of model labels. Intersecting image bounds are
merged. Each crop masks the positions of existing native text before passing
through Docling's ScannedConverter, detector and recognizer. DigitalConverter
keeps the native text and performs layout without a second recognition pass.
Results retain each original region document, crop coordinates, masked native
cell IDs and native/OCR provenance. Repeated words at different positions survive.
Bounds are quantized outward to 1/256 of the rendered page; these rectangles
do not establish exact irregular clipping coverage. Vector outlines without a
text layer or image operations still require explicit full-page OCR.
Password-protected PDFs are unsupported.
`engine: 'fast-text', ocr: 'off'` instead calls PDF Oxide with unverified order,
no layout/OCR and no page selection. The engines do not run on the same job.
Switching to fast text releases any idle Docling runtime first.

Each page retains text, original page number, original Docling JSON,
`doclingPageMap`, immutable PDF.js cells, model `layout`, `orderedBlocks`,
`order`, order provenance, links, diagnostics, timings and resources.
`doclingPageMap` maps local page numbers inside upstream per-page JSON back to
original PDF page numbers. Coordinates retain the declared TOPLEFT/BOTTOMLEFT
origin. JavaScript JSON numbers have browser precision limits. Result export
and the workspace avoid nesting a second complete copy of all pages in metadata.

## Pinned runtime and provisioning

Docling.rs WASM **1.104.2** at
[`29de9d1e842e6ebb35890c3f171ac0c38f273eed`](https://github.com/docling-project/docling.rs/tree/29de9d1e842e6ebb35890c3f171ac0c38f273eed/crates/docling-wasm)
runs actual DigitalConverter/ScannedConverter with PDF.js 5.4.624 and ONNX Runtime
Web 1.24.3. Interop follows upstream's MIT-licensed `www/pipeline.js`. Every ONNX
session selects CPU `wasm`, one thread, with proxy execution disabled. There is
no GPU implementation or experiment. No Tesseract call occurs on this path.

From `web`, stage the checksum-pinned static artifacts, then explicitly opt into
models. Package lifecycle hooks and repository security policies are unchanged.

```sh
node scripts/copy-pdf-wasm.mjs
node scripts/stage-docling-assets.mjs
node scripts/stage-docling-assets.mjs --models
```

This environment uses Node 24 `--use-env-proxy` for staging fetches. Hugging Face
returned 403; upstream GitHub `models-v1` release assets worked. Archive SHA-512
and model SHA-256 are pinned in source; licenses are staged with the runtime.
Models total **87,592,123 bytes**: layout 68,695,321; English recognition 8,967,018;
detector 9,929,594; dictionary 190. Runtime plus models is about 117 MiB on disk.
Models are omitted without `--models`. Jobs fetch only same-origin runtime and
model assets, never third-party models. Missing/invalid assets fail explicitly;
there is no hidden recognition-only fallback when the detector is missing.
TableFormer and multilingual OCR are unsupported.

## Parsers, copies and retained memory

The pinned browser `DigitalConverter::new(bytes, dict)` parses the document
once in Rust via Docling's textparse/lopdf path. It has no exported constructor
for external parsed cells. Native `PdfPage::from_cells` seams are not browser
exports. Docling does not depend on PDF Oxide. The integration therefore uses:

- one PDF.js document for page count, selected-page text evidence and rendering;
- at most one Rust parse, delayed until the first digital page; none for
  scanned-only jobs or `ocr: 'always'`;
- zero PDF Oxide instances during a Docling job;
- one selected page's raster at a time, with its RGBA buffer transferred to the
  inference worker and the canvas backing store reset after pixel extraction.

A snapshot preserves caller bytes. Automatic/digital jobs copy that snapshot
once for PDF.js and transfer the retained snapshot to Rust's worker only when
needed. `always` transfers the single snapshot directly to PDF.js. wasm-bindgen
copies byte slices into Rust linear memory; upstream preprocessing then copies
normalized float arrays to JavaScript. ONNX Runtime owns its additional tensor
and arena allocations. The adapter does not claim zero-copy interop. PDF.js
parses only one document; classification and geometry reuse each selected
page's single bounded text stream. A whole-document page ceiling also limits
Rust parsing when callers select a subset of a large PDF.

One idle inference worker retains verified model sessions, not parsed documents.
An end command frees DigitalConverter after each successful job. Session IDs,
model asset IDs, output shapes and SHA-256 fingerprints make reuse observable.
Idle expiry after 30 seconds, explicit `disposePdfRuntime()`, interruption,
failed execution, or Rust linear capacity above 256 MiB discards the worker.
Rust linear memory does not shrink when document objects are freed; worker
termination drops its ownership, but immediate OS RSS reduction is not promised.

## Bounds and foreign-runtime containment

Host ceilings: 25 MiB input; 200 document pages; 20 selected pages; 6 million
rendered pixels per page at scale 2; 16 raster OCR regions; 100,000 text cells and 1,000,000 text
characters per page; 8 MiB serialized result; 120 seconds; one active job per
API instance. Callers can lower limits (output minimum 16 KiB). An overflowing
page/result is withheld, with a failed status, rather than returned beyond the
output limit. Model downloads use a fixed-size buffer and validate length and
SHA-256 before ONNX session creation. These controls are not a hard cap on
internal allocations by upstream PDF.js, Rust WASM, ONNX or the browser.

The actual boundary is Rust/wasm-bindgen ↔ JavaScript typed arrays ↔ ONNX Runtime
Web WASM. Async Rust conversion awaits each JS session callback. A single
worker owns sessions and document state; commands and tensor operations are
serialized. Each session run awaits completion before its arrays are consumed
by Rust postprocessing. Shapes, float32 type and finite values are checked
before decoding; output hashes are recorded without retaining tensor payloads.
OCR off never passes a cached recognizer to DigitalConverter. No shared-memory
payloads or application-managed concurrent inference threads are used.

A lease binds messages to a job ID and worker identity. Busy commands are
rejected. Cancellation terminates the owned worker, cancels rendering and closes
PDF.js; queued events from an old worker cannot resolve a new job. Healthy
completion frees document state only after pending conversion has completed.
Faults discard the entire runtime so a subsequent job starts fresh. The browser
harness tests active-inference cancellation, initialization cancellation,
concurrent-call rejection, an injected worker crash and successful recovery.

A Web Worker is **not OS process isolation**. WebAssembly bounds linear memory,
but Rust's type guarantees do not prove ONNX's C/C++ implementation or the browser
engine memory-safe. Upstream defects can still corrupt results, exhaust memory,
trap or crash a renderer. The one-thread provider and ownership protocol contain
application-level races; they do not certify foreign runtimes. Native ONNX FFI,
Tesseract CLI and the separate image-OCR Tesseract WASM path are outside this
implementation's evidence. Separate tabs/API instances have separate limits.

## Browser evidence and reproducibility

Only verified public-safe fixtures are used. The native control's SHA-256 is
`099a620bd29179e329704c152808ad8e3e34f0d5388894c43d17fb3340d373c8`.
Scanned/mixed/six-page and overlapping-layer derivatives preserve its independently authored expected
sentences. The two column witnesses and handwritten expectations were copied
byte-for-byte from PR269 head `850a66b577f03205ad4692d578319e1e7fe95417`;
no legacy implementation was imported. Their hashes are asserted by the harness.

Upstream layout merged each tiny column witness into a single region and kept
scrambled stream order. A bounded horizontal-LTR ordering policy resolves
repeated whitespace gutters independently within text bands, with gaps of at
least 1.5 text heights supported by at least two rows. Nearby paired rows handle
staggered baselines; a vertical gap above three text heights starts a new band.
Spanning rows divide column runs. Word cells on the same ordered line are joined
without changing the retained source cells. Repair requires conservation of the
PDF.js cell/Docling character multiset and absence of model tables. Original
model layout stays intact and diagnostics retain gutter positions and support.
The [expanded browser evidence](evidence/docling-browser-columns-memory-baseline.json)
passes the original two/three-column expectations plus independently specified
derivatives with changing column counts, staggered baselines and spanning
headings/footers. Rotated/RTL text does not enter this policy. This is not general
scientific-layout or scanned multi-column qualification.
Phones and Safari are unqualified; English-only synthetic OCR is limited evidence.

The [automatic-region checkpoint](evidence/docling-browser-regions.json) proves
both expected text copies from the same-page native/raster mixture in automatic
mode. A separate overlapping native/raster witness yields only one copy, with
three native text cells spatially masked in the OCR crop. This replaces the
earlier `DIGITAL_OCR_SCOPE` limitation; originals and prior evidence are retained.

The [recorded Chromium 151 run](evidence/docling-browser-owned-runtime.json)
passes 110 API checks, both independent column-order witnesses, the real UI,
missing/corrupt assets, an injected worker fault and idle expiry, with no unhandled
page errors or external requests. Its [UI screenshot](evidence/docling-browser-owned-runtime.png)
shows the same callable result. The run took about 5.5 s cold and 3.3 s warm for
one digital page, 6.4 s for a scan, 9.4 s for two mixed pages and 18.5 s for six
digital pages. Rust linear capacity was approximately 35–65 MiB. Aggregate
browser-process RSS peaked near 1.3 GiB, counting shared mappings in each process;
proportional-set size peaked near 1.07 GiB. Neither is an in-browser bound.
Download size significantly understates browser RAM needs.

CI installs test-only dependencies from `scripts/browser-tools/package-lock.json`,
stages the exact assets, runs real Chromium and retains the JSON/screenshot
artifact for the exact PR head. Local reproduction:

```sh
npm ci --prefix scripts/browser-tools --ignore-scripts
node scripts/browser-tools/node_modules/playwright/cli.js install chromium
PLAYWRIGHT_MODULE=./browser-tools/node_modules/playwright/index.mjs \
PDF_LIB_MODULE=./browser-tools/node_modules/pdf-lib/cjs/index.js \
EVIDENCE_PATH=/tmp/docling-evidence.json node scripts/test-docling-browser.mjs
```

`CHROMIUM_PATH=/usr/bin/chromium` selects an existing browser. Evidence distinguishes
actual tensor inference (shapes, finite ranges, hashes, timings, session IDs)
from success logs. The same input must yield the same tensor fingerprint in a
reused session; changed pixels must change it. Complete normalized expected text
and handwritten column order are checked independently. Resource measurements
sample only the launched test browser's process tree and exclude the Node
fixture server. They are observations on that machine, not mobile guarantees.
