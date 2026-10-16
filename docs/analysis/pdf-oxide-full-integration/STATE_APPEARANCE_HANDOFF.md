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

The focused correction passes all 41 editing/form tests and strict workspace
Clippy with rendering, barcodes, signatures and offline OCR enabled together.
Ruff and all 210 Python 3.14.7 tooling tests pass. OSV remains unreachable;
Swift/CMake/CTest are unavailable. The complete clean-source run and supervised
CLI negative/positive controls are pending the new source checkpoint.

PR273 reconciliation remains Ada-owned. No PR is opened, closed, merged or
advanced by this follow-up. No capability is promoted to qualified; the existing
Python binding, independent fidelity, platform and ARM64 Native evidence gaps
remain. The full earlier [native handoff](FINAL_HANDOFF.md) retains those limits.
