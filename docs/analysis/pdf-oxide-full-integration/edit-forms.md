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
| `edit.annotations` | `action: inspect`, `flatten`, or `edit` with ordered `changes`. Changes add, update contents/rectangle/RGB, or remove an index. Rectangle arguments use `[x,y,width,height]`. Flatten requires a normal appearance stream for every selected annotation; widget flattening uses `forms.flatten`. | `get_annotations`, `get_page`, `AnnotationWrapper` setters, `PdfPage` add/remove, `save_page`, `flatten_page_annotations`; explicit `lopdf 0.45.0` serialized-object repair | CRUD saved/read tests. All changes to one page share a DOM instance. The pinned writer omits original-annotation updates/removals; the adapter reconciles exact original/addition inventory and changes only requested dictionary keys/page arrays. Flatten separately verifies raw selected Annots are empty and nonselected annotation dictionaries are unchanged after saving. |
| Annotation creation kinds | `text`, `free_text`, `highlight`, `underline`, `strikeout`, `squiggly`, `square`, `circle`, `stamp`, `link_uri`, `link_page`. | Corresponding pinned writer annotation builders | Plain text and HTTP(S)/internal-page links only. No JavaScript, Launch, submit action, remote-file action or rich-media payload. No link is fetched. Other annotation creation variants remain unsupported. |
| `edit.attachments` | `action: add`, `files: [{input:1,name:"fixture.txt"}]`; or `action: extract`. Additional input slots contain explicit bytes. | `embed_file`, `extract_embedded_files`, bounded object walk and original PDF text-string decoder | Exact byte round-trip, safe generated extraction names, traversal rejection. Original UTF-16/PDFDocEncoding file names are decoded before associating upstream extracted bytes; ambiguous lossy-name projections reject. Source names are metadata, never paths. Existing EmbeddedFiles trees and indirect Names dictionaries reject additions to prevent pinned writer data loss. |
| `edit.bookmarks` | `action: inspect`; flat records contain object ID, depth, title, raw destination identity, action presence. | Pinned object reader and PDF text-string decoder | Bounded depth/items and reference-cycle detection. Does not call the unbounded `get_outline` sibling walk or invent page numbers for unresolved destinations. Existing-document bookmark modification is not exposed by `DocumentEditor`. |
| `edit.content` | Ordered ASCII Helvetica `add_text` changes with page/rectangle/text/size/optional RGB. Typed `replace_text` requests return `Unsupported` after parameter/match checks. | `PdfPage::add_text`, `find_text_containing`, `DocumentEditor::save_page` | Overlays round-trip text. Replacement is disabled: pinned `set_text`/`save_page` paints black redaction masks even while extracted replacement text looks correct. A direct upstream regression preserves that evidence; text extraction alone is insufficient to enable it. |
| `edit.image_bounds` | `changes: [{page,name,rect}]` using `[x,y,width,height]`. Name must identify exactly one direct image occurrence. | `get_page_images`, `set_image_bounds` | Reopens output and compares each saved bounding rectangle. Repeated, missing and indirect/nested occurrences that cannot be identified exactly reject. |
| `edit.whiteout` | `regions: [{page,rect}]` with PDF corner boxes and `acknowledge_text_remains: true`. | `erase_region` | Explicit visual overlay only. Tests prove underlying text remains recoverable. Never described as removing confidential content. |
| `forms.inspect` | Empty parameters; stable fully qualified names, typed values, bounds, flags, tooltip, MaxLen, alignment, parent data, XFA presence. | Bounded shared AcroForm tree walk, `DocumentEditor::get_form_fields`, `FormFieldWrapper`, `has_xfa` | Cyclic/repeated nodes, excessive depth/items and duplicate names reject before unbounded upstream field recursion. Signed-source inspection remains available. Does not report the upstream default/first-widget page number as exact provenance. |
| `forms.add` | `fields` with name/page/rectangle/widget plus optional read-only/required/tooltip. Widget kinds: text, checkbox, combo, list, inert push-button, empty signature widget. Any existing AcroForm rejects. | Pinned widget builders, `add_form_field` / `add_form_field_hierarchical`; explicit `lopdf 0.45.0` property repair | Saved names and requested flags/tooltip are checked. Adding to an existing form is disabled because the pinned rebuild drops unique catalog properties and can drop existing fields. No silent renaming of duplicate fields. Creating an empty signature widget is not signing. |
| `forms.fill` | `fields` map to typed `{kind,value}`: text, boolean, choice, multi_choice, none. Changed values reject when the target field or descendant widget retains a non-null `/AP`. | `set_form_field_value` | Reopens saved PDF and compares every requested typed value. Rejects stale-appearance cases, incompatible types, read-only fields, MaxLen overflow, unsupported signature values, missing names, and multi-selection on a single-choice field. Unchanged values do not invoke the upstream setter or set NeedAppearances. |
| `forms.update` | Bounded unique field list with optional read-only, required, tooltip, rectangle, MaxLen, alignment, RGB background/border, border width. Style/geometry/MaxLen requests reject existing field or descendant-widget `/AP`; nonvisual flags/tooltip remain available. | Corresponding `set_form_field_*` APIs; explicit `lopdf 0.45.0` property repair | The pinned writer omits staged field properties; the adapter writes requested dictionary keys against extracted indirect field identities and reopens the result. Geometry updates also require one merged field/widget identity; every requested property is checked after saving. Arbitrary appearance strings, raw flags and event scripts are not accepted. |
| `forms.remove` | `names` list of terminal fields. | `remove_form_field`; explicit `lopdf 0.45.0` reference repair | Reopens output and requires requested names to be absent. The corrective pass unlinks exact field/widget identities from AcroForm Fields, parent Kids, calculation order and page Annots, preserving unrelated annotations and siblings. Parent-container, ambiguous/direct-widget and signature-field removal reject. This does not erase orphaned values from the file. |
| `forms.flatten` | Empty parameters; whole document, or explicitly selected pages. An AcroForm with both `NeedAppearances: true` and an appearance-bearing field/widget rejects before flattening. | `flatten_forms` / `flatten_forms_on_page` | Full flatten must leave no AcroForm fields. The guard prevents baking stale appearances saved by an earlier fill. Missing appearance warnings are retained; accepted flattening does not prove complete visual appearance. Signature widgets reject. |
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
- Annotation flattening computes `flatten_data` only when appearance extraction
  returns a nonempty list (`editor/document_editor.rs:2452`). Only that branch
  removes the selected page's Annots array (line 2843), so a link-only page with
  no AP returns a derivative retaining interactive links. The adapter rejects
  selected annotations without a resolvable normal appearance stream, including
  appearance-state dictionaries that are not qualified here. It verifies raw
  selected annotation removal and unchanged nonselected dictionaries after
  saving instead of relying on an enumeration API that can skip entries. Two
  synthetic pages preserve native failure evidence and exercise accepted AP
  flattening with an untouched link lacking AP on the nonselected page.
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
- The pinned new-field writer at `editor/document_editor.rs:2205–2257` rebuilds
  the AcroForm instead of preserving its dictionary. It copies old Fields only
  from an indirect AcroForm with a direct Fields array; inline AcroForm fields
  disappear, and unique dictionary keys/resources/calculation order can be
  replaced even in the indirect case. `forms.add` therefore rejects any existing
  AcroForm. Direct pinned-library regressions retain both cases and independent
  catalog/field sentinel assertions; there is no verified preservation repair.
- The existing-value writer at `editor/document_editor.rs:1698` changes `/V`
  (and button `/AS`) and sets `/NeedAppearances` without regenerating `/AP`.
  Property repair also does not rebuild appearance streams. Changed fills and
  requested rectangle, MaxLen, alignment, background, border color or border
  width updates therefore reject targets with any non-null `/AP`, including
  separate widget children. A bounded reader follows only field/widget Kids
  edges and rejects ambiguous indirect identities. The conservative style gate
  also rejects an explicitly repeated style value. Read-only/required flags and
  tooltips remain available; tests require source/catalog sentinels and original
  appearance bytes to survive. No-op fills skip the pinned setter.
- The flattener at `editor/document_editor.rs:6089` regenerates some text only
  when a value was staged in the same editor session (`modified_widget_text` at
  line 6181). After saving and reopening, it prefers the old `/AP`; the synthetic
  regression records `/V = After`, an unchanged stream painting `Before`, and a
  separate flatten that paints `Before` after removing all form fields. The
  adapter rejects flattening when NeedAppearances is true and existing AP is
  present. This conservative document-level check also applies to page-selected
  flattening. A same-session fill-plus-flatten control exercises the limited
  private regeneration path without substituting flattened output for an
  interactive fill. There is no public verified general in-place appearance
  regeneration path; missing AP and unmarked preexisting stale AP remain visual
  qualification limits, not evidence of correctness.
- Independent Poppler rendering of the five form fixtures confirms that the
  same-session control paints `After`, while the separate-session flatten paints
  stale `Before`. The same-session control loses the original border/background,
  so it is not a qualified visual fallback. Owner evidence is in
  `/workspace/tpe-pdf-oxide-evidence-20261015/forms-poppler-001/receipt.json`.
  The original 28/29 test failure remains in `forms-guards-02.log`: native text
  extraction omitted the entire control field, rather than inserting spaces.
  `document.rs:2823` peeks at the first raw `/Subtype` without dictionary depth;
  the generated Form's inline Helvetica `/Subtype /Type1` appears before its
  enclosing `/Subtype /Form`, causing `extractors/text.rs:6782` to skip the Form.
  The regression independently requires the invoked stream's exact `After` text,
  retains cold native text/character omission, and requires a fresh document with
  the exact Form parsed before the faulty peek to expose `After` as both text and
  glyphs. This diagnostic preload is not a production query workaround.
  The bounded native query receipt is `forms-native-query-001/receipt.json`:
  executable `git-e36807e-dirty`, SHA256
  `ffb96561450fbc21cf55d74488cee5096ed13cb78f3c8923cb52b574eacdb66e`;
  explicitly an unqualified dirty build, not the final combined-source result.
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
Six further tests cover existing inline/indirect AcroForm rejection, direct
pinned catalog-loss evidence, merged/separate-widget stale-appearance rejection,
unchanged values/nonvisual property preservation, direct upstream stale fill
followed by separate flatten, and flattening of an existing complete appearance.
All earlier success assertions remain intact; the new tests distinguish rejected
adapter operations from preserved upstream failure evidence.
Two more tests cover native no-AP annotation flattening that retains interactive
links, explicit adapter rejection, and selected-link removal with nonselected
no-AP link and page-content preservation.
The helper snapshots every input to assert it remains unchanged.

When the owner supplies `TPE_OXIDE_FIXTURE_CAPTURE`, these tests retain bounded
synthetic PDFs under that directory: `forms-appearance-source.pdf`,
`forms-appearance-upstream-filled.pdf`,
`forms-appearance-upstream-separate-flattened.pdf`,
`forms-appearance-upstream-same-session-control.pdf`, and
`forms-appearance-control-flattened.pdf`. The source paints `Before` in one
Helvetica widget, with unrelated page text and catalog sentinels. The direct
upstream request sets that field to `After`. The owner completed the limited
Poppler comparison described above; it does not establish general fidelity.
Additional captures are `forms-appearance-native-observations.json`,
`annotations-no-ap-source.pdf`, `annotations-no-ap-upstream-flattened.pdf`,
`annotations-selected-ap-source.pdf`, and `annotations-selected-ap-flattened.pdf`.

The integration owner runs the tests in the shared Cargo target; this module
does not run concurrent Cargo jobs. `full-tests-03.log` recorded 14/17 passes and
three concrete serialization/encoding failures: annotation removal retained an
entry, a UTF-16 attachment name became mojibake, and updated read-only flags were
absent. Those original assertions remain unchanged. `full-tests-05.log` recorded
16/17 passes: all three repairs passed, exposing the next assertion that a form
field requested for removal remained present. The removal repair and additional
regressions then produced 20/22 passes in `full-tests-06.log`: UTF-16 annotation
inspection and form geometry verification exposed the two reader defects above.
Those assertions remain unchanged and both reader corrections are implemented.
The owner reported strict full-workspace Clippy passing in `full-clippy-06.log`.
The 29-test form run passed 28, with the control's newly diagnosed native Form
peek defect causing the remaining failure. All new form guards and original
success assertions passed. The precise control regression and two annotation
flattening tests bring this worker's suite to 31 tests; results for the changed
source are pending the owner's next combined run.
No commit, PR,
merge, publication or release is performed by this worker.
