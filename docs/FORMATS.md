# Non-PDF formats: `tpe-formats`

`crates/tpe-formats` extracts text and structure from inputs the PDF engine
does not read. It is a separate library and binary; `tpe` and the routing in
`src/` are unchanged. Inputs are never modified: every result is a new file in
the output directory. Nothing in this crate touches the network.

## Command line

```sh
tpe-formats <inputs...> --out DIR [--recursive] [--force] [--json]
```

- Each input yields `DIR/<stem>.json` (always, when the input was recognised)
  and `DIR/<stem>.txt` (when the status is `complete` or `partial`). Two
  inputs with the same stem in one run get `<stem>-2`, `<stem>-3`, ...
- `--recursive` walks directories, taking files whose extension is listed
  below and treating `.pages`/`.numbers` directory bundles as single inputs.
  Without it, a directory argument is a usage error.
- `--force` overwrites existing outputs; otherwise they are left alone and the
  input is reported as `skipped`.
- `--json` prints one JSON array for the run (input, status, format, output
  paths, warnings) instead of one line per input.

Exit codes: `0` every input produced a complete or partial result (skipped
inputs count as success); `1` at least one input failed (unreadable or
malformed; the others are still written); `2` usage error (missing input,
directory without `--recursive`, unwritable `--out`); `3` at least one input
was recognised but unsupported here and none failed.

## Output shape

`<stem>.json` mirrors the engine's `ExtractionResult` where the two overlap:

| field | meaning |
| --- | --- |
| `schema_version` | `1` |
| `document.hash`, `document.size`, `document.sources` | SHA-256 of the input bytes, byte count, path as given |
| `backend.name`, `backend.version`, `backend.engine` | `tpe-formats`, crate version, external engine label (audio only) |
| `format` | `docx`, `pptx`, `xlsx`, `csv`, `tsv`, `html`, `markdown`, `text`, `pages`, `numbers`, `audio` |
| `status` | `complete`, `partial` (a `partial:` warning names the gap), `failed`, `unsupported` |
| `title` | stated title (core properties, `<title>`, front matter) or the first `Title`/`h1` heading; never guessed |
| `metadata` | string properties as found: core/app properties, `<meta>` tags, front matter keys, delimiter/encoding, sheet row counts, engine details |
| `sections[]` | `kind` (`body`, `slide`, `sheet`, `header`, `footer`, `front_matter`, `table`, `transcript`, `package`), `index`, `title`, `blocks[]` |
| `blocks[]` | `kind` (`paragraph`, `heading`, `list_item`, `table`, `code`, `quote`), `level` (heading level or list depth), `text`, `rows` (tables only, row-major cells) |
| `notes[]` | `kind` (`speaker_notes`, `footnote`, `endnote`, `comment`), `anchor` (`slide 3`, footnote id), `text` |
| `warnings[]` | diagnostics; `unsupported:` and `partial:` prefixes carry the status |
| `text` | the `.txt` content: section titles and blocks joined by blank lines, tables tab-separated, notes appended as `[footnote 1]` / `[speaker notes slide 2]` paragraphs |

## What each decoder does

**docx** (`word/document.xml`): paragraphs with run text, tabs, line breaks,
symbol runs and footnote/endnote markers (`[id]`); tracked deletions skipped,
insertions kept; hyperlinks, content controls and smart tags transparent;
tables as rows (cell paragraphs joined by newlines, `gridSpan` padded with
empty cells); headings from the `Title`/`Heading N` styles (resolved through
`styles.xml`, so localised style ids still work) or `outlineLvl`; numbered
paragraphs as list items with their level; headers and footers as their own
sections; footnotes, endnotes and comments as notes; `docProps/core.xml` and
`app.xml` as metadata. Not decoded: field results that are not plain text,
embedded objects, images (the image-text track owns those).

**pptx**: slides in `p:sldIdLst` order (file order with a warning when the
list is missing), title placeholders as the slide title, other text frames as
paragraphs (indent level kept), `a:tbl` tables as rows, grouped shapes
descended, slide-number/date/footer placeholders dropped, speaker notes from
the notes slide's body placeholder. Core properties as metadata.

**xlsx**: sheets in workbook order (hidden sheets included with a warning),
shared strings (rich runs joined, phonetic runs dropped), inline strings,
booleans, error cells, and formulas through their cached values (a formula
with no cached value gives an empty cell and a `partial:` warning). Numbers
are rendered as Excel displays them for `General`, fixed decimals, thousands
grouping, percentages, scientific notation and date/time formats (1900 and
1904 systems, including the 1900 leap-year quirk). Fractions, conditional
sections and other custom codes keep the raw stored value and are listed once
in a warning. Rows sit at their sheet row numbers (gaps are empty rows).

**csv/tsv**: BOM and encoding detection (UTF-8, UTF-8 with BOM, UTF-16 LE/BE;
invalid UTF-8 falls back to Windows-1252 with a warning); delimiter sniffing
over the first 50 lines among `,`, tab, `;`, `|` (TSV always uses tab);
RFC 4180 quoting with doubled quotes and embedded line breaks; ragged rows
are reported, not padded.

**html**: a tolerant tokenizer (unbalanced or unknown tags never fail).
`script`, `style`, `template`, `noscript` and `svg` content is dropped,
comments and doctype skipped, entities decoded (numeric and the Latin-1 plus
common typographic named set), whitespace collapsed except inside `pre`.
Headings keep their level, `li` items their nesting depth, tables become rows
(nested tables flatten into the cell), `<title>` becomes the title and
`description`/`author`/`keywords`/`og:*`/`citation_*` meta tags plus
`<html lang>` become metadata.

**md**: line endings normalised, trailing whitespace trimmed. YAML (`---`) or
TOML (`+++`) front matter is split off into a `front_matter` section and its
top-level `key: value` pairs into metadata. ATX and Setext headings, `-`/`*`/
`+`/`1.` lists with depth, fenced code, block quotes, pipe tables and
reference definitions are recognised; inline emphasis, links, images, code
spans, autolinks and `<br>` are reduced to their text.

**txt**: encoding detection as for CSV, line endings normalised, paragraphs
split on blank lines.

## Apple Pages and Numbers: what is and is not done

A `.pages`/`.numbers` document is a zip (or a directory bundle) whose content
lives in `Index/*.iwa`. Each IWA file is a sequence of Snappy-compressed
chunks of protobuf messages whose schema Apple does not publish; decoding it
faithfully needs the per-version message definitions and, for Numbers, the
packed tile layout of cell storage. This crate does **not** decode IWA, and
it does not guess text from the bytes. What it does:

- opens the package (zip or bundle), counts entries and `.iwa` files, records
  `Metadata/DocumentIdentifier`, the template and build versions from
  `Metadata/BuildVersionHistory.plist`, and whether `preview.pdf` exists;
- when the author saved a preview (`preview.pdf` begins with `%PDF`), copies
  it to `DIR/<stem>.preview.pdf` so the PDF engine can read it. The preview is
  whatever Pages/Numbers rendered at save time, which is not guaranteed to be
  the whole document, and the result says so;
- returns `status: unsupported` with the exact reason (`Pages text is stored
  in Index/*.iwa as Snappy-framed protobuf messages without a published
  schema; tpe-formats does not decode IWA ...`), or `... has no
  Index/Document.iwa` for the pre-2013 `index.xml.gz` format. Exit code 3.

Keynote (`.key`) is not handled.

## Audio: local engines only

`wav`, `mp3` and `m4a` are transcribed only through an engine found on this
machine at run time. No network API is ever called and no model is ever
downloaded.

| engine | how it is found | model |
| --- | --- | --- |
| whisper.cpp | `TPE_WHISPER_BIN`, else `whisper-cli`, `whisper-cpp` or `whisper.cpp` on `PATH` | `TPE_WHISPER_MODEL=/path/ggml-*.bin`, else the first `ggml-*.bin` in `~/.cache/whisper.cpp[/models]`, `/usr/share/whisper.cpp/models`, `/usr/local/share/whisper.cpp/models` or `/opt/homebrew/share/whisper-cpp/models` |
| openai-whisper CLI | `whisper` on `PATH` | `TPE_WHISPER_MODEL_DIR` must already contain `<name>.pt` (`TPE_WHISPER_MODEL_NAME`, default `base`) |

`mp3`/`m4a` are converted to 16 kHz mono WAV with `ffmpeg` in a temporary
directory first; without `ffmpeg` they are unsupported. When no engine or no
local model is found the result is `status: unsupported` with a warning that
starts `unsupported here: no local speech engine.` and says what to install
or which variable to set; exit code 3. WAV headers still yield `sample_rate`,
`channels` and `duration_seconds` metadata. A transcript is one `transcript`
section of paragraphs; `backend.engine` records the binary and model used.

Raster images (OCR) are not part of this crate.

## Verification

```sh
export CARGO_TARGET_DIR=/home/user/pdftextract/target
cargo clippy -p tpe-formats --all-targets -- -D warnings
cargo test -p tpe-formats
```

The tests build `docx`/`pptx`/`xlsx` fixtures by zipping hand-written XML,
use inline CSV/HTML/Markdown/text, a synthetic Pages package with a stub
`preview.pdf` and a Numbers directory bundle, and exercise the audio path with
an empty `PATH` (engine absent) plus a fake `whisper-cli` with and without a
model. If a real engine is installed, the test prints that it skipped live
transcription; it never runs it. The CLI test checks every exit code.
