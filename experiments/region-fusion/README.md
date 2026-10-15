# Recorded-review-assisted region fusion experiment

This opt-in pilot produces a **selected span projection** from an immutable
`lopdf` baseline and candidate extraction artifacts. It selects a candidate only
when a caller explicitly trusts an independently recorded regional transcription,
the supplied render bytes match that record, and a complete geometric scan finds
one unique baseline span and one unique candidate span inside the reviewed region.
All other baseline spans retain their exact text and array order.

This is the functional increment after the separate `region-evidence` contract.
It is not automatic accuracy discovery or a production `ExtractionResult`
replacement. The new nested Cargo workspace depends only on that standalone
contract plus serialization libraries; production parsers, manifests, routing,
native adapters and output schemas remain unwired. Rust 1.98.1 is the declared
minimum; the repository toolchain selects the executable. Cargo generated this
workspace's independent lockfile and implicit package version.

## Run the recorded-artifact demonstration

```sh
cargo test --locked --manifest-path experiments/region-fusion/Cargo.toml
cargo clippy --locked --manifest-path experiments/region-fusion/Cargo.toml --all-targets -- -D warnings

fixture_root=experiments/region-fusion/fixtures
source_hash=97779d74bc9f286de6f4578ce86f47a266d7f263a28ce37b955b61b7333f2a1f
cargo run --locked --manifest-path experiments/region-fusion/Cargo.toml --bin fuse_artifacts -- \
  --source "$fixture_root/positive-stream-cmap.pdf" \
  --baseline "$fixture_root/captured/positive-stream-cmap/lopdf/$source_hash.json" \
  --candidate "$fixture_root/captured/positive-stream-cmap/pdfium/$source_hash.json" \
  --review "$fixture_root/approved-review.json" \
  --trusted-review-sha256 926b144c886d90734390df8e951026cc0115610fdb68fb28660ddea7cd48889b \
  --trusted-by reviewed-fixture-policy \
  --render "$fixture_root/captured/positive-stream-cmap/page-1-mupdf.png" \
  --output /tmp/reviewed-region-projection.json
```

The source PDF contains two separated regions. Actual captured `lopdf` output
retains `BASELINE REGION RETAINS THIS TEXT.` and emits replacement characters in
the other span. The captured PDFium candidate has `RECOVER ALPHA 2026`, matching
the predeclared source text and separate-engine MuPDF render. The derived view
keeps the first span from `baseline` and selects the second from `candidate`.
It retains the original baseline's `partial` status and its unchanged artifact,
including every warning. It cannot promote that status to `complete`.

The fixture's truth was specified before extraction, and its visible text was
reviewed against the MuPDF image. Reviewers were AI agents shown those strings;
this was neither blind nor human annotation. The accepted review is an input to
selection, so this demonstration is **not an independent holdout accuracy result**.
See the fixture's `visual-review.json`, source truth, render and capture manifest
for provenance and limitations.

`fuse_artifacts` reads existing files only. It checks byte budgets while reading,
rejects nonregular input, and atomically publishes a new output file without
overwriting existing paths. Source files have no default size ceiling; an optional
`--max-source-bytes` provides a caller-selected limit. A repeated demonstration
must choose a new output pathname or explicitly remove its previous result.

The separate Linux `run_extraction.py` invokes actual supplied extractor binaries
with a durable `lopdf` pass first and an explicitly requested PDFium pass. Its
journal and child-process limits are runner behavior, not guarantees supplied by
this library. Run `uv run --no-project python
experiments/region-fusion/run_extraction.py --help` for its exact CLI and limits.

For a fresh extraction using explicitly provisioned local executables:

```sh
fixture_root=experiments/region-fusion/fixtures
PDFIUM_DYNAMIC_LIB_PATH=/absolute/path/to/libpdfium.so \
uv run --locked python experiments/region-fusion/run_extraction.py \
  --source "$fixture_root/positive-stream-cmap.pdf" \
  --out /tmp/reviewed-region-job \
  --tpe /absolute/path/to/tpe \
  --fusion /absolute/path/to/fuse_artifacts \
  --pdfium \
  --review "$fixture_root/approved-review.json" \
  --trusted-review-sha256 926b144c886d90734390df8e951026cc0115610fdb68fb28660ddea7cd48889b \
  --trusted-by reviewed-fixture-policy \
  --render "$fixture_root/captured/positive-stream-cmap/page-1-mupdf.png"
```

Use a new output directory for each invocation. This fixture's successful
reviewed job intentionally exits 1 because its baseline remains Partial. Await
runner termination, then inspect `journal.json` for `state: "completed"` and
verify the recorded `fused.json` digest. A live tentative file or completed
journal is not irrevocable while final cancellation handling is still running.
Omit `--pdfium` for the durable baseline-only path.

## Review and API contract

`ReviewBundle` binds exact source SHA-256/size to a list of `RegionReview` records.
Each record supplies:

- Page array index and page number; positive page dimensions and rotation;
  a finite, positive, in-page region in an explicit canonical frame.
- Eligible candidate engine name and optional version, expected baseline and
  candidate text, and a separately recorded adjudicated transcription. Equality
  is exact; selection performs no normalization or text repair.
- SHA-256/size of supplied render bytes and review provenance: reviewer, method,
  renderer/config digest, record reference and the basis for the coordinate frame.

The static review contains **no span indices, extraction artifact hashes or
path-dependent backend configuration digest**. It approves a source/render
transcription rather than a particular execution artifact. Real JSON includes
variable timings, while PDFium configuration can include its library path.
Geometric matching independently discovers the actual span indices; the result
records actual source, extraction hashes, engine/config identities and locators.
Engine/version filtering is an eligibility rule, not proof of runtime provenance.

`TrustedReview::accept_explicitly(bytes, expected_digest, trusted_by, limits)` is
the only constructor for the non-deserializable trusted wrapper. The expected
digest and authority label must come from caller policy outside the PDF, parser
output or review file. Computing a digest from an untrusted file and automatically
accepting it would defeat this boundary. The shipped example uses an explicitly
pinned, reviewed fixture hash. This is not a digital signature or proof of
authorship. Strict typed deserialization rejects duplicate and unknown members.

Load exact render bytes into `WitnessStore`. First validate source and extraction
artifacts with the existing `region-evidence` library, then call:

```text
select(&validated_artifacts, Some(&trusted_review), &render_store,
       &caller_limits, &cancellation_flag) -> DerivedView
```

Passing no trusted review produces baseline rows only. The original validated
store remains unchanged. Keep its complete baseline and alternative bytes along
with review/render bytes when persisting the derived view: its hashes and locators
are references, not embedded substitutes for the evidence.

## Selection policy

The serialized policy is `trusted_visible_transcription_v1`. It requires one
eligible `complete` or `partial` candidate attempt. Multiple eligible attempts
abstain; `failed`, `cancelled` and `resource_limit` attempts cannot supply a selected
span. Status labels are preserved declarations, never correctness scores.

Before selection, every pair of reviewed rectangles on the same page is checked.
Overlapping reviews all abstain, independent of input order. For each remaining
review, both artifacts must declare the same reviewed canonical frame and exactly
match the recorded page geometry. Each complete span array is scanned. Any
missing, nonpositive or off-page span box anywhere on that page prevents that
correspondence. A span crossing the region boundary, no contained span, duplicate
or multiple contained spans, or an empty contained text atom also causes abstention.
Repeated text elsewhere on a page is not identity and does not replace this scan.

For a unique pair, both actual texts must match their expected review values.
Equal baseline/candidate text retains baseline. A differing candidate is selected
only if it exactly matches the adjudicated transcription and the baseline does
not. A longer candidate, matching status or extra output never confers authority.
An unsupported or ambiguous region leaves every baseline span intact.

`DerivedView.pages` contains actual text and baseline/selected `SpanLocator`s in
baseline array order, with the review ID for selected rows. It is not reconstructed
reading order. Original `PageText.text`, lines, metadata, citations, links, figures,
warnings and timings remain exclusively in the unchanged original artifacts.
Consumers must not relabel this span projection as a consistent fused schema-5
document or combine selected text with stale baseline layout metadata.

## Bounds and limitations

- The supported geometry policy uses positive page dimensions and zero-origin
  canonical PDF user-space page bounds. Unknown transformations remain unknown.
  The CLI declares frames only from the accepted review's explicit frame basis;
  this is an external assertion, not automatic proof of a native transform.
  Nonzero crop origins and geometry requiring unrecorded transforms abstain or
  require a later contract; this pilot makes no general rotated/cropped PDF claim.
- Render bytes are verified once per distinct digest/size. The library does not
  decode images, prove which PDF produced them, or independently read text from
  pixels. Source/frame/render association and transcription remain trusted review
  assertions. Content hashes establish identity/integrity, not semantic accuracy
  or trusted execution. Existing spans are normalized extraction text, not raw
  PDF glyph bytes or independently verified character-code mappings.
- Caller limits bound review bytes/counts, individual/total render bytes,
  candidate attempts/bytes, projected spans and charged comparisons. Charges
  include review-pair checks, attempt eligibility checks and actual scanned spans.
  These are accepted-input/work bounds, not peak-RSS or hard wall-time promises.
  Default limits are declared in `Limits::default`; no untrusted record can raise
  them. Input buffers and foundation validation already exist before selection.
- Cancellation is cooperative. It is checked between expensive operations and
  at every charged comparison, with a final check before publication. An in-flight
  hash or JSON parse is not interruptible, and initial baseline parsing/projection
  can precede the first check. Cancellation or exhausted work budgets discards
  **all** proposed selections for the run. A projection that exceeds its own span
  bound has `pages: null`; the complete original baseline artifact remains the
  explicit fallback. A truncated prefix is never published.
- Foundation validation still requires exactly the same complete ordered selected
  page list in every artifact. This pilot does not run OCR, choose additional
  engines, merge split spans, reconstruct reading order, or supply citations,
  figure or URI selection. The Linux runner's bounded child lifecycle is a separate
  opt-in component; the library itself does not launch or supervise workers.

Integration tests consume the captured real artifacts and pinned review for the
positive case. The actual missing-mapping negative rejects PDFium's incorrect
text, the actual named-CMap control retains equal baseline text, and the actual
repeated-text source selects only its reviewed occurrence. Those additional
cases explicitly accept their predeclared generator truth as a test-only policy;
they do not alter or extend the shipped approved visual review. Explicitly labeled
in-memory mutations exercise trust rejection,
source/page/render mismatches, unknown and missing geometry, crossing/duplicate
atoms, repeated text, conflicting review order, competing and failed attempts,
longer/equal alternatives, cancellation, and budget exhaustion after an earlier
proposal. Those mutations are adversarial test inputs, not new extractor results.
