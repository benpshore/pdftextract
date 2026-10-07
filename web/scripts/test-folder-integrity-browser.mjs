import { createServer } from 'node:http';
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const { build } = require(require.resolve('esbuild', { paths: [require.resolve('vite')] }));
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const built = await build({ entryPoints: ['scripts/folder-regression-cases.mjs'], bundle: true, write: false, format: 'esm', platform: 'browser' });
const server = createServer((request, response) => {
  response.setHeader('Content-Type', request.url === '/cases.js' ? 'text/javascript' : 'text/html');
  response.end(request.url === '/cases.js' ? built.outputFiles[0].text : '<!doctype html><title>Synthetic folder regression</title>');
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
let browser;
try {
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  const origin = `http://127.0.0.1:${server.address().port}`;
  await page.route('**/*', route => route.request().url().startsWith(origin + '/') ? route.continue() : route.abort());
  await page.goto(origin);
  const results = await page.evaluate(async () => (await import('/cases.js')).runFolderRegressions());
  for (const result of results) console.log(result.ok ? 'PASS' : 'FAIL', result.name, result.error || '');
  if (results.some(result => !result.ok)) throw new Error('Browser folder regression failed');
} finally { await browser?.close(); await new Promise(resolve => server.close(resolve)); }
