import { extractArticle, toExtracted } from './article-extract';
import { escapeHtml, textContent } from './article-extract/dom';
import type { Extracted, LinkEvidence } from './types';

export { safeUrl, doiFrom, textDois } from './article-extract/url';
import { safeUrl, textDois } from './article-extract/url';

/**
 * Clean an HTML page into a saved article. The article pipeline lives in `./article-extract`;
 * this wrapper keeps the workspace's `Extracted` record shape and its capture timestamp.
 */
export function clipHtml(source: string, url: string, title = 'Saved page', options: { fragment?: boolean; capturedAt?: string } = {}): Extracted {
  const article = extractArticle(source, url, { fragment: options.fragment, fallbackTitle: title });
  return toExtracted(article, options.capturedAt ?? new Date().toISOString());
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
