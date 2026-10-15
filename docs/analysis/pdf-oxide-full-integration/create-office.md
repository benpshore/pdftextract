# Creation and Office capability record

This checkpoint adds byte-based operations to the existing native operation
dispatcher. Source inputs remain immutable and are identified by SHA-256 in every
result. Artifact names are application-owned. These functions require the shared
worker deadline/address-space/cancellation boundary for untrusted input; their
local checks are not a sandbox.

## Source and license provenance

The reviewed registry artifacts are `pdf_oxide = 0.3.78` and
`office_oxide = 0.1.9`, both MIT OR Apache-2.0. The project's license is unchanged.
The exact crate SHA-256 values are:

| Crate | SHA-256 |
| --- | --- |
| pdf_oxide 0.3.78 | `95b1ecc1a60a314fbd1aea5fe8f700ec32af1efa70fef5c6879162444da3437d` |
| office_oxide 0.1.9 | `e07e156a6bc62caaa66fc698daad7405dd62e86f5292fd22898c1a55f154748d` |

The implementation was checked against the registry source, including
`src/api/pdf_builder.rs` (SHA-256
`332ba0f6a4c3225a93186275a54107a21eef1def86902f83259611b16a9d1826`),
`src/converters/office/mod.rs` (`96fb63a94a8fe359e37e0796018c86a5101cb49ad0c63f3bb1a2a4d4966d8efb`),
and office_oxide's `src/core/opc.rs`
(`6470a5de985c32ff6abb28649dfa287bbf769d5b7fb4680e013133964a7fab0d`).
Public references are the [Office guide](https://pdf.oxide.fyi/rust/docs/creation/office-conversion),
[pinned Office source](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/converters/office/mod.rs),
and [pinned creation source](https://github.com/yfedoseev/pdf_oxide/blob/v0.3.78/src/api/pdf_builder.rs).

The HTML/CSS fixture uses the package's public bundled DejaVu Sans bytes, SHA-256
`abdc775b21b1bc470d50c97e790d276f2054b7504e56e5bd3e64f48d68582322`.
Its upstream notice is `src/fonts/assets/LICENSE-DejaVu`; the other packaged
fallback-font notice is `LICENSE-fallback-fonts`. No external/private font was
imported for this fixture. The integration owner preserves these with the other
third-party notices.

## Operation contract

All creation parameters reject unknown fields. Text/Markdown/HTML input is UTF-8
in input slot 0. HTML/CSS fonts and composition images refer to immutable input
slots, never paths. Whole-document Office operations reject partial page ranges.

| Operation | Actual pinned API / behavior | Material limits |
| --- | --- | --- |
| `create.text` | `PdfBuilder::from_text`, fallible `save_to_bytes` | Pinned writer emits 12pt text even when its font-size option changes spacing; non-12pt requests fail and can use composition instead. Subject metadata is rejected because the pinned text writer omits it. |
| `create.markdown` | `PdfBuilder::from_markdown` | Supported upstream Markdown subset; no full CommonMark or wrapping/fidelity claim. |
| `create.html` | `PdfBuilder::from_html` | Basic HTML-to-Markdown conversion. CSS/images fail explicitly rather than disappear silently. |
| `create.html_css` | `Pdf::from_html_css_with_fonts` | Supplied font bytes, A4 upstream pagination, passive HTML, in-memory PNG/JPEG data URIs. No browser fidelity claim. |
| `create.images` | `PdfBuilder::from_image_data_multiple` | Ordered PNG/JPEG inputs, one image per page. Strict dimensions and actual image decode precede embedding. |
| `create.document` | `DocumentBuilder` typed pages/elements | Text with Standard-14 fonts, line, rectangle, filled rectangle, indexed image and bounded buffered tables. Tables permit a header and colspans with equal-width columns; rowspans are rejected because the pinned placement can overlap cells. Coordinates and total table height are validated against each page. Other fluent-builder operations are not represented as implemented. |
| `office.to_pdf` | `Document::from_reader` → IR → `ir_to_pdf_bytes_pub` | Default `flow_no_system_fonts` avoids host font discovery, but omits embedded fonts and positioned import routing. |
| `office.to_pdf`, mode `native` | `OfficeConverter::convert_{docx,xlsx,pptx}_bytes` | Requires verified worker filesystem containment because Unicode/CJK fallback reads fixed host font paths. Availability is separate from the default flow path. |
| `office.from_pdf_docx` | explicit layout/flow APIs after fallible page count | `auto` uses layout through 30 pages and flow from 31; requested and effective modes are recorded. |
| `office.from_pdf_pptx` | explicit layout/flow APIs after fallible page count | Same 30/31 threshold; flow can change slide count. |
| `office.from_pdf_xlsx` | explicit layout/flow APIs after fallible page count | Layout through 200 pages, flow from 201; cells versus positioned drawing semantics differ. |
| `office.create_markdown` | `DocumentIR::from_markdown` → `create_from_ir_to_writer` | DOCX/XLSX/PPTX bytes; parsed output undergoes the same archive preflight. |

Creation has additional limits of 4 MiB text/CSS, eight supplied fonts, 8 MiB per
font, 16 MiB per image, 14,400-point page dimensions and 64 levels of HTML nesting.
The shared limits further bound total input/output, pixels, pages and items.
Image decoder allocation accounting is best effort; the worker memory budget is
still required. The adapter does not invoke upstream's `Pdf::to_bytes` or
`into_bytes`, avoiding their temporary-file/error-suppression branches.
Unsupported creation keywords and image font/line-spacing overrides are rejected
instead of being silently ignored. Unicode Markdown/basic HTML prose uses the
pinned bundled DejaVu regular/bold fonts and reports their hashes. Italic styling
can degrade, and non-ASCII code/table content can lose glyphs in Courier.

## Office preflight

Archive input and generated Office output both pass these checks:

- A bounded standard ZIP directory; ZIP64, split/self-extracting containers,
  encrypted entries, overlapping members and duplicate/case-colliding names fail.
  Checking the declared directory count also catches duplicates silently collapsed
  by zip 8.6's internal name map.
- At most 4,096 entries, 8 MiB per expanded member, 64 MiB total expansion and the
  stricter shared budget. Data is read with `take(limit + 1)` and CRC errors fail.
- UTF-8 XML 1.0 with matching tags, duplicate-attribute checks, bounded events,
  depth, attributes, sheets/slides, worksheet coordinates and cells. DTDs,
  processing instructions, unknown entities and malformed XML fail.
- Macro/OLE/ActiveX/embedded-object/external-link parts and relationships fail.
  Relationship targets must resolve to existing canonical internal parts.
- Only reviewed XML, font and PNG/JPEG image parts are accepted. Images are
  decoded under dimension/allocation checks; IR geometry, item counts and
  serialized expansion are also bounded before rendering.

Legacy DOC/XLS/PPT, macro-enabled or encrypted Office containers, non-UTF-8 XML,
unreviewed binary parts and non-PNG/JPEG images remain explicit unsupported
inputs. Office image/vector/font omissions remain possible upstream even after
preflight. Outputs always record unverified fidelity and transformation warnings;
an empty upstream image list is not treated as proof that a page has no images.

PDF-to-Office image preflight checks the shared bounded page/resource graph for
transparency masks before asking for image handles. It checks handle dimensions,
pixel/count/declared-byte budgets before decoding, then verifies decoded and
encoded PNG dimensions. Masked images fail explicitly because the pinned memory
decoder drops alpha; flattening is not silently substituted. The upstream handle
enumerator can still suppress errors and conversion can omit small images, so
these checks do not establish image completeness. Encoded stream dimensions and
decompression work also require the shared process memory/deadline boundary.

XLSX layout export stores text in DrawingML shapes, whereas flow export uses
worksheet cells. The output reports this distinction as `content_representation`.
The pinned cell-only Markdown view omits layout shapes; it is not a valid test of
whether that layout output contains source text.

## Verification evidence

`tests/oxide_create_office.rs` contains synthetic creation, supplied-font CSS,
image decode/pixel/output checks, three-format export/import, real 30/31 and
200/201 threshold conversions, minimal DOCX import, legacy-format rejection,
partial-selection rejection, CRC corruption, duplicate ZIP members, bounded
expansion, malformed XML, macros, OLE and external relationship regressions. It
also checks buffered table cells/blank cells, explicit rowspan/overflow failures,
actual RGB pixels in all three Office output formats, oversized PDF image
metadata rejection before decode, and explicit rejection of alpha loss.

The owner's `full-tests-03.log` ran the original 12 tests: 11 passed, including all
30/31 and 200/201 threshold conversions. The three-format round-trip test failed
because it expected XLSX layout text in the cell-only Markdown view. Source review
confirmed the DrawingML representation; the revised test checks both page labels
in the shapes and in the reimported PDF. The expanded 14-test suite is pending the
owner's next combined-source run. Evidence lives in
`/workspace/tpe-pdf-oxide-evidence-20261015/`; these results do not assert that the
latest edits have passed. Earlier DOM index and XML enum compilation errors were
repaired against the actual pinned types.

Visual review across representative real documents, Office application
interoperability, complex-script fidelity, native-font mode and platform parity
remain separate acceptance work. This checkpoint is not exhaustive toolkit
parity completion.
