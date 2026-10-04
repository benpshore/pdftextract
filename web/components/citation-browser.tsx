'use client';

import { useId, useMemo, useState } from 'react';
import type { DocumentRow, Extracted } from '@/lib/types';
import { CITATION_CSV_COLUMNS, citationCsvRows, citationUrl, filterCitationRows, serializeCitationCsv, validateCitations } from '@/lib/citations';
import type { BibliographicReference, CitationBundle, CitationCollection, CitationMention, CitationCsvRow } from '@/lib/citations';

type View = 'references' | 'mentions' | 'csv';
const PAGE_SIZE = 25;

function Fields({ values }: { values: [string, string | undefined][] }) {
  return <dl className="citation-fields">{values.filter(([, value]) => value !== undefined && value !== '').map(([name, value]) => <div key={name}><dt>{name}</dt><dd>{value}</dd></div>)}</dl>;
}
function CollectionState({ collection, name }: { collection: CitationCollection<unknown>; name: string }) {
  if (collection.state === 'ready' && collection.items.length) return null;
  return <div className="citation-state" role="status">
    <p>{collection.state === 'unavailable' ? name + ' unavailable.' : collection.state === 'pending' ? name + ' pending.' : collection.state === 'failed' ? name + ' extraction failed.' : collection.state === 'partial' ? name + ' are partial.' : 'No ' + name.toLowerCase() + ' were reported.'}</p>
    {collection.state !== 'ready' && <details><summary>Details</summary><p>{collection.reason}</p></details>}
  </div>;
}
function ReferenceCard({ reference }: { reference: BibliographicReference }) {
  const url = citationUrl(reference.url);
  return <article className="citation-card" aria-label="Supplied reference">
    <h4>{reference.title || 'Reference'}</h4>
    <Fields values={[
      ['Printed label', reference.label], ['Page', reference.page === undefined ? undefined : String(reference.page)], ['Authors', reference.authors.join('; ')], ['Year', reference.year], ['Publication', reference.venue],
      ['Reference text', reference.raw], ['DOI', reference.doi], ['PMID', reference.pmid],
    ]} />
    {url && <a href={url} target="_blank" rel="noopener noreferrer">Open supplied source</a>}
    <details><summary>Details</summary><Fields values={[
      ['ID', reference.id], ['URL', reference.url],
      ['Resolution', reference.resolution.status], ['Reported providers', reference.resolution.providers.join(', ')], ['Resolution note', reference.resolution.note],
    ]} /></details>
  </article>;
}
function MentionCard({ mention }: { mention: CitationMention }) {
  return <article className="citation-card" aria-label="In-text mention">
    <h4>{mention.text}</h4>
    <Fields values={[
      ['Page', mention.page === undefined ? undefined : String(mention.page)], ['Context', mention.context],
    ]} />
    <details><summary>Details</summary><Fields values={[
      ['ID', mention.id],
      ['Reported reference IDs', mention.reference_ids.length ? mention.reference_ids.join(', ') : 'No supplied relationship'],
    ]} /></details>
  </article>;
}
function CsvCard({ row }: { row: CitationCsvRow }) {
  return <article className="citation-card" aria-label={'CSV row ' + row.type + ' ' + row.id}>
    <h4>{row.type === 'reference' ? 'Reference' : 'Mention'}: {row.id}</h4>
    <dl className="citation-fields">{CITATION_CSV_COLUMNS.map((column, index) => <div key={column}><dt>{column}</dt><dd>{row.cells[index] || <span className="help">Empty cell</span>}</dd></div>)}</dl>
  </article>;
}

function SuppliedCitations({ bundle, binding }: { bundle: CitationBundle; binding: { hash: boolean; revision: boolean } }) {
  const [view, setView] = useState<View>('references'), [query, setQuery] = useState(''), [page, setPage] = useState(0), [error, setError] = useState('');
  const headingId = useId(), noteId = useId();
  const rows = useMemo(() => citationCsvRows(bundle), [bundle]);
  const filtered = useMemo(() => filterCitationRows(rows, query, view === 'references' ? 'reference' : view === 'mentions' ? 'mention' : 'all'), [rows, query, view]);
  const references = useMemo(() => new Map('items' in bundle.references ? bundle.references.items.map(item => [item.id, item]) : []), [bundle]);
  const mentions = useMemo(() => new Map('items' in bundle.mentions ? bundle.mentions.items.map(item => [item.id, item]) : []), [bundle]);
  const pages = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE)), currentPage = Math.min(page, pages - 1);
  const shown = filtered.slice(currentPage * PAGE_SIZE, (currentPage + 1) * PAGE_SIZE);
  const selectedCollection = view === 'references' ? bundle.references : view === 'mentions' ? bundle.mentions : undefined;
  const hasDataset = view === 'csv' ? rows.length > 0 : !!selectedCollection && 'items' in selectedCollection && selectedCollection.items.length > 0;
  function download() {
    setError('');
    try {
      const url = URL.createObjectURL(new Blob([serializeCitationCsv(filtered)], { type: 'text/csv;charset=utf-8' }));
      const anchor = document.createElement('a'); anchor.href = url; anchor.download = 'citations-' + view + '.csv';
      document.body.appendChild(anchor);
      try { anchor.click(); } finally { anchor.remove(); setTimeout(() => URL.revokeObjectURL(url), 5000); }
    } catch { setError('CSV download could not start. Try again.'); }
  }
  return <section className="citation-browser" aria-label="Citations">
    <h3 id={headingId}>Supplied citations</h3>
    <p className="help">Supplied provenance is unverified.</p>
    <div className="citation-controls">
      <label>Citation view<select aria-label="Citation view" value={view} onChange={event => { setView(event.target.value as View); setPage(0); }}>
        <option value="references">References</option><option value="mentions">In-text mentions</option><option value="csv">CSV rows</option>
      </select></label>
      {hasDataset && <><label>Filter citations<input type="search" aria-label="Filter citations" value={query} onChange={event => { setQuery(event.target.value); setPage(0); }} /></label>
        <button type="button" onClick={download} disabled={!filtered.length} aria-describedby={noteId}>Download CSV</button></>}
    </div>
    {selectedCollection && <CollectionState collection={selectedCollection} name={view === 'references' ? 'References' : 'In-text mentions'} />}
    {view === 'csv' && <><CollectionState collection={bundle.references} name="References" /><CollectionState collection={bundle.mentions} name="In-text mentions" /></>}
    {hasDataset && <>
      <p className="help" id={noteId}>CSV includes all {filtered.length} rows in this view and filter. Formula-like cells receive a leading apostrophe; canonical data stays unchanged. CSV rows shows these exact cell values.</p>
      <p role="status">{filtered.length ? `${currentPage * PAGE_SIZE + 1}–${Math.min((currentPage + 1) * PAGE_SIZE, filtered.length)} of ${filtered.length} rows` : query.trim() ? 'No matching citations.' : 'No rows to display.'}</p>
      <div className="citation-list" aria-labelledby={headingId}>{shown.map(row => view === 'csv' ? <CsvCard key={row.type + row.id} row={row} /> : row.type === 'reference' ? <ReferenceCard key={row.id} reference={references.get(row.id)!} /> : <MentionCard key={row.id} mention={mentions.get(row.id)!} />)}</div>
      {pages > 1 && <nav className="citation-controls" aria-label="Citation pages"><button type="button" disabled={currentPage === 0} onClick={() => setPage(currentPage - 1)}>Previous citations</button><span>Page {currentPage + 1} of {pages}</span><button type="button" disabled={currentPage + 1 === pages} onClick={() => setPage(currentPage + 1)}>Next citations</button></nav>}
    </>}
    {error && <p role="alert">{error}</p>}
    <details className="citation-details"><summary>Details</summary>
      <p>This is supplied data. Matching stored identifiers does not authenticate the provider or verify extraction accuracy. Metadata resolution is reported per reference; no lookups run here.</p>
      <Fields values={[
        ['Document ID', bundle.source.document_id], ['Original filename (reported)', bundle.source.original_name], ['Source URL (reported)', bundle.source.source_url],
        ['Display name (reported)', bundle.document?.display_name], ['Document title (reported)', bundle.document?.title], ['Document authors (reported)', bundle.document?.authors.join('; ')],
        ['Producer (reported)', bundle.provenance.producer], ['Native version (reported)', 'native_version' in bundle.provenance ? bundle.provenance.native_version : undefined], ['GROBID version (reported)', 'grobid_version' in bundle.provenance ? bundle.provenance.grobid_version : undefined], ['Generated (reported)', bundle.provenance.generated_at],
        ['Source hash', binding.hash ? 'Matches selected document' : 'Not checked: selected document hash unavailable'],
        ['Result revision', binding.revision ? 'Matches selected result' : bundle.source.result_key ? 'Not checked: selected result revision unavailable' : 'Not checked: no result revision supplied'],
      ]} />
    </details>
  </section>;
}

/** The parent keys this component by selected document identity to clear view/filter state. */
export function CitationBrowser({ result, record }: { result: Extracted; record?: DocumentRow }) {
  const checked = useMemo(() => validateCitations(result.bibliography, record), [result.bibliography, record]);
  if (checked.state !== 'available') return <section className="citation-browser citation-state" aria-label="Citations">
    <h3>{checked.state === 'unavailable' ? 'Bibliography unavailable' : 'Supplied citations unavailable'}</h3>
    <p role="status">{checked.state === 'unavailable' ? checked.message : 'The supplied result could not be matched or validated.'}</p>
    {checked.state === 'invalid' && <details><summary>Details</summary><p>{checked.message}</p></details>}
  </section>;
  return <SuppliedCitations key={checked.bundle.source.document_id + ':' + (record?.result_key || checked.bundle.source.result_key || '')} bundle={checked.bundle} binding={checked.binding} />;
}
