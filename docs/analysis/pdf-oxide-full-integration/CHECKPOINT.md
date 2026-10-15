# Work-in-progress checkpoint — 2026-10-15

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

Evidence before this snapshot (logs under
`/workspace/tpe-pdf-oxide-evidence-20261015`):

- Python 3.14.7: 210 tests passed; Ruff format/check passed after the inventory
  formatting fixes. This is repository Python testing, not binding parity.
- Default Rust: Clippy passed; 838 tests passed, 8 ignored. Subsequent worker
  integration changes still require exact-source reruns.
- First broad native operation run: 82 passed, 8 failed. Subsequent runs repaired
  several cases and exposed additional preservation failures; all original logs
  are retained. The latest `full-tests-05.log` is **not green**.
- Five bounded containment subprocess scenarios ran. Actual `seccomp_sealed`
  path/socket/process restrictions and inherited-thread synchronization passed.
  Landlock returned ENOSYS; its positive allowlist test did not run. Threads remain
  allowed; no separate thread-count/cgroup bound is claimed. Tests use finite
  denial probes, never a live fork bomb.
- Python dependency audit could not reach OSV. Swift, CMake and CTest are absent.
  Optional rendering/barcode/signature/OCR feature builds remain pending here.

Known failing/unqualified behavior at this checkpoint includes native table
creation, terminal form removal, encryption serialization preserving pages, and
one stdout-backpressure assertion. The pinned native character API corrupts
non-ASCII ActualText; its original observation is retained with an explicit
diagnostic and a separate span-derived representation. Image extraction rejects
mask-bearing resources instead of returning silently opaque PNGs. Real OCR
recognition is unverified without approved model fixtures.

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
