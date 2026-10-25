import { Readability } from '@mozilla/readability';
import { normalizeSpace, textContent } from './dom';
import type { SelectionMode } from './types';

export type Selection = { html: string; mode: SelectionMode; title: string | null; byline: string | null; lang: string | null; siteName: string | null; excerpt: string | null; warnings: string[] };

const POSITIVE = /article|body|content|entry|hentry|h-entry|main|page|post|story|text|blog|prose|abstract/i;
const NEGATIVE = /-ad-|\bad\b|ads|advert|banner|breadcrumb|combx|comment|community|cover-wrap|disqus|extra|footer|gdpr|header|legends|menu|modal|related|remark|replies|rss|share|shoutbox|sidebar|skyscraper|social|sponsor|supplemental|promo|widget|pagination|pager|popup|newsletter|cookie|subscribe/i;

function classWeight(element: Element) {
  let weight = 0;
  for (const value of [element.getAttribute('class') || '', element.id || '']) {
    if (!value) continue;
    if (NEGATIVE.test(value)) weight -= 25;
    if (POSITIVE.test(value)) weight += 25;
  }
  return weight;
}

function linkDensity(element: Element) {
  const total = normalizeSpace(element.textContent || '').length;
  if (!total) return 0;
  const linked = Array.from(element.querySelectorAll('a')).reduce((sum, anchor) => sum + normalizeSpace(anchor.textContent || '').length, 0);
  return linked / total;
}

export type ScoredBlock = { element: Element; score: number };

/**
 * Readability-style block scorer: paragraphs vote for their parent and grandparent by length and
 * punctuation, class names adjust the vote, and link-heavy containers are penalised.
 */
export function scoreBlocks(root: Element | Document, maxElements = 50000): ScoredBlock[] {
  const scores = new Map<Element, number>();
  const paragraphs = Array.from(root.querySelectorAll('p,pre,td,blockquote,li,dd,h2,h3,h4')).slice(0, maxElements);
  for (const paragraph of paragraphs) {
    const text = normalizeSpace(paragraph.textContent || '');
    if (text.length < 25) continue;
    const parent = paragraph.parentElement, grandparent = parent?.parentElement;
    if (!parent) continue;
    const vote = 1 + (text.match(/[,，、;；]/g) || []).length + Math.min(Math.floor(text.length / 100), 3);
    for (const [ancestor, share] of [[parent, 1], [grandparent, 0.5]] as const) {
      if (!ancestor || ancestor === root || ancestor.nodeName === 'HTML') continue;
      if (!scores.has(ancestor)) scores.set(ancestor, classWeight(ancestor) + (ancestor.matches('article,main,[role="main"],[itemprop~="articleBody"]') ? 10 : ancestor.matches('div,section') ? 5 : ancestor.matches('blockquote,pre,td,th,ul,ol,dl,li,form,address') ? -3 : 0));
      scores.set(ancestor, scores.get(ancestor)! + vote * share);
    }
  }
  return Array.from(scores, ([element, score]) => ({ element, score: score * (1 - linkDensity(element)) })).sort((a, b) => b.score - a.score);
}

export function mainBoundary(document: Document): Element | undefined {
  // Prefer an explicit publishing boundary before the broader page main region.
  for (const selector of ['.mw-parser-output', 'article', '[itemprop~="articleBody"]', 'main,[role="main"]']) {
    const candidates = Array.from(document.querySelectorAll(selector));
    if (candidates.length) return candidates.sort((a, b) => textContent(b).length - textContent(a).length)[0];
  }
  return undefined;
}

export function selectContent(document: Document, options: { fragment?: boolean; maxElements: number }): Selection {
  const warnings: string[] = [];
  if (options.fragment) return { html: document.body?.innerHTML || '', mode: 'fragment', title: null, byline: null, lang: null, siteName: null, excerpt: null, warnings };
  let article: ReturnType<Readability['parse']> = null;
  try {
    article = new Readability(document.cloneNode(true) as Document, { keepClasses: false, charThreshold: 100, maxElemsToParse: options.maxElements }).parse();
  } catch (error) {
    warnings.push(`Article scoring stopped early (${error instanceof Error ? error.message : 'error'}); the cleaned page structure was used instead.`);
  }
  const boundary = mainBoundary(document);
  let selected = article?.content || '';
  let mode: SelectionMode = 'readability';
  if (!selected) {
    const best = scoreBlocks(document, options.maxElements)[0];
    if (best && normalizeSpace(best.element.textContent || '').length >= 200 && (!boundary || boundary.contains(best.element) || best.element.contains(boundary))) {
      selected = best.element.innerHTML;
      mode = 'scored block';
      warnings.push('Article scoring was inconclusive; the densest text block was retained.');
    } else {
      selected = boundary?.innerHTML || document.body?.innerHTML || '';
      mode = boundary ? 'main boundary' : 'body';
      warnings.push(boundary ? 'Article scoring was inconclusive; the cleaned main section was retained.' : 'No main article boundary was identified; cleaned body content was retained and may include page chrome.');
    }
  }
  if (article && boundary) {
    const candidate = new DOMParser().parseFromString(selected, 'text/html');
    const candidateText = normalizeSpace(textContent(candidate.body));
    const missing = Array.from(boundary.querySelectorAll('table,pre,[role="doc-footnote"],[role="doc-endnote"]')).filter(element => {
      const value = normalizeSpace(textContent(element));
      return value && !candidateText.includes(value);
    });
    // Reference documentation often keeps its definition above a separate body wrapper.
    // Guard the leading paragraph of an explicit main region as well as its grids/code.
    if (boundary.tagName === 'MAIN' || boundary.getAttribute('role') === 'main') {
      const lead = Array.from(boundary.querySelectorAll('p,h2,h3,h4,h5,h6')).find(element => !element.closest('details,table,figure'));
      if (lead?.tagName === 'P') {
        const value = normalizeSpace(textContent(lead));
        if (value && !candidateText.includes(value)) missing.push(lead);
      }
    }
    if (missing.length) {
      selected = boundary.innerHTML;
      mode = 'structured main content';
      warnings.push(`Article scoring omitted ${missing.length} table, code, footnote, or introductory block(s); the cleaned main boundary was retained to preserve structured content. It may include additional surrounding material.`);
    }
  }
  return { html: selected, mode, title: article?.title || null, byline: article?.byline || null, lang: article?.lang || null, siteName: article?.siteName || null, excerpt: article?.excerpt || null, warnings };
}
