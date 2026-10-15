# PDF Oxide 0.3.78 query integration

The native `oxide::execute` dispatcher accepts immutable `Input` bytes and a
`Context` with the selected pages and limits. The query implementation is
`src/oxide/query.rs`; its synthetic functional tests are
`tests/oxide_query.rs`. No path, network, shell, OCR, model acquisition, writing,
or persistent cache operation is exposed by this module. Parsing untrusted
input still requires the supervised operation worker: upstream allocations and
fallbacks happen before post-call result checks.

Every result includes the pinned source name, input SHA-256, operation and
parameter echo, explicit `completeness: "unverified"`, and structured upstream
warnings with category, stage, original page index, one-based page, and spec
section when supplied. Absence of a warning does not establish completeness.
Document-scoped warnings keep a null page; a page is never invented for them.

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
| `query.attachments` | `extract_embedded_files` after bounded name-tree validation | Exact synthetic payload, traversal-like filename retained only as metadata, cycle rejection; upstream may omit malformed streams and decode filenames lossily |

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

Verified source caveats affect interpretation:

- `extract_chars` can return an empty vector after a content decode failure.
  The damaged-stream regression therefore requires an unverified result, not
  an unsupported claim that the page is truly empty.
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

Pinned source references: [document API](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/document.rs),
[search](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/search/text_search.rs),
[outlines](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/outline.rs),
[structured regions](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/structured.rs),
[raw layout fields](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/layout/text_block.rs).
