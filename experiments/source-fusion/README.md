# Source mapping region selection experiment

This separate, opt-in experiment derives verification evidence from immutable
PDF bytes. The selector receives the source PDF and exact baseline/candidate
extraction artifacts. It accepts no review, expected transcript, truth file or
rendered answer. It preserves the complete baseline and alternatives and emits
a separate selected-span projection.

The supported profile is deliberately restricted. Raw character codes are tied
to the executed content stream and operator, resolved font and mapping objects,
embedded font identity and source-declared geometry. Verification computes the
Unicode mapping and checks its consistency with the embedded font. Unique
geometric correspondence and exact candidate agreement are prerequisites for
selection. Unknown, inconsistent or unsupported evidence must abstain.

This policy establishes agreement among source mappings within its qualified
profile. It does not prove that font outlines visually express a Unicode value,
that every PDF has honest mappings, or that arbitrary rotated, hidden, scanned,
marked-content or complex-layout documents are correctly transcribed. Rendered
truth is an evaluation input only, kept outside the selector's API and process
arguments. No manual reviewer choice authorizes selection.

## Isolation and execution

The crate has its own Cargo workspace and lockfile. It depends on the frozen
`region-evidence` artifact contract. The Linux runner reuses the frozen
`region-fusion` runner's lifecycle helpers but replaces its orchestration;
it does not call the reviewed-selection path or accept its review inputs.
PR202 and PR208 remain unchanged, as do production parsers, schema, manifests,
native runtime pins and other owners' branches.

```sh
cargo build --locked --manifest-path experiments/source-fusion/Cargo.toml --bin fuse_source

# Offline selection from existing exact artifacts:
experiments/source-fusion/target/debug/fuse_source \
  --source /absolute/path/to/source.pdf \
  --baseline /absolute/path/to/lopdf.json \
  --candidate /absolute/path/to/pdfium.json \
  --output /tmp/new-source-projection.json

# Fresh extraction through the existing native adapters:
PDFIUM_DYNAMIC_LIB_PATH=/absolute/path/to/libpdfium.so \
uv run --locked python experiments/source-fusion/run_extraction.py \
  --source /absolute/path/to/source.pdf \
  --out /tmp/new-source-job \
  --tpe /absolute/path/to/tpe \
  --fusion /absolute/path/to/fuse_source \
  --pdfium
```

Use new output paths. The runner first snapshots the source and durably saves
lopdf output, then optionally runs PDFium. Partial extraction remains Partial
and keeps exit code 1, even after a region is selected. Await termination before
accepting the final completed journal and matching projection hash. Unsupported
or stopped source verification retains the baseline and cannot publish a final
fused artifact. Diagnostic projection bytes remain explicitly uncommitted.

## Qualification

```sh
cargo fmt --manifest-path experiments/source-fusion/Cargo.toml --check
cargo clippy --locked --manifest-path experiments/source-fusion/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path experiments/source-fusion/Cargo.toml
uv run --locked pytest experiments/source-fusion/test_runner.py
uv run --locked pytest experiments/source-fusion/test_evaluate.py
uv run --locked pytest experiments/region-fusion/test_runner.py

SOURCE_TPE_BIN=/absolute/path/to/tpe \
SOURCE_FUSION_BIN=/absolute/path/to/fuse_source \
PDFIUM_DYNAMIC_LIB_PATH=/absolute/path/to/libpdfium.so \
uv run --locked pytest experiments/source-fusion/test_real_extraction.py
```

Read the [independent review](../../docs/SOURCE_REGION_SELECTION_REVIEW.md) and
the frozen fixture/evaluation records for the final qualified profile, observed
outcomes and outstanding gates. The standard limits are accepted-input and
cooperative work limits, not proof of bounded peak memory or a hard wall-time
guarantee for parsing or blocking kernel I/O. Existing native subprocess
containment remains necessary. This experiment is neither a release nor a
production integration.

The frozen six-case run is **not qualified**: zero regions selected, six neighbors
preserved, and visible target correctness 1/6, unchanged from baseline. Supported
cases abstain with `crossing_span`; three other sources are unsupported. The
fresh-engine qualification retains its positive-selection requirements and
reports one pass/four failures. No geometry threshold, source policy or held-out
label was changed after unsealing. See
[qualification evidence](../../docs/SOURCE_REGION_SELECTION_VALIDATION.md).
