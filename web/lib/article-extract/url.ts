import type { LinkEvidence } from '../types';

export function safeUrl(value: string, base?: string) {
  if (!value.trim()) return null;
  try {
    const u = new URL(value, base);
    return ['http:', 'https:'].includes(u.protocol) && !u.username && !u.password ? u.href : null;
  } catch { return null; }
}

const DOI = /10\.\d{4,9}\/[^\s<>"?#]+/i;

export function doiFrom(value: string) {
  let decoded = value;
  try { decoded = decodeURIComponent(value); } catch { /* Preserve malformed percent encoding. */ }
  return decoded.match(DOI)?.[0].replace(/[.,;)]+$/, '');
}

export function textDois(text: string): LinkEvidence[] {
  return Array.from(new Set(text.match(/10\.\d{4,9}\/[^\s<>"?#]+/gi) || [])).map(value => {
    const doi = value.replace(/[.,;]+$/, '');
    return { url: `https://doi.org/${doi}`, doi, kind: 'printed DOI' };
  });
}

export function pmidFrom(value: string) {
  return value.match(/(?:pubmed\.ncbi\.nlm\.nih\.gov|ncbi\.nlm\.nih\.gov\/pubmed)\/(\d{1,9})(?:[/?#]|$)/i)?.[1] || value.match(/^(?:pmid:?\s*)(\d{1,9})$/i)?.[1];
}
export function pmcidFrom(value: string) {
  return value.match(/\b(PMC\d{1,9})\b/i)?.[1].toUpperCase();
}
export function arxivFrom(value: string) {
  return value.match(/arxiv\.org\/(?:abs|pdf|html)\/(\d{4}\.\d{4,5}(?:v\d+)?)/i)?.[1] || value.match(/^(?:arxiv:)?(\d{4}\.\d{4,5}(?:v\d+)?)$/i)?.[1];
}

/** Two hosts belong to the same site when one is the other, or a subdomain of it, after dropping www/m/amp prefixes. */
export function sameSite(a: string, b: string) {
  const strip = (host: string) => host.toLowerCase().replace(/^(?:www|m|amp|mobile)\./, '');
  const x = strip(a), y = strip(b);
  if (x === y) return true;
  const [short, long] = x.length <= y.length ? [x, y] : [y, x];
  return short.includes('.') && long.endsWith(`.${short}`);
}
