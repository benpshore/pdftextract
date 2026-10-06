# Article identity, deduplication and renaming (`tpe-identify`)

`crates/tpe-identify` answers three questions about a pile of PDFs: which
files are the same article, which of them are versions of one work, and what
each file should be called. It is a library plus the `tpe-identify` command.
Scanning never modifies an input's contents and never touches the network.
An explicit apply can move an input's directory entry; undo can remove a
verified copy when its unchanged original still exists.

```sh
cargo run -p tpe-identify -- scan papers/ --json report.json
cargo run -p tpe-identify -- scan papers/ --rename-into renamed/            # dry run
cargo run -p tpe-identify -- scan papers/ --rename-into renamed/ --apply    # copy + manifest
cargo run -p tpe-identify -- undo renamed/tpe-identify-manifest.json --apply
```

`scan` accepts PDF files, directories (walked recursively for `*.pdf`) and
`tpe` result JSON files (`ExtractionResult`, as `tpe extract --out` writes
them). It prints a table, then the evidence, warnings and errors, and writes
the full report with `--json`. Exit status is 1 when any input failed to
load, a report cannot be published, or an apply operation fails; the report
still covers loaded inputs. `--json` requires a new destination. Existing
paths (including hard links, symlinks and dangling symlinks) are refused,
without choosing another filename. The report is staged and published
before any requested apply, so an invalid report destination cannot move
sources first.

## Where the text comes from

For each PDF the bytes are read once through the engine's `acquire`
snapshot (so a file that changes while it is read is reported, not hashed)
and the text is taken from the first of these that exists:

| source | flag | `text_source` in the report |
| --- | --- | --- |
| the result JSON given on the command line | (a `.json` path) | `json:<path>` |
| `<hash>.json` or `<hash>.pdf.json` in a results directory | `--results DIR` | `results:<path>` |
| the latest run for the hash in a ledger | `--db ledger.sqlite` | `ledger:<db>#<run>` |
| a fresh extraction with the pure-Rust `lopdf` backend | default; `--no-extract` disables | `extract:lopdf` |

Without any of these the input still has its byte identity
(`none:extraction disabled`) and can only be matched as an exact copy.
A result JSON whose recorded source path no longer exists is grouped but
not renamed.

Fresh extraction consumes those same acquired bytes; it never reopens the
pathname after hashing. The ledger is opened read-only, without schema/WAL
initialization or migration. Compatible v4 results remain readable as v4.

## Identities

Every input gets three identities (`Identity` in the report's `inputs`):

1. **Exact**: lower-case hex SHA-256 of the bytes (the engine's
   `ContentHash`).
2. **Textual**: the page texts are normalised (NFKC, lower-case,
   line-break hyphenation joined, every non-alphanumeric character a
   separator, only tokens with at least one letter and two characters kept,
   so page numbers, line numbers and equation digits that differ between
   versions do not count). From the normalised text:
   `text_sha256` (same text in different bytes), a **MinHash** signature
   over word **3-gram shingles** hashed with FNV-1a, 128 universal-hash
   permutations modulo 2^61-1 with a fixed seed (reproducible across
   platforms; the standard error of the Jaccard estimate is about 0.03
   near the thresholds), and a 64-bit **SimHash** over unigrams and bigrams
   (`simhash`, compact secondary evidence; Hamming distance is at most a
   few bits for near-identical texts and about 32 for unrelated ones).
   Texts shorter than 20 tokens give no text evidence.
3. **Bibliographic** (`key`): the DOI and `arXiv` id from the engine's
   metadata, normalised with `tpe-common` (resolver prefixes and versions
   stripped); when the metadata has none, the first one printed on page 1
   is used and marked `text` (weaker, since it could be a cited DOI). The
   title is kept only when it passes a plausibility check (two words with
   letters, ten letters, more letters than symbols, not `Microsoft Word -
   x.docx`, `untitled`, `draft.tex`, `PowerPoint Presentation`, a repeated
   character and so on) and is normalised to lower-case alphanumeric tokens.
   The first author's surname is parsed from `Given Surname`, `Surname,
   Given`, `G. Surname Jr.` and `SURNAME Given`, ASCII-folded, and rejected
   when it is an e-mail address, `et al.`, an institution word or shorter
   than two letters. Years outside 1800–2100 are dropped. Every rejection is
   recorded in the input's `warnings`, so garbled metadata degrades to
   weaker evidence instead of wrong links.

Each input is also classified as `preprint` (an `arXiv` id; a DOI of
arXiv, bioRxiv/medRxiv, SSRN, OSF, Research Square, ChemRxiv, Preprints.org,
Authorea, PsyArXiv, EcoEvoRxiv or TechRxiv; or page-1 phrases such as
`arXiv:`, `bioRxiv preprint`, `not peer reviewed`, `working paper`),
`published` (any other DOI) or `unknown`.

## Evidence and grouping

Every pair of inputs is compared (quadratic; fine for a library of
thousands, not for millions). Each relation found becomes one `Evidence`
record with a score in 0–1 and a human-readable `detail`:

| relation | when | score |
| --- | --- | --- |
| `exact_bytes` | same SHA-256 (nothing else is checked) | 1.00 |
| `same_text` | same normalised text, different bytes (re-saved, stamped, different producer) | 0.99 |
| `near_duplicate_text` | estimated Jaccard ≥ `--near-threshold` (default **0.85**) | the estimate (capped at 0.99) |
| `shared_text` | estimated Jaccard in [`--text-threshold` (default **0.50**), near) | the estimate |
| `same_doi` | equal normalised DOIs | 0.95, or 0.80 when either came from page text |
| `same_arxiv` | equal `arXiv` ids | 0.95 / 0.80 as above |
| `title_author_year` | same normalised title, same surname, years within one (or missing) | 0.85, +0.05 when text Jaccard ≥ 0.20, −0.10 when both undated |
| `title_year` | same title, years compatible, a surname missing on one side | 0.70, +0.10 / −0.10 as above |

Two inputs with the same title but different surnames get no title
evidence; a shared-text link can still join them. Different DOIs are not a
negative signal, because a preprint and its published version always have
different ones: that pair is linked by `title_author_year` (years within
one) and usually corroborated by `shared_text`.

With 3-word shingles, changing every 50th word of a 600-word text leaves an
estimate above 0.85 (near-duplicate); changing every 8th word gives roughly
0.2–0.6 (a revision: linked on its own above 0.50, otherwise only as
corroboration); unrelated texts in one field stay under 0.10.

Inputs are joined into **works** by union-find over the evidence. Each
group reports:

- `kind`: `single`, `exact_duplicates` (all bytes equal), `near_duplicates`
  (every member has an exact/same-text/near-duplicate link) or `versions`
  (at least one member differs in text, e.g. preprint and published).
- `canonical`: the member to keep — `published` over `preprint` over
  `unknown`, then the most text, then the lexically first path.
- member `role`: `canonical`, `duplicate` (same or near-same text as the
  canonical member) or `version`; and `link_score`, the member's best
  evidence to the rest of the group.
- `confidence`: the weakest member link (1.0 for singletons), i.e. the
  confidence that every member belongs.

## Renaming

Every input gets `<FirstAuthor>_<Year>_<ShortTitle>.pdf`: the surname
ASCII-folded (`Müller` → `Muller`, `Søren` → `Soren`, `ß` → `ss`) and
capitalised, `Unknown` when none; the year or `nd`; up to six title words,
stopwords dropped unless nothing would remain, capitalised and joined by
`-`, or the first twelve hex digits of the hash when there is no plausible
title. The base name is capped at `--max-name-len` (default 96) by dropping
title words, then by truncation.

Collisions are resolved deterministically (groups in order, canonical
first, then by path): members of the same work get `_dup2`, `_dup3`, …
after the canonical member; a different work landing on a taken name gets
`_2`, `_3`, … Names already present in the output directory count as taken
(case-insensitively), so nothing is ever overwritten.

`scan` is a **dry run** unless `--apply` is given together with
`--rename-into DIR`. Applying:

- stages each complete copy from a verified immutable read, then publishes
  with an exclusive hard link and verifies its SHA-256; an existing target
  is skipped with `target exists`;
- renames in place only regular inputs already inside `DIR`, using an
  atomic no-replace rename on Linux/macOS; unsupported systems fail closed;
- skips an input whose bytes changed since the scan (`source changed since
  the scan`);
- never writes outside `DIR`, never changes contents, never deletes;
- saves and syncs `DIR/tpe-identify-manifest.json` (`-2`, `-3`, … when a
  directory entry, including a dangling symlink, exists) **before** any
  operation. Manifest v2 records absolute `from`/`to`, `op`, `sha256`,
  `skipped` and `pending` per entry. Each outcome is checkpointed using
  a complete staged sibling and atomic replacement of the owned record.

The manifest is always a complete old or new JSON record. Failure after
an operation but before its durable checkpoint leaves `pending: true` and
the error names the recovery manifest. Undo deliberately keeps both paths
for these uncertain entries: inspect them against the recorded hash before
manual recovery. It never guesses ownership and deletes a same-content
file. Completed entries remain undoable, and legacy v1 manifests remain
readable. This is recoverable evidence, not an all-or-nothing crash
transaction across every document and the manifest. Cooperating writers
are supported; hostile concurrent replacement between file-identity checks
and filesystem operations is outside this boundary. Hard links and
directory sync are required for publication, with no partial-write fallback.

`tpe-identify undo MANIFEST` prints what reversing would do;
`--apply` does it, newest entry first: a `rename` is reversed when the
renamed file still has the recorded hash and the original name is free; a
`copy` is removed only when the copy still has the recorded hash **and** the
original still exists with the same hash. Anything else is reported and
kept. Undo is the only operation that deletes, and only files this tool
created.

## Report shape

`--json` writes `{tool, version, params, inputs[], evidence[], groups[],
rename{into, max_name_len, entries[]}, errors[]}`. `inputs[i].index` is the
position used by `evidence[].a/b`, `groups[].members[].index` and
`rename.entries[].index`. MinHash signatures stay in memory and are not
serialised.

## Limits

- Pairwise comparison is O(n²); there is no LSH banding yet.
- Text evidence needs text: scanned PDFs without OCR match only by bytes
  or metadata.
- The bibliographic key is as good as the engine's metadata; a page-1 DOI
  scan can pick up a cited DOI (hence the lower score for `text`-sourced
  identifiers).
- Extraction is the `lopdf` backend only; stored results from any backend
  are accepted through `--results` or `--db`.

## Tests

`cargo test -p tpe-identify` runs unit tests for every module (normalisation,
MinHash/SimHash behaviour on synthetic texts, title/author sanity checks,
identifier scanning, pairwise evidence, grouping, name generation and
collision suffixes) and `tests/scan.rs`, which builds six PDFs with `lopdf`
(an original, an exact copy, a re-saved copy with different bytes, a
two-word near-duplicate, an unrelated paper and a different work whose
canonical name collides), extracts them with the `lopdf` backend, and
checks grouping, names, `--apply` by copy and by in-directory rename, the
no-overwrite rule, hash verification, both undo paths and the command line.
