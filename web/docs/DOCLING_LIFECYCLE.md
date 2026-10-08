# Browser lifecycle and longer scanned documents

This extends the [initial memory assessment](DOCLING_MEMORY.md), using the
existing CPU/WASM API and pinned models. It does not establish a phone memory
threshold or iOS readiness.

## Concrete retention fix

The API kept completed job handles for lookup. Their methods shared a closure
scope with the result finalizer, which captured the original options object.
Clearing the progress-listener Set therefore did **not** release
`options.onProgress` and its arbitrary caller-owned captures. A separate
4 MiB payload object stayed reachable after the job completed, model disposal,
and diagnostic garbage collection in the
[before-fix probe](evidence/docling-observer-before.json). This is object
reachability evidence, not a claim that all four MiB were resident.

Completion now clears the options reference after detaching the abort relay.
Subscriptions added after completion are not retained. The
[fixed lifecycle evidence](evidence/docling-lifecycle-fixed.json) shows both
payloads collectible; all 47 lifecycle assertions pass. Existing active
progress and cancellation still run through the same API.

The lookup retains at most eight job handles and their byte-limited results.
This is intentional ownership, separate from model disposal. Callers can retain
their own results indefinitely. JSON size does not measure JS object overhead.

## Method and controls

`scripts/test-docling-lifecycle.mjs` rasterizes the checksum-verified native
control, crops its three text lines, and places five copies per letter page at
their original readable size. Each page has a separate JPEG, with alternating
x offsets and a tiny non-text mark to prevent identical-image reuse. These are
controlled synthetic scans, not a real-world scholarly OCR accuracy sample.
The expected text is fixed independently before extraction. Fixture hashes,
raw returned text/layout, tensor fingerprints, routing and diagnostics are in
the evidence.

One owned Chromium 151 process tree runs 1-, 8- and 12-page documents, then
three extract/cancel-on-page-three/retry/dispose cycles. The host is Linux x86-64,
AMD EPYC 9V74, five visible logical CPUs; ONNX uses one CPU/WASM thread. Process
RSS/PSS is sampled every 100 ms. Page-completion observations use the nearest
process sample, not an atomic sample at the callback. Browser startup and
fixture generation precede the measured operations.

A test-only worker probe observes both WASM capacities and holds **WeakRefs**
to incoming page RGBA buffers. After selected jobs, the harness waits 1.5 s,
records natural settling, then separately requests diagnostic V8 GC in the
main context and workers. Production never exposes or requests that GC. It is
not included in extraction time and is not a proposed memory-control strategy.
RSS/PSS, reserved linear capacity and reachability are different measurements;
they must not be added together.

## Observed length behavior

| Pages | Total seconds | Peak RSS MiB | Peak PSS MiB | Settled PSS before / after diagnostic GC, MiB | Returned JSON bytes |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 11.58 | 1283.2 | 1023.1 | 992.3 / 944.0 | 15,824 |
| 8 | 64.68 | 1395.2 | 1133.8 | 990.1 / 944.2 | 113,445 |
| 12 | 97.43 | 1411.4 | 1149.9 | 1005.6 / 957.1 | 168,647 |

The first job loads sessions; later jobs reuse them. Docling capacity is
64.4375 MiB and ORT capacity 480.125 MiB throughout these dense-page runs.
The denser recognition workload needs more ORT capacity than the earlier
three-line scan (400.1 MiB); page count does not multiply the model heaps.
The transferred letter-page RGBA buffer is about 7.4 MiB. One page buffer
remained reachable before each diagnostic collection; **zero** remained after,
including all 21 observed buffers after the 12-page job. No accumulating
per-page RGBA ownership was found in this controlled run.

The roughly 13 MiB post-GC PSS difference at 12 pages is not explained by a
capacity-only probe. It may include touched pages within existing heaps,
allocator/cache state and retained output overhead. These observations do not
prove that arbitrary inputs cannot cause retained growth, and are not a hard
browser memory bound.

A prior [16-page attempt](evidence/docling-lifecycle-timeout.json) hit the
existing 120-second limit after 14 completed pages. It returned `TIMEOUT`,
preserved those pages and terminated the worker. Its peak was 1401.6 MiB RSS /
1140.3 MiB PSS. The measurement harness stopped on its expected-completion
assertion; the artifact records that failure. The timeout was not increased
to make the workload pass.

## Repeated interruption and disposal

All three cycles cancelled during real model inference on page three, retained
two completed pages and terminated the model owner. Every subsequent one-page
retry completed (10.44, 10.25 and 10.82 seconds). Each explicit disposal left no
model worker. Settled process-tree PSS before diagnostic GC was 387.6, 390.7 and
392.9 MiB; after GC it was 386.1, 389.9 and 391.9 MiB. Browser/test infrastructure
remains. This small residual drift over three cycles is measured, not declared
zero or extrapolated into an unlimited-run guarantee.

## OCR and compatibility limits remain visible

The dense control exposed extra recognized text in the first block of some
pages: pages 1 and 3 in the eight-page document, and 1, 3 and 9 in the twelve-page
document failed exact expected-text comparison. The harness records the fixed
expectation, returned text and mismatched page numbers while testing lifecycle.
**47 passing lifecycle assertions are not 47 OCR accuracy passes.** There is no
text cleanup or deduplication in this experiment. The earlier native/scanned,
mixed-region and five column-order witnesses remain separate acceptance gates.
All outputs retain incomplete-coverage diagnostics.

Local desktop WebKit provisioning failed with HTTP 403 from all configured
Playwright mirrors. The CI workflow attempts actual desktop WebKit extraction,
cancellation, retry and disposal as a separately labelled compatibility
observation, preserving any failure. Its step is experimental and is allowed
to fail without hiding the recorded result. Desktop WebKit on a Linux CI host
does not qualify Safari on macOS or iOS hardware. Phones remain unqualified.

## Reproduce

Stage the pinned assets and test dependencies using the
[API guide](DOCLING_PDF_API.md), then from `web`:

```sh
PLAYWRIGHT_MODULE=./browser-tools/node_modules/playwright/index.mjs \
PDF_LIB_MODULE=./browser-tools/node_modules/pdf-lib/cjs/index.js \
EVIDENCE_PATH=/tmp/lifecycle.json node scripts/test-docling-lifecycle.mjs
```

`LIFECYCLE_COUNTS=1,8,12` and `LIFECYCLE_CYCLES=3` are the defaults. `16` reproduces
the longer time-budget attempt; whether it times out depends on the host.
`BROWSER=webkit LIFECYCLE_COUNTS=1 LIFECYCLE_CYCLES=1` runs the additional browser
observation if WebKit and its platform dependencies are installed. Forced-GC
reachability assertions are skipped when that browser does not expose GC;
the evidence says whether collection actually occurred.
