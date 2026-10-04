# Region-fusion fixtures and adjudicated evidence

These synthetic fixtures establish one narrow, real capability difference: **PDFium recovers a correctly encoded text region that the current baseline cannot decode, while the baseline retains a separate good region on the same page.** The experiment uses no private documents, OCR models, parser changes or broad alternative-engine evaluation.

The selection witness supplies adjudicated truth as input. A successful selection therefore demonstrates **review-assisted functional projection**, not automatic correctness discovery or accuracy on held-out truth.

## Actual source and runtime evidence

Sources and captures are under [`experiments/region-fusion/fixtures`](../experiments/region-fusion/fixtures). The executable was the existing `/workspace/targets/root/debug/tpe`, reporting `tpe git-e27e1fb`, SHA-256 `2674dbc4712d65b7f41ddee341086fb3ee1be8a5497fd4c5b18c07272b5ec498`. The parent identified it as built from unchanged main `e27e1fb28a5b40dbca7f517c6fe396e6f11ed4ec` with main's lock and PDFium enabled; this fixture task did not rebuild it.

The actual native runtime was `chromium/8066`, Linux x64, at `/workspace/runtimes/pdfium-8066/lib/libpdfium.so`, SHA-256 `7670b3c597b02dfa3f98b23b49c3bb52536312f1ea686b739321731b6011f5a9`. Runtime backend identity reports `pdfium` / `dynamic-binding-0.8.37`; the baseline reports `lopdf` / `0.45.0`. Full identities, commands, exit codes, source hashes, raw artifact hashes, renderer versions and render hashes are in the [capture manifest](../experiments/region-fusion/fixtures/captured/manifest.json). The raw JSON/text/stdout/stderr files remain unchanged. Temporary SQLite ledgers were removed after each run.

| Actual generated fixture | Baseline observation | PDFium observation | Purpose |
| --- | --- | --- | --- |
| [`positive-stream-cmap.pdf`](../experiments/region-fusion/fixtures/positive-stream-cmap.pdf) | Good neighbor correct; target is 18 U+FFFD characters; `Partial`, exit 1, explicit undecodable-font warning. | Both source strings exact; `Complete`, exit 0, no warnings. | Positive region substitution with useful baseline retained. |
| [`control-named-cmap.pdf`](../experiments/region-fusion/fixtures/control-named-cmap.pdf) | Both strings exact; `Complete`, exit 0. | Both strings exact; `Complete`, exit 0. | Same glyphs/Unicode mapping with named `Identity-H`; no justified text improvement. |
| [`negative-empty-tounicode.pdf`](../experiments/region-fusion/fixtures/negative-empty-tounicode.pdf) | Good neighbor correct; target is 18 U+FFFD; `Partial`, exit 1. | Good neighbor correct; target is character-code echoes, with 18 Unicode-map errors; `Partial`, exit 1. | A nonempty candidate is not evidence of recovery. No replacement approval is supplied. |
| [`repeated-stream-cmap.pdf`](../experiments/region-fusion/fixtures/repeated-stream-cmap.pdf) | Good neighbor correct; two distinct target placements become replacement characters. | Both target placements decode correctly. | Text alone cannot establish placement. The fixture has two identical strings at different coordinates; duplicate same-position mutations test ambiguity separately. |

These are observed backend runs. The negative mutation plans described below are separate from those observations; their intended selector outcomes are not recorded as passed unless the region-fusion implementation actually executes them.

## Why the positive fixture differs

The [deterministic generator](../experiments/region-fusion/fixtures/generate.py) writes PDF primitives directly with Python's standard library. It declares the source strings before extraction:

```text
BASELINE REGION RETAINS THIS TEXT.
RECOVER ALPHA 2026
```

The good neighbor uses Helvetica. The target uses an embedded licensed DejaVu Sans subset, a compact complete `ToUnicode` `bfchar` mapping, and a custom **stream** encoding CMap with an identity `cidrange`. The baseline's existing `composite_decode` accepts supported named encodings but treats this stream encoding shape as unusable, retaining replacement characters and its region box. PDFium interprets both CMaps correctly. The control changes only the target font's `/Encoding` reference to named `/Identity-H`; both engines then return the declared target string. No malformed CMap or deliberately incorrect Unicode truth is needed for the positive case.

The font subset and notice are in [`font/`](../experiments/region-fusion/fixtures/font); [`font-source.json`](../experiments/region-fusion/fixtures/font/font-source.json) records original/subset hashes, glyph IDs, widths and subsetting tool version. The generator checks the subset hash. Regeneration needs no system font, model, native renderer or third-party Python package. The PDFs were regenerated and their bytes stayed identical; see [verification.json](../experiments/region-fusion/fixtures/verification.json).

| Fixture | PDF SHA-256 |
| --- | --- |
| Positive | `97779d74bc9f286de6f4578ce86f47a266d7f263a28ce37b955b61b7333f2a1f` |
| Named encoding control | `ce061489c99fb500e1663bb9b3afa85196cec3306c96108762f48e49a054d324` |
| Empty Unicode map negative | `ef40d5b45f720ff59a633b5690a407516a8e9f51eeb46baf277b8557132de46b` |
| Repeated target | `43086e368ca75ba07e762048726dd46d8f6e6b97a71606f8840b780baccfd881` |

## Source truth, coordinates and visual review

Each `*.truth.json` is emitted from the generator's declarations before extraction. The positive's [truth record](../experiments/region-fusion/fixtures/positive-stream-cmap.truth.json) fixes page 1, 612 × 792 points, `MediaBox = CropBox = [0,0,612,792]`, rotation 0, bottom-left origin, x right and y up.

| Source region | Text baseline, points | Authored enclosing region `[x0,y0,x1,y1]` |
| --- | --- | --- |
| Good neighbor | `[50,722]`, 14 pt | `[40,708,330,742]` |
| Recovery target | `[50,622]`, 22 pt | `[40,608,315,650]` |
| Repeated target, repeated fixture only | `[50,522]`, 22 pt | `[40,508,315,550]` |

These authored regions enclose the intended runs; they are not claimed to be exact glyph ink bounds. Parser boxes differ, particularly for the Helvetica neighbor. Preserving that baseline span means preserving its actual text, geometry and provenance unchanged; this example does not independently certify the baseline's glyph geometry.

The capture utility generated 1224 × 1584 images at two pixels per point, with a top-left raster origin:

- [Positive MuPDF render](../experiments/region-fusion/fixtures/captured/positive-stream-cmap/page-1-mupdf.png), PyMuPDF **1.26.6** / MuPDF **1.26.11**, SHA-256 `3164d458097f50a137f5b555915c60dd7258d26b9ab97fab008e0781ce18c85d`, 22,306 bytes.
- [Positive PDFium render](../experiments/region-fusion/fixtures/captured/positive-stream-cmap/page-1-pdfium.png), pinned **8066**, SHA-256 `3393417e193601a57936304e97db75480492e8cc2f5dbf5cef863dc4fe65e665`, 27,809 bytes.

Both visibly show the exact two declared strings with clear vertical separation. The source/generator reviewer, a separate design reviewer, and the parent inspected the images; their recorded observations are in [visual-review.json](../experiments/region-fusion/fixtures/visual-review.json). These were **AI visual reviews with expected strings supplied beforehand**, not blind annotations or human judgments. PDFium rendering shares an engine with the candidate; MuPDF supplies a separate-engine raster view. Neither rendering alone supplies an automatic general-purpose text oracle.

## Approved witness and adversarial cases

[`approved-review.json`](../experiments/region-fusion/fixtures/approved-review.json) contains one explicit source-bound review for `recovery-target`. Its externally approved SHA-256 is:

```text
926b144c886d90734390df8e951026cc0115610fdb68fb28660ddea7cd48889b
```

The implementation's test/operator policy must supply that digest independently; computing the expected digest from an arbitrary received review file is self-approval. The digest verifies reviewed bytes, not reviewer identity or cryptographic authorship. The witness records source hash/size, page/frame/region, the observed baseline regression string, source-declared candidate/adjudicated text, MuPDF raster hash/size, renderer configuration digest and review provenance. Candidate version is optional; full actual backend/configuration/artifact identities belong in the run receipt. No span index, runtime library path, or timing-dependent extraction artifact hash is preselected by the witness.

The good neighbor has no replacement witness. The matcher must discover exactly one applicable baseline/candidate correspondence geometrically; a correct target string elsewhere on the page does not qualify. The recovered view is a separate projection. Original baseline warnings, statuses, artifacts and useful neighboring spans must remain available rather than being rewritten into a claim of complete extraction.

[`adversarial-mutations.json`](../experiments/region-fusion/fixtures/adversarial-mutations.json) lists explicit synthetic mutations of captured evidence, each distinguished from a parser result:

- Equal-length wrong target `RECOVER ALPHA 2025`, a longer invented suffix, and a partial target.
- Duplicate identical or conflicting candidate spans at the same position, and a box crossing the good neighbor.
- Wrong source, page or unknown frame, altered render bytes, and an untrusted review digest.
- Permuted candidate order, which must not cause length/order-based acceptance.

These cases require rejection or conservative abstention with the baseline retained. An attempt with one legitimate and one overlapping duplicate must not become acceptable merely because one string matches the supplied truth.

## Reproduction and limits

Generate the same source bytes:

```sh
python3 experiments/region-fusion/fixtures/generate.py
```

The existing capture is intentionally immutable. For a fresh capture, use an isolated copy of the fixture directory with its `captured` directory omitted, then run:

```sh
python3 experiments/region-fusion/fixtures/capture.py \
  --binary /absolute/path/to/tpe \
  --pdfium-library /absolute/path/to/libpdfium.so
```

The capture script refuses to overwrite existing captures and checks the exact reviewed Linux x64 library hash. Rendering uses existing PyMuPDF/Pillow installations; if dependencies must be provisioned, follow repository policy and use `uv`, not pip. The PDFium C-API helper uses only fixed synthetic fixtures, fixed raster dimensions, explicit signatures, null checks and owned-handle cleanup. This is not a general user-document rendering API.

Actual source/capture consistency checks passed for all four fixtures, including raw artifact and render hashes and exact good-neighbor text. PDF generation was byte-identical on rerun. No OCR model was provisioned. No performance, large-document, multilingual, automatic adjudication, generalized fusion accuracy or whole-document correctness claim follows from this small controlled set. Initial unsuitable exploratory probes were moved outside the deliverable; the checked-in fixture set contains the frozen positive, useful controls, source truth and explicitly labeled evidence.
