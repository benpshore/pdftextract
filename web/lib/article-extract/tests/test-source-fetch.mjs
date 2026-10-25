// SSRF policy and bounded fetching for the capture route. No network: resolver and fetch are stubbed.
// Run from web/: node lib/article-extract/tests/test-source-fetch.mjs
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const { build } = require(require.resolve('esbuild', { paths: [require.resolve('vite')] }));
const web = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
const bundle = await build({ entryPoints: [resolve(web, 'lib/source-fetch.ts')], bundle: true, write: false, format: 'esm', platform: 'node' });
const { isPublicIp, parseIpv6, publicUrl, allowed, fetchPublicSource, readBounded, looksBinary, withTimeout } = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`);

const cases = [
  ['Private, loopback, link-local, reserved and embedded-IPv4 addresses are never public', () => {
    for (const ip of ['10.0.0.1', '127.0.0.1', '127.255.255.255', '0.0.0.0', '169.254.169.254', '172.16.0.1', '172.31.255.254', '192.168.1.1', '100.64.0.1', '100.127.255.255', '192.0.0.1', '192.0.2.5', '192.88.99.1', '198.18.0.1', '198.19.255.255', '198.51.100.7', '203.0.113.9', '224.0.0.1', '239.255.255.255', '240.0.0.1', '255.255.255.255',
      '::', '::1', 'fc00::1', 'fd12:3456::1', 'fe80::1', 'febf::1', 'fec0::1', 'ff02::1', '2001:db8::1', '::ffff:127.0.0.1', '::ffff:10.1.2.3', '::ffff:7f00:1', '::10.0.0.1', '64:ff9b::10.0.0.1', '64:ff9b::a00:1', '2002:0a00:0001::', '2002:c0a8:101::1', '2001::1', '2001:10::1', '100::1', 'not-an-ip', '256.1.1.1', '1.2.3', '', 'localhost']) {
      assert.equal(isPublicIp(ip), false, `${ip} must not be public`);
    }
  }],
  ['Globally routable addresses are public', () => {
    for (const ip of ['1.1.1.1', '8.8.8.8', '93.184.216.34', '172.15.0.1', '172.32.0.1', '100.63.0.1', '100.128.0.1', '192.0.3.1', '198.17.0.1', '198.20.0.1', '223.255.255.255', '2606:4700:4700::1111', '2a00:1450:4001:80b::200e', '::ffff:8.8.8.8', '64:ff9b::808:808', '2002:0808:0808::1', '[2606:4700::1]']) {
      assert.equal(isPublicIp(ip), true, `${ip} must be public`);
    }
    assert.deepEqual(parseIpv6('::1'), [0, 0, 0, 0, 0, 0, 0, 1]);
    assert.equal(parseIpv6('1::2::3'), null);
    assert.equal(parseIpv6('12345::'), null);
  }],
  ['URL policy rejects non-public shapes before any network use', () => {
    for (const url of ['ftp://example.test/x', 'file:///etc/passwd', 'http://user:pw@example.test/', 'http://example.test:8080/', 'http://localhost/', 'http://localhost.localdomain/', 'http://intranet/', 'http://127.0.0.1/', 'http://0x7f.1/', 'http://2130706433/', 'http://[::1]/', 'http://[fe80::1]/', 'http://metadata.internal/', 'http://printer.local/', 'http://site.chatgpt.site/', 'http://app.workers.dev/', 'http://foo.example/', 'http://x.onion/', 'http://1.2.3.4.in-addr.arpa/', 'http://a.test/', 'http://bad_host.example.com/', 'http://example.com./', 'not a url', 'javascript:alert(1)', 'data:text/html,hi']) {
      assert.throws(() => publicUrl(url), /public HTTP or HTTPS page URL/, `${url} must be rejected`);
    }
    assert.equal(publicUrl('https://Example.COM:443/path?q=1#frag').href, 'https://example.com/path?q=1');
    assert.equal(publicUrl('http://news.example.co.uk:80/a').href, 'http://news.example.co.uk/a');
    assert.equal(publicUrl('https://xn--bcher-kva.example.net/').hostname, 'xn--bcher-kva.example.net');
  }],
  ['Resolved addresses must all be public', async () => {
    await assert.rejects(allowed('https://example.net/', async () => ['10.0.0.5']), /not a public web server/);
    await assert.rejects(allowed('https://example.net/', async () => ['93.184.216.34', '::1']), /not a public web server/);
    await assert.rejects(allowed('https://example.net/', async () => ['93.184.216.34', '::ffff:169.254.169.254']), /not a public web server/);
    await assert.rejects(allowed('https://example.net/', async () => []), /not a public web server/);
    await assert.rejects(allowed('https://example.net/', async () => { throw new Error('Could not verify the destination.'); }), /Could not verify/);
    assert.equal((await allowed('https://example.net/', async () => ['93.184.216.34', '2606:2800:220:1:248:1893:25c8:1946'])).href, 'https://example.net/');
  }],
  ['Redirects are limited and every hop is re-checked', async () => {
    const dns = { 'example.net': ['93.184.216.34'], 'cdn.example.org': ['93.184.216.35'], 'internal.example.net': ['10.0.0.9'] };
    const resolve = async host => { if (!(host in dns)) throw new Error('Could not verify the destination.'); return dns[host]; };
    const seen = [];
    const fetchStub = routes => async (input, init) => {
      const url = String(input);
      seen.push({ url, redirect: init.redirect, credentials: init.credentials, accept: init.headers.Accept });
      const route = routes[url];
      if (!route) return new Response('missing', { status: 404 });
      return typeof route === 'function' ? route() : new Response(route.body ?? '', { status: route.status ?? 200, headers: route.headers ?? {} });
    };
    const result = await fetchPublicSource('https://example.net/start', 'text/html', undefined, { resolve, fetch: fetchStub({
      'https://example.net/start': { status: 301, headers: { location: '/second' } },
      'https://example.net/second': { status: 302, headers: { location: 'https://cdn.example.org/final' } },
      'https://cdn.example.org/final': { body: '<p>hello</p>', headers: { 'content-type': 'text/html; charset=utf-8' } },
    }) });
    assert.equal(result.url, 'https://cdn.example.org/final');
    assert.equal(result.redirects, 2);
    assert.equal(await result.response.text(), '<p>hello</p>');
    assert.ok(seen.every(call => call.redirect === 'manual' && call.credentials === 'omit' && call.accept === 'text/html'));
    await assert.rejects(fetchPublicSource('https://example.net/start', 'text/html', undefined, { resolve, fetch: fetchStub({ 'https://example.net/start': { status: 302, headers: { location: 'https://internal.example.net/admin' } } }) }), /not a public web server/);
    await assert.rejects(fetchPublicSource('https://example.net/start', 'text/html', undefined, { resolve, fetch: fetchStub({ 'https://example.net/start': { status: 302, headers: { location: 'http://127.0.0.1/' } } }) }), /public HTTP or HTTPS page URL/);
    await assert.rejects(fetchPublicSource('https://example.net/start', 'text/html', undefined, { resolve, fetch: fetchStub({ 'https://example.net/start': { status: 302, headers: { location: 'ftp://example.net/x' } } }) }), /public HTTP or HTTPS page URL/);
    await assert.rejects(fetchPublicSource('https://example.net/start', 'text/html', undefined, { resolve, fetch: fetchStub({ 'https://example.net/start': { status: 302 } }) }), /empty redirect/);
    const loop = {}; for (let i = 0; i < 8; i++) loop[`https://example.net/hop${i}`] = { status: 302, headers: { location: `/hop${i + 1}` } };
    await assert.rejects(fetchPublicSource('https://example.net/hop0', 'text/html', undefined, { resolve, fetch: fetchStub(loop) }), /too many times/);
    await assert.rejects(fetchPublicSource('https://example.net/missing', 'text/html', undefined, { resolve, fetch: fetchStub({}) }), /HTTP 404/);
    await assert.rejects(fetchPublicSource('https://nowhere.example.com/', 'text/html', undefined, { resolve, fetch: fetchStub({}) }), /Could not verify/);
  }],
  ['Timeouts abort the fetch and bodies are bounded', async () => {
    const resolve = async () => ['93.184.216.34'];
    const hanging = (input, init) => new Promise((_, reject) => init.signal.addEventListener('abort', () => reject(init.signal.reason)));
    await assert.rejects(fetchPublicSource('https://example.net/slow', 'text/html', withTimeout(undefined, 30), { resolve, fetch: hanging }), /did not respond within/);
    const parent = new AbortController(); parent.abort(new Error('cancelled by user'));
    await assert.rejects(fetchPublicSource('https://example.net/slow', 'text/html', withTimeout(parent.signal, 5000), { resolve, fetch: hanging }), /cancelled by user/);
    const stream = new ReadableStream({ start(controller) { for (let i = 0; i < 10; i++) controller.enqueue(new Uint8Array(1000).fill(65 + i)); controller.close(); } });
    const bounded = await readBounded(stream, 4500);
    assert.equal(bounded.bytes.length, 4500);
    assert.equal(bounded.truncated, true);
    assert.equal(bounded.bytes[4499], 65 + 4);
    const small = await readBounded(new Response('abc').body, 100);
    assert.equal(new TextDecoder().decode(small.bytes), 'abc');
    assert.equal(small.truncated, false);
    assert.equal((await readBounded(null, 10)).bytes.length, 0);
    assert.equal(looksBinary(new TextEncoder().encode('%PDF-1.7 ...')), true);
    assert.equal(looksBinary(new Uint8Array([0x50, 0x4b, 0x03, 0x04, 0, 0])), true);
    assert.equal(looksBinary(new TextEncoder().encode('<html><body>text\u0000</body>')), true);
    assert.equal(looksBinary(new TextEncoder().encode('<!doctype html><html lang="en"><body>ok</body></html>')), false);
    assert.equal(looksBinary(new Uint8Array([0xff, 0xfe, 0x3c, 0x00, 0x68, 0x00])), false, 'UTF-16 with BOM is text');
  }],
];

let failed = 0;
for (const [name, check] of cases) {
  try { await check(); console.log(`PASS ${name}`); }
  catch (error) { failed++; console.error(`FAIL ${name}: ${error.stack || error.message}`); }
}
console.log(`${cases.length - failed}/${cases.length} source-fetch cases passed.`);
if (failed) process.exitCode = 1;
