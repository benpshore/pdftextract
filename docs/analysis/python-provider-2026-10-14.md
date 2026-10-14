# Python launcher and portable provider strings — 2026-10-14

Base: `8f7272a94833d9c06eb55b93900a44791a2e6665` (remote main verified).
Checkpoint branch: `codex/python-cli-20261014`.
No existing working-tree changes or populated workspace `.agents/skills` were found.
Repository AGENTS.md governs validation. No releases, merges or deployments.

## Selected changes

The Python entry point previously returned success after printing a greeting.
It now resolves the separately installed `tpe` from PATH and replaces the
process with the native command. Arguments are passed as an argv array, not a
shell string; output is not buffered in Python. Both the installed entry point
and `python -m text_processing_engine` work. Missing/unexecutable engines fail
with statuses 127/126 and actionable diagnostics. This does not add extraction
containment to commands which lack it in Rust, or make PATH an authenticity check.

The provider FFI used `c_char.cast_unsigned()`. On Linux ARM64, `c_char` is
already unsigned and that method does not exist. `to_ne_bytes()[0]` preserves
the representation for either signedness; tests cover multibyte UTF-8, invalid
UTF-8, NUL termination and the identity read limit. Existing tests' byte creation
uses the corresponding `from_ne_bytes` operation. No ABI or allocation-bound
change is intended. The existing native Linux CI matrix now checks/tests this
optional provider feature without requiring a PDF provider library.

## Prior work checked, not assumed open

PR #273 already implements extraction ordering/width/status fixes. At its
recorded head `7cabac188b5d89c5f0a9127392f5213df7d349a1`, the connected GitHub
reader reported CI, PMC bibliography, Native repair, Native bibliography and
Build binaries successful; Native and PDFium Unicode failed. These are historical
head checks, not checks of this branch. The `gh` API returned Forbidden; git
transport and the connected reader worked.

- [Native run](https://github.com/benpshore/pdftextract/actions/runs/38169034277),
  job 114557539328: strict model provisioning, build, Clippy, tests and all four
  backend evaluations succeeded. Validation then rejected every report with
  `reviewed-partial provenance mismatch: native_manifest_sha256`.
- [PDFium Unicode run](https://github.com/benpshore/pdftextract/actions/runs/38169034292),
  ARM job 114557539330: provisioning passed; compilation failed at provider.rs
  lines 302/317 with E0599 for `cast_unsigned` on `u8` / `&u8`. The x86-64 job
  114557539592 succeeded.

The model integrity gate remains unchanged. Updating the reviewed-partial hash
without examining the new dumps would falsely renew prior evidence. Reviewing
those artifacts against the changed native manifest remains separate work.

## Local validation

Pinned Rust 1.98.1 and Python 3.14.7 were provisioned under /tmp; no system
credentials or default toolchain settings were altered. Rust builds use two jobs.

- Baseline Python: 210 passed.
- Changed Python: Ruff format/check and 214 tests passed; wheel and sdist built.
- Real-process launcher tests check PID continuity, arguments containing shell
  metacharacters, inherited stdin/stdout/stderr and nonzero exit status for both
  entry points. They use an executable test double, not real PDF extraction.
- Provider Rust: seven tests passed on x86-64; one existing doctest ignored.
  Clippy passed for all provider targets on x86-64 and ARM64 cross compilation.
  Cross compilation does not establish ARM64 runtime behavior.
- An initial focused Clippy run found an existing test's wrapping `as c_char`
  conversion; the representation-preserving conversion fixed it and rerun passed.
- `uv audit --preview-features audit-command` could not reach api.osv.dev after
  retries. No clean dependency audit is claimed.
- Swift and CMake commands could not start (tools absent). The CMake app targets
  additionally require macOS. No app runtime/macOS verification is claimed.

Full root Rust checks and final exact-head checks are recorded in the PR/handoff.
No benchmark or fidelity improvement is inferred from these tests.
