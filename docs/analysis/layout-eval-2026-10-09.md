# Layout and reading-order check of the lopdf engine against Poppler (2026-10-09)

Prepared by Claude (Anthropic) in a cloud session on 2026-10-09, at Ben Shore's request.
Configured model id `claude-opus-5-5`; the model serving a given turn can differ.
Session: https://claude.ai/code/session_01WiCyvWnyZTq54HgMA1T2Kd

This is a measurement record. Nothing in the engine was changed. The scripts, the pinned
corpus list and the result files are in
[`layout-eval-2026-10-09/`](layout-eval-2026-10-09/).

## 1. Findings

1. **The lopdf engine interleaves side-by-side columns on 29 of 292 pages (4 of 17
   documents), and reports all four documents as `complete`.** Poppler does the same on 4
   pages of 1 document. (Section 5.2.)
2. **A font with no `/Widths` array makes the engine scramble text.** It advances every
   glyph of such a font by a fixed 500/1000 em. Two documents whose body text uses such
   fonts come out with letters and words out of order, for example `difficul) ditsappears`
   where the page reads `difficulty (3) disappears`. (Section 6, D1.)
3. **The positions needed for a `pdftotext -layout` style output are already published by
   the engine, but no such output exists without PDFium.** A prototype that reads only the
   engine's JSON reproduces 73.9% of Poppler's `-layout` rows word for word (per-document
   median 83.1%, range 33.3% to 100%). (Section 5.1.)
4. **The character-order figure quoted earlier in the session (median 99%) is not an
   accuracy figure.** It scored the scrambled document at 96.9% and cannot see interleaved
   columns. It is kept in the tables for completeness and should not be used to judge
   correctness. (Section 7.)
5. **This sample is not the repository's arXiv or PMC corpus.** It does not test the
   numbers in `docs/ENGINE.md`. It does show failures those numbers did not surface.

## 2. What was asked

The session started from the question of what a pure-Rust, lopdf-derived engine would
involve, and narrowed to: how close is the existing engine to a layout-preserving output
like Poppler's `pdftotext -layout`. Ben then challenged the first summary figures and
asked for the test pages to be looked at directly, and for Poppler to be judged on the
same pages. This record covers all of that.

One point about the target: `pdftotext -layout` preserves physical position, not reading
order. Columns are printed side by side. Reading order is Poppler's default mode. Both
modes were used here, for different checks.

## 3. Setup

| Item | Value |
| --- | --- |
| Engine | `benpshore/pdftextract` at `1bc92cc` (2026-10-05), default build: lopdf backend only |
| Build | `cargo build --locked --bin tpe`, debug profile, Rust 1.97.0. The pinned 1.98.1 could not be downloaded (`static.rust-lang.org` was blocked in the container) |
| Engine command | `tpe extract FILE --db LEDGER --out DIR` (defaults: backend `lopdf`, 60 s deadline) |
| Poppler binary | `pdftotext` 24.02.0 (Ubuntu package): `-layout`, default mode, `-bbox-layout` |
| Poppler source read | GitHub mirror `tsdgeos/poppler_mirror` at `5b97e47` (2026-10-04, version 26.09.90), `poppler/TextOutputDev.cc`. The binary and the source read are different versions |
| Other tools | qpdf 11.9.0, Python 3.13.16 (standard library only), pdfTeX 1.40.25, `pdftoppm` 24.02.0 |
| Host | x86-64 Linux cloud container, 2 cores. No timing claims are made |

### Corpus

17 documents, 292 pages. PDF bytes are not committed; each file is pinned by SHA-256 and
source commit in [`corpus.json`](layout-eval-2026-10-09/corpus.json).

- **14 documents** are the first 14 entries of a seeded shuffle of the PDFs in
  `papers-we-love/papers-we-love` (203 PDFs at commit `9b92363`), restricted to files
  between 150 kB and 1.5 MB. All 14 were kept. Command:
  `git ls-tree -r -l HEAD | grep -i '\.pdf$' | awk '{print $4"\t"$5}' | sort -k2 | awk -F'\t' '$1>150000 && $1<1500000' | shuf -n 45 --random-source=<(yes 20261009)`
- **2 documents** were hand-picked before any result was seen: the TraceMonkey paper from
  `mozilla/pdf.js` (two-column ACM) and a Federal Register notice from `jsvine/pdfplumber`
  (three-column).
- **1 document** is a control made here with pdflatex: two columns, one in-column table,
  one full-width table, one display equation. Source:
  [`latex-twocol.tex`](layout-eval-2026-10-09/latex-twocol.tex).

Producers in the sample: pdfTeX (7), Ghostscript (4), Acrobat Distiller (2), Quartz (2),
iText (1), none recorded (1). There are no Elsevier, Springer or Wiley production PDFs:
arXiv, PMC and Europe PMC were unreachable from the container.

**One exclusion made after seeing its result.** `TAMReview.pdf` from `mozilla/pdf.js`
(23 pages, htmldoc) was dropped from the layout sample. Both tools print unmapped
characters on its body pages, so it tests font decoding, not layout. On it the engine
printed 3,526 words against Poppler's 6,150.

## 4. Methods

Four automated checks and one check by eye. Poppler is the comparator throughout, not
ground truth.

### 4.1 Layout prototype (`layout_proto.py`)

Renders a `-layout` style text page from the engine's JSON only. It never reads the PDF.
The algorithm is ported from Poppler's `TextOutputDev.cc`:

- rows: fragments whose baselines differ by less than 0.5 x font size
  (`maxIntraLineDelta`);
- column assignment: `TextBlock::coalesce` for lines inside a block, then
  `TextPage::coalesce` across blocks; `TextPage::assignColumns` when there is no block
  structure;
- output: pad each fragment to its column; 1 to 5 newlines by baseline distance.

Two departures from Poppler: character edges inside a fragment are interpolated linearly,
because the engine publishes one box per shown string and no per-character positions; and
the word-space threshold is the engine's own (0.15 x size).

Three variants were run:

| Variant | Input |
| --- | --- |
| `lines` | the engine's `lines[].text` exactly as published |
| `spans` | spans only, with no engine line or block structure |
| `linespans` | the engine's line membership and block ids (`lines[].column`), with text rebuilt from the member spans |

Scores against `pdftotext -layout`, per page, summed per document:

- **exact rows**: share of Poppler's non-blank rows for which the prototype has a row with
  the identical token sequence (NFKC, whitespace-split, multiset match within the page);
- **cells**: the same after splitting each row into cells at runs of two or more spaces,
  so it is sensitive to where the wide gaps fall;
- **words in order**: longest common subsequence of words in row-major order, divided by
  Poppler's word count;
- **characters in order**: the same over characters with all whitespace removed.

The longest common subsequence is exact (bit-parallel), not sampled.

What these scores cannot show: whether either tool is right; anything about reading order
(both sides are row-major by construction); and, for the two in-order scores, local
scrambling, which they forgive.

### 4.2 Column interleaving (`interleave_check.py`)

Counts pages where a tool reads side-by-side columns line by line. It uses the ordered
line list each tool publishes: the engine's `lines`, and Poppler's `-bbox-layout`. Two
signals per page:

- **zip**: two consecutive lines on one baseline (within 3 pt) with disjoint x-ranges,
  each at least 25 non-space characters;
- **fused**: one line whose box covers two of the other tool's long lines sitting side by
  side on that baseline.

Both require column-sized geometry: each line at least 20% of the page width, the pair
spanning at least 50%. A page is flagged when zip + fused is 5 or more.

The geometry rule was added after the first version flagged one Poppler page
(`pwl12` page 5) where the engine had only split a single column line in two. The
thresholds were set on this sample, so the counts are descriptive of it, not a validated
detector. It sees one failure only: it does not see a table or figure read in the wrong
place, or any error inside a column.

### 4.3 Unknown words (`word_check.py`)

Counts words (four or more ASCII letters) in the engine's own page text that do not occur,
ignoring case, in Poppler's default-mode output for the same page. It is a floor on
garbling, not an error rate: it cannot see order, and it also counts harmless differences
in how the two tools join end-of-line hyphens (fragments such as `tion`).

### 4.4 Fonts without widths (`font_widths.py`)

Lists simple fonts (`Type1`, `TrueType`, `MMType1`) with no `/Widths` entry, from
`qpdf --json`. Every font object in the file is listed, used or not.

### 4.5 Check by eye

Seven pages were rendered with `pdftoppm` at 72 to 80 dpi and read as images by the model,
then compared with the engine's page text and Poppler's default-mode text for that page.
Four were chosen after Ben asked for the pages to be looked at; three more were chosen to
test the interleaving check (two flagged pages, one borderline). Seven pages is too few to
estimate a rate.

## 5. Results

### 5.1 Layout prototype against `pdftotext -layout` (variant `linespans`)

| Document | Producer | Pages | Poppler rows | Exact rows | Cells | Words in order | Characters in order | Engine status |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `fedreg-3col` | iText | 15 | 703 | 91.0% | 80.4% | 98.7% | 98.2% | complete |
| `latex-twocol` | pdfTeX | 3 | 119 | 95.8% | 91.6% | 99.7% | 100.0% | partial |
| `pwl01` | pdfTeX | 16 | 1115 | 83.1% | 81.9% | 96.7% | 99.6% | complete |
| `pwl02` | Distiller | 7 | 287 | 98.3% | 94.8% | 100.0% | 100.0% | complete |
| `pwl03` | Quartz | 15 | 516 | 91.3% | 89.9% | 98.9% | 99.7% | partial |
| `pwl04` | (none) | 8 | 588 | 70.2% | 61.6% | 87.6% | 99.8% | complete |
| `pwl05` | Quartz | 16 | 668 | 100.0% | 97.0% | 100.0% | 100.0% | complete |
| `pwl06` | pdfTeX | 14 | 893 | 83.3% | 79.5% | 98.2% | 99.6% | complete |
| `pwl07` | Ghostscript | 14 | 848 | 79.4% | 77.6% | 96.2% | 99.0% | partial |
| `pwl08` | pdfTeX | 11 | 969 | 53.9% | 50.7% | 91.8% | 98.5% | partial |
| `pwl09` | Ghostscript | 60 | 2258 | 73.5% | 72.9% | 91.1% | 99.2% | partial |
| `pwl10` | pdfTeX | 34 | 1516 | 76.4% | 71.1% | 89.3% | 99.8% | complete |
| `pwl11` | pdfTeX | 8 | 454 | 63.4% | 51.1% | 93.4% | 98.6% | complete |
| `pwl12` | Ghostscript | 10 | 957 | 33.3% | 33.2% | 81.0% | 90.0% | partial |
| `pwl13` | Distiller | 12 | 924 | 35.0% | 30.6% | 82.6% | 96.9% | complete |
| `pwl14` | Ghostscript | 35 | 879 | 91.9% | 91.9% | 96.6% | 99.2% | complete |
| `tracemonkey` | pdfTeX | 14 | 927 | 85.8% | 82.4% | 98.2% | 97.8% | partial |
| **Pooled** | | **292** | **14621** | **73.9%** | **70.6%** | **92.8%** | **98.4%** | |

Per document, variant `linespans`: exact rows median 83.1%, range 33.3% to 100.0%; words in order median 96.6%, minimum 81.0%; characters in order median 99.2%, minimum 90.0%.

| Variant | Exact rows, pooled | Exact rows, per-document median | Cells, pooled |
| --- | ---: | ---: | ---: |
| `lines` | 58.0% | 56.6% | 54.7% |
| `spans` | 74.2% | 83.8% | 69.6% |
| `linespans` | 73.9% | 83.1% | 70.6% |

Reading these: the gap between `lines` and `linespans` is the engine's text cleanup. It
rewrites line text after lines are built (for example it joins `rhon-` and `cus` into
`rhoncus` on the upper line), so the published line text no longer matches the physical
row. `spans` and `linespans` score alike on row content; they differ on alignment, where
the engine's block ids keep the columns apart (on the control document, cells 91.6%
against 37.0%).

### 5.2 Column interleaving

| Document | Pages | Engine: pages flagged | Poppler: pages flagged | Engine status |
| --- | ---: | --- | --- | --- |
| `fedreg-3col` | 15 | 7 (1, 2, 3, 4, 5, 6, 15) | 0 (none) | complete |
| `pwl04` | 8 | 6 (1, 2, 3, 5, 6, 8) | 0 (none) | complete |
| `pwl11` | 8 | 5 (2, 3, 4, 6, 7) | 4 (1, 3, 4, 5) | complete |
| `pwl13` | 12 | 11 (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11) | 0 (none) | complete |
| 13 other documents | 249 | 0 | 0 | |
| **Total** | **292** | **29** | **4** | |

Engine: 29 of 292 pages (9.9%) in 4 of 17 documents. Poppler: 4 of 292 pages (1.4%) in 1 of 17 documents. Page numbers are in parentheses. Per-page signal counts are in `results/interleave.json`.

### 5.3 Unknown words and fonts without widths

| Document | Engine words | Unknown to Poppler | Share | Simple fonts without `/Widths` | Engine status |
| --- | ---: | ---: | ---: | --- | --- |
| `pwl13` | 5116 | 462 | 9.0% | 5: Helvetica, Helvetica-Bold, Times-Bold, Times-Italic, Times-Roman | complete |
| `pwl04` | 3499 | 136 | 3.9% | 5: Arial,Bold, TimesNewRoman, TimesNewRoman,Bold, TimesNewRoman,BoldItalic, TimesNewRoman,Italic | complete |
| `pwl11` | 3183 | 68 | 2.1% | 2: Helvetica, ZapfDingbats | complete |
| `pwl12` | 6567 | 94 | 1.4% | 0 | partial |
| `pwl09` | 11013 | 102 | 0.9% | 1: Helvetica | partial |
| `pwl03` | 2832 | 25 | 0.9% | 0 | partial |
| `pwl08` | 5183 | 41 | 0.8% | 1: Times-Roman | partial |
| `pwl14` | 3449 | 22 | 0.6% | 1: Helvetica | complete |
| `pwl01` | 9194 | 46 | 0.5% | 0 | complete |
| `pwl06` | 6680 | 23 | 0.3% | 0 | complete |
| `pwl07` | 6533 | 18 | 0.3% | 12: Courier, Times-Roman | partial |
| `tracemonkey` | 8154 | 20 | 0.2% | 0 | partial |
| `pwl02` | 2208 | 4 | 0.2% | 0 | complete |
| `pwl10` | 7727 | 13 | 0.2% | 0 | complete |
| `latex-twocol` | 1059 | 1 | 0.1% | 0 | partial |
| `pwl05` | 4973 | 2 | 0.0% | 0 | complete |
| `fedreg-3col` | 4979 | 1 | 0.0% | 1: Helvetica | complete |

The share is under 1% in 13 of 17 documents; the other four are 1.4%, 2.1%, 3.9%, 9.0%. Fonts without `/Widths` occur in 8 of 17 documents; none of them is embedded. The two documents with the highest share (`pwl13`, `pwl04`) are the two whose body text is set in such fonts.

### 5.4 Pages checked by eye

| Document, page | What the page is | Engine (own page text) | Poppler (default mode) |
| --- | --- | --- | --- |
| `pwl13` p2 (Kalman 1960, retypeset) | two columns of prose with a display formula | columns interleaved line by line; letters and words out of order within lines. Unusable. Status `complete` | correct, columns in order |
| `pwl04` p2 (Aho and Corasick 1975) | two columns with a figure in the left column | columns interleaved line by line. Unusable. Status `complete` | lines intact; jumps to the right column before finishing the left one at the figure; some italic words letter-spaced (`o u t p u t`); `≠` printed as `#` |
| `tracemonkey` p1 | title, author block, two columns | body correct; in the author block the affiliation marks (`∗ + $ #`) are printed as separate lines, detached from the names | correct; marks attached to names |
| `pwl06` p3 (Spanner) | two columns, two figures | correct | correct |
| `pwl11` p3 (B-epsilon trees) | two columns of prose | interleaved line by line from the top of the page. Status `complete` | correct at the top; interleaves left and right lines in the middle of the page |
| `fedreg-3col` p2 | three columns with footnotes | the three columns interleaved line by line. Status `complete` | the first eight lines, which are all that was compared, follow the first column |
| `pwl12` p5 (consistent hashing) | two columns, heavy mathematics in Type 3 fonts | the first eight lines, which are all that was compared, are runs of U+FFFD. Status `partial` | body prose readable; mathematics printed as unrelated symbols with no warning |

All six flagged pages that were viewed (five engine, one Poppler) were truly interleaved.
`pwl12` p5, flagged for Poppler by the first version of the check and not by the final
one, has no Poppler line spanning both columns.

### 5.5 The control page

On page 1 of the pdflatex control, the engine's reading order follows the page: left
column, then the right column from its top (table, text, equation, text). Poppler's
default mode prints the table caption and first header cell directly after the `Abstract`
heading, and the rest of the table after the abstract paragraph, ahead of the
Introduction; the two section headings are split from their numbers. The prototype's rendering of this
page is close to Poppler's `-layout` output: columns and table cells line up, and 114 of
119 rows match word for word across the document's three pages.

In the display equation the summation sign comes from `cmex10`, whose `ToUnicode` map has
no entry for the sign's code (0x58). The engine prints U+FFFD, with a warning and status
`partial`. Poppler prints the letter `X`, with no warning.

## 6. Defects and gaps found in the engine

Evidence level: **confirmed** means the cause was traced to code and to the PDF object;
**observed** means the behaviour was seen and the cause was not traced; **read** means it
comes from reading the code and was not exercised.

| Id | Finding | Level | Evidence |
| --- | --- | --- | --- |
| D1 | No width fallback for a font without `/Widths`. Every glyph advances 500/1000 em, so string boxes are wrong, neighbouring strings overlap, and sorting by position scrambles text and closes the column gutter | confirmed | `DEFAULT_WIDTH = 500.0` (`src/backend/lopdf_backend.rs:94`), used when no width is known (`src/backend/lopdf_backend/widths.rs:39`). There are no built-in metrics for the 14 standard fonts. `pwl13` object 64 is `<< /BaseFont /Times-Roman /Encoding /WinAnsiEncoding /Subtype /Type1 /Type /Font >>`; `pwl04` object 63 is `<< /BaseFont /TimesNewRoman /Encoding /WinAnsiEncoding /Name /F4 /Subtype /TrueType /Type /Font >>`. In `pwl13` p2 the string `e` spans x 210.7 to 215.2 at 9 pt, which is 0.5 em |
| D2 | Columns interleaved on pages whose body fonts do have widths | observed | `fedreg-3col` (7 pages) and `pwl11` (5 pages). On the pages inspected every line carries the same `column` id although the line boxes leave clear gutters (`fedreg-3col` p2: x 45 to 203, 222 to 383, 399 to 536). The cause in `reading_order.rs` was not traced |
| D3 | Status `complete` on interleaved or scrambled output | observed | All four documents with flagged pages are `complete`. The engine's own notes say `complete` means no detected gap, not correct text; this is a concrete case |
| D4 | Published line text does not match the physical row | confirmed | `pipeline.rs` runs `reading_order::order_page`, then `text_cleanup::clean_document`. Cleanup moves hyphenated word halves between lines. Variant `lines` 58.0% against `linespans` 73.9% |
| D5 | Superscript marks become lines of their own | observed | `tracemonkey` p1 author block; the marks are present as spans, in separate lines |
| D6 | No layout-preserving output in the pure-Rust build | confirmed | The only layout mode is `liteparse-layout`, feature-gated on `pdfium` (`Cargo.toml`) |
| D7 | Per-character positions are computed and discarded | confirmed | `advance()` sums per-glyph widths; `emit()` stores one box per shown string (`lopdf_backend.rs` around 2027 to 2082) |
| D8 | The content interpreter has no operator for marked content or text rendering mode, so `ActualText` and invisible text are not distinguished | read | `OpKind` in `src/backend/lopdf_backend/content.rs` lists the operators acted on |

Fonts without `/Widths` occur in 8 of the 17 documents. In two of them (`pwl13`,
`pwl04`) they carry the body text; whether the others are affected was not checked.

### What a layout mode would need

From the prototype and the Poppler source: run between `order_page` and `clean_document`
(D4); keep per-character advances, or accept interpolation (D7); fix D1 first, because
every position depends on widths; and bound the column assignment, which in Poppler is
quadratic in lines per block and in blocks per page, against the engine's 20,000-line
page limit. The prototype is Python over JSON. No Rust was written.

## 7. Corrections to statements made earlier in the session

1. **"Characters in the same order, median 99%"** was presented as the headline. It is a
   lenient score: local scrambling costs it almost nothing and it is blind to column
   order. Ben called it out; the page images confirmed he was right.
2. **"The engine you want already exists in the repo"** was said early, on the strength of
   the repository's own documents. It was withdrawn when Ben reported the engine had many
   problems, and the measurements here support him.
3. **The first prototype had a column-assignment bug** (fragments were compared in row
   order, not x order). It was found by looking at the output and fixed before any figure
   in this record was produced. The row-content scores did not depend on it.
4. **An earlier reading-order score against Poppler's default mode** (word-order LCS:
   `tracemonkey` 0.963, control 0.698, `fedreg-3col` 0.464, `TAMReview` 0.089) was
   dropped, because Poppler's default mode is not ground truth and the score punishes a
   moved block heavily. In hindsight the `fedreg-3col` value was the interleaving
   showing up. That script is not part of this record.
5. **The first font scan** (with `pikepdf`, walking page resources) missed one font that
   the final scan (`qpdf --json`, all font objects) lists: a `Helvetica` without widths in
   `fedreg-3col`.

## 8. Limits

- Poppler is a comparator. Where the two tools differ, only the seven pages in 5.4 and
  the control page say which one is right.
- 17 documents from computer-science sources and one government notice. No biomedical
  publisher PDFs. The repository's arXiv and PMC corpora were not run.
- The Poppler binary (24.02.0) is older than the current release and than the source that
  was read.
- Debug build with Rust 1.97.0, not the pinned toolchain. Output text was byte-identical
  across two runs; no check was made that a release build on 1.98.1 gives the same text.
- The interleaving thresholds were tuned on this sample.
- Nothing here was reviewed by a second person or a second model.

## 9. Other checks made in the session

| Statement | Basis |
| --- | --- |
| The web app depends on `pdf-oxide-wasm ^0.3.77`; nothing under `web/` references lopdf | `web/package.json` and a search of `web/` at `1bc92cc` |
| No `wasm32` target or build of the engine exists in the repository or its workflows | search of manifests, workflows and scripts at `1bc92cc` |
| The lopdf backend loads from memory; its filesystem, tempfile and SQLite uses are inside its test module (from line 4128) | `src/backend/lopdf_backend.rs` |
| `lopdf` 0.45.0 has a `wasm_js` feature and a `wasm-bindgen-test` dev-dependency; its default features include `rayon` | the crate's `Cargo.toml` from crates.io |
| PDFium returns 0 from `FPDFText_GetUnicode` for a character it cannot map, and exposes `FPDFText_HasUnicodeMapError` | `public/fpdf_text.h` on pdfium.googlesource.com, fetched 2026-10-09 |
| Package versions and licences on 2026-10-09: `mupdf` (npm) 1.28.1, AGPL-3.0-or-later; PyMuPDF 1.28.2, AGPL or Artifex commercial; docling 2.137.0, MIT; docling-mcp 3.3.0, MIT; markitdown 0.1.8, MIT; markitdown-mcp 0.0.1a7; pymupdf4llm 1.28.2; mineru 4.0.11, custom licence (not read); mineru-mcp 1.0.0 (2025-06-09); marker-pdf 2.0.0, Apache-2.0 per PyPI; zotero-mcp-server 0.14.1 | registry JSON from npm and PyPI |
| Not verified: that the engine compiles to WebAssembly; GROBID's current release and licence (the GitHub API call was refused); any quality ranking of the tools above | |

## 10. Tool use in this session

What was run, in order, and what failed.

1. **Context.** Read the saved memory notes on this project; one search of past
   conversations; searched Google Drive and read the architecture document
   (`1K3w6QcaaRaGH0fHWyWZby3Ty1wgea3QSMNCONaOHv0Q`).
2. **Repositories.** Shallow clone of `benpshore/pdftextract` at `1bc92cc`, read-only,
   later re-cloned with push access for this record. Looked at `tpe-lib` (empty),
   `tpe-wasm` (17-line stub) and `scholarlypdftools` (Python, archived).
3. **Reading.** `CLAUDE.md`, `AGENTS.md`, `docs/ENGINE.md`, `CLAUDE_HANDOFF.md`,
   `UPSTREAMS.md`, `FONT_RESOURCES.md`, `EXTRACTION_ROUTING_REVIEW.md`,
   `ENGINE_REPAIR_2026-10-03.md`, `USER_REQUIREMENTS_HISTORY.md`,
   `HORIZONTAL_GROUPING.md`, `LITEPARSE_LAYOUT.md`, `COMPARE_EVAL.md`; `Cargo.toml` and
   every crate manifest; parts of `lopdf_backend.rs`, `content.rs`, `widths.rs`,
   `reading_order.rs`, `schema.rs`, `pipeline.rs`, `scripts/compare_poppler.py`.
4. **Build.** `cargo build --locked --bin tpe` (1 min 11 s). `rustup` could not fetch
   1.98.1 or a `wasm32` target.
5. **Poppler source.** `gitlab.freedesktop.org`, `poppler.freedesktop.org` and a
   `freedesktop/poppler` GitHub mirror were unreachable or absent; the
   `tsdgeos/poppler_mirror` mirror was cloned. Read in `TextOutputDev.cc`: the tuning
   constants, `TextLine::coalesce`, the column assignment in `TextBlock::coalesce` and
   `TextPage::coalesce`, `TextPage::dump`, `TextPage::assignColumns`.
6. **Corpus.** Files fetched from `raw.githubusercontent.com`. Listings came from
   blobless clones. Four docling test PDFs returned 404 and were not used.
7. **Runs.** `tpe extract` on each file; `pdftotext` in three modes; `pdfinfo`,
   `pdffonts`, `qpdf --show-object` for the font dictionaries; `pdflatex` for the control.
8. **Scripts.** First written with `rapidfuzz` 3.14.6 and `pikepdf`, installed with pip in
   the scratch container. Rewritten on the standard library for this record. The
   replacement LCS was checked equal to `rapidfuzz` on 400 random cases, and the rerun
   reproduced every earlier figure with zero differences.
9. **Images.** `pdftoppm` renders of eight pages; seven were read by the model (5.4).
10. **Web.** One fetch of the PDFium header; npm and PyPI registry lookups; two web
    searches whose result pages were not opened and are not relied on.
11. **Not done.** No engine code changed. No Rust written. Nothing was run on Ben's
    computer, although the session was linked to it part-way through.

## 11. Reproduce

```sh
cargo build --locked --bin tpe
# put <id>.pdf for every id in corpus.json into PDFS/, bytes matching the sha256
docs/analysis/layout-eval-2026-10-09/run.sh target/debug/tpe PDFS WORK
```

`WORK/layout.json`, `interleave.json`, `word-check.json` and `font-widths.json` should
equal the files in [`results/`](layout-eval-2026-10-09/results/), given the same engine
commit and Poppler 24.02.0.
[`results/output-hashes.json`](layout-eval-2026-10-09/results/output-hashes.json) holds the
SHA-256 of each engine text output, each Poppler output and each prototype rendering from
this run. [`SHA256SUMS`](layout-eval-2026-10-09/SHA256SUMS) covers every file in the
directory.
