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
- Default Rust: second Clippy pass succeeded; the second test run passed 838,
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
are preserved; diagnosis of the native observation and exact-source verification
remain pending.

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
