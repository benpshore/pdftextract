# Source concordance qualification — 2026-10-04

The automatic source-mapping experiment remains **unqualified**. Fresh lopdf and
PDFium extraction of all six frozen cases yielded zero selected regions. All six
neighboring text regions were preserved; visible target correctness is 1/6,
identical to baseline. This bounded synthetic cohort is not human, blind or
population accuracy evidence. Matching ToUnicode and embedded font cmap remain
within the PDF author's trust domain and do not prove visible glyph meaning.

Tracking: [#196](https://github.com/benpshore/pdftextract/issues/196), under
[#181](https://github.com/benpshore/pdftextract/issues/181) and
[#182](https://github.com/benpshore/pdftextract/issues/182). This standalone draft
stacks on reviewed-region PR208 at `cd95a8b7243c690b24f59b1994ecb7670c95c1c0`.
Production source, schemas, native pins, dependency manifests and PR202/208 are
unchanged. This is not integration into the frozen eb5f711 corpus candidate.

## Preserved identities and separation

- Transferred archive: SHA-256
  `71e9b3168f19e67ed7f4917c14b5c7aa4562771ac89e06da83378756d9b0ce5f`.
  Safe archive paths/types and all 91 manifest file hashes/sizes pass.
- Six-case input freeze: SHA-256
  `169bf0c9dc1bdabe36d779e87004afc39ac3d63e434e1a6e5bbfe88ae1d411b4`,
  originally recorded before native capture. Sources, captures, font notices,
  truth commitments and prior render review are retained without regeneration.
- Source/selector/CLI/runner/evaluator policy freeze: SHA-256
  `c895a4657dfa416f5c99985ce39e48f6ccda1934989bda55e98d70cee1ff055f`,
  recorded at **03:27:12.284164 UTC before truth unsealing**.
  [Frozen implementation hashes](../experiments/source-fusion/fixtures/policy-freeze.json)
  and [unseal receipt](../experiments/source-fusion/fixtures/evaluation/unseal-receipt.json).
- The new evaluator executed every native/selector attempt before its first
  truth file read: execution started 03:27:40.791375, all attempts finished
  03:27:48.810658, scoring started 03:27:48.811958 UTC. Truth, rendered images and
  review never enter selector or native-runner argv.
- TPE was built from the unchanged production source inherited by PR208, with
  `--locked --features pdfium`, in the unoptimized dev profile. Rust/Cargo 1.98.1;
  wrapper 0.8.37 and pinned PDFium8066. Runtime archive SHA-256
  `0b43f405477cf2cfc4dbff06905093c3309756c6bca1fb9da99234a2ca97fed2`;
  library SHA-256 `7670b3c597b02dfa3f98b23b49c3bb52536312f1ea686b739321731b6011f5a9`.
  Executable hashes and original journal tool identities are retained. No local
  optimized production release-mode test is claimed.

## Actual outcomes

[Report and original receipts](../experiments/source-fusion/fixtures/evaluation/results/report.json)
retain all six cases and separate source consistency, visible truth and
availability. An evaluated projection can contain only baseline spans; the
status does not imply any recovery. Five native extraction pairs contain Partial
outcomes, and their runners return exit1 even where projection publication
completes. Case03 is the unchanged complete control and returns exit0.

| Case | Selector outcome | Selected | Neighbor preserved | Visible target correct |
|---|---|---:|---|---|
| 01 | evaluated; crossing-span abstentions | 0 | yes | no |
| 02 | evaluated; crossing-span abstentions | 0 | yes | no |
| 03 | evaluated; crossing-span abstentions | 0 | yes | yes |
| 04 | unsupported_source; missing map | 0 | yes | no |
| 05 | unsupported_source; conflicting map | 0 | yes | no |
| 06 | unsupported_source; semantic/visible disagreement | 0 | yes | no |

The geometry blocker is directly visible in case01. The source-derived target
envelope has x0 `70.15625`, y0 `609.7167959976197`, y1 `624.843759765625`.
The actual lopdf target span has x0 `70`, y0 `606`, y1 `626`; PDFium has x0 `70`,
y0 `609.74`, y1 `625.06`. Both cross the qualified containment envelope, so the
selector abstains. The same mechanism rejects both positive cases. This is an
unqualified relationship between source-derived metric envelopes and backend span-box
conventions, not permission to loosen geometric thresholds using held-out truth.

There are zero execution/scoring harness errors. Zero selected cases also means
zero source-inconsistent or visibly wrong **selected** cases, but that empty
denominator cannot establish precision or successful recovery. The separate
adversarial mutation of both ToUnicode and font cmap with unchanged outlines
remains pending; case06 only covers a conflicting ToUnicode declaration.

## Validation and retained failures

Truth-free checks pass: 7 CLI, 16 selector/parser, 15 evaluator, 8 source-runner
and 22 existing lifecycle tests. Formatting, Ruff and strict experiment Clippy
pass. These tests validate evidence boundaries, malformed inputs, budgets,
rollback, failure retention and committed receipts.

The unchanged production baseline's default Rust tests and strict Clippy also
pass locally, along with its 92 ordinary Python tests and Ruff. They are distinct
from the frozen eb5f711 corpus candidate and from release-mode tests. Local OSV
audit is network-tunnel blocked; Swift/CMake/CTest are absent. The draft's hosted
checks remain separate from these local observations.

Fresh-engine `test_real_extraction.py` reports **1 passed / 4 failed**. Its
unchanged requirements demand at least two real selections and a selected case
for the candidate-mutation checks. The failures remain visible; no test is
skipped, weakened or relabeled. The successful test verifies all native/selector
argv and exact artifact lineage exclude truth/review/render inputs.
The new hosted pilot preserves failure outputs as an artifact and is expected
to remain red until a separately qualified policy meets those requirements.

Reproduction uses the committed source/profile fingerprint and a new output
directory:

```sh
cargo build --locked --features pdfium --bin tpe
cargo build --locked --manifest-path experiments/source-fusion/Cargo.toml --bin fuse_source
PDFIUM_DYNAMIC_LIB_PATH=/absolute/path/to/pinned/libpdfium.so \
uv run --locked python experiments/source-fusion/evaluate.py \
  --truth-dir experiments/source-fusion/fixtures/evaluation/truth \
  --out /tmp/new-source-evaluation \
  --tpe target/debug/tpe \
  --fusion experiments/source-fusion/target/debug/fuse_source
```

Ledgers and runtime binaries remain outside git. Published results contain only
owned synthetic source/derived text, evidence JSON, execution logs and reviewed
render assets. The original complete runtime directory remains preserved
locally, including ledgers; the committed subset has a separate file-hash index.
The frozen source-policy files were not edited after unsealing. Any next geometry
policy must use development inputs and a separately frozen qualification cohort;
these six cases are now observed regression evidence.

The four-backend production corpus gate still rejects 209 Partials; PR210 provides
its complete classification. Security scanning is explicitly deferred. No
security clearance, merge, release or deployment occurred.
