import { owner, boundedBody, failure } from '@/lib/server';
import { fetchPublicSource, looksBinary, readBounded, withTimeout, FETCH_LIMITS } from '@/lib/source-fetch';

const ACCEPT = 'text/html,application/xhtml+xml,application/rss+xml,application/atom+xml,application/rdf+xml,application/xml,text/xml,text/css,text/plain';
const TEXT_SOURCE = /^(?:text\/(?:html|xml|css|plain)|application\/(?:xhtml\+xml|rss\+xml|atom\+xml|rdf\+xml|xml|feed\+json))$/i;
/** Largest HTML/feed source forwarded to the browser; longer responses are cut and flagged so the extractor can repair the tail. */
export const MAX_SOURCE_BYTES = 16 * 1024 * 1024;

export async function POST(request: Request) {
  try {
    await owner(request);
    const payload = JSON.parse(new TextDecoder().decode(await boundedBody(request, 8192)));
    if (typeof payload.url !== 'string' || payload.url.length > 4096) throw new Error('A URL is required.');
    const signal = withTimeout(request.signal, FETCH_LIMITS.timeoutMs);
    const { response, url, redirects } = await fetchPublicSource(payload.url, ACCEPT, signal);
    const type = response.headers.get('content-type') || '';
    const mime = type.split(';')[0].trim().toLowerCase();
    if (!TEXT_SOURCE.test(mime)) { await response.body?.cancel(); throw new Error('This URL is not an HTML page, stylesheet, text source, or feed. Add the original file instead.'); }
    const declared = Number(response.headers.get('content-length') || 0);
    const { bytes, truncated } = await readBounded(response.body, MAX_SOURCE_BYTES);
    if (looksBinary(bytes)) throw new Error('The URL returned binary data, not a text source. Add the original file instead.');
    const headers: Record<string, string> = {
      'Content-Type': type || 'text/html',
      'X-TPE-Source-URL': url,
      'X-TPE-Redirects': String(redirects),
      'X-TPE-Source-Bytes': String(bytes.length),
      'Cache-Control': 'private, no-store',
      'X-Content-Type-Options': 'nosniff',
      'Content-Security-Policy': "default-src 'none'; sandbox",
    };
    if (truncated || declared > MAX_SOURCE_BYTES) headers['X-TPE-Truncated'] = 'true';
    return new Response(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer, { headers });
  } catch (error) { return failure(error); }
}
