# PDF Oxide 0.3.78: editing and forms integration

This is the implementation and verification record for `src/oxide/edit.rs`,
`src/oxide/forms.rs`, and `tests/oxide_edit_forms.rs`. It describes the pinned
registry source, not the current website's broader claims. The integration owner
records exact combined-source commits and test outcomes in the parent handoff.

All operations receive immutable input bytes and return in-memory artifacts.
Input slot 0 is the source PDF. Page numbers in request JSON are one based;
page ranges are inclusive. Annotation indices are zero based within their page.
The dispatcher limits aggregate inputs, request JSON, artifacts and serialized
output. Editor operations additionally bound source/output page counts and item
collections. Upstream parsing and serialization allocate before some checks, so
untrusted input still requires the owner's supervised process, memory/time limits
and cancellation boundary. These local checks are not a security sandbox.

## Implemented surface

| Operation | Parameters and behavior | Pinned API | Verification scope |
| --- | --- | --- | --- |
| `edit.pages` | Optional `select: [3,1,3]`, `rotations: [{page,degrees}]`, `boxes: [{page,media,crop}]`. Boxes use `[x0,y0,x1,y1]`. Mutations address original pages; selection/reordering/duplication happens last. | `DocumentEditor::select_pages`, `set_page_rotation`, `set_page_media_box`, `set_page_crop_box`, `save_to_bytes` | Saved page count; tests independently read selected text, rotation and crop. Empty/out-of-range selection, huge range, invalid rotation/boxes and unknown JSON keys fail. |
| `edit.merge` | Empty parameters; all input PDFs in slot order. Every source must pass encrypted/signed-source policy; all page counts are checked before any import. | `merge_from_bytes` | Saved total count and source text order. A synthetic certification marker in either source position rejects. Page imports do not merge document-level outline, attachment, label, metadata or AcroForm catalogs; warning retained. |
| `edit.split` | `ranges: [[2,3],[1,1]]`. Aggregate page count is checked before expanding ranges. One range is serialized at a time, with aggregate output budget. | `extract_pages_to_bytes` | Each derivative is reopened; page count and text assertions. No path API or whole-corpus accumulation. |
| `edit.metadata` | `fields` map: `title`, `author`, `subject`, `keywords`, `creator`, `producer`, `creation_date`, `modification_date`; `null` clears a key. | `EditableDocument::get_info`, `set_info` | ASCII values round-trip. Non-ASCII new or retained Info values are rejected because the pinned Info writer emits raw UTF-8 while its reader expects PDF text strings. XMP is separate and may disagree. |
| `edit.annotations` | `action: inspect`, `flatten`, or `edit` with ordered `changes`. Changes add, update contents/rectangle/RGB, or remove an index. Rectangle arguments use `[x,y,width,height]`. | `get_annotations`, `get_page`, `AnnotationWrapper` setters, `PdfPage` add/remove, `save_page`, `flatten_page_annotations`; explicit `lopdf 0.45.0` serialized-object repair | CRUD saved/read tests. All changes to one page share a DOM instance. The pinned writer omits original-annotation updates/removals; the adapter reconciles the exact original/addition inventory and changes only the requested dictionary keys and page annotation array. |
| Annotation creation kinds | `text`, `free_text`, `highlight`, `underline`, `strikeout`, `squiggly`, `square`, `circle`, `stamp`, `link_uri`, `link_page`. | Corresponding pinned writer annotation builders | Plain text and HTTP(S)/internal-page links only. No JavaScript, Launch, submit action, remote-file action or rich-media payload. No link is fetched. Other annotation creation variants remain unsupported. |
| `edit.attachments` | `action: add`, `files: [{input:1,name:"fixture.txt"}]`; or `action: extract`. Additional input slots contain explicit bytes. | `embed_file`, `extract_embedded_files`, bounded object walk and original PDF text-string decoder | Exact byte round-trip, safe generated extraction names, traversal rejection. Original UTF-16/PDFDocEncoding file names are decoded before associating upstream extracted bytes; ambiguous lossy-name projections reject. Source names are metadata, never paths. Existing EmbeddedFiles trees and indirect Names dictionaries reject additions to prevent pinned writer data loss. |
| `edit.bookmarks` | `action: inspect`; flat records contain object ID, depth, title, raw destination identity, action presence. | Pinned object reader and PDF text-string decoder | Bounded depth/items and reference-cycle detection. Does not call the unbounded `get_outline` sibling walk or invent page numbers for unresolved destinations. Existing-document bookmark modification is not exposed by `DocumentEditor`. |
| `edit.content` | Ordered ASCII Helvetica `add_text` changes with page/rectangle/text/size/optional RGB. Typed `replace_text` requests return `Unsupported` after parameter/match checks. | `PdfPage::add_text`, `find_text_containing`, `DocumentEditor::save_page` | Overlays round-trip text. Replacement is disabled: pinned `set_text`/`save_page` paints black redaction masks even while extracted replacement text looks correct. A direct upstream regression preserves that evidence; text extraction alone is insufficient to enable it. |
| `edit.image_bounds` | `changes: [{page,name,rect}]` using `[x,y,width,height]`. Name must identify exactly one direct image occurrence. | `get_page_images`, `set_image_bounds` | Reopens output and compares each saved bounding rectangle. Repeated, missing and indirect/nested occurrences that cannot be identified exactly reject. |
| `edit.whiteout` | `regions: [{page,rect}]` with PDF corner boxes and `acknowledge_text_remains: true`. | `erase_region` | Explicit visual overlay only. Tests prove underlying text remains recoverable. Never described as removing confidential content. |
| `forms.inspect` | Empty parameters; stable fully qualified names, typed values, bounds, flags, tooltip, MaxLen, alignment, parent data, XFA presence. | Bounded shared AcroForm tree walk, `DocumentEditor::get_form_fields`, `FormFieldWrapper`, `has_xfa` | Cyclic/repeated nodes, excessive depth/items and duplicate names reject before unbounded upstream field recursion. Signed-source inspection remains available. Does not report the upstream default/first-widget page number as exact provenance. |
| `forms.add` | `fields` with name/page/rectangle/widget plus optional read-only/required/tooltip. Widget kinds: text, checkbox, combo, list, inert push-button, empty signature widget. | Pinned widget builders, `add_form_field` / `add_form_field_hierarchical`; explicit `lopdf 0.45.0` property repair | Saved names and requested flags/tooltip are checked. No silent renaming of duplicate fields. Creating an empty signature widget is not signing. Radio-group insertion into existing PDFs remains unsupported. |
| `forms.fill` | `fields` map to typed `{kind,value}`: text, boolean, choice, multi_choice, none. | `set_form_field_value` | Reopens saved PDF and compares every requested typed value. Rejects incompatible types, read-only fields, MaxLen overflow, unsupported signature values, missing names, and multi-selection on a single-choice field. |
| `forms.update` | Bounded unique field list with optional read-only, required, tooltip, rectangle, MaxLen, alignment, RGB background/border, border width. | Corresponding `set_form_field_*` APIs; explicit `lopdf 0.45.0` property repair | The pinned writer omits staged field properties; the adapter writes requested dictionary keys against extracted indirect field identities and reopens the result. Geometry/appearance updates require one merged field/widget identity; every requested property is checked after saving. Arbitrary appearance strings, raw flags and event scripts are not accepted. |
| `forms.remove` | `names` list of terminal fields. | `remove_form_field`; explicit `lopdf 0.45.0` reference repair | Reopens output and requires requested names to be absent. The corrective pass unlinks exact field/widget identities from AcroForm Fields, parent Kids, calculation order and page Annots, preserving unrelated annotations and siblings. Parent-container, ambiguous/direct-widget and signature-field removal reject. This does not erase orphaned values from the file. |
| `forms.flatten` | Empty parameters; whole document, or explicitly selected pages. | `flatten_forms` / `flatten_forms_on_page` | Full flatten must leave no AcroForm fields. Missing appearance warnings are retained; flattening does not prove complete visual appearance. Signature widgets reject. |
| `forms.export` | `format: fdf` or `xfdf`; optional `include_no_export` defaults false. | `FdfWriter`, `XfdfWriter` on current field snapshots | In-memory output; FDF literal escaping and XML escaping tests. Editor file-export convenience helpers are not used because they read the original source instead of staged values. |
| `forms.xfa` | `action: inspect` or `convert_static` with `accept_layout_loss: true`. | `XfaExtractor`, `XfaParser`, `XfaConverter`, `add_converted_page`, `PdfWriter` | Decoded XML byte count, element count, nesting, field/page count bounded. DTDs, custom entity references, scripts, connections, resource references and dynamic/repeated sections reject. Standard XML escapes and numeric character references are permitted. Skipped fields reject conversion. Output explicitly records lost original layout/non-form content. |

Document-scoped form operations reject an outer page selection instead of
silently ignoring it. Form flattening alone supports page selection. Mutating
operations use the owner's encrypted/signed-source admission policy. No source
file is rewritten and no private key, signing identity, credential, filesystem
path or network resource is accepted by these modules.

## Known limitations and preserved evidence

- Page-count and saved-value checks establish structural behavior on the tested
  fixtures. They do not establish general extraction accuracy, visual fidelity,
  preservation of every unknown PDF extension or broad safety.
- The pinned FDF writer does not encode Unicode strings or escape Name values.
  FDF export therefore rejects non-ASCII fields/values and Name values outside
  a conservative ASCII subset, directing callers to XFDF. XFDF rejects
  multi-value arrays because its pinned writer comma-joins them and loses
  boundaries. FDF supports bounded ASCII arrays. These are reported failures,
  not successful-but-lossy exports.
- Upstream attachment extraction may omit malformed streams silently. Every
  extraction response carries this uncertainty, including empty results.
- The attachment writer at `editor/document_editor.rs:2292` replaces the
  EmbeddedFiles tree and clones only a direct Names dictionary. The adapter
  rejects either an existing EmbeddedFiles tree or any indirect Names
  dictionary before adding an attachment; preserving unique preexisting data
  takes precedence over claiming support.
- `editor/document_editor.rs:4378` reloads annotations from the original source
  on every `get_page`; per-page grouping in the adapter prevents overwriting
  earlier changes in the same request. Its serialization paths around lines
  2425, 3050 and 3680 allocate, append and write new annotations without
  persisting updates/removals of original annotations. The corrective object
  pass rejects any inventory that differs from the expected original entries
  plus surviving additions; it does not guess which unrelated entries to remove.
- `editor/form_fields.rs:619` and the existing-field serialization path omit
  staged properties other than values. The corrective pass preserves existing
  field dictionaries and writes only typed requested flags, tooltip, rectangle,
  MaxLen, alignment and appearance-characteristic keys. Output metadata names
  both `pdf_oxide 0.3.78` and `lopdf 0.45.0` in
  `data.postserialization_repair`; warnings identify the corrective rewrite.
  This is mixed-library adapter behavior, not an upstream-only fidelity claim.
  Encrypted/signed-source rejection runs before either mutation/repair path.
- The pinned attachment reader prefers `/UF` but applies UTF-8-lossy decoding
  to its PDF string bytes. The adapter reads original `/UF` or `/F` strings with
  PDF text-string decoding, checks the name tree for cycles/depth/items, and
  associates those names with upstream decoded attachment bytes. Names remain
  metadata even when they contain separators or other unsafe path characters.
- `remove_form_field` records names in `deleted_form_fields`, but the pinned
  existing-field writer retains the original field/widget graph. The adapter
  reconciles exact indirect identities and verifies name absence. Unreferenced
  objects may still retain old values; removal is not confidential erasure.
- `save_page` turns existing-text changes into destructive redaction with
  default black fill. The independent visual finding is preserved by a direct
  pinned-API native regression: extracted text changes correctly while the
  serialized page contains a new black filled rectangle. Adapter replacement
  remains `Unsupported` until a visually qualified implementation is available.
- `extractors/forms.rs` descends field `/Kids` recursively without a visited
  set. Every wrapper field read and direct repair extraction now runs the
  shared bounded AcroForm walk first, including read-only inspect/export.
- The annotation reader also UTF-8-lossy decodes `/Contents`, `/T` and `/Subj`.
  Inspection now decodes those bounded original raw PDF strings correctly;
  the Unicode annotation regression checks this independently of saved bytes.
- `FormFieldWrapper::bounds` passes PDF `[x0,y0,x1,y1]` corners to a constructor
  expecting `[x,y,width,height]`. Form inspection and geometry verification use
  the original extracted corner array and subtract the origin explicitly.
- Form value/property tests establish saved field dictionaries, not appearance
  stream regeneration. Ordinary upstream fills set `/NeedAppearances`, and
  existing `/AP` streams may remain stale; property updates likewise do not
  regenerate them. Viewer-independent visual form fidelity remains unverified.
- The upstream annotation reader can skip malformed or direct annotation
  entries. Mutation reconciliation rejects a serialized inventory mismatch;
  read-only annotation enumeration is not proof that no other entries exist.
- `outline.rs:59` traverses sibling references without a visited set. The
  bounded object walk is used for the adapter's bookmark inspection alias.
- Existing-document outline/label replacement, arbitrary low-level object
  mutation, text replacement, DOM path/table editing and image insertion/replacement, radio-group insertion, form import,
  dynamic XFA, scripts/actions/media activation, real signatures, and rich
  multimedia annotation creation are not implemented here. Creation/query/
  security modules may expose separately verified operations; the shared
  capability matrix must distinguish these from this module's coverage.
- Saved derivatives retain structured warning category, source hash, one-based
  page (when upstream supplies it), message and spec section in
  `data.diagnostics`, alongside compatible warning strings. The upstream
  thread-local warning sink can still have attribution limitations across
  multiple live documents; no global provenance guarantee is claimed.

## Test record

`tests/oxide_edit_forms.rs` uses generated `lopdf` fixtures only. Its tests cover
page reorder/duplication/rotation/crop, merge/split text order, invalid ranges and
boxes, metadata, annotation CRUD and unsafe URI rejection, attachment bytes and
path/name rejection, form values/flags/removal, FDF/XFDF escaping, flattening,
static-XFA derivative acknowledgement and hostile XML rejection, and item/page/
output budgets. Additional tests cover text overlays and rejection of defective
replacement, exact match preconditions, the pinned black-mask content regression,
image-bounds persistence, recoverable whiteout content, cyclic bookmarks/AcroForm
reads, ordered annotation identities and unknown dictionary keys, UTF-16 names,
hierarchical field removal, independent widget geometry/appearance checks, rejection
of signature-marked appended merge inputs, and
preservation of unrelated annotations.
The helper snapshots every input to assert it remains unchanged.

The integration owner runs the tests in the shared Cargo target; this module
does not run concurrent Cargo jobs. `full-tests-03.log` recorded 14/17 passes and
three concrete serialization/encoding failures: annotation removal retained an
entry, a UTF-16 attachment name became mojibake, and updated read-only flags were
absent. Those original assertions remain unchanged. `full-tests-05.log` recorded
16/17 passes: all three repairs passed, exposing the next assertion that a form
field requested for removal remained present. The removal repair and additional
regressions then produced 20/22 passes in `full-tests-06.log`: UTF-16 annotation
inspection and form geometry verification exposed the two reader defects above.
Those assertions remain unchanged and both reader corrections are implemented;
results for the current 23-test suite are pending.
No commit, PR,
merge, publication or release is performed by this worker.
