# Local CPU runtime qualification

This work is scoped by [OCR/evaluation issue #197](https://github.com/benpshore/pdftextract/issues/197)
under the dedicated [OCR-PDF epic #199](https://github.com/benpshore/pdftextract/issues/199)
and epics #180/#182, and [GROBID issue #198](https://github.com/benpshore/pdftextract/issues/198)
under #182. It starts from PR #175 at `baafb472873750e7e84d32c3c8e6fa7a680e37db`.
The runtime tools and evaluation database run on the Codex Cloud Linux executor.
They are separate from the browser import path and production service hosting.

## Evidence and reproduction

- [CPU OCR](CPU_OCR.md): explicit local Tesseract/Poppler processing, immutable
  input/model snapshots, page/raster word regions, partial results, process limits,
  deadline and cancellation behavior.
- [SQLite evaluation](CPU_OCR_EVAL.md): dedicated normalized evidence database,
  authored scanned fixtures and a pinned subset of the published Tesseract test
  corpus, retained originals/truth/raw outputs/errors, accuracy
  denominators, measured timing and resource methods. The database is retained
  outside git because `AGENTS.md` forbids committing databases.
- [CPU GROBID](GROBID_CPU_RUNTIME.md): digest-pinned loopback CRF service,
  authored scholarly fixture, raw TEI and exact runtime identity, finite server
  limits and explicit client integration.
- [Docling and acceleration](OCR_RUNTIME_FEASIBILITY.md): exact upstream crate
  source, precompiled runtime/model identities, native/WASM boundaries, and
  primary-source feasibility evidence for CPU, GPU and CoreML execution.

## Ownership and interpretation

The Rust dependency manifests, lockfile, toolchain pin, PDF parser/FFI sources and
web files are unchanged by this work. Toolchain coordination is recorded under
[#196](https://github.com/benpshore/pdftextract/issues/196) and #153. The existing
PR #175 readiness/bibliography and web integration owners retain those files.
One narrow change to `src/grobid.rs` follows the actual CPU service response:
GROBID 0.9.1 returns empty `coords` attributes for three citation author names.
The client now retains those attributes and raw TEI with an explicit unavailable
geometry warning and no inferred boxes. Nonempty malformed coordinate lists
remain errors. The captured response and positive/negative regressions are
retained; the diagnosis and source ownership are recorded under #198.
An upstream Docling binary test is not a test of TPE's full `docling` adapter;
`docling-text` parses existing PDF text and does not perform OCR.

OCR text is inferred, with actual raster geometry and retained warnings. No
unverified PDF-coordinate transform, native font identity, semantic confidence,
or completeness guarantee is manufactured. GROBID emits a scholarly semantic
projection; its successful HTTP response is not exhaustive PDF extraction and
does not make it OCR.

The evaluator records the observed executor allocation rather than inferring
capacity from a product name: five visible logical CPUs, a four-CPU cgroup quota
(`400000 100000`), and a 16 GiB memory limit in this session. First/repeat request
timings describe their stated cache/process conditions, not reboot-cold hardware
or controlled iPhone measurements. Heavy evaluation is local to Codex Cloud;
the added ordinary test suite consists of lightweight regressions.

Safari WebGPU targets GPU execution. Native CoreML's selectable compute units
are a separate route and do not demonstrate a Safari-to-Apple-Neural-Engine
bridge. Browser/runtime/model compatibility and actual iPhone/iPad performance
remain separate acceptance gates.

No user documents, medical records, production databases or credentials are used.
No Site deployment, public service endpoint, merge or release is performed.
Ben's future Python dewarping/warp-skew program is not available in this workstream;
its integration remains separate work after OCR validation. No substitute
dewarping algorithm was invented.

## Published corpus selection

The [pinned public-corpus manifest](validation/cpu-ocr-public-corpus.json) selects
the clean `phototest.tif` and its published left-rotated PNG from the official
[Tesseract test repository](https://github.com/tesseract-ocr/test/tree/232ff181c66516116ec0e84c4963f70de15050fd).
Its repository [Apache-2.0 license](https://github.com/tesseract-ocr/test/blob/232ff181c66516116ec0e84c4963f70de15050fd/LICENSE)
and [image/ground-truth README](https://github.com/tesseract-ocr/test/blob/232ff181c66516116ec0e84c4963f70de15050fd/testing/README.md)
are retained along with the original images and exact upstream gold text. This
small, predetermined regression subset is not an independent holdout corpus.
The content was inspected as benign generic OCR test prose. No OCR result was
used to choose a different ground truth or remove the rotated example.

The manifest records repository commit/tree, a deterministic `git archive` SHA-256,
each file's SHA-256/size, fixture and truth addition commits/dates, identifiers,
and explicit nulls for unavailable publication date, release version and DOI.
The image's TIFF resolution is 200 dpi; the rotated PNG's pHYs is 7,874 pixels
per meter (approximately 200 dpi). These are image metadata, not fabricated PDF
coordinates. Reproduce acquisition without transferring any local documents:

```sh
uv run python scripts/fetch_ocr_corpus.py --destination /tmp/tpe-ocr-public
```

The resulting `manifest.json` supplies absolute verified paths to the evaluator's
`--public-manifest` option. A failed or partial download never produces the ready
manifest; use a new destination on retry.

## Validated local checkpoint

The [exact source/check record](validation/cpu-runtime-checks.json) records
126 Python tests, 28 source-exchange tests, default Rust format/Clippy/tests,
the locked workspace Docling-text/GROBID checks and the final 12 GROBID contract
and empty-coordinate regressions. Independent review found no remaining
substantive code or evidence-integrity findings. The source record distinguishes
the full workspace check before the narrow Rust fix from its focused validation
and the final default checks afterward.

Both dedicated databases remain outside git, with source/schema/reproduction
and JSON summaries in this PR. The initial database and its renderer defect are
retained unchanged; the second uses a separately verified font configuration.
The [font-qualified results](validation/cpu-ocr-eval-font-qualified-20261004.json)
retain all 16 attempts and 16 scored pages: clean generated image/scan/mixed pages
and the published clean sample have zero normalized word error; the degraded
scan has one word error; both rotation cases remain poor. Zero errors on these
small regression fixtures do not establish general OCR accuracy. All OCR remains
Partial. [Evaluation detail and both database identities](CPU_OCR_EVAL.md).

The [real GROBID report](validation/grobid-cpu/report.json) measures 2.632-second
startup, 25.370-second first request and about 1.1-second repeat service/Rust
requests. Missing author geometry and two semantic model shortcomings remain
visible. The [Docling probe](validation/docling-runtime-audit.json) measures
1.437/1.453-second fresh-process recognition of three exact benign lines. These
are shared-VM observations with the explicit cache/process definitions in their
reports, not controlled throughput or device claims.

Local checks still unavailable: the OSV dependency audit could not connect
through this environment's network tunnel; Swift and CMake are not installed,
and the CMake targets require macOS Foundation. Those commands were attempted
and are not counted as passes. Hosted CI, production service integration,
PDF-coordinate transforms, broader OCR/scholarly accuracy and real mobile
execution remain separate gates. No database, runtime binary or model was
committed.
