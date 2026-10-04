import assert from 'node:assert/strict';
import { readFile, mkdtemp, writeFile, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const temporary = await mkdtemp(join(tmpdir(), 'tpe-egress-'));
const originalFetch = globalThis.fetch;
let child, sentinel;
try {
  const config = await readFile(join(root, 'vite.config.ts'), 'utf8');
  assert.match(config, /compatibility_flags:\s*\[[^\]]*"global_fetch_strictly_public"/);
  assert.doesNotMatch(config, /"global_fetch_private_origin"/);
  const built = JSON.parse(await readFile(join(root, 'dist/server/wrangler.json'), 'utf8'));
  assert(built.compatibility_flags.includes('global_fetch_strictly_public'), 'production output must retain the flag');
  assert(!built.compatibility_flags.includes('global_fetch_private_origin'));

  const require = createRequire(import.meta.url);
  const { build } = require(require.resolve('esbuild', { paths: [require.resolve('vite')] }));
  async function sourceModule(name, plugins = []) {
    const outfile = join(temporary, name + '.mjs');
    await build({ entryPoints: [join(root, 'lib/source-fetch.ts')], bundle: true, format: 'esm', platform: 'node', outfile, plugins });
    return import(pathToFileURL(outfile));
  }
  const production = await sourceModule('production');
  let disabledRequests = 0;
  globalThis.fetch = async () => { disabledRequests++; throw Error('production gate allowed network work'); };
  for (const operation of [() => production.allowed('https://example.com/'), () => production.fetchPublicSource('https://example.com/', 'text/html')]) {
    await assert.rejects(operation, error => error instanceof Response && error.status === 503);
  }
  assert.equal(disabledRequests, 0, 'production must refuse before DNS or HTTP');
  // Exercise retained transport independently, without an enabling production setting.
  // This virtual module exists only in this fixture bundle; it cannot qualify re-enablement.
  const { fetchPublicSource } = await sourceModule('transport-fixture', [{ name: 'explicit-test-only-capability', setup(build) {
    build.onResolve({ filter: /network-capabilities$/ }, () => ({ path: 'fixture-capability', namespace: 'fixture' }));
    build.onLoad({ filter: /.*/, namespace: 'fixture' }, () => ({ contents: 'export function requireRemoteExtraction() {}', loader: 'js' }));
  } }]);
  let requests = [], dns = {}, replies = [];
  globalThis.fetch = async (value, options) => {
    const url = new URL(value);
    if (url.hostname === 'cloudflare-dns.com') {
      const name = url.searchParams.get('name'), type = url.searchParams.get('type');
      const ip = dns[name] ?? '93.184.215.14';
      return Response.json({ Answer: type === 'A' ? [{ type: 1, data: ip }] : [] });
    }
    requests.push({ url: url.href, options });
    return replies.shift() ?? new Response('public article');
  };
  const reset = () => { requests = []; dns = {}; replies = []; };
  for (const value of ['http://127.0.0.1/', 'http://[::1]/', 'https://user:pass@example.com/', 'http://example.com:8080/']) {
    reset(); await assert.rejects(fetchPublicSource(value, 'text/html')); assert.equal(requests.length, 0);
  }
  reset();
  const publicResult = await fetchPublicSource('https://example.com/article', 'text/html');
  assert.equal(await publicResult.response.text(), 'public article');
  assert.equal(requests[0].options.redirect, 'manual');
  assert.equal(requests[0].options.headers.Accept, 'text/html');
  assert.equal(requests[0].options.headers.Cookie, undefined);
  reset(); replies = [new Response(null, { status: 302, headers: { location: '/next' } }), new Response('next')];
  assert.equal((await fetchPublicSource('https://example.com/start', 'text/html')).url, 'https://example.com/next');
  assert.equal(requests.length, 2);
  for (const location of ['http://127.0.0.1/', 'http://private.example.com/', 'http://[::1]/']) {
    reset(); dns['private.example.com'] = '10.0.0.7';
    replies = [new Response(null, { status: 302, headers: { location } })];
    await assert.rejects(fetchPublicSource('https://example.com/', 'text/html'));
    assert.equal(requests.length, 1, 'private redirect must not be contacted');
  }
  reset(); replies = [new Response(null, { status: 302, headers: { location: '/next' } })];
  let lookups = 0;
  const mockFetch = globalThis.fetch;
  globalThis.fetch = async (value, options) => {
    const url = new URL(value);
    if (url.hostname === 'cloudflare-dns.com' && ++lookups > 2) dns['example.com'] = '127.0.0.1';
    return mockFetch(value, options);
  };
  await assert.rejects(fetchPublicSource('https://example.com/', 'text/html'));
  assert.equal(requests.length, 1, 'same-host redirect must recheck changed DNS');
  globalThis.fetch = originalFetch;

  // A real workerd connection test, not a mocked private-origin rejection.
  // Emulate a preflight returning public, then connect a hostname resolving to
  // loopback. This tests the connection boundary; it is not a live DNS flip or
  // a test of Cloudflare's production-only own-zone routing flag.
  let hits = 0;
  sentinel = createServer((_req, res) => { hits++; res.end('private sentinel'); });
  await new Promise(resolve => sentinel.listen(0, '127.0.0.1', resolve));
  const target = `http://localhost:${sentinel.address().port}/`;
  assert.equal(await (await originalFetch(target)).text(), 'private sentinel');
  hits = 0;
  await writeFile(join(temporary, 'worker.mjs'), `export default { async fetch() {
    const preflight = { Answer: [{type:1, data:'93.184.215.14'}] };
    if (!preflight.Answer.length) throw new Error('no public preflight');
    try { const r = await fetch(${JSON.stringify(target)}); return new Response(await r.text(), {status:200}); }
    catch { return new Response('connection rejected', {status:403}); }
  } }`);
  await writeFile(join(temporary, 'test.capnp'), `using Workerd = import "/workerd/workerd.capnp";
const config :Workerd.Config = (
 services = [(name = "test", worker = (
  modules = [(name = "worker.mjs", esModule = embed "worker.mjs")],
  compatibilityDate = "2026-05-15",
  compatibilityFlags = ["global_fetch_strictly_public"]
 ))],
 sockets = [(name = "http", address = "127.0.0.1:0", http = (), service = "test")]
);
`);
  const wranglerRequire = createRequire(require.resolve('wrangler/package.json'));
  const miniflareRequire = createRequire(wranglerRequire.resolve('miniflare/package.json'));
  const executable = join(dirname(miniflareRequire.resolve('workerd/package.json')), 'bin/workerd');
  child = spawn(executable, ['serve', join(temporary, 'test.capnp'), '--control-fd=3'], { stdio: ['ignore', 'ignore', 'pipe', 'pipe'] });
  let diagnostics = '';
  child.stderr.on('data', chunk => { diagnostics += chunk; });
  const port = await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error(`workerd readiness timeout: ${diagnostics}`)), 10000);
    let buffer = '';
    child.stdio[3].on('data', chunk => {
      buffer += chunk;
      for (const line of buffer.split('\n').slice(0, -1)) {
        const message = JSON.parse(line);
        if (message.event === 'listen') { clearTimeout(timeout); resolve(message.port); }
      }
    });
    child.on('error', error => { clearTimeout(timeout); reject(error); });
    child.on('exit', code => { clearTimeout(timeout); reject(new Error(`workerd exited ${code}: ${diagnostics}`)); });
  });
  const result = await originalFetch(`http://127.0.0.1:${port}/`);
  assert.equal(result.status, 403, await result.text());
  assert.equal(hits, 0, 'private sentinel must never receive the Worker connection');
  console.log('Disabled production gate, retained transport fixtures, production flag, and isolated workerd connection boundary passed. No production re-enablement or formal audit qualification.');
} finally {
  globalThis.fetch = originalFetch;
  if (child) { child.kill(); await new Promise(resolve => child.once('close', resolve)); }
  if (sentinel) await new Promise(resolve => sentinel.close(resolve));
  await rm(temporary, { recursive: true, force: true });
}
