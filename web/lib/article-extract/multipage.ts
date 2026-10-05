import { normalizeSpace, textContent } from './dom';
import { safeUrl, sameSite } from './url';
import type { Article } from './types';

const NEXT_TEXT = /^(?:next(?:\s+page)?|page\s+suivante|nächste(?:\s+seite)?|siguiente|próxima|weiter|continue to page \d+|›|»|→|>)\s*(?:›|»|→|>)?$/i;

/** The next page of the same article on the same site, or null when the page does not point to one. */
export function nextPageUrl(document: Document, base: string, currentUrl: string): string | null {
  const current = (() => { try { return new URL(currentUrl); } catch { return null; } })();
  if (!current) return null;
  const candidates: Element[] = [
    ...Array.from(document.querySelectorAll('link[rel~="next"][href]')),
    ...Array.from(document.querySelectorAll('a[rel~="next"][href]')),
    ...Array.from(document.querySelectorAll('a.next[href],a.next-page[href],a.pagination-next[href],a[aria-label="Next" i][href],a[aria-label="Next page" i][href],.pagination a[href],.pager a[href],nav[aria-label*="pagination" i] a[href],.page-numbers a[href]')).filter(anchor => anchor.matches('a.next,a.next-page,a.pagination-next,[aria-label]') || NEXT_TEXT.test(normalizeSpace(textContent(anchor)))),
  ];
  for (const candidate of candidates) {
    const url = safeUrl(candidate.getAttribute('href') || '', base);
    if (!url) continue;
    const target = new URL(url);
    target.hash = '';
    const here = new URL(current.href);
    here.hash = '';
    if (target.href === here.href || !sameSite(target.hostname, here.hostname)) continue;
    return target.href;
  }
  return null;
}

export type PageLoader = (url: string) => Promise<string>;
export type ChainOptions = { maxPages?: number; deadlineMs?: number; signal?: AbortSignal; now?: () => number };

function renumber(html: string, offset: number, headingOffset: number) {
  let images = offset;
  return html.replace(/data-image-id="image-\d+"/g, () => `data-image-id="image-${++images}"`).replace(/ id="section-(\d+)"/g, (_match, index) => ` id="section-${Number(index) + headingOffset}"`);
}

/** Append the article content of a following page to a merged article; the first page's metadata is authoritative. */
export function mergeArticles(first: Article, next: Article, nextUrl: string): Article {
  const titleLine = normalizeSpace(first.title).toLowerCase();
  let nextHtml = next.html, nextMarkdown = next.markdown, nextText = next.text;
  const repeated = new RegExp(`^\\s*<h1[^>]*>\\s*${titleLine.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replace(/\s+/g, '\\s+')}\\s*</h1>`, 'i');
  if (repeated.test(nextHtml.toLowerCase())) nextHtml = nextHtml.replace(/^\s*<h1[^>]*>[\s\S]*?<\/h1>/i, '');
  if (nextMarkdown.toLowerCase().startsWith(`# ${titleLine}`)) nextMarkdown = nextMarkdown.replace(/^# [^\n]*\n+/, '');
  if (nextText.toLowerCase().startsWith(titleLine)) nextText = nextText.slice(first.title.length).trimStart();
  const imageOffset = first.images.length, headingOffset = first.headings.length;
  const html = `${first.html}\n<section data-article-page="${first.pages.length + 1}">\n${renumber(nextHtml, imageOffset, headingOffset)}\n</section>`;
  const images = [...first.images, ...next.images.map((image, index) => ({ ...image, id: `image-${imageOffset + index + 1}` }))];
  const headings = [...first.headings, ...next.headings.map(heading => ({ ...heading, id: heading.id.replace(/^section-(\d+)$/, (_match, index) => `section-${Number(index) + headingOffset}`) }))];
  const seen = new Set(first.links.map(link => `${link.url}\u0000${link.label || ''}`));
  const links = [...first.links, ...next.links.filter(link => { const key = `${link.url}\u0000${link.label || ''}`; if (seen.has(key)) return false; seen.add(key); return true; })];
  return {
    ...first,
    html,
    markdown: `${first.markdown}\n\n${nextMarkdown}`.trim(),
    text: `${first.text}\n\n${nextText}`.trim(),
    images,
    headings,
    links,
    tables: [...first.tables, ...next.tables],
    pages: [...first.pages, nextUrl],
    nextPageUrl: next.nextPageUrl,
    truncated: first.truncated || next.truncated,
    repairs: [...first.repairs, ...next.repairs.map(repair => `Page ${first.pages.length + 1}: ${repair}`)],
    warnings: Array.from(new Set([...first.warnings, ...next.warnings.filter(warning => !/^Captured the available HTML/.test(warning))])),
    paywall: first.paywall.detected || !next.paywall.detected ? first.paywall : next.paywall,
  };
}

/**
 * Follow rel="next" links on the same site, merging each page into one article.
 * Pages are loaded through the supplied loader so the caller decides how a URL is fetched.
 */
export async function followPages(first: Article, load: PageLoader, extract: (source: string, url: string) => Article, options: ChainOptions = {}): Promise<Article> {
  const maxPages = Math.max(1, options.maxPages ?? 10);
  const now = options.now || (() => Date.now());
  const deadline = now() + (options.deadlineMs ?? 20000);
  let article = first;
  const visited = new Set(first.pages.map(page => page.replace(/#.*$/, '')));
  const seenText = new Set([normalizeSpace(first.text).slice(0, 2000)]);
  while (article.nextPageUrl && article.pages.length < maxPages) {
    const url = article.nextPageUrl;
    if (visited.has(url.replace(/#.*$/, ''))) { article = { ...article, warnings: [...article.warnings, `Pagination loops back to ${url}; following stopped.`], nextPageUrl: null }; break; }
    visited.add(url.replace(/#.*$/, ''));
    if (now() > deadline) { article = { ...article, warnings: [...article.warnings, `Stopped before page ${article.pages.length + 1} (${url}): the time budget was used up.`] }; break; }
    options.signal?.throwIfAborted();
    let source: string;
    try { source = await load(url); } catch (error) {
      article = { ...article, warnings: [...article.warnings, `Page ${article.pages.length + 1} (${url}) could not be loaded: ${error instanceof Error ? error.message : 'error'}.`] };
      break;
    }
    const next = extract(source, url);
    const fingerprint = normalizeSpace(next.text).slice(0, 2000);
    if (!fingerprint || seenText.has(fingerprint)) { article = { ...article, warnings: [...article.warnings, `Page ${article.pages.length + 1} (${url}) repeated earlier content and was not appended.`], nextPageUrl: null }; break; }
    seenText.add(fingerprint);
    article = mergeArticles(article, next, url);
  }
  if (article.nextPageUrl && article.pages.length >= maxPages) article = { ...article, warnings: [...article.warnings, `The article continues at ${article.nextPageUrl}; the page limit of ${maxPages} was reached.`] };
  return article;
}
