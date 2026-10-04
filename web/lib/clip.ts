import { Readability } from '@mozilla/readability';
import DOMPurify from 'dompurify';
import TurndownService from 'turndown';
import { gfm } from 'turndown-plugin-gfm';
import type { Extracted, LinkEvidence } from './types';
import { passiveHtmlSnapshot } from './passive-html';

export function safeUrl(value: string, base?: string) {
  if (!value.trim()) return null;
  try {
    const u = new URL(value, base);
    return ['http:', 'https:'].includes(u.protocol) && !u.username && !u.password ? u.href : null;
  } catch { return null; }
}
export function doiFrom(value: string) {
  let decoded = value;
  try { decoded = decodeURIComponent(value); } catch { /* Preserve malformed percent encoding. */ }
  return decoded.match(/10\.\d{4,9}\/[^\s<>"?#]+/i)?.[0].replace(/[.,;]+$/, '');
}
export function textDois(text: string): LinkEvidence[] {
  return Array.from(new Set(text.match(/10\.\d{4,9}\/[^\s<>"?#]+/gi) || [])).map(value => {
    const doi = value.replace(/[.,;]+$/, '');
    return { url: `https://doi.org/${doi}`, doi, kind: 'printed DOI' };
  });
}

const cleanText = (value: string) => value.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/g, '');
const escapeHtml = (value: string) => value.replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
const textContent = (element: Element | null | undefined) => cleanText(element?.textContent || '').trim();

/** Article selection follows the passive boundary; acquisition has already gone.
 * Keep editorial cleanup here: page chrome, text controls and navigation URLs.
 * Do not restore noscript/lazy images: that former path contradicted local-only
 * intake and could confuse a future maintainer into reopening resource loading.
 */
function cleanDocument(document: Document, base: string, fragment = false) {
  for (const node of Array.from(document.querySelectorAll('script,template,canvas'))) node.remove();
  if (!fragment) {
    for (const node of Array.from(document.querySelectorAll('nav,[role="navigation"],[role="banner"],[role="complementary"],aside,footer'))) {
      if (!node.matches('[role="doc-footnote"],[role="doc-endnote"]') && !node.querySelector('[role="doc-footnote"],[role="doc-endnote"]')) node.remove();
    }
  for (const node of Array.from(document.querySelectorAll('header'))) {
    if (!node.closest('article,main,[role="main"]')) node.remove();
  }
  // Explicit chrome markers only: broad substring rules can delete real article sections.
  for (const node of Array.from(document.querySelectorAll('[role="dialog"],[aria-modal="true"],[data-ad-slot],[data-ad-unit],.advertisement,.ad-container,.cookie-banner,.cookie-consent,.newsletter-signup,.social-share,.related-articles,.related-posts'))) node.remove();
  }
  const textNodes = document.createTreeWalker(document.body, 4); // NodeFilter.SHOW_TEXT
  while (textNodes.nextNode()) textNodes.currentNode.nodeValue = cleanText(textNodes.currentNode.nodeValue || '');
  for (const anchor of Array.from(document.querySelectorAll('a[href]'))) {
    const href = safeUrl(anchor.getAttribute('href') || '', base);
    if (href) anchor.setAttribute('href', href); else anchor.removeAttribute('href');
  }
}

function structuredMetadata(document: Document, base: string) {
  const articles: Record<string, unknown>[] = [];
  const pending: unknown[] = [];
  let malformed = 0;
  for (const node of Array.from(document.querySelectorAll('script[type="application/ld+json"]'))) {
    try { pending.push(JSON.parse(node.textContent || '')); } catch { malformed++; }
  }
  while (pending.length) {
    const value = pending.pop();
    if (Array.isArray(value)) { for (const member of value) pending.push(member); continue; }
    if (!value || typeof value !== 'object') continue;
    const item = value as Record<string, unknown>;
    if (item['@graph']) pending.push(item['@graph']);
    const types = Array.isArray(item['@type']) ? item['@type'] : [item['@type']];
    if (!types.some(type => typeof type === 'string' && /Article|Posting|Review|Book$/.test(type))) continue;
    const projection: Record<string, unknown> = { type: item['@type'] };
    for (const key of ['headline', 'name', 'description', 'datePublished', 'dateModified', 'articleSection', 'keywords', 'inLanguage', 'identifier', 'isbn', 'issn', 'doi']) {
      const value = item[key];
      if (typeof value === 'string') projection[key] = cleanText(value);
      else if (Array.isArray(value) && value.every(part => typeof part === 'string')) projection[key] = value.map(cleanText);
    }
    for (const key of ['url', 'license']) {
      if (typeof item[key] === 'string') projection[key] = safeUrl(item[key] as string, base);
    }
    for (const key of ['author', 'publisher']) {
      if (item[key]) projection[key] = (Array.isArray(item[key]) ? item[key] : [item[key]] as unknown[]).map((person: unknown) => {
        if (typeof person === 'string') return cleanText(person);
        if (person && typeof person === 'object' && 'name' in person && typeof person.name === 'string') return cleanText(person.name);
        return null;
      }).filter(Boolean);
    }
    articles.push(projection);
  }
  return { articles, malformed };
}

function mainBoundary(document: Document): Element | undefined {
  // Prefer an explicit publishing boundary before the broader page main region.
  for (const selector of ['.mw-parser-output', 'article', 'main,[role="main"]']) {
    const candidates = Array.from(document.querySelectorAll(selector));
    if (candidates.length) return candidates.sort((a, b) => textContent(b).length - textContent(a).length)[0];
  }
  return undefined;
}

function htmlText(document: Document): string {
  const tree = document.body.cloneNode(true) as HTMLElement;
  for (const cell of Array.from(tree.querySelectorAll('th,td'))) cell.appendChild(document.createTextNode('\t'));
  for (const element of Array.from(tree.querySelectorAll('p,div,section,article,h1,h2,h3,h4,h5,h6,li,tr,blockquote,pre,figure,figcaption,br'))) element.parentNode?.insertBefore(document.createTextNode('\n'), element.nextSibling);
  return cleanText(tree.textContent || '').replace(/[ \t]+\n/g, '\n').replace(/\n{3,}/g, '\n\n').trim();
}

export function clipHtml(source: string, url: string, title = 'Saved page', options: { fragment?: boolean } = {}): Extracted {
  // One inert boundary retains provenance before removing acquisition-bearing
  // structure. Readability sees sanitized content with the original root lang;
  // canonical/feed strings remain evidence and are resolved only as URLs below.
  const { document, canonicalHref, feeds } = passiveHtmlSnapshot(source);
  const base = safeUrl(document.querySelector('base[href]')?.getAttribute('href') || '', url) || url;
  const structured = structuredMetadata(document, base);
  const metadata: Record<string, unknown> = { sourceUrl: url, capturedAt: new Date().toISOString(), capture: 'Local HTML snapshot; external resources disabled',remoteResources:'disabled' };
  const metas: Record<string, string> = {};
  for (const element of Array.from(document.querySelectorAll('meta[name],meta[property]'))) {
    const key = element.getAttribute('name') || element.getAttribute('property') || '';
    metas[key] = cleanText(element.getAttribute('content') || '');
  }
  metadata.meta = metas;
  metadata.canonical = safeUrl(canonicalHref, base);
  metadata.structuredData = structured.articles;
  metadata.feeds = feeds.map(feed => ({ type: feed.type, url: safeUrl(feed.href, base) })).filter(feed => feed.url);
  const pageTitle = textContent(document.querySelector('title'));
  cleanDocument(document, base, options.fragment);
  const warnings = ['Captured the available HTML without running scripts. Content requiring client-side rendering or authentication may be absent.'];
  if (structured.malformed) warnings.push(`${structured.malformed} malformed structured metadata block(s) were omitted; their raw script content was not included.`);
  const article = options.fragment ? null : new Readability(document.cloneNode(true) as Document, { keepClasses: false, charThreshold: 100 }).parse();
  let selected = options.fragment ? document.body.innerHTML : article?.content || '';
  if (!selected && !options.fragment) {
    const boundary = mainBoundary(document);
    selected = boundary?.innerHTML || document.body.innerHTML;
    warnings.push(boundary ? 'Article scoring was inconclusive; the cleaned main section was retained.' : 'No main article boundary was identified; cleaned body content was retained and may include page chrome.');
  }
  if (article && !options.fragment) {
    const boundary = mainBoundary(document);
    if (boundary) {
      const candidate = new DOMParser().parseFromString(selected, 'text/html');
      const normalize = (value: string) => value.replace(/\s+/g, ' ').trim();
      const candidateText = normalize(textContent(candidate.body));
      const missing = Array.from(boundary.querySelectorAll('table,pre,[role="doc-footnote"],[role="doc-endnote"]')).filter(element => {
        const value = normalize(textContent(element));
        return value && !candidateText.includes(value);
      });
      // Reference documentation often keeps its definition above a separate body wrapper.
      // Guard the leading paragraph of an explicit main region as well as its grids/code.
      if (boundary.tagName === 'MAIN' || boundary.getAttribute('role') === 'main') {
        const lead = Array.from(boundary.querySelectorAll('p,h2,h3,h4,h5,h6')).find(element => !element.closest('details,table,figure'));
        if (lead?.tagName === 'P') {
          const value = normalize(textContent(lead));
          if (value && !candidateText.includes(value)) missing.push(lead);
        }
      }
      if (missing.length) {
        selected = boundary.innerHTML;
        metadata.selection = 'structured main content';
        warnings.push(`Article scoring omitted ${missing.length} table, code, footnote, or introductory block(s); the cleaned main boundary was retained to preserve structured content. It may include additional surrounding material.`);
      }
    }
  }
  const html = DOMPurify.sanitize(selected, { FORBID_TAGS: ['script', 'style', 'iframe', 'object', 'embed', 'form', 'input', 'button', 'svg', 'audio', 'video', 'source', 'track', 'link'], FORBID_ATTR: ['style', 'srcset'] });
  const visible = new DOMParser().parseFromString(html, 'text/html');
  cleanDocument(visible, base, options.fragment);
  // Source images are deliberately absent; retain the metadata shape expected
  // by storage/reader clients without claiming that captions imply image bytes.
  metadata.images = [];
  metadata.headings = Array.from(visible.querySelectorAll('h1,h2,h3,h4,h5,h6')).map((heading, index) => {
    const id = heading.id || `section-${index + 1}`;
    heading.id = id;
    return { id, level: Number(heading.tagName[1]), title: textContent(heading) };
  });
  metadata.byline = article?.byline || null;
  metadata.language = article?.lang || document.documentElement.lang || null;
  const links: LinkEvidence[] = Array.from(visible.querySelectorAll('a[href]')).map(anchor => {
    const target = anchor.getAttribute('href')!;
    return { url: target, label: textContent(anchor), kind: 'HTML link', doi: doiFrom(target) };
  });
  const tables = Array.from(visible.querySelectorAll('table')).map(table => Array.from(table.querySelectorAll('tr')).filter(row => row.closest('table') === table).map(row => Array.from(row.children).filter(cell => ['TH', 'TD'].includes(cell.tagName)).map(textContent)));
  const text = htmlText(visible);
  if (text.includes('\ufffd')) warnings.push('The decoded article contains replacement characters (U+FFFD); the original bytes must be checked for an encoding error.');
  const converter = new TurndownService({ headingStyle: 'atx', codeBlockStyle: 'fenced' });
  converter.use(gfm);
  converter.keep(node => node.nodeName.toLowerCase() === 'math');
  const cleanHtml = visible.body.innerHTML;
  const markdown = converter.turndown(cleanHtml);
  links.push(...textDois(text));
  return { title: cleanText(article?.title || pageTitle || title), text, html: cleanHtml, markdown, links, tables, metadata, warnings, engine: 'Mozilla Readability + DOMPurify + GFM', status: 'partial' };
}

export function clipText(source: string, title: string, kind: 'css' | 'text'): Extracted {
  const warnings: string[] = [];
  if (source.includes('\ufffd')) warnings.push('The decoded source contains replacement characters (U+FFFD); check the original byte encoding.');
  return { title, text: source, html: `<pre>${escapeHtml(source)}</pre>`, markdown: source, links: textDois(source), warnings, metadata: { format: kind }, engine: kind === 'css' ? 'CSS source capture' : 'Plain text capture', status: warnings.length ? 'partial' : 'ready' };
}

export function parseFeed(source: string, url: string): Extracted {
  if (/<!DOCTYPE|<!ENTITY/i.test(source)) throw new Error('XML document type and entity declarations are unsupported.');
  const xml = new DOMParser().parseFromString(source, 'application/xml');
  if (xml.querySelector('parsererror')) throw new Error('The feed is not valid XML.');
  const root = xml.documentElement.localName;
  if (!['rss', 'feed', 'RDF'].includes(root)) throw new Error('This is not an RSS or Atom feed.');
  const local = (element: Element, name: string) => textContent(Array.from(element.children).find(child => child.localName === name));
  const elementBase = (element: Element) => {
    const ancestors: Element[] = [];
    for (let current: Element | null = element; current; current = current.parentElement) ancestors.push(current);
    return ancestors.reverse().reduce((base, node) => safeUrl(node.getAttribute('xml:base') || '', base) || base, url);
  };
  const nodes = Array.from(xml.getElementsByTagNameNS('*', root === 'feed' ? 'entry' : 'item'));
  const entries = nodes.map(element => {
    const children = Array.from(element.children);
    const link = children.find(child => child.localName === 'link' && (!child.getAttribute('rel') || child.getAttribute('rel') === 'alternate'));
    const target = safeUrl(link?.getAttribute('href') || link?.textContent || '', link ? elementBase(link) : elementBase(element)) || '';
    const atomNamespace = 'http://www.w3.org/2005/Atom';
    const rssNamespace = 'http://purl.org/rss/1.0/';
    const contentNamespace = 'http://purl.org/rss/1.0/modules/content/';
    const standardChild = (name: string) => children.find(child => child.localName === name && (root === 'feed' ? child.namespaceURI === atomNamespace : !child.namespaceURI || child.namespaceURI === rssNamespace));
    const content = root === 'feed' ? standardChild('content') : children.find(child => child.localName === 'encoded' && child.namespaceURI === contentNamespace) || standardChild('content');
    const externalContent = root === 'feed' && content?.getAttribute('src') ? safeUrl(content.getAttribute('src')!, elementBase(content)) : null;
    const contentElement = externalContent || (content?.hasAttribute('src') && root === 'feed') ? standardChild('summary') : content || standardChild('description') || standardChild('summary');
    const contentType = contentElement?.getAttribute('type') || (root === 'feed' ? 'text' : 'html');
    let contentHtml = contentElement?.textContent || '';
    if (contentType === 'xhtml') contentHtml = contentElement?.innerHTML || '';
    else if (contentType === 'text' || contentType === 'text/plain') contentHtml = `<p>${escapeHtml(contentHtml)}</p>`;
    const contentBase = contentElement ? elementBase(contentElement) : target || elementBase(element);
    const clipped = clipHtml(contentHtml, contentBase, local(element, 'title'), { fragment: true });
    if (content?.hasAttribute('src') && root === 'feed') {
      clipped.warnings.push('Atom content references an external resource; the available summary was retained and the resource was not fetched.');
      if (externalContent) clipped.links.push({ url: externalContent, kind: 'external feed content' });
    }
    return { id: local(element, 'id') || local(element, 'guid') || target || local(element, 'title'), title: local(element, 'title'), url: target, published: local(element, 'published') || local(element, 'pubDate'), updated: local(element, 'updated'), author: local(element, 'author') || local(element, 'creator'), content: clipped.markdown, contentUrl: externalContent, html: clipped.html, text: clipped.text, images: clipped.metadata?.images, links: clipped.links, warnings: clipped.warnings, enclosures: children.filter(child => child.localName === 'enclosure' || (child.localName === 'link' && child.getAttribute('rel') === 'enclosure')).map(child => ({ url: safeUrl(child.getAttribute('url') || child.getAttribute('href') || '', elementBase(child)), type: child.getAttribute('type'), length: child.getAttribute('length') })) };
  });
  const container = root === 'feed' ? xml.documentElement : xml.querySelector('channel') || xml.documentElement;
  const links: LinkEvidence[] = entries.flatMap(entry => [{ url: entry.url, label: entry.title, kind: 'feed entry' }, ...entry.links, ...entry.enclosures.filter(enclosure => enclosure.url).map(enclosure => ({ url: enclosure.url!, kind: 'enclosure', label: enclosure.type || 'Attachment' }))]).filter(link => link.url);
  const markdown = entries.map(entry => `## ${entry.title}\n\n${entry.url}\n\n${entry.content}`).join('\n\n');
  return { title: local(container, 'title') || 'Feed', text: entries.map(entry => `${entry.title}\n\n${entry.text}`).join('\n\n'), markdown, links, entries, warnings: ['Feed content may be a summary. Linked articles and enclosures are not fetched automatically.'], metadata: { sourceUrl: url, capturedAt: new Date().toISOString(), format: root, entryCount: entries.length }, engine: 'RSS 2.0 / Atom parser', status: 'partial' };
}
