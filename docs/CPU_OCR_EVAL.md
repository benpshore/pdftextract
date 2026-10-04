# CPU OCR evaluation evidence

`scripts/cpu_ocr_eval.py` runs the explicit local [CPU OCR CLI](CPU_OCR.md) against
synthetic fixtures and an optional pinned public corpus. Run this evaluation on
the **Codex Cloud executor**, outside GitHub CI. CI only runs the lightweight
schema, scoring, error-retention and process-supervision regressions in
`tests/test_cpu_ocr_eval.py`.

The evaluator creates a **new dedicated SQLite database**, preserving original
input bytes and the actual OCR output. It never opens an existing database for
replacement. Databases are local artifacts and must not be committed.

## Reproduce

First acquire the pinned public subset with the repository's corpus fetcher:

```sh
UV_CACHE_DIR=/workspace/scratch/uv-cache uv run python scripts/fetch_ocr_corpus.py \
  --destination /workspace/scratch/ocr-public-corpus
```

Use the resolved local manifest it produces, retaining the upstream license,
README and ground truth files. The committed acquisition specification is
[`validation/cpu-ocr-public-corpus.json`](validation/cpu-ocr-public-corpus.json).
The corpus loader rechecks all manifest file hashes, both input image hashes,
the upstream ground-truth hash, and agreement between inline and upstream truth.

The example below names this executor's actual native Poppler binaries. The
shorter paths through `dependencies/bin/override` and
`dependencies/native/poppler/bin` are shell launchers and are not accepted by
the OCR CLI. On another executor, supply the corresponding actual executables
and English `eng.traineddata`; no runtime/model download happens during evaluation.

```sh
POPPLER_DATADIR=/opt/codex/runtimes/codex-primary-runtime/dependencies/native/poppler/poppler/share/poppler \
UV_CACHE_DIR=/workspace/scratch/uv-cache uv run python scripts/cpu_ocr_eval.py \
  --database /workspace/scratch/cpu-ocr-evaluation-font-qualified.sqlite \
  --summary /workspace/scratch/cpu-ocr-evaluation-font-qualified.json \
  --tessdata-dir /usr/share/tesseract-ocr/5/tessdata \
  --pdftoppm /opt/codex/runtimes/codex-primary-runtime/dependencies/native/poppler/poppler/bin/pdftoppm \
  --pdfinfo /opt/codex/runtimes/codex-primary-runtime/dependencies/native/poppler/poppler/bin/pdfinfo \
  --public-manifest /workspace/scratch/ocr-public-corpus/manifest.json \
  --fixture-font-file /usr/share/fonts/opentype/urw-base35/NimbusSans-Regular.otf \
  --workers 2 --repetitions 2 \
  --execution-note 'Other workstream CPU-heavy validation paused for this run'
```

Both database and summary paths must be new. Choose fresh names for later
evaluations; retained runs must not be overwritten. The adjacent
`cpu-ocr-evaluation-font-qualified-artifacts/` directory holds the generated inputs and original
JSONL/TSV diagnostics for ordinary file inspection. The database also stores the
inputs, raw JSONL, raw text, raw TSV, runtime/model binaries, and corpus provenance
assets as SHA-256-addressed BLOBs, so those outputs remain inspectable after the
adjacent directory is removed. Shared native libraries and operating-system
images are not bundled or claimed to be fully reproducible.

The explicit font option creates a private Fontconfig file and a directory
containing only the byte-identical selected font. It changes this evaluation
process's environment, not any user or system setting. Its configuration and
font bytes/hashes are retained in `components`. An optional
`--fixture-font-probe PATH` retains a pre-OCR native file-open trace. Omit the
font option only when deliberately testing the inherited rendering environment.

## Fixtures and limits of the measurement

The six synthetic inputs are a clean PNG, image-only scan PDF, 90-degree rotated
PNG, degraded scan PDF, a two-page native-text/image-scan PDF, and an intentionally
malformed PDF. The text describes a fictional garden-robot study. It was created
for these tests and is dedicated under CC0-1.0. Degradation has a fixed random seed
of 1729, a contrast multiplier of 0.5, an offset of 100, and uniform pixel noise
from -35 to 35. The source PDF and generator SHA-256 are retained. PDF rasterization
uses 150 DPI. Synthetic line regions are nominal generator regions, not
glyph-tight annotations.

The published subset contains the original `testing/phototest.tif` and its
published left-rotated PNG from the official Tesseract test repository, plus
`phototest.gold.txt`. Its Apache-2.0 license and test-corpus README are retained.
The exact repository commit, tree, archive checksum, image/truth revisions,
addition dates, retrieval date, source URLs and fixture identifiers are stored.
DOI and publication/version fields remain null where upstream supplies none.
This is a predetermined, known Tesseract test subset, **not a holdout accuracy
corpus**. It provides an independently published regression example, not a
general OCR quality estimate. No word-region ground truth is invented for it.

Every OCR invocation is a fresh Python/Tesseract process. `cold` means the first
invocation of a fixture in this evaluation; `warm` means its repeated invocation
in the same executor. OS caches are not flushed, and fixture creation already
uses Poppler. These are first/repeated process measurements, **not machine-cold
or resident-model timings**. Provisioning is not included: binaries/models are
already available offline. Fixture preparation is timed separately. Attempt
wall time includes process startup, identity/model hashing, source snapshot,
rasterization, OCR, parsing and shutdown; page events retain rasterization/OCR/
parse/total timings separately.

An outer sequential loop submits one input at a time; the OCR CLI uses the
requested bounded page-worker count (two for the recorded qualification) with
one OpenMP thread per native process. Each invocation has a 30-second deadline
and each worker/native process has a 1024 MiB address-space limit. The evaluator
adds a 120-second watchdog; cancellation signals the OCR supervisor, which owns
worker cleanup. Queued inputs in an interrupted run are recorded as unattempted,
and their expected pages count as complete omissions. This is a qualification
CLI, not integration with a UI thread or the Rust provider registry.

The environment snapshot independently records CPU affinity, cgroup CPU quota,
cpuset, memory limit and CPU model. Cgroup memory values may include other work
in the executor. Per-attempt `os.wait4` resource measurements and per-page runtime
`getrusage` maxima are **individual process maxima, not a sampled sum of all
concurrent processes**. `process_max_rss_kib` is therefore not a total memory
budget or a whole-tree peak claim. The SQLite row identifies every measurement's
scope and method. This Linux executor cannot validate iPhone speed or hosting
capacity.

The outer invocation's `wait4` high-water mark also includes inherited process
creation memory from the evaluator. In the measured runs it was much larger than
the OCR workers' own reported peaks. The second run additionally records current
evaluator RSS before spawn; neither value is an OCR-only resident-memory metric.
Use the separately named native-child and worker maxima with their stated scope.

## Accuracy, failures and database inspection

Character error rate is Levenshtein edits divided by reference characters after
NFKC, case folding and whitespace collapse. Word error rate uses Unicode word
tokens under the same normalization. Word omissions are the multiset deficit
of expected tokens, including substitution mismatches; they are not alignment
deletions. Raw text is retained unchanged alongside these derived scores. A
missing expected page gets empty output and CER/WER 1. The malformed PDF has no
ground-truth page and is counted separately as an expected operational failure.
Successful execution does not imply correct OCR: rotated-input omissions and
incorrect words remain visible even when Tesseract exits successfully.

## Measured Codex Cloud qualification, 2026-10-04

Two evaluations are retained independently:

| Evaluation | SQLite artifact | Committed JSON evidence |
| --- | --- | --- |
| Initial inherited-font environment | `/workspace/scratch/cpu-ocr-evaluation.sqlite` | [`cpu-ocr-eval-20261004.json`](validation/cpu-ocr-eval-20261004.json) |
| Qualified explicit-font environment | `/workspace/scratch/cpu-ocr-evaluation-font-qualified.sqlite` | [`cpu-ocr-eval-font-qualified-20261004.json`](validation/cpu-ocr-eval-font-qualified-20261004.json) |

The first run's intended `clean_*` synthetic fixtures visibly contained uneven
glyph spacing and overlapping characters. These results are preserved unchanged;
they are confounded by a fixture-rendering defect. Investigation before the
second OCR run showed that the bundled native rasterizer could open a different
font from the one reported by the system `fc-match` command. A file-open trace
then confirmed that a private directory containing only
`NimbusSans-Regular.otf` makes the same rasterizer open that exact font. The
resulting raster was visually checked by two workstream owners **before OCR**.
The second run retains the exact font, configuration and trace. No text truth,
public input, PSM, DPI, model, orientation or dewarping algorithm was changed.

Word error rates below are identical between first and repeated invocations
within each evaluation. WER can exceed 100% when insertions outnumber reference
words. These are descriptive results from a small qualification set.

| Fixture/page | Initial WER | Qualified-font WER | Qualified-font CER |
| --- | ---: | ---: | ---: |
| Clean PNG | 59.62% | 0% | 0% |
| Image-only scan PDF | 46.15% | 0% | 0% |
| Synthetic 90-degree rotated PNG | 228.85% | 201.92% | 104.21% |
| Degraded scan PDF | 50.00% | 1.92% | 0.32% |
| Mixed document, native-text page | 59.62% | 0% | 0% |
| Mixed document, scanned page | 46.15% | 0% | 0% |
| Published Tesseract phototest | 0% | 0% | 0% |
| Published left-rotated phototest | 100% | 100% | 78.17% |

The qualified degraded page has one word mismatch. The synthetic rotated page
has 46 missing reference tokens and 103 extra tokens; the published rotated
fixture has 59 missing and 57 extra tokens. Rotation handling is a demonstrated
limitation, despite successful process exits. Each database contains 16 attempts,
16 scored page results from eight fixtures over two phases, and two intentionally
malformed-PDF failures. `expected_outcome_met` is only an **operational outcome**
check, not an accuracy pass. The malformed PDFs retain both the CLI error and
absence of a normal summary as four error rows per database.

Qualified first/repeated attempt wall times, including startup and rasterization:

| Input | First process | Repeated process |
| --- | ---: | ---: |
| Clean PNG | 0.423 s | 0.423 s |
| Scan PDF | 0.644 s | 0.625 s |
| Synthetic rotated PNG | 0.644 s | 0.624 s |
| Degraded scan PDF | 0.685 s | 0.624 s |
| Two-page mixed PDF | 0.646 s | 0.627 s |
| Invalid PDF | 0.262 s | 0.283 s |
| Published phototest | 0.383 s | 0.383 s |
| Published rotated phototest | 0.484 s | 0.504 s |

The executor exposed five affinity CPUs, a **four-CPU cgroup quota**
(`400000 100000`) and a **16 GiB memory limit**. No eight-core allocation was
assumed. CPU-heavy work in the other workstreams was paused for both windows.
The initial run shared cgroup memory with an idle GROBID service; the service had
been removed before the second run. Cgroup memory is not attributed to OCR.

The qualified runtime reported native-child process maxima of 27,320–35,092 KiB
and worker maxima of 20,868–20,916 KiB. The outer `wait4` maximum was 130,184 KiB,
including the inherited spawn high-water mark; it must not be presented as an
OCR-only peak or a simultaneous worker-tree sum. Tesseract 5.5.0, Poppler 26.05.0,
Python 3.14.7 and uv 0.12.19 are recorded with hashes. The unversioned installed
English model is pinned by SHA-256
`7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2`.
Rust 1.98.1, Cargo 1.98.1, Clippy 0.1.98 and rustup 1.28.2 are captured as
environment facts; no Rust OCR provider was exercised by this Python CLI.

Both databases pass SQLite integrity/foreign-key checks and independent byte
length/SHA-256 verification of every retained artifact. The initial run has
1,012 word regions, 82 warnings and 47 unique artifacts; the qualified run has
1,032 word regions, 72 warnings and 50 unique artifacts. Its native stderr
sidecars are retained in the adjacent artifacts directory; the measured CLI
revision exposes raw TSV in JSONL but does not expose diagnostic file paths,
so `page_artifacts` is empty in these two databases. Raw JSONL/text/TSV, source
and truth bytes, model/runtime identities and all warnings/errors are in SQLite.

## Schema and queries

[`scripts/cpu_ocr_eval_schema.sql`](../scripts/cpu_ocr_eval_schema.sql) defines
the normalized tables:

| Tables | Evidence |
| --- | --- |
| `artifacts`, `sources`, `source_assets` | Immutable byte content, hashes, license/repository provenance and retained corpus files |
| `fixtures`, `fixture_pages`, `truth_regions` | Fixture transformations, independently supplied truth, expected page geometry and nominal synthetic regions |
| `environments`, `components`, `run_components` | Allocated resources, exact source/runtime/model binaries and versions/hashes |
| `runs`, `attempts`, `events` | Phase semantics, exact commands/configuration, invocation timing/outcome and raw JSONL events |
| `page_outputs`, `page_artifacts`, `ocr_regions`, `accuracy` | Per-page raw text/TSV and diagnostics, pixel-coordinate word boxes/confidence, provenance and accuracy |
| `warnings`, `errors`, `resource_measurements` | OCR limitations, failures/cancellation/missing pages and measured resource scope |

Example queries for any SQLite viewer:

```sql
PRAGMA integrity_check;
PRAGMA foreign_key_check;
SELECT * FROM page_scores ORDER BY repetition, fixture, page_number;
SELECT r.phase, f.name, a.outcome, a.expected_outcome_met, a.elapsed_seconds
FROM attempts a JOIN runs r ON r.id=a.run_id
JOIN fixtures f ON f.id=a.fixture_id ORDER BY a.id;
SELECT f.name, s.license, s.repository_commit, s.fixture_identifier,
       s.fixture_added_at, s.doi, b.sha256, b.byte_count
FROM fixtures f JOIN sources s ON s.id=f.source_id
JOIN artifacts b ON b.id=s.original_artifact_id;
SELECT attempt_id, page_number, metric, value, unit, method
FROM resource_measurements ORDER BY attempt_id, page_number;
SELECT attempt_id, page_number, category, message FROM errors;
```

Expected acceptance is complete evidence and honest measured behavior: unchanged
input bytes, all expected pages scored, exact runtime/model identity, word boxes
and raw TSV retained, failures preserved, successful SQL integrity/foreign-key
checks, and actual CPU OCR output measured in this executor. Rotation correction,
dewarping, multilingual coverage and general scholarly-document quality remain
separate work. The future user-supplied dewarping implementation is not available
and is not approximated here.
