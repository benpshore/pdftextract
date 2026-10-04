import type { DocumentRow } from './types';

/** A consumer contract, not a native service or an adapter for arbitrary JSON. */
export type CitationResolution = {
  status: 'resolved' | 'unresolved' | 'not-requested' | 'unavailable';
  providers: ('doi' | 'crossref' | 'pmid' | 'europe-pmc' | 'publisher')[];
  note?: string;
};
export type BibliographicReference = {
  id: string;
  /** Printed label and page, only when explicitly supplied by the producer. */
  label?: string;
  page?: number;
  title?: string;
  authors: string[];
  year?: string;
  venue?: string;
  raw?: string;
  doi?: string;
  pmid?: string;
  url?: string;
  resolution: CitationResolution;
};
export type CitationMention = {
  id: string;
  text: string;
  context?: string;
  page?: number;
  /** Explicit producer relationships only. An empty array means unlinked. */
  reference_ids: string[];
};
export type CitationCollection<T> =
  | { state: 'unavailable' | 'pending' | 'failed'; reason: string }
  | { state: 'ready'; items: T[] }
  | { state: 'partial'; items: T[]; reason: string };
export type CitationProvenance = { generated_at: string } & (
  | { producer: 'native'; native_version: string }
  | { producer: 'grobid'; grobid_version: string }
  | { producer: 'native-grobid'; native_version: string; grobid_version: string }
);
export type CitationBundle = {
  schema: 'tpe.web-citations';
  version: 1;
  source: { document_id: string; sha256?: string; result_key?: string; original_name: string; source_url?: string };
  provenance: CitationProvenance;
  /** Optional supplied metadata. Never changes the original filename/source. */
  document?: { title?: string; authors: string[]; display_name?: string };
  references: CitationCollection<BibliographicReference>;
  mentions: CitationCollection<CitationMention>;
};
export type CitationValidation =
  | { state: 'unavailable' | 'invalid'; message: string }
  | { state: 'available'; bundle: CitationBundle; binding: { hash: boolean; revision: boolean } };

const HASH = /^[a-f\d]{64}$/i;
const own = (value: object, key: string) => Object.prototype.hasOwnProperty.call(value, key);

/** Unknown persisted JSON must pass this boundary before any scholarly control renders. */
export function validateCitations(value: unknown, record?: Pick<DocumentRow, 'id' | 'sha256' | 'result_key'>): CitationValidation {
  if (value === undefined || value === null) return { state: 'unavailable', message: 'No native/GROBID bibliography result was supplied for this document.' };
  const fail = (message: string): never => { throw new Error(message); };
  function object(input: unknown, required: string[], optional: string[] = []): Record<string, unknown> {
    if (!input || typeof input !== 'object' || Array.isArray(input)) fail('Expected an object.');
    const result = input as Record<string, unknown>, allowed = new Set([...required, ...optional]);
    if (Object.keys(result).some(key => !allowed.has(key)) || required.some(key => !own(result, key))) fail('Missing or unsupported fields.');
    return result;
  }
  function string(input: unknown, max = Infinity, nonempty = false): asserts input is string {
    if (typeof input !== 'string' || input.length > max || (nonempty && !input.trim()) || input.includes('\0')) fail('Invalid text field.');
  }
  function id(input: unknown): asserts input is string {
    string(input, Infinity, true);
    if (/[\u0000-\u001f\u007f]/.test(input as string)) fail('Invalid identifier.');
  }
  function strings(input: unknown, maxItems = Infinity, maxText = Infinity) {
    if (!Array.isArray(input) || input.length > maxItems) fail('Invalid text list.');
    for (const item of input as unknown[]) string(item, maxText, true);
  }
  function optionalStrings(input: Record<string, unknown>, fields: string[]) {
    for (const field of fields) if (own(input, field)) string(input[field]);
  }
  function resolution(input: unknown) {
    const item = object(input, ['status', 'providers'], ['note']);
    if (!['resolved', 'unresolved', 'not-requested', 'unavailable'].includes(item.status as string)) fail('Invalid resolution state.');
    strings(item.providers, 5, 32);
    const providers = item.providers as string[];
    if (providers.some(provider => !['doi', 'crossref', 'pmid', 'europe-pmc', 'publisher'].includes(provider)) || new Set(providers).size !== providers.length) fail('Invalid resolution provider.');
    if (item.status === 'resolved' && !providers.length) fail('Resolved metadata needs a reported provider.');
    if (item.status === 'not-requested' && providers.length) fail('Unrequested resolution cannot report a provider.');
    optionalStrings(item, ['note']);
  }
  function collection(input: unknown, check: (item: unknown) => void) {
    if (!input || typeof input !== 'object' || Array.isArray(input)) fail('Invalid collection.');
    const state = (input as Record<string, unknown>).state;
    if (state === 'unavailable' || state === 'pending' || state === 'failed') {
      const item = object(input, ['state', 'reason']); string(item.reason, Infinity, true); return;
    }
    if (state !== 'ready' && state !== 'partial') fail('Invalid collection state.');
    const item = object(input, state === 'partial' ? ['state', 'items', 'reason'] : ['state', 'items']);
    if (state === 'partial') string(item.reason, Infinity, true);
    if (!Array.isArray(item.items)) fail('Invalid collection rows.');
    for (const row of item.items as unknown[]) check(row);
  }
  try {
    const root = object(value, ['schema', 'version', 'source', 'provenance', 'references', 'mentions'], ['document']);
    if (root.schema !== 'tpe.web-citations' || root.version !== 1) fail('Unsupported citation schema or version.');
    const source = object(root.source, ['document_id', 'original_name'], ['sha256', 'result_key', 'source_url']);
    id(source.document_id); string(source.original_name, Infinity, true);
    if (own(source, 'sha256')) { string(source.sha256, 64); if (!HASH.test(source.sha256 as string)) fail('Invalid source SHA-256.'); }
    if (own(source, 'result_key')) id(source.result_key);
    if (own(source, 'source_url')) string(source.source_url, Infinity, true);
    const reported = root.provenance as Record<string, unknown> | null;
    const producer = reported && reported.producer;
    if (producer !== 'native' && producer !== 'grobid' && producer !== 'native-grobid') fail('Native/GROBID provenance is required.');
    const versions = producer === 'native' ? ['native_version'] : producer === 'grobid' ? ['grobid_version'] : ['native_version', 'grobid_version'];
    const provenance = object(root.provenance, ['producer', 'generated_at', ...versions]);
    for (const field of versions) string(provenance[field], Infinity, true);
    string(provenance.generated_at, 64, true);
    if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,3})?Z$/.test(provenance.generated_at as string) || !Number.isFinite(Date.parse(provenance.generated_at as string))) fail('Invalid provenance timestamp.');
    if (own(root, 'document')) {
      const metadata = object(root.document, ['authors'], ['title', 'display_name']);
      strings(metadata.authors); optionalStrings(metadata, ['title', 'display_name']);
    }
    const referenceIds = new Set<string>(), mentionIds = new Set<string>();
    collection(root.references, input => {
      const item = object(input, ['id', 'authors', 'resolution'], ['label', 'page', 'title', 'year', 'venue', 'raw', 'doi', 'pmid', 'url']);
      id(item.id); strings(item.authors); optionalStrings(item, ['label', 'title', 'year', 'venue', 'raw', 'doi', 'pmid', 'url']); resolution(item.resolution);
      if (own(item, 'page') && (!Number.isSafeInteger(item.page) || (item.page as number) < 1)) fail('Invalid reference page.');
      if (referenceIds.has(item.id as string)) fail('Duplicate reference identifier.');
      referenceIds.add(item.id as string);
      if (![item.title, item.raw, item.doi, item.pmid].some(field => typeof field === 'string' && field.trim())) fail('Reference has no bibliographic content.');
    });
    collection(root.mentions, input => {
      const item = object(input, ['id', 'text', 'reference_ids'], ['context', 'page']);
      id(item.id); string(item.text, Infinity, true); optionalStrings(item, ['context']); strings(item.reference_ids);
      if (mentionIds.has(item.id as string)) fail('Duplicate mention identifier.');
      mentionIds.add(item.id as string);
      if (own(item, 'page') && (!Number.isSafeInteger(item.page) || (item.page as number) < 1)) fail('Invalid mention page.');
      const linked = item.reference_ids as string[];
      if (new Set(linked).size !== linked.length || linked.some(reference => !referenceIds.has(reference))) fail('Mention points to an unavailable reference.');
    });
    if (!record || typeof record.id !== 'string' || !record.id.trim()) return { state: 'unavailable', message: 'Citation source identity is unavailable. Open a saved document to match supplied citations to its source.' };
    if (source.document_id !== record.id) fail('Citations belong to a different document.');
    if (record.sha256 != null && typeof record.sha256 !== 'string') fail('Invalid selected source hash.');
    if (record.result_key != null && typeof record.result_key !== 'string') fail('Invalid selected result revision.');
    const hash = typeof record.sha256 === 'string' && record.sha256.length > 0;
    if (hash && (!HASH.test(record.sha256) || typeof source.sha256 !== 'string' || source.sha256.toLowerCase() !== record.sha256.toLowerCase())) fail('Citation source hash is missing or does not match this document.');
    const revision = typeof record.result_key === 'string' && record.result_key.length > 0 && typeof source.result_key === 'string';
    if (revision && source.result_key !== record.result_key) fail('Citation result revision is stale.');
    return { state: 'available', bundle: value as CitationBundle, binding: { hash, revision } };
  } catch (error) {
    return { state: 'invalid', message: 'Supplied citations cannot be shown. ' + (error instanceof Error ? error.message : 'Invalid citation data.') };
  }
}

/** Links are supplied evidence, never reconstructed from guessed identifiers. */
export function citationUrl(value: string | undefined): string | undefined {
  if (!value || value !== value.trim() || /[\u0000-\u0020\u007f]/.test(value)) return undefined;
  try {
    const url = new URL(value);
    return (url.protocol === 'https:' || url.protocol === 'http:') && !url.username && !url.password ? url.href : undefined;
  } catch { return undefined; }
}

export const CITATION_CSV_COLUMNS = ['type', 'id', 'label', 'title', 'authors', 'year', 'venue', 'raw', 'doi', 'pmid', 'url', 'page', 'mention', 'reference_ids', 'context', 'resolution_status', 'resolution_providers', 'resolution_note'] as const;
export type CitationCsvRow = { type: 'reference' | 'mention'; id: string; cells: string[] };

/** This spreadsheet-safe projection leaves every canonical field untouched. */
export function citationCsvCell(value: string): string {
  return /^[\s\u0000-\u001f\u007f]*[=+\-@]/u.test(value) || /^[\t\r\n]/.test(value) ? "'" + value : value;
}
export function citationCsvRows(bundle: CitationBundle): CitationCsvRow[] {
  const rows: CitationCsvRow[] = [];
  function row(type: CitationCsvRow['type'], id: string, values: Partial<Record<typeof CITATION_CSV_COLUMNS[number], string>>): CitationCsvRow {
    const data = { ...values, type, id };
    return { type, id, cells: CITATION_CSV_COLUMNS.map(column => citationCsvCell(data[column] || '')) };
  }
  if ('items' in bundle.references) for (const ref of bundle.references.items) {
    rows.push(row('reference', ref.id, { label: ref.label, title: ref.title, authors: JSON.stringify(ref.authors), year: ref.year, venue: ref.venue, raw: ref.raw, doi: ref.doi, pmid: ref.pmid, url: ref.url, page: ref.page === undefined ? '' : String(ref.page), resolution_status: ref.resolution.status, resolution_providers: JSON.stringify(ref.resolution.providers), resolution_note: ref.resolution.note }));
  }
  if ('items' in bundle.mentions) for (const mention of bundle.mentions.items) {
    rows.push(row('mention', mention.id, { page: mention.page === undefined ? '' : String(mention.page), mention: mention.text, reference_ids: JSON.stringify(mention.reference_ids), context: mention.context }));
  }
  return rows;
}
export function serializeCitationCsv(rows: CitationCsvRow[]): string {
  const quote = (cell: string) => '"' + cell.replaceAll('"', '""') + '"';
  return '\uFEFF' + [CITATION_CSV_COLUMNS as readonly string[], ...rows.map(row => row.cells)].map(row => row.map(quote).join(',')).join('\r\n') + '\r\n';
}
export function filterCitationRows(rows: CitationCsvRow[], query: string, kind: 'all' | 'reference' | 'mention' = 'all'): CitationCsvRow[] {
  const needle = query.trim().toLocaleLowerCase();
  return rows.filter(row => (kind === 'all' || row.type === kind) && (!needle || row.cells.some(cell => cell.toLocaleLowerCase().includes(needle))));
}
