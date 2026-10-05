import { cleanText } from './dom';
import { safeUrl } from './url';

const LAZY_SOURCE_ATTRIBUTES = ['data-src', 'data-original', 'data-lazy-src', 'data-lazy', 'data-url', 'data-hi-res-src', 'data-full-src', 'data-large-src', 'data-orig-file', 'data-src-large', 'data-image', 'data-img-src', 'data-echo', 'data-actualsrc', 'data-pin-media'];
const LAZY_SET_ATTRIBUTES = ['data-srcset', 'data-lazy-srcset', 'srcset'];

/** Resolve the real image URL, preferring lazy-loading attributes and the largest srcset candidate over placeholders. */
export function imageSource(image: Element, base: string): string | null {
  const usable = (value: string | null) => {
    const url = safeUrl(value || '', base);
    if (!url) return null;
    const path = new URL(url).pathname;
    return /(?:^|\/)(?:transparent|spacer|blank|placeholder|tracking[-_]?pixel|pixel)(?:[._-]|$)/i.test(path) ? null : url;
  };
  for (const attr of LAZY_SOURCE_ATTRIBUTES) {
    const url = usable(image.getAttribute(attr));
    if (url) return url;
  }
  const sets = LAZY_SET_ATTRIBUTES.map(attr => image.getAttribute(attr));
  for (const source of Array.from(image.closest('picture')?.querySelectorAll('source') || [])) {
    sets.push(source.getAttribute('data-srcset'), source.getAttribute('srcset'));
  }
  for (const set of sets) {
    if (!set) continue;
    const candidates = set.split(',').map(part => {
      const [value, descriptor] = part.trim().split(/\s+/);
      return { url: usable(value), size: Number.parseFloat(descriptor || '1') || 1 };
    }).filter(candidate => candidate.url).sort((a, b) => b.size - a.size);
    if (candidates[0]) return candidates[0].url;
  }
  return usable(image.getAttribute('src'));
}

// Explicit chrome markers only: broad substring rules can delete real article sections.
const CHROME_SELECTORS = [
  '[role="dialog"]', '[aria-modal="true"]', '[role="search"]', '[role="menubar"]', '[role="contentinfo"]',
  '[data-ad-slot]', '[data-ad-unit]', '.advertisement', '.ad-container', '.ad-slot', '.ad-wrapper', 'ins.adsbygoogle', '[id^="div-gpt-ad"]', '.sponsored-content',
  '.cookie-banner', '.cookie-consent', '.cookie-notice', '[id*="cookie-banner"]', '[id*="cookie-consent"]', '[class*="cookieconsent"]', '#onetrust-consent-sdk', '#CybotCookiebotDialog', '.gdpr-banner', '.consent-banner',
  '.newsletter-signup', '.newsletter-form', '.subscribe-box', '.social-share', '.share-buttons', '.share-bar', '.sharing-tools', '.addthis_toolbox', '.a2a_kit',
  '.related-articles', '.related-posts', '.related-content', '.related-stories', '.read-next', '.more-stories', '.recommended-articles', '.recirculation', '.trending-now', '.popular-posts', '[id*="taboola"]', '[id*="outbrain"]', '.OUTBRAIN', '[data-module="related"]',
  '#comments', '.comments', '.comment-list', '.comments-area', '.comments-section', '#disqus_thread', '#respond', '.comment-respond', '[itemtype*="schema.org/Comment"]', 'section[aria-label*="comment" i]',
  '.paywall', '#paywall', '.paywall-prompt', '.regwall', '.tp-modal', '.tp-backdrop', '.piano-offer', '[data-paywall]', '.subscription-wall', '.subscribe-wall', '.meter-prompt',
  '.breadcrumb', '.breadcrumbs', '[aria-label="breadcrumb" i]', '.skip-link', 'a[href^="#main-content"]', '.print-only', '.visually-hidden', '.sr-only', '.screen-reader-text',
];

const AMP_REMOVED = 'amp-ad,amp-sticky-ad,amp-auto-ads,amp-analytics,amp-pixel,amp-sidebar,amp-consent,amp-user-notification,amp-iframe,amp-video,amp-audio,amp-embed,amp-social-share,amp-ad-exit,amp-list,amp-install-serviceworker,amp-geo,amp-story-auto-ads';

export function normalizeAmp(document: Document) {
  for (const node of Array.from(document.querySelectorAll('amp-img,amp-anim'))) {
    const image = document.createElement('img');
    for (const attr of ['src', 'srcset', 'alt', 'title', 'width', 'height']) if (node.hasAttribute(attr)) image.setAttribute(attr, node.getAttribute(attr)!);
    node.replaceWith(image);
  }
  for (const node of Array.from(document.querySelectorAll(AMP_REMOVED))) node.remove();
  // Content gated for subscribers is not public: keep only sections visible to anonymous readers. Never bypass.
  for (const node of Array.from(document.querySelectorAll('[amp-access],[subscriptions-section]'))) {
    const access = node.getAttribute('amp-access'), section = node.getAttribute('subscriptions-section');
    const publicAccess = access === null || /^\s*(?:NOT\b|TRUE\b)/i.test(access);
    const publicSection = section === null || !/^content$/i.test(section);
    if (!publicAccess || !publicSection) node.remove();
  }
}

export type CleanOptions = { fragment?: boolean };

export function cleanDocument(document: Document, base: string, options: CleanOptions = {}) {
  if (!document.body) return;
  // Promote inert noscript image fallbacks before deleting scripting/hydration containers.
  for (const node of Array.from(document.querySelectorAll('noscript'))) {
    const fallback = node.querySelector('img') ? node : new DOMParser().parseFromString(node.textContent || '', 'text/html');
    for (const image of Array.from(fallback.querySelectorAll('img'))) node.parentNode?.insertBefore(document.importNode(image, true), node);
    node.remove();
  }
  normalizeAmp(document);
  for (const node of Array.from(document.querySelectorAll('script,style,template,iframe,object,embed,svg,canvas,form,input,button,select,textarea,link,meta'))) node.remove();
  for (const node of Array.from(document.querySelectorAll('[hidden],[aria-hidden="true"],[inert],[style]'))) {
    const style = (node.getAttribute('style') || '').replace(/\s+/g, '').toLowerCase();
    if (node.hasAttribute('hidden') || node.getAttribute('aria-hidden') === 'true' || node.hasAttribute('inert') || /(?:^|;)display:none(?:!important)?(?:;|$)|(?:^|;)visibility:hidden(?:!important)?(?:;|$)/.test(style)) node.remove();
  }
  if (!options.fragment) {
    for (const node of Array.from(document.querySelectorAll('nav,[role="navigation"],[role="banner"],[role="complementary"],aside,footer'))) {
      if (!node.matches('[role="doc-footnote"],[role="doc-endnote"]') && !node.querySelector('[role="doc-footnote"],[role="doc-endnote"]')) node.remove();
    }
    for (const node of Array.from(document.querySelectorAll('header'))) {
      if (!node.closest('article,main,[role="main"]')) node.remove();
    }
    for (const node of Array.from(document.querySelectorAll(CHROME_SELECTORS.join(',')))) {
      if (node.closest('pre,code,figure,table') || node.matches('article,main,[role="main"]')) continue;
      node.remove();
    }
  }
  const textNodes = document.createTreeWalker(document.body, 4); // NodeFilter.SHOW_TEXT
  while (textNodes.nextNode()) textNodes.currentNode.nodeValue = cleanText(textNodes.currentNode.nodeValue || '');
  for (const image of Array.from(document.querySelectorAll('img'))) {
    const src = imageSource(image, base);
    const width = Number(image.getAttribute('width')), height = Number(image.getAttribute('height'));
    if (!src || (width > 0 && width <= 2 && height > 0 && height <= 2)) { image.remove(); continue; }
    image.setAttribute('src', src);
    for (const attr of ['alt', 'title']) if (image.hasAttribute(attr)) image.setAttribute(attr, cleanText(image.getAttribute(attr)!));
    for (const attr of Array.from(image.attributes)) {
      if (!['src', 'alt', 'title', 'width', 'height'].includes(attr.name)) image.removeAttribute(attr.name);
    }
  }
  // A resolved img remains; source elements no longer trigger uncontrolled alternate fetches.
  for (const source of Array.from(document.querySelectorAll('source'))) source.remove();
  for (const figure of Array.from(document.querySelectorAll('figure'))) {
    if (!figure.querySelector('img,table,pre,blockquote,figcaption') && !cleanText(figure.textContent || '').trim()) figure.remove();
  }
  for (const anchor of Array.from(document.querySelectorAll('a[href]'))) {
    const href = safeUrl(anchor.getAttribute('href') || '', base);
    if (href) anchor.setAttribute('href', href); else anchor.removeAttribute('href');
  }
}
