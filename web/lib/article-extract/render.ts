import DOMPurify from 'dompurify';
import TurndownService from 'turndown';
import { gfm } from 'turndown-plugin-gfm';
import type { LinkEvidence } from '../types';
import { cleanText, normalizeSpace, textContent } from './dom';
import { doiFrom, pmidFrom } from './url';
import type { ArticleHeading, ArticleImage } from './types';

export function sanitize(html: string) {
  return DOMPurify.sanitize(html, { FORBID_TAGS: ['script', 'style', 'iframe', 'object', 'embed', 'form', 'input', 'button', 'svg', 'audio', 'video', 'source', 'track', 'link', 'meta'], FORBID_ATTR: ['style', 'srcset', 'onerror', 'onload'] });
}

/** Plain text that keeps one block per line, list markers, quote markers and tab-separated table cells. */
export function plainText(document: Document): string {
  const tree = document.body.cloneNode(true) as HTMLElement;
  for (const cell of Array.from(tree.querySelectorAll('th,td'))) cell.appendChild(document.createTextNode('\t'));
  for (const item of Array.from(tree.querySelectorAll('li'))) {
    const parent = item.parentElement;
    const ordered = parent?.tagName === 'OL';
    const index = parent ? Array.from(parent.children).filter(child => child.tagName === 'LI').indexOf(item) + 1 : 1;
    item.insertBefore(document.createTextNode(ordered ? `${index}. ` : '- '), item.firstChild);
  }
  for (const quote of Array.from(tree.querySelectorAll('blockquote'))) {
    const blocks = Array.from(quote.children).filter(child => /^(P|DIV|LI|H[1-6])$/.test(child.tagName));
    for (const block of blocks.length ? blocks : [quote]) block.insertBefore(document.createTextNode('> '), block.firstChild);
  }
  for (const element of Array.from(tree.querySelectorAll('p,div,section,article,h1,h2,h3,h4,h5,h6,li,tr,blockquote,pre,figure,figcaption,br,dt,dd,hr'))) element.parentNode?.insertBefore(document.createTextNode('\n'), element.nextSibling);
  for (const element of Array.from(tree.querySelectorAll('p,h1,h2,h3,h4,h5,h6,blockquote,pre,figure,ul,ol,table'))) element.parentNode?.insertBefore(document.createTextNode('\n'), element);
  return cleanText(tree.textContent || '').replace(/[ \t]+\n/g, '\n').replace(/\n{3,}/g, '\n\n').trim();
}

export function markdownConverter() {
  const converter = new TurndownService({ headingStyle: 'atx', codeBlockStyle: 'fenced', bulletListMarker: '-', hr: '---' });
  converter.use(gfm);
  converter.keep(node => node.nodeName.toLowerCase() === 'math');
  converter.addRule('figure', {
    filter: 'figure',
    replacement: (_content, node) => {
      const figure = node as HTMLElement;
      const caption = normalizeSpace(cleanText(figure.querySelector('figcaption')?.textContent || ''));
      const parts: string[] = [];
      for (const image of Array.from(figure.querySelectorAll('img'))) {
        const alt = normalizeSpace(cleanText(image.getAttribute('alt') || '')).replace(/[[\]]/g, '');
        const src = image.getAttribute('src') || '';
        if (src) parts.push(`![${alt}](${src})`);
      }
      const body = Array.from(figure.childNodes).filter(child => child.nodeName.toLowerCase() !== 'figcaption' && child.nodeName.toLowerCase() !== 'img').map(child => converter.turndown((child as HTMLElement).outerHTML || child.textContent || '')).join('\n\n').trim();
      if (body) parts.push(body);
      if (caption) parts.push(`*${caption.replace(/\*/g, '\\*')}*`);
      return parts.length ? `\n\n${parts.join('\n\n')}\n\n` : '';
    },
  });
  converter.addRule('figcaption', { filter: 'figcaption', replacement: content => content ? `\n\n*${normalizeSpace(content).replace(/\*/g, '\\*')}*\n\n` : '' });
  return converter;
}

export function collectImages(visible: Document, maxImages: number): { images: ArticleImage[]; dropped: number } {
  const all = Array.from(visible.querySelectorAll('img'));
  const kept = all.slice(0, maxImages);
  for (const extra of all.slice(maxImages)) extra.remove();
  const images = kept.map((image, index) => {
    const id = `image-${index + 1}`;
    image.setAttribute('data-image-id', id);
    const figure = image.closest('figure');
    return { id, url: image.getAttribute('src')!, alt: cleanText(image.getAttribute('alt') || ''), caption: textContent(figure?.querySelector('figcaption')), width: image.getAttribute('width') || undefined, height: image.getAttribute('height') || undefined, source: 'article' as const };
  });
  return { images, dropped: all.length - kept.length };
}

export function collectHeadings(visible: Document): ArticleHeading[] {
  const seen = new Set<string>();
  return Array.from(visible.querySelectorAll('h1,h2,h3,h4,h5,h6')).map((heading, index) => {
    let id = heading.id || `section-${index + 1}`;
    while (seen.has(id)) id = `${id}-${index + 1}`;
    seen.add(id);
    heading.id = id;
    return { id, level: Number(heading.tagName[1]), title: textContent(heading) };
  });
}

export function collectLinks(visible: Document, maxLinks: number): { links: LinkEvidence[]; dropped: number } {
  const anchors = Array.from(visible.querySelectorAll('a[href]'));
  const links = anchors.slice(0, maxLinks).map(anchor => {
    const target = anchor.getAttribute('href')!;
    const doi = doiFrom(target);
    const pmid = pmidFrom(target);
    return { url: target, label: textContent(anchor), kind: doi ? 'DOI link' : pmid ? 'PubMed link' : 'HTML link', doi };
  });
  return { links, dropped: Math.max(0, anchors.length - maxLinks) };
}

export function collectTables(visible: Document, maxTables: number): { tables: string[][][]; dropped: number } {
  const all = Array.from(visible.querySelectorAll('table'));
  const tables = all.slice(0, maxTables).map(table => Array.from(table.querySelectorAll('tr')).filter(row => row.closest('table') === table).map(row => Array.from(row.children).filter(cell => ['TH', 'TD'].includes(cell.tagName)).map(textContent)));
  return { tables, dropped: Math.max(0, all.length - maxTables) };
}
