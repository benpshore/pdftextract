# Docling modules

The integration remains pinned to **docling.rs 1.69.2**. Its APIs and behavior
were checked against the published crate source, commit
`908f080d5def736bbc094beeaf5b4a04fe504dd3`. Current official documentation exposes
1.91.0 and a newer default Rust renderer; those capabilities are not claimed by
this pinned integration: https://docs.rs/docling-pdf/latest/docling_pdf/ and
https://github.com/docling-project/docling.rs.

## Text parser: ready for supervised extraction

```sh
cargo build --features docling-text
./target/debug/tpe extract --backend docling-text --db output.sqlite input.pdf
```

This feature disables upstream default features. It needs no PDFium, ONNX,
models, environment-selected OCR, model downloads, or subprocesses. A retained
`PageTextParser` opens immutable bytes once and parses only requested pages. Rust
owns reading order and cleanup. Cells keep their real geometry, translated from
Docling's clipped crop frame to unrotated PDF coordinates. The public cells API
provides neither font name nor font size; those fields remain absent.

The lopdf evidence session preserves raw URI annotations, images, metadata, and
completeness warnings. Non-whitespace Unicode scalar coverage is compared against
that native evidence. A mismatch is Partial; a shorter Docling candidate retains
the native text. Matching scalar counts establish a coverage cross-check, not a
claim that reading order or punctuation interpretation is perfect. Existing
mapping or resource-limit evidence is never cleared. The upstream parser does
not support password input, so encrypted documents require another backend.

The CLI's existing disposable worker applies hard resource limits. The library
API does not itself install a process boundary. Output cells/text are capped,
while parser computation and native allocations remain contained by that worker.

## Layout and OCR: separate opt-in deployment

`--features docling` includes the text feature and explicitly enables the older
PDFium/ONNX pipeline. The `docling` backend remains unavailable in supervised CLI
extraction. It converts and caches the document rather than providing the bounded
page parser above; model discovery and inference still require a separately
provisioned trusted runtime. No models are downloaded by this adapter.

The adapter explicitly selects in-process PP-OCR, so an environment variable
cannot silently switch it to an external Tesseract command. Raw URI annotation
targets are retained separately from Docling's Markdown wrappers. When crop or
rotation frames cannot be verified, their rectangles remain absent with a Partial
diagnostic instead of combining incompatible geometry. Missing pages,
unknown geometry, undecoded formulas, and unverified reconstruction coverage are
reported as Partial. A successful upstream conversion is not completeness proof.

Enabling layout/OCR in the supervised CLI needs a further deployment contract:
absolute per-file model paths and hashes, PDFium and ONNX runtime identities,
explicit language/mode, fixed worker thread limits, no CWD asset fallback, and
verified page/output coverage. These assets are not included in standard releases.
The upstream model resolver checks the working directory, and missing recognition
models can degrade to warnings, so merely setting a model directory is not enough.

## Scan service for the browser alpha

`crates/tpe-scan-service` is a loopback-only HTTP service that the web alpha
can hand a scanned PDF, PNG or JPEG to and get page text back through the
docling backends above. `web/lib/scan-service.ts` is its browser client. The
service is the only way the browser reaches docling: the web app itself never
compiles or downloads models, and the workspace decides between this service
and the in-browser OCR; the client never falls back on its own.

```sh
cargo build -p tpe-scan-service --features docling-text   # mode=text, no models
cargo build -p tpe-scan-service --features docling        # mode=ocr: PDFium + models
./target/debug/tpe-scan-service --port 5209 --origin http://localhost:3000 \
    --models-dir .models
```

Start-up prints the bound address, the allowed origins, whether the models and
PDFium are provisioned, the limits, and the per-launch bearer token exactly
once. The person pastes the port and the token into the web app's scan-service
dialog; the client keeps them in `sessionStorage` for that tab only. Without
the `docling-text` or `docling` feature the binary still runs and answers
`/capabilities`, and every scan fails with `not_compiled`.

Security model, enforced and tested in `tests/service.rs`:

- Binds `127.0.0.1` (or `::1`) only; any other `--bind` is refused.
- Every request needs `Authorization: Bearer <token>`; the token is 64 random
  hex characters generated per launch and never written to disk.
- `Host` must name the bound address; a browser `Origin` must be one of the
  `--origin` values (`scheme://host[:port]`, normalised). CORS preflight and
  `Access-Control-Allow-Origin` are scoped to exactly those origins, and
  `Retry-After` is the only exposed header.
- Bounded: `--max-body-mib` (64), `--max-pages` (50), `--max-concurrent` (1),
  `--scan-timeout-s` (120). Oversize bodies are `413 limit`, a busy service is
  `429 busy` with `Retry-After`. Host/Origin/token validation precedes
  body-length validation and upload storage. Only a fixed 4 KiB read chunk
  can arrive alongside the bounded head before authentication; a declared
  length never reserves body storage. Admission covers upload and execution,
  so only `max_concurrent` scans accumulate bodies or convert images.
- Head receipt has a fixed 10 s deadline from acceptance; upload completion
  has a fixed 70 s deadline (10 s head budget plus 60 s upload budget) from
  acceptance. Sending more bytes does not extend either deadline (`408
  request_timeout`). Shutdown and upload cancellation interrupt reads.
- Each scan runs in a disposable worker process of the same binary (hidden
  `worker` subcommand) with an address-space growth limit
  (`--worker-memory-mib`, 4096) and the deadline; a hung or crashed conversion
  is killed and reported as `504 timeout` or `500 internal`, and the service
  stays up. PNG/JPEG decoding, conversion, compression and PDF wrapping all
  occur in that worker after its hard memory limit is installed, under the
  same scan deadline. Worker stderr is drained through fixed 4 KiB buffers,
  retaining only its last 4 KiB while reading. The worker boundary exists on
  Linux and macOS (`rustix`).
- Disconnect, shutdown or authenticated cancellation kills the request's
  private worker process group and joins its input/output drains before
  releasing admission. The browser assigns a random `id` to each scan and
  sends `POST /cancel?id=<id>` on a separate authenticated connection when
  aborted; it waits for acknowledgement before its promise settles. A
  `204` acknowledges cleanup and capacity release; `cancel_pending` means
  cleanup could not be confirmed, so the caller must wait before Retry.
  Cancellation that overtakes scan headers is recorded in at most 64
  short-lived entries (head deadline plus one second) to block late starts.
  IDs are operation identifiers, never credentials. XHR abort listeners are
  removed at completion; abort during result parsing suppresses late success.
- No outbound connections, no telemetry, nothing on disk. Model files are
  probed (and hashed unless `--no-hash`) at start and re-probed on each
  `/capabilities` call, so provisioning while the service runs is noticed.

HTTP contract (success/error bodies are JSON; preflight/cancel acknowledgements are empty):

- `POST /cancel?id=<id>`: token/Host/Origin checks are identical to scans,
  accepts no body, and returns `204` only after the matching request releases
  its slot. Other active scans continue. Cancellation tracking exhaustion is
  `429 cancel_pending`; worker cleanup exceeding 5 s is `503 cancel_pending`.
- `GET /capabilities`: `service` (name, version, pinned docling version,
  pid), `build` (`ocr_compiled`, `text_layer_compiled`), `ocr` (`available`,
  engine `ppocr`, `languages`, `reason` when unavailable, what `confidence`
  means), `text_layer`, `models` (`provisioned`, `missing`, `searched`,
  per-file path, bytes and sha256), `pdfium` (configured path, library,
  present), `limits`, `accepts`, `modes`, `origins`.
- `POST /scan?mode=ocr|text&pages=first-last`: the body is the raw PDF, PNG
  or JPEG bytes, or a multipart form whose file part carries them. The kind
  is sniffed from the bytes, not the declared type (`415
  unsupported_media_type` otherwise). A still image is embedded losslessly in
  a one-page PDF at an assumed 300 dpi and goes through the same backend; the
  result's `warnings` record the pixel size and that assumption. `pages` is
  an inclusive 1-based window (`1-20`, or a single page); omitted means from
  page 1 up to `max_pages`, and clients window longer documents. The reply:
  `backend` (name, version, config digest), `mode`, `input`, `pages_total`,
  `pages_scanned`, `pages[]` (number, size, rotation, `status`
  `complete`/`partial`, `text`, `confidence`, `blocks`, `spans`, `figures`,
  per-page `warnings`), `warnings`, `elapsed_ms`.
- Errors are `{ "error": <code>, "message": <text> }`. Request errors:
  `bad_mode`, `bad_pages`, `no_file`, `bad_multipart`, `empty_body` (400),
  `unsupported_media_type` (415). Scan errors: `models_not_provisioned`,
  `not_compiled` (503), `malformed`, `page_range` (400), `encrypted`,
  `unsupported` (422), `limit` (413), `timeout` (504), `busy` (429),
  `internal` (500). The client maps each to a plain message.

`mode=text` uses the pure-Rust page parser (`docling-text`): it reads an
existing text layer and reports no confidence; a scan or an image yields empty
text with a warning rather than a guess. `mode=ocr` needs the `docling`
feature, the hash-verified PDFium library (`PDFIUM_DYNAMIC_LIB_PATH`, see
`docs/NATIVE.md`) and the docling.rs 1.69.2 model files in `.models`
(`--models-dir` or `DOCLING_RS_MODELS_DIR`); `/capabilities` lists exactly
which files are missing and where it looked, and the service refuses OCR with
`models_not_provisioned` until they are all present. OCR confidence is the
backend's per-page mean score when it reports one; there is no per-word
confidence. Pages the backend could not fully reconstruct are `partial` and
the client surfaces them, following the Partial rules above.

Verification without models or PDFium: `cargo test -p tpe-scan-service`
(token, `Host`/`Origin`, CORS, limits, routing, worker boundary and
deadline, shutdown) and, from `web/`, `node tests/scan-service.test.mjs` (a
fake service: connection parsing, session-only storage, capability
detection, progress, page windows, error mapping, the fallback message).
`cargo test -p tpe-scan-service --features docling-text` adds the real
`mode=text` scan of the engine's probe PDF and the PNG wrapper.
