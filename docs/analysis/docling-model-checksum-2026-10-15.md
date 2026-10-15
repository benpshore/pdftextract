# PR281 Native provisioning provenance — 2026-10-15

The unchanged main pin fails because the upstream mutable release no longer serves the September artifact. This checkpoint reuses only PR273's traced model correction from `7cabac188b5d89c5f0a9127392f5213df7d349a1`, with its historical evidence intact. The functional diff from PR281 `8758ffa530937b62395f8bee101b6a9387d67911` is one layout-model SHA-256 value.

The passing acquisition checkpoint remains on `codex/pr281-main-refresh-checkpoint-20261015` and existing PR281 remains at `8758ffa`. Main remains `b3730aa958d947fcbf6ea45b0efef07fcfa20f29`; the PR273 integration remains separately owned at `88822fddfca1c84f81bb3e9c2cd0bdd3ce8fbb35`. This checkpoint is a proposed provisioning repair for parent review, not Native acceptance or permission to merge.

## Exact failure

[PR281 Native run 38811957528](https://github.com/benpshore/pdftextract/actions/runs/38811957528), job `116687117541`, fails before compilation:

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| Original manifest pin | 68,695,321 | `d0eb9da0515a2121a20c3262f49ec4761109da45423b37c14b8b98b9759499b5` |
| Fresh workflow download | 68,695,321 | `87ac925d4183b4f4745fb48f6ff216fe88ce244d9ab635725e6d514fe490fc2a` |

Later missing-binary/evaluation failures follow from provisioning; they do not establish new extraction failures. PR281 did not change the manifest, fetcher, validator, policy or dependency lock from main.

The original pin was valid. A fresh, cache-miss download in [run 36437023879](https://github.com/benpshore/pdftextract/actions/runs/36437023879), job `108977471494`, independently recorded the original digest on September 28. That historical job subsequently failed tests; it is not presented as a successful full run.

## Fresh independent corroboration

The [October 11 investigation](docling-model-checksum-2026-10-11.md) remains a dated historical record. A fresh October 15 check found another upstream publication after that investigation:

- The [release API](https://api.github.com/repos/docling-project/docling.rs/releases/tags/models-v1) still reports release `351549900` as `immutable: false`.
- October 10 asset `627624021` now returns HTTP 404.
- Current asset [636374744](https://api.github.com/repos/docling-project/docling.rs/releases/assets/636374744), uploaded by `github-actions[bot]` at October 13 08:19:46–49 UTC, records the same 68,695,321 bytes and `87ac925d…` digest.
- [Publishing run 38367663071](https://github.com/docling-project/docling.rs/actions/runs/38367663071), source `a3c1ed9a657849127e45e9738d9af9ee55dddc90`, job `115196581868`, successfully exported, quantized and published that asset. Its fresh fp32/INT8 agreement gate reports 53 calibration samples, 539 confident fp32 detections and zero lost detections/tables.
- The export and quantization script Git blobs are identical between the October 10 and October 13 publishing heads: `3aea48b6345d3924e3873c2c636b6cba47aab88d` and `df1d67bb609ec98f6f6e58b660ff9779f943aefd` respectively.
- A fresh bounded HTTPS download in this VM independently matches the current API digest and size, and the previously verified PR273 model copy. No model code was executed during these checks.
- The other three current model assets still match their existing pins. Their hashes, PDFium pins, URL, fetch implementation, validator, historical exceptions and lockfile remain unchanged.

[Machine-readable evidence](docling-model-checksum-2026-10-15-evidence.json) records the observations, source identities, selected original log excerpts and recovery limits.

This supports identifying the exact official replacement artifact and reusing the already traced PR273 provisioning correction. It does not establish a reproducible source-to-byte attestation. The publisher still uses `--clobber`, the layout export dependencies are not fully pinned, and the Hugging Face model load has no revision argument. Equal October 10/13 digests establish byte identity for those publications, not equivalence to the original September artifact. The agreement gate compares a fresh INT8 model with its fresh fp32 parent.

## Verification and remaining gates

Six bounded offline fetch experiments passed using the unchanged PR281 fetch script and the fresh official asset: the proposed digest succeeds, the old digest rejects fresh and cached current bytes, corrupt fresh and cached bytes fail, and unpinned models remain refused. Fresh mismatched destinations are removed. The existing 139 provenance/measurement tests passed on Python 3.14.7. These tests establish verification behavior, not extraction quality.

The staged checkpoint passed Rust formatting, strict workspace Clippy, 1,129 Rust tests (10 ignored; nested probes are not double-counted), Ruff and all 210 Python tests on Python 3.14.7. The unchanged fetcher verified all four models and the pinned Linux x64 PDFium library. The local audit request failed at the OSV proxy; Swift and CMake are unavailable. The required commands were attempted and their failures retained.

A fresh full-feature build attempt stopped in `ort-sys` before model execution: the ONNX Runtime download from `cdn.pyke.io` returned CONNECT proxy 403. No restriction was bypassed. Existing hosted PR273 evidence is relevant comparison evidence, not an exact-head test of this new checkpoint. Full exact-head hosted Native qualification remains outstanding.

The historical Partial policy is unchanged at SHA-256 `ff92be145cb8fa57e0948f984088c21079c9ad5a85bdbdb27b53d5ceed33e234`. Its native-manifest pin remains `194f06dd…`; this proposed manifest hashes to `82094444…`. The historical lock is `ae0367f2…`; current lock is `c903547b…`. No historical exception is renewed by changing a digest.

PR273 already proves why provisioning success is insufficient: [run 38774398530](https://github.com/benpshore/pdftextract/actions/runs/38774398530) completed all four backend evaluations but failed strict validation. It retained historical-provenance rejection and 52 / 53 / 55 / 60 strict outcome errors for lopdf / PDFium / Docling-text / Docling, applying zero Partial exceptions. The retained bibliography regression for `arxiv:2503.04404` also remains. Those results belong to PR273's source; they are not rerun results for this checkpoint.

Original September model bytes were not recovered. The known September publishing run and first independently verified downstream run list no retained workflow artifacts, and the available local model is the new one. This is not proof that no trusted copy exists elsewhere. To preserve the old pin, the owner would need to provide a cloud-accessible original 68,695,321-byte artifact that independently hashes to `d0eb9da…`, or access to a retained trusted cache/backup containing it. No Mac access is needed or authorized.

Parent review must decide whether to advance this scoped replacement-artifact correction within the existing PR281/integration workflow. Any update then needs exact-head Native evidence and keeps current quality/security holds. No pin restamping, validator relaxation, new PR, release, protected-branch merge or other PR closure is part of this checkpoint.
