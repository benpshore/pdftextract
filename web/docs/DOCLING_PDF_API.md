# Docling browser PDF checkpoint

This is a local browser JavaScript API and small UI, not an HTTP conversion
server, WASI module, deployment, or native Docling service. Upstream native
`docling-serve` already exists; this branch adds no competing server.

`/pdf-api/v1/api.js` exports `discoverPdf()`, `createPdfJob(input, options)`,
`getPdfJob(id)`, `cancelPdfJob(id)`, `schemas`, and `API_VERSION`. The job owns
`id`, `status`, `progress`, `subscribe(listener)`, `cancel()`, and `result`.
`result` resolves a structured partial/failed/cancelled result. Invalid options
and a concurrent job throw errors with a `code`. Input is File/Blob/Uint8Array/
ArrayBuffer, copied before transfer; originals remain unchanged.

```js
import { discoverPdf, createPdfJob } from '/pdf-api/v1/api.js';
console.log(await discoverPdf());
const job = createPdfJob(file, {ocr: 'auto', pages: [1, 2], onProgress: console.log});
const result = await job.result;
```

The default engine is `docling`; OCR supports `auto`, `always`, `off` and
English only. Layout always runs in that engine. `engine: 'fast-text',
ocr: 'off'` calls retained PDF Oxide; its order is explicitly unverified and
it provides no OCR/layout. The workspace and standalone
`/pdf-api/v1/index.html` call this same API. No PDF is uploaded by the standalone
UI. The existing authenticated workspace still saves originals as before.

Docling.rs WASM **1.104.2**, source
`29de9d1e842e6ebb35890c3f171ac0c38f273eed`, runs DigitalConverter/ScannedConverter
with PDF.js 5.4.624 and ONNX Runtime Web 1.24.3. All sessions explicitly use
single-thread CPU/WASM. No GPU implementation or experiment is included.
Interop follows upstream MIT-licensed `crates/docling-wasm/www/pipeline.js`.
The staging script retains dependency licenses and verifies npm archive
SHA-512 and model SHA-256. It executes no package lifecycle hooks and changes
no repository dependency/security policy.

From `web`, explicitly stage static assets, then opt into models once:

```sh
node scripts/copy-pdf-wasm.mjs
node scripts/stage-docling-assets.mjs
node scripts/stage-docling-assets.mjs --models
```

In this environment Node 24 needs `--use-env-proxy` for the staging fetches.
Hugging Face returned 403; the upstream `models-v1` GitHub release assets worked.
Models total **87,592,123 bytes**: layout 68,695,321, English recognition
8,967,018, detector 9,929,594, dictionary 190. Staged runtime plus models is
approximately 117 MiB. Models are omitted without `--models`; PDF jobs only
request same-origin assets, use browser caching, and never fetch third-party
models. Each owned worker still initializes its own ONNX sessions. RAM can be
substantially greater than download size; phones/Safari are unqualified.
TableFormer and multilingual OCR are unsupported.

Host ceilings are 25 MiB input, 20 selected pages, 6 million rendered pixels
at scale 2 per page, 8 MiB structured output, one active job, 120 seconds.
Callers may lower limits. These are host admission/output/time controls, not
a guarantee against allocations inside PDF.js, ONNX Runtime or Rust WASM.
Missing/invalid models fail explicitly; a scanned page requires the detector.
No detector timeout or recognition-only downgrade is hidden as success.

Results retain original Docling JSON, PDF.js text cells, original page identity,
`doclingPageMap`, order/layout evidence, actual inference output shapes/timing,
engine/provider/model identities, and diagnostics. All recovered output remains
`partial`: successful inference does not prove every region was recovered.
Docling JSON numbers follow browser JSON precision limits.

Actual Chromium 151 execution recovered the three handwritten expected
sentences from the verified public-safe native control, its raster-only
derivative, and a mixed two-page derivative. Originals were unchanged; routes
were `1:digital,2:scanned`. Cancellation followed by a successful next job and
selection of original page 2 passed. No third-party requests occurred.

The two PR269 semantic witnesses were copied byte-for-byte from
`850a66b577f03205ad4692d578319e1e7fe95417`; no legacy implementation was imported.
Upstream model output merged each witness into a single region and kept scrambled
stream order. A small independent browser adapter now orders repeated whitespace
gutters only when the original PDF.js cell text and Docling output conserve the
same character multiset and no tables are present. Both handwritten token
sequence expectations passed real PDF/API execution. This heuristic is
attributed separately from Docling, preserves both inputs, and remains partial.
General scientific layout and scanned multi-column conformance are unqualified.

The initial full browser evidence is retained separately from the current
expanded harness. Reproduce with Playwright and pdf-lib tools outside this
project's dependency policy:

```sh
PLAYWRIGHT_MODULE=/tmp/pdf-tools/node_modules/playwright/index.mjs \
PDF_LIB_MODULE=/tmp/pdf-tools/node_modules/pdf-lib/cjs/index.js \
CHROMIUM_PATH=/usr/bin/chromium EVIDENCE_PATH=/tmp/docling-evidence.json \
node scripts/test-docling-browser.mjs
```

Checkpoint handoff: TypeScript, Ruff, 210 Python tests, and import-flow tests
passed. Expanded missing-model/invalid-model/UI/time-budget tests were running
when the user requested the implementation-model handoff. The existing
`scripts/test-web-workspace.cjs` loader needs an alias for `@/lib/pdf-api`.
No hosted browser CI has been added yet; exact-head CI must still be followed.
Local Rust/Swift/CMake tools are absent; uv audit cannot reach api.osv.dev.
