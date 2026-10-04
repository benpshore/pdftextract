# Supplied scholarly citations boundary

This increment is a **consumer contract**, not a citation extraction service. The web app has no scholarly producer, resolver, native adapter, or GROBID endpoint in this change. Ordinary `Extracted.links`, page metadata, DOI-shaped text, and arbitrary imported native JSON do not become bibliography entries. No web heuristic creates references or in-text relationships.

`Extracted.bibliography?: CitationBundle` is defined in `web/lib/citations.ts`. The component always validates this field as unknown persisted JSON. Type annotations and user-writable saved JSON do not establish trusted provenance. A valid payload is displayed as supplied data with unverified provenance.

## Version 1 contract

```ts
{
  schema: 'tpe.web-citations',
  version: 1,
  source: {
    document_id: 'saved-document-id',
    sha256: '<64 hexadecimal characters>',
    result_key: 'saved-document-id/results/current-revision',
    original_name: 'original.pdf',
    source_url?: 'https://publisher.example/source'
  },
  provenance: {
    producer: 'native-grobid',
    native_version: '<actual reported version>',
    grobid_version: '<actual reported version>',
    generated_at: '<UTC ISO timestamp>'
  },
  document?: { title?: string, authors: string[], display_name?: string },
  references: {
    state: 'ready',
    items: [{
      id: 'b1', label?: string, page?: number,
      title: 'Supplied title', authors: ['Supplied author'],
      year?: string, venue?: string, raw?: string, doi?: string,
      pmid?: string, url?: string,
      resolution: {
        status: 'not-requested', providers: [], note?: string
      }
    }]
  },
  mentions: { state: 'unavailable', reason: 'No in-text extraction supplied' }
}
```

The example is explanatory; there are no production fixtures. Provenance is a strict union: `native` requires only `native_version`, `grobid` requires only `grobid_version`, and `native-grobid` requires both. Every variant requires `generated_at`. No engine version is invented to satisfy the contract.

Each collection independently uses `{state: 'ready', items: [...]}`, `{state: 'partial', items: [...], reason: string}`, or `{state: 'unavailable' | 'pending' | 'failed', reason: string}`. Pending/failed/unavailable states cannot include rows. Ready with no rows means completed-empty; it does not mean unavailable. Partial results remain explicitly partial, including zero rows. A native reference-only bibliography never implies in-text extraction.

References require stable unique IDs, an authors array (possibly empty), and explicit resolution status. Optional `label` preserves the actual printed reference label separately from the opaque ID; optional `page` is a supplied positive integer. Neither is inferred from order or an XML ID. Both appear in the reading card and CSV. At least one title/raw/DOI/PMID field must contain bibliographic content. Resolution states are `resolved`, `unresolved`, `not-requested`, or `unavailable`. Reported provider values are `doi`, `crossref`, `pmid`, `europe-pmc`, and `publisher`. `resolved` requires a provider; `not-requested` cannot report one. This UI neither decides applicability nor performs or verifies DOI/Crossref/PMID/Europe PMC/publisher resolution. The future producer must use actual native/GROBID extraction and applicable resolvers, preserving honest outcome/provider evidence.

Mentions are `{id: string, text: string, context?: string, page?: positive integer, reference_ids: string[]}`. Reference IDs are explicit matched relationships and must name supplied references. An empty relationship array is valid even when references are unavailable, pending, failed, or partial. Unmatched native target strings must not be placed in `reference_ids`; retain those in the producer's canonical artifact until a contract extension represents them. The UI never guesses relationships from mention text, order, page, or DOI.

Unknown keys, versions, malformed objects/arrays/optional values, duplicate IDs, inconsistent resolution claims, invalid pages, and dangling matched relationships are rejected. No bibliographic row count or text-content cap truncates data. The component paginates 25 rows at a time and exports all rows in the selected view/filter.

## Source and revision binding

The selected `DocumentRow` supplies comparison context independently of the embedded bibliography. No selected record means source identity is unavailable and no scholarly controls appear. `source.document_id` must match its ID. A known nonempty record SHA-256 requires a matching payload hash (case-insensitive). When both the record and payload declare a `result_key`, they must match exactly. A missing required known hash or any known mismatch rejects the payload. The embedded result revision is optional; without it, Details explicitly says no result revision was supplied and no revision comparison was performed. When the selected record lacks a hash or revision, Details identifies that unavailable check. The artifact is never labelled verified.

`DocumentRow.result_key` describes the existing storage revision returned by the document GET endpoint; adding its optional type changes no API. Uploads/PATCH allocate a new result key server-side after JSON serialization, so existing save helpers cannot embed that future key. An otherwise valid supplied bundle can therefore omit `source.result_key` and round-trip through existing storage, using the outer GET record/result envelope as current context while explicitly retaining no embedded revision assurance. A future producer/publication adapter could coordinate a declared final storage key before publishing it. This change adds neither that adapter nor a scholarly producer. Saving an artifact with an old declared key makes the declaration stale and causes rejection when the current revision is known. Tests use synthetic record/result fixtures and separately exercise declared/missing revision behavior.

Matching these strings prevents known cross-document/stale displays; it does not authenticate native/GROBID execution, cryptographically verify provider claims, or validate scholarly accuracy. Saved JSON remains user-writable. The selected record's original filename/source remain canonical. Artifact `original_name`, `source_url`, and optional metadata-derived `display_name` are labelled *reported* in Details and never overwrite stored source metadata or rename files. No display name is derived from scraped DOI-shaped text.

## Component and CSV

Mount `<CitationBrowser key={selectedIdentity} result={result} record={selectedRecord} />`. The parent key resets filters/views across documents. The component also resets on a selected record revision change, including when no revision was embedded. Missing, invalid, or unmatched bundles have a named unavailable state and no scholarly action buttons. A valid bundle offers a native select for References, In-text mentions, or CSV rows. Pending/failed/unavailable/empty collections stay distinct; filtering/download controls appear only when a selected dataset has rows. There is no extraction/retry button without a real service. Opaque IDs, resolution diagnostics, and provenance are in Details; CSV inspection intentionally shows its actual columns.

CSV uses the exported `CITATION_CSV_COLUMNS`, `citationCsvRows`, and `serializeCitationCsv` functions. The CSV inspector is a stacked list of exact exported cell values, including empty cells, not a horizontal spreadsheet. Arrays use JSON cell encoding to preserve individual Unicode author names, provider names, and reference IDs. Downloads contain a UTF-8 BOM, quoted fields with escaped quotes, CRLF record separators, and preserved embedded newlines. Formula-like cells (including after whitespace/control characters) and cells starting with tab/CR/LF receive an apostrophe. The UI explains this spreadsheet-safe projection; canonical JSON fields remain unchanged. This CSV is a reading/export projection, not a lossless substitute for the complete provenance/source artifact.

Filtering applies to the same CSV projection and download. The download includes all filtered rows across pages in the selected view; the button is disabled for an empty filter match. CSV rows view combines the independently available reference and mention rows. CSV inspection and its download share exactly the same row projection/filter. Native HTML is never inserted. Only supplied absolute, credential-free HTTP(S) URLs become links, with `noopener noreferrer`; identifiers are not converted to guessed URLs. Unsafe URLs remain literal text/cells.

## Focused validation

Run `node web/scripts/test-citations.mjs`. It loads the actual TypeScript module and React component with esbuild, tests hostile unknown shapes/binding, independent states, provider variants, literal rendering/safe URLs, pagination, filter/download bytes, formula-safe CSV round trips, canonical preservation, and document reset. Fixtures are clearly synthetic; these checks do not prove native/GROBID extraction or resolver E2E. Real-browser workspace checks separately exercise integration and narrow-screen layout.
