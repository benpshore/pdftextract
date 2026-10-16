# Indirect widget-state appearance correction — 2026-10-16

The source-derived mismatch is reproduced. An indirect `/AP /N` state dictionary
can contain a valid selected `/AS` stream, yet pinned PDF Oxide 0.3.78 treats the
resolved dictionary as a final appearance object. With no `/BBox` on that state
dictionary, it silently regenerates generic paint. The adapter previously accepted
the input and reported `existing_widget_appearances_required: true`.

Four synthetic cases cover checkbox/radio flags and selected on/off states.
All four lost their custom selected paint without a flatten warning. Four direct
state-dictionary controls preserved the exact selected appearance stream. The
baseline test run passed the direct control and failed the indirect regression;
the original log, test patch and harness bytes, source PDFs, upstream/adapter
derivatives and decoded appearance observations are indexed in the
[evidence manifest](evidence/widget-state-manifest.json).

Eight independent, bounded Poppler renders completed with unchanged sources.
The viewed selected checkbox changes from a magenta custom appearance with a cyan
diagonal to a generic black checkmark. Its direct-dictionary control retains the
custom paint. This is concrete defect evidence, not general rendering qualification.

The production correction is a narrow admission guard: `forms.flatten` returns
Unsupported when `/N` is an indirect reference resolving to a state dictionary.
Direct state dictionaries and references to ordinary normal appearance streams
retain their existing behavior and guards. No fallback, dependency change or new
feature is introduced.

The prior tested checkpoint `fa9beab88b24c76344039b2f524eb2236c7f67ef` is preserved
in branch history and local ref
`archive/20261016-native-fa9beab-before-state-appearance`. Its exact a39af2b source
receipts, binary and earlier defect evidence remain unchanged.

The complete clean source is `e2991be508ff6e68ee175fceaea6d493da36ed89` (tree
`b3c7da974a3734e0c456c4faee327f1fdcc7ba6a`), on the existing recovery branch. The preserved
binary is `tpe git-e2991be`, SHA-256
`eb66c5ca65fa94a59249d0e4526c85b897b940db2cb37d550e97321f47db917d`.

The complete workspace passes with rendering, barcodes, signatures and offline
OCR enabled together: 1,295 top-level tests plus three nested child probes, zero
failures and ten ignored. Five additional bounded containment scenarios ran;
Landlock remains explicitly unavailable. Formatting, strict workspace Clippy,
Ruff and all 210 Python 3.14.7 tooling tests pass. These include all 41 editing/
form tests. OSV remains unreachable; Swift/CMake/CTest are unavailable.

The same clean binary passed eight supervised CLI cases. All four indirect-state
inputs returned the expected Unsupported reason, preserved source bytes, published
no output and removed private captures. All four direct-state controls published
bytes matching their artifact receipts, preserved input identities, reported
actual sealed operation workers and removed captures. Independent 96-dpi Poppler
first-page PNGs match each positive source/derivative pair exactly. This validates
these four synthetic appearances, not general form fidelity.

[Workspace receipts](evidence/widget-state-exact-receipt.json),
[CLI controls](evidence/widget-state-exact-cli-receipt.json) and the
[exact manifest](evidence/widget-state-exact-manifest.json) preserve hashes and
commands. Each command ran in a disposable bounded process. The CLI cases used
an outer 2 GiB / 15-second limit and 5-second / 256 MiB growth worker limit;
Poppler used 1 GiB / 15 seconds. Earlier shared-fixture and standalone OCR receipts
remain bound to a39af2b. The following documentation-only checkpoint archives
original receipt bytes without changing their tested source identity.

PR273 reconciliation remains Ada-owned. No PR is opened, closed, merged or
advanced by this follow-up. No capability is promoted to qualified; the existing
Python binding, independent fidelity, platform and ARM64 Native evidence gaps
remain. The full earlier [native handoff](FINAL_HANDOFF.md) retains those limits.

The first CLI verifier incorrectly expected published PDF bytes to remain in the
inline artifact payload. In --out mode the publisher deliberately consumes that
payload. Published file hashes/sizes already matched their receipts. The original
failed verifier and receipt are preserved; the corrected run checks published
digest, size, name and MIME plus the expected empty inline payload. No runtime
change or validator weakening was needed for that harness correction.

Live shell, Python, filesystem and Git commands succeeded at
2026-10-16 01:15:14 UTC after the reported 01:14 disconnect callback.
[The availability receipt](evidence/widget-state-environment-availability-01.json)
records that no command-access blocker was observed.
