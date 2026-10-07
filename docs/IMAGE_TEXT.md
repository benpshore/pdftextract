# Image to text (`tpe-image-text`)

`crates/tpe-image-text` turns still images into text: one `<stem>.txt` and one
`<stem>.json` per input, written by whichever OCR engine is really available.
It never downloads anything at run time, never modifies an input, never
overwrites an output without `--force`, and never fabricates text: with no
engine it exits 2 and says exactly what was probed. This page describes what
the crate does today, as verified by its tests; it is not a roadmap.

```sh
cargo build -p tpe-image-text
tpe-image-text scans/ photo.jpg --out out/ --recursive
tpe-image-text page.png --out out/ --engine tesseract --lang deu
tpe-image-text --list-engines            # what can run here, and why not
tpe-image-text scans/ --out out/ --json  # one JSON document for the run
```

## Inputs and outputs

| Input | Behaviour |
| --- | --- |
| PNG, JPEG, TIFF, BMP, WebP | Decoded with the `image` crate (only these decoders are compiled in). The format is detected from the leading bytes; a mismatching extension is a warning in the report. |
| GIF | First frame only; an animated file gets the warning `gif: animated; only the first frame was read`. |
| HEIC/HEIF | Recognised by its `ftyp` brand and refused: `unsupported: <path>: HEIC/HEIF is not decodable in this build (no libheif is linked); convert it to PNG or JPEG first`. The file counts as failed (exit 1), everything else is still processed. |
| Anything else | `unsupported: <path>: not a recognised image …`. |

Files named on the command line are always attempted. Directories contribute
files whose extension is one of `png jpg jpeg jpe tif tiff bmp webp gif heic
heif`, sorted by name; subdirectories need `--recursive` (otherwise a note is
printed). Decoding refuses images larger than 20000 px per side or 1 GiB of
pixel memory.

Outputs go to `--out DIR` (created if missing):

- `<stem>.txt`: blocks in reading order separated by a blank line, lines
  within a block separated by `\n`, trailing newline. Empty when nothing was
  recognised (and the report then carries the warning `no text recognised`).
- `<stem>.json`: tool name/version, input path, size and sha256, output paths,
  engine `{name, version, lang, resource_limits_applied}`, decoded image
  `{format, width, height}`, the preprocessing report (steps applied,
  contrast bounds, Otsu threshold, inversion, estimated text height, upscale
  factor, deskew degrees, output size), `blocks[]` with `index`, `text`,
  `confidence` (0..1 or `null`) and `bbox` in **preprocessed-image pixels**
  (`bbox_space` says so; divide by `preprocessing.upscale_factor` to approximate
  input pixels when no deskew was applied), `text_chars`, `timing_ms`
  `{decode, preprocess, ocr, write, total}` and `warnings[]`.

Two inputs with the same stem (`x/scan.png`, `y/scan.jpg`) map to the same
output names; the second fails with `output exists` rather than overwriting the
first, even with `--force` absent or present for the first. Pick distinct
stems or separate `--out` directories.

Publication refuses an output that is an input path, a hard link to an
input, or a symlink resolving to it, including other inputs in the batch
and including with `--force`. A dangling
symlink counts as an existing output without `--force`. With `--force`, a
legitimate output entry (including a dangling symlink) is replaced; its
symlink referent is never written. Both complete files are staged and synced
before either final name changes. Existing forced outputs are retained in
a private sibling recovery directory until both exclusive publications and
the directory sync succeed. A handled failure removes newly published files
owned by this operation and restores old entries; incomplete rollback names
the retained directory (`0` is text, `1` is JSON) for manual recovery.

This is a cooperating-writer contract: keep the input and output directories
stable while a call runs. The source is checked by an open file identity
before OCR and again before publication; pathname checks and renames are
not a defense against hostile concurrent directory replacement. Each new
file appears complete, but the pair is not a single atomic transaction;
forced names can be briefly absent during replacement. Process termination
or power loss can leave a partial pair and recovery files. Crash recovery,
concurrent source-content mutation and hostile writers are not verified.
The report's write/total timings are captured before final publication.

Exit codes: 0 every input written; 1 at least one input failed (the others are
still written); 2 no engine available, unusable arguments, or no input found.

## Engines

`--engine auto` (default) takes the first available of `tesseract`, `ocrs`,
`docling`. `--list-engines` prints the probe for each, `--json` makes it JSON.

### tesseract (external program)

Used when a `tesseract` executable is found: `--tesseract-bin PATH`, then the
`TPE_TESSERACT_BIN` environment variable, then `PATH`. The version comes from
`tesseract --version`. Each image is written as a temporary PNG and recognised
with `tesseract <png> stdout -l <lang> --psm 3 tsv`, `OMP_NUM_THREADS=1`, and:

- a wall-clock deadline (`--timeout-secs`, default 120): the controller polls,
  kills and reaps the process, and the file fails with `timed out after …`;
- kernel limits installed by re-executing this binary as
  `tpe-image-text exec-limited <cpu-seconds> <address-space-bytes> <program> <args>`,
  which lowers `RLIMIT_CORE` (0), `RLIMIT_CPU` (timeout, hard +5 s),
  `RLIMIT_FSIZE` (256 MiB) and `RLIMIT_AS` (`--max-memory-mib`, default 2048),
  each one independently and never above an inherited limit, on itself through
  `rustix` and then `exec`s tesseract, which inherits them. A limit the kernel
  refuses or does not keep is skipped, not fatal (macOS answers `EINVAL` for
  `RLIMIT_AS`): the helper prints one `exec-limited: applied …; skipped …` line
  on stderr, the controller turns it into `engine.resource_limits_applied`
  (the list of limits in force: `["core","cpu","fsize","as"]` on Linux) plus a
  `resource limit not applied: <name>: <reason>` warning per skipped limit, and
  the wall-clock kill remains the guarantee. No `unsafe`, no shell: every
  argument is its own `OsString`;
- stdout/stderr spooled to files (no pipes to deadlock), 32 MiB read back,
  tesseract's stderr lines kept as warnings, a non-zero exit quoted as the error.

The TSV gives word confidences; a block is a tesseract paragraph, its
`confidence` the mean word confidence /100, its `bbox` the union of its words.
`--lang` is validated (`eng`, `chi_sim`, `eng+fra`, `script/Latin`); anything
that could be read as an option is rejected before spawning.

### ocrs (pure Rust, feature `ocrs`, on by default)

[ocrs 0.13.1](https://github.com/robertknight/ocrs) on the `rten` 0.26 runtime,
with the two model files pinned by size and sha256 in
`crates/tpe-image-text/models/manifest.json` (2.4 MiB + 9.3 MiB, Apache-2.0,
from the ocrs project). Provision them once:

```sh
sh crates/tpe-image-text/fetch-models.sh            # into .models/ocrs/, verified
sh crates/tpe-image-text/fetch-models.sh --print-hashes
```

The script mirrors `native/fetch.sh`: one URL per file, no mirrors, size and
digest checked, a mismatching download deleted, exit 1 on any failure. The
engine repeats the check before loading (`--ocrs-models DIR`,
`TPE_OCRS_MODELS_DIR`, else `.models/ocrs` under the working directory): a
missing, short or altered file makes the engine unavailable with that reason.
The models are English/Latin-script only; `--lang` other than `eng` fails the
file with `unsupported: ocrs has only the English/Latin-script models …`. ocrs
exposes no per-character confidence, so `confidence` is `null` and a warning
says so. Blocks are reading-ordered lines grouped by vertical gap.

Speed depends on the build profile: rten is unoptimised in `dev` builds, and
on a shared 4-CPU machine the 420x90 px fixtures took 12 to 25 s each there
(`timing_ms.ocr`); use `cargo build --release -p tpe-image-text` for real use.
No throughput claim is made beyond that measurement.

### docling (detected, not runnable here)

docling.rs 1.69.2 performs OCR with PP-OCR ONNX models only inside its PDF
pipeline (`docling-pdf` feature `ml`: ONNX Runtime fetched at build time plus
PDFium); the root crate's `docling-text` backend parses PDF text layers and does
no OCR at all ([docs/DOCLING.md](DOCLING.md)). This tool links neither. The probe
reports whether `ocr_det.onnx`, `ocr_rec_en.onnx` and `en_dict.txt` from
`native/manifest.json` are present (`DOCLING_RS_MODELS_DIR`, else `.models`)
and always ends with the reason the engine cannot run. `--engine docling`
therefore exits 2 with that message.

## Preprocessing (pure Rust, reported)

In order, each recorded in `preprocessing.steps`:

1. grayscale;
2. auto-contrast: the 1st..99th percentile gray levels are stretched to 0..255,
   skipped for flat images (range under 16) and images already spanning the
   range (`--no-contrast` disables);
3. polarity: when more than half of the pixels are below the Otsu threshold the
   image is inverted (light text on dark background);
4. text height: ink rows (pixels below the threshold) are grouped into runs and
   the median run height is the estimate; below 20 px the image is upscaled
   2x with Catmull-Rom, unless the result would exceed 40 Mpx
   (`--no-upscale` disables);
5. deskew: on a copy no larger than 800 px, binarised, rotations from -5 to
   +5 degrees in 0.5 steps are scored by the sum of squared row ink counts
   (projection profile); the best angle is applied with bilinear sampling
   only when it beats 0 degrees by 5 % (`--no-deskew` disables).

The engine sees the result; `bbox` values refer to it.

## Tests and verification

```sh
export CARGO_TARGET_DIR=/path/to/isolated-target-for-this-head
cargo test -p tpe-image-text
cargo clippy -p tpe-image-text --all-targets -- -D warnings
```

`tests/fixtures/` holds images rendered with ImageMagick and DejaVu Sans (64 KiB
in all: every accepted format, an animated GIF, a 3-degree skew, tiny text, low
contrast, inverted, a HEIC header, garbage bytes). Unit tests cover sniffing,
every preprocessing step (including recovering a known 3-degree rotation),
TSV parsing, language validation, manifest verification failing closed and
output-collision rules. Integration tests drive the binary with
`tests/fixtures/fake-tesseract.sh` (via `TPE_TESSERACT_BIN`) so the subprocess
path is exercised everywhere: outputs and JSON fields, `--force`, directory
walking, the timeout kill, quoted engine failures, rejected language codes, the
`exec-limited` helper, and that `ulimit -v`/`ulimit -t` inside the engine
process equal the requested limits for every limit the report lists as applied
(all four on Linux; on macOS the skipped `RLIMIT_AS` must appear as a warning). Tests needing a real engine print
`skipped: …` and pass when `tesseract` is not on `PATH` or the ocrs models are
not under `TPE_OCRS_MODELS_DIR` / `.models/ocrs`; with the models present the
ocrs test recognises `hello.png` and the skewed fixture end to end.

`tests/preservation.rs` exercises the actual `process_file` production path
with tiny synthetic PNGs and a synthetic in-process engine. It covers source
paths, hard links, symlinks, dangling outputs, no-force collisions, legitimate
forced replacement, successful pairs and handled second-output failures,
without OCR models. A publisher unit test injects a failure after the first
exclusive link and checks cleanup/restoration for new and forced outputs.

Not done: HEIC/HEIF decoding, docling OCR, per-character confidence for ocrs,
languages other than English for ocrs, orientation detection (upside-down or
90-degree pages), and mapping block boxes back through the deskew rotation.
