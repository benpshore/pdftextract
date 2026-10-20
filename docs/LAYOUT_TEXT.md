# Physical layout checkpoint

This branch preserves an optional physical text renderer and independent
fixtures. It is **not a repair of the default extraction reading sequence**.
Work on the legacy ordering implementation was paused at Ben's request for a
clean rebuild. No change to `reading_order`, extraction, cleanup, browser,
OCR, storage, schemas, publication or dependencies is included.

## API and evidence

`tpe::layout_text::render_page(&PageText, LayoutOptions)` reads only immutable
page dimensions, rotation and original spans. Call it on
`LopdfBackend::open(bytes, password)?.page_text(page)?`, before any cleanup.
It never reconstructs geometry from `page.text` or `page.lines`; tests poison
those cleaned strings and verify the entire serialized input is unchanged.

The returned `LayoutText` contains a separate string, inferred grid pitches,
and one `Placement` per input span, including empty strings. Each placement's
UTF-8 byte range selects exactly the original span string. Generated spaces
and newlines lie outside those ranges. This preserves mapped Unicode,
whitespace, duplicate text, overlapping strings and undecodable U+FFFD evidence;
it does not infer missing glyphs or undo lopdf's existing NFC/ligature policy.

Physical layout reads across each displayed row and preserves common column
positions. Semantic reading order should read a prose column to completion,
then the next column, while respecting spanning content and changing regions.
These are separate outputs with separate acceptance criteria. Rendering a
two-column grid does not establish correct semantic extraction.

## Rendering policy and limits

- Apply `/Rotate` in multiples of 90 degrees using f64 arithmetic over the
  evidence's unrotated PDF boxes. Other rotations fail explicitly.
- Set the shared horizontal origin to the leftmost positioned span. Omit page
  margin padding, top/bottom padding, and any generated final newline.
- Use explicit point pitches when supplied, otherwise estimate a column pitch
  from median box width / Unicode scalar count, and a row pitch from 1.2 times
  median box height. Empty/degenerate estimates fall back to 5 and 12 points.
- Group sorted box bottom edges within 0.3 row pitches of a fixed anchor.
  Heading offsets, indentation, table cells and vertical row gaps are retained
  to this grid's resolution. Slight row drift cannot chain many rows together.
- Preserve a span verbatim at its rounded horizontal position. An overlap
  shifts later text right by one space and reports `OverlapShift`, rather than
  overwriting, deduplicating or dropping characters.
- Missing, invalid, inverted or off-page boxes, control text, and inferred
  vertical text are appended after a blank line in `(seq, input index)` order.
  Their placement reports the uncertainty and has no positioned row/column.
  Source control characters remain verbatim and are not interpreted as grid
  instructions. Individual text direction cannot be recovered reliably from
  axis-aligned span boxes alone.
- Count one grid cell per Unicode scalar, including combining marks, CJK and
  emoji. UTF-8 byte limits are separate. This does not establish alignment in a
  terminal using grapheme clusters or East Asian display widths.

Default ceilings are 512 scalar columns, 4,096 output rows, 20,000 spans and
2 MiB UTF-8 output. Callers may lower them or increase them up to hard ceilings
of 4,096 columns, 20,000 rows, 100,000 spans and 8 MiB. Invalid limits fail.
Every returned string, including fallback source text, respects row, column
and byte limits. Newline characters count actual output rows; an ending source
newline therefore includes an empty final row. Input bytes are checked before
character scans or run-list allocation. Extreme finite coordinates and tiny
positive pitches fail a grid bound before coordinate-dependent allocation.
Failures return `LayoutError` with no partial output; callers must surface
that error, not publish empty/partial output as complete.

Work is O(spans log spans + input/output bytes), with linear auxiliary metadata
bounded by the span count. There is no pairwise geometry search, recursive cut
or full page canvas. String allocator capacity overhead and memory already
owned by the caller are outside these component bounds.

## Portability boundary

The module imports only `std` and `schema::{BBox, PageText, Span}`. It performs
no file access, networking, threading, native calls, browser calls or OCR and
adds no dependencies. An eventual portable extraction crate can move this
module with minimal evidence types and an owned byte/string adapter.

The current root package still contains CLI, SQLite, network and native-facing
dependencies and is not claimed to be a browser WASM or WASI extraction
runtime. No WASM/WASI runtime has been run for this checkpoint; even compiling
the isolated renderer would establish only its compilation boundary. Future
runtime acceptance must execute PDF bytes through lopdf and this renderer in
the actual browser/WASI host, test output and errors, and record runtime,
artifact and fixture identities. That vertical slice remains unimplemented.

## Preserved semantic failures for the clean rebuild

The independent PDF specifications and `.txt` expectations live in
`tests/fixtures/layout_text/semantic-{two,three}.*`. They intentionally scramble
content-stream order. The cases reproduce failures on the unchanged ordering
code from main `1bc92ccb7b566f11a3b750f21007cfeb310593b2`:

| Case | Expected sequence | Observed raw sequence |
| --- | --- | --- |
| Two columns under a tight centered heading | HEADING, left first row, left second row, right first row, right second row | HEADING, left first row, right first row, left second row, right second row |
| Three columns under a tight heading | CENTER HEADING, L1, L2, M1, M2, R1, R2 | L1, L2, CENTER HEADING, M1, R1, M2, R2 |

Run `cargo test --locked --test reading_sequence_checkpoint -- --ignored
--nocapture` to reproduce them. These two acceptance tests are explicitly
ignored in the normal checkpoint suite because both are known failures;
they are not counted as passing semantic evidence. They verify real lopdf
extraction, unchanged span evidence and unique span membership before the
order comparison. Expected order is handwritten, not derived from existing
XY-cut output or Poppler. The fixture files also specify intended paragraph
boundaries; the current checkpoint assertions prioritize token order.

The inspected legacy mechanisms explain plausible failure paths: column cuts
require a gutter open across the complete region, title masking depends on
width/centering heuristics, row separation requires a median-height whitespace
gap, bridge detection uses the region midpoint and minimum prose width, and
fused-row splitting assumes two columns around a midpoint. The two observed
failures are evidence; these mechanisms are an implementation assessment, not
an independently established diagnosis of every corpus defect.

The clean rebuild still needs acceptance cases for changing column regions,
spanning internal headings, footnotes, mixed prose/tables/captions, narrow or
fused three-column rows, rotated individual glyphs, hostile input with truthful
incomplete status, and real scholarly PDFs with independent ground truth.
No real scholarly corpus assessment or accuracy claim is made here. No PR246
superscript comparison harness is imported or treated as accepted evidence.

## Verification

`tests/layout_text.rs` contains 14 companion-renderer tests covering independent
goldens, committed PDF extraction, all four page rotations, Unicode, overlap,
vertical/control fallback, character multisets and exact span slices, unchanged
input, contaminated cleaned strings, tied sequences, empty evidence, row drift,
coincident spans, extreme coordinates and exact row/column/UTF-8 limits.
Regenerate synthetic PDFs using the fixture README; expected output files are
never generated by the renderer or fixture generator.

Local checkpoint validation on x86_64 Linux with pinned Rust 1.98.1:

- `cargo fmt --all`, workspace Clippy with `-D warnings`, and locked workspace
  tests passed. The two new semantic acceptance cases are ignored in this run.
- All 14 physical-layout tests passed. Explicit `--ignored --nocapture` runs
  reproduced both semantic failures before and after cleanup (exit 101).
- `uv run ruff format`, `uv run ruff check` and all 210 Python tests passed.
- Regenerating all eight PDFs reproduced the staged bytes exactly; staged
  whitespace checks and a focused new-file secret-pattern scan passed.
- `uv audit --locked --preview-features audit-command` was attempted but could
  not reach `https://api.osv.dev/v1/querybatch` (proxy tunnel connection failure).
- Swift and CMake checks were attempted, but neither command is installed here.

Hosted CI results must be associated with the final commit independently of
these local observations. This checkpoint does not turn ignored known failures
into accepted reading-order evidence or authorize reuse in the clean rebuild.
