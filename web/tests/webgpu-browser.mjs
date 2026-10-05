/** Actual browser regression for the WebGPU kernels against the CPU fallback, the detection path and
 * the OCR integration. Runs from `web/`: `node tests/webgpu-browser.mjs`.
 * Needs Playwright Chromium (PLAYWRIGHT_MODULE may point to a provisioned module; PLAYWRIGHT_BROWSERS_PATH
 * to its browsers). Headless Chromium gets a SwiftShader WebGPU adapter from
 * `--enable-unsafe-webgpu --enable-features=Vulkan --use-angle=vulkan --use-vulkan=swiftshader`; when no
 * adapter appears the test still verifies detection and the fallback, and says so in its output.
 * The end-to-end OCR check needs `node scripts/copy-ocr-assets.mjs` first and is skipped otherwise.
 * Chromium only: it says nothing about Safari, iOS or a real GPU. No network: foreign requests are aborted. */
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, rm, access } from 'node:fs/promises';
import { createReadStream } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { createRequire } from 'node:module';
import { web, transpileSources } from './webgpu-helpers.mjs';
const require = createRequire(import.meta.url);
const playwright = await import(process.env.PLAYWRIGHT_MODULE || (await (async () => { try { return require.resolve('playwright'); } catch { return '/opt/node22/lib/node_modules/playwright/index.mjs'; } })()));
const chromium = playwright.chromium ?? playwright.default?.chromium;
if (!chromium) throw new Error('Playwright Chromium is unavailable; set PLAYWRIGHT_MODULE.');
const temporary = await mkdtemp(join(tmpdir(), 'tpe-webgpu-browser-'));
await transpileSources(temporary);
const ocrAssets = await access(join(web, 'public/ocr/7.0.0/worker.min.js')).then(() => true, () => false);
const server = createServer(async (request, response) => {
  const url = new URL(request.url, 'http://localhost');
  if (url.pathname.includes('..')) { response.statusCode = 400; response.end(); return; }
  if (url.pathname === '/') { response.setHeader('Content-Type', 'text/html'); response.end('<!doctype html><title>WebGPU pre-processing regression</title>'); return; }
  const serve = (path, type) => { response.setHeader('Content-Type', type); const stream = createReadStream(path); stream.on('error', () => { response.statusCode = 404; response.end(); }); stream.pipe(response); };
  if (url.pathname.startsWith('/lib/')) return serve(join(temporary, url.pathname), 'text/javascript');
  if (url.pathname.startsWith('/fixtures/')) return serve(join(web, 'scripts', url.pathname), 'image/png');
  if (url.pathname.startsWith('/ocr/7.0.0/')) return serve(join(web, 'public', url.pathname), url.pathname.endsWith('.js') ? 'text/javascript' : 'application/octet-stream');
  response.statusCode = 404; response.end();
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const GPU_ARGS = ['--no-sandbox', '--enable-unsafe-webgpu', '--enable-features=Vulkan', '--use-angle=vulkan', '--use-vulkan=swiftshader', '--ignore-gpu-blocklist'];
const summary = { gpuTested: false, checks: [], console: [] };

async function open(browser) {
  const context = await browser.newContext();
  const foreign = [];
  await context.route('**/*', async route => { const url = route.request().url(); if (url.startsWith(origin + '/')) await route.continue(); else { foreign.push(url); await route.abort(); } });
  const page = await context.newPage();
  page.on('console', message => { if (message.text().startsWith('[ocr]')) summary.console.push(message.text()); });
  const errors = []; page.on('pageerror', error => errors.push(String(error)));
  await page.goto(origin);
  return { context, page, foreign, errors };
}

/** Browser-side test body: compares every kernel on three fixtures, then the entry point and OCR. */
const body = async ({ ocrAssets }) => {
  const kernels = await import('/lib/webgpu/index.js');
  const ocr = await import('/lib/image-ocr.js');
  const checks = [], check = (condition, message) => { if (!condition) throw new Error(`FAILED: ${message}`); checks.push(message); };
  const maxDiff = (a, b) => { if (a.length !== b.length) return Infinity; let worst = 0; for (let i = 0; i < a.length; i++) { const d = Math.abs(a[i] - b[i]); if (d > worst) worst = d; } return worst; };
  const status = await kernels.detectWebGpu();
  const report = { status, fixtures: {}, checks, verdict: null, ocr: null };
  // Fixtures: the committed OCR PNG, canvas text tilted by 2°, and odd-sized colour noise.
  const fromCanvas = (width, height, draw) => { const canvas = document.createElement('canvas'); canvas.width = width; canvas.height = height; const context = canvas.getContext('2d'); context.fillStyle = '#fff'; context.fillRect(0, 0, width, height); draw(context); return { canvas, data: context.getImageData(0, 0, width, height).data, width, height }; };
  const still = await createImageBitmap(await (await fetch('/fixtures/ocr-still.png')).blob());
  const fixtures = {
    'ocr-still.png': fromCanvas(still.width, still.height, context => context.drawImage(still, 0, 0)),
    'canvas-text-2deg': fromCanvas(1200, 700, context => { context.translate(600, 350); context.rotate(2 * Math.PI / 180); context.fillStyle = '#111'; context.font = 'bold 40px sans-serif'; const lines = ['WEBGPU PREPROCESS CHECK', 'Reference 10.1000/example', 'Pixels stay on this device.', 'Projection profiles find the tilt.', 'Otsu picks the ink threshold.']; lines.forEach((line, index) => context.fillText(line, -520, -140 + index * 70)); }),
    'noise-1023x517': (() => { const width = 1023, height = 517, data = new Uint8ClampedArray(width * height * 4); let state = 0x1234567; for (let i = 0; i < data.length; i++) { state ^= state << 13; state >>>= 0; state ^= state >>> 17; state ^= state << 5; state >>>= 0; data[i] = state & 255; } return { data, width, height }; })(),
  };
  const cpuStages = {};
  for (const [name, fixture] of Object.entries(fixtures)) cpuStages[name] = kernels.preprocessCpu(fixture.data, fixture.width, fixture.height);
  check(Math.abs(cpuStages['canvas-text-2deg'].skew.degrees - 2) <= 0.25, `CPU skew estimate recovers the 2° canvas text tilt (${cpuStages['canvas-text-2deg'].skew.degrees}°)`);
  check(Math.abs(cpuStages['ocr-still.png'].skew.degrees) <= 0.25, `CPU skew estimate finds the fixture straight (${cpuStages['ocr-still.png'].skew.degrees}°)`);
  if (status.state !== 'available') {
    const fallback = await kernels.preprocessForOcr(fixtures['ocr-still.png'].data, still.width, still.height, { mode: 'auto', log: () => {} });
    check(fallback.runtime === 'cpu' && fallback.fallbackReason === status.reason, `Entry point falls back to the CPU and reports the detection reason: ${status.reason}`);
    report.gpuTested = false;
    return report;
  }
  const acquired = await kernels.acquireWebGpuDevice();
  check(acquired && acquired.device, 'A GPU device was created');
  for (const [name, fixture] of Object.entries(fixtures)) {
    const cpu = cpuStages[name], gpu = await kernels.preprocessGpu(acquired.device, fixture.data, fixture.width, fixture.height);
    const diffs = { gray: maxDiff(gpu.gray, cpu.gray), histogram: maxDiff(gpu.histogram, cpu.histogram), threshold: Math.abs(gpu.threshold - cpu.threshold), profiles: maxDiff(gpu.profiles, cpu.profiles), skewDegrees: Math.abs(gpu.skew.degrees - cpu.skew.degrees) };
    report.fixtures[name] = { width: fixture.width, height: fixture.height, diffs, gpuMs: gpu.timings, cpuMs: cpu.timings, threshold: gpu.threshold, skewDegrees: gpu.skew.degrees, confidence: gpu.skew.confidence };
    check(diffs.gray === 0 && diffs.histogram === 0 && diffs.threshold === 0 && diffs.profiles === 0 && diffs.skewDegrees === 0, `${name}: GPU pipeline equals the CPU pipeline bit for bit (tolerance 0; max diffs ${JSON.stringify(diffs)})`);
    const gray = await kernels.grayscaleGpu(acquired.device, fixture.data, fixture.width * fixture.height);
    const histogram = await kernels.histogramGpu(acquired.device, gray);
    const profiles = await kernels.profilesGpu(acquired.device, gray, fixture.width, fixture.height, cpu.threshold, cpu.table, cpu.stride);
    check(maxDiff(gray, cpu.gray) === 0 && maxDiff(histogram, cpu.histogram) === 0 && maxDiff(profiles, cpu.profiles) === 0, `${name}: standalone grayscale, histogram and profile kernels equal the CPU kernels`);
  }
  // A signal aborted before the call rejects with its reason; one aborted afterwards has no effect.
  const early = new AbortController(); early.abort();
  const earlyRejected = await kernels.preprocessForOcr(fixtures['noise-1023x517'].data, 1023, 517, { signal: early.signal, log: () => {}, mode: 'webgpu' }).then(() => false, error => error.name === 'AbortError');
  const late = new AbortController();
  const lateResult = await kernels.preprocessForOcr(fixtures['noise-1023x517'].data, 1023, 517, { signal: late.signal, log: () => {}, crossCheck: false, mode: 'webgpu' });
  late.abort();
  check(earlyRejected && lateResult.runtime === 'webgpu', 'Abort before the call rejects with AbortError; abort after completion changes nothing');
  // Entry point with the one-time cross-check and timing comparison.
  kernels.resetPreprocessVerdict();
  const lines = [];
  const auto = await kernels.preprocessForOcr(fixtures['ocr-still.png'].data, still.width, still.height, { mode: 'auto', log: line => lines.push(line) });
  report.verdict = kernels.preprocessVerdict();
  check(report.verdict && report.verdict.identical === true && lines.length === 1 && lines[0].includes('WebGPU pre-processing check'), `One-time cross-check ran once, results identical, logged: ${lines[0]}`);
  check(auto.crossCheck === report.verdict && (auto.runtime === 'webgpu' || auto.runtime === 'cpu' && report.verdict.decision === 'cpu'), `Auto mode honours the verdict (runtime ${auto.runtime}: ${report.verdict.reason})`);
  const forced = await kernels.preprocessForOcr(fixtures['ocr-still.png'].data, still.width, still.height, { mode: 'webgpu', log: () => {} });
  check(forced.runtime === 'webgpu' && forced.fallbackReason === null && maxDiff(forced.gray, auto.gray) === 0, 'webgpu mode keeps using the adapter after the verdict and returns the same bytes');
  const line = await kernels.accelerationStatusLine();
  check(line.includes('WebGPU') && line.includes('Nothing leaves this device'), `Accessible status line: ${line}`);
  report.statusLine = line;
  if (ocrAssets) {
    const canvas = fixtures['canvas-text-2deg'].canvas;
    const file = new File([await new Promise(resolve => canvas.toBlob(resolve, 'image/png'))], 'tilted.png', { type: 'image/png' });
    ocr.setOcrAcceleration('webgpu');
    const started = performance.now(), progress = [];
    const result = await ocr.recognizeImage(file, event => progress.push(event.status));
    const pre = result.metadata.ocr.preprocessing;
    report.ocr = { elapsedMs: performance.now() - started, engine: result.engine, preprocessing: pre, text: result.text, progress: [...new Set(progress)] };
    check(pre.runtime === 'webgpu' && pre.deskewed === true && Math.abs(pre.skewDegrees - 2) <= 0.25 && pre.timings.totalMs > 0, `OCR used WebGPU pre-processing and deskewed by ${pre.skewDegrees}° in ${pre.timings.totalMs.toFixed(1)} ms`);
    check(/WEBGPU PREPROCESS CHECK/.test(result.text) && result.text.includes('10.1000/example') && result.links[0]?.doi === '10.1000/example', 'Tesseract recognised the deskewed text and the DOI');
    check(result.engine.includes('WebGPU pre-processing') && result.warnings.some(warning => warning.includes('rotated')) && typeof pre.status === 'string', 'Result labels the pre-processing runtime and discloses the rotation');
    ocr.setOcrAcceleration('off');
    const plain = await ocr.recognizeImage(file);
    report.ocr.withoutPreprocessing = { engine: plain.engine, runtime: plain.metadata.ocr.preprocessing.runtime, text: plain.text };
    check(plain.metadata.ocr.preprocessing.runtime === 'none' && plain.engine === 'Tesseract.js 7.0.0 (CPU/WASM)', 'The off setting hands the image to the recognizer unchanged');
  }
  kernels.releaseWebGpuDevice();
  report.gpuTested = true;
  return report;
};

let browser;
try {
  browser = await chromium.launch({ headless: true, args: GPU_ARGS });
  const { context, page, foreign, errors } = await open(browser);
  const report = await page.evaluate(body, { ocrAssets });
  summary.gpuTested = report.gpuTested; summary.checks.push(...report.checks); summary.status = report.status; summary.fixtures = report.fixtures; summary.verdict = report.verdict; summary.ocr = report.ocr; summary.statusLine = report.statusLine;
  assert.deepEqual(foreign, [], 'no foreign requests');
  assert.deepEqual(errors, [], 'no page errors');
  await context.close();
  await browser.close();
  // Plain headless Chromium (no flags): detection must explain the absence instead of failing.
  browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] });
  const plain = await open(browser);
  const detection = await plain.page.evaluate(async () => { const kernels = await import('/lib/webgpu/index.js'); const status = await kernels.detectWebGpu(); const result = await kernels.preprocessForOcr(new Uint8ClampedArray(16 * 16 * 4).fill(200), 16, 16, { log: () => {} }); return { status, runtime: result.runtime, reason: result.fallbackReason }; });
  summary.plainChromium = detection;
  assert.equal(detection.runtime, 'cpu');
  if (detection.status.state === 'unavailable') { assert.equal(detection.reason, detection.status.reason); summary.checks.push(`Plain headless Chromium without flags: ${detection.status.reason}; fallback used`); }
  else summary.checks.push('Plain headless Chromium exposed WebGPU as well');
  await plain.context.close();
  summary.conclusion = summary.gpuTested ? 'WebGPU (SwiftShader fallback adapter) path tested against the CPU path: identical results' : `WebGPU unavailable in this headless Chromium (${summary.status?.reason}); detection and the CPU fallback were tested, the GPU kernels were not executed`;
  console.log(JSON.stringify(summary, null, 2));
} finally { await browser?.close(); server.close(); await rm(temporary, { recursive: true, force: true }); }
