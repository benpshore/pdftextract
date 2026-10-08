/** Real Upload -> local Workers D1/R2 -> CPU/WASM OCR -> reader -> saved result.
 * Requires a running local `pnpm dev` and Playwright (PLAYWRIGHT_MODULE can point
 * to an external install). Only synthetic pixels are uploaded. No OCR mocks.
 * Worker instrumentation counts ownership and observes progress; recognition
 * still runs in the actual pinned same-origin Tesseract worker.
 */
import assert from 'node:assert/strict';
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const base = process.env.TPE_BASE_URL || 'http://127.0.0.1:5173';
const web = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const checks = [], witnesses = [], unexpected = [], assetRequests = [];
const check = (condition, name) => { assert(condition, name); checks.push(name); };
const runId = Date.now().toString(36);
const sourceIdentity = {
  checkoutCommit: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: web, encoding: 'utf8' }).trim(),
  checkoutTree: execFileSync('git', ['rev-parse', 'HEAD^{tree}'], { cwd: web, encoding: 'utf8' }).trim(),
  files: ['app/workspace.tsx', 'lib/image-ocr.ts', 'scripts/test-browser-ocr.mjs'].map(path => ({ path, sha256: createHash('sha256').update(readFileSync(join(web, path))).digest('hex') })),
  ocrAssetManifestSha256: createHash('sha256').update(readFileSync(join(web, 'public/ocr/7.0.0/manifest.json'))).digest('hex'),
  app: 'Local pnpm dev from this web checkout; production Site not tested',
};
for (let attempt = 0; ; attempt++) {
  const response = await fetch(base + '/api/documents?q=', { headers: { cookie: '__sites_local_auth=1' } }).catch(() => null);
  if (response) {
    const text = await response.text();
    if (text.includes('no such table')) {
      const { DatabaseSync } = await import('node:sqlite');
      const dir = join(web, '.wrangler/state/v3/d1/miniflare-D1DatabaseObject');
      const file = readdirSync(dir).find(n => n.endsWith('.sqlite') && n !== 'metadata.sqlite');
      const db = new DatabaseSync(join(dir, file));
      for (const sql of readdirSync(join(web, 'drizzle')).filter(n => n.endsWith('.sql')).sort()) db.exec(readFileSync(join(web, 'drizzle', sql), 'utf8').replaceAll('--> statement-breakpoint', ''));
      db.close();
    } else assert(response.ok, 'Local app must expose its mock sign-in and storage, not a deployed Site');
    break;
  }
  if (attempt >= 120) throw new Error('No local app: start pnpm dev before this test.');
  await new Promise(resolve => setTimeout(resolve, 500));
}
const browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_EXECUTABLE_PATH ? { executablePath: process.env.PLAYWRIGHT_EXECUTABLE_PATH } : {}), args: ['--no-sandbox'] });
async function boot() {
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  context.on('request', request => {
    const url = request.url();
    if (!url.startsWith(base + '/') && !/^(?:data|blob):/.test(url)) unexpected.push(url);
    if (url.startsWith(base + '/ocr/')) assetRequests.push(url.slice(base.length));
  });
  await context.addInitScript(() => {
    window.ocrWitness = { created: 0, active: 0, progress: [], cancelOnRecognition: false, cancelledOnRecognition: false };
    const NativeWorker = window.Worker;
    window.Worker = class extends NativeWorker {
      constructor(url, options) {
        super(url, options);
        if (!String(url).includes('/ocr/')) return;
        this.ocrOwned = true; window.ocrWitness.created++; window.ocrWitness.active++;
        this.addEventListener('message', event => {
          if (event.data?.status !== 'progress') return;
          window.ocrWitness.progress.push(event.data.data);
          if (window.ocrWitness.cancelOnRecognition && event.data.data?.status === 'recognizing text') {
            window.ocrWitness.cancelOnRecognition = false;
            const button = document.querySelector('button[aria-label^="Cancel "]');
            if (button) { window.ocrWitness.cancelledOnRecognition = true; button.click(); }
          }
        });
      }
      terminate() {
        if (this.ocrOwned) { this.ocrOwned = false; window.ocrWitness.active--; }
        return super.terminate();
      }
    };
  });
  const page = await context.newPage(), errors = [];
  page.on('pageerror', error => errors.push(String(error)));
  await page.goto(base + '/signin-with-chatgpt?return_to=%2F');
  await page.waitForSelector('textarea#source-paste', { timeout: 60000 });
  for (let attempt = 0; attempt < 60; attempt++) {
    await page.fill('textarea#source-paste', 'x');
    if (await page.locator('button[aria-label="Import pasted source"]').isEnabled()) break;
    await page.waitForTimeout(100);
  }
  await page.fill('textarea#source-paste', '');
  return { context, page, errors };
}
async function fixture(page, { font = 42, angle = 3, blank = false, type = 'image/jpeg', multi = false } = {}) {
  return Buffer.from(await page.evaluate(async ({ font, angle, blank, type, multi }) => {
    const canvas = document.createElement('canvas'); canvas.width = 1500; canvas.height = 700;
    const c = canvas.getContext('2d'); c.fillStyle = '#b7b4aa'; c.fillRect(0, 0, 1500, 700);
    c.translate(750, 350); c.rotate(angle * Math.PI / 180); c.fillStyle = '#dbd9cf'; c.fillRect(-680, -280, 1360, 560);
    c.fillStyle = '#484945'; c.font = font + 'px Arial'; c.filter = 'blur(0.8px)';
    if (!blank) for (const [i, line] of (multi ? ['PHOTO OCR CHECK', 'Reference 10.1000/example', 'Local pixels stay on this device.'] : ['PHOTO OCR CHECK']).entries()) c.fillText(line, -590, -120 + i * 105);
    if (blank) { c.setTransform(1, 0, 0, 1, 0, 0); c.filter = 'none'; c.fillStyle = '#fff'; c.fillRect(0, 0, 1500, 700); }
    const blob = await new Promise(resolve => canvas.toBlob(resolve, type, 0.55));
    return Array.from(new Uint8Array(await blob.arrayBuffer()));
  }, { font, angle, blank, type, multi }));
}
async function upload(page, buffer, name, mimeType = 'image/jpeg') {
  await page.getByRole('button', { name: 'Upload', exact: true }).click();
  const [picker] = await Promise.all([page.waitForEvent('filechooser'), page.getByRole('button', { name: 'Add photos', exact: true }).click()]);
  await picker.setFiles({ name, mimeType, buffer });
}
async function terminal(page, name, phase = 'saved') {
  const row = page.locator('.queue-item').filter({ has: page.locator('.queue-open strong', { hasText: name }) });
  await row.locator('.phase-' + phase).waitFor({ timeout: 150000 });
  await row.locator('.queue-open').click();
  const id = new URL(page.url()).searchParams.get('document');
  assert(id, 'The saved original must have a document ID');
  const response = await page.request.get(base + '/api/documents/' + id); assert(response.ok());
  return { id, ...(await response.json()) };
}
async function preserved(page, data, buffer) {
  const response = await page.request.get(base + '/api/documents/' + data.id + '/original'); assert(response.ok());
  const original = await response.body(); assert.deepEqual(original, buffer, 'Saved original must be byte-for-byte unchanged');
  assert.equal(data.record.sha256, createHash('sha256').update(buffer).digest('hex'));
}
async function cleanFinish(session) {
  check(await session.page.evaluate(() => window.ocrWitness.active === 0), 'Every owned OCR worker is terminated at the terminal state');
  check(session.errors.length === 0, 'No uncaught browser errors');
  await session.context.close();
}
try {
  const session = await boot(), { page } = session;
  for (const font of [14, 18]) {
    const buffer = await fixture(page, { font }), name = `imperfect-${font}-${runId}.jpg`;
    await upload(page, buffer, name);
    const data = await terminal(page, name); await preserved(page, data, buffer);
    check(data.result.text === 'PHOTO OCR CHECK' && (await page.locator('.plain-reading').innerText()).includes('PHOTO OCR CHECK'), `${font}px blurred, tilted JPEG reaches actual Upload, OCR, saved output and visible reader`);
    check(data.result.metadata.ocr.attempts[0].characters === 0 && data.result.metadata.ocr.attempts[1].segmentation === 'sparse', 'Sparse fallback recovers a witnessed empty first pass');
    check(data.result.status === 'partial' && (await page.locator('#reader').innerText()).includes('review it against the original'), 'OCR uncertainty is visible beside the transcription');
    witnesses.push({ name, documentId: data.id, bytes: buffer.length, sha256: data.record.sha256, text: data.result.text, ocr: data.result.metadata.ocr });
  }
  for (const [type, extension] of [['image/png', 'png'], ['image/jpeg', 'jpg'], ['image/webp', 'webp']]) {
    const buffer = await fixture(page, { multi: true, type }), name = `control-${runId}.${extension}`;
    await upload(page, buffer, name, type); const data = await terminal(page, name); await preserved(page, data, buffer);
    check(data.result.text.includes('PHOTO OCR CHECK') && data.result.text.includes('10.1000/example') && data.result.text.includes('Local pixels stay on this device'), `${type} still-image OCR recovers known words and exact DOI (punctuation may be misread)`);
    check(data.result.links[0]?.kind === 'OCR DOI (unverified)', 'Recognized DOI remains unverified evidence');
  }
  const blank = await fixture(page, { blank: true }), blankName = `blank-${runId}.jpg`;
  await upload(page, blank, blankName); const blankData = await terminal(page, blankName); await preserved(page, blankData, blank);
  check(blankData.result.text === '' && blankData.result.metadata.ocr.outcome === 'empty' && blankData.result.metadata.ocr.attempts.length === 2, 'Blank image stays empty after exactly two OCR passes');
  check((await page.locator('#reader').innerText()).includes('OCR completed but recognized no text'), 'Empty recognition is explicit in the reader');
  const gif = Buffer.from('R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==', 'base64'), gifName = `unsupported-${runId}.gif`;
  await upload(page, gif, gifName, 'image/gif'); const gifData = await terminal(page, gifName, 'failed'); await preserved(page, gifData, gif);
  check(gifData.result.status === 'failed' && (await page.locator('#reader').innerText()).includes('JPEG, PNG and WebP still images'), 'Unsupported image produces an explicit OCR error with its original retained');
  const corrupt = Buffer.from([255, 216, 255, 224, 0, 16]), corruptName = `corrupt-${runId}.jpg`;
  await upload(page, corrupt, corruptName); const corruptData = await terminal(page, corruptName, 'failed'); await preserved(page, corruptData, corrupt);
  check(corruptData.result.status === 'failed' && corruptData.result.text === '', 'Corrupt JPEG reports failure without fabricated text');
  const progress = await page.evaluate(() => window.ocrWitness.progress);
  check(progress.some(event => event.status === 'recognizing text' && event.progress > 0 && event.progress < 1), 'Real worker recognition emits intermediate progress');
  await page.reload(); await page.waitForSelector('textarea#source-paste');
  await page.goto(base + '/?document=' + witnesses[0].documentId); await page.locator('.plain-reading').waitFor();
  check((await page.locator('.plain-reading').innerText()).includes('PHOTO OCR CHECK'), 'Recovered transcription survives reopening its saved result');
  await page.goto(base + '/?document=' + blankData.id); await page.locator('.plain-reading').waitFor();
  check((await page.locator('#reader').innerText()).includes('OCR completed but recognized no text'), 'Empty OCR notice survives reopening the saved record');
  await cleanFinish(session);

  const missing = await boot();
  await missing.context.route('**/ocr/7.0.0/worker.min.js', route => route.abort());
  const photo = await fixture(missing.page, { multi: true }), missingName = `missing-worker-${runId}.jpg`;
  await upload(missing.page, photo, missingName); const missingData = await terminal(missing.page, missingName, 'failed'); await preserved(missing.page, missingData, photo);
  check(missingData.result.status === 'failed' && missingData.result.warnings.some(w => w.includes('worker failed to load')), 'Missing same-origin worker asset surfaces a readable error and preserves original');
  await missing.context.unroute('**/ocr/7.0.0/worker.min.js');
  const nextName = `after-asset-error-${runId}.jpg`; await upload(missing.page, photo, nextName);
  check((await terminal(missing.page, nextName)).result.text.includes('PHOTO OCR CHECK'), 'Next upload succeeds after asset failure');
  await cleanFinish(missing);

  const cancelled = await boot(); let releaseCore, coreRequested;
  const requested = new Promise(resolve => { coreRequested = resolve; });
  const held = new Promise(resolve => { releaseCore = resolve; });
  await cancelled.context.route('**/ocr/7.0.0/core/**', async route => { coreRequested(); await held; await route.continue().catch(() => {}); });
  const cancelName = `cancel-initialize-${runId}.jpg`; await upload(cancelled.page, photo, cancelName);
  let watchdog;
  try { await Promise.race([requested, new Promise((_, reject) => { watchdog = setTimeout(() => reject(new Error('Core request was not observed')), 15000); })]); }
  finally { clearTimeout(watchdog); }
  await cancelled.page.getByRole('button', { name: 'Cancel ' + cancelName, exact: true }).click();
  const cancelledData = await terminal(cancelled.page, cancelName, 'cancelled'); await preserved(cancelled.page, cancelledData, photo);
  check(cancelledData.result.status === 'failed' && cancelledData.result.text === '' && await cancelled.page.evaluate(() => window.ocrWitness.active === 0), 'Cancel during stalled real core loading terminates worker and preserves original');
  releaseCore(); await cancelled.context.unroute('**/ocr/7.0.0/core/**');
  const afterCancel = `after-cancel-${runId}.jpg`; await upload(cancelled.page, photo, afterCancel);
  check((await terminal(cancelled.page, afterCancel)).result.text.includes('PHOTO OCR CHECK'), 'Next upload succeeds after initialization cancellation');
  await cancelled.page.evaluate(() => { window.ocrWitness.cancelOnRecognition = true; });
  const recognizeCancel = `cancel-recognize-${runId}.jpg`; await upload(cancelled.page, photo, recognizeCancel);
  const stopped = await terminal(cancelled.page, recognizeCancel, 'cancelled'); await preserved(cancelled.page, stopped, photo);
  check(stopped.result.text === '' && await cancelled.page.evaluate(() => window.ocrWitness.cancelledOnRecognition && window.ocrWitness.active === 0), 'Cancel button during actual recognition progress stops OCR without late success');
  const finalName = `after-recognition-cancel-${runId}.jpg`; await upload(cancelled.page, photo, finalName);
  check((await terminal(cancelled.page, finalName)).result.text.includes('PHOTO OCR CHECK'), 'Next upload succeeds after recognition cancellation');
  await cleanFinish(cancelled);
  check(unexpected.length === 0, 'App and OCR use only same-origin network requests');
  check(assetRequests.some(path => path.endsWith('eng.traineddata.gz')) && assetRequests.some(path => path.includes('tesseract-core')), 'Actual pinned English model and WASM core are fetched');
  const report = { environment: 'Chromium against actual local browser app, real Workers D1/R2 and CPU/WASM worker', browser: browser.version(), sourceIdentity, checks, witnesses, assets: [...new Set(assetRequests)] };
  if (process.env.OCR_REPORT_PATH) writeFileSync(process.env.OCR_REPORT_PATH, JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify(report, null, 2));
} finally { await browser.close(); }
