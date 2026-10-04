# Published-fixture rotation follow-up

This follow-up keeps both original evaluation databases and all PR #206 evidence
unchanged. The new evaluation uses the exact two published Tesseract test
images and their retained upstream `phototest.gold.txt`, plus the exact rotated
synthetic image and truth imported from the qualified baseline database. Its scope is an explicit
native Tesseract PSM 1 option. Default PSM 6 behavior is unchanged. No dewarping,
external image rotation, truth rewriting, new dependency pin, or browser/UI
integration is introduced.

## Retained corpus provenance audit

[`cpu-ocr-corpus-provenance-audit-20261004.json`](validation/cpu-ocr-corpus-provenance-audit-20261004.json)
was produced by opening both original databases with SQLite
`mode=ro&immutable=1`. It independently verifies database SHA-256, every artifact
SHA-256/length, integrity/foreign keys, and normalized metadata against each
database's own retained acquisition manifest. Both published sources in both
databases preserve:

- Official `tesseract-ocr/test` repository identity, pinned commit/tree, and the
  deterministic repository archive checksum and its method.
- Per-image original bytes, SHA-256, fixture identifier, revision and addition
  date; upstream truth bytes, SHA-256, revision and addition date.
- License and test-corpus README bytes, source URLs, retrieval date, and the
  upstream null DOI/publication/version fields without fabricated values.

The images were added in 2018 and predate this evaluation. Their repository
snapshot is pinned to `232ff181c66516116ec0e84c4963f70de15050fd`. The audit checks
the retained archive-checksum metadata against the retained manifest; it does
not claim to recompute the entire repository archive from SQLite, which contains
the selected corpus files rather than the complete repository.

## What the selected mode means

The opt-in `--psm 1` path snapshots and hashes the installed `osd.traineddata` in
addition to `eng.traineddata`. It runs a separate native PSM 0 orientation
diagnostic, then native PSM 1 recognition. The diagnostic's observed orientation,
suggested clockwise rotation and confidence are reported with their source.
They do not prove the internal rotation chosen by the subsequent PSM 1 call;
that unavailable value remains null. Raw diagnostic stdout/stderr and OCR TSV
are retained, including diagnostics on failed pages.

PSM 1 changes page segmentation as well as enabling orientation handling. The
comparison therefore measures the combined selected mode, not a causal estimate
of rotation correction alone. Ground truth is used only afterward by the
evaluator's scoring function; it is never provided to Tesseract or used to select
orientation. The original input bytes remain unchanged. Source pixel coordinate
frames and the native runtime's geometry evidence remain attached to each page
event and word region. This is neither a dewarping implementation nor evidence of
mobile GPU/Neural Engine performance.

## Reproduce a separate evaluation

Acquire the unchanged pinned corpus using
[`fetch_ocr_corpus.py`](../scripts/fetch_ocr_corpus.py) if it is not already present.
Supply fresh database and summary names; existing artifacts are never overwritten.

```sh
UV_CACHE_DIR=/workspace/scratch/uv-cache uv run python scripts/cpu_ocr_eval.py \
  --database /workspace/scratch/cpu-ocr-evaluation-rotation-psm1.sqlite \
  --summary /workspace/scratch/cpu-ocr-evaluation-rotation-psm1.json \
  --tessdata-dir /usr/share/tesseract-ocr/5/tessdata \
  --pdftoppm /opt/codex/runtimes/codex-primary-runtime/dependencies/native/poppler/poppler/bin/pdftoppm \
  --pdfinfo /opt/codex/runtimes/codex-primary-runtime/dependencies/native/poppler/poppler/bin/pdfinfo \
  --public-manifest /workspace/scratch/ocr-public-corpus/manifest.json \
  --retained-database /workspace/scratch/cpu-ocr-evaluation-font-qualified.sqlite \
  --retained-fixture rotated_image \
  --public-only --psm 1 --workers 2 --repetitions 2 \
  --execution-note 'Reserved Codex Cloud CPU window; same pinned published images and truth'
```

This runs six attempts: the three retained inputs in first/repeated-process
phases. There is no synthetic fixture regeneration or font configuration in this
mode. The synthetic input is copied from its immutable original BLOB; its bytes,
truth and nominal truth regions are retained without rerendering. The old database
hash and generator provenance are recorded in the new fixture's `generator_json`.
The shared harness still records detected Poppler/font inventory as component
metadata; those tools/fonts did not render or transform any input in this run.
Runtime startup, model identity/snapshot, the separate orientation
diagnostic, native PSM 1 recognition and parsing all contribute to measured
attempt time. The timing therefore includes diagnostic overhead and is not an
optimized resident-model throughput claim. The prior cold/warm definitions,
bounded workers/deadlines, per-process resource scope and omission scoring remain
as documented in [CPU_OCR_EVAL.md](CPU_OCR_EVAL.md).

The corpus is a predetermined, known Tesseract regression subset, not a holdout
accuracy corpus. Both poor prior rotation results and every new result remain
available; this small experiment cannot establish general rotation robustness.

## Measured retained artifacts

The completed evaluation is
`/workspace/scratch/cpu-ocr-evaluation-rotation-psm1.sqlite` (47,939,584 bytes,
SHA-256 `c4e24c6ad6cab9d3e51941ab6aaaa09f8b040c2d54e36876da861520533e03ca`).
The committed export is
[`cpu-ocr-eval-rotation-psm1-20261004.json`](validation/cpu-ocr-eval-rotation-psm1-20261004.json).
It contains six successful attempts, six scored pages, 342 word regions,
26 warnings, zero errors, and 24 retained diagnostic artifact links. SQLite
integrity/foreign-key checks and every artifact SHA-256 check pass.

All three fixtures achieved zero character error rate, word error rate and
missing reference tokens in both first/repeated-process phases. The unchanged
qualified PSM 6 baseline had WER 100% on the published rotated image and 201.92%
on the synthetic rotated image; its published clean image already had zero WER.
This improvement applies to the combined PSM 1 mode on these three inputs.

| Fixture | First process | Repeated process | Separate diagnostic suggestion |
| --- | ---: | ---: | --- |
| Published clean phototest | 0.891 s | 0.925 s | 0 degrees clockwise |
| Published left-rotated phototest | 0.927 s | 0.889 s | 90 degrees clockwise |
| Retained synthetic clockwise-rotated image | 1.131 s | 0.986 s | 270 degrees clockwise |

The diagnostic orientation scores were respectively 10.86, 9.62 and 11.11;
these are Tesseract scores, not probabilities. The native internal applied
rotation remains unknown/null. All six page events retain input-identical
raster hashes, original-input dimensions and `tsv_to_input_affine` identity
`[1, 0, 0, 1, 0, 0]`. Raw OSD output, stdout, stderr and OCR stderr are stored
as `page_artifacts` BLOBs; source-frame word boxes are in `ocr_regions`.

The added OSD model is pinned by SHA-256
`9cf5d576fcc47564f11265841e5ca839001e7e6f38ff7f7aacf46d15a96b00ff`;
the English recognition model and native Tesseract binary match the baseline.
Native-child process maxima were 66,124–72,592 KiB and worker maxima were
21,144–21,160 KiB. The separate outer invocation high-water mark was 122,168 KiB,
including inherited process-creation memory; it is not an OCR-only or summed
process-tree peak. The executor again reported a four-CPU quota, five affinity
CPUs and a 16 GiB memory limit. Other workstream tests were paused during the
measured interval.

For retrievable provenance and orientation evidence, query:

```sql
SELECT name, generator_json FROM fixtures;
SELECT * FROM sources;
SELECT json_extract(payload_json, '$.orientation')
FROM events WHERE json_extract(payload_json, '$.event') = 'page';
SELECT p.page_number, a.role, b.sha256, b.content
FROM page_outputs p JOIN page_artifacts a ON a.page_output_id=p.id
JOIN artifacts b ON b.id=a.artifact_id;
```
