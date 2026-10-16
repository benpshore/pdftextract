# Work-in-progress recovery checkpoint — 2026-10-16

Integration owner: parent Ada, existing tracked task PR273. This separate cloud
checkpoint starts at `b3730aa958d947fcbf6ea45b0efef07fcfa20f29`. It is **not an
update proposed for PR273, a release, a parity claim, or a qualified build**.
Existing PR heads, native validators, exception hashes, visibility and project
license are unchanged. Repository LICENSE SHA-256 remains
`57411afebd084482658b294bc643db7f93b32a01d5215492f96a149fa5f92e78`.

The snapshot contains the pinned source/capability inventory, byte-based native
operation adapters, bounded worker dispatch/publication, extraction supplements,
schema migration, and synthetic success/failure tests. Original source bytes and
raw observations remain separate from derived projections. Source-index counts
and dispatched operation names are not implementation or accuracy evidence.

The first deliberately failing snapshot is preserved at
`3add612ed47c913d2540cc902d1d757d8e4a632c`. Subsequent repairs retain those tests
and failure observations. Current evidence (full logs under
`/workspace/tpe-pdf-oxide-evidence-20261015`):

- Python 3.14.7: 210 tests passed; Ruff format/check passed after the inventory
  formatting fixes. This is repository Python testing, not binding parity.
- Default Rust: second Clippy pass succeeded; the second test run's summaries
  total 838 passes, including three nested child-probe invocations (835 top-level),
  with 8 ignored. This does not enable the optional operation families.
- Native history: the initial focused run passed 82 and failed 8. Run06's full
  suite passed 953, failed 4, and ignored 8. Its remaining defects were Unicode
  annotation reading, form geometry, independent encryption-password handling,
  and a test pipe inherited without CLOEXEC. The original logs and assertions
  are retained; neither run was a qualification pass.
- A combined rendering/barcode/signature/OCR run then passed 83 focused tests:
  creation/Office 15, editing/forms 23, rendering/OCR 15, security 20, workers 10.
  This includes deterministic synthetic ONNX inference, not trained-model OCR
  accuracy. A later targeted run passed all 27 OCR/rendering and worker tests,
  including the thin-image budget, receipt rollback and routed-rejection checks.
- Combined-feature strict Clippy now passes across all workspace targets
  (`full-clippy-06.log`). The final exact-source tests remain pending.
- Five bounded containment subprocess scenarios ran. Actual `seccomp_sealed`
  path/socket/process restrictions and inherited-thread synchronization passed.
  Landlock returned ENOSYS; its positive allowlist test did not run. Threads remain
  allowed; no separate thread-count/cgroup bound is claimed. Tests use finite
  denial probes, never a live fork bomb.
- The private FFI test executed finite actual fork/vfork/process-clone denial
  probes. clone3 returns ENOSYS for bounded thread fallback. These are process
  creation-denial tests, not a destructive fork bomb or a cgroup/PID guarantee.
- Python dependency audit could not reach OSV on either attempt. Swift, CMake
  and CTest are absent. Cross-platform builds and cross-binding parity remain
  unverified.

Known defective table creation, HTML/CSS page sizing and text replacement now
return explicit Unsupported errors. Form removal and serialized properties use
bounded identity-preserving repairs with named lopdf provenance; AES-256 repair
appends the required encrypted /Perms without discarding original page objects.
Supervised routed extraction rejects until lazy PDFium loading has a qualified
filesystem policy; its library source is preserved. Publication response and
receipt writes now remain inside the artifact rollback boundary.

The pinned native character API corrupts
non-ASCII ActualText; its original observation is retained with an explicit
diagnostic and a separate span-derived representation. Image extraction rejects
mask-bearing resources instead of returning silently opaque PNGs. Detected tables
remain raw candidates with validation errors. Form appearance fidelity, trained
OCR recognition and broad extraction/rendering accuracy remain unverified.

A final source review found that ordinary value/style updates can retain stale
/AP streams, and adding fields can rebuild an existing AcroForm catalog without
preserving its fields/resources. The adapter now rejects affected value/style
changes, existing-AcroForm additions, and flattening of pending appearances.
Nonvisual property updates and unchanged-value fills remain available. Six new
tests preserve direct upstream defects and controls. The form target passed
28/29: the same-session upstream control failed native text extraction even
though bounded independent Poppler rendering/text extraction displays `After`.
Its generated appearance also loses the original border/background, so it is
not enabled as a fallback. The failed assertion and all five synthetic outputs
are preserved. Diagnosis then proved that the native shallow subtype scan mistakes
an inline font for the enclosing Form and omits the whole Form. The next test
revision preserves that cold-reader omission and a parsed-object diagnostic
control; all 31 editing/form tests passed. Text-consuming adapters reject known
misclassified/inline-subtype Forms rather than relying on cache preloading.

The next focused admission run passed 114 and failed one. Its sole failure was
the cyclic-resource test's old image-stage label: the new Form guard rejects the
same source earlier, before publication. All 27 security, 13 worker, 31 editing/
form, 11 backend, 10 query and 10 conversion tests passed. The supplement target
passed 12/13. The query Clippy findings were repaired; subsequent findings in expanded test
helpers and worker functions were also corrected without lint suppression. This
is still intermediate evidence, not final-source qualification.

Independent Poppler observations preserve 20 synthetic redaction PDFs, all source
hashes unchanged. A black overlay hides the rise-case target visually while text
extraction recovers it. ActualText retention is separately established in decoded
objects; it is not inferred from Poppler extraction. The adapter now rejects
q/Q saved text state and requires explicit bounded targets with independent
pre/post decoded-content and extraction verification. Request receipts/errors
omit target values. All-page carrier/page-tree verification passed all 29 security
tests in the next run. A distinct widget normal-appearance gate rejects
unqualified regeneration and omission warnings. Its regression preserves native
widget-value extraction separately from static page painting; no-AP alone is not
a claim of universal data loss.

Operator-limit baseline evidence now includes fallback, silent prescan and
saved-state-restore paths. The latter returned success, retained the prefix,
dropped the tail and emitted no warning. The conservative adapter preflight
rejects these text-work ceilings without changing native caps or manufacturing
native warnings. Query/converter completeness remains unverified or explicitly
truncated; missing native page values remain missing with separate operation-page
context.

Parent visually confirmed additional upstream/shared-library cases: text crossing
table borders, overlapping HTML/CSS cells, black masks during text replacement,
wrapped descriptions becoming extra rows, lost table headers/blank rows, short
prose misclassified as a table, and meaningful headings/citations removed by
cleanup. These are acceptance regressions, not reasons to lower assertions.
Defective mutation/layout paths require explicit errors or an independently
verified fallback with provenance. Same-library reopening/rendering and green
smoke tests do not establish fidelity. Exact parent fixture identities are being
requested; equivalent approved synthetic cases are retained while waiting.

The original backlog source index is preserved on PR273 checkpoint
`88822fddfca1c84f81bb3e9c2cd0bdd3ce8fbb35`; this work does not reconcile or close
other proposals. PR275/280/281 and all parent holds remain unchanged.

The acquisition/publication boundary now binds ordered immutable staged inputs,
operation/public request options, an opaque invocation ID, canonical build identity
and the exact captured response before any publication. Original file reads stay
in a bounded acquisition worker; private request/password digests stay private.
Seven transport regressions and all 13 worker regressions passed, including
valid-looking digest swaps, operation/options swaps, replay, artifact capture
tampering, producer/publication-state checks and a positive exact-byte control.
This proves transport binding for these cases, not semantic artifact correctness.

The new acquisition phase exposed an old cancellation-test race: the first child
could already be a zombie. The probe now identifies the operation phase and proves
it entered stopped state before testing timeout/INT/TERM kill-and-reap. The
original failed logs remain preserved. Source checks for the final complete
workspace run are next; none of these focused passes establishes toolkit parity.

The corrected missing-normal-appearance proof subsequently passed: all 32 form/
editing tests in `forms-guards-03.log`. It distinguishes recoverable retained
widget values from absent static page painting, and preserves a separate native
regeneration positive control. The only following Clippy finding was a 101-line
test; helper extraction retains its assertions. Production guards are unchanged.

The final clean native checkpoint is now `325166027331ee4ca6a053bd81f8ca43b7527de1`
(tree `efb93bb23c13964bd28983644caffec9faa97c10`). All four optional features
passed workspace formatting, strict Clippy and tests together: 1,286 top-level
passes plus three nested child-probe passes, zero failures, ten ignored; five
additional bounded containment harness scenarios ran with Landlock explicitly
unavailable. Python 3.14.7 passed 210 tooling tests and both Ruff checks.

The preserved clean binary also processed all eight shared fixtures / 29 expected
pages as explicit Partial outcomes (exit 1). Source identities and bytes, page
coverage, sealed-parser reporting and capture cleanup matched. All three table
candidates remained unverified. The deterministic offline OCR control passed,
including source digests, separate image placements, bounded pixels and actual
sealed operation-worker state; its graphs ignore image content. Neither exercise
establishes extraction or OCR accuracy.

[FINAL_HANDOFF.md](FINAL_HANDOFF.md) records the exact source, binary and receipts.
This native branch is based on main b3730aa and does not include PR273's backlog
checkpoint. Ada still owns their reconciliation and final exact combined-source
tests. No capability was promoted to qualified, and no PR action was taken. The
following documentation-only checkpoint archives original receipt/log bytes; it
does not claim the compiled binary was built from that later documentation commit.

A subsequent parent source review identified two additional carrier risks in both
annotation and form flattening. Four concrete tests failed: no-Contents pages lost
valid appearances after annotation/field removal, and inherited-resource pages
lost original font/image bindings. Raw upstream and admitted adapter outputs, the
failed harness binary, original log and 12 independent Poppler observations are
preserved. Poppler renders confirm the no-Contents form becomes blank and the
inherited-resource form loses body text/image while retaining its appearance.

A bounded raw page-tree admission guard now rejects those shapes before mutation;
the inherited projection returned by get_page cannot serve this check. Seven new
regressions include direct/indirect local Resources, explicit empty Contents,
original page-operation/resource/appearance survival and malformed Contents. All
39 form/editing tests and strict workspace Clippy pass. The first follow-up run
had a test compilation error (Operation lacks PartialEq); its original log is
retained. Final exact complete-source verification of these guards is next.

The exact clean follow-up source is now `a39af2b312a39f36f384031d7f81bb22d22daf59`
(tree `ed4aa34eca5fc11e0a1785e759548b95328eb465`). All four features passed formatting,
strict workspace Clippy and the complete workspace tests: 1,293 top-level passes
plus three nested child probes, zero failures and ten ignored; five additional
bounded containment scenarios ran with Landlock explicitly unavailable. Python
3.14.7 again passed 210 tooling tests and both Ruff checks.

The same preserved clean binary repeated the eight shared fixtures / 29 pages as
Partial outcomes, the deterministic offline OCR control, and four new supervised
flatten-carrier rejections. All four failed for their expected explicit carrier
reason, kept inputs unchanged, published no output and removed private captures.
Original 3251660 receipts and the four failed carrier baselines remain unchanged.
See [FINAL_HANDOFF.md](FINAL_HANDOFF.md) and
[evidence/exact-a39af2b-manifest.json](evidence/exact-a39af2b-manifest.json).
This does not qualify any capability or the still-unreconciled PR273 combination.

A later exact source review identified another state-appearance mismatch. The
new baseline passed its direct-state control and failed its indirect-state
regression: all four checkbox/radio flag and on/off cases silently replaced
custom paint while reporting existing_widget_appearances_required=true. The
production checkpoint fa9beab, exact baseline test patch/harness, original failed
log and synthetic outputs remain preserved. Eight independent Poppler renders
completed with unchanged inputs; the selected checkbox changes from a magenta
custom appearance to a generic black check, while the direct control retains it.

The narrow guard rejects an indirect /AP /N resolving to a state dictionary,
without changing direct-state or ordinary normal-stream handling. All 41 editing/
form tests, strict workspace Clippy, Ruff and 210 Python 3.14.7 tooling tests pass.
OSV remains unreachable and Swift/CMake/CTest unavailable. Complete exact-source
verification follows the new source checkpoint. See STATE_APPEARANCE_HANDOFF.md.

The exact clean indirect-state correction `e2991be508ff6e68ee175fceaea6d493da36ed89`
passed the complete four-feature workspace: 1,295 top-level tests plus three
nested child probes, zero failures, ten ignored; five additional bounded
containment scenarios ran with Landlock unavailable. Strict Clippy/formatting,
Ruff and 210 Python 3.14.7 tooling tests pass. Eight supervised CLI cases passed:
four explicit indirect-state rejections with no publication, plus four direct
controls whose published first-page Poppler renders match their sources exactly.
All source bytes and private capture cleanup checks passed. No capability was
qualified; prior checkpoints and their source-scoped receipts remain preserved.
See [STATE_APPEARANCE_HANDOFF.md](STATE_APPEARANCE_HANDOFF.md).
