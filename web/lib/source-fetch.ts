/**
 * SSRF-safe fetching of public web sources for the server routes.
 * Every hop is checked syntactically (scheme, credentials, port, host shape, reserved names) and
 * by resolving the host through DNS-over-HTTPS and rejecting private, loopback, link-local,
 * multicast, reserved, NAT64/6to4/Teredo and IPv4-mapped addresses. Redirects are followed manually
 * so each target is checked again. The DNS check and the fetch are separate requests, so a resolver
 * that changes answers between them (DNS rebinding) is not prevented here; Workers cannot pin a
 * socket to the checked address.
 */
export type Resolver = (hostname: string) => Promise<string[]>;
export type FetchLike = (input: string | URL, init?: RequestInit) => Promise<Response>;

export const FETCH_LIMITS = { maxRedirects: 5, timeoutMs: 15000, dnsTimeoutMs: 6000 };

export function parseIpv4(value: string): number[] | null {
  const match = value.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
  if (!match) return null;
  const octets = match.slice(1).map(Number);
  return octets.every(octet => octet <= 255) ? octets : null;
}

function publicIpv4(octets: number[]) {
  const [a, b, c] = octets;
  if (a === 0 || a === 10 || a === 127) return false; // this network, private, loopback
  if (a === 100 && b >= 64 && b <= 127) return false; // carrier-grade NAT
  if (a === 169 && b === 254) return false; // link-local, cloud metadata
  if (a === 172 && b >= 16 && b <= 31) return false; // private
  if (a === 192 && b === 0 && (c === 0 || c === 2)) return false; // IETF protocol assignments, TEST-NET-1
  if (a === 192 && b === 88 && c === 99) return false; // 6to4 relay anycast
  if (a === 192 && b === 168) return false; // private
  if (a === 198 && (b === 18 || b === 19)) return false; // benchmarking
  if (a === 198 && b === 51 && c === 100) return false; // TEST-NET-2
  if (a === 203 && b === 0 && c === 113) return false; // TEST-NET-3
  if (a >= 224) return false; // multicast, reserved, broadcast
  return true;
}

/** Expand an IPv6 textual address to eight 16-bit groups; null when it is not a valid address. */
export function parseIpv6(value: string): number[] | null {
  let text = value.trim().replace(/^\[|\]$/g, '').replace(/%.*$/, '').toLowerCase();
  if (!/^[0-9a-f:.]+$/.test(text) || text.split('::').length > 2) return null;
  const mapped = text.match(/^(.*:)(\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3})$/);
  if (mapped) {
    const octets = parseIpv4(mapped[2]);
    if (!octets) return null;
    text = `${mapped[1]}${((octets[0] << 8) | octets[1]).toString(16)}:${((octets[2] << 8) | octets[3]).toString(16)}`;
  }
  const [head, tail] = text.split('::');
  const parse = (part: string) => part ? part.split(':').map(group => (group.length >= 1 && group.length <= 4 ? Number.parseInt(group, 16) : Number.NaN)) : [];
  const left = parse(head), right = tail === undefined ? [] : parse(tail);
  if ([...left, ...right].some(Number.isNaN)) return null;
  if (tail === undefined) return left.length === 8 ? left : null;
  const missing = 8 - left.length - right.length;
  return missing < 1 ? null : [...left, ...new Array<number>(missing).fill(0), ...right];
}

function publicIpv6(groups: number[]) {
  const v4 = (high: number, low: number) => publicIpv4([high >> 8, high & 0xff, low >> 8, low & 0xff]);
  if (groups.every(group => group === 0)) return false; // unspecified
  if (groups.slice(0, 7).every(group => group === 0) && groups[7] === 1) return false; // loopback
  if (groups.slice(0, 5).every(group => group === 0) && groups[5] === 0xffff) return v4(groups[6], groups[7]); // IPv4-mapped
  if (groups.slice(0, 6).every(group => group === 0)) return v4(groups[6], groups[7]); // IPv4-compatible (deprecated)
  if (groups[0] === 0x64 && groups[1] === 0xff9b && groups.slice(2, 6).every(group => group === 0)) return v4(groups[6], groups[7]); // NAT64
  if (groups[0] === 0x2002) return v4(groups[1], groups[2]); // 6to4
  if (groups[0] === 0x2001 && groups[1] === 0) return false; // Teredo: embedded addresses are obfuscated
  if (groups[0] === 0x2001 && groups[1] === 0xdb8) return false; // documentation
  if (groups[0] === 0x2001 && (groups[1] & 0xfff0) === 0x10) return false; // ORCHID
  if (groups[0] === 0x100 && groups.slice(1, 4).every(group => group === 0)) return false; // discard-only
  if ((groups[0] & 0xfe00) === 0xfc00) return false; // unique local fc00::/7
  if ((groups[0] & 0xffc0) === 0xfe80) return false; // link-local
  if ((groups[0] & 0xffc0) === 0xfec0) return false; // site-local (deprecated)
  if ((groups[0] & 0xff00) === 0xff00) return false; // multicast
  return true;
}

/** True only for globally routable unicast addresses. Unparseable input is never public. */
export function isPublicIp(ip: string) {
  const v4 = parseIpv4(ip.trim());
  if (v4) return publicIpv4(v4);
  const v6 = parseIpv6(ip);
  return v6 ? publicIpv6(v6) : false;
}

const RESERVED_HOST = /(?:^|\.)(?:localhost|localdomain|local|internal|intranet|corp|home|lan|test|invalid|example|onion|arpa|chatgpt\.site|workers\.dev)$/i;

/** Syntactic policy for a user-supplied URL; throws a user-facing error when the URL is not a public web page. */
export function publicUrl(value: string): URL {
  let u: URL;
  try { u = new URL(value); } catch { throw new Error('Use a public HTTP or HTTPS page URL.'); }
  const host = u.hostname.toLowerCase();
  if (!['https:', 'http:'].includes(u.protocol) || u.username || u.password) throw new Error('Use a public HTTP or HTTPS page URL.');
  if (u.port && !['80', '443'].includes(u.port)) throw new Error('Use a public HTTP or HTTPS page URL.');
  if (!host.includes('.') || host.length > 253 || host.endsWith('.') || /[:[\]]/.test(host) || parseIpv4(host) || /^(?:0x[0-9a-f]+|\d+)$/i.test(host) || RESERVED_HOST.test(host)) throw new Error('Use a public HTTP or HTTPS page URL.');
  if (!/^[a-z0-9-]+(?:\.[a-z0-9-]+)+$/.test(host)) throw new Error('Use a public HTTP or HTTPS page URL.');
  u.hash = '';
  return u;
}

export const dohResolver: Resolver = async hostname => {
  const answers = await Promise.all(['A', 'AAAA'].map(async type => {
    const response = await fetch(`https://cloudflare-dns.com/dns-query?name=${encodeURIComponent(hostname)}&type=${type}`, { headers: { Accept: 'application/dns-json' }, signal: AbortSignal.timeout(FETCH_LIMITS.dnsTimeoutMs) });
    if (!response.ok) throw new Error('Could not verify the destination.');
    const data = await response.json() as { Status?: number; Answer?: { type: number; data: string }[] };
    if (data.Status !== undefined && data.Status !== 0 && data.Status !== 3) throw new Error('Could not verify the destination.');
    return (data.Answer || []).filter(answer => [1, 28].includes(answer.type)).map(answer => answer.data);
  }));
  return answers.flat();
};

/** Check the URL policy and resolve the host; every resolved address must be public. */
export async function allowed(value: string, resolve: Resolver = dohResolver): Promise<URL> {
  const u = publicUrl(value);
  const ips = await resolve(u.hostname);
  if (!ips.length || ips.some(ip => !isPublicIp(ip))) throw new Error('The destination is not a public web server.');
  return u;
}

/** An abort signal that fires when the parent aborts or after `ms` milliseconds. */
export function withTimeout(signal: AbortSignal | undefined, ms: number): AbortSignal {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(new Error(`The source did not respond within ${Math.round(ms / 1000)} seconds.`)), ms);
  const clear = () => clearTimeout(timer);
  controller.signal.addEventListener('abort', clear, { once: true });
  if (signal) {
    if (signal.aborted) { clear(); controller.abort(signal.reason); }
    else signal.addEventListener('abort', () => controller.abort(signal.reason), { once: true });
  }
  return controller.signal;
}

export type FetchOptions = { resolve?: Resolver; fetch?: FetchLike; maxRedirects?: number; signal?: AbortSignal };

export async function fetchPublicSource(value: string, accept: string, signal?: AbortSignal, options: FetchOptions = {}) {
  const resolve = options.resolve || dohResolver;
  const doFetch = options.fetch || fetch;
  const maxRedirects = options.maxRedirects ?? FETCH_LIMITS.maxRedirects;
  const activeSignal = options.signal || signal;
  let current = value;
  for (let redirects = 0; redirects <= maxRedirects; redirects++) {
    activeSignal?.throwIfAborted();
    const url = await allowed(current, resolve);
    const response = await doFetch(url, { redirect: 'manual', credentials: 'omit', headers: { Accept: accept, 'User-Agent': 'TPE-Private-Alpha/1.0' }, signal: activeSignal });
    if (response.status >= 300 && response.status < 400) {
      const location = response.headers.get('location');
      await response.body?.cancel();
      if (!location) throw new Error('The source returned an empty redirect.');
      let target: URL;
      try { target = new URL(location, url); } catch { throw new Error('The source redirected to an invalid address.'); }
      if (redirects === maxRedirects) break;
      current = target.href;
      continue;
    }
    if (!response.ok) { await response.body?.cancel(); throw new Error(`The source returned HTTP ${response.status}.`); }
    return { response, url: url.href, redirects };
  }
  throw new Error('The source redirected too many times.');
}

/** Read up to `maxBytes` of a body; the rest is discarded and reported as truncation. */
export async function readBounded(body: ReadableStream<Uint8Array> | null, maxBytes: number): Promise<{ bytes: Uint8Array; truncated: boolean }> {
  if (!body) return { bytes: new Uint8Array(), truncated: false };
  const reader = body.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0, truncated = false;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      if (size + value.length > maxBytes) { chunks.push(value.subarray(0, maxBytes - size)); size = maxBytes; truncated = true; await reader.cancel(); break; }
      chunks.push(value); size += value.length;
    }
  } finally { reader.releaseLock(); }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
  return { bytes, truncated };
}

/** Text sources never start with a PDF/ZIP/GZIP/ELF signature and contain no NUL bytes in their first kilobyte (UTF-16 excepted). */
export function looksBinary(bytes: Uint8Array) {
  if (bytes.length >= 4) {
    const head = String.fromCharCode(...bytes.subarray(0, 4));
    if (head.startsWith('%PDF') || head.startsWith('PK\u0003\u0004') || head.startsWith('\u001f\u008b') || head.startsWith('\u007fELF') || head.startsWith('\u0089PNG') || head.startsWith('GIF8') || head.startsWith('ÿØÿ')) return true;
  }
  const utf16 = (bytes[0] === 0xff && bytes[1] === 0xfe) || (bytes[0] === 0xfe && bytes[1] === 0xff);
  if (utf16) return false;
  let nul = 0;
  for (let i = 0; i < Math.min(bytes.length, 1024); i++) if (bytes[i] === 0) nul++;
  return nul > 0;
}
