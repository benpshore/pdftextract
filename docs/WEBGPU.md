# WebGPU acceleration for browser OCR

Evidence recorded on 2026-10-05 from the code in `web/lib/webgpu/` and the tests in
`web/tests/webgpu-*.mjs`. This describes what is implemented and what was measured in headless
Chromium on this machine. It is **not** a claim that Safari, iOS or a real GPU has been tested.

## What runs where

The browser OCR path (`web/lib/image-ocr.ts`) uses **Tesseract.js 7.0.0** in a Web Worker with its
WASM core. Tesseract.js has no GPU execution provider, so WebGPU accelerates the **pre-processing**
of the working image, not the recognizer:

| Stage | WebGPU (WGSL compute, `shaders.ts`) | CPU fallback (`cpu.ts`) | Used for |
| --- | --- | --- | --- |
| RGBA → grayscale | `GRAYSCALE_WGSL`: one invocation per four pixels, BT.601 integer weights, packed 8-bit output | `grayscaleCpu` | The grayscale is written back to the working canvas and is what the recognizer receives |
| Otsu threshold | `HISTOGRAM_WGSL`: workgroup-shared `atomic<u32>` bins flushed with one global atomic add per bin; threshold from the histogram on the host (`otsuThreshold`) | `histogramCpu` + `otsuThreshold` | Defines "ink" pixels for skew estimation; reported in the result metadata |
| Projection-profile skew | `PROFILE_WGSL`: every sampled ink pixel is rotated about the image centre by each of 41 candidate angles (±5° in 0.25° steps) using 16.16 fixed-point sine/cosine supplied by the host, and its destination row is counted with `atomicAdd` | `profilesCpu` | `scoreProfiles` picks the angle whose row profile is sharpest; the working image is rotated back by that angle before recognition when the estimate is ≥ 0.25° and ≥ 1.05× better than no rotation |

Both paths use only integer arithmetic with the same formulas, so a GPU result is compared with
the CPU result for **equality**, not approximate agreement. The browser test asserts a maximum
absolute difference of 0 for the grayscale bytes, the 256 histogram bins, the threshold, every
projection profile and the chosen angle, on three fixtures (the committed `ocr-still.png`, canvas
text tilted by 2°, and 1023×517 colour noise, whose pixel count is not a multiple of four).

Pixels never leave the device: there is no remote call, no telemetry, and the only output besides
the result is one `console.info` line from the one-time timing comparison.

## Policy: on when available, demoted when it does not pay

`recognizeImage` reads a module-level flag, `setOcrAcceleration(mode)`, default `auto`:

| Mode | Behaviour |
| --- | --- |
| `auto` (default) | WebGPU when `detectWebGpu()` reports `available`; the first image of the session runs both paths, compares them and logs the timing. A mismatch, or a GPU time more than 2× the CPU time (typical for a software fallback adapter), switches the session to the CPU kernels. |
| `webgpu` | Keeps using the adapter after the timing verdict; still falls back to the CPU when WebGPU is unavailable, a stage fails, or the cross-check found a mismatch. |
| `cpu` | CPU kernels only. |
| `off` | No pre-processing; the recognizer receives the working image as drawn. |

`ocrAccelerationStatus()` returns one plain sentence for an accessible status line, for example
"WebGPU available (google swiftshader, software fallback adapter): image pre-processing for OCR
runs on the GPU; text recognition runs on the CPU (WASM). Nothing leaves this device." The result's
`engine` label says which pre-processing ran ("…; WebGPU pre-processing" or "…; CPU pre-processing")
and `metadata.ocr.preprocessing` records runtime, fallback reason, threshold, skew, timings, the
cross-check verdict and the status sentence. A deskew adds a warning naming the rotation.

The kernels are loaded with a dynamic `import('./webgpu/index')`, so a page whose module cannot
load (or the Node regression that transpiles `image-ocr.ts` alone) still recognizes text and
reports `preprocessing.runtime: 'none'` with the reason.

## Capability detection (`capability.ts`)

`detectWebGpu()` reports GPU failures as status; caller cancellation throws the signal reason. In order it checks:

1. `navigator.gpu` exists. It is `[SecureContext]`: `about:blank` and plain `http://` pages other
   than localhost do not expose it, and the reason says so.
2. `requestAdapter()`; when it returns `null` (blocklisted GPU, missing driver, policy), a second
   request with `forceFallbackAdapter: true` tries the software adapter and notes it.
3. Adapter identity from `adapter.info` (Safari 26, current Chrome) or the removed
   `requestAdapterInfo()` on older Chrome; `isFallbackAdapter` from either place.
4. Limits against the OCR working image (≤ 4,000,000 pixels): `maxStorageBufferBindingSize` and
   `maxBufferSize` ≥ 16 MB, `maxComputeWorkgroupsPerDimension` ≥ 15,625, workgroup size 256 and 1 KiB
   of workgroup storage. The WebGPU default limits (128 MiB, 256 MiB, 65,535, 256, 16 KiB) cover this;
   nothing above the defaults is requested, and no optional feature is requested.
5. The device is created once and shared; `device.lost` records the reason and the status becomes
   `lost` (CPU path) until the page reloads. Every stage runs inside `validation` and
   `out-of-memory` error scopes; an error becomes a `WebGpuStageError` and that image falls back to
   the CPU kernels. Buffers are destroyed in `finally`.

The status object also carries browser hints (`webkit`/`blink`/`gecko`, iOS, Safari, Chrome) for
the UI and for the notes below; they are derived from the user agent and are informational only.

## Browser support matrix

| Browser | WebGPU status | What this implementation does | Verified here |
| --- | --- | --- | --- |
| Safari 26 / iOS 26 / iPadOS 26 (WebKit) | Shipped (see `docs/MOBILE_OCR_READER.md` for the WebKit release evidence) | Default limits only, no optional features, `adapter.info` for identity, device recreated after loss (iOS releases GPU devices when a tab is backgrounded for long). Chrome and other iOS browsers run the same WebKit engine. | **No.** No Safari or iOS device was available. |
| Chrome 113+ desktop (Windows, macOS, ChromeOS), Chrome 121+ Android | Shipped | Hardware adapter when the GPU is not blocklisted; otherwise the SwiftShader fallback adapter, which the timing verdict usually demotes. | Only the Linux headless build below. |
| Chrome on Linux | Depends on the installed version and GPU allowlist; may need `--enable-unsafe-webgpu` | Same as above. | Headless Chromium 141 with `--enable-unsafe-webgpu --enable-features=Vulkan --use-angle=vulkan --use-vulkan=swiftshader` exposes a SwiftShader adapter (`vendor google, architecture swiftshader, isFallbackAdapter true`); without flags `navigator.gpu` exists on localhost but `requestAdapter()` returns `null`. |
| Firefox | Shipping progressively since Firefox 141 (Windows first) | Untested; detection follows the same path. | No. |
| Any browser without `navigator.gpu`, or on an insecure origin | — | CPU kernels; the status line names the reason. | Yes (Node, plain headless Chromium). |

## Fallback matrix

| Condition | Result |
| --- | --- |
| `navigator.gpu` missing or insecure context | CPU kernels; reason "navigator.gpu is not exposed…" or "…requires a secure context" |
| No adapter, no fallback adapter | CPU kernels; reason "no WebGPU adapter (GPU blocklisted, driver unsupported, or disabled by browser policy)" |
| Adapter limits below the working image | CPU kernels; reason names the limit and the bytes needed |
| `requestDevice` rejects | CPU kernels; reason "requestDevice failed: …" |
| Device lost during the session | CPU kernels for the rest of the session; status `lost` |
| Pipeline/buffer validation error or out of memory in a stage | CPU kernels for that image; reason "WebGPU stage failed: …" |
| First-image cross-check differs from the CPU reference | CPU kernels for the session in every mode; reason logged |
| GPU slower than 2× CPU on the first image | CPU kernels in `auto`; `webgpu` mode keeps the GPU |
| Canvas without `getImageData` (test shims, unusual embedders) | No pre-processing; `runtime: 'none'` |
| Module `./webgpu/index` fails to load | No pre-processing; `runtime: 'none'` |
| Abort signal | Rejects promptly with the exact signal reason during pending GPU waits; acquired buffers and abandoned devices are released |

## Measurements (headless Chromium 141, SwiftShader, 4-CPU shared machine, 2026-10-05)

`node tests/webgpu-browser.mjs`, one run, milliseconds including readback:

| Fixture | GPU grayscale / histogram / skew / total | CPU grayscale / histogram / skew / total | Equal |
| --- | --- | --- | --- |
| `ocr-still.png` 1500×340 | 126.9 / 192.2 / 32.1 / 351.2 | 4.3 / 2.4 / 12.8 / 19.5 | yes |
| canvas text 1200×700 tilted 2° | 24.2 / 137.1 / 48.0 / 209.3 | 3.8 / 2.2 / 10.7 / 16.7 | yes (estimate 2.00°, confidence 9.6) |
| colour noise 1023×517 | 13.6 / 96.8 / 225.1 / 335.5 | 2.3 / 0.8 / 74.7 / 77.8 | yes |

The one-time verdict on the first image was "135.0 ms against 26.9 ms on the CPU (software fallback
adapter); the CPU path is used for this session", which is the intended outcome for a software
adapter: SwiftShader emulates the GPU on the same four CPUs, and the first dispatch includes pipeline
compilation. These numbers say nothing about a hardware adapter; the same test on a device with a
real GPU is the required evidence before advertising a speed-up. With `setOcrAcceleration('webgpu')`
the end-to-end run (WebGPU pre-processing, deskew by 2°, Tesseract recognition) took 1.3 s and
recognized all five lines and the DOI.

## Limits and known gaps

- The working image is at most 4,000,000 pixels / 4096 px per side (unchanged); the kernels check
  the adapter limits for exactly that size.
- Skew search is ±5° in 0.25° steps on at most 1,000,000 sampled pixels; larger tilts are not
  corrected. Deskew rotates about the centre and may clip corners; the result warns about it.
- Pre-processing runs on the main thread (the OCR worker is Tesseract's own). Safari's WebGPU in
  workers was not relied on.
- The GPU is slower than the CPU for these kernels when the adapter is software, and the timing
  verdict handles that. Whether a hardware adapter is faster than the ~20 ms CPU path for a
  1500×340 image is unmeasured; the realistic gain is on 4-megapixel photographs on phones.
- WebNN and the Apple Neural Engine are not used or claimed (see `docs/MOBILE_OCR_READER.md`).

## Tests

From `web/`:

```sh
node tests/webgpu-cancellation.mjs # Node: pending-wait abort, deadline and resource cleanup
node tests/webgpu-cpu.mjs        # Node: CPU kernels, shared maths, detection without WebGPU, flag/status line
node scripts/copy-ocr-assets.mjs # once, for the end-to-end OCR check in the browser test
PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers PLAYWRIGHT_MODULE=/opt/node22/lib/node_modules/playwright/index.mjs \
  node tests/webgpu-browser.mjs  # headless Chromium: GPU vs CPU equality, verdict, status line, OCR with deskew
```

The browser test aborts every request that is not same-origin. If the headless build exposes no
adapter it still verifies detection and the fallback and says so in `conclusion`.

## PR #243 cancellation correction (2026-10-07)

The full preprocessing call has one **10-second maximum WebGPU wait budget**. Callers may
shorten it with `preprocessForOcr(..., { timeoutMs })`; they cannot disable it. Adapter discovery
(including fallback and legacy adapter-info requests), device acquisition, asynchronous pipeline
creation, each readback `mapAsync`, and error-scope completion all use the same deadline and
abort signal. Timeout reports the stalled stage and returns CPU fallback; cancellation throws
the caller's exact reason without running the CPU fallback.

WebGPU does not natively cancel these requests. Every abandoned promise has a rejection handler.
A late device is destroyed instead of cached; reset/teardown invalidates in-flight acquisitions,
and an old device's loss callback cannot evict a newer device. Pending acquisition belongs to
its caller, so one cancellation cannot cancel another caller. Concurrent successful acquisitions
retain one cached session device and destroy duplicates. The normal cached device remains owned
by the session until `releaseWebGpuDevice()`; cancellation of one image releases that image's
buffers, including staging buffers, without destroying another caller's shared device. Pipeline
cache entries abandoned by abort/timeout are evicted. Error scopes are popped before asynchronous
cleanup, and a stalled cleanup cannot delay abort or deadline return. Allocations enter their
cleanup registry immediately, including allocations preceding a later synchronous failure.

`node tests/webgpu-cancellation.mjs` runs synthetic pending-promise and late-settlement lifecycle
regressions in Node. The browser suite runs the same fault body **before** adapter detection so
these checks cannot be skipped on a GPU-less browser. `WEBGPU_REQUIRE_GPU=1` additionally requires
real GPU kernel execution and end-to-end OCR; the Web alpha workflow sets this for its pinned
Chromium/SwiftShader check. A missing adapter or OCR assets fails that gate. Existing integer
parity assertions (tolerance zero) and OCR text assertions remain unchanged.

**Separate landing question: OCR quality and default-on policy remain unresolved.** Current code
still defaults to `auto`, including CPU preprocessing on browsers without WebGPU. Integer parity
between the GPU and CPU kernels, lifecycle fault injection, and one synthetic OCR fixture do not
establish quality parity against the previous unprocessed recognizer across representative
images. This correction grants no default-on approval. Keep the PR draft pending independent
review, representative OCR-quality evidence, and resolution of that policy; hardware GPU and
Safari/iOS validation remain outstanding.

Correction validation: `docs/validation/pr243-webgpu-waits.json` records 88 lifecycle assertions
in Node and Chromium 151.0.7922.34, 39 CPU checks, and all 19 original browser checks with
actual SwiftShader GPU execution and OCR. Four separate original-head controls fail after
the 1-second abort watchdog. These are implementation-author checks, not independent approval.
