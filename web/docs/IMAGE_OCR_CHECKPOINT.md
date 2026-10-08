# Browser OCR checkpoint for clean rebuild

Further legacy feature layering is paused at the user's request. This isolated
checkpoint preserves the diagnosis, a small adapter correction and acceptance
tests for potential reuse after independent verification. It is not a delivery
of the requested clean rebuild and has not been merged or deployed.

## Diagnosis and patch boundary

Main `1bc92ccb7b566f11a3b750f21007cfeb310593b2` already sends PNG/JPEG/WebP
uploads to Tesseract.js after saving the original, but only performs one English
recognition pass. PRs 264 and 265/266 address separate WebGPU and native scan-service
paths; none is imported here. Stored-document deletion and storage implementation
are outside this branch.

Actual local-browser uploads of synthetic JPEGs with muted contrast, 0.8px blur,
3-degree tilt and JPEG quality 0.55 reproduced zero recognized characters for
isolated 14px and 18px English headings. The original and empty Partial result
were saved. Larger 24px and 32px headings were recognized. The patch tries one
sparse-text pass (PSM 11) only after an empty first pass. It recovers the small
headings as `PHOTO OCR CHECK` (confidence 94 and 95). Both attempts are retained
in result evidence. This demonstrates a concrete failure and correction, but
does not establish why Ben's particular photo failed on the deployed Site.

The adapter now has one total 120-second deadline, cancellable decode/encode
waits, owned-worker termination, late-bitmap disposal, monotonic phase progress
and explicit `text`/`empty` outcome evidence. Empty outputs carry no confidence
claim. The reader shows English-model uncertainty and empty OCR explicitly.
Recognized identifiers remain unverified, and all OCR remains Partial.

## Independently useful acceptance tests

`scripts/test-browser-ocr.mjs` clicks the visible Upload control, opens Add photos
and uses the real file chooser. It runs the actual browser app, local Workers
D1/R2 storage and pinned same-origin CPU/WASM worker. It checks recognizable text
in the reader and saved record, reopening, actual recognition progress, exact
original bytes/SHA-256, still PNG/JPEG/WebP controls, empty/corrupt/unsupported
inputs, missing worker assets, cancellation during core loading and actual
recognition, worker disposal and successful subsequent uploads. Worker
instrumentation observes ownership and native progress; it does not fake OCR.
Synthetic pixels are generated in the browser; no private user fixtures are used.

`scripts/test-ocr-lifecycle.mjs` uses explicit decoder/canvas/worker fakes to probe
stalled decoding, encoding, runtime/model loading, initialization and recognition,
deadlines, exact abort reasons, late resource delivery, malformed output and
synchronous post failures. It separately proves cleanup and subsequent operation.
These lifecycle checks are not OCR-quality tests.

Reproduce after locked installation and copying the pinned OCR assets:

```sh
node scripts/test-ocr-lifecycle.mjs
pnpm dev
# In another terminal with an independently installed Playwright:
PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs node scripts/test-browser-ocr.mjs
```

Set `PLAYWRIGHT_EXECUTABLE_PATH` to use a provisioned Chromium and
`OCR_REPORT_PATH` to write the browser report. The browser suite requires local
mock sign-in and creates only synthetic records in local storage. It does not
delete any user data. Checked source identity is main `1bc92ccb` plus this branch's
recorded commit; deployed Site identity, buttons, authentication and storage are
not verified. Chromium is not Safari/iOS or a real mobile device.

## Explicit capability gaps

- English recognition of still JPEG/PNG/WebP is tested. Returned Unicode is
  retained, but multilingual, accents/diacritics and handwriting quality are
  unqualified. Lossy WebP misread a sentence's period as a comma in a control.
- Empty text does not prove a blank original. A text-free tilted rectangle
  returned punctuation (`|]`) in a probe; nonempty output is not validation.
- No right-angle orientation correction, deskew, perspective dewarping or
  preprocessing quality claim is made. At 8–12 degrees the original larger-text
  JPEG probe misread text and DOI characters. This correction does not fix that.
- Animated PNG/WebP and multi-picture JPEG are rejected by the existing inspector.
  GIF/BMP/TIFF return explicit unsupported-image errors with originals saved.
  HEIC and other binary formats remain storage-only; export a still JPEG/PNG.
- PDFs still use embedded-text extraction; scanned pages have no OCR in this
  browser path. Existing Office extraction retains embedded images without OCR.
  Audio/video have no transcription.
- The current API is `recognizeImage(File, progress?, signal?, options?)`, using
  DOM decoding/canvas and Web Worker lifecycle around WASM. It is not a portable
  Rust/WASI API. A raster-input/result contract and runtime adapters need separate
  implementation and acceptance tests in the clean rebuild.
- The working raster requests at most 4 million pixels and 4096 pixels per side;
  this is not a strict bound on browser decoder internal memory. Native decoding
  and encoding cannot be forcibly aborted: waits settle and late bitmaps are
  closed. Synchronous browser calls and background timer throttling can delay
  deadline delivery. Worker termination occurs when abort/deadline is delivered.
- The original is saved before OCR. Changes to that dependency, source provenance,
  durable recovery, deletion and a clean upload-to-runtime contract belong to the
  clean architecture assignment rather than this checkpoint.
