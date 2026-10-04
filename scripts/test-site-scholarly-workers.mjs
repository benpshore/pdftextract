#!/usr/bin/env node
/** Actual scholarly routes/adapter with disposable workerd D1/R2.
 * Auth identity and the native service are synthetic test boundaries. Native JSON
 * is the exact captured PR206 artifact; this is not a live GROBID execution.
 */
import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const requireWeb = createRequire(new URL('../web/package.json', import.meta.url));
const { build } = requireWeb(requireWeb.resolve('esbuild', { paths: [requireWeb.resolve('vite')] }));
const requireWrangler = createRequire(requireWeb.resolve('wrangler/package.json'));
const { Miniflare, Log, LogLevel } = requireWrangler('miniflare');
const web = fileURLToPath(new URL('../web/', import.meta.url));
const pdf = await readFile(new URL('../web/tests/fixtures/scholarly/synthetic-scholarly.pdf', import.meta.url));
const nativeJson = await readFile(new URL('../web/tests/fixtures/scholarly/grobid-pr206-fixed-live.json', import.meta.url), 'utf8');
const nativeDocument = JSON.parse(nativeJson);
const sha = value => createHash('sha256').update(value instanceof ArrayBuffer ? new Uint8Array(value) : value).digest('hex');
assert.equal(sha(pdf), nativeDocument.source_sha256);
assert.equal(sha(nativeDocument.raw_tei), nativeDocument.tei_sha256);

// All production functions are bundled unchanged. Only platform bindings and
// the existing identity provider are injected; owner()/CSRF and SQL stay real.
const entry = `
import {env as rawEnv} from 'fixture:raw-workers';
import {GET as statusGET,POST as attachPOST} from './app/api/documents/[id]/scholarly/route.ts';
import {GET as evidenceGET} from './app/api/documents/[id]/scholarly/evidence/route.ts';
import {deleteDocument} from './lib/document-lifecycle.ts';
let holdPoint=null,holdReady=null,releaseHold=null,counts={},nativeJson='',nativeVariant=null,resolverFailure=false;
function count(key){counts[key]=(counts[key]||0)+1;}
async function barrier(point){
  if(holdPoint!==point)return;
  holdPoint=null;const held=new Promise(resolve=>{releaseHold=resolve;});holdReady?.();await held;
}
function statement(value,sql){return new Proxy(value,{get(target,key){
  if(key==='bind')return(...args)=>statement(target.bind(...args),sql);
  if(key==='run')return async(...args)=>{
    const publishes=/^UPDATE documents SET result_key=/.test(sql);
    if(publishes)await barrier('before-cas');
    const result=await target.run(...args);
    if(publishes)await barrier('after-cas');
    return result;
  };
  const value=Reflect.get(target,key,target);return typeof value==='function'?value.bind(target):value;
}});}
function bindings(){return {
  DB:new Proxy(rawEnv.DB,{get(target,key){
    if(key==='prepare')return sql=>statement(target.prepare(sql),sql);
    const value=Reflect.get(target,key,target);return typeof value==='function'?value.bind(target):value;
  }}),
  BUCKET:new Proxy(rawEnv.BUCKET,{get(target,key){
    if(key==='put')return async(...args)=>{
      count('bucket-put');const name=String(args[0]),kind=name.includes('/scholarly/')?'evidence':name.includes('/results/')?'result':null;
      if(kind)await barrier('before-'+kind+'-put');
      const result=await target.put(...args);
      if(kind)await barrier('after-'+kind+'-put');return result;
    };
    if(key==='get')return async(...args)=>{count('bucket-get');return target.get(...args);};
    const value=Reflect.get(target,key,target);return typeof value==='function'?value.bind(target):value;
  }}),
  SCHOLARLY:{async fetch(request){
    const path=new URL(request.url).pathname;
    if(path==='/health'){count('health');return Response.json({status:'ready',mode:'captured-native',resolver:resolverFailure,max_input_bytes:33554432,max_output_bytes:33554432});}
    if(path==='/bibliography'&&resolverFailure){count('resolver');return new Response('Synthetic resolver request failure',{status:502});}
    count('native');if(path!=='/grobid')throw Error('Unexpected native invocation');
    const input=await request.arrayBuffer();if(input.byteLength===0)throw Error('Missing original bytes');
    await barrier('runtime');request.signal.throwIfAborted();
    return new Response(nativeVariant??nativeJson,{headers:{'Content-Type':'application/json','X-TPE-Scholarly-Mode':'captured-native'}});
  }}
};}
async function invoke(action,controller){
  globalThis.__scholarlyFixtureUser=action.user===null?null:{userId:action.user||'owner',email:'synthetic@fixture.invalid',displayName:'Synthetic owner',fullName:null};
  const headers={'Content-Type':'application/json'};if(action.origin!==undefined)headers.Origin=action.origin;
  const request=new Request('https://fixture.test/api/documents/'+action.id+'/scholarly'+(action.operation==='evidence'?'/evidence?format='+encodeURIComponent(action.format||'native-json'):''),{
    method:action.operation==='attach'?'POST':'GET',headers,...(action.operation==='attach'?{body:JSON.stringify(action.body)}:{}),...(controller?{signal:controller.signal}:{})
  });
  const context={params:Promise.resolve({id:action.id})};
  const response=await(action.operation==='attach'?attachPOST(request,context):action.operation==='evidence'?evidenceGET(request,context):statusGET(request,context));
  const body=await response.text();let value;try{value=JSON.parse(body);}catch{}
  return {status:response.status,body,value,headers:Object.fromEntries(response.headers)};
}
export default {async fetch(request){
  const input=await request.json();counts={};nativeJson=input.nativeJson;nativeVariant=input.nativeVariant??null;resolverFailure=input.resolverFailure===true;
  globalThis.__scholarlyFixtureBindings=bindings();
  if(input.race){
    const controller=new AbortController();const ready=new Promise(resolve=>{holdReady=resolve;});holdPoint=input.race.point;
    if(input.race.intervention==='already-aborted')controller.abort();
    const pending=invoke(input.action,controller);
    if(input.race.intervention==='already-aborted')return Response.json({pending:await pending,counts});
    await ready;
    let other;
    try{
      if(input.race.intervention==='cancel')controller.abort();
      else if(input.race.intervention==='delete'){await deleteDocument(input.action.id,input.action.user||'owner');other={status:204};}
      else if(input.race.intervention==='newer')other=await invoke(input.action);
      else throw Error('Unknown race intervention');
    }finally{releaseHold?.();}
    return Response.json({pending:await pending,other,counts});
  }
  return Response.json({response:await invoke(input.action),counts});
}};
`;
const compiled = await build({
  stdin: { contents: entry, resolveDir: web, sourcefile: 'scholarly-workers-fixture.mjs' },
  bundle: true, write: false, format: 'esm', platform: 'neutral', target: 'es2022', tsconfig: web + 'tsconfig.json',
  plugins: [{ name: 'isolated-platform-and-auth', setup(builder) {
    builder.onResolve({ filter: /^fixture:raw-workers$/ }, () => ({ path: 'cloudflare:workers', external: true }));
    builder.onResolve({ filter: /^cloudflare:workers$/ }, () => ({ path: 'bindings', namespace: 'fixture' }));
    builder.onResolve({ filter: /(?:^@\/app\/|^\.\/|^\.\.\/)chatgpt-auth$/ }, () => ({ path: 'auth', namespace: 'fixture' }));
    builder.onLoad({ filter: /.*/, namespace: 'fixture' }, args => ({
      contents: args.path === 'auth'
        ? 'export async function getChatGPTUser(){return globalThis.__scholarlyFixtureUser;}'
        : 'export const env=new Proxy({},{get:(_,key)=>globalThis.__scholarlyFixtureBindings[key]});',
      loader: 'js',
    }));
  } }],
});
const mf = new Miniflare({ modules: true, script: compiled.outputFiles[0].text, compatibilityDate: '2026-05-15', r2Buckets: ['BUCKET'], d1Databases: ['DB'], log: new Log(LogLevel.ERROR) });
const checks = [];
try {
  const db = await mf.getD1Database('DB'), bucket = await mf.getR2Bucket('BUCKET');
  await db.exec('CREATE TABLE documents (id TEXT PRIMARY KEY,owner TEXT NOT NULL,title TEXT NOT NULL,kind TEXT NOT NULL,source_url TEXT,original_name TEXT NOT NULL,mime TEXT NOT NULL,status TEXT NOT NULL,engine TEXT NOT NULL,sha256 TEXT NOT NULL,bytes INTEGER NOT NULL,created_at TEXT NOT NULL,search_text TEXT NOT NULL,result_key TEXT)');
  await db.exec('CREATE TABLE document_deletions (id TEXT PRIMARY KEY,owner TEXT NOT NULL)');
  const originalResult = { title: 'Keep existing title', text: 'Keep exact readable body', markdown: 'Keep exact readable body', links: [{ url: 'https://example.invalid/source' }], warnings: ['Original warning'], engine: 'Existing PDF worker', status: 'partial', metadata: { original: 'retained' } };
  async function seed(owner = 'owner') {
    const id = randomUUID(), key = `${id}/results/${randomUUID()}`;
    await db.prepare('INSERT INTO documents (id,owner,title,kind,source_url,original_name,mime,status,engine,sha256,bytes,created_at,search_text,result_key) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)')
      .bind(id, owner, originalResult.title, 'pdf', 'https://example.invalid/original', 'unchanged original.pdf', 'application/pdf', 'partial', originalResult.engine, sha(pdf), pdf.length, '2026-10-04T00:00:00Z', originalResult.text, key).run();
    await bucket.put(`${id}/original`, pdf); await bucket.put(key, JSON.stringify(originalResult));
    return { id, key, operation: 'attach', body: { baseResultKey: key } };
  }
  async function run(action, options = {}) {
    const response = await mf.dispatchFetch('https://isolated-fixture.test/', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ action, nativeJson, ...options }) });
    assert.equal(response.status, 200, await response.clone().text()); return response.json();
  }
  const current = id => db.prepare('SELECT * FROM documents WHERE id=?').bind(id).first();
  const keys = async id => (await bucket.list({ prefix: `${id}/` })).objects.map(item => item.key).sort();
  const assertPreserved = async action => {
    const row = await current(action.id); assert.equal(row.result_key, action.key); assert.equal(row.original_name, 'unchanged original.pdf');
    assert.equal(sha(await (await bucket.get(`${action.id}/original`)).arrayBuffer()), sha(pdf));
    assert.deepEqual(await (await bucket.get(action.key)).json(), originalResult);
    assert.deepEqual(await keys(action.id), [`${action.id}/original`, action.key].sort());
  };
  const pass = message => { checks.push(message); console.log('PASS ' + message); };

  const access = await seed();
  for (const [user, expected] of [[null, 401], ['foreign-owner', 404]]) for (const operation of ['attach', 'status', 'evidence']) {
    const result = await run({ ...access, user, operation }); assert.equal(result.response.status, expected);
    assert.equal(result.counts.native || 0, 0); assert.equal(result.counts.health || 0, 0); assert.equal(result.counts['bucket-get'] || 0, 0);
  }
  const csrf = await run({ ...access, origin: 'https://foreign.invalid' }); assert.equal(csrf.response.status, 403); assert.equal(csrf.counts.health || 0, 0);
  const extra = await run({ ...access, body: { ...access.body, owner: 'other', bridge: 'http://127.0.0.1:1' } }); assert.equal(extra.response.status, 400); assert.equal(extra.counts.health || 0, 0);
  await assertPreserved(access); pass('Actual owner and CSRF checks reject unauthenticated/foreign/extra-authority requests before native or object reads');

  const sourceMismatch = await seed(); await bucket.put(`${sourceMismatch.id}/original`, new Uint8Array([37, 80, 68, 70, 45, 0]));
  const sourceBad = await run(sourceMismatch); assert.equal(sourceBad.response.status, 409); assert.equal(sourceBad.counts.native || 0, 0); assert.equal((await current(sourceMismatch.id)).result_key, sourceMismatch.key);
  const outputMismatch = await seed(); const bad = { ...nativeDocument, source_sha256: '0'.repeat(64) };
  assert.equal((await run(outputMismatch, { nativeVariant: JSON.stringify(bad) })).response.status, 400); await assertPreserved(outputMismatch);
  const teiBad = { ...nativeDocument, tei_sha256: '0'.repeat(64) };
  assert.equal((await run(outputMismatch, { nativeVariant: JSON.stringify(teiBad) })).response.status, 400); await assertPreserved(outputMismatch);
  pass('Source bytes and captured-native source/TEI hash failures leave the prior readable result intact');

  const failedResolver=await seed();const failedResolution=(await run(failedResolver,{resolverFailure:true}));
  assert.equal(failedResolution.response.status,200,failedResolution.response.body);assert.equal(failedResolution.counts.resolver||0,0);
  const retained=failedResolution.response.value.result;
  assert.equal(retained.text,originalResult.text);assert.equal(retained.metadata.scholarly.has_resolution,false);
  assert(retained.bibliography.references.items.every(reference=>reference.resolution.status==='not-requested'&&reference.resolution.providers.length===0));
  for(let i=0;i<nativeDocument.citations.length;i++)assert.equal(retained.bibliography.references.items[i].doi,nativeDocument.citations[i].identifiers.DOI?.[0]??nativeDocument.citations[i].identifiers.doi?.[0]);
  const failedEvidence=await (await bucket.get(failedResolver.id+'/scholarly/'+retained.metadata.scholarly.evidence_id+'.json')).json();assert.equal(failedEvidence.grobid.raw_json,nativeJson);assert.equal(failedEvidence.native_resolution,undefined);
  assert.equal((await run({...failedResolver,operation:'evidence',format:'resolution'})).response.status,404);
  pass('A bridge advertising resolution cannot cause registry lookup during the release hold; supplied bibliography, unverified identifiers, prior reading and canonical GROBID evidence remain intact');

  const success = await seed(); const attached = (await run(success)).response;
  assert.equal(attached.status, 200, attached.body);
  const saved = attached.value; assert.notEqual(saved.record.result_key, success.key); assert.equal(saved.result.text, originalResult.text); assert.equal(saved.result.title, originalResult.title); assert.equal(saved.result.engine, originalResult.engine);
  assert.deepEqual(saved.result.links, originalResult.links); assert.equal(saved.result.metadata.original, 'retained'); assert.equal(saved.result.metadata.scholarly.mode, 'captured-native');
  assert.equal(saved.result.bibliography.source.result_key, saved.record.result_key); assert.equal(saved.result.bibliography.references.items.length, 2); assert.equal(saved.result.bibliography.mentions.state, 'unavailable');
  assert(nativeDocument.warnings.every(warning => saved.result.warnings.includes(warning)));
  const row = await current(success.id); assert.equal(row.title, originalResult.title); assert.equal(row.original_name, 'unchanged original.pdf'); assert.equal(row.sha256, sha(pdf));
  const evidenceKey = `${success.id}/scholarly/${saved.result.metadata.scholarly.evidence_id}.json`;
  const envelope = await (await bucket.get(evidenceKey)).json(); assert.equal(envelope.grobid.raw_json, nativeJson); assert.equal(envelope.grobid.raw_json_sha256, sha(nativeJson)); assert.equal(envelope.grobid.raw_tei, nativeDocument.raw_tei);
  for (const [format, expected] of [['native-json', nativeJson], ['tei', nativeDocument.raw_tei]]) {
    const response = (await run({ ...success, operation: 'evidence', format })).response; assert.equal(response.status, 200, response.body); assert.equal(response.body, expected);
    assert.equal(response.headers['content-type'], 'application/octet-stream'); assert.match(response.headers['content-disposition'], /^attachment;/); assert.equal(response.headers['x-content-type-options'], 'nosniff'); assert.equal(response.headers['cache-control'], 'private, no-store'); assert.match(response.headers['content-security-policy'], /sandbox/);
  }
  assert.equal((await run({ ...success, operation: 'evidence', format: 'resolution' })).response.status, 404);
  await bucket.put(evidenceKey, JSON.stringify({ ...envelope, grobid: { ...envelope.grobid, raw_tei: envelope.grobid.raw_tei + 'tampered' } }));
  assert.equal((await run({ ...success, operation: 'evidence', format: 'tei' })).response.status, 409); await bucket.put(evidenceKey, JSON.stringify(envelope));
  pass('Actual captured output attaches two references without invented mentions; original reading/warnings and byte-exact passive evidence downloads survive with integrity checks');

  const stale = await run(success); assert.equal(stale.response.status, 409); assert.equal(stale.counts.health || 0, 0);
  const race = await seed(); const raced = await run(race, { race: { point: 'before-cas', intervention: 'newer' } });
  assert.equal(raced.other.status, 200, raced.other.body); assert.equal(raced.pending.status, 409); assert.equal((await current(race.id)).result_key, raced.other.value.record.result_key);
  assert.equal((await keys(race.id)).filter(key => key.includes('/scholarly/')).length, 1); assert.equal((await keys(race.id)).filter(key => key.includes('/results/')).length, 2);
  pass('Stale preconditions and an interleaved real D1 CAS cannot overwrite the newer winner or retain the losing evidence');

  const already = await seed(); const cancelledFirst = await run(already, { race: { intervention: 'already-aborted' } }); assert.notEqual(cancelledFirst.pending.status, 200); await assertPreserved(already);
  for (const point of ['runtime', 'before-evidence-put', 'after-evidence-put', 'before-result-put', 'after-result-put']) {
    const action = await seed(), outcome = await run(action, { race: { point, intervention: 'cancel' } }); assert.notEqual(outcome.pending.status, 200, point); await assertPreserved(action);
  }
  const after = await seed(); const committed = await run(after, { race: { point: 'after-cas', intervention: 'cancel' } });
  assert.equal(committed.pending.status, 200, committed.pending.body); assert.equal((await current(after.id)).result_key, committed.pending.value.record.result_key);
  pass('Cancellation before publication cleans this attempt; cancellation after the real CAS preserves a discoverable committed result');

  for (const point of ['runtime', 'before-evidence-put', 'after-evidence-put', 'before-result-put', 'after-result-put', 'before-cas', 'after-cas']) {
    const action = await seed(); const outcome = await run(action, { race: { point, intervention: 'delete' } });
    assert.equal(outcome.other.status, 204); assert.notEqual(outcome.pending.status, 200, point); assert.equal(await current(action.id), null); assert.deepEqual(await keys(action.id), [], point);
    assert(await db.prepare('SELECT id FROM document_deletions WHERE id=?').bind(action.id).first());
    for (const operation of ['status', 'evidence']) assert.equal((await run({ ...action, operation })).response.status, 404);
  }
  pass('Deletion at bridge, artifact-write and pre/post-CAS barriers never resurrects metadata or leaves late attempt objects readable');
  console.log(JSON.stringify({ checks, fixture: { source_sha256: sha(pdf), native_json_sha256: sha(nativeJson), tei_sha256: sha(nativeDocument.raw_tei) }, scope: 'Actual API/service/adapter, real disposable workerd D1/R2. Identity and service responses are isolated fixtures; captured-native replay is not live GROBID, deployed auth or production evidence.' }, null, 2));
} finally { await mf.dispose(); }
