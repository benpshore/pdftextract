# Opt-in CPU OCR orientation qualification

This follow-up addresses the rotated-fixture accuracy gap in issue #197 under
epic #199. It starts from draft PR #206 at
`df395cc4be6792d0c26e2226a72ad7aa83d9500b` on a separate branch. All PR #206
runtime JSON evidence and both original SQLite databases remain unchanged.
Required CI and all three native binary builds for that exact PR #206 head
passed on 2026-10-04.

## Diagnosis and supported behavior

The original `--psm 6` selects a single text block without automatic orientation.
It remains the default. The opt-in `--psm 1` path uses native Tesseract automatic
page segmentation with orientation detection. It requires and snapshots both
the selected recognition language and `osd.traineddata`; the latter is recorded
as an orientation model using the legacy classifier, distinct from LSTM text
recognition. No dependency or toolchain pins change.

A bounded, separate PSM 0 diagnostic records orientation, recommended clockwise
rotation, script, and raw Tesseract scores. These scores are not probabilities.
That diagnostic is not proof of the internal transform later selected by the
separate PSM 1 recognition process; `internal_applied_rotation_degrees` remains
null. Raw diagnostic artifacts and warnings are retained. Sparse pages with
insufficient orientation evidence remain explicit Partial results. Missing or
invalid orientation models fail explicitly.

No external pixel transform is applied. TSV word boxes remain in the original
input raster's top-left coordinate frame. Tests check 90, 180 and 270 degree
word-box mappings, including exchanged width/height, against upright input.
This matches Tesseract 5.5.0's source, which re-rotates iterator boxes and maps
them to input-image coordinates before TSV emission:
[page iterator](https://github.com/tesseract-ocr/tesseract/blob/5.5.0/src/ccmain/pageiterator.cpp#L312-L350)
and [TSV writer](https://github.com/tesseract-ocr/tesseract/blob/5.5.0/src/api/baseapi.cpp#L1246-L1330).
A PDF-to-raster transform is still unverified and remains null. Correct raster
boxes do not establish PDF crop, rotation, point-space, or semantic provenance.

Use the existing qualification command with `--psm 1` and a trusted model
directory containing both `eng.traineddata` and `osd.traineddata`. Select new
output/artifact paths. The same worker pool, memory bound, deadline, process
group cancellation and descendant reaping cover both native stages. Emitted
page and page-error diagnostic bytes are retained by the SQLite evaluator;
hard termination can still interrupt an in-progress page before its temporary
diagnostics are published, and does not count as success.

Right-angle page orientation is separate from skew correction, curved-page
dewarping and camera perspective correction. Ben's personal dewarping program
remains deferred until he supplies it; this work does not substitute an algorithm
for it or claim its effects.

## Separate measured result

The [new evaluation report](CPU_OCR_ROTATION_EVAL.md) imports the exact synthetic
rotated source BLOB and truth from the font-qualified baseline and uses the
unchanged pinned public manifest. No input was rerendered to improve a score.
The new database contains three fixtures, six attempts and six scored pages:

| Fixture | Baseline PSM 6 WER | New PSM 1 WER, first / repeated | New elapsed seconds, first / repeated |
| --- | --- | --- | --- |
| Published clean phototest | 0% | 0% / 0% | 0.891 / 0.925 |
| Published left-rotated phototest | 100% | 0% / 0% | 0.927 / 0.889 |
| Retained synthetic rotated image | 201.92% | 0% / 0% | 1.131 / 0.986 |

All six new pages also have zero normalized character error and zero token
omissions. WER can exceed 100% when insertions occur. This is a targeted
regression repair using known fixtures, not independent general accuracy
qualification. The clean public control remains correct. The original degraded,
mixed-page and malformed fixtures were not rerun in this follow-up.

The report retains 342 word regions, 26 warnings, zero recorded errors and
24 page-diagnostic artifact links. Native-child peak RSS is 66,124–72,592 KiB;
worker peak RSS is 21,144–21,160 KiB. These are separate per-process maxima.
The outer wait4 peak of 122,168 KiB includes process-spawn/evaluator inheritance;
none is an aggregate simultaneous process-tree memory estimate. First/repeated
invocations use fresh processes and unflushed filesystem caches on the same
four-CPU-quota, 16 GiB executor. Timings include the added orientation diagnostic.

The new database is `cpu-ocr-evaluation-rotation-psm1.sqlite`, 47,939,584 bytes,
SHA-256 `c4e24c6ad6cab9d3e51941ab6aaaa09f8b040c2d54e36876da861520533e03ca`.
Its [JSON summary](validation/cpu-ocr-eval-rotation-psm1-20261004.json) and
reproduction instructions are committed; databases remain outside git.

The full local suite passes 141 tests. Independent focused review exercised
28 worker tests and the evaluator's diagnostic/provenance regressions, including
missing/corrupt/sparse orientation models, all three right-angle word-box
mappings, active native OSD timeout/SIGTERM cleanup, and retained failure
diagnostics without excluding missing pages from accuracy denominators.

## Immutable corpus provenance and durable evidence

The [read-only provenance audit](validation/cpu-ocr-corpus-provenance-audit-20261004.json)
verified both original database hashes, SQLite integrity/foreign keys, all
retained artifact BLOB lengths/hashes, and the published sources' official
repository URL, commit/tree, repository checksum/method, per-file identifiers,
checksums, addition/retrieval dates, unchanged truth and license/scope records.
Absent release version, publication date and DOI are explicitly null.

The official corpus commit is
`232ff181c66516116ec0e84c4963f70de15050fd`. A separate deterministic
`git archive --format=tar` recomputation matched the stored repository SHA-256
`3e944b19628d9b805da5ebd1a2fdd75436ec2de8c23f0b546a66950bce0e4d1f`.
The full archive is included in the durable bundle, so that checksum is now
independently verifiable offline; the complete archive is not a SQLite BLOB.
Only the selected two phototest samples contribute to the public-corpus metrics.

The user-facing Library artifact
`pdftextract-cpu-ocr-evidence-20261004-pr206.tar.gz` contains both original SQLite
databases, the exact PR #206 source snapshot, raw evidence/reports, validation
logs, the provenance audit and the pinned public corpus archive. It is
50,862,409 bytes, SHA-256
`cf104511a1cfdeb8a6e722a62dc966b69222ae2f4c3f28074d61adc489236ff3`.
Its `MANIFEST.json` records every payload hash, and all entries were read back
and verified before upload. This private Library delivery does not publish user
documents or depend on the executor's filesystem remaining available.

The original databases remain:

- Initial: `5cac89b7fbaaeb1d945e70cf46ff3cf2529946b7c903ad0f0698016ba587ba17`.
- Font-qualified: `ec01327b4714832a840e39e883c4a99dd9a2c253d4bdeafbde400b457b3fdd5c`.

This archived checkpoint preserves the initial rendering defect and all rotation
failures. New orientation measurements use a separate database and report;
they never replace the baseline's outputs, truth or errors.
