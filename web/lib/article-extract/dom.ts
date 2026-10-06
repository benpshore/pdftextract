export const cleanText = (value: string) => value.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/g, '');
export const escapeHtml = (value: string) => value.replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
export const textContent = (element: Element | Document | null | undefined) => cleanText((element as Element | null)?.textContent || '').trim();
export const normalizeSpace = (value: string) => value.replace(/\s+/g, ' ').trim();

export function parseHtml(source: string): Document {
  return new DOMParser().parseFromString(source, 'text/html');
}

/**
 * Close constructs that swallow the rest of a truncated or garbled document when left open.
 * The repairs are reported so a reader knows the page was incomplete.
 */
export function repairHtml(source: string): { html: string; repairs: string[] } {
  const repairs: string[] = [];
  let html = source;
  if (html.includes('\u0000')) { html = html.replace(/\u0000/g, ''); repairs.push('Removed NUL bytes from the HTML.'); }
  const count = (pattern: RegExp) => (html.match(pattern) || []).length;
  // Comments do not nest. Move past each closing delimiter, even when its body
  // contains another opener; never search backwards with a negative offset.
  let cursor = 0;
  while (cursor < html.length) {
    const open = html.indexOf('<!--', cursor);
    if (open < 0) break;
    const close = html.indexOf('-->', open + 4);
    if (close < 0) {
      html = html.slice(0, open) + html.slice(open).replaceAll('<!--', '<!-- -->');
      repairs.push('Neutralised an unterminated HTML comment.');
      break;
    }
    cursor = close + 3;
  }
  for (const tag of ['script', 'style', 'textarea', 'title', 'noscript', 'template', 'iframe']) {
    const opens = count(new RegExp(`<${tag}(?:\\s[^>]*)?>`, 'gi')), closes = count(new RegExp(`</${tag}\\s*>`, 'gi'));
    if (opens > closes) { html += `</${tag}>`.repeat(opens - closes); repairs.push(`Closed ${opens - closes} unterminated <${tag}> element(s).`); }
  }
  if (count(/<!\[CDATA\[/g) > count(/\]\]>/g)) { html += ']]>'; repairs.push('Closed an unterminated CDATA section.'); }
  return { html, repairs };
}

/** Byte sequences typical of UTF-8 decoded as Latin-1/Windows-1252: Ã©, â€™, Â  and friends. */
export function looksMojibake(text: string) {
  const sample = text.slice(0, 200000);
  const hits = (sample.match(/Ã[\u0080-¿ŒœŠšŸŽžˆ˜–—‘-„†-•…‰‹›€™]|â€[\u0098-\u009fŒœ–—‘-„†-•…‰‹›€™]|Â[ -¿]/g) || []).length;
  return hits >= 3 && hits * 400 > sample.length;
}
