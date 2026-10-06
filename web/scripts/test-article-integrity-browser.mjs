// Real Chromium + local application/D1/R2. Only the remote capture response is synthetic.
// Run `pnpm dev` with local storage bindings, then this script; PLAYWRIGHT_MODULE is optional.
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const base = process.env.TPE_BASE_URL || 'http://127.0.0.1:5173';
if (!['127.0.0.1', 'localhost', '[::1]'].includes(new URL(base).hostname)) throw new Error('Use a disposable loopback app only');
const probe = await fetch(base + '/api/documents', { headers: { cookie: '__sites_local_auth=1' } });
if ((await probe.text()).includes('no such table')) {
  const dir = resolve('.wrangler/state/v3/d1/miniflare-D1DatabaseObject');
  const file = readdirSync(dir).find(name => name.endsWith('.sqlite') && name !== 'metadata.sqlite');
  const db = new DatabaseSync(join(dir, file));
  for (const sql of readdirSync('drizzle').filter(name => name.endsWith('.sql')).sort()) db.exec(readFileSync(join('drizzle', sql), 'utf8').replaceAll('--> statement-breakpoint', ''));
  db.close();
}
const browser = await chromium.launch({ headless: true });
const context = await browser.newContext();
const page = await context.newPage();
page.setDefaultTimeout(30000);
const errors = [], captures = [];
page.on('pageerror', error => errors.push(String(error)));
const run = Date.now();
const partialUrl = `https://synthetic.test/partial-${run}`, completeUrl = `https://synthetic.test/complete-${run}`;
const content = 'Synthetic source prefix: é, 日本語. The missing tail was not captured.';
const bytes = Buffer.from(content);
await page.route('**/*', async route => {
  const request = route.request(), url = new URL(request.url());
  if (url.origin !== new URL(base).origin) return route.abort();
  if (url.pathname !== '/api/capture') return route.continue();
  const source = request.postDataJSON().url;
  assert.ok([partialUrl, completeUrl].includes(source)); captures.push(source);
  await route.fulfill({ status: 200, headers: {
    'Content-Type': 'text/plain;charset=utf-8', 'X-TPE-Source-URL': source,
    'X-TPE-Source-Bytes': String(bytes.length), ...(source === partialUrl ? { 'X-TPE-Truncated': 'true' } : {}),
  }, body: bytes });
});
const recordFor = url => page.evaluate(async source => {
  const { documents } = await (await fetch('/api/documents')).json();
  const record = documents.find(value => value.source_url === source);
  return record ? (await fetch('/api/documents/' + record.id)).json() : null;
}, url);
const openComposer = async url => {
  await page.fill('textarea#source-paste', url);
  await page.getByRole('button', { name: 'Import pasted source', exact: true }).click();
  await page.waitForFunction(source => [...document.querySelectorAll('.queue-item')].some(item => item.textContent.includes(source) && item.querySelector('.phase-saved')), url, { timeout: 60000 });
};
try {
  await page.goto(base + '/signin-with-chatgpt?return_to=%2F');
  await page.waitForSelector('textarea#source-paste');
  for (let attempt = 0; attempt < 40; attempt++) {
    await page.fill('textarea#source-paste', `hydration-${attempt}`);
    if (await page.getByRole('button', { name: 'Import pasted source', exact: true }).isEnabled()) break;
    await page.waitForTimeout(250);
  }
  await openComposer(partialUrl);
  let saved = await recordFor(partialUrl);
  assert.equal(saved.result.status, 'partial'); assert.equal(saved.result.metadata.truncated, true);
  assert.deepEqual(saved.result.metadata.sourceCapture, { truncated: true, capturedBytes: bytes.length });
  assert.equal(saved.result.text, content); assert.equal(saved.record.status, 'partial');
  assert.ok(await page.locator('.capture-warning').isVisible());
  const stored = await page.evaluate(async id => {
    const response = await fetch('/api/documents/' + id + '/original');
    return { truncated: response.headers.get('X-TPE-Truncated'), count: response.headers.get('X-TPE-Source-Bytes'), bytes: [...new Uint8Array(await response.arrayBuffer())] };
  }, saved.record.id);
  assert.equal(stored.truncated, 'true'); assert.equal(Number(stored.count), bytes.length);
  assert.deepEqual(Buffer.from(stored.bytes), bytes);
  console.log('PASS capture metadata, partial status, visible warning and exact original bytes saved in real local D1/R2');
  await page.reload();
  await page.waitForSelector('.capture-warning');
  const details = page.getByText('Details and review notes', { exact: true });
  if (!await page.getByRole('button', { name: 'Re-read original', exact: true }).isVisible()) await details.click();
  const committed = page.waitForResponse(response => /\/api\/uploads\/[^/?]+$/.test(new URL(response.url()).pathname) && response.request().method() === 'POST');
  await page.getByRole('button', { name: 'Re-read original', exact: true }).click();
  assert.equal((await committed).status(), 200);
  saved = await recordFor(partialUrl);
  assert.equal(saved.result.status, 'partial'); assert.equal(saved.result.metadata.truncated, true);
  assert.deepEqual(saved.result.metadata.sourceCapture, { truncated: true, capturedBytes: bytes.length });
  assert.equal(captures.filter(url => url === partialUrl).length, 1, 're-read uses saved source, not a new capture');
  console.log('PASS reload and re-read preserve source truncation without refetching');
  await openComposer(completeUrl);
  const complete = await recordFor(completeUrl);
  assert.equal(complete.result.status, 'ready'); assert.equal(complete.result.metadata.sourceCapture.truncated, false);
  await page.locator('.queue-item').filter({ hasText: completeUrl }).locator('.queue-open').click();
  await page.waitForSelector('.capture-warning', { state: 'detached' });
  assert.equal(await page.locator('.capture-warning').count(), 0);
  console.log('PASS complete-source positive control remains ready without an incomplete-source warning');
  const html = '<!-- <!-- --><article><h1>Comment regression</h1><p>Readable synthetic content after the closed comment.</p></article>';
  await page.setInputFiles('input[aria-label="Choose source files"]', { name: `comment-${run}.html`, mimeType: 'text/html', buffer: Buffer.from(html) });
  await page.waitForSelector(`.queue-item:has-text("comment-${run}.html") .phase-saved`, { timeout: 60000 });
  await page.locator('.queue-item').filter({ hasText: `comment-${run}.html` }).locator('.queue-open').click();
  assert.match(await page.locator('article.reading').innerText(), /Readable synthetic content/);
  assert.deepEqual(errors, []);
  console.log('PASS formerly looping HTML completes through actual browser workspace with no page errors');
} finally { await context.close(); await browser.close(); }
