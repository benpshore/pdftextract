/**
 * Node regression for lib/scan-service.ts with a fake service (no network,
 * no browser): connection parsing, session-only storage, capability
 * detection, scan submission with progress, page windows, error mapping and
 * the fallback message. Run from web/: `node tests/scan-service.test.mjs`.
 */
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdtemp, rm } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';
import ts from 'typescript';

const web = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const temporary = await mkdtemp(join(tmpdir(), 'tpe-scan-service-'));
const checks = [];
const check = (condition, message) => { assert(condition, message); checks.push(message); };

try {
  const source = await readFile(join(web, 'lib', 'scan-service.ts'), 'utf8');
  const { outputText } = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } });
  await writeFile(join(temporary, 'scan-service.mjs'), outputText);
  const lib = await import(pathToFileURL(join(temporary, 'scan-service.mjs')));
  const { parseConnection, serviceOrigin, rememberConnection, recallConnection, forgetConnection, detectService, scanFile, scanDocument, describeUnavailable, ocrAvailability, scanResultText, partialPages, ScanServiceError, SESSION_KEY } = lib;

  // A fixed test fixture value, not a real credential.
  const TOKEN = 'test-fixture-token-0123456789abcdef';

  // Connection parsing is tolerant of what people paste.
  for (const input of ['5209', ' :5209 ', '127.0.0.1:5209', 'http://localhost:5209/', 'http://[::1]:5209/capabilities']) {
    const connection = parseConnection(input, TOKEN);
    check(connection.port === 5209 && connection.token === TOKEN, `port parsed from ${JSON.stringify(input)}`);
  }
  check(parseConnection('5209', `token (shown once; paste it): ${TOKEN}`).token === TOKEN, 'pasted token line is cleaned');
  check(parseConnection('5209', `Bearer "${TOKEN}" `).token === TOKEN, 'Bearer prefix and quotes are stripped');
  for (const [port, token, code] of [['', TOKEN, 'invalid_port'], ['70000', TOKEN, 'invalid_port'], ['evil.example:5209', TOKEN, 'invalid_port'], ['5209', 'short', 'invalid_token'], ['5209', 'has spaces inside the token value', 'invalid_token']]) {
    assert.throws(() => parseConnection(port, token), error => error instanceof ScanServiceError && error.code === code, `${port}/${token} -> ${code}`);
    checks.push(`rejects ${JSON.stringify(port)} / ${JSON.stringify(token)} as ${code}`);
  }
  check(serviceOrigin({ port: 5209, token: TOKEN }) === 'http://127.0.0.1:5209', 'service origin is loopback http');

  // Session-only storage, tolerant of a broken storage.
  const store = new Map();
  const storage = { getItem: key => store.has(key) ? store.get(key) : null, setItem: (key, value) => store.set(key, value), removeItem: key => store.delete(key) };
  check(rememberConnection({ port: 5209, token: TOKEN }, storage) === true, 'connection remembered');
  check(store.has(SESSION_KEY) && recallConnection(storage).port === 5209, 'connection recalled from session storage');
  store.set(SESSION_KEY, '{"port":"nope"}');
  check(recallConnection(storage) === null, 'garbled stored connection is ignored');
  forgetConnection(storage);
  check(!store.has(SESSION_KEY), 'connection forgotten');
  const broken = { getItem() { throw new Error('blocked'); }, setItem() { throw new Error('blocked'); }, removeItem() { throw new Error('blocked'); } };
  check(rememberConnection({ port: 1, token: TOKEN }, broken) === false && recallConnection(broken) === null, 'blocked storage never throws');
  forgetConnection(broken);

  // A fake service.
  const capabilities = {
    service: { name: 'tpe-scan-service', version: '0.2.0', docling_version: '1.69.2', pid: 1, loopback_only: true, telemetry: false },
    build: { ocr_compiled: true, text_layer_compiled: true },
    ocr: { available: false, engine: 'ppocr', languages: [], reason: 'models not provisioned: missing ocr_det.onnx; run `sh native/fetch.sh`', confidence: 'per-page' },
    text_layer: { available: true, reason: null },
    models: { provisioned: false, missing: ['ocr_det.onnx'], searched: [], files: [] },
    pdfium: { configured: null, library: null, present: false },
    limits: { max_body_bytes: 1024, max_pages: 2, max_concurrent: 1, scan_timeout_ms: 1000, worker_memory_growth_mib: 1 },
    accepts: ['application/pdf'], modes: ['ocr', 'text'], origins: ['http://localhost:5209'],
  };
  const page = number => ({ page: number, width: 612, height: 792, rotation: 0, status: number === 3 ? 'partial' : 'complete', text: `page ${number}`, confidence: { parse: null, layout: 0.8, ocr: 0.9, table: null }, blocks: [], spans: [], figures: [], warnings: number === 3 ? ['extraction_incomplete: x'] : [] });
  const requests = [];
  const json = (status, body, headers = {}) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json', ...headers } });
  let behaviour = 'ok';
  const fakeFetch = async (url, init = {}) => {
    const parsed = new URL(url);
    requests.push({ url: String(url), method: init.method, auth: init.headers?.Authorization, body: init.body });
    if (behaviour === 'down') throw new TypeError('fetch failed');
    if (init.headers?.Authorization !== `Bearer ${TOKEN}`) return json(401, { error: { code: 'unauthorized', message: 'token' } });
    if (parsed.pathname === '/capabilities') return json(200, capabilities);
    if (parsed.pathname === '/scan') {
      if (behaviour === 'models') return json(503, { error: { code: 'models_not_provisioned', message: 'models not provisioned: run native/fetch.sh' } });
      if (behaviour === 'busy') return json(429, { error: { code: 'busy', message: 'one scan running' } }, { 'Retry-After': '5' });
      if (behaviour === 'forbidden') return json(403, { error: { code: 'origin_not_allowed', message: 'origin' } });
      if (behaviour === 'junk') return new Response('<html>', { status: 200 });
      const [first, last] = (parsed.searchParams.get('pages') ?? '1-2').split('-').map(Number);
      const total = 5;
      const pages = [];
      for (let number = first; number <= Math.min(last, total); number++) pages.push(page(number));
      return json(200, { backend: { name: 'docling', version: '1.69.2', config_digest: 'x' }, mode: parsed.searchParams.get('mode') ?? 'ocr', input: 'pdf', pages_total: total, pages_scanned: [first, Math.min(last, total)], pages, warnings: ['w'], elapsed_ms: 10 });
    }
    return json(404, { error: { code: 'not_found', message: 'nope' } });
  };
  const connection = { port: 5209, token: TOKEN };

  const detected = await detectService(connection, { fetch: fakeFetch });
  check(detected.service.name === 'tpe-scan-service' && requests.at(-1).url === 'http://127.0.0.1:5209/capabilities', 'capabilities fetched from the loopback origin with the bearer token');
  const availability = ocrAvailability(detected);
  check(availability.available === false && /native\/fetch\.sh/.test(availability.reason), 'OCR unavailability carries the provisioning reason');
  await assert.rejects(detectService({ port: 5209, token: 'wrong-token-0123456789abcdef' }, { fetch: fakeFetch }), error => error.code === 'unauthorized' && error.status === 401);
  checks.push('a wrong token is reported as unauthorized');
  behaviour = 'down';
  await assert.rejects(detectService(connection, { fetch: fakeFetch }), error => error.code === 'unreachable');
  checks.push('an absent service is reported as unreachable');
  check(/No scan service answered at http:\/\/127\.0\.0\.1:5209/.test(describeUnavailable(new ScanServiceError('unreachable', 'x'), connection)) && /in-browser OCR/.test(describeUnavailable(new ScanServiceError('unreachable', 'x'), connection)), 'fallback message names the origin, the launch command and the in-browser OCR');
  check(/--origin/.test(describeUnavailable(new ScanServiceError('origin_not_allowed', 'x'), connection)), 'origin problem explains --origin');
  check(/not provisioned/.test(describeUnavailable(new ScanServiceError('models_not_provisioned', 'models not provisioned: run native/fetch.sh'), connection)), 'provisioning problem is passed through');
  check(/in 5 s/.test(describeUnavailable(new ScanServiceError('busy', 'x', 429, 5), connection)), 'busy message uses Retry-After');
  check(/could not be used/.test(describeUnavailable(new Error('boom'), null)), 'unknown errors still get a fallback message');
  behaviour = 'ok';

  // Scanning with progress, limits and errors.
  const file = new Blob([new Uint8Array(100)], { type: 'application/pdf' });
  const progress = [];
  const result = await scanFile(connection, file, { fetch: fakeFetch, mode: 'ocr', pages: [1, 2], capabilities: detected, onProgress: p => progress.push(p.phase) });
  check(result.pages.length === 2 && result.pages_scanned[1] === 2, 'single-window scan returns its pages');
  check(requests.at(-1).url === 'http://127.0.0.1:5209/scan?mode=ocr&pages=1-2' && requests.at(-1).body === file, 'scan posts the file with mode and pages');
  check(progress[0] === 'uploading' && progress.includes('scanning') && progress.at(-1) === 'done', `progress phases ${progress.join(',')}`);
  await assert.rejects(scanFile(connection, new Blob([new Uint8Array(2048)]), { fetch: fakeFetch, capabilities: detected }), error => error.code === 'too_large');
  checks.push('oversized files are refused before upload');
  await assert.rejects(scanFile(connection, new Blob([]), { fetch: fakeFetch }), error => error.code === 'empty');
  checks.push('empty files are refused');
  behaviour = 'models';
  await assert.rejects(scanFile(connection, file, { fetch: fakeFetch }), error => error.code === 'models_not_provisioned' && error.status === 503 && /native\/fetch\.sh/.test(error.message));
  checks.push('503 models_not_provisioned maps to its code and message');
  behaviour = 'busy';
  await assert.rejects(scanFile(connection, file, { fetch: fakeFetch }), error => error.code === 'busy' && error.retryAfterSeconds === 5);
  checks.push('429 busy carries Retry-After');
  behaviour = 'forbidden';
  await assert.rejects(scanFile(connection, file, { fetch: fakeFetch }), error => error.code === 'origin_not_allowed');
  checks.push('403 maps to origin_not_allowed');
  behaviour = 'junk';
  await assert.rejects(scanFile(connection, file, { fetch: fakeFetch }), error => error.code === 'bad_result' || error.code === 'unreachable' || error instanceof Error);
  checks.push('a non-JSON 200 does not pass as a result');
  behaviour = 'ok';
  const aborted = new AbortController();
  aborted.abort();
  await assert.rejects(scanFile(connection, file, { fetch: fakeFetch, signal: aborted.signal }), error => error.name === 'AbortError');
  checks.push('an aborted signal rejects before sending');

  // Page windows over the service's page limit.
  const windows = [];
  const whole = await scanDocument(connection, file, { fetch: fakeFetch, capabilities: detected, onProgress: p => { if (p.phase === 'done') windows.push(p.pagesTotal); } });
  check(whole.pages.length === 5 && whole.pages.map(p => p.page).join(',') === '1,2,3,4,5', 'document scanned in windows of the page limit and merged in order');
  check(whole.pages_scanned[0] === 1 && whole.pages_scanned[1] === 5 && whole.warnings.length === 1, 'merged result covers every page and dedups warnings');
  const windowUrls = requests.slice(-3).map(r => new URL(r.url).searchParams.get('pages'));
  check(windowUrls.join(' ') === '1-2 3-4 5-5', `windows requested: ${windowUrls.join(' ')}`);
  const partial = await scanDocument(connection, file, { fetch: fakeFetch, capabilities: detected, pages: [2, 3] });
  check(partial.pages.map(p => p.page).join(',') === '2,3', 'explicit page range is respected across windows');
  check(scanResultText(whole).split('\f').length === 5 && partialPages(whole).length === 1 && partialPages(whole)[0].page === 3, 'text and partial-page helpers');
  console.log(`scan-service: ${checks.length} checks passed`);
} finally {
  await rm(temporary, { recursive: true, force: true });
}
