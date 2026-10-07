# Reference check (`tpe-refcheck`)

`crates/tpe-refcheck` validates the reference entries of an engine JSON output
against DOI and Crossref records. It follows the engine's resolution rule
(`src/resolve.rs`): a registry record counts only when it matches what is
printed. A DOI that resolves to another work is a mismatch, not a
verification, and a query hit is accepted only above documented thresholds.

Nothing here certifies a citation. It says whether the printed fields agree
with the record the DOI (or the best query candidate) points to.

## Input

Any of the engine's JSON outputs, read from one file:

| Shape | Source |
|-------|--------|
| object with `references` | `tpe extract --out` result (`ExtractionResult`), or a `tpe bibliography` record |
| JSON Lines of such objects | `tpe bibliography a.pdf b.pdf > refs.jsonl` |
| array of such objects, or of bare entries | hand-assembled lists |

Entries deserialise as `tpe::schema::ReferenceEntry`; `raw` is authoritative
and the parsed fields (`authors`, `title`, `year`, `venue`, `volume`,
`pages`, `doi`, `doi_link`) are what the engine parsed. The input file is
never modified.

## What is looked up

1. **Entry with a DOI.** The `doi.org` link annotation (`doi_link`, an exact
   string from the PDF) wins over the printed `doi`. The DOI is normalised
   (lower case, resolver prefix and trailing punctuation removed) and
   resolved at `https://doi.org/<doi>` with
   `Accept: application/vnd.citationstyles.csl+json`; when that returns
   anything but a CSL-JSON item, Crossref `GET /works/<doi>` is tried. `404`
   from both means the DOI is not registered.
2. **Entry without a DOI**, or whose DOI is unregistered or resolves to
   another work: Crossref `GET /works?query.bibliographic=<entry>&rows=5`
   with the raw entry text (label removed, whitespace collapsed, cut at 300
   characters). Every candidate is scored as below; the best *same-work*
   candidate is accepted unless a second candidate with a different DOI is
   within `query_margin` of it (then the entry stays `not-found` as
   ambiguous: a preprint and its published version are not decided by
   response order). `--no-query` disables this step.

## Scoring

Every string is folded first: Unicode NFKC, diacritics removed (NFKD plus
hand-mapped `ł ø ß æ œ đ þ ı`), lower-cased, punctuation turned into spaces.
Registries store ASCII forms of many names and extraction loses accents, so
neither side is trusted for diacritics.

| Field | Printed side | Record side | Rule |
|-------|--------------|-------------|------|
| title | `title`, else the raw text | `title` (+ `subtitle`) | `max(normalised Levenshtein, mean(word Jaccard, word containment))`; a title that is a word-prefix of the other with 3+ words scores 1 (dropped subtitle). Without a parsed title: share of the record title's words found in the raw entry. |
| author | first of `authors`, else the first 10 raw words | first author (`sequence: first`) | surname similarity: 1 when the folded family keys agree (particles dropped, umlaut transliteration `ue`/`u`, a hyphen part or last word in common), else Levenshtein on the keys. |
| year | `year`, else the first 1800–2099 number in the raw text | `issued`, `published-print`, `published-online`, `published` | agrees when any record year is within `year_tolerance`; an online-first year and a print year are both accepted. |
| container | `venue` | `container-title`, `container-title-short` / `short-container-title` | 1 for equal names or a token-wise abbreviation (`Proc Natl Acad Sci` / `Proceedings of the National Academy of Sciences`), else `max(word Jaccard, Levenshtein)`. |
| volume | `volume` | `volume` | alphanumerics equal after `vol.`/`v.` removal. |
| pages | `pages` | `page`, else `article-number` | first pages equal after dash unification and expansion of abbreviated ranges (`436-44` is `436-444`). |

A field is `unknown` (not compared) when either side lacks it; `unknown`
never counts as a field mismatch. It also does not count as positive identity
evidence: an empty or DOI-only record cannot verify an entry. Registry items
without a valid DOI, or with malformed identity fields, are response errors.

**Identity** (is the record the printed work?): the title agrees and the
first author agrees, or the title agrees at `title_strong` and the record
has no author to compare. With no record title, the author, year and one of
volume or pages must agree. `identity` in the report is the mean of the
title, author and year evidence.

Default thresholds (`Thresholds::default()`, written into every report):

| Threshold | Value | Meaning |
|-----------|-------|---------|
| `title_match` | 0.80 | titles agree (`--title-match` overrides) |
| `title_weak` | 0.60 | below this the titles are clearly different works |
| `title_strong` | 0.90 | identifies the work without author evidence |
| `author_match` | 0.85 | surnames agree |
| `year_tolerance` | 1 | years agree within ±1 |
| `container_match` | 0.75 | containers agree |
| `query_margin` | 0.05 | least lead of the best query candidate over a rival DOI |

## Verdicts

| Verdict | Meaning |
|---------|---------|
| `verified` | a valid registry DOI matches the printed DOI (when present), positive same-work identity is established, and every compared field agrees |
| `mismatch` | a record was obtained and `fields` lists what disagrees (`doi`, `title`, `author`, `year`, `container`, `volume`, `pages`); `comparison.same_work` says whether it is still the printed work (wrong pages) or another work (wrong DOI) |
| `not-found` | neither the DOI nor a query produced a record that identifies the entry (`detail` says why, including ambiguity) |
| `offline-or-error` | offline without a cached answer, or a request failed after its retries; nothing is known |

`suggested_doi` is set when a query identified the work: for an entry
without a DOI, with an unregistered DOI, or with a DOI that resolved to
another work (then `fields` also lists `doi`).

## Polite behaviour

* `User-Agent: tpe-refcheck/<version> (https://github.com/benpshore/pdftextract; mailto:<address>)`.
  The address comes from `--mailto` or the `TPE_MAILTO` environment variable
  (the same variable `tpe bibliography --resolve` uses) and is also sent as
  Crossref's `mailto` query parameter (the polite pool). It is never
  hard-coded; without it the tool warns once and uses the anonymous pool.
* Least spacing between two requests to the same host: `--interval-ms`
  (default 200 ms), applied per host (`doi.org`, `api.crossref.org`).
* `429` and `5xx` answers and transport failures are retried `--retries`
  times (default 4) with 1.5 s backoff doubled each time; a `Retry-After`
  header (seconds or HTTP date) replaces the computed wait. Waits are capped
  at 60 s.
* Whole-request timeout `--timeout-secs` (default 30).
* `--cache-dir DIR`: every final answer (`2xx`, `404`, `410`) is stored as
  one JSON file keyed by the SHA-256 of `doi:<doi>`, `crossref:doi:<doi>` or
  `query:<rows>:<text>`, written to a temporary sibling and renamed into
  place. Rate limits and server errors are never cached. `--offline` answers
  from the cache only (and requires `--cache-dir`).

## Command line

```sh
export TPE_MAILTO=you@example.org
tpe bibliography paper.pdf > paper.refs.jsonl
tpe-refcheck paper.refs.jsonl --cache-dir ~/.cache/tpe-refcheck --out report/
tpe-refcheck paper.refs.jsonl --cache-dir ~/.cache/tpe-refcheck --stdout markdown
tpe-refcheck paper.refs.jsonl --cache-dir ~/.cache/tpe-refcheck --strict   # exit 2 on any unverified entry
```

`--out DIR` writes `refcheck.json` (the report below) and `refcheck.md` (a
table: label, verdict, printed DOI, suggested DOI, title and author
similarity, differing fields, detail). `--stdout summary|json|markdown`
chooses what is printed. `--limit N` checks the first N entries.

Exit status: `0`; `2` when `--strict` is set and any entry is not `verified`,
including `not-found` and `offline-or-error`. Invalid registry responses and
insufficient positive identity evidence cannot pass strict mode. `1` for
unreadable input, an input without entries, or a usage error.

## Report

```json
{
  "tool": "tpe-refcheck", "version": "...", "generated_at": 1760000000,
  "input": "paper.refs.jsonl", "polite": true, "offline": false,
  "thresholds": { "title_match": 0.8, "...": "..." },
  "summary": { "entries": 42, "verified": 30, "mismatch": 5, "not_found": 6, "error": 1,
               "requests": 50, "cache_hits": 3, "retries": 0 },
  "entries": [
    { "index": 1, "label": "[1]", "verdict": "mismatch", "method": "doi",
      "printed": { "doi": "10.1038/nature14539", "title": "Deep learning", "first_author": "LeCun, Y.",
                   "year": 2013, "venue": "Nature", "volume": "521", "pages": "437–445" },
      "record": { "doi": "10.1038/nature14539", "title": "Deep learning", "first_author": "Yann LeCun",
                  "years": [2015], "container": "Nature", "volume": "521", "pages": "436-444",
                  "kind": "article-journal", "source": "doi.org" },
      "suggested_doi": null, "fields": ["year", "pages"],
      "comparison": { "title": 1.0, "title_outcome": "agree", "author": 1.0, "author_outcome": "agree",
                      "year": "differ", "container": 1.0, "container_outcome": "agree",
                      "volume": "agree", "pages": "differ", "identity": 0.67, "same_work": true,
                      "differing": ["year", "pages"] },
      "detail": "doi: 10.1038/nature14539 (doi.org) identity 0.67" }
  ]
}
```

## Tests

`cargo test -p tpe-refcheck`. No test touches the network: the client runs
against a scripted transport (backoff, `Retry-After`, cache, offline, the
`doi.org` to Crossref fallback, polite URLs), the scorer against tricky
cases (diacritics and umlaut transliteration, dropped subtitles, `et al.`,
OCR noise, preprint versus published year, abbreviated journals, expanded
page ranges, raw-text fallbacks), and the binary against an offline cache
seeded from hand-written fixtures in `crates/tpe-refcheck/tests/fixtures`
that follow the documented CSL-JSON and Crossref `/works` shapes.

## Limits

* Crossref coverage only: DataCite-registered DOIs (datasets, many
  preprints) resolve through `doi.org` CSL-JSON but have no Crossref
  fallback or query; PubMed and arXiv identifiers are not consulted.
* The query step sends the raw entry text to Crossref; the tool is meant for
  the user's own extraction output, not for text that must stay private.
* Scores are heuristics. A `verified` verdict means the compared fields
  agree with the registry record; it does not establish that the record is
  the work the author meant to cite when the printed entry is itself wrong
  in a way the registry repeats.
