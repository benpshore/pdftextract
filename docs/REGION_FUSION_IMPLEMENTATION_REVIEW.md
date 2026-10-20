# Region correspondence and selection implementation review

Review date: 2026-10-04 UTC. Worktree:
`/workspace/pdftextract-region-fusion`. Frozen foundation: PR #202,
`fb623bec2c8c68341a45e1f220ff0f46245dc039`. Production source remains
`e27e1fb28a5b40dbca7f517c6fe396e6f11ed4ec`. This review owns this new document only;
it does not change the frozen foundation, production parsers/configuration,
native providers, runtime pins or GitHub state. Security/release owners retain
their separate scopes.

## Initial design decision

A caller-trusted witness can support an **externally adjudicated region
projection**. It cannot by itself establish automatic transcription correctness.
A witness bound to PDF SHA-256, physical page, rectangle and exact text prevents
some substitutions and replay; those bindings do not prove its text is true.
A caller could copy an incorrect candidate into such a witness and satisfy all
four fields. Provenance must therefore identify the external adjudication and
its trust boundary, not re-label it an independently verified native mapping.

Using expected fixture text to select an alternative is legitimate for testing
the mechanics of that trusted-adjudication consumer. Reporting the same match
as independent extraction accuracy or general recovery success would be
circular. Holdout labels used to score automatic accuracy must be unavailable
to the selector. A frozen synthetic source generator can establish known fixture
content before extraction; candidate output must not be used to invent or tune
that content retrospectively. Rendering with PDFium is useful visual evidence,
but it shares the candidate's PDFium engine and is not independent-engine
corroboration.

The static review and per-run receipt should be distinct. The static review binds
the source, physical page, frame/page geometry, region, expected transcription and
render/adjudication evidence. The receipt binds the actual baseline and candidate
artifact hashes plus the approved review digest and exact selected locators.
Timing-bearing extraction JSON changes between executions, so requiring its
artifact hashes in an earlier static transcription review would make honest
re-execution needlessly invalidate that review. The receipt must still bind the
actual bytes used on each run. An explicit caller-accepted review digest is the
trust input; a serialized `authority: "trusted_review"` label cannot approve itself.

The feasible narrow increment is one whole baseline span to one whole candidate
span at an unambiguous witnessed occurrence, keeping the original baseline and
all alternatives. The projected text must come from the candidate artifact,
never be injected from the witness. No splitting, concatenation, normalization,
new ordering or longest-text/majority/confidence rule is justified by this step.
The initial positive qualification should stay with simple visible text and
known page frames. Hidden text, ActualText, ambiguous Unicode/glyphs, rotations
and crop origins need their own representation and qualification.

This follows the frozen [region design](REGION_FUSION_DESIGN.md) and
[FFI review](FFI_REGION_SAFETY_REVIEW.md). Schema 5's normalized spans and
producer-declared boxes do not retain the original character codes, source
font/glyph mapping witnesses or native-to-PDF transforms needed for a general
automatic source-mapping rule.

## Required implementation invariants

| Area | Required behavior |
| --- | --- |
| Witness authority | Distinguish the exact witness artifact from its external caller approval. Record source/page/frame/region, exact target representation, and adjudication method/evidence. A candidate's `verified: true` or matching copied text cannot self-authorize selection. A hash authenticates bytes, not an adjudicator. |
| Source and artifact identity | Resolve all locators against immutable, validated source and artifact bytes. Do not reopen a user pathname between candidates and assume identity. Record exact baseline, alternative and witness hashes; never overwrite baseline artifacts or the source. |
| Atomic membership | Select only a whole existing baseline atom and whole existing alternative atom. Use artifact/page/array-index identity, never `seq` or string equality. Every baseline atom has exactly one retained/projected owner. |
| Geometry | Require finite positive-area boxes and compatible explicit frames. A region must not partially consume another text atom or a candidate crossing its boundary. Missing geometry on the affected page prevents claims of exhaustive unique spatial ownership. Matching width/height alone is insufficient transform evidence. |
| Ambiguity | Duplicate/overlapping same-position spans, competing admissible candidates, conflicting witnesses and colliding consumed baseline atoms cause abstention. Distinct repeated text outside the witnessed region remains a distinct retained occurrence. Iteration order must not resolve ties. |
| Selection | Candidate text must exactly satisfy the externally adjudicated target. No witness text may become output when no actual candidate supports it. If baseline already satisfies the target, retain it. Wrong candidates of equal or greater length must not gain eligibility. |
| Partial status | Structurally valid Partial extraction may contribute a witnessed local span, but all original uncertainty remains and document/page status must not be upgraded. Schema 5 cannot prove localization of every mapping warning. Terminal failed/cancelled/resource-limited attempts are not eligible successful replacements. |
| Derived data | Emit a separate projected span/decision view. Do not mutate `PageText.text` while retaining stale lines/chunks/citations/metadata under a claim of a fully coherent fused schema-5 result. Keep baseline links, images, warnings and derived fields attributable to baseline. |
| Work bounds | Count actual span/candidate/comparison work against one caller-owned budget, with checked arithmetic and stop checks inside potentially large loops. Accepted-input byte limits do not prove peak RSS or native process limits. No new arbitrary production PDF limit is implied. |
| Cancellation and resource stops | A stop must leave baseline available and explicit unresolved/abstained outcomes. Prefer no-promotion rollback for a stopped projection unless already committed decisions have a clearly defined partial-run contract. A cancellation flag field alone is not cancellation execution. |
| Serialization | Reject duplicate keys and malformed/foreign/replayed identifiers. Any serialized result must identify the adjudication policy and exact inputs; a derived selection is not a new native parser result. |

## Adversarial acceptance cases

1. One source page contains good neighboring text and an unresolved target. The
   actual candidate supplies the witnessed target; the projected view retains
   the neighbor's exact baseline atom and all baseline side evidence.
2. Equal-length and longer wrong candidates abstain. A witness matching baseline
   produces no unnecessary replacement; no witness produces no replacement.
3. The correct-looking witness is unapproved, belongs to another source/page/frame,
   or was altered after approval. It cannot authorize selection.
4. Repeated text at separate positions stays occurrence-specific. Duplicate or
   overlapping occurrences inside the same region, crossing candidate spans and
   overlapping target regions abstain rather than silently deleting evidence.
5. Unknown/reversed/nonfinite/zero-area geometry and ambiguous page ownership
   abstain or reject. An unlocated text atom cannot be assumed outside the target.
6. A Partial baseline/candidate retains status and warnings after a local
   adjudication. Failed/cancelled/limited candidates cannot replace it.
7. Cancellation or exhaustion before and during correspondence leaves baseline
   and alternatives intact, returns an explicit stop reason, and produces no
   unreported accepted work after the shared budget.
8. Altered source/artifact/witness bytes, duplicate members, out-of-range locators
   and conflicting witness claims fail before projection.
9. Text payload is taken from retained candidate bytes. The selector cannot
   synthesize a third transcription from witness text or multiple alternatives.
10. Source/baseline output aliasing cannot truncate an input; rereading a changed
    pathname cannot substitute a different PDF unnoticed.

The witness-driven tests establish consumer behavior, preservation and
abstention. They do not constitute a native security scan, a general accuracy
benchmark, production service deployment or proof of correct PDF rendering.

## Implementation and validation checkpoint

The reviewed implementation is a functional, deliberately narrow consumer of
externally approved transcription. The reviewed checks support this isolated
experiment. It is not authorization to replace production
extraction, infer unknown coordinate frames, or claim automatic correctness.
An independent adversarial specialist reached the same design conclusion. This
reviewer read the implementation and tests; no scan or production build was
duplicated.

### Rust correspondence, ownership and output

[`TrustedReview::accept_explicitly`](../experiments/region-fusion/src/lib.rs)
requires a caller-supplied exact digest and nonempty authority. The private
`TrustedReview` is not deserializable. Its typed nested review objects reject
duplicate/unknown members. This separates the review bytes from caller approval;
it does not verify the caller's identity or the truth of the review. Supplying a
digest computed from an arbitrary received review would still be self-approval.

`select` borrows the frozen foundation's validated, owned artifact store. It
parses owned values and creates a separate projection; no native pointers,
borrowed native buffers, string slicing by foreign offsets, or new unsafe Rust
are introduced. The complete original evidence sidecar and immutable artifact
references remain in the result. A selected row records its actual baseline and
candidate artifact/page/array-index locators plus the review ID. Its text comes
from the candidate JSON, never from the adjudication string. The explicit policy
is `trusted_visible_transcription_v1`.

`evaluate` resolves candidate eligibility from backend/version and rejects more
than one matching attempt. The prepass marks both sides of overlapping review
rectangles as conflicts. `unique_span` inspects every span on each affected page;
missing, zero-area, off-page or malformed geometry prevents selection even when
the bad span's text would otherwise seem irrelevant. It rejects boundary-crossing
or multiple contained spans before comparing text. Separate occurrences outside
the region do not acquire ownership through text or `seq` equality. Frame and
page geometry are explicit requirements; their truth remains an external review
claim. The CLI's frame declaration does not derive a native-to-PDF transform.

Candidate Complete and Partial outcomes are eligible for externally adjudicated
local selection. Terminal Failed, Cancelled and ResourceLimit are ineligible.
All original global status and warning evidence is retained: local selection
does not upgrade a Partial document. The projection is in baseline span-array
order and does not pretend to regenerate `PageText.text`, lines, citations,
chunks, links or images.

Comparison charges cover review pairs, attempted candidates and scanned spans.
Budget exhaustion rolls back proposed selections, including a proposal already
created before a later comparison exhausts the budget. Cancellation checks occur
between major stages, in the comparison counter and around proposal publication.
An observed cancellation after proposal construction restores baseline rows.
The initial baseline parse/projection and each in-flight JSON parse/hash are not
preempted: this is cooperative cancellation and bounded accepted input, not an
absolute latency or peak-RSS guarantee. Each distinct render digest/size is
hashed once per selection call, avoiding repeated whole-render hashing for every
review region.

The standalone CLI bounds reads and output writes, validates the foundation
contract before selection, and publishes a fully written file through an
exclusive hard link. Existing output files and symlinks cannot be overwritten.
Its regular-file prechecks are not a security sandbox against concurrent hostile
filesystem mutation; the supervised runner supplies private snapshot paths.

The fixture-only PDFium raster helper also remains an FFI boundary. It uses
explicit C signatures, checks document/page/bitmap handles, copies pixels while
the bitmap is alive, then destroys bitmap, page, document and library in that
order. Review found `FPDFBitmap_FillRect` declared as returning void despite the
bundled primary header declaring `FPDF_BOOL`. The helper now uses `c_int` and
rejects failure. Its fixed-size bitmap requires the exact expected stride before
`ctypes.string_at`, bounding that native buffer copy. These corrections do not
prove arbitrary native pointers safe. The fixture owner rerendered the positive
PDF to a temporary file and reported an identical PNG hash, with all 56
non-script evidence files unchanged. This helper is not a general untrusted-PDF
rendering service.

### Runner issues found and corrected during review

These were concrete pre-publication implementation findings, not claims of a
vulnerability in the frozen foundation or production parser.

| Finding | Consequence | Required correction reviewed in source |
| --- | --- | --- |
| An unconditional new 64 MiB source ceiling | Unrelated input rejection despite a caller having sufficient resource budget. | Optional caller source cap with no default size ceiling; pass the actual snapshot size to supervised extraction and the fusion CLI. |
| Untyped variadic `ctypes` call to `prctl` | ABI argument widths were implicit. | Explicit `c_int` result/first argument and four `c_ulong` arguments. This remains a Linux FFI boundary. |
| Reaping only the controller process group | A native worker in its own session/group could be omitted after adoption. | Single-job subreaper uses `waitpid(-1, WNOHANG)` and identifies adopted children by `/proc` parent PID; separate-session descendant fault test. |
| Cleanup bound recorded only as an annotation | A successful child exit could allow later work while cleanup remained incomplete. | Raise explicit `cleanup_failed` after recording the journal; stop the run. |
| Publishing `fused.json` before validating the derived outcome | A resource-limited or cancelled projection could acquire a final-looking filename. | Check bounded pending bytes, digest and `evaluated` outcome before publication; demote tentative output on a caught failure. |
| Cancellation during final journal persistence | The last pre-write check alone could report completion after receiving a stop signal. | Post-persistence completion check and committed-state flag; real SIGTERM injected during final directory sync returns cancellation and demotes the tentative output. |

The runner snapshots the PDF into an exclusively created private job directory,
checks descriptor metadata while copying, hashes exact bytes, and rechecks
snapshot digests before and after extraction. The source path supplied by the
user is never an output target. It persists and fsyncs baseline JSON and its
journal receipt before optional review/candidate work. Candidate crashes, missing
outputs, invalid status/exit combinations, output floods and later adjudication
errors therefore retain the completed baseline. This is protection against
ordinary changes and failures, not immutability against a malicious process with
the same user privileges.

The shared monotonic deadline is not reset per pass. Native work is separately
supervised; diagnostic capture and individual output-file sizes are bounded.
These controls are not a system-wide memory, total-disk or hostile-executable
sandbox. Regular-file reads, filesystem writes/fsync, process creation and a
process stuck in uninterruptible kernel I/O cannot be given a hard wall-time
bound by these Python checks. Abrupt SIGKILL cannot update a journal; the last
durable receipt remains the available state. Consumers must await runner
termination and then inspect its journal: a tentative filename is insufficient,
and live polling can briefly observe a completed journal before the final
cancellation checkpoint demotes it. Neither the journal nor these tests establish
an external distributed transaction or crash-recovery protocol.

### Actual fixture evidence and limits

The final deterministic positive PDF is 10,736 bytes, SHA-256
`97779d74bc9f286de6f4578ce86f47a266d7f263a28ce37b955b61b7333f2a1f`.
The captured baseline reports lopdf 0.45.0, Partial, a good neighboring span
`BASELINE REGION RETAINS THIS TEXT.`, and 18 replacement characters at the target.
The captured candidate reports PDFium `dynamic-binding-0.8.37`, Complete, and the
target `RECOVER ALPHA 2026`. Exact records, native runtime identity and separate
control/negative/repeated-source observations are retained in the
[capture manifest](../experiments/region-fusion/fixtures/captured/manifest.json).

The approved review digest is
`926b144c886d90734390df8e951026cc0115610fdb68fb28660ddea7cd48889b`.
It references a separate-engine MuPDF 1.26.11 render, SHA-256
`3164d458097f50a137f5b555915c60dd7258d26b9ab97fab008e0781ce18c85d`.
The generator supplied the intended strings before extraction; reviewers were
given those strings when inspecting the images. The
[visual-review record](../experiments/region-fusion/fixtures/visual-review.json)
accurately labels this as nonblind AI source/visual review, not human annotation.
PDFium's own render is additional evidence from the candidate engine. The
negative empty-ToUnicode fixture demonstrates why a nonempty candidate alone
cannot authorize replacement.

This reviewer also opened the final MuPDF PNG and observed both exact declared
strings at separate vertical positions. That review likewise occurred with the
expected strings already known, so it is corroboration of the recorded example,
not blind accuracy evidence.

The selector tests use captured real engine artifacts plus explicitly synthetic
adversarial mutations. They check retained neighboring text, exact lineage,
unapproved/replayed/altered review rejection, unknown geometry, duplicate/crossing
spans, competing attempts, overlapping reviews in both orders, incorrect longer
text, equal-text preservation and budget rollback. Final tests also consume the
actual captured empty-mapping negative, equal-text control and repeated-source
fixture: the negative abstains and the repeated source selects only the reviewed
occurrence. Those tests explicitly approve predeclared generator truth as a
test-only policy; they neither extend the shipped visual approval nor provide
held-out accuracy. A separate repeated-text mutation test remains clearly
distinguished from actual backend capture.

The runner fault tests launch real Linux processes using fake extraction/fusion
executables. They verify process/signal/journal behavior, not PDFium correctness.
The real-engine integration suite uses actual lopdf and PDFium for the positive
projection. Its injected crash case runs actual lopdf first, then aborts a
wrapper before the native candidate starts; it proves durable-baseline retention
after a candidate-controller crash, not a demonstrated crash inside PDFium.

### Validation evidence and source pins

This reviewer read `/tmp/region-fusion-cargo-test.log`: **16 Rust integration
tests and six CLI I/O tests passed**, with no ignored or failed tests. I also
read the successful `/tmp/region-fusion-clippy.log` and empty successful format
log; the implementation owner reported the commands used pinned Rust 1.98.1
and Clippy with warnings denied. The final parent execution log
`/tmp/region-fusion-python-tests.log` reports **28 passed in 5.07 seconds**:
22 process tests and six fresh real-engine tests under Python 3.14. The runner
owner also reported Ruff format/check passing.
Those process tests include separate-session descendants, controller crash,
SIGINT/SIGTERM, final-persistence cancellation, shared timeout, source tampering,
source/output limits and preservation of Partial status from either pass.

This reviewer read the parent's actual `/tmp/region-fusion-real-tests.log`:
**six passed in 2.13 seconds**. The suite performs actual lopdf/PDFium extraction,
baseline-only operation, unapproved-review rejection, the explicitly injected
candidate-controller crash and two synthetic candidate mutations. I inspected
the completed job's journal and projection under
`/tmp/pytest-of-agent/pytest-1/real-region-parent0/job`: attempts were baseline,
candidate, fusion; baseline stayed Partial and the final exit code was 1.
The first selected locator remained baseline span 0; the second selected actual
candidate span 1. Their runtime artifact hashes were respectively
`8755d02b2eaba1250e0c6ee91882e4089cf7ed51bfd195de577675b759fa6bf8`
and `5b6287c99de1993c37698926e3ade6e9a164dd6f95156a90cb55ccf2ecd59384`.
Those differ from static captured artifact hashes, as expected for fresh
timing-bearing JSON, while the approved source/review/render identities match.
The temporary execution paths are local observations, not checked-in fixtures.

Reviewed source fingerprints (SHA-256):

| File under `experiments/region-fusion/` | SHA-256 |
| --- | --- |
| `src/lib.rs` | `d830f47c3129f58ac075506f1b29ac4d483868dd6907eee4b984e6d910115583` |
| `src/bin/fuse_artifacts.rs` | `5e995561bacda194ddcb3628225ea5eef2da139a4a28fddfb31a10b34ecbf587` |
| `tests/selection.rs` | `7f5fcb5b3226ae086a963bb54d10bf0aa344321252c88fc42ee4b74040c6e1e0` |
| `run_extraction.py` | `f1886bcd6e17d67104a7ecc1e80ba4c281d96039944138ee046f63c30f8bd3ef` |
| `test_runner.py` | `dcbd2e3c6bf41001447ba3ba52dd7b4d320f57715d3652a87fe9b2c6a1977757` |
| `fixtures/capture.py` | `6488255d41404194271af46495d9551232499ca0e802b52e8c16e470c5047259` |

At these hashes, principal review locations are `src/lib.rs:135` (trust boundary),
`:401` (selection/rollback), `:644` (correspondence) and `:786` (whole-page geometry);
`src/bin/fuse_artifacts.rs:193` (external frame declaration), `:236` (bounded input)
and `:371` (exclusive publication); `run_extraction.py:43` (typed `prctl`), `:168`
(source snapshot), `:225` (supervision), `:311` (all-child reap), `:334` (cleanup
failure), `:474` (pre-publication output validation) and `:500` (completion
checkpoint); `fixtures/capture.py:41` (FillRect signature), `:69` (return check),
`:76` (fixed stride) and `:82` (handle cleanup). See the parent's
[pilot validation record](REGION_FUSION_PILOT_VALIDATION.md) for reproducible
commands and the complete runtime/toolchain scope.

The Rust suite checks an already-set cancellation flag and budget exhaustion
after an earlier proposal. It does not claim to test an asynchronous token flip
inside Rust matching; asynchronous process cancellation is covered by runner
tests. There is no broad native fuzzing, sanitizer/security scan, peak-memory
benchmark, multilingual/rotated/cropped corpus qualification, independent
held-out accuracy benchmark, or production integration in this review. Those
remain outside this experiment's approval scope.

No unresolved selection/ownership/publication blocker was identified at these
source fingerprints for the isolated Linux pilot with the stated caller-trust
and completion-consumption contract. Production use still requires the separate
qualification gates recorded above.
