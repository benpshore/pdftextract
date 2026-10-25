// Real Chromium, loopback-only synthetic API and documents; no Zotero writes.
// PLAYWRIGHT_MODULE may point to an existing Playwright installation.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const { build } = require(require.resolve('esbuild', { paths: [require.resolve('vite')] }));
const web = fileURLToPath(new URL('..', import.meta.url));
const bundled = await build({
  stdin: { contents: `import {createRoot} from 'react-dom/client'; import {createElement} from 'react';
    import {ZoteroPanel, ZoteroApi} from './lib/zotero/index';
    window.ZoteroApi = ZoteroApi;
    window.mount = () => createRoot(document.getElementById('root')).render(createElement(ZoteroPanel, {
      article: {title: 'Synthetic article', text: '', engine: 'fixture', metadata: {}, links: []},
      original: {url: '/original', name: 'fixture.pdf'},
      apiOptions: {base: location.origin, sleep: async () => {}}
    }));`, resolveDir: resolve(web), loader: 'ts' },
  bundle: true, write: false, format: 'iife', platform: 'browser', jsx: 'automatic',
  define: { 'process.env.NODE_ENV': '"development"' }, logLevel: 'silent',
});
const KEY = 'synthetic-browser-key';
let receiverHits = 0;
let originals = 0;
let parentMode = 'success';
let originalMode = 'delayed';
let releaseOriginal;
let writes = [];
const sockets = new Set();
function listen(handler) {
  const server = createServer(handler);
  server.on('connection', socket => { sockets.add(socket); socket.on('close', () => sockets.delete(socket)); });
  return new Promise(resolve => server.listen(0, '127.0.0.1', () => resolve(server)));
}
const origin = server => `http://127.0.0.1:${server.address().port}`;
const receiver = await listen((req, res) => {
  receiverHits++;
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Headers', 'Zotero-API-Key,Zotero-API-Version,Zotero-Write-Token,Content-Type');
  res.end('{}');
});
const server = await listen(async (req, res) => {
  if (req.url === '/favicon.ico') { res.writeHead(204); res.end(); return; }
  if (req.url === '/') { res.setHeader('Content-Type', 'text/html'); res.end('<main id="root"></main><script src="/bundle.js"></script>'); return; }
  if (req.url === '/bundle.js') { res.setHeader('Content-Type', 'text/javascript'); res.end(bundled.outputFiles[0].text); return; }
  if (req.url.startsWith('/redirect/')) { res.writeHead(302, { Location: origin(receiver) + '/capture' }); res.end(); return; }
  if (req.url === '/original') {
    originals++;
    if (originalMode === 'failed') { res.writeHead(500); res.end(); return; }
    releaseOriginal = () => { res.setHeader('Content-Type', 'application/pdf'); res.end('%PDF synthetic'); };
    return;
  }
  assert.equal(req.headers['zotero-api-key'], KEY);
  res.setHeader('Content-Type', 'application/json');
  if (req.url === '/keys/current') { res.end(JSON.stringify({ userID: 1, username: 'fixture', access: {user: {library: true, write: true}} })); return; }
  if (req.method === 'GET') { res.end('[]'); return; }
  let body = '';
  for await (const chunk of req) body += chunk;
  if (req.url === '/users/1/items') {
    const items = JSON.parse(body);
    writes.push({items, token: req.headers['zotero-write-token']});
    if (parentMode === 'lost') { res.destroy(); return; }
    const successful = {};
    items.forEach((item, i) => { successful[i] = { key: item.itemType === 'attachment' ? 'FILE2345' : 'PARN2345' }; });
    res.end(JSON.stringify({successful})); return;
  }
  res.end(JSON.stringify({exists: 1}));
});
let browser;
try {
  browser = await chromium.launch({headless: true});
  const page = await browser.newPage();
  await page.goto(origin(server));
  const result = await page.evaluate(async key => {
    const api = new window.ZoteroApi(key, {base: location.origin + '/redirect', maxAttempts: 1});
    try { await api.keyInfo(); return 'unexpected success'; } catch (error) { return error.kind; }
  }, KEY);
  assert.equal(result, 'network');
  assert.equal(receiverHits, 0, 'redirect target receives neither preflight nor key');
  console.log('PASS Chromium cross-origin 302 is rejected before target contact');

  async function mount() {
    await page.goto(origin(server));
    await page.evaluate(() => { sessionStorage.clear(); window.mount(); });
    await page.getByLabel('Zotero API key', {exact: true}).fill(KEY);
    await page.getByRole('button', {name: 'Connect', exact: true}).click();
    await page.getByLabel('Title', {exact: true}).waitFor();
  }
  const sendButton = () => page.getByRole('button', {name: /^(Send to Zotero|Sending…)$/});
  const parentWrites = () => writes.filter(write => write.items[0].itemType !== 'attachment');
  async function forceSubmitTwice() {
    // Same JS turn: exercises the synchronous guard before React can disable.
    await page.evaluate(() => {
      const form = document.querySelector('select').form;
      form.dispatchEvent(new Event('submit', {bubbles: true, cancelable: true}));
      form.dispatchEvent(new Event('submit', {bubbles: true, cancelable: true}));
    });
  }
  await mount();
  await page.getByLabel('The original file', {exact: false}).check();
  await sendButton().click();
  await page.waitForFunction(() => document.querySelector('button[type=submit]').disabled);
  assert.equal(await sendButton().isDisabled(), true, 'busy includes original acquisition');
  await forceSubmitTwice();
  for (let i = 0; originals === 0 && i < 200; i++) await new Promise(resolve => setTimeout(resolve, 5));
  assert.equal(originals, 1);
  assert.equal(parentWrites().length, 0);
  releaseOriginal();
  await page.getByText('Item PARN2345 saved', {exact: false}).waitFor();
  await forceSubmitTwice();
  assert.equal(parentWrites().length, 1);
  assert.equal(await sendButton().isDisabled(), true);
  // Reset/forget and reconnect must not erase a saved outcome or write guard.
  await page.getByRole('button', {name: 'Forget key'}).click();
  await page.getByLabel('Zotero API key', {exact: true}).fill(KEY);
  await page.getByRole('button', {name: 'Connect', exact: true}).click();
  await page.getByText('Item PARN2345 saved', {exact: false}).waitFor();
  await forceSubmitTwice();
  assert.equal(parentWrites().length, 1);
  console.log('PASS delayed-original clicks and same-turn submits create one parent; outcome survives reset');

  writes = []; originals = 0; originalMode = 'failed';
  await mount();
  await page.getByLabel('The original file', {exact: false}).check();
  await sendButton().click();
  await page.getByText(/Original acquisition failed/).waitFor();
  assert.equal(writes.length, 0);
  assert.equal(await sendButton().isEnabled(), true);
  await page.getByLabel('The original file', {exact: false}).uncheck();
  await sendButton().click();
  await page.getByText('Item PARN2345 saved', {exact: false}).waitFor();
  assert.equal(parentWrites().length, 1);
  console.log('PASS original-read failure permits retry before any parent write');

  writes = []; parentMode = 'lost';
  await mount();
  await sendButton().click();
  await page.getByText(/The parent write may have completed/).waitFor();
  // Chromium may also retry a stale connection internally after res.destroy().
  const attempts = parentWrites().length;
  assert.ok(attempts >= 3, 'transport retries occurred');
  assert.equal(new Set(parentWrites().map(write => write.token)).size, 1, 'transport retries share one idempotency token');
  await forceSubmitTwice();
  assert.equal(parentWrites().length, attempts, 'uncertain result cannot create a fresh parent attempt');
  assert.equal(await sendButton().isDisabled(), true);
  console.log('PASS lost parent response retains uncertainty and one token, blocks fresh submission');
} finally {
  await browser?.close();
  for (const socket of sockets) socket.destroy();
  await Promise.all([server, receiver].map(server => new Promise(resolve => server.close(resolve))));
}
