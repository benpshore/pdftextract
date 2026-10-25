# Engine baseline

The `tpe` binary extracts, for each PDF, into one SQLite database:

- **Document identity**: SHA-256 of the complete bytes; paths/inodes are observations.
- **Page text** with positioned spans (evidence) and the reading order the engine inferred (`lines`, `text`).
- **Metadata**: title, authors, DOI, arXiv id, year, venue, abstract, with per-field provenance.
- **Reference list**: every entry, raw text authoritative, parsed fields best-effort and never invented.
- **In-text citation markers** linked to reference entries.
- **Chunks**: 20-page groups with a text digest and service time, the unit of the 30 ms target.

Results are keyed by input hash **and** backend identity (name, version, config digest, schema
version). Publishing a run is one transaction and idempotent: re-running replaces the same key.

## Backends

| name | status | notes |
| --- | --- | --- |
| `lopdf` | baseline, pure Rust, always compiled | content-stream interpreter with glyph geometry; no native deps, runs on every CI target |
| `pdfium` | implemented behind feature `pdfium` | `pdfium-render` 0.8 (same major as `docling-pdf`, so one instance links `libpdfium`); needs the PDFium shared library at run time; spans ordered by the engine's XY-cut; image objects become `raster` figures |
| `docling-text` | implemented behind feature `docling` | `docling-pdf` text layer; no models, no PDFium; docling's own reading order is kept |
| `docling` | implemented behind feature `docling` (implies `pdfium`) | full docling pipeline: layout, OCR (scanned pages), pictures as `layout` figures; needs PDFium, the ONNX models and ONNX Runtime |
| `pdf-oxide` | implemented behind feature `pdf-oxide` | independent pure-Rust character parser (`pdf_oxide` 0.3.78); per-character spans ordered by the engine's XY-cut; every page stays Partial by policy ([PDF_OXIDE.md](PDF_OXIDE.md)) |
| `poppler`, `mupdf` | implemented behind features `poppler` / `mupdf` | user-built providers behind the C ABI in `native/provider.h`, loaded only from explicitly configured absolute library paths; GPL/AGPL obligations stay with the user ([NATIVE_FALLBACK.md](NATIVE_FALLBACK.md)) |

The default build has only `lopdf`. Build the native ones with
`cargo build --release --features docling` (or `--features pdfium`); provisioning of the
libraries and models is described in [NATIVE.md](NATIVE.md). `tpe backends` lists every
backend, whether it is compiled in, and whether it opens a one-page probe PDF (for example
`open failed: unsupported: pdfium library not found: ...`). Asking `extract`/`eval` for a
backend that is not compiled in fails with a hint naming the feature.

A backend that declares its own reading order (`docling-text`, `docling`) gets one line per
span in `seq` order; the others are ordered by the XY-cut.

## Measured status (2026-09-29)

The `lopdf` backend's reference/metadata/marker/timing numbers from Eval run 36511221210
(`dev`, 60 papers) and run 36511549138 (`holdout`, 10 papers), both after PRs #39, #40 and #41
on `ubuntu-24.04-arm`, are in [README.md](../README.md) and GitHub issue #15; per-loop
taxonomies are in [docs/analysis/](analysis/). Body-text alignment (0.950 `dev` / 0.940
`holdout`, body only; exact word LCS; math, digit and operator tokens dropped on both sides;
appendices included) is still short of the 99% error-free-chunk goal, and body word precision
is 91.8% `dev` / 89.2% `holdout`. Since the last refresh, loops 11 (math/footnote token
dropping, lopdf figure boxes, geometry-based figure/table tagging, column overhangs) and 13
(caption continuations, figure labels, longtable pages, biography/front-matter roles,
digit/listing handling) worked on body text, and loop 12 (marker residue, parser tail cases,
hyphen pairs, INFORMS truth, NFKC title comparison) worked on references and markers; PR #42
is pending for a figure-box regression on one paper (2509.04183, at 0.711). Citation-marker
precision (targets that resolve to the correct entry) is 100% `dev` / 100% `holdout`, and
marker key recall is 99.6% `dev` / 99.3% `holdout`. Perf loop (PR #35) took arm p50 from 35.4
ms to 18.7 ms, before loop 10's region tagging added about 4 ms back; the added tagging (loops
11 and 13) brought p50 to 27.5 ms `dev` / 25.5 ms `holdout`, still a sub-30 ms diagnostic on
these hosted runners though creeping toward it, so a second perf loop is due; the M1
service-time target is not yet measured.

A pre-loop-10 three-backend comparison on the reference-metrics path (Native run 36491886979)
found `lopdf` at 99.6%/99.7% with body alignment 0.738 at p50 18.7 ms; `pdfium` at 97.3%/99.6%
with body alignment 0.714 at p50 109 ms, about 6x slower than `lopdf`; `docling-text` at
78.1%/94.7%; and the full `docling` layout+OCR pipeline at 96.9%/98.8% with body alignment
0.785 at p50 4892 ms (about 4.9 s per chunk), a routed exception. `lopdf` remains the fast
path. Accuracy reports for `pdfium`, `docling-text` and `docling` come from the Native workflow
(first docling comparison on issue #15).

## Cross-backend comparison harness

`tests/engine_compare.rs` is an offline harness for tuning text quality
across the parsers. It runs every backend compiled into the build (`lopdf`
always; `pdf-oxide` under `--features pdf-oxide`; `poppler`/`mupdf` when
their provider and runtime libraries are configured, otherwise the table
names the missing configuration) over the committed fixture PDFs
(`tests/fixtures/native-worker`, `tests/fixtures/pdfium-unicode`) and a set
of synthetic layout fixtures built in-test: the shared two-column paper,
ligatures through glyph names and through `ToUnicode` presentation forms,
line-end hyphenation, superscript citation markers, a three-page running
head with page numbers, and two columns with an interleaved caption. For
each fixture it prints every backend's status, line and word counts, the
word-level similarity of each backend pair (`2 * LCS / (words_a + words_b)`
over the whitespace-normalised page text) and a line diff where the pages
differ. `TPE_ENGINE_COMPARE_DIR=/dir/of/pdfs` adds uncommitted PDFs to the
printout without adding assertions.

```sh
cargo test --test engine_compare --features pdf-oxide,pdf-extract -- --nocapture
```

The harness asserts the behaviours the engine guarantees on those fixtures
on *every* backend (ligature expansion, `pipe-`/`line` joined while
`state-of-`/`the-art` and the attested `self-`/`contained` keep their
hyphens, `literature⁵` and `work²,³`, running heads and page numbers out of
the text, left column before right column with the caption in place) and
that the synthetic fixtures produce identical normalised text across
backends.

Measured 2026-10-05 on this branch, `lopdf` versus `pdf-oxide` 0.3.78
(debug build, Linux x86_64; diagnostic, not a corpus result):

| Fixture | Before | After | Change |
| --- | ---: | ---: | --- |
| synthetic/ligatures-tounicode | 0.727 | 1.000 | `pdf-oxide` handed on `ﬁ`/`ﬂ` from `ToUnicode` unexpanded (`ﬁnite ﬂow of the ofﬁce`); it now applies the same ligature expansion and NFC as `lopdf`, and the native providers do too |
| synthetic/ligatures (glyph names) | 1.000 | 1.000 | both parsers already resolve `/fi`-style glyph names to letters |
| synthetic/paper, hyphens, superscripts, running-heads, two-columns, shared-baseline-columns | 1.000 | 1.000 | per-character `pdf-oxide` spans and per-string `lopdf` spans reach the same lines, spaces, joins and markers through the shared reading-order and cleanup passes |
| fixtures/native-worker/*.pdf, pdfium-unicode/native.pdf, existing-ocr.pdf | 1.000 | 1.000 | ReportLab Helvetica controls, including the invisible existing-OCR text |
| fixtures/pdfium-unicode/partial-cmap.pdf | 0.000 | 0.000 | intended divergence: the `ToUnicode` map covers one code, so `lopdf` keeps U+FFFD at every unmapped position (`A���A����A��A��A`, Partial) while `pdf-oxide` recovers the letters from the embedded font instead (`ALPHA BETA GAMMA`, Partial by policy). Neither result is certified; the `auto` router's mapping guards decide, not output length |

The expansion is part of the backend identity: `pdf-oxide` records
`ligatures=expand` (adapter revision 2) and the providers add
`ligatures=expand` to their configuration digest, so runs cached before
this change are not confused with runs after it. The ligature count appears
as `ligatures expanded: N` on the page, as it does for `lopdf`; the provider
coverage check still counts the provider's own characters. Word spacing
(`SPACE_GAP`, 0.15 em), paragraph breaks (1.5 median line heights), column
cuts and the hyphen and superscript rules were not retuned: on these
fixtures they already agree across backends, and the licensed real-paper
corpus is not committed, so a threshold change would have no measured
evidence behind it. The harness is the place to add a fixture that shows
such a disagreement before changing a constant.

## Figures

Images never enter page text. Each page records `figures` (index, bbox, kind, MIME, pixel
size, SHA-256, file, caption). When the backend has the bytes, their SHA-256 is recorded;
with `--figures-dir DIR` they are also written to `DIR/<document hash>/p<page>-f<index>.<ext>`
(`png`, `jpg`, `jp2`, else `bin`) and `file` holds that path relative to `DIR`. The ledger
stores them in the `figures` table (`run_id`, `page`, `idx`, `kind`, `mime`, `width_px`,
`height_px`, `sha256`, `file`, `caption`, bbox columns); `tpe stats` prints the count.

## Commands

```sh
tpe extract paper.pdf --db corpus.sqlite --out out/ --jobs 4
tpe extract paper.pdf --db corpus.sqlite --backend docling --figures-dir figures/
tpe backends
tpe stats --db corpus.sqlite
tpe show --db corpus.sqlite --hash 3f2a --refs --meta
tpe bench paper.pdf --iterations 5
```

## Accuracy protocol

A chunk counts as correct only when every page text, every reference entry (raw) and every
citation link match the independently checked reference exactly. Character/word error rates
are diagnostics, not the acceptance metric. Fixtures in `tests/` are synthetic and generated
in-test; a licensed real-paper corpus (arXiv CC-BY, PMC OA) is added by manifest, not committed.
