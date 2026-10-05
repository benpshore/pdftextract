# Reverse-citation exports (`tpe-export`)

`crates/tpe-export` turns one extracted article and the works in its
reference list into files other tools can take in: Zotero RDF, Zotero CSV
and a small `SQLite` database. The article is item 1; every reference entry
becomes an item linked back to it ("reverse citation": the cited work knows
what cites it). Nothing is fetched; the export only reshapes what the engine
recorded. A wrong parse stays visible: the printed entry is carried verbatim
in `Extra` (RDF `dc:description`, CSV `Extra`, `SQLite` `works.raw`).

## Command line

```sh
tpe-export --format zotero-rdf --input out/<hash>.json --output paper.rdf
tpe-export --format csv        --input paper.bibliography.json --output paper.csv
tpe-export --format sqlite     --input corpus.sqlite --hash 9f2c --output paper.sqlite
```

- `--format` is one of `zotero-rdf`, `csv`, `sqlite`.
- `--input` is an engine JSON file or a ledger (`SQLite`, sniffed by its file
  header). JSON inputs: the `<hash>.json` an `extract --out` run writes (an
  `ExtractionResult`), the object `tpe extract --json` prints (its `result`
  member is used), or one record of `tpe bibliography` (the paper's own
  record comes from `paper`, so metadata is only present after `--resolve`;
  a `failed` record is refused).
- Ledger inputs export one run: `--run <id>`, `--hash <hex prefix>` (latest
  run of that document) or, with neither, the latest run in the ledger. The
  file is first opened read-only and checked for the ledger tables, so a
  foreign database is never given the ledger schema. Opening a ledger may
  create its `-wal`/`-shm` companions, as every ledger reader does.
- `--output` is never overwritten unless `--force` is given. The file is
  staged beside the target under a temporary name and renamed into place, so
  an interrupted export leaves no partial file.
- Exit code 0 on success with one summary line on stdout; 1 with
  `error: ...` on stderr otherwise; 2 for usage errors.

Order is deterministic: the article first, then references in list order;
creators in printed order; identifiers in a fixed scheme order. Two runs on
the same input produce byte-identical RDF and CSV, and `SQLite` files with
identical rows (`SQLite` stamps a change counter in its header).

## Zotero RDF (`--format zotero-rdf`)

The output has the statements Zotero's own "Zotero RDF" export writes, and
the text layout of the RDF/XML serializer Zotero bundles, so the file reads
back through Zotero's RDF importer like a Zotero export. Specifically:

- Root `rdf:RDF` with one `xmlns:` declaration per line, only for the
  prefixes used, in first-use order with `rdf` first. The prefixes and URIs
  are those of the translator: `rdf`, `bib` (`http://purl.org/net/biblio#`),
  `dc`, `dcterms`, `prism`, `foaf`, `vcard`, `vcard2`, `link`, `z`
  (`http://www.zotero.org/namespaces/export#`). No XML declaration (the
  serializer writes none).
- One typed element per item with `rdf:about`. The resource identifier is
  `urn:isbn:<ISBN>` when the item has an unused ISBN, else the item's URL
  when it is an unused absolute http(s) URL, else `#item_<n>` (`n` = 1 for
  the article, reference index + 1 for cited works). The element is the
  translator's class for the type: `bib:Article` (journalArticle),
  `bib:Book`, `bib:BookSection`, `bib:Thesis`, `bib:Report`, `bib:Document`
  (webpage); `conferencePaper` and `preprint` have no class in the translator
  and serialise as `rdf:Description`. `z:itemType` always names the type.
- Containers under `dcterms:isPartOf`: `bib:Journal` for journal articles and
  conference papers (`dc:title` = journal/proceedings title, `prism:volume`,
  `prism:number` = issue, `dc:identifier` `DOI …` / `ISSN …`), `bib:Book` for
  book sections, `z:Website` for web pages. Without a container those
  statements sit on the item. A container with an ISSN becomes a shared
  top-level `urn:issn:` node, as in Zotero.
- Creators: `bib:authors` → `rdf:Seq` → `rdf:li` → `foaf:Person` with
  `foaf:surname` and, when known, `foaf:givenName`.
- Item fields: `dc:title`, `dcterms:abstract`, `dc:date` (the year),
  `bib:pages`, `dc:identifier` with a nested `dcterms:URI`/`rdf:value` for
  the URL, `dc:publisher` → `foaf:Organization`/`foaf:name` for publisher,
  university, institution or repository, `z:type` for thesis/report type,
  `prism:number` for report numbers and arXiv ids, `z:PMID`/`z:PMCID`,
  `dc:subject` for keywords (manual tags), `dc:description` for Extra.
- Reverse citation: every cited work carries
  `<dcterms:isReferencedBy rdf:resource="<article>"/>` and
  `<dc:relation rdf:resource="<article>"/>`; the article carries one
  `dc:relation` per cited work. Zotero imports `dc:relation` as related
  items in both directions. Its importer only turns a
  `dcterms:isReferencedBy` target into a child note when that target is a
  `bib:Memo`, so the citation link is accepted without side effects.
- Layout: four-space indentation; a subtree whose one-line form is shorter
  than `80 - 4 × level` characters is folded onto one line prefixed by
  three spaces (the serializer's behaviour, visible in real Zotero exports
  as lines such as `   <z:AutomaticTag><rdf:value>…`). Only `&`, `<`, `>`
  and `"` are escaped, as the serializer does.

Statement order within an item follows the translator: type, container,
publisher, creators, notes slot (`isReferencedBy`), relations, tags, then
the item's fields in the order the Zotero schema (version 45) lists them for
the type. In Zotero that last order comes from the local database's field
ids and can differ between installations; RDF semantics do not depend on it.

## Zotero CSV (`--format csv`)

The column set is `exportedFields` of Zotero's CSV export translator, all 87
columns in its order: `Key, Item Type, Publication Year, Author, Title,
Publication Title, ISBN, ISSN, DOI, Url, Abstract Note, Date, Date Added,
Date Modified, Access Date, Pages, Num Pages, Issue, Volume, Number Of
Volumes, Journal Abbreviation, Short Title, Series, Series Number, Series
Text, Series Title, Publisher, Place, Language, Rights, Type, Archive,
Archive Location, Library Catalog, Call Number, Extra, Notes, File
Attachments, Link Attachments, Manual Tags, Automatic Tags, Editor, …,
Guest, Number, Edition, …, Legislative Body`. Header labels are derived
exactly as the translator derives them (capitalised, a space before each
capital following a lower-case letter: `abstractNote` → `Abstract Note`,
`ISBN` stays `ISBN`, `url` → `Url`).

Quoting follows the translator and RFC 4180: every field is wrapped in
double quotes, embedded quotes are doubled, runs of CR/LF inside a value
become one space, multi-value fields (`Author`, `Manual Tags`) join with
`; `, records are separated by `\n` with no trailing newline, and the file
starts with a UTF-8 byte-order mark (the translator writes one). `Key` is a
deterministic eight-character key in Zotero's key alphabet derived from the
document hash and the item number. Columns the engine cannot fill (dates
added/modified, attachments, notes, editors, …) are empty, never invented.

## SQLite (`--format sqlite`)

Written with WAL off (`journal_mode = DELETE`, so no `-wal`/`-shm` files),
`foreign_keys = ON`, `user_version = 1`, inside one transaction. Schema
(verbatim in `crates/tpe-export/src/sqlite.rs`, `SCHEMA_SQL`):

| table | columns | notes |
| --- | --- | --- |
| `export_meta` | `key`, `value` | `generator`, `schema_version`, `zotero_schema_version`, `source_sha256`, `work_count` |
| `works` | `id`, `role` (`citing`/`cited`), `zotero_key`, `item_type`, `reference_index`, `label`, `title`, `abstract`, `publication_title`, `publisher`, `type`, `number`, `date`, `year`, `volume`, `issue`, `pages`, `extra`, `raw` | `id` 1 is the article; base-field names as in Zotero |
| `creators` | `work_id`, `seq`, `creator_type`, `last_name`, `first_name` | `creator_type` is always `author` |
| `identifiers` | `work_id`, `scheme`, `value` | scheme ∈ `doi`, `issn`, `isbn`, `url`, `arxiv`, `pmid`, `pmcid`; index on `(scheme, value)` |
| `tags` | `work_id`, `seq`, `tag` | the paper's keywords |
| `citations` | `citing_work_id`, `cited_work_id`, `reference_index`, `label` | one edge per reference entry |

Queries such as "which works cite DOI x" join `identifiers` to `citations`.

## Field mapping and type inference

Engine fields (`src/schema.rs`: `Metadata`, `ReferenceEntry`, `Resolved`)
map to Zotero base fields: `title`, `authors` (split into surname/given
names: comma form, Vancouver `Smith JA`, natural order with surname
particles such as `van der`), `year` → `date`, `venue` → the container title
or publisher depending on the type, `volume`, `issue`, `pages`, `doi`
(printed DOI, else the `doi.org` link annotation, else the resolved DOI),
`url` (absolute http(s) only), `arxiv_id`, resolved `pmid`/`pmcid`, the
paper's `abstract_text` and `keywords`. When a reference has no printed
title, authors, year or venue but a verified `resolved` record has them,
those are used.

The item type is inferred from the venue string, in this order; the venue
itself is never guessed:

| cue | itemType | venue goes to |
| --- | --- | --- |
| `thesis` / `dissertation` (with PhD, MSc, … prefix) | `thesis` | `thesisType` = matched phrase, `university` = rest |
| `Technical Report`, `Tech. Rep.`, `working paper`, `RFC n`, … | `report` | `reportType`, `reportNumber` (number after the phrase), `institution` = rest |
| `arXiv`, `preprint`, `bioRxiv`, `SSRN`, … | `preprint` | `repository` = `arXiv` and `archiveID` = `arXiv:<id>` only when an arXiv id is present |
| `In:` / `In ` prefix, `(Ed.)`, `editors`, `edited by` | `bookSection` | `bookTitle` (without the `In:`) |
| `Proceedings`, `Proc.`, `Conference`, `Symposium`, `Workshop`, … or a known conference acronym | `conferencePaper` | `proceedingsTitle` |
| publisher names (`Press`, `Springer`, `Wiley`, …) or `2nd ed.`, with no volume/issue | `book` | `publisher` |
| anything else | `journalArticle` | `publicationTitle` |

Without a venue: an arXiv id → `preprint`; a URL and no DOI → `webpage`;
thesis/report/publisher cues in the raw entry → `thesis`/`report`/`book`;
otherwise `preprint` (Zotero's type for unreviewed works). `Extra` lists
`arXiv: <id>` (non-preprints), `PMID:`/`PMCID:` (non-journal items, which
have no native field), `Reference index:`, `Reference label:` and
`Reference text:` (the printed entry); the article's Extra records
`Source SHA-256: <hash>`.

## Limits

- Item types are heuristic; `raw` is authoritative. ISSN/ISBN are modelled
  and written when present but the engine does not extract them today.
- The engine knows years, not full dates; `date` is `YYYY`.
- ORCID, affiliations and in-text citation markers are not exported.
- Zotero's own exports carry `dateAdded`/`dateModified`, `accessDate` and
  `libraryCatalog`; those are left empty rather than invented.
- No BibTeX/RIS/CSL-JSON; no network resolution (use `tpe bibliography
  --resolve` first if resolved records are wanted).

## Sources

The shapes were taken from Zotero's own code, read on 2026-10-05:

- `Zotero RDF.js`, translator `14763d24-8ba0-45df-8f52-b8d1108e7ac9`
  (lastUpdated 2026-03-06), github.com/zotero/translators: namespaces,
  class per item type, container/series/publisher/creator statements, field
  → predicate mapping, resource identifiers (`doExport`), `dc:relation`.
- `CSV.js`, translator `25f4c5e2-d790-4daa-a667-797619c7e2f2` (lastUpdated
  2022-06-28), github.com/zotero/translators: `exportedFields`, header
  derivation, quoting, BOM, separators.
- `RDF.js` (the RDF importer), github.com/zotero/translators: handling of
  `dc:relation`, `dcterms:isReferencedBy` (notes only for `bib:Memo`) and
  `dc:identifier` prefixes `DOI `, `ISSN `, `ISBN `.
- `src/rdf/serialize.js` (`statementsToXML`, `XMLtreeToString`),
  github.com/zotero/translate: indentation, folding, escaping, namespace
  declarations, `rdf:about`/`rdf:resource`/anonymous blank nodes.
- `chrome/content/zotero/xpcom/utilities_internal.js`
  (`itemToExportFormat`), github.com/zotero/zotero: type-specific fields are
  handed to export translators under their base names (`thesisType` →
  `type`, `university` → `publisher`, `bookTitle` → `publicationTitle`),
  which is why thesis types appear as `z:type`.
- `schema.json` version 45, github.com/zotero/zotero-schema: item types,
  their fields and base-field mappings, creator types.

## Verification

```sh
export CARGO_TARGET_DIR=/home/user/pdftextract/target
cargo clippy -p tpe-export --all-targets -- -D warnings
cargo test -p tpe-export
```

Tests (`crates/tpe-export/tests/golden.rs`) render the hand-written fixture
`tests/fixtures/sample.json` (eleven items covering every type, escaping,
particles, resolved-only fields and a reused URL) and compare it with the
reviewed goldens `tests/golden/sample.rdf`, `sample.csv` and the row dump
`sample.sqlite.txt`; parse the RDF with `roxmltree` and check namespaces,
classes, creators, identifiers and both link directions; parse the CSV with
an RFC 4180 reader; read the `SQLite` file back read-only (journal mode,
`user_version`, `integrity_check`, `foreign_key_check`); check that the
same result comes from a ledger (`--hash`, `--run`, latest) and from a
bibliography record; and exercise the CLI's overwrite refusal, `--force`,
bad format and missing input.
