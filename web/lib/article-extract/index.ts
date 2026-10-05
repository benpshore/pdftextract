import type { Extracted } from '../types';
import { cleanDocument } from './clean';
import { cleanText, looksMojibake, normalizeSpace, parseHtml, repairHtml, textContent } from './dom';
import { collectMetadata, normalizeDate, stripSiteSuffix } from './metadata';
import { followPages, mergeArticles, nextPageUrl, type ChainOptions, type PageLoader } from './multipage';
import { collectHeadings, collectImages, collectLinks, collectTables, markdownConverter, plainText, sanitize } from './render';
import { scoreBlocks, selectContent } from './select';
import type { Article, ArticleLimits, ExtractOptions } from './types';
import { arxivFrom, doiFrom, pmcidFrom, pmidFrom, safeUrl, sameSite, textDois } from './url';

export const DEFAULT_LIMITS: ArticleLimits = { maxInputChars: 12 * 1024 * 1024, maxImages: 500, maxLinks: 5000, maxTables: 500, maxElements: 150000, maxPages: 10 };

export const STATIC_CAPTURE_WARNING = 'Captured the available HTML without running scripts. Content requiring client-side rendering or authentication may be absent.';
export const PAYWALL_WARNING = 'A paywall or subscriber gate was detected; only the publicly served part of the article was captured and no bypass was attempted.';

export const ENGINE = 'Mozilla Readability + block scorer + DOMPurify + GFM';

/**
 * Title precedence: explicit metadata (citation_title, JSON-LD headline, og:title), the page's own h1 when the
 * document title contains it, the document title without its site suffix, then the article scorer's guess.
 */
export function chooseTitle(fromMetadata: string | null, pageHeading: string, pageTitle: string, siteName: string | null, scoredTitle: string | null): string | null {
  const stripped = pageTitle ? stripSiteSuffix(pageTitle, siteName) : '';
  const heading = pageHeading && pageHeading.length <= 300 && (!siteName || pageHeading.toLowerCase() !== siteName.toLowerCase()) ? pageHeading : '';
  if (fromMetadata) return fromMetadata;
  if (heading && stripped.toLowerCase().includes(heading.toLowerCase())) return heading;
  return stripped || (scoredTitle ? normalizeSpace(cleanText(scoredTitle)) : '') || heading || null;
}

/** Extract one clean article from HTML. Pure: the same input always produces the same output. */
export function extractArticle(source: string, url: string, options: ExtractOptions = {}): Article {
  const limits = { ...DEFAULT_LIMITS, ...options.limits };
  const warnings: string[] = [];
  const truncated = source.length > limits.maxInputChars;
  const bounded = truncated ? source.slice(0, limits.maxInputChars) : source;
  if (truncated) warnings.push(`The HTML was cut at ${limits.maxInputChars} characters before parsing; the end of the page is missing.`);
  const repaired = repairHtml(bounded);
  const document = parseHtml(repaired.html);
  const base = safeUrl(document.querySelector('base[href]')?.getAttribute('href') || '', url) || url;
  const metadata = collectMetadata(document, base, url);
  const next = options.fragment ? null : nextPageUrl(document, base, url);
  if (!options.fragment) warnings.unshift(STATIC_CAPTURE_WARNING);
  if (repaired.repairs.length) warnings.push(`The HTML was incomplete or garbled: ${repaired.repairs.join(' ')}`);
  if (metadata.malformedStructuredData) warnings.push(`${metadata.malformedStructuredData} malformed structured metadata block(s) were omitted; their raw script content was not included.`);
  const pageHeading = normalizeSpace(textContent(document.querySelector('article h1,main h1,[role="main"] h1,h1')));
  cleanDocument(document, base, { fragment: options.fragment });
  const selection = selectContent(document, { fragment: options.fragment, maxElements: limits.maxElements });
  warnings.push(...selection.warnings);
  const visible = parseHtml(sanitize(selection.html));
  cleanDocument(visible, base, { fragment: options.fragment });
  const { images, dropped: droppedImages } = collectImages(visible, limits.maxImages);
  if (droppedImages) warnings.push(`${droppedImages} image(s) beyond the limit of ${limits.maxImages} were dropped.`);
  const headings = collectHeadings(visible);
  const { links, dropped: droppedLinks } = collectLinks(visible, limits.maxLinks);
  if (droppedLinks) warnings.push(`${droppedLinks} link(s) beyond the limit of ${limits.maxLinks} were not recorded.`);
  const { tables, dropped: droppedTables } = collectTables(visible, limits.maxTables);
  if (droppedTables) warnings.push(`${droppedTables} table(s) beyond the limit of ${limits.maxTables} were not recorded as grids.`);
  const text = plainText(visible);
  if (text.includes('\ufffd')) warnings.push('The decoded article contains replacement characters (U+FFFD); the original bytes must be checked for an encoding error.');
  if (looksMojibake(text)) warnings.push('The text looks like UTF-8 decoded with the wrong character set (mojibake); check the declared encoding of the original.');
  const html = visible.body.innerHTML;
  let markdown = markdownConverter().turndown(html);
  links.push(...textDois(text));
  if (metadata.scholarly?.doi && !links.some(link => link.doi === metadata.scholarly!.doi)) links.unshift({ url: `https://doi.org/${metadata.scholarly.doi}`, doi: metadata.scholarly.doi, kind: 'article DOI' });
  if (metadata.scholarly?.pmid && !links.some(link => link.url.includes(`pubmed.ncbi.nlm.nih.gov/${metadata.scholarly!.pmid}`))) links.push({ url: `https://pubmed.ncbi.nlm.nih.gov/${metadata.scholarly.pmid}/`, kind: 'article PMID', label: `PMID ${metadata.scholarly.pmid}` });
  if (metadata.paywall.detected) warnings.push(PAYWALL_WARNING);
  const title = cleanText(chooseTitle(metadata.title, pageHeading, metadata.pageTitle, metadata.siteName || selection.siteName, selection.title) || options.fallbackTitle || 'Saved page');
  if (!options.fragment && title && !new RegExp(`^#\\s+${title.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*$`, 'im').test(markdown.split('\n', 1)[0])) markdown = `# ${title.replace(/^#+\s*/, '')}\n\n${markdown}`.trim();
  const byline = metadata.byline || (selection.byline ? normalizeSpace(cleanText(selection.byline)) : null);
  const authors = metadata.authors.length ? metadata.authors : byline ? byline.replace(/^by\s+/i, '').split(/\s*,\s*|\s+and\s+|\s*&\s*/).map(name => normalizeSpace(name)).filter(name => name && name.length <= 80).map(name => ({ name })) : [];
  return {
    title,
    authors,
    byline,
    published: metadata.published,
    modified: metadata.modified,
    sourceUrl: url,
    canonicalUrl: metadata.canonicalUrl,
    siteName: metadata.siteName || selection.siteName || null,
    language: metadata.language || selection.lang || null,
    description: metadata.description || (selection.excerpt ? normalizeSpace(cleanText(selection.excerpt)) : null),
    html,
    markdown,
    text,
    headings,
    images,
    links,
    tables,
    feeds: metadata.feeds,
    scholarly: metadata.scholarly,
    structuredData: metadata.structuredData,
    meta: metadata.meta,
    amp: metadata.amp,
    paywall: metadata.paywall,
    nextPageUrl: next,
    pages: [url],
    selection: selection.mode,
    truncated,
    repairs: repaired.repairs,
    warnings,
  };
}

/** Extract an article and follow its rel="next" pages through the supplied loader. */
export async function extractArticleChain(source: string, url: string, load: PageLoader, options: ExtractOptions & ChainOptions = {}): Promise<Article> {
  const first = extractArticle(source, url, options);
  return followPages(first, load, (pageSource, pageUrl) => extractArticle(pageSource, pageUrl, options), { maxPages: options.maxPages ?? options.limits?.maxPages ?? DEFAULT_LIMITS.maxPages, deadlineMs: options.deadlineMs, signal: options.signal, now: options.now });
}

/** Project an article onto the workspace's generic extraction record. */
export function toExtracted(article: Article, capturedAt?: string): Extracted {
  const metadata: Record<string, unknown> = {
    sourceUrl: article.sourceUrl,
    capture: 'Fetched HTML snapshot; scripts were not executed',
    meta: article.meta,
    canonical: article.canonicalUrl,
    structuredData: article.structuredData,
    feeds: article.feeds,
    images: article.images,
    headings: article.headings,
    byline: article.byline,
    authors: article.authors,
    published: article.published,
    modified: article.modified,
    siteName: article.siteName,
    language: article.language,
    description: article.description,
    scholarly: article.scholarly,
    amp: article.amp,
    paywall: article.paywall,
    nextPage: article.nextPageUrl,
    pages: article.pages,
    selection: article.selection,
    truncated: article.truncated,
  };
  if (capturedAt) metadata.capturedAt = capturedAt;
  return { title: article.title, text: article.text, html: article.html, markdown: article.markdown, links: article.links, tables: article.tables, metadata, warnings: article.warnings, engine: ENGINE, status: 'partial' };
}

export { cleanDocument, followPages, mergeArticles, nextPageUrl, normalizeDate, parseHtml, repairHtml, looksMojibake, scoreBlocks, safeUrl, sameSite, doiFrom, pmidFrom, pmcidFrom, arxivFrom, textDois, textContent };
export type { Article, ArticleLimits, ExtractOptions, PageLoader, ChainOptions };
export type * from './types';
