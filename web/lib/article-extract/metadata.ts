import { cleanText, normalizeSpace, textContent } from './dom';
import { arxivFrom, doiFrom, pmcidFrom, pmidFrom, safeUrl } from './url';
import type { AmpInfo, ArticleAuthor, ArticleFeed, PaywallInfo, ScholarlyMetadata } from './types';

export type PageMetadata = {
  meta: Record<string, string>;
  title: string | null;
  pageTitle: string;
  authors: ArticleAuthor[];
  byline: string | null;
  published: string | null;
  modified: string | null;
  canonicalUrl: string | null;
  siteName: string | null;
  language: string | null;
  description: string | null;
  scholarly: ScholarlyMetadata | null;
  structuredData: Record<string, unknown>[];
  malformedStructuredData: number;
  feeds: ArticleFeed[];
  amp: AmpInfo;
  paywall: PaywallInfo;
};

const MONTHS: Record<string, string> = { jan: '01', feb: '02', mar: '03', apr: '04', may: '05', jun: '06', jul: '07', aug: '08', sep: '09', sept: '09', oct: '10', nov: '11', dec: '12' };
const pad = (value: string) => value.padStart(2, '0');

/** Normalise a date string to ISO 8601 without applying the local time zone; unparseable input is kept verbatim. */
export function normalizeDate(raw: string | null | undefined): string | null {
  const value = normalizeSpace(cleanText(raw || ''));
  if (!value) return null;
  let m = value.match(/^(\d{4})-(\d{1,2})-(\d{1,2})(?:[T ](\d{1,2}):(\d{2})(?::(\d{2}))?(?:\.\d+)?\s*(Z|[+-]\d{2}:?\d{2}|UTC|GMT)?)?$/i);
  if (m) {
    const date = `${m[1]}-${pad(m[2])}-${pad(m[3])}`;
    if (!m[4]) return date;
    const zone = !m[7] ? '' : /^(Z|UTC|GMT)$/i.test(m[7]) ? 'Z' : m[7].length === 5 ? `${m[7].slice(0, 3)}:${m[7].slice(3)}` : m[7];
    return `${date}T${pad(m[4])}:${m[5]}:${m[6] || '00'}${zone}`;
  }
  m = value.match(/^(\d{4})\/(\d{1,2})\/(\d{1,2})$/);
  if (m) return `${m[1]}-${pad(m[2])}-${pad(m[3])}`;
  const TIME = /(?:\s+(?:at\s+)?(\d{1,2}):(\d{2})(?::(\d{2}))?\s*(Z|GMT|UTC|[+-]\d{2}:?\d{2})?)?/.source;
  const withTime = (date: string, hour?: string, minute?: string, second?: string, zone?: string) => {
    if (!hour) return date;
    const suffix = !zone ? '' : /^(Z|UTC|GMT)$/i.test(zone) ? 'Z' : zone.length === 5 ? `${zone.slice(0, 3)}:${zone.slice(3)}` : zone;
    return `${date}T${pad(hour)}:${minute}:${second || '00'}${suffix}`;
  };
  m = value.match(new RegExp(`^(?:[A-Za-z]+,?\\s+)?([A-Za-z]{3,9})\\.?\\s+(\\d{1,2})(?:st|nd|rd|th)?,?\\s+(\\d{4})${TIME}`));
  if (m && MONTHS[m[1].slice(0, 3).toLowerCase()]) return withTime(`${m[3]}-${MONTHS[m[1].slice(0, 3).toLowerCase()]}-${pad(m[2])}`, m[4], m[5], m[6], m[7]);
  m = value.match(new RegExp(`^(?:[A-Za-z]+,?\\s+)?(\\d{1,2})(?:st|nd|rd|th)?\\.?\\s+([A-Za-z]{3,9})\\.?,?\\s+(\\d{4})${TIME}`));
  if (m && MONTHS[m[2].slice(0, 3).toLowerCase()]) return withTime(`${m[3]}-${MONTHS[m[2].slice(0, 3).toLowerCase()]}-${pad(m[1])}`, m[4], m[5], m[6], m[7]);
  m = value.match(/^(\d{4})$/);
  if (m) return m[1];
  if (/\b(?:GMT|UTC|Z)\b|[+-]\d{2}:?\d{2}$/.test(value)) {
    const parsed = Date.parse(value);
    if (Number.isFinite(parsed)) return new Date(parsed).toISOString().replace(/\.000Z$/, 'Z');
  }
  return value;
}

function cleanName(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  const name = normalizeSpace(cleanText(value)).replace(/^(?:by|von|par|de)\s+/i, '').replace(/[,\s]+$/, '');
  if (!name || name.length > 120 || /^https?:\/\//i.test(name) || /@/.test(name)) return null;
  return name;
}

function splitAuthors(value: string): string[] {
  const parts = value.includes(';') ? value.split(';') : /\band\b|&/.test(value) && !/,\s*[A-Z]\.?\s*$/.test(value) ? value.split(/\s*,\s*|\s+and\s+|\s*&\s*/) : [value];
  return parts.map(cleanName).filter((name): name is string => Boolean(name));
}

/** "Okonkwo, Mira" and "Mira Okonkwo" are the same person: compare sorted lower-case name tokens. */
const authorKey = (name: string) => name.toLowerCase().replace(/\./g, '').split(/[\s,]+/).filter(Boolean).sort().join(' ');

function addAuthors(target: ArticleAuthor[], candidates: ArticleAuthor[]) {
  for (const candidate of candidates) {
    const key = authorKey(candidate.name);
    const existing = target.find(author => authorKey(author.name) === key);
    if (!existing) target.push(candidate);
    else { existing.url ||= candidate.url; existing.affiliation ||= candidate.affiliation; }
  }
}

function stripSiteSuffix(title: string, siteName: string | null) {
  const value = normalizeSpace(title);
  if (siteName) {
    const site = normalizeSpace(siteName);
    for (const separator of [' | ', ' - ', ' – ', ' — ', ' :: ', ' » ', ' · ']) {
      if (value.endsWith(`${separator}${site}`)) return value.slice(0, -separator.length - site.length).trim();
      if (value.startsWith(`${site}${separator}`)) return value.slice(site.length + separator.length).trim();
    }
  }
  const match = value.match(/^(.{15,}?)\s+(?:\||–|—|-|::|»|·)\s+[^|–—:»·-]{2,40}$/);
  return match ? match[1] : value;
}

type Jsonld = Record<string, unknown>;
const asArray = (value: unknown): unknown[] => Array.isArray(value) ? value : value === undefined || value === null ? [] : [value];
const asString = (value: unknown): string | null => typeof value === 'string' ? normalizeSpace(cleanText(value)) || null : typeof value === 'number' ? String(value) : null;

function jsonldPeople(value: unknown): ArticleAuthor[] {
  return asArray(value).flatMap(person => {
    if (typeof person === 'string') return splitAuthors(person).map(name => ({ name }));
    if (!person || typeof person !== 'object') return [];
    const item = person as Jsonld;
    const name = cleanName(asString(item.name) || [asString(item.givenName), asString(item.familyName)].filter(Boolean).join(' '));
    if (!name) return [];
    const author: ArticleAuthor = { name };
    const url = asString(item.url) || asString(item['@id']);
    if (url && safeUrl(url)) author.url = safeUrl(url)!;
    const affiliation = asArray(item.affiliation).map(part => typeof part === 'string' ? part : asString((part as Jsonld)?.name)).filter(Boolean)[0];
    if (affiliation) author.affiliation = normalizeSpace(cleanText(affiliation as string));
    return [author];
  });
}

function structuredArticles(document: Document, base: string) {
  const articles: Jsonld[] = [];
  const raw: Jsonld[] = [];
  const pending: unknown[] = [];
  let malformed = 0;
  for (const node of Array.from(document.querySelectorAll('script[type="application/ld+json"]'))) {
    try { pending.push(JSON.parse(node.textContent || '')); } catch { malformed++; }
  }
  while (pending.length) {
    const value = pending.shift();
    if (Array.isArray(value)) { pending.push(...value); continue; }
    if (!value || typeof value !== 'object') continue;
    const item = value as Jsonld;
    if (item['@graph']) pending.push(item['@graph']);
    if (item.mainEntity && typeof item.mainEntity === 'object') pending.push(item.mainEntity);
    const types = asArray(item['@type']).filter((type): type is string => typeof type === 'string');
    if (!types.some(type => /Article|Posting|Review|Report|Book$|^Thesis$|^Dataset$|^WebPage$/.test(type))) continue;
    raw.push(item);
    const projection: Jsonld = { type: item['@type'] };
    for (const key of ['headline', 'name', 'description', 'datePublished', 'dateModified', 'articleSection', 'keywords', 'inLanguage', 'identifier', 'isbn', 'issn', 'doi', 'pageStart', 'pageEnd', 'volumeNumber', 'issueNumber', 'isAccessibleForFree']) {
      const member = item[key];
      if (typeof member === 'string') projection[key] = cleanText(member);
      else if (typeof member === 'number' || typeof member === 'boolean') projection[key] = member;
      else if (Array.isArray(member) && member.every(part => typeof part === 'string')) projection[key] = member.map(cleanText);
    }
    for (const key of ['url', 'license']) if (typeof item[key] === 'string') projection[key] = safeUrl(item[key] as string, base);
    for (const key of ['author', 'publisher']) if (item[key]) projection[key] = jsonldPeople(item[key]).map(person => person.name);
    articles.push(projection);
  }
  return { articles, raw, malformed };
}

function readMeta(document: Document) {
  const meta: Record<string, string> = {};
  const all: Record<string, string[]> = {};
  for (const element of Array.from(document.querySelectorAll('meta[name],meta[property],meta[itemprop],meta[http-equiv]'))) {
    const key = (element.getAttribute('name') || element.getAttribute('property') || element.getAttribute('itemprop') || element.getAttribute('http-equiv') || '').trim();
    const value = normalizeSpace(cleanText(element.getAttribute('content') || ''));
    if (!key || !value) continue;
    if (!(key in meta)) meta[key] = value;
    const lower = key.toLowerCase();
    (all[lower] ||= []).push(value);
  }
  const get = (...keys: string[]) => { for (const key of keys) { const found = all[key.toLowerCase()]; if (found?.length) return found[0]; } return null; };
  const list = (...keys: string[]) => keys.flatMap(key => all[key.toLowerCase()] || []);
  return { meta, get, list };
}

export function collectMetadata(document: Document, base: string, sourceUrl: string): PageMetadata {
  const { meta, get, list } = readMeta(document);
  const structured = structuredArticles(document, base);
  const ld = structured.raw.find(item => asArray(item['@type']).some(type => /Article|Posting|Review|Report|Thesis/.test(String(type)))) || structured.raw[0] || null;
  const html = document.documentElement;
  const amp: AmpInfo = {
    isAmp: Boolean(html && (html.hasAttribute('amp') || html.hasAttribute('⚡') || html.hasAttribute('⚡4ads') || html.hasAttribute('amp4ads'))),
    ampUrl: safeUrl(document.querySelector('link[rel~="amphtml"]')?.getAttribute('href') || '', base),
  };
  const canonicalUrl = safeUrl(document.querySelector('link[rel~="canonical"]')?.getAttribute('href') || '', base)
    || safeUrl(get('og:url') || '', base)
    || (ld ? safeUrl(asString(ld.url) || asString(typeof ld.mainEntityOfPage === 'object' && ld.mainEntityOfPage ? (ld.mainEntityOfPage as Jsonld)['@id'] : ld.mainEntityOfPage) || '', base) : null);
  const siteName = get('og:site_name') || (ld ? jsonldPeople(ld.publisher)[0]?.name || null : null) || get('application-name') || get('citation_publisher') || null;
  const pageTitle = textContent(document.querySelector('title'));
  const title = cleanName(get('citation_title')) || (ld ? cleanName(asString(ld.headline)) : null) || (get('og:title') ? stripSiteSuffix(get('og:title')!, siteName) : null) || cleanName(get('dc.title', 'dcterms.title', 'twitter:title')) || null;

  const authors: ArticleAuthor[] = [];
  addAuthors(authors, list('citation_author').flatMap(value => splitAuthors(value).map(name => ({ name }))));
  if (ld) addAuthors(authors, jsonldPeople(ld.author).concat(jsonldPeople(ld.creator)));
  addAuthors(authors, list('author', 'article:author', 'dc.creator', 'dcterms.creator', 'parsely-author', 'sailthru.author', 'twitter:creator').flatMap(value => /^https?:\/\//i.test(value) || /^@/.test(value) ? [] : splitAuthors(value).map(name => ({ name }))));
  if (!authors.length) {
    for (const node of Array.from(document.querySelectorAll('[rel~="author"],[itemprop~="author"],.byline [itemprop~="name"],.author-name,.byline__name,.byline a,.author a'))) {
      if (node.closest('nav,footer,aside,[role="navigation"]')) continue;
      const name = cleanName(textContent(node.querySelector('[itemprop~="name"]') || node));
      if (name && name.length <= 80) addAuthors(authors, [{ name }]);
      if (authors.length >= 8) break;
    }
  }
  const bylineNode = document.querySelector('.byline,[class~="byline"],[rel~="author"],[itemprop~="author"]');
  const bylineText = bylineNode && !bylineNode.closest('nav,footer,aside') ? normalizeSpace(textContent(bylineNode)) : '';
  const byline = authors.length ? `By ${authors.map(author => author.name).join(', ')}` : bylineText && bylineText.length <= 160 ? bylineText : null;

  const timeAttr = (selector: string) => { const node = document.querySelector(selector); return node?.getAttribute('datetime') || node?.getAttribute('content') || textContent(node) || null; };
  const published = normalizeDate(get('citation_publication_date', 'citation_date', 'citation_online_date', 'article:published_time', 'og:article:published_time') || (ld ? asString(ld.datePublished) : null) || get('dc.date', 'dcterms.issued', 'dcterms.created', 'dc.date.issued', 'dc.date.created', 'date', 'pubdate', 'publish-date', 'publication_date', 'sailthru.date', 'parsely-pub-date', 'datePublished') || timeAttr('time[pubdate],time[itemprop~="datePublished"],[itemprop~="datePublished"]') || timeAttr('article time[datetime],main time[datetime],.byline time[datetime],time[datetime]'));
  const modified = normalizeDate(get('article:modified_time', 'og:updated_time', 'dcterms.modified', 'dc.date.modified', 'last-modified', 'dateModified', 'revised') || (ld ? asString(ld.dateModified) : null) || timeAttr('time[itemprop~="dateModified"],[itemprop~="dateModified"]'));
  const language = (html?.getAttribute('lang') || get('citation_language', 'dc.language', 'dcterms.language', 'language') || (get('og:locale') || '').replace(/_/g, '-') || (ld ? asString(ld.inLanguage) : null) || get('content-language') || '').trim() || null;
  const description = cleanName(get('description', 'og:description', 'dc.description', 'dcterms.description', 'twitter:description')) || (ld ? cleanName(asString(ld.description)) : null) || null;

  const scholarly: ScholarlyMetadata = {};
  const identifiers = [
    ...list('citation_doi', 'dc.identifier', 'dcterms.identifier', 'dc.identifier.doi', 'prism.doi', 'citation_pmid', 'citation_pmcid', 'citation_arxiv_id'),
    ...(ld ? asArray(ld.identifier).map(value => typeof value === 'string' ? value : asString((value as Jsonld)?.value) || '') : []),
    ...(ld ? asArray(ld.sameAs).filter((value): value is string => typeof value === 'string') : []),
    canonicalUrl || '',
    sourceUrl,
  ];
  for (const value of identifiers) {
    if (!value) continue;
    scholarly.doi ||= doiFrom(value);
    scholarly.pmid ||= pmidFrom(value);
    scholarly.pmcid ||= pmcidFrom(value);
    scholarly.arxiv ||= arxivFrom(value);
  }
  if (!scholarly.doi) {
    const anchor = Array.from(document.querySelectorAll('a[href*="doi.org/10."],[data-doi]')).find(node => node.hasAttribute('data-doi') || node.closest('.doi,[class*="doi"],header,.article-header,[class*="article-meta"],[class*="citation"]'));
    const doi = anchor ? doiFrom(anchor.getAttribute('data-doi') || anchor.getAttribute('href') || '') : undefined;
    if (doi) scholarly.doi = doi;
  }
  if (!scholarly.pmid) { const link = Array.from(document.querySelectorAll('a[href*="pubmed"]')).map(node => pmidFrom(node.getAttribute('href') || '')).find(Boolean); if (link) scholarly.pmid = link; }
  if (!scholarly.pmcid) { const link = Array.from(document.querySelectorAll('a[href*="/articles/PMC"],a[href*="/pmc/articles/"]')).map(node => pmcidFrom(node.getAttribute('href') || '')).find(Boolean); if (link) scholarly.pmcid = link; }
  const periodical = ld && ld.isPartOf && typeof ld.isPartOf === 'object' ? ld.isPartOf as Jsonld : null;
  const issue = periodical && periodical.isPartOf && typeof periodical.isPartOf === 'object' ? periodical.isPartOf as Jsonld : null;
  const volume = issue && issue.isPartOf && typeof issue.isPartOf === 'object' ? issue.isPartOf as Jsonld : null;
  const periodicalName = asString(periodical?.name) || asString(issue?.name) || asString(volume?.name) || null;
  const pick = (value: string | null | undefined) => value ? normalizeSpace(cleanText(value)) : undefined;
  Object.assign(scholarly, {
    journal: pick(get('citation_journal_title', 'citation_conference_title', 'prism.publicationName', 'dc.source', 'dcterms.source') || periodicalName),
    volume: pick(get('citation_volume', 'prism.volume') || (ld ? asString(ld.volumeNumber) : null) || asString(volume?.volumeNumber) || asString(periodical?.volumeNumber)),
    issue: pick(get('citation_issue', 'prism.number') || (ld ? asString(ld.issueNumber) : null) || asString(issue?.issueNumber) || asString(periodical?.issueNumber)),
    firstPage: pick(get('citation_firstpage', 'prism.startingPage') || (ld ? asString(ld.pageStart) : null)),
    lastPage: pick(get('citation_lastpage', 'prism.endingPage') || (ld ? asString(ld.pageEnd) : null)),
    publisher: pick(get('citation_publisher', 'dc.publisher', 'dcterms.publisher') || (ld ? jsonldPeople(ld.publisher)[0]?.name : null)),
    issn: pick(get('citation_issn', 'prism.issn') || (ld ? asString(ld.issn) : null) || asString(periodical?.issn)),
    isbn: pick(get('citation_isbn') || (ld ? asString(ld.isbn) : null)),
    pdfUrl: safeUrl(get('citation_pdf_url') || '', base) || undefined,
    abstract: pick(get('citation_abstract', 'dc.description.abstract', 'dcterms.abstract')),
  });
  const keywords = Array.from(new Set([...list('citation_keywords', 'keywords', 'dc.subject', 'dcterms.subject', 'article:tag', 'news_keywords'), ...(ld ? asArray(ld.keywords).filter((value): value is string => typeof value === 'string') : [])].flatMap(value => value.split(/[;,]/)).map(value => normalizeSpace(cleanText(value))).filter(Boolean))).slice(0, 50);
  if (keywords.length) scholarly.keywords = keywords;
  if (ld) scholarly.type = asArray(ld['@type']).filter((type): type is string => typeof type === 'string').join(', ') || undefined;
  else if (get('citation_title')) scholarly.type = 'citation metadata';
  for (const key of Object.keys(scholarly) as (keyof ScholarlyMetadata)[]) if (scholarly[key] === undefined) delete scholarly[key];
  const hasScholarly = Boolean(scholarly.doi || scholarly.pmid || scholarly.pmcid || scholarly.arxiv || scholarly.journal || get('citation_title') || (scholarly.type && /Scholarly|Thesis|Report/.test(scholarly.type)));

  const paywall: PaywallInfo = { detected: false, evidence: [] };
  const free = (value: unknown) => value === false || (typeof value === 'string' && /^false$/i.test(value));
  if (ld && free(ld.isAccessibleForFree)) paywall.evidence.push('Structured data declares isAccessibleForFree=false.');
  if (ld && asArray(ld.hasPart).some(part => part && typeof part === 'object' && free((part as Jsonld).isAccessibleForFree))) paywall.evidence.push('Structured data marks part of the article as not freely accessible.');
  const tier = get('article:content_tier', 'content_tier');
  if (tier && /locked|metered|premium|subscriber/i.test(tier)) paywall.evidence.push(`The page declares content tier "${tier}".`);
  if (document.querySelector('[amp-access]:not([amp-access^="NOT "]):not([amp-access="TRUE"]),[subscriptions-section="content"],[subscriptions-action],[subscriptions-display]')) paywall.evidence.push('Sections are gated by amp-access or amp-subscriptions.');
  if (document.querySelector('.paywall,#paywall,[class*="paywall"],[id*="paywall"],[class*="regwall"],[id*="regwall"],.meteredContent,[class*="metered-"],.tp-modal,.piano-offer,[data-paywall],[class*="subscription-wall"],[class*="subscribe-wall"]')) paywall.evidence.push('A paywall or registration prompt is present in the page.');
  paywall.detected = paywall.evidence.length > 0;

  const feeds = Array.from(document.querySelectorAll('link[rel~="alternate"]')).filter(element => /rss|atom|feed\+json|json/i.test(element.getAttribute('type') || '')).map(element => ({ type: element.getAttribute('type'), url: safeUrl(element.getAttribute('href') || '', base) })).filter((feed): feed is ArticleFeed => Boolean(feed.url));

  return { meta, title, pageTitle, authors, byline, published, modified, canonicalUrl, siteName: siteName ? normalizeSpace(siteName) : null, language, description, scholarly: hasScholarly ? scholarly : null, structuredData: structured.articles, malformedStructuredData: structured.malformed, feeds, amp, paywall };
}

export { stripSiteSuffix };
