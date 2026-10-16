# PDF Oxide 0.3.78: rendering, barcodes and offline OCR

This record describes adapter behavior and source review. It is not a claim of
rendering fidelity, OCR accuracy, complete toolkit parity, or sandbox assurance.
Exact combined-source test results belong in the integration owner's report.

The adapter uses the pinned registry source, `pdf_oxide =0.3.78`, corresponding to
upstream commit `ad49c4cb3638dc950a29350ef882161372fde473`. The optional features
are separate: `pdf-oxide-rendering`, `pdf-oxide-barcodes`, and `pdf-oxide-ocr`.
The last enables the pure Rust `ocr-tract` backend; it does not enable upstream
`ocr`, ONNX Runtime, model acquisition, GPU, or `ml` dependencies.

| Operation | Actual upstream API | Contract and qualification |
| --- | --- | --- |
| `render.pages` | `rendering::render_page` | Selected pages to owned PNG, JPEG, or premultiplied RGBA bytes; source SHA-256 and one-based page identity. Requires verified worker filesystem containment. |
| `render.fit` | `rendering::render_page_fit` | Bounded width/height; pinned implementation derives scale from MediaBox and clips to CropBox. Returned dimensions must match preflight. |
| `render.region` | `rendering::render_page_region` | PNG/JPEG crop `[x,y,width,height]` in PDF points. Pinned crop helper assumes an unrotated zero-origin page without an effective CropBox; other cases return Unsupported. Full raster and cropped output count against the pixel budget. |
| `render.separations` | `render_separation`, `render_separations` | One requested ink or bounded process/spot plates. Memory-only PGM output, 0=no ink and 255=full tint. No claim of press/color fidelity. Unsupported format/annotation/transparency options are rejected. |
| `render.flatten` | Page renderer plus `PdfWriter::add_page`, `ImageContent`, `finish` | Builds a raster-only PDF in memory, retaining the displayed page dimensions and source-to-derivative page mapping. Original bytes are untouched. Text, tags, links and editability are not retained; this is not verified redaction or PDF/A. |
| `barcode.generate` | `BarcodeGenerator::generate_qr[_svg]`, `generate_1d[_svg]`, `Pdf::from_image_bytes` | QR, Code128, Code39, EAN13, EAN8, UPC-A, ITF, Code93 and Codabar. PNG/SVG/PDF output. No network fetch when payload happens to contain a URL. Human-readable captions are rejected because upstream has no implementation. No scanner-readability claim. |
| `ocr.classify` | `PdfDocument::classify_page` | Heuristic page classification without OCR or rendering; typed upstream signals/reasons are preserved alongside one-based source page IDs. Empty/image-free/recognized completeness is not inferred. |
| `ocr.images` | `OcrEngine::from_bytes`, `page_image_handles`, `PdfImageHandle::decode`, `detector().detect`, `crop_text_region`, `recognizer().recognize` | Explicit offline model and dictionary bytes only. Recognizes bounded painted images and returns image-pixel polygons, confidence values and source/page/paint-order provenance. Native extraction is not overwritten. Requires verified worker filesystem containment. |
| `ml.capabilities` | `SmartLayoutAnalyzer::capabilities` | Reports actual false ML reading-order/heading/model-loaded flags. Pinned implementation removed these ML methods; enabling `ml` does not restore them. |

## Bounds and retained uncertainty

All operations use the existing dispatcher/supervisor limits: at most eight
inputs and 64 MiB total input, at most 256 selected pages, bounded item count,
16 million total pixels, and at most 64 MiB serialized output (callers may lower
limits). Rendering additionally accepts DPI 1–600 and dimensions 1–8192. The
adapter rejects over-budget geometry instead of accepting the renderer's silent
resolution reduction. Selected pages are validated before bounded collection.
All artifacts have application-generated names; ink names and document metadata
cannot select output paths. No operation in this module writes its own artifacts
to disk.

The upstream renderer unconditionally initializes its system font database.
It can read host font paths even for vector-only pages. The adapter therefore
fails closed outside the verified worker filesystem boundary. On this VM,
Landlock reports `ENOSYS`; the separately identified `seccomp_sealed` worker
mode denies all filesystem paths and synchronizes its policy across threads.
It is not a Landlock fallback or authorization to read system fonts. The renderer's
warnings and font substitutions/drops still make successful output partial.
Its last-resort font path draws rectangle placeholders for non-space characters;
it does not provide a usable built-in Standard14 raster font. That missing-font
path logs a warning but does not reliably emit a structured record, so the adapter
adds an explicit warning even when the upstream diagnostic array is empty.
No upstream flatten helper is called: `rendering::flatten_to_images` creates
predictable `pdf_oxide_flatten_PID` directories and accumulates file paths.

Separation rendering has content/color-management limitations documented in the
pinned source, including paths involving RGB/Gray/Indexed images, patterns,
inline images, annotations and some color transforms. The adapter carries an
explicit warning rather than presenting these plates as production prepress
verification. Different pinned source comments describe different transparency
paths, so those comments are not treated as an empirical guarantee.

OCR request input slots are PDF, detection ONNX, recognition ONNX, and UTF-8
dictionary. Expected `detection_sha256`, `recognition_sha256`, and
`dictionary_sha256` are mandatory and checked before model parsing. Each model
is bounded by the total input limit; the dictionary is additionally capped at
1 MiB and 65,536 lines with exactly one Unicode character per line. These are
caller-pinned identity checks, not approved-model attestations. No trained OCR
model or recognition accuracy fixture is bundled. Test-only ONNX graphs contain
small deterministic tensors to exercise offline plumbing; they do not recognize
image content and are not production OCR models.

OCR processes images and detected regions sequentially, with at most 1,024
configured detection candidates and a 32–2,048 maximum detection side. The
adapter sets `num_threads=1`, but upstream honors that setting only for its
native backend; pinned tract 0.22.5 independently defaults to a single-thread
executor. This setting is not presented as a process/thread security boundary.
Decoded images, padded detector inputs,
recognition crops, and padded recognizer inputs share the pixel budget. A thin
crop can expand significantly when resized to the recognizer's 48-pixel height,
so its resized width is checked before recognition preprocessing. Public
detector and recognizer stages preserve the pinned crop/CTC behavior without
using its unbounded high-level recognition helper. Image handles are enumerated
before decode so count/dimension/
compressed-size checks apply to inline and XObject images; decoded dimensions
are checked again. Upstream parsing and graph/image decoding can allocate
before these local checks, so hard worker memory/time limits remain necessary.
Inference graphs are untrusted inputs, not made safe by their SHA-256 values.
Graph internals remain subject to hard worker memory/time limits. Returned
coordinates must be finite; confidence values must also be within 0–1.

OCR outputs preserve image coordinates instead of assuming scan DPI yields
correct PDF geometry. It does not silently deduplicate repeated images, merge
native text, or select only the largest image and call it a full-page result.
No images reported and no recognition spans are separate partial/uncertain
conditions. `ocr.classify` never invokes `AutoExtractor` or model acquisition.
Model loading, cancellation, malformed graphs and resource limits must be
qualified in disposable workers; missing models/build features are errors.

Barcode generation bounds the intermediate unscaled bars as well as requested
output dimensions. One-dimensional content must be ASCII, avoiding upstream
fixed-byte slicing hazards for numeric barcode paths. Supplied EAN13/EAN8/UPC-A
check digits are validated before body-only data reaches upstream; invalid
complete codes are never silently repaired. SVG alpha is rejected because the
pinned QR SVG encoder ignores it. SVG dimensions follow
upstream module sizing and PDF output follows upstream page-fit layout. The
payload is encoded as bars/modules and is not interpolated as executable SVG.

## Source anchors

- [Pinned rendering entry points and disk-based flatten helper](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/rendering/mod.rs)
- [Pinned rendering options and automatic downscaling](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/rendering/page_renderer.rs)
- [Pinned font database initialization](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/rendering/text_rasterizer.rs)
- [Pinned separation renderer](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/rendering/separation_renderer.rs)
- [Pinned OCR byte constructor and image inference](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/ocr/engine.rs)
- [Pinned OCR inference backend selection](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/ocr/backend.rs)
- [Pinned image handles](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/extractors/images.rs)
- [Pinned ML capability implementation](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/hybrid/smart_analyzer.rs)
- [Pinned barcode implementation](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/writer/barcode.rs)

## Verification record

`tests/oxide_render_ocr.rs` covers source/page identity, malformed PDFs, bounded
selection, unavailable build/model errors, unconfined font access refusal,
caller model hash mismatch, generated PNG dimensions, SVG payload isolation,
barcode PDF reopening, unimplemented captions, numeric multibyte rejection, and
intermediate/output bounds. Render dimension/counter unit tests check invalid
geometry and counter overflow. These tests must be executed against the exact
combined branch; their presence alone is not evidence that they passed.

Successful confined raster/image assertions, real model recognition, visual
source-to-derivative review, cancellation/resource failures, and cross-platform
execution must be recorded separately as completed or blocked. Do not substitute
unconfined host rendering or a missing-model error for successful OCR evidence.

The integration owner's `full-tests-06.log` records 10 passing tests in this
target, including the parent test that actually starts the sealed raster child,
all eight one-dimensional barcode encoders, numeric checksum rejection, and
baseline classification/feature errors. That run predates the synthetic ONNX
test and the extra OCR preprocessing caps. Other targets in the same combined
run failed; this module's pass is not an overall integration pass.

The subsequent deterministic ONNX test constructs two tiny graphs using a
documented protobuf subset from the [official ONNX schema](https://github.com/onnx/onnx/blob/v1.14.1/onnx/onnx.proto).
Both consume input `x` and compute `ReduceSum(x) * 0 + constant`. The detector
returns one rectangle in a 32×32 probability map; the recognizer returns CTC
scores that decode to `AB`, exercising duplicate and blank removal. A synthetic
PDF paints the same 32×32 image twice to check distinct paint-order provenance.
The probe covers identity preservation, output shape failures, no detections,
no reported images, and pixel/item/output caps. It runs only in a sealed child
with at most 1 GiB address space, 10 CPU seconds, no core dumps, and a 20-second
parent deadline followed by kill/reap. Test source alone is not passing evidence;
execution results are recorded separately below.

`ocr-combined-tests-01.log` subsequently records **15 passed, 0 failed** in this
target, including the parent that executes the deterministic ONNX child,
malformed-model child, and sealed raster child. That compiled the updated OCR
preprocessing bounds. The final 1024×1 thin-image regression was added after
that executable was built and still needs the next exact-source run. The
positive result establishes byte-only inference and adapter plumbing for these
tiny graphs, without recognition-accuracy evidence for trained OCR models.

An independent visual check of the creation owner's text-only HTML fixture is
preserved at `/workspace/tpe-pdf-oxide-evidence-20261015/render-ocr-visual/html-css-text-only-001`.
The exact input is `<p>HTML source text</p>` with 18px supplied DejaVu Sans; the
font and its notice were copied unchanged from the pinned registry package.
Poppler 26.05.0 at 144 DPI shows one readable line with no visible clipping or
overlap. Its `pdfinfo` reports **794×1123 points**, exposing an upstream CSS
pixel/PDF-point geometry mismatch rather than qualifying A4 dimensions.
The executable snapshot is SHA-256
`5bbf6a72f831f54b9a862a97b7cac4110a5c6519f3dd35cc4c32c6388abdb2f6`
(`git-b3730aa-dirty`); the produced PDF is
`2760581c208d502d68aa32bfd8a593bbffb77d747a3a7fe6345fa71559e5ef4f`.
This evidence does not represent the subsequent exact combined-source rebuild,
complex HTML/CSS fidelity, or native parity.
