# PDF operations (`tpe-pdfops`)

`crates/tpe-pdfops` is a library and a command-line tool for PDF operations
that always write **new** files. Inputs are opened read-only and are never
modified or deleted; an output path that names an input is refused, and an
existing output is refused unless `--force` is given. Every result is
serialised, parsed again with `lopdf` (the page count must survive) and only
then written.

```sh
cargo build -p tpe-pdfops                    # structural operations
cargo build -p tpe-pdfops --features pdfium  # plus the raster `clean` path
tpe-pdfops <op> ... --out <file|dir> [--force]
```

Exit status: `0` success, `1` error, `2` usage, `3` explicit unsupported
result (the message says why and what would enable it).

## Operations

| Command | What it does | Mechanism |
| --- | --- | --- |
| `concat A.pdf B.pdf ... --out merged.pdf [--outlines]` | Merge in the given order, page order preserved. | Objects of each input are renumbered and copied; inherited page attributes (`Resources`, `MediaBox`, `CropBox`, `Rotate`) are made explicit; a new page tree and catalog are written. `--outlines` adds one top-level outline entry per input (its file stem, `/Fit` to the input's first page). Inputs' own outlines are dropped; shared resources are copied per input, not deduplicated. |
| `paginate in.pdf --out dir/ (--pages 1-3,7 [--pages 9-] ... \| --every N) [--stamp] [--stem name]` | Split into `dir/<stem>-001.pdf`, ... | Each `--pages` selection is one output, in the order listed (open ranges like `9-` run to the end); `--every N` makes consecutive groups. `--stamp` appends a content stream per page showing `Page n of N` in source numbering (Helvetica, bottom centre, inside its own `q`/`Q` and `BT`/`ET`); the page's resource dictionary is copied before the font is added so shared resources stay untouched. |
| `reorient in.pdf --out out.pdf (--rotate 90\|180\|270 \| --auto) [--pages 2-4]` | Rotate pages via `/Rotate`. | `--rotate` adds clockwise degrees to the effective (inherited) rotation. `--auto` reads the content stream: every text-showing operator (`Tj`, `TJ`, `'`, `"`) is weighted by the bytes it shows into a quadrant from the direction of the text-space x axis after `Tm` and the CTM (`cm`, `q`/`Q`). One quadrant must hold at least 60 % of the weight, else the page is left unchanged and reported (`no text on the page`, `no dominant text direction`). |
| `letter in.pdf --out out.pdf [--pages ...]` | Normalise to US Letter (612 x 792 pt). | The visible box (`CropBox` clipped to `MediaBox`, else `MediaBox`) is scaled uniformly to fit and centred. The original content streams are kept and wrapped by a prefix stream `q sx 0 0 sy tx ty cm` and a suffix stream `Q`; `MediaBox` and `CropBox` become `[0 0 612 792]` (or `[0 0 792 612]` when `/Rotate` is 90 or 270 so the displayed page is portrait Letter); `BleedBox`, `TrimBox`, `ArtBox` are removed; indirect annotation `/Rect`s are moved with the content. |
| `unlock in.pdf --out out.pdf --password PW` | Remove the open password. | `lopdf` 0.45 decrypts at load time (RC4 40/128, AES-128, AES-256). The given user or owner password is tried exactly once; nothing is guessed. A wrong password or a non-encrypted input is an error. The output is written without `/Encrypt`. |
| `linearize in.pdf --out out.pdf` | Fast web view. | **Unsupported, by design** (exit 3). A linearized file needs a leading `/Linearized` parameter dictionary with exact byte offsets, a first-page cross-reference section and a hint stream (PDF 32000-1 Annex F); `lopdf` writes classic or object-stream files only and exposes no offset table to build those from. The tool reports the reason (and whether the input already carries a `/Linearized` dictionary) rather than write a file that merely claims to be linearized. Use `qpdf --linearize`. |
| `clean in.pdf --out out.pdf [--water-stain] [--dewarp] [--deskew] [--dpi 200] [--encoding jpeg\|flate] [--quality 85] [--password PW]` | Raster clean-ups. | Needs the `pdfium` feature and `PDFIUM_DYNAMIC_LIB_PATH` (see below). Pages are rendered as displayed (their `/Rotate` applied) to 8-bit grey at `--dpi`, processed in the order water-stain, deskew, dewarp, and written as one full-page image XObject per page (`/DCTDecode` JPEG or `/FlateDecode`) with a `MediaBox` of the rendered size in points. Text becomes pixels: run text extraction on the original, not on the cleaned file. |
| `inspect in.pdf` | Print version, page count, encryption and linearization flags, and per page `MediaBox`, `Rotate` and `CropBox`. | The same re-parse the tests use. |

### Raster algorithms (`tpe_pdfops::raster`)

Pure functions over `image::GrayImage`, testable without PDFium:

- **Water-stain cleanup** (`normalise_background`): the local background is the
  90th percentile of each tile (tile = dpi/6 px, at least 16), interpolated
  bilinearly and divided out (`255 * pixel / background`). Tiles darker than
  64 are treated as content (photographs, fills) and not brightened.
- **Deskew** (`estimate_skew`, `deskew`): Otsu threshold, then the clockwise
  angle within +/-5 degrees that maximises the sharpness (sum of squared row
  counts) of the ink projection profile, searched at 0.5, 0.1 and 0.02 degree
  steps; ties go to the angle nearest zero. The page is rotated with
  bilinear sampling and white fill.
- **Dewarp** (`dewarp`): simple baseline straightening. The page is cut into
  12 vertical strips; text-line centres are the peaks of each strip's ink
  profile; the strip with the most lines is the reference, every other
  strip's median vertical offset of matching lines is its shift, and columns
  are moved vertically by the linearly interpolated shift. It corrects a
  bowed page whose lines share one curve; it does not model perspective or
  per-line curl.

### PDFium

The raster path uses `pdfium-render` 0.8 (same features as the root crate:
`pdfium_latest`, `thread_safe`; no `unsafe` in this crate) and loads
`libpdfium` only from the absolute directory or file in
`PDFIUM_DYNAMIC_LIB_PATH`, binding and dropping the library per call.
Without the feature, or without the variable, `clean` is an explicit
unsupported result naming what is missing. Provision the pinned library with
`sh native/fetch.sh --pdfium-only` and `export
PDFIUM_DYNAMIC_LIB_PATH="$PWD/.pdfium/lib"` (see `docs/NATIVE.md`).

## Library

```rust
use tpe_pdfops::{Output, PageSelection};
use tpe_pdfops::concat::{concatenate, ConcatOptions};
use tpe_pdfops::output::save_document;

let mut doc = concatenate(&[a, b], &ConcatOptions { outlines: true })?;
save_document(&mut doc, &Output::new("merged.pdf"))?;   // Err(OutputExists) if present
```

Every operation returns a `lopdf::Document` (or, for `paginate`, writes its
files) and `output::save_document` performs the self-check and the
new-file write. Errors are `PdfOpsError`: `Io`, `Pdf`, `OutputExists`,
`OutputIsInput`, `Invalid` (bad request, wrong password), `Unsupported`.

## Tests

```sh
cargo test -p tpe-pdfops
PDFIUM_DYNAMIC_LIB_PATH=/abs/path/.pdfium/lib cargo test -p tpe-pdfops --features pdfium
```

Fixtures are built with `lopdf` in the tests (multi-size pages, rotated
pages, pages with rotated text matrices, RC4 and AES encrypted documents);
every output is re-parsed and its page count, boxes, rotation and content
checked, and every input is compared byte for byte afterwards. The raster
unit tests use synthetic images (ruled lines rotated by a known angle, a
gradient stain, bowed lines). The PDFium test runs only when
`PDFIUM_DYNAMIC_LIB_PATH` is set and prints `skipped` otherwise; without the
feature the test asserts the unsupported result instead.

## Limits

- No linearization (above). No deduplication of shared resources in `concat`.
- `paginate --stamp` positions text with an approximate Helvetica width
  (0.55 em per character) and is drawn in unrotated page space.
- `reorient --auto` only reads text matrices; pages whose text is drawn
  through Form XObjects or as images are reported undecided and left alone.
- `letter` keeps `/Rotate`; direct (non-indirect) annotation dictionaries
  are not moved.
- `clean` output is image-only; its file size depends on `--dpi` and
  `--encoding`.
