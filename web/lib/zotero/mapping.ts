// From an extraction result to Zotero item JSON. Nothing is invented: a
// field the extraction does not provide stays empty, and the user can edit
// the prefilled values before sending.

import type { Extracted, LinkEvidence } from '../types';

export type ZoteroItemType = 'journalArticle' | 'preprint' | 'conferencePaper' | 'report' | 'book' | 'webpage' | 'document';
export const ITEM_TYPES: { value: ZoteroItemType; label: string }[] = [
  { value: 'journalArticle', label: 'Journal article' },
  { value: 'preprint', label: 'Preprint' },
  { value: 'conferencePaper', label: 'Conference paper' },
  { value: 'report', label: 'Report' },
  { value: 'book', label: 'Book' },
  { value: 'webpage', label: 'Web page' },
  { value: 'document', label: 'Document' },
];

export type ArticleMetadata = { title: string; authors: string[]; date: string; venue: string; doi: string; url: string; abstract: string; arxivId: string; pmid: string };
export type Reference = { url: string; doi?: string; label?: string; page?: number };
export type ZoteroCreator = { creatorType: string; name?: string; firstName?: string; lastName?: string };

/** Container field per item type, in preference order (mirrors crates/tpe-zotero). */
export function venueFields(itemType: string): string[] {
  switch (itemType) {
    case 'conferencePaper': return ['proceedingsTitle', 'conferenceName'];
    case 'bookSection': return ['bookTitle'];
    case 'book': return ['publisher'];
    case 'report': return ['institution'];
    case 'thesis': return ['university'];
    case 'preprint': return ['repository'];
    case 'webpage': return ['websiteTitle'];
    default: return ['publicationTitle'];
  }
}

const DOI_FIELD_TYPES = new Set(['journalArticle', 'conferencePaper', 'preprint']);

/** Normalise a DOI (`https://doi.org/`, `doi:` prefixes dropped, lower case) or return null. */
export function normalizeDoi(raw: string | undefined | null): string | null {
  if (!raw) return null;
  let value = raw.trim();
  for (let stripped = true; stripped;) {
    stripped = false;
    for (const prefix of ['https://doi.org/', 'http://doi.org/', 'https://dx.doi.org/', 'http://dx.doi.org/', 'doi:']) {
      if (value.toLowerCase().startsWith(prefix)) { value = value.slice(prefix.length).trim(); stripped = true; }
    }
  }
  value = value.replace(/[.,;]+$/, '');
  return /^10\.\d{4,9}\/\S+$/.test(value) ? value.toLowerCase() : null;
}

/** Normalise an arXiv id (`arXiv:` prefix and version dropped) or return null. */
export function normalizeArxivId(raw: string | undefined | null): string | null {
  if (!raw) return null;
  const value = raw.trim().replace(/^arxiv:\s*/i, '').replace(/^https?:\/\/arxiv\.org\/(abs|pdf)\//i, '').replace(/\.pdf$/i, '');
  const match = value.match(/^(\d{4}\.\d{4,5}|[a-z-]+(?:\.[A-Z]{2})?\/\d{7})(v\d+)?$/i);
  return match ? match[1].toLowerCase() : null;
}

function metadataText(metadata: Record<string, unknown> | undefined, names: string[]): string {
  if (!metadata) return '';
  const lower = new Map(Object.entries(metadata).map(([key, value]) => [key.toLowerCase(), value]));
  for (const name of names) {
    const value = lower.get(name.toLowerCase());
    if (typeof value === 'string' && value.trim()) return value.trim();
    if (typeof value === 'number' && Number.isFinite(value)) return String(value);
  }
  return '';
}

function metadataList(metadata: Record<string, unknown> | undefined, names: string[]): string[] {
  if (!metadata) return [];
  const lower = new Map(Object.entries(metadata).map(([key, value]) => [key.toLowerCase(), value]));
  for (const name of names) {
    const value = lower.get(name.toLowerCase());
    if (Array.isArray(value)) return value.filter((entry): entry is string => typeof entry === 'string' && !!entry.trim()).map(entry => entry.trim());
    if (typeof value === 'string' && value.trim()) return value.split(/;|\band\b/).map(part => part.trim()).filter(Boolean);
  }
  return [];
}

/** What the extraction knows about the article itself (editable before sending). */
export function articleMetadata(extracted: Extracted, sourceUrl?: string | null): ArticleMetadata {
  const metadata = extracted.metadata;
  const nested = metadata && typeof metadata.nativeRecord === 'object' && metadata.nativeRecord && typeof (metadata.nativeRecord as Record<string, unknown>).metadata === 'object' ? ((metadata.nativeRecord as Record<string, unknown>).metadata as Record<string, unknown>) : undefined;
  const pick = (names: string[]) => metadataText(metadata, names) || metadataText(nested, names);
  const ownDoi = normalizeDoi(pick(['doi', 'DOI', 'identifier'])) ?? firstPageDoi(extracted.links);
  return {
    title: pick(['title', 'dc.title', 'citation_title']) || extracted.title || '',
    authors: metadataList(metadata, ['authors', 'author', 'creator', 'citation_author']).length ? metadataList(metadata, ['authors', 'author', 'creator', 'citation_author']) : metadataList(nested, ['authors', 'author', 'creator']),
    date: pick(['date', 'year', 'published', 'citation_publication_date', 'citation_date']),
    venue: pick(['journal', 'publicationTitle', 'venue', 'container', 'citation_journal_title', 'siteName']),
    doi: ownDoi ?? '',
    url: pick(['url', 'sourceUrl', 'canonical']) || (sourceUrl ?? ''),
    abstract: pick(['abstract', 'abstractNote', 'description']),
    arxivId: normalizeArxivId(pick(['arxiv', 'arxivId', 'arxiv_id', 'citation_arxiv_id'])) ?? '',
    pmid: pick(['pmid', 'citation_pmid']).replace(/\D/g, ''),
  };
}

/** A DOI link found on the first page (or without page information) is the article's own DOI in most layouts; the user can still change it. */
function firstPageDoi(links: LinkEvidence[]): string | null {
  for (const link of links) {
    const doi = normalizeDoi(link.doi ?? link.url);
    if (doi && (link.page === undefined || link.page <= 1)) return doi;
  }
  return null;
}

/** DOI references found in the text (deduplicated, the article's own DOI excluded). */
export function references(extracted: Extracted, ownDoi?: string): Reference[] {
  const seen = new Set<string>();
  const own = normalizeDoi(ownDoi);
  const out: Reference[] = [];
  for (const link of extracted.links) {
    const doi = normalizeDoi(link.doi ?? link.url);
    const id = doi ?? link.url.trim();
    if (!id || seen.has(id) || (doi && doi === own)) continue;
    seen.add(id);
    out.push({ url: doi ? 'https://doi.org/' + doi : link.url, doi: doi ?? undefined, label: link.label, page: link.page });
  }
  return out;
}

/** `preprint` for arXiv items without a venue, `journalArticle` with a venue or DOI, `webpage` for a plain URL, else `document`. */
export function suggestItemType(meta: ArticleMetadata): ZoteroItemType {
  if (meta.arxivId && !meta.venue) return 'preprint';
  if (meta.venue || meta.doi) return 'journalArticle';
  if (meta.url) return 'webpage';
  return 'document';
}

/** `"Family, Given"` becomes a two-field name; anything else is a single-field name (no guessing where the family name starts). */
export function creatorFromDisplay(display: string, creatorType = 'author'): ZoteroCreator {
  const trimmed = display.trim();
  const comma = trimmed.indexOf(',');
  if (comma > 0 && comma < trimmed.length - 1) {
    const lastName = trimmed.slice(0, comma).trim(), firstName = trimmed.slice(comma + 1).trim();
    if (lastName && firstName) return { creatorType, firstName, lastName };
  }
  return { creatorType, name: trimmed };
}

export type BuildOptions = { itemType: ZoteroItemType; collections: string[]; tags: string[]; template?: Record<string, unknown> | null };

/** The JSON object for `POST /items`. With a template (`GET /items/new?itemType=…`) only its fields are filled, so the object is valid for the type. */
export function buildItem(meta: ArticleMetadata, options: BuildOptions): Record<string, unknown> {
  const template = options.template ?? null;
  const has = (name: string) => !template || name in template;
  const item: Record<string, unknown> = template ? { ...template } : {};
  item.itemType = options.itemType;
  item.title = meta.title;
  item.creators = meta.authors.map(author => creatorFromDisplay(author));
  item.tags = options.tags.map(tag => tag.trim()).filter(Boolean).map(tag => ({ tag }));
  item.collections = options.collections.filter(Boolean);
  item.relations = {};
  if (meta.abstract && has('abstractNote')) item.abstractNote = meta.abstract;
  if (meta.date && has('date')) item.date = meta.date;
  if (meta.url && has('url')) item.url = meta.url;
  const venueField = venueFields(options.itemType).find(has);
  if (meta.venue && venueField) item[venueField] = meta.venue;
  const extra: string[] = [];
  if (meta.doi) {
    if (template ? has('DOI') : DOI_FIELD_TYPES.has(options.itemType)) item.DOI = meta.doi;
    else extra.push('DOI: ' + meta.doi);
  }
  if (meta.arxivId) {
    if (options.itemType === 'preprint' && has('archiveID')) item.archiveID = 'arXiv:' + meta.arxivId;
    if (options.itemType === 'preprint' && has('repository') && !item.repository) item.repository = 'arXiv';
    extra.push('arXiv: ' + meta.arxivId);
  }
  if (meta.pmid) extra.push('PMID: ' + meta.pmid);
  if (extra.length) item.extra = extra.join('\n');
  return item;
}

/** HTML-escape text for a note. */
export function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char] as string);
}

/** A child note listing the references as links. */
export function referencesNoteHtml(refs: Reference[], engine?: string): string {
  const items = refs.map(ref => {
    const label = ref.label ? escapeHtml(ref.label) + ' — ' : '';
    const page = ref.page !== undefined ? ' <span>(page ' + escapeHtml(String(ref.page)) + ')</span>' : '';
    const href = /^https?:\/\//i.test(ref.url) ? escapeHtml(ref.url) : '';
    const shown = escapeHtml(ref.doi ?? ref.url);
    return '<li>' + label + (href ? '<a href="' + href + '">' + shown + '</a>' : shown) + page + '</li>';
  });
  const source = engine ? '<p>Extracted by ' + escapeHtml(engine) + '.</p>' : '';
  return '<h1>References found in the text</h1>' + source + '<ol>' + items.join('') + '</ol>';
}

/** A child `linked_url` attachment (no file). */
export function linkedUrlAttachment(parentItem: string, url: string, title: string): Record<string, unknown> {
  return { itemType: 'attachment', linkMode: 'linked_url', parentItem, title, url, accessDate: '', note: '', contentType: '', charset: '', tags: [], relations: {} };
}

/** A child `imported_file` attachment whose bytes are uploaded afterwards. */
export function importedFileAttachment(parentItem: string, file: { title: string; filename: string; contentType: string; md5: string; mtimeMs: number }): Record<string, unknown> {
  return { itemType: 'attachment', linkMode: 'imported_file', parentItem, title: file.title, filename: file.filename, contentType: file.contentType, md5: file.md5, mtime: file.mtimeMs, charset: '', accessDate: '', note: '', tags: [], relations: {} };
}
