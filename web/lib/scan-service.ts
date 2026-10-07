/**
 * Browser client for the local scan service (`crates/tpe-scan-service`):
 * discovery from a user-entered port and token (kept for this tab session
 * only), capability detection, scan submission with progress, page windows
 * for long documents, and a plain fallback message when the service is not
 * there. The caller (the workspace) decides between this and the in-browser
 * OCR in `lib/image-ocr.ts`; this module never falls back on its own.
 *
 * Security model: see `docs/DOCLING.md`. The service listens on
 * `127.0.0.1` only, needs a per-launch bearer token, and only answers
 * browser requests whose `Origin` it was started with.
 */
export type ScanServiceConnection = { port: number; token: string };
export type ScanMode = 'ocr' | 'text';
export type ScanBBox = { x0: number; y0: number; x1: number; y1: number };
export type ScanBlock = { text: string; bbox: ScanBBox | null; role: string; column: number };
export type ScanConfidence = { parse: number | null; layout: number | null; ocr: number | null; table: number | null };
export type ScanPage = {
  page: number;
  width: number;
  height: number;
  rotation: number;
  status: string;
  text: string;
  confidence: ScanConfidence | null;
  blocks: ScanBlock[];
  spans: { text: string; bbox: ScanBBox | null; seq: number }[];
  figures: { index: number; kind: string; bbox: ScanBBox | null; width_px: number | null; height_px: number | null }[];
  warnings: string[];
};
export type ScanResult = {
  backend: { name: string; version: string; config_digest: string };
  mode: ScanMode;
  input: 'pdf' | 'png' | 'jpeg';
  pages_total: number;
  pages_scanned: [number, number];
  pages: ScanPage[];
  warnings: string[];
  elapsed_ms: number;
};
export type ScanCapabilities = {
  service: { name: string; version: string; docling_version: string; pid: number; loopback_only: boolean; telemetry: boolean };
  build: { ocr_compiled: boolean; text_layer_compiled: boolean };
  ocr: { available: boolean; engine: string; languages: string[]; reason: string | null; confidence: string };
  text_layer: { available: boolean; reason: string | null };
  models: { provisioned: boolean; missing: string[]; searched: string[]; files: { name: string; path: string | null; bytes: number | null; sha256: string | null }[] };
  pdfium: { configured: string | null; library: string | null; present: boolean };
  limits: { max_body_bytes: number; max_pages: number; max_concurrent: number; scan_timeout_ms: number; worker_memory_growth_mib: number };
  accepts: string[];
  modes: string[];
  origins: string[];
};
export type ScanProgress = { phase: 'uploading' | 'scanning' | 'done'; fraction: number; message: string; window?: [number, number]; pagesTotal?: number };
export type ScanOptions = {
  mode?: ScanMode;
  /** Inclusive 1-based page window; omitted means from page 1 up to the service's page limit. */
  pages?: [number, number];
  signal?: AbortSignal;
  onProgress?: (progress: ScanProgress) => void;
  /** Known capabilities; the body-size limit is checked before uploading. */
  capabilities?: ScanCapabilities | null;
  /** Transport override (tests); the browser default uses XMLHttpRequest for upload progress. */
  fetch?: typeof fetch;
};

/** A failure talking to the service; `code` is stable, `message` is for people. */
export class ScanServiceError extends Error {
  readonly code: string;
  readonly status: number | null;
  readonly retryAfterSeconds: number | null;
  constructor(code: string, message: string, status: number | null = null, retryAfterSeconds: number | null = null) {
    super(message);
    this.name = 'ScanServiceError';
    this.code = code;
    this.status = status;
    this.retryAfterSeconds = retryAfterSeconds;
  }
}

/** sessionStorage key for the remembered connection (this tab only, never persisted). */
export const SESSION_KEY = 'tpe.scan-service.connection';
const MIN_TOKEN_CHARS = 16;

/**
 * Interpret what a person typed for the port and the token. Tolerates
 * `5209`, `:5209`, `127.0.0.1:5209`, `http://localhost:5209/`, a pasted
 * `token: …` line, a `Bearer …` prefix, quotes and surrounding whitespace.
 */
export function parseConnection(portInput: string, tokenInput: string): ScanServiceConnection {
  const portText = String(portInput ?? '').trim().replace(/^https?:\/\//i, '').replace(/\/.*$/, '').trim();
  const colon = portText.lastIndexOf(':');
  const host = colon >= 0 ? portText.slice(0, colon).trim() : '';
  const portPart = (colon >= 0 ? portText.slice(colon + 1) : portText).trim();
  const port = /^\d{1,5}$/.test(portPart) ? Number(portPart) : NaN;
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    throw new ScanServiceError('invalid_port', `Enter the port the scan service printed when it started (a number from 1 to 65535), not "${portInput}".`);
  }
  if (host && !/^(?:127\.0\.0\.1|localhost|\[::1\])$/i.test(host)) {
    throw new ScanServiceError('invalid_port', 'The scan service only runs on this computer (127.0.0.1); enter just its port.');
  }
  let token = String(tokenInput ?? '').trim();
  token = token.replace(/^token\s*(?:\([^)]*\))?\s*:\s*/i, '').replace(/^bearer\s+/i, '').replace(/^["'`]+|["'`]+$/g, '').trim();
  if (token.length < MIN_TOKEN_CHARS || !/^[A-Za-z0-9._~+/=-]+$/.test(token)) {
    throw new ScanServiceError('invalid_token', 'Paste the token the scan service printed once when it started (it is shown right after "token").');
  }
  return { port, token };
}

/** The service's origin for `connection`. */
export function serviceOrigin(connection: ScanServiceConnection): string {
  return `http://127.0.0.1:${connection.port}`;
}

type StorageLike = { getItem(key: string): string | null; setItem(key: string, value: string): void; removeItem(key: string): void };
function sessionStore(storage?: StorageLike): StorageLike | null {
  if (storage) return storage;
  try { return typeof sessionStorage === 'undefined' ? null : sessionStorage; } catch { return null; }
}

/** Keep the connection for this tab session only (never localStorage, never a cookie). */
export function rememberConnection(connection: ScanServiceConnection, storage?: StorageLike): boolean {
  const store = sessionStore(storage);
  if (!store) return false;
  try { store.setItem(SESSION_KEY, JSON.stringify(connection)); return true; } catch { return false; }
}

/** The remembered connection for this tab session, if any and well-formed. */
export function recallConnection(storage?: StorageLike): ScanServiceConnection | null {
  const store = sessionStore(storage);
  if (!store) return null;
  try {
    const raw = store.getItem(SESSION_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<ScanServiceConnection>;
    return parseConnection(String(parsed.port ?? ''), String(parsed.token ?? ''));
  } catch { return null; }
}

/** Forget the remembered connection. */
export function forgetConnection(storage?: StorageLike): void {
  const store = sessionStore(storage);
  try { store?.removeItem(SESSION_KEY); } catch { /* storage unavailable */ }
}

/** What to tell the person when the service cannot be used; always names the in-browser alternative. */
export function describeUnavailable(error: unknown, connection?: ScanServiceConnection | null): string {
  const origin = connection ? serviceOrigin(connection) : 'http://127.0.0.1:<port>';
  const here = typeof location === 'undefined' ? '<this web app origin>' : location.origin;
  const launch = `Start it on this computer with \`tpe-scan-service --origin ${here}\`, then enter the port and token it prints.`;
  const fallback = 'Until then, the in-browser OCR can read the scan here (slower, English only).';
  if (error instanceof ScanServiceError) {
    switch (error.code) {
      case 'unreachable': return `No scan service answered at ${origin}. ${launch} ${fallback}`;
      case 'unauthorized': return `The scan service at ${origin} rejected the token. Enter the token it printed when it started (each launch prints a new one). ${fallback}`;
      case 'origin_not_allowed': return `The scan service at ${origin} was started without this web app's origin. Restart it with \`--origin ${here}\`. ${fallback}`;
      case 'models_not_provisioned': return `The scan service is running but its OCR models are not provisioned: ${error.message} ${fallback}`;
      case 'not_compiled': return `The scan service was built without OCR: ${error.message} ${fallback}`;
      case 'busy': return `The scan service is busy with another scan; try again${error.retryAfterSeconds ? ` in ${error.retryAfterSeconds} s` : ' shortly'}.`;
      case 'cancel_pending': return 'Cancellation could not be confirmed yet. Wait briefly before retrying the scan.';
      case 'timeout': return `The scan took too long and was stopped: ${error.message}`;
      case 'too_large': case 'limit': return `This file is too large for the scan service in one request: ${error.message}`;
      default: return `The scan service reported ${error.code}: ${error.message} ${fallback}`;
    }
  }
  return `The scan service could not be used${error instanceof Error && error.message ? ` (${error.message})` : ''}. ${launch} ${fallback}`;
}

/** Whether OCR can run on the service, and why not when it cannot. */
export function ocrAvailability(capabilities: ScanCapabilities): { available: boolean; reason: string | null; languages: string[] } {
  return { available: Boolean(capabilities.ocr?.available), reason: capabilities.ocr?.available ? null : capabilities.ocr?.reason ?? 'OCR is not available on the scan service.', languages: capabilities.ocr?.languages ?? [] };
}

async function errorFromResponse(response: Response): Promise<ScanServiceError> {
  const retry = Number(response.headers.get('Retry-After'));
  const retryAfter = Number.isFinite(retry) && retry > 0 ? retry : null;
  let code = response.status === 401 ? 'unauthorized' : response.status === 403 ? 'origin_not_allowed' : `http_${response.status}`;
  let message = `The scan service answered HTTP ${response.status}.`;
  try {
    const body = await response.json() as { error?: { code?: string; message?: string } };
    if (body?.error?.code) code = body.error.code;
    if (body?.error?.message) message = body.error.message;
  } catch { /* not JSON */ }
  return new ScanServiceError(code, message, response.status, retryAfter);
}

function unreachable(connection: ScanServiceConnection, cause: unknown): ScanServiceError {
  const detail = cause instanceof Error && cause.message ? ` (${cause.message})` : '';
  return new ScanServiceError('unreachable', `No scan service answered at ${serviceOrigin(connection)}${detail}.`);
}

/** Ask the service what it can do. Throws a `ScanServiceError` with a stable code otherwise. */
export async function detectService(connection: ScanServiceConnection, options: { signal?: AbortSignal; fetch?: typeof fetch } = {}): Promise<ScanCapabilities> {
  const doFetch = options.fetch ?? fetch;
  let response: Response;
  try {
    response = await doFetch(`${serviceOrigin(connection)}/capabilities`, { method: 'GET', headers: { Authorization: `Bearer ${connection.token}` }, signal: options.signal, cache: 'no-store', credentials: 'omit', mode: 'cors' });
  } catch (error) {
    if (options.signal?.aborted) throw error;
    throw unreachable(connection, error);
  }
  if (!response.ok) throw await errorFromResponse(response);
  const capabilities = await response.json() as ScanCapabilities;
  if (capabilities?.service?.name !== 'tpe-scan-service' || !capabilities.limits) {
    throw new ScanServiceError('not_scan_service', `Something answered at ${serviceOrigin(connection)}, but it is not the scan service.`);
  }
  return capabilities;
}

function scanUrl(connection: ScanServiceConnection, options: ScanOptions, id: string): string {
  const query = new URLSearchParams();
  if (options.mode) query.set('mode', options.mode);
  if (options.pages) query.set('pages', `${options.pages[0]}-${options.pages[1]}`);
  query.set('id', id);
  const suffix = query.toString();
  return `${serviceOrigin(connection)}/scan${suffix ? `?${suffix}` : ''}`;
}

function uploadWithProgress(url: string, token: string, file: Blob, options: ScanOptions): Promise<Response> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open('POST', url);
    xhr.responseType = 'text';
    xhr.setRequestHeader('Authorization', `Bearer ${token}`);
    xhr.setRequestHeader('Content-Type', file.type || 'application/octet-stream');
    xhr.upload.onprogress = event => {
      if (event.lengthComputable) options.onProgress?.({ phase: 'uploading', fraction: event.total ? event.loaded / event.total : 0, message: `Sending ${Math.round((100 * event.loaded) / Math.max(event.total, 1))}%…`, window: options.pages });
    };
    xhr.upload.onload = () => options.onProgress?.({ phase: 'scanning', fraction: 1, message: 'Scanning…', window: options.pages });
    const abort = () => xhr.abort();
    const cleanup = () => options.signal?.removeEventListener('abort', abort);
    xhr.onerror = () => { cleanup(); reject(new TypeError('network error')); };
    xhr.onabort = () => { cleanup(); reject(new DOMException('The scan was cancelled.', 'AbortError')); };
    xhr.onload = () => {
      cleanup();
      const headers = new Headers();
      for (const line of xhr.getAllResponseHeaders().trim().split(/[\r\n]+/)) {
        const index = line.indexOf(':');
        if (index > 0) headers.append(line.slice(0, index).trim(), line.slice(index + 1).trim());
      }
      resolve(new Response(xhr.response as string, { status: xhr.status, statusText: xhr.statusText, headers }));
    };
    options.signal?.addEventListener('abort', abort, { once: true });
    if (options.signal?.aborted) { cleanup(); reject(new DOMException('The scan was cancelled.', 'AbortError')); return; }
    try { xhr.send(file); } catch (error) { cleanup(); reject(error); }
  });
}

/**
 * Scan one file (PDF, PNG or JPEG) in one request. Checks the body limit
 * first, reports upload and scan progress, and maps every failure to a
 * `ScanServiceError`.
 */
export async function scanFile(connection: ScanServiceConnection, file: Blob, options: ScanOptions = {}): Promise<ScanResult> {
  options.signal?.throwIfAborted();
  const limit = options.capabilities?.limits?.max_body_bytes;
  if (limit && file.size > limit) {
    throw new ScanServiceError('too_large', `${formatBytes(file.size)} exceeds the service's ${formatBytes(limit)} request limit; split the document or start the service with a larger --max-body-mib.`);
  }
  if (file.size === 0) throw new ScanServiceError('empty', 'The file is empty.');
  options.onProgress?.({ phase: 'uploading', fraction: 0, message: 'Sending to the local scan service…', window: options.pages });
  options.signal?.throwIfAborted();
  const id = crypto.randomUUID();
  const url = scanUrl(connection, options, id);
  const doFetch = options.fetch ?? fetch;
  let cancellation: Promise<void> | undefined;
  const cancel = () => {
    if (cancellation) return;
    // Separate authenticated request: never pass the already-aborted signal.
    // The service acknowledges only after its worker is reaped and slot freed.
    cancellation = (async () => {
      const reply = await doFetch(`${serviceOrigin(connection)}/cancel?id=${encodeURIComponent(id)}`, {
        method: 'POST', headers: { Authorization: `Bearer ${connection.token}` },
        signal: AbortSignal.timeout(6000), cache: 'no-store', credentials: 'omit', mode: 'cors',
      });
      if (!reply.ok) throw await errorFromResponse(reply);
    })().catch(error => {
      throw error instanceof ScanServiceError ? error : new ScanServiceError('cancel_pending', 'The scan service did not confirm cancellation.');
    });
    // Mark handled immediately; the scan still awaits and propagates failures.
    void cancellation.catch(() => {});
  };
  options.signal?.addEventListener('abort', cancel, { once: true });
  try {
    let response: Response;
    try {
      if (!options.fetch && typeof XMLHttpRequest !== 'undefined') {
        response = await uploadWithProgress(url, connection.token, file, options);
      } else {
        response = await doFetch(url, { method: 'POST', headers: { Authorization: `Bearer ${connection.token}`, 'Content-Type': file.type || 'application/octet-stream' }, body: file, signal: options.signal, cache: 'no-store', credentials: 'omit', mode: 'cors' });
        options.onProgress?.({ phase: 'scanning', fraction: 1, message: 'Scanning…', window: options.pages });
      }
    } catch (error) {
      if (options.signal?.aborted || (error instanceof DOMException && error.name === 'AbortError')) { cancel(); await cancellation; throw error; }
      throw unreachable(connection, error);
    }
    if (!response.ok) throw await errorFromResponse(response);
    const result = await response.json() as ScanResult;
    if (options.signal?.aborted) { cancel(); await cancellation; options.signal.throwIfAborted(); }
    if (!Array.isArray(result?.pages)) throw new ScanServiceError('bad_result', 'The scan service returned an unexpected result.');
    options.onProgress?.({ phase: 'done', fraction: 1, message: `Scanned pages ${result.pages_scanned?.[0]}–${result.pages_scanned?.[1]} of ${result.pages_total}.`, window: options.pages, pagesTotal: result.pages_total });
    return result;
  } catch (error) {
    if (options.signal?.aborted) { cancel(); await cancellation; options.signal.throwIfAborted(); }
    throw error;
  } finally {
    options.signal?.removeEventListener('abort', cancel);
  }
}

/**
 * Scan a whole document in page windows no larger than the service's page
 * limit (or `windowPages`), merging the pages in order. Images are one page
 * and take one request. Progress reports each window.
 */
export async function scanDocument(connection: ScanServiceConnection, file: Blob, options: ScanOptions & { windowPages?: number } = {}): Promise<ScanResult> {
  const limitPages = Math.max(1, Math.min(options.windowPages ?? Infinity, options.capabilities?.limits?.max_pages ?? 50));
  const first = options.pages?.[0] ?? 1;
  const last = options.pages?.[1] ?? Infinity;
  const window: [number, number] = [first, Math.min(last, first + limitPages - 1)];
  const progress = (progress: ScanProgress, total: number | undefined) => options.onProgress?.({ ...progress, pagesTotal: total ?? progress.pagesTotal, fraction: total && progress.window ? Math.min(1, (progress.window[0] - 1 + progress.fraction * (progress.window[1] - progress.window[0] + 1)) / Math.min(total, last)) : progress.fraction });
  const merged = await scanFile(connection, file, { ...options, pages: window, onProgress: p => progress(p, undefined) });
  const stop = Math.min(last, merged.pages_total);
  let next = merged.pages_scanned[1] + 1;
  while (next <= stop) {
    options.signal?.throwIfAborted();
    const pages: [number, number] = [next, Math.min(stop, next + limitPages - 1)];
    const part = await scanFile(connection, file, { ...options, pages, onProgress: p => progress(p, merged.pages_total) });
    merged.pages.push(...part.pages);
    merged.warnings.push(...part.warnings.filter(warning => !merged.warnings.includes(warning)));
    merged.elapsed_ms += part.elapsed_ms;
    merged.pages_scanned = [merged.pages_scanned[0], part.pages_scanned[1]];
    next = part.pages_scanned[1] + 1;
  }
  options.onProgress?.({ phase: 'done', fraction: 1, message: `Scanned ${merged.pages.length} of ${merged.pages_total} pages.`, pagesTotal: merged.pages_total });
  return merged;
}

/** The ordered text of a result, pages separated by a form feed line. */
export function scanResultText(result: ScanResult): string {
  return result.pages.map(page => page.text).join('\n\f\n');
}

/** Pages whose status is not `complete`, with their warnings (for review prompts). */
export function partialPages(result: ScanResult): { page: number; warnings: string[] }[] {
  return result.pages.filter(page => page.status !== 'complete').map(page => ({ page: page.page, warnings: page.warnings }));
}

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KiB`;
  return `${bytes} bytes`;
}
