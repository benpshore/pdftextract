import assert from 'node:assert/strict';
import { Worker } from 'node:worker_threads';
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const { build } = require(require.resolve('esbuild', { paths: [require.resolve('vite')] }));
const bundle = async entry => {
  const result = await build({ entryPoints: [entry], bundle: true, write: false, format: 'esm', platform: 'node' });
  return `data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}`;
};
const failures = [];
async function test(name, run) {
  try { await run(); console.log('PASS', name); }
  catch (error) { failures.push(name); console.error('FAIL', name, String(error)); }
}
await test('comment repair terminates, preserves closed comments and recovers missing tails', async () => {
  const moduleUrl = await bundle('lib/article-extract/dom.ts');
  const worker = new Worker(`const { parentPort, workerData } = require('node:worker_threads'); import(workerData).then(({repairHtml}) => parentPort.postMessage(['<!-- <!-- -->', '<!-- closed --><p>body</p>', '<!-- missing<p>body</p>', '<!-- x -->'.repeat(100000)].map(source => ({source, ...repairHtml(source)}))));`, { eval: true, workerData: moduleUrl });
  try {
    const results = await new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error('repairHtml exceeded its 3-second watchdog')), 3000);
      worker.on('message', result => { clearTimeout(timeout); resolve(result); });
      worker.on('error', error => { clearTimeout(timeout); reject(error); });
    });
    for (const i of [0, 1, 3]) { assert.equal(results[i].html, results[i].source); assert.deepEqual(results[i].repairs, []); }
    assert.match(results[2].html, /<!-- --> missing<p>body<\/p>/);
    assert.equal(results[2].repairs.length, 1);
  } finally { await worker.terminate(); }
});
await test('capture flags and exact multibyte bytes survive upload and result metadata', async () => {
  const U = await import(await bundle('lib/upload-client.ts'));
  const originalFetch = globalThis.fetch;
  // 16 MiB of UTF-8 is below the separate 12 Mi-character article limit.
  const bytes = new TextEncoder().encode('é'.repeat(8 * 1024 * 1024));
  const calls = [];
  try {
    globalThis.fetch = async (url, options = {}) => {
      if (url === '/api/capture') return new Response(bytes, { headers: { 'Content-Type': 'text/plain;charset=utf-8', 'X-TPE-Truncated': 'true', 'X-TPE-Source-Bytes': String(bytes.length) } });
      if (url === '/api/uploads') { calls.push(JSON.parse(options.body)); return Response.json({ session: 'fixture', chunkSize: 8 * 1024 * 1024 }); }
      if (options.method === 'PUT') return Response.json({ partNumber: Number(new URL(url, 'https://fixture.test').searchParams.get('part')), etag: 'synthetic' });
      return Response.json({ id: 'fixture' });
    };
    const captured = await U.captureSource('https://fixture.test/source');
    assert.equal(captured.decodedSource.length, 8 * 1024 * 1024);
    assert.deepEqual(new Uint8Array(await captured.file.arrayBuffer()), bytes);
    assert.deepEqual(captured.capture, { truncated: true, capturedBytes: bytes.length });
    await U.uploadOriginal(captured.file, { sourceUrl: captured.url, capture: captured.capture });
    assert.deepEqual(calls[0].capture, captured.capture);
    const ready = { title: 'Fixture', text: 'é', status: 'ready', metadata: { truncated: false }, warnings: [], links: [], engine: 'fixture' };
    const result = U.applyCaptureEvidence(ready, captured.capture);
    assert.equal(result.status, 'partial'); assert.equal(result.metadata.truncated, true);
    assert.deepEqual(result.metadata.sourceCapture, captured.capture);
    assert.match(result.warnings.join(' '), /incomplete/i);
    assert.equal(U.applyCaptureEvidence({ ...ready, status: 'failed' }, captured.capture).status, 'failed');
    assert.equal(U.applyCaptureEvidence(ready, { truncated: false, capturedBytes: 2 }).status, 'ready');
    assert.equal(U.applyCaptureEvidence(ready), ready, 'ordinary local files unchanged');
    assert.equal(ready.status, 'ready', 'input result not mutated');
  } finally { globalThis.fetch = originalFetch; }
});
if (failures.length) throw new Error(`${failures.length} article integrity regressions failed`);
