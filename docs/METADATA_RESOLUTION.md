# Metadata resolution: identifiers, registries, verification

How `tpe` turns what is printed in a PDF (a DOI, a PMID, a title) into a
registry record, and why a record is accepted or refused. The rule from
`src/resolve.rs` is unchanged: **a resolution is verified against what is
printed, never fabricated**. A registry answer that does not agree with the
page is logged as a `mismatch` and the paper or reference keeps only its
printed fields.

Code: `crates/tpe-biblio` (clients, pure parsers, cache, retry) and
`src/resolve.rs` / `src/metadata.rs` (verification and wiring). Nothing in
this document is a claim about accuracy on arbitrary PDFs; see the limits.

## Privacy posture: on-device, opt-in network

- Extraction is on-device. No registry is contacted unless a caller
  constructs a `Resolver` (the CLI only does so for `tpe bibliography
  --resolve`). The library API resolves nothing by itself.
- Requests carry only the identifier or the citation text being resolved,
  the `tpe/<version>` User-Agent and, when the person running the tool
  supplies one, a contact address. The address is read from the
  `TPE_MAILTO` environment variable (or `--mailto`); **no address is
  hard-coded anywhere** and no request is sent "on behalf of" a fixed
  account. Without it, requests go to the registries' anonymous pools.
- No telemetry, no identifiers of the person or machine, no API keys in
  code. The NCBI key, when wanted, is supplied by the caller through
  `Client::with_key`.
- Responses can be cached on disk, but only under a directory the caller
  names (`TPE_BIBLIO_CACHE_DIR` or `Resolver::with_cache_dir`). Unset means
  no cache. The cache never leaves that directory, never deletes files and
  stores only the registry's JSON/XML body, the status and the time.
- Tests never touch the network: every parser is exercised on hand-written
  fixtures reproducing the APIs' documented shapes, and the resolver tests
  run an offline client against a pre-filled cache.

## Identifier extraction (`tpe_biblio::identifiers`, pure)

| Kind | Accepted forms | Normalised to | Refused |
| --- | --- | --- | --- |
| DOI | Crossref's recommended pattern `10.<4-9 digits>/<suffix>` with suffix alphabet `-._;()/:A-Za-z0-9`; `doi:` and `doi.org` / `dx.doi.org` prefixes, percent-encoded or not; Wiley SICI suffixes with `<>` under `10.1002` | lower case, no prefix, trailing `.,;:` and unbalanced `)]>` removed | anything else |
| arXiv | `arXiv:YYMM.NNNNN[vN]`, `arxiv.org/abs|pdf/...`, old scheme `archive[.SC]/YYMMNNN`; bare new-style ids only with a plausible month (2007-04 on) and five digits; bare old-style ids only for a known archive | version removed | decimal numbers, prices, unknown archives without a label |
| PMID | `PMID: n`, `pubmed.ncbi.nlm.nih.gov/n`, `ncbi.nlm.nih.gov/pubmed/n` | digits, no leading zeros | **bare numbers are never PMIDs** |
| PMCID | `PMC n`, `PMCn`, PMC article URLs | `PMC<digits>` | |
| ISBN | `ISBN`, `ISBN-10`, `ISBN-13` labels; bare hyphenated 10/13-digit groups | digits only; `isbn13_of` converts | any value whose check digit fails |
| ISSN | `ISSN`, `e-ISSN`, `print ISSN` labels; bare `NNNN-NNNC` | `NNNN-NNNC` | any value whose check digit fails |

ISBN and ISSN hits carry `labelled`; `src/metadata.rs` keeps only labelled
values for the paper because a page range such as `1741-1750` can pass the
ISSN check digit by chance (1 in 11).

`metadata::extract_identifiers(meta, pages)` combines the metadata DOI and
arXiv id with the labelled identifiers printed on page 1 and in the `/Info`
strings (the `Subject` entry often carries the citation). A DOI missing
from the metadata is taken from `/Info` only, never from page text, because
page 1 cites other papers too. Every field records its provenance.

## Registries and clients (`crates/tpe-biblio`)

Each source has a pure `parse_*` function tested on a recorded fixture and
a thin fetch wrapper. Shapes follow the services' public documentation;
the crate docs say the fixtures reproduce the well-known shapes rather than
a verified capture.

| Source | Endpoint | Used for |
| --- | --- | --- |
| Crossref | `GET /works/{doi}` | the paper's and references' DOIs; `CrossrefWork` carries publisher, member, licences (URL, start, embargo, version), funders (name, Funder Registry DOI, awards), ISSN (print/electronic), ISBN, container title, type, volume/issue/page, dates, counts |
| Crossref | `GET /works?query.bibliographic=` | a whole citation string or the paper title when no identifier verifies |
| doi.org | `GET /{doi}` with `Accept: application/vnd.citationstyles.csl+json` | CSL JSON for any agency (fewer fields) |
| doi.org | `GET /ra/{doi}` | which registration agency owns a DOI |
| DataCite | `GET /dois/{doi}` | datasets, software, preprints (Zenodo, figshare, Dryad, arXiv DOIs): creators with ORCID, publisher, rights, funding, related identifiers |
| PubMed | `esearch` / `esummary` / `efetch` (`db=pubmed`) | a printed PMID; `efetch` XML is read by a minimal scanner for the documented elements only |
| PMC ID converter | `idconv ... ?ids=&format=json` | PMID <-> PMCID <-> DOI |
| Europe PMC | `search?query=EXT_ID:… AND SRC:MED` (existing) | exact PMID/PMCID/DOI lookups for references |

`doi_metadata::resolve_doi` asks Crossref, then DataCite, then doi.org
(DataCite first for prefixes known to be its, such as `10.5281`). The
first record carrying the requested DOI wins. If every source says "not
found" the answer is `NotFound`; if any source could not be asked and
none found the DOI the answer is `Unresolved(<reason>)`. **Offline is
never reported as "not found."**

### Transport: politeness, retries, cache

- `Client::polite(product, version, mailto)` builds `tpe/<version>
  (mailto:<address>)` and sends the same address as the `mailto` query
  parameter, which Crossref and OpenAlex route to their polite pools. NCBI
  requests carry `tool=tpe` and, when known, `email`.
- Per-host spacing (Crossref 200 ms, E-utilities 350 ms, Semantic Scholar
  1.1 s) as before.
- `RetryPolicy`: four retries with 1.5 s, 3 s, 6 s, 12 s backoff after a
  429, a 5xx, or a transport failure; a `Retry-After` header (seconds or
  HTTP date) is honoured as asked, up to 60 s; a longer request stops the
  retries. 4xx answers, bad JSON and offline mode are never retried.
- `DiskCache`: one JSON file per (source, key) under the caller's
  directory, written atomically, with a 7-day TTL; `200` bodies and `404`
  answers are cached, errors are not.
- Every lookup outcome is a `Lookup<T>`: `Found`, `NotFound` or
  `Unresolved(reason)` with reasons such as `offline`, `network: timeout`,
  `rate limited`, `HTTP status 503`.

## Verification (`src/resolve.rs`)

References keep the existing rules (see `docs/BIBLIOGRAPHY.md`): the
record must echo the requested DOI; its first-author family name, its year
(within one) and most of its title words must appear in the printed entry;
query candidates within 0.05 of each other are `ambiguous`. Reference DOI
lookups now go through the cache and the Retry-After-aware retry.

The paper itself (`Resolver::resolve_paper_report(meta, pages)`):

All three paths use the same compatibility rule: a supplied title must agree
at 0.85 or better; first-author family names must agree when both sides have
one; known years must differ by at most one; labelled printed ISSNs must
overlap record ISSNs when both are available. Missing optional fields add
no contradiction. An exact identifier can resolve without a metadata title.

1. **DOI from the metadata** -> `resolve_doi`. The record must carry that DOI
   and pass the common rule. The accepted record brings `PublisherMetadata`.
2. **PMID printed on page 1 or in `/Info`** -> PubMed `esummary`. The summary
   must carry that PMID and pass the common rule, including its ISSNs. When
   its DOI lookup returns metadata, that metadata must also pass the rule
   before it can be attached. Method `pmid`, source `pubmed`; PMCID and DOI
   are retained when supplied.
3. **Title query** (12+ characters) -> `query.bibliographic`. Full Crossref
   works retain ISSNs through verification. Only compatible candidates enter
   ranking; distinct DOIs within 0.05 are ambiguous. The paper query uses the
   same retrying, opt-in cache as identifier lookups, under
   `crossref-paper-query` with a `rows:title` key.

A contradicted DOI or PMID remains rejected for the rest of the resolution
pass, even if a later response omits the conflicting fields or spells the
DOI differently. A genuinely different compatible identity can still resolve.
Contradictory duplicates in one query are excluded independent of response
order. Rejections are local to the current paper and are never persisted as
registry absence.

`PaperResolution.status` is `resolved`, `mismatch`, `ambiguous`, `not_found`,
`unresolved: <reason>` or `unresolved: no identifier or title`; `attempts`
lists every lookup. `resolve_paper(meta)` keeps its old signature and returns
the accepted record only.

## Configuration

| Setting | Source | Default |
| --- | --- | --- |
| contact address | `TPE_MAILTO` or `--mailto` | none (anonymous pools) |
| response cache | `TPE_BIBLIO_CACHE_DIR` or `Resolver::with_cache_dir` | no cache |
| NCBI API key | `Client::with_key(KEY_NCBI, …)` | none (3 requests/s) |
| offline mode | `Resolver::with_offline(true)` / `Client::with_offline` | off |

`Resolver::from_env()` reads the two variables; `Resolver::new(mailto)` is
unchanged. The CLI still constructs the resolver with `--mailto` /
`TPE_MAILTO` only; wiring `--cache-dir` and the page-1 PMID path into
`tpe bibliography --resolve` is a one-line change in `src/main.rs` that is
not part of this change.

## Limits

- The parsers reproduce documented shapes; a field a registry renames or
  omits is read as absent, never invented.
- The PubMed `efetch` reader handles the elements listed in its module
  docs and nothing else; it is not an XML parser.
- Title agreement is Levenshtein similarity over folded text; two editions
  of the same work with identical titles are told apart only by DOI, ISSN
  or venue evidence.
- `DATACITE_PREFIXES` is a short hint list for the order of registries; the
  authoritative agency comes from `doi.org/ra/`.
- Bare (unlabelled) ISSNs and ISBNs are extracted but not used to verify the
  paper; a labelled ISSN is used and can refuse a record (for example a
  Crossref record for a different journal under a mistyped DOI).
- Live checks remain opt-in (`cargo test --lib resolve::tests::live_ --
  --ignored`); ordinary test success never depends on a registry.
