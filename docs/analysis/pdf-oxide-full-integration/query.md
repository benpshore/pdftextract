# PDF Oxide 0.3.78 query integration

The native `oxide::execute` dispatcher accepts immutable `Input` bytes and a
`Context` with the selected pages and limits. The query implementation is
`src/oxide/query.rs`; its synthetic functional tests are
`tests/oxide_query.rs`. No path, network, shell, OCR, model acquisition, writing,
or persistent cache operation is exposed by this module. Parsing untrusted
input still requires the supervised operation worker: upstream allocations and
fallbacks happen before post-call result checks.

Every result includes the pinned source name, input SHA-256, operation and
parameter echo, explicit partial status and completeness, and structured upstream
warnings with category, stage, original page index, one-based page, and spec
section when supplied. Absence of a warning does not establish completeness.
Document-scoped warnings keep a null page; a page is never invented for them.
`operation_page` separately records the one-based page of the native call.
An observed native `operator_cap_exceeded` produces `completeness: "truncated"`
and `truncated: true`; otherwise completeness remains `unverified` and truncated
is null (unknown), never a proof that all content was processed. Query pages and
conversion baseline/requested projections carry their own outcomes. Search
separates `match_limit_reached: false` from source truncation; a source cutoff
cannot be reported as a complete search merely because it returned few matches.

Outer `page` fields are one-based. Nested upstream payload indices and MCID
scopes retain their original zero-based meaning. Search also preserves its
original UTF-8 **byte** offsets and reports that bounding boxes cover whole
matching spans, not exact highlighted glyphs. Original bytes are not changed.

| Operation | Pinned API and supported controls | Specific evidence/limits |
| --- | --- | --- |
| `query.document` | Fallible `page_count`, version, authentication state, producer/creator, permission flags, MarkInfo, output-intent presence | Producer and true count fixture; values are observations, not permission enforcement or certification |
| `query.page` | MediaBox, visible box, rotation, text-layer presence | Nonzero origin and CropBox fixture; corners are explicitly named; width/height subtract origins |
| `query.text` | `extract_text_filtered`; optional excluded layer/ink names | Independent left/right text fixture; excludes nothing by default |
| `query.chars` | Native `extract_chars_filtered` or explicit `representation: "span_derived"` via `extract_page_text_with_options` | Native observations and known ActualText defect retained separately; derived path must preserve Unicode and labels its geometry as derived; U+FFFD is not globally suppressed |
| `query.spans` | Native spans, ordered/filtered spans, or a named merging preset | ActualText; `mirrored`, `page_rotation_applied`, and `page_bbox` restored alongside upstream serde fields; incompatible options rejected |
| `query.words` | `extract_words_with_thresholds`, bounded word-gap override and named profile | Actual word strings and characters checked; artifacts remain included |
| `query.lines` | `extract_text_lines_with_thresholds`, word/line gap and profile | Actual line text and word data checked; artifacts remain included |
| `query.page_text` | `extract_page_text_with_options` with top-to-bottom, column-aware, or structure reading order | Both spans and chars retained; raw upstream dimensions and independently computed MediaBox dimensions are separate |
| `query.structured` | `extract_structured_with_column_mode` with auto/single/two | Regions, roles, columns, section IDs, original spans, boxes; geometric roles remain heuristic |
| `query.region` | Text/chars/spans/words/text-lines/path extraction in a PDF-coordinate rectangle | Left/right boundary fixture; intersects/contained/positive overlap; unsupported filtering combinations rejected |
| `query.layers` | `get_layers`, default-off OCG names | Names remain data; default-off list does not silently change baseline extraction |
| `query.inks` | Shallow or deep page ink names | Deep extraction exercised; no tint-based visibility inference |
| `query.fonts` | Per-page `page_font_stats` | Font name and dimensions exercised; empty-page estimates are upstream defaults, not actual font evidence |
| `query.search` | Per-page `TextSearcher::search_page` | Unicode byte offsets, literal/regex, case, whole-word, invalid regex, match overflow; no hidden whole-document `prepare_search` |
| `query.classify` | Per-page `classify_page` | Empty page and real text signals; **not** `classify_document`, which converts some page errors into Empty |
| `query.outlines` | `get_outline` after bounded graph validation | UTF-16 title, destination page, Next and destination cycles; parsed node count must match the checked source graph |
| `query.paths` | All paths, rectangles, or vector lines | Painted rectangle fixture; raw path operations and rendered extents; no active action execution |
| `query.content` | `get_page_content_data` | Actual decoded page content returned in an app-named in-memory artifact; raw PDF operators remain untrusted data |
| `query.font_data` | Embedded font bytes, Unicode-to-GID and width maps | Whole-document page bound, deterministic metadata maps and app-named artifacts; Standard 14 absence exercised, embedded-font fidelity still needs corpus evidence |
| `query.attachments` | `extract_embedded_files` after bounded name-tree validation and shared original-name decoding | Exact synthetic payload, UTF-16 UF/F filename restored from source and lossy upstream projection retained separately, traversal-like filename metadata only, cycle rejection; upstream may omit malformed streams |

The implementation does not run convenience “all pages” extraction. Page
selection is validated before building a range, and no partial range is silently
ignored by document-scoped queries. Whole-document embedded-font and outline
operations require the complete page count to fit the configured page limit.
All output is subject to aggregate serialized byte and retained collection
budgets; overflows are errors, not silently truncated successes. Search compiles
patterns with a 4,096-byte source limit, 1 MiB compiled/DFA limits, and a nesting
limit of 128. It requests one result beyond the requested cap to detect overflow.

Outline First/Next links and direct destination chains are checked before
calling upstream. Cycles, shared nodes, excessive depth, missing object loads,
and count mismatches fail. Destination-name and attachment name trees have
separate depth/item/cycle checks. No action is followed, attachment launched, or
source-provided filename used as an output path. Returned font/attachment names
are `font-NNNNNN.bin` and `attachment-NNNNNN.bin`; original names remain metadata.
Attachment filenames are decoded from original UF/F PDF strings using the
edit module's shared bounded walker. The lossy upstream filename is retained
separately for provenance; conflicting source-to-projection mappings fail.

Verified source caveats affect interpretation:

- `extract_chars` can return an empty vector after a content decode failure.
  The decoded-content preflight now reports that failure explicitly before the
  native character path can return an empty successful value.
- A real pinned native-character defect affects non-ASCII ActualText:
  `extractors/text.rs` decodes the PDF text string, then calls
  `show_text(actual_text.as_bytes())` in character mode, mapping the UTF-8 bytes
  through the document font again. The Unicode fixture produced mojibake in
  the first combined test run. Native output stays intact and carries the
  typed `actual_text_character_mapping_unverified` diagnostic. The explicit
  span-derived path, spans, and search remain independently tested against the
  original Unicode. This adapter does not reverse mojibake heuristically or
  claim to repair native glyph observations.
- Some span APIs filter or repair mappings, and reading order uses heuristics.
  Raw character and span results are distinct observations; no global mapping
  filter is changed.
- `extract_page_text_with_options` assigns upper-right MediaBox coordinates to
  fields named `page_width`/`page_height`. Those raw fields are preserved;
  `media_box_width`/`media_box_height` supply the actual extents and the output
  explains the distinction.
- Embedded fonts are deduplicated by base font name; the largest program wins,
  with bytewise tie-breaking. A larger subset is not necessarily a superset.
  The returned inventory records this omission risk.
- Upstream outline parsing skips some failed nodes. The checked source-node
  count is compared to the returned tree; known omissions become errors.

No generic object graph dump, parser/debug mutator, arbitrary low-level Rust
configuration, hidden global cache mutation, or eager hierarchical content
export is exposed. Hierarchical content includes image payloads without a
serializable, bounded provenance contract; the separate structured/text/table
and image interfaces cover those outputs explicitly. Region tables and images
belong to the integration owner's direct table/image adapters. Explicit header
cleanup and image/HTML projections are also separate so baseline extraction
remains available.

The tests use only small generated repository fixtures. Compilation and test
execution are centralized by the integration owner; this document is an API and
source-evidence inventory, not a claim that all combined-source checks passed.
It does not establish broad extraction accuracy, hostile-input containment,
toolkit parity, or cross-platform qualification.

The sibling `src/oxide/convert.rs` module exposes selected-page `convert.text`,
`convert.markdown`, and `convert.html`. Each calls the pinned corresponding
native converter and returns a baseline projection alongside requested-option
output. When options differ, the baseline and requested conversions use separate
parser instances. A baseline projection is not the original PDF string bytes.
Both complete effective option sets, the source digest, and typed native warning
provenance are recorded. Raw output is limited to 1 MiB per page and checked
against aggregate byte/item budgets; overflow is an error. All-page convenience
converters are not used.

Pinned operator-cutoff behavior was reproduced in the current Rust worker,
independently of any Python finding. The default cap is 1,000,000 operators.
The full/streaming-fallback parser emits `operator_cap_exceeded` with null page
and spec section Annex C. In contrast, the >256 KiB text pre-scan routes through
`parse_region_text_only`, whose cap branch silently stops. Both synthetic cases
retained `CAP_PREFIX` but dropped `CAP_TAIL`; the small control retained both.
The original observations, binary/input/output hashes and bounded commands are
under `operator-cap-native/before-fix-native.json` in the VM evidence directory.

Before a page text query, search, or conversion, the shared
`backend/pdf_oxide_content.rs` guard runs the public
`parse_and_execute_text_only` callback without retaining an operator vector.
There is no `StreamingContentParser` type in the pinned public source. Native
`parse_region_text_only` also increments its count for an unmatched restore
that the bracketed CTM route suppresses from the callback. Therefore the guard
charges every raw ASCII `Q` byte first, then every emitted non-restore operator.
It raises an explicit Limit at 1,000,000 conservative text work units or above
16 MiB decoded stream content. Literal/image `Q` bytes and injected pre-scan
context can overcharge this ceiling; it is not an exact native count or a native
diagnostic. The native global cap is never changed. Raw decoded content export
is separate and does not claim to interpret every operator.

Before those page calls load extraction resources, the shared Form preflight
rejects the confirmed shallow-subtype omission shape and calls the same stream
guard on nested decoded Forms. Its walk separately bounds aggregate Form
decoded bytes, depth and nodes. Both baseline and requested converter parsers
pass these gates independently. Native path-parser cutoff warnings still
retain typed provenance and produce partial/truncated outputs. These checks
cover the identified source paths, not every possible silent native omission.

`operator_cutoffs_fail_before_publication_in_bounded_native_workers` generates
two-page fallback/pre-scan/CTM-restore cases and runs only the sealed CLI worker with a
15-second deadline, 512 MiB growth and 1 MiB output cap. It requires explicit
cutoff failure and no publication. The CTM case mixes fewer than 1,000,000
suppressed restores with text-state operators, requiring the combined work
accounting. Its small control keeps both markers
in baseline and derived text. The path cutoff test retains the first vector
shape, preserves native page=null with operation_page=2, and requires explicit
partial/truncated status. Capture names are `operator-cap-control.pdf`,
`operator-cap-fallback.pdf`, `operator-cap-prescan.pdf`, `operator-cap-ctm-restores.pdf`, and
`operator-cap-native-paths.pdf`. These additions await the parent's next combined
build; no Cargo invocation was made by the query agent.

Conversion controls are reading order, heading detection, and inline form
values; HTML additionally accepts layout preservation, and Markdown accepts bold
marker behavior. Table reconstruction is disabled by default, and explicit
`extract_tables: true` fails as Unsupported until cell/row mapping is qualified.
Native table candidates are retained separately as evaluation evidence, not
successful validated table conversions. Images, formula rendering, image/file paths, automatic header
stripping, artifact suppression, and ligature expansion are disabled. Unsupported
options are rejected, not ignored. HTML retains baseline/raw native markup and
adds a separate `display_html` using the supplements module's shared structural
tag sanitizer and stable source-hash/page wrapper. Raw HTML and Markdown remain
untrusted source projections. No completeness or exact reading-order claim is
made for these native converters.

`tests/oxide_convert.rs` exercises the real Unicode ActualText, citations,
duplicate words, hard hyphens, baseline isolation, HTML source identity and
sanitization, page ranges, emptiness/malformed content, unsafe option rejection,
omitted image resources, and output limits. `tests/oxide_supplements.rs` keeps
cleanup candidates separate from the baseline and tests exact opaque PNG pixels,
repeated paints, inline and CMYK images, empty/decode failure distinctions, and
atomic failure. Soft masks/image masks are explicitly unsupported until faithful
bounded alpha composition is implemented. Form-resource cycles and masks,
page/form inline masks, and image-budget failures are exercised only in bounded
CLI workers. Green results are limited to these fixtures, not broader parity.

The synthetic adversarial conversion cases are captured as
`convert-adversarial-wrapped-description.pdf`, `convert-adversarial-blank-row.pdf`,
and `convert-adversarial-short-prose.pdf` when `TPE_OXIDE_FIXTURE_CAPTURE` is set.
Matching `.native-tables.json` files retain exact native candidates with
`evaluation_only: true`, `validated: false`, source hashes, and the independent
row/content obligation. Tests require original spans to retain all painted
strings and table-enabled conversion to fail explicitly. These synthetic
equivalents do not replace the parent's exact confirmed counterexamples, whose
source paths remain pending. Cleanup additionally captures
`supplement-adversarial-meaningful-heading-citation.pdf` and requires the observed
heading/citation removal to remain visible in preview provenance while baseline
text remains intact.

The run-06 [adversarial fixture index](adversarial-fixtures.json) preserves 22
owned captured files with exact hashes, raw native candidate cells, test/generator
provenance, and four independently viewed bounded Poppler source renders. It
records the confirmed wrapped-cell split, dropped blank/final row, and invented
prose table separately from the passing adapter rejection checks. Run 06 passed
10 query, 8 conversion, and 13 supplement tests, but three unrelated test targets
failed. It ran on a changing worktree; the index explicitly does not attribute
those results to the final current tree. Frozen combined checks remain pending.

Pinned source references: [document API](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/document.rs),
[search](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/search/text_search.rs),
[outlines](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/outline.rs),
[structured regions](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/structured.rs),
[raw layout fields](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/layout/text_block.rs).
