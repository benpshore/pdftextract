# Local CPU OCR qualification command

`scripts/cpu_ocr.py` performs **real, offline OCR** using an explicitly provisioned
native Tesseract LSTM runtime and language model. PDF pages are rendered with
Poppler before recognition. This Linux command is a small qualification path;
it is not wired into the Rust extractor, Docling, the browser, or a hosted service.
It never uploads a document or downloads a model.

## Run it

Provision trusted native `tesseract`, `pdfinfo`, `pdftoppm` binaries and the
selected `.traineddata` files first. The command requires the model directory
explicitly. It fingerprints actual native executables and refuses shell
launchers: supply the executable behind a launcher using the named options.
`pdfinfo` and `pdftoppm` are only needed for PDFs.

```sh
uv run python scripts/cpu_ocr.py scanned.pdf \
  --tessdata-dir /usr/share/tesseract-ocr/5/tessdata \
  --tesseract /usr/bin/tesseract \
  --pdfinfo /usr/bin/pdfinfo --pdftoppm /usr/bin/pdftoppm \
  --language eng --workers 2 --dpi 200 --psm 6 \
  --artifacts-dir new-ocr-artifacts --output new-ocr.jsonl
```

`--output` and `--artifacts-dir` must be new paths; existing data is never
overwritten. Omit `--output` for JSONL on stdout. Both ordinary output pipes and
terminal backpressure are subject to the deadline. An inherited socket that
Linux cannot reopen fails explicitly; use `--output` in that case.

On this Codex VM, the preinstalled PATH entries for Poppler are two levels of
shell launchers. The actual **26.05.0** executables are:

```text
/opt/codex/runtimes/codex-primary-runtime/dependencies/native/poppler/poppler/bin/pdfinfo
/opt/codex/runtimes/codex-primary-runtime/dependencies/native/poppler/poppler/bin/pdftoppm
```

The separate `/usr/bin` Poppler tools report **25.03.0**. Do not conflate those
identities. The tests execute the actual 26.05.0 binaries. Fontconfig/system-font
and Poppler data choices can affect rendering and are not a hermetic pinned
dependency closure. The command records executable paths, hashes and reported
versions, plus each language file's path, byte count and SHA-256. It copies model
bytes into the private job directory, then directs Tesseract to that snapshot.
Executable hashes describe inspected trusted installations; replacing binaries
or shared libraries during a run is outside this qualification contract.

Inputs are PDFs, still PNG/JPEG images, binary grayscale/RGB PNM images, and
classic little/big endian TIFF. Multi-frame TIFF is rejected if Tesseract reports
more than one page; a successful first frame is never silently published as a
complete TIFF result. Animated images, BigTIFF and automatic orientation are
not qualified. Mode 6 assumes one text block; select another supported `--psm`
for a different layout. Modes requiring automatic orientation are excluded.

## Evidence and completion

The source is opened once as a regular file, streamed into a private immutable
snapshot and hashed. FIFO input is rejected without waiting. Changes to source
size or timestamps during copying fail; source bytes are never modified.

JSONL contains a `start` record with source/runtime identities and effective
options, one `page` or `page_error` record for each published page, then a
`summary`. Page completion order can differ from source order when workers are
parallel; the original one-based PDF page number is authoritative. Each successful
page includes:

- Text, per-word confidence, block/paragraph/line identifiers and actual raster
  pixel boxes with a top-left origin.
- Source SHA-256, exact raster-file SHA-256 and raster dimensions. A PDF raster's
  requested DPI is recorded. Images have no invented physical DPI.
- `pdf_transform: null` and a PDF transform warning. No unverified crop,
  rotation, page-point, font or Unicode-map provenance is manufactured.
- Raw TSV SHA-256 and, when selected, a retained local TSV path plus diagnostic
  files. Without an artifacts directory, working TSV/diagnostics are removed.
- Rasterization, recognition, parse/artifact and total page elapsed seconds,
  plus maximum RSS values for the Python page worker and its native children.

Every OCR page is `partial`, including apparently accurate output. Low-confidence
words and empty recognition are explicit warnings; empty text does not prove a
blank page. A successful summary means the selected OCR work finished, not that
the transcript is complete or correct. Exit codes are 0 for successful work,
1 for failures, 124 for deadline, and 128 plus the signal number for cancellation.
Read the final summary and process exit status. Truncated JSONL, or a missing
summary after setup, broken pipe or forced process termination, is not success.
The summary distinguishes completed, failed, interrupted and unattempted pages.

## Resource and cancellation contract

Defaults are one page worker, 200 DPI, a 120-second job deadline and **1,024 MiB
address space per worker/native process**. `--workers`, `--dpi`,
`--timeout-seconds` and `--memory-mib` are explicit operational controls.
`OMP_THREAD_LIMIT=1` and `OMP_NUM_THREADS=1` constrain Tesseract's OpenMP work.
There are no document-byte or page-count admission limits.

The page queue holds at most the selected number of workers. Each disposable
Python page worker renders one page, runs Tesseract, parses its TSV and writes
one staged JSON record. Native processes and TSV/JSON allocations inherit the
page worker's `RLIMIT_AS`; the controller copies 64 KiB output chunks and does
not deserialize whole page text. The controller's snapshot copy and metadata
inspection are bounded too. Workers/native children set `RLIMIT_CORE` to zero.
A large page can fail its resource budget; no
automatic downsampling or remote fallback hides that loss.

SIGINT, SIGTERM and deadline stop scheduling and kill the entire page process
group. Linux subreaper mode lets the controller reap orphaned native descendants,
as well as the immediate worker. Model loading happens again in each fresh
Tesseract process. Cleanup and bounded final diagnostic delivery can add time
after the job deadline; each final diagnostic delivery attempt has one second
of backpressure allowance, with at most two attempts (summary, then error).
OS-level uninterruptible filesystem operations
are outside a Python deadline guarantee. This is resource containment, not a
security sandbox for hostile native binaries.

The address-space limit is **not a total job RSS cap**: a page worker and its
active native child are separate processes, and multiple workers run concurrently.
Reported RSS maxima are separate process maxima, not a measured simultaneous
process-tree peak. Temporary disk needs the original snapshot, selected models
and active page rasters/TSV/results. Retained artifacts need additional disk;
filesystem exhaustion fails explicitly. There is no persistent service, UI
thread execution or production hosting implied by this command.

## Qualification and remaining work

`uv run pytest -q tests/test_cpu_ocr.py` runs real recognition of a generated
benign image, an image-only two-page PDF, single-frame TIFF and a blank image.
It verifies source preservation, word boxes, raw TSV, model identity, distinct
page provenance and two-worker completion. It also covers malformed inputs,
missing models, overwrite refusal, FIFO refusal, multi-frame TIFF refusal,
nonfinite confidence, memory failure, blocked output, timeout and SIGTERM.
Cancellation checks actual page/native descendant PIDs and verifies they vanish
after return; a separate process-tree test checks both timeout and cancellation.

The SQLite evaluation workstream records exact source/truth bytes, raw OCR,
layout, warnings, environment and first/repeated-process timing. Its database
is local evidence, not a committed user database. Cold/warm labels mean first
and repeated invocations on this VM; the OS cache is not flushed and models
are not resident between invocations. Keep this distinction in any speed claim.

Unfinished acceptance includes mixed-document native/OCR region routing,
verified PDF-to-raster transforms, language/orientation/complex-layout quality,
production dependency distribution, Rust/FFI integration, and browser/device
qualification. This implementation demonstrates CPU OCR independently of TPE's
still-gated supervised Docling integration. It provides no Safari WebGPU, CoreML,
Apple Neural Engine or measured iPhone speed claim.
