import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, rm, writeFile, access } from 'node:fs/promises';
import { createServer, request as httpRequest } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { configuration, createLocalRuntime } from './scholarly-local-runtime.mjs';

const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const pdf = Buffer.from('%PDF-1.7\nSynthetic bridge contract only\n');
const rawTei = '<TEI><persName coords=""/><ref target="#missing">[9]</ref></TEI>';
function canonical(bytes = pdf) {
  return Buffer.from(JSON.stringify({ source_sha256: sha(bytes), tei_sha256: sha(rawTei), raw_tei: rawTei, citations: [], warnings: ['geometry unavailable', 'semantic projection is partial'], unknown_native_field: 'preserve me' }, null, 2) + '\n');
}
const headers = { 'Content-Type': 'application/pdf', 'X-TPE-Local-Bridge': '1' };
async function temporary(t) {
  const directory = await mkdtemp(join(tmpdir(), 'tpe-local-runtime-test-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  return directory;
}
async function capturedRuntime(t) {
  const directory = await temporary(t), captured = join(directory, 'native.json');
  await writeFile(captured, canonical());
  const runtime = createLocalRuntime(configuration({ SCHOLARLY_LOCAL_PORT: '0', SCHOLARLY_CAPTURED_JSON: captured }));
  const url = await runtime.listen();
  t.after(() => runtime.close());
  return { directory, captured, runtime, url };
}
async function nativeRuntime(t, mode = 'ok', additions = {}) {
  const directory = await temporary(t), binary = join(directory, 'native'), proof = join(directory, 'proof.json');
  const source = `#!${process.execPath}
import {readFileSync,writeFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
const args=process.argv.slice(2),file=args[1],bytes=readFileSync(file);
writeFileSync(process.env.PROOF,JSON.stringify({pid:process.pid,file,args,env:Object.keys(process.env)}));
if(process.env.MODE==='hang'){process.on('SIGTERM',()=>{});setInterval(()=>{},1000);}
else if(process.env.MODE==='overflow'){process.stdout.write('x'.repeat(4096));}
else if(process.env.MODE==='invalid-utf8'){process.stdout.write(Buffer.from([0xff]));}
else if(process.env.MODE==='failed'){process.stderr.write('synthetic failure');process.exitCode=2;}
else {const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const tei=${JSON.stringify(rawTei)};
const result=args[0]==='bibliography'?{sha256:hash(bytes),references:[],warnings:['native unresolved'],status:'not_found',resolution:{resolved:0}}:{source_sha256:hash(bytes),tei_sha256:hash(tei),raw_tei:tei,citations:[],warnings:['geometry unavailable','semantic projection is partial'],unknown_native_field:'preserve me'};
if(process.env.MODE==='wrong-hash')result.source_sha256='0'.repeat(64);
process.stdout.write(JSON.stringify(result,null,2)+'\\n');
if(args[0]==='bibliography')process.exitCode=1;}
`;
  await writeFile(binary, source, { mode: 0o700 });
  const service = createServer((request, response) => { assert.equal(request.url, '/api/isalive'); response.end('true'); });
  await new Promise(resolve => service.listen(0, '127.0.0.1', resolve));
  t.after(() => new Promise(resolve => service.close(resolve)));
  const config = configuration({ SCHOLARLY_LOCAL_PORT: '0', TPE_NATIVE_BIN: binary, TPE_GROBID_URL: `http://127.0.0.1:${service.address().port}`, ...additions });
  config.childEnv = { ...config.childEnv, PROOF: proof, MODE: mode };
  const runtime = createLocalRuntime(config), url = await runtime.listen();
  t.after(() => runtime.close());
  return { directory, runtime, url, proof };
}
async function waitForProof(path) {
  for (let count = 0; count < 100; count++) {
    try { return JSON.parse(await readFile(path, 'utf8')); } catch {}
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  throw new Error('Synthetic child did not start');
}

test('configuration restricts endpoint, command path and explicit mode', () => {
  for (const url of ['https://127.0.0.1:8070', 'http://localhost:8070', 'http://example.com', 'http://127.0.0.1@evil.test', 'http://127.0.0.1:8070/path', 'http://127.0.0.1?url=evil']) assert.throws(() => configuration({ TPE_GROBID_URL: url }));
  assert.throws(() => configuration({ TPE_NATIVE_BIN: './tpe' }));
  assert.throws(() => configuration({ TPE_NATIVE_BIN: '/tmp/tpe', SCHOLARLY_CAPTURED_JSON: '/tmp/result.json' }));
  assert.throws(() => configuration({ SCHOLARLY_MAX_INPUT_BYTES: 'Infinity' }));
});
test('missing runtime is blocked and never silently replays fixtures', async t => {
  const runtime = createLocalRuntime(configuration({ SCHOLARLY_LOCAL_PORT: '0' })), url = await runtime.listen(); t.after(() => runtime.close());
  const state = await (await fetch(url + '/health')).json();
  assert.equal(state.status, 'blocked'); assert.equal(state.mode, 'live-native'); assert.equal(state.resolver, false);
  assert.equal(state.max_input_bytes, 32 * 1024 * 1024); assert.equal(state.max_output_bytes, 32 * 1024 * 1024);
  const response = await fetch(url + '/grobid', { method: 'POST', headers, body: pdf });
  assert.equal(response.status, 503); assert.equal(response.headers.get('X-TPE-Scholarly-Mode'), 'live-native');
});
test('captured mode is explicit, exact-byte preserving and source bound', async t => {
  const { url } = await capturedRuntime(t);
  const state = await (await fetch(url + '/health')).json(); assert.equal(state.mode, 'captured-native'); assert.equal(state.resolver, false);
  const response = await fetch(url + '/grobid', { method: 'POST', headers, body: pdf });
  assert.equal(response.status, 200); assert.equal(response.headers.get('X-TPE-Scholarly-Mode'), 'captured-native');
  assert.deepEqual(Buffer.from(await response.arrayBuffer()), canonical());
  assert.equal((await fetch(url + '/grobid', { method: 'POST', headers, body: Buffer.concat([pdf, Buffer.from('changed')]) })).status, 409);
  assert.equal((await fetch(url + '/bibliography', { method: 'POST', headers, body: pdf })).status, 503);
});
test('browser origins, browser fetch metadata, Host rebinding and absent bridge marker are rejected', async t => {
  const { url } = await capturedRuntime(t);
  for (const extra of [{ Origin: 'https://evil.test' }, { 'Sec-Fetch-Site': 'same-origin' }, { Host: 'evil.test' }]) {
    const status = await new Promise((resolve, reject) => {
      const request = httpRequest(url + '/grobid', { method: 'POST', headers: { ...headers, ...extra } }, response => { response.resume(); resolve(response.statusCode); });
      request.on('error', reject); request.end(pdf);
    });
    assert.equal(status, 403, JSON.stringify(extra));
  }
  assert.equal((await fetch(url + '/grobid', { method: 'POST', headers: { 'Content-Type': 'application/pdf' }, body: pdf })).status, 403);
  assert.equal((await fetch(url + '/grobid', { method: 'OPTIONS' })).status, 404);
});
test('rejects invalid input type, non-PDF bytes and over-limit bodies', async t => {
  const { url } = await nativeRuntime(t, 'ok', { SCHOLARLY_MAX_INPUT_BYTES: '64' });
  assert.equal((await fetch(url + '/grobid', { method: 'POST', headers: { ...headers, 'Content-Type': 'application/json' }, body: '{}' })).status, 415);
  assert.equal((await fetch(url + '/grobid', { method: 'POST', headers, body: 'not PDF' })).status, 415);
  assert.equal((await fetch(url + '/grobid', { method: 'POST', headers, body: Buffer.alloc(65) })).status, 413);
});
test('invokes native CLI with fixed safe arguments, retains all JSON and removes temporary PDF', async t => {
  const { url, proof } = await nativeRuntime(t);
  const response = await fetch(url + '/grobid', { method: 'POST', headers, body: pdf });
  assert.equal(response.status, 200); assert.deepEqual(Buffer.from(await response.arrayBuffer()), canonical());
  const invocation = await waitForProof(proof);
  assert.equal(invocation.args[0], 'grobid'); assert.equal(invocation.args[invocation.args.indexOf('--consolidation') + 1], '0');
  assert.ok(!invocation.env.includes('TPE_GROBID_BEARER_TOKEN')); assert.ok(!invocation.env.includes('HTTP_PROXY'));
  await new Promise(resolve => setTimeout(resolve, 20)); await assert.rejects(access(invocation.file));
});
test('optional resolver calls the real bibliography command separately and retains partial evidence', async t => {
  const { url, proof } = await nativeRuntime(t, 'ok', { TPE_SCHOLARLY_RESOLVE: '1' });
  assert.equal((await (await fetch(url + '/health')).json()).resolver, true);
  const response = await fetch(url + '/bibliography', { method: 'POST', headers, body: pdf });
  assert.equal(response.status, 200); assert.equal((await response.json()).status, 'not_found');
  const invocation = await waitForProof(proof); assert.equal(invocation.args[0], 'bibliography'); assert.ok(invocation.args.includes('--resolve'));
});
for (const mode of ['overflow', 'invalid-utf8', 'wrong-hash', 'failed']) {
  test(`rejects ${mode} native output without publishing result`, async t => {
    const { url } = await nativeRuntime(t, mode, { SCHOLARLY_MAX_OUTPUT_BYTES: '1024' });
    assert.equal((await fetch(url + '/grobid', { method: 'POST', headers, body: pdf })).status, 502);
  });
}
test('deadline kills a SIGTERM-ignoring child and exposes no partial result', async t => {
  const { url, proof } = await nativeRuntime(t, 'hang', { SCHOLARLY_TIMEOUT_MS: '100' });
  const response = await fetch(url + '/grobid', { method: 'POST', headers, body: pdf });
  assert.equal(response.status, 504);
  const invocation = await waitForProof(proof); assert.throws(() => process.kill(invocation.pid, 0), { code: 'ESRCH' });
});
test('request socket cancellation kills child, clears temporary file, and releases busy slot', async t => {
  const { url, proof } = await nativeRuntime(t, 'hang');
  const request = httpRequest(url + '/grobid', { method: 'POST', headers }); request.on('error', () => {}); request.end(pdf);
  const invocation = await waitForProof(proof);
  assert.equal((await fetch(url + '/grobid', { method: 'POST', headers, body: pdf })).status, 503);
  request.destroy();
  await new Promise(resolve => setTimeout(resolve, 400));
  assert.throws(() => process.kill(invocation.pid, 0), { code: 'ESRCH' }); await assert.rejects(access(invocation.file));
});
test('runtime shutdown awaits forced child termination and temporary-file cleanup', async t => {
  const { url, proof, runtime } = await nativeRuntime(t, 'hang');
  const request = httpRequest(url + '/grobid', { method: 'POST', headers }); request.on('error', () => {}); request.end(pdf);
  const invocation = await waitForProof(proof);
  await runtime.close();
  assert.throws(() => process.kill(invocation.pid, 0), { code: 'ESRCH' }); await assert.rejects(access(invocation.file));
});
