/** Source evidence survives separately from elements that can acquire resources. */
export type PassiveHtmlSnapshot = {
  document: Document;
  canonicalHref: string;
  feeds: { type: string | null; href: string }[];
};

/** Recover only a leading document root, before template fragment parsing loses it.
 *
 * This is a deliberately small prologue recognizer, not a second HTML parser.
 * Whitespace, complete comments and a conventional doctype may precede <html>.
 * Quoted > characters stay inside attributes. A later <html> in article text,
 * comments or scripts cannot supply invented document provenance. Fragments and
 * unrecognized/malformed prologues have no declared root language here.
 *
 * Parse the matched opening tag on an inert span so the browser, rather than a
 * custom entity decoder, interprets lang="fr&#45;CA" and duplicate attributes.
 * No source content enters a live document and none of its attributes are copied
 * to the result except lang. This does not validate a language tag's semantics.
 */
function rootLanguage(source: string): string | null {
  const opening = source.match(/^(?:\s|<!--[\s\S]*?-->|<!doctype\b(?:"[^"]*"|'[^']*'|[^'">])*?>)*(<html\b(?:"[^"]*"|'[^']*'|[^'">])*?>)/i)?.[1];
  if (!opening) return null;
  const carrier = document.createElement('template');
  carrier.innerHTML = `<span${opening.slice(5)}</span>`;
  return carrier.content.firstElementChild?.getAttribute('lang') ?? null;
}

/** Preserve provenance, strip acquisition, then construct a detached document.
 *
 * Template contents are inert: source images, styles and scripts never join the
 * active document. Canonical/feed links remain strings for downstream validation,
 * not link elements. The document returned to Readability contains only the
 * sanitized serialization; callers still sanitize selected publication markup.
 * An optional image predicate is for existing owner-local assets, not remote
 * acquisition. The default local-upload path accepts no source images.
 */
export function passiveHtmlSnapshot(
  source: string,
  allowImage: (src: string) => boolean = () => false,
): PassiveHtmlSnapshot {
  const language = rootLanguage(source);
  const template = document.createElement('template');
  template.innerHTML = source;

  // Capture evidence before dropping every link element. Resolve/validate these
  // strings in the caller's source URL context; reading href never fetches it.
  const canonicalHref = template.content.querySelector('link[rel="canonical"]')?.getAttribute('href') || '';
  const feeds = Array.from(template.content.querySelectorAll('link[rel="alternate"]'))
    .filter(element => /rss|atom/i.test(element.getAttribute('type') || ''))
    .map(element => ({ type: element.getAttribute('type'), href: element.getAttribute('href') || '' }));

  // Inspect hidden styles before removing style attributes, so hidden content
  // does not become visible merely because its presentation was sanitized.
  for (const node of Array.from(template.content.querySelectorAll('[hidden],[aria-hidden="true"],[inert],[style]'))) {
    const style = (node.getAttribute('style') || '').replace(/\s+/g, '').toLowerCase();
    if (node.hasAttribute('hidden') || node.getAttribute('aria-hidden') === 'true' || node.hasAttribute('inert') || /(?:^|;)display:none(?:!important)?(?:;|$)|(?:^|;)visibility:hidden(?:!important)?(?:;|$)/.test(style)) {
      node.remove();
    }
  }
  for (const node of Array.from(template.content.querySelectorAll('iframe,object,embed,link,style,svg,audio,video,source,track,picture,noscript,form,input,button,select,textarea'))) {
    node.remove();
  }
  // JSON-LD is inert evidence. clipHtml projects selected fields and reports bad
  // JSON; executable/hydration scripts are discarded before document parsing.
  for (const script of Array.from(template.content.querySelectorAll('script'))) {
    if (script.getAttribute('type') !== 'application/ld+json') script.remove();
  }
  for (const node of Array.from(template.content.querySelectorAll('*'))) {
    const image = node.tagName === 'IMG' && allowImage(node.getAttribute('src') || '');
    if (node.tagName === 'IMG' && !image) {
      node.remove();
      continue;
    }
    // Snapshot attribute lists before mutation; browser NamedNodeMaps are live.
    for (const attr of Array.from(node.attributes)) {
      if (/^on/i.test(attr.name) || ['srcset', 'background', 'poster', 'ping', 'action', 'formaction', 'srcdoc', 'style', 'http-equiv'].includes(attr.name) || (attr.name === 'src' && !image)) {
        node.removeAttribute(attr.name);
      }
    }
  }
  const sanitized = new DOMParser().parseFromString(template.innerHTML, 'text/html');
  if (language !== null) sanitized.documentElement.setAttribute('lang', language);
  return { document: sanitized, canonicalHref, feeds };
}

/** Compatibility entry point for callers that need only passive content. */
export function passiveHtmlDocument(source: string, allowImage: (src: string) => boolean = () => false): Document {
  return passiveHtmlSnapshot(source, allowImage).document;
}
