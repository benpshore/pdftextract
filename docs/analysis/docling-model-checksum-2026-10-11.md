# Docling layout model checksum investigation — 2026-10-11

Investigated PR #273 at `f3fbe828f1531f1d62c346586ea8a24f0406fc37`.
The original pin was valid; a mutable upstream release was republished.
The smallest supported provisioning correction is the one layout-model
SHA-256 replacement in `native/manifest.json`, with verification unchanged.
This adopts a newly published model artifact; it does not establish numerical
or extraction equivalence to the original. PR #273 remains draft and unmerged
pending independent review and downstream native qualification.

## Evidence and chronology

| Observation | SHA-256 of `layout_heron_int8.onnx` |
| --- | --- |
| Original pin, verified by a fresh September 28 download | `d0eb9da0515a2121a20c3262f49ec4761109da45423b37c14b8b98b9759499b5` |
| Both failed PR-head jobs, October 10 at approximately 01:28 UTC | `387be1f67bd58ec7fc575c53966492890d91af0810866bb383af45a18cc66161` |
| Current October 10 publication, GitHub asset metadata and two independent local downloads | `87ac925d4183b4f4745fb48f6ff216fe88ce244d9ab635725e6d514fe490fc2a` |

All three files have the recorded size 68,695,321 bytes. Equal size is not
evidence of equal contents. The October 11 status inspection concerns runs
whose actual failed downloads occurred on October 10, before that day's
10:07 UTC republication.

1. [Pin commit d108fcd](https://github.com/benpshore/pdftextract/commit/d108fcd55a0f44be490e392274f160bd50a68c6a)
   introduced the digest on September 28; the squash merge was
   [a3efd659](https://github.com/benpshore/pdftextract/commit/a3efd659d5f439976f715875a06b7fdbdb1fb62d).
   Manifest history through the investigated head shows no later change.
   [Native run 36437023879](https://github.com/benpshore/pdftextract/actions/runs/36437023879),
   job `108977471494`, records `downloaded,verified` and the original digest.
   Provisioning and compilation succeeded in that historical run; later
   tests failed, so the entire historical run is not cited as passing.
2. The [models-v1 release](https://github.com/docling-project/docling.rs/releases/tag/models-v1)
   is release ID `351549900`, initially published July 9. Its REST record says
   `immutable: false`. The [publishing workflow at the current source head](https://github.com/docling-project/docling.rs/blob/78c21e004662bb13b91b4c5a1ed0ac4d08c6bc2a/.github/workflows/publish-models.yml)
   re-exports models and uses `gh release upload ... --clobber`. The release
   tag therefore does not identify immutable model bytes. Its tag commit
   likewise does not identify the source of a later uploaded asset.
3. Upstream [run 35014913317](https://github.com/docling-project/docling.rs/actions/runs/35014913317)
   (September 15, source `de4a51e17cc5b08991b7b9043bc5567a224e7b05`)
   used torch 2.14.0 / transformers 5.17.0 and 52 calibration samples.
   [Run 37448237701](https://github.com/docling-project/docling.rs/actions/runs/37448237701)
   (October 6, source `27a3c9512dfa2f6229cb59fe5c3ae6e80dcaf42e`)
   used torch 2.14.1 / transformers 5.18.0 and 53 samples, then overwrote
   the same release assets. The latter publication is the temporally
   consistent origin of `387be1f6…`; its logs do not record a SHA-256,
   so that precise digest-to-run association is an inference.
4. [Current publishing run 38043332827](https://github.com/docling-project/docling.rs/actions/runs/38043332827)
   (October 10, source `78c21e004662bb13b91b4c5a1ed0ac4d08c6bc2a`)
   used torch 2.14.1 / transformers 5.19.0 and 53 samples. Job
   `114187798685` passed layout quantization and its agreement gate:
   539 confident fp32 detections, zero lost detections, zero lost tables.
   It staged the 68,695,321-byte file and published with `--clobber`.
   The resulting asset ID `627624021` was created at `2026-10-10T10:07:06Z`,
   updated at `2026-10-10T10:07:08Z`, and records the new `87ac925d…`
   digest in [GitHub's release API](https://api.github.com/repos/docling-project/docling.rs/releases/tags/models-v1).
   Two separate local downloads of the manifest URL independently match
   that digest and size. Other three model asset digests match their
   existing manifest pins, which remain unchanged.

Selected verbatim log excerpts and current asset metadata are preserved in
[the evidence JSON](docling-model-checksum-2026-10-11-evidence.json).

## Origin and the boundary of the explanation

The [export script](https://github.com/docling-project/docling.rs/blob/78c21e004662bb13b91b4c5a1ed0ac4d08c6bc2a/scripts/install/export_layout.py)
loads `docling-project/docling-layout-heron` using `from_pretrained` without a
pinned Hugging Face revision, exports static and dynamic ONNX graphs, and
splices the folded position embedding. The
[quantization script](https://github.com/docling-project/docling.rs/blob/78c21e004662bb13b91b4c5a1ed0ac4d08c6bc2a/scripts/install/quantize_models.py)
uses static Conv-only QDQ INT8, 7-bit weights, folded BatchNorm, two fp32
stem convolutions and moving-average calibration on repository PDFs.
The [model notice](https://github.com/docling-project/docling.rs/blob/78c21e004662bb13b91b4c5a1ed0ac4d08c6bc2a/docs/MODELS_NOTICE.md)
attributes the layout model to Docling's Apache-2.0 model.

The export and quantization scripts are unchanged between the three
publishing heads. The calibration corpus gained
`tests/data/pdf/sources/table_misidentified_as_form.pdf` between September
15 and October 6, consistent with the logged sample increase. Export
dependencies are not fully pinned; the logged transformers version changed
again between October 6 and October 10. These are established changing
inputs, not proof that a particular one alone caused either digest change.
No original model bytes were recovered or tensor-by-tensor compared; no
claim of bitwise, numerical, or corpus-output equivalence is made.
The upstream gate compares a fresh INT8 export with its fresh fp32 parent,
not with our old pinned model. GitHub metadata and workflow logs are
provenance evidence, not a signed reproducible-build attestation.

## Local provisioning is not the mismatch source

Both failed jobs report misses for the native-assets cache and explicitly
download the same URL. The
[Native run](https://github.com/benpshore/pdftextract/actions/runs/38013284465)
and [bibliography run](https://github.com/benpshore/pdftextract/actions/runs/38013284457)
independently report the same size and digest. `native/fetch.sh` passes raw
model bytes through unchanged, hashes the destination, returns failure on
mismatch, and deletes a freshly downloaded mismatched destination. The
historical verified download rules out an originally mistyped/stale pin;
the pin became stale relative to the mutable upstream publication.
There is no evidence of a local transformation or poisoned restored model
cache in these two runs.

## Correction and validation

Only the layout digest changes, to the publication corroborated by release
metadata, source, successful export/gate logs and independent downloads.
The URL, release label, remaining artifact digests, fetch implementation,
native tests, evaluation corpus and acceptance thresholds are unchanged.
The manifest-hash cache key will change automatically. A cached original
model outside CI must still fail until deliberately re-fetched with
`sh native/fetch.sh --force`; it is never accepted under the new pin.
Future upstream replacements will again fail closed, as intended.

Local verification:

- `sh -n native/fetch.sh` and `git diff --check` passed.
- Two fresh model downloads matched GitHub's recorded new digest.
- `sh native/fetch.sh` verified all four models plus the pinned Linux x64
  PDFium library. PDFium needed manual extraction with `--no-same-owner`
  after its archive and library hashes were verified: this container cannot
  chown to the archive's UID. No fetch-script change was made for that local
  environment limitation.
- An isolated harness running the unchanged fetch script with the real
  current model bytes passed four cases: correct pin succeeds, wrong fresh
  download fails and removes the destination, wrong cached bytes fail, and
  an unpinned artifact fails without the explicit bootstrap override.
- Ruff format/check, all 210 Python tests and the uv dependency audit passed.
- Required pre-commit Rust, Swift and CMake commands were attempted but
  unavailable in this environment. Those results must come from Actions.

Publishing this correction to the existing PR branch requests new runs of
the existing affected workflows through the PR's already matching file
changes. Re-running the old SHA would reuse its old manifest and would not
test this correction. New-head workflow status must be inspected separately;
successful provisioning alone does not qualify native behavior or the full
backend evaluation. Independent review remains required before merge.
