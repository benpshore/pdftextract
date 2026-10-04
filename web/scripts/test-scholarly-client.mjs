/** Actual streamed GET reconciliation helper used by Workspace. */
import assert from 'node:assert/strict';
import {createRequire,Module} from 'node:module';
import {fileURLToPath} from 'node:url';
const require=createRequire(import.meta.url),web=fileURLToPath(new URL('../',import.meta.url));
const {build}=require(require.resolve('esbuild',{paths:[require.resolve('vite')]}));
const compiled=new Module(web+'synthetic-scholarly-client.cjs');compiled.filename=web+'synthetic-scholarly-client.cjs';
compiled._compile((await build({entryPoints:[web+'lib/scholarly-client.ts'],bundle:true,write:false,platform:'node',format:'cjs'})).outputFiles[0].text,compiled.filename);
const {fetchCurrentDocument,currentDocumentResult}=compiled.exports,versions=new Map(),results=new Map(),actualFetch=globalThis.fetch,checks=[];
const old={record:{id:'first',result_key:'old'},result:{text:'old reading'}},latest={record:{id:'first',result_key:'new'},result:{text:'new reference result'}};
try{
  let stream;globalThis.fetch=async()=>new Response(new ReadableStream({start(controller){stream=controller;}}),{headers:{'Content-Type':'application/json'}});
  const pending=fetchCurrentDocument('first',versions,results);await new Promise(resolve=>setImmediate(resolve));
  results.set('first',latest);versions.set('first',1);stream.enqueue(new TextEncoder().encode(JSON.stringify(old)));stream.close();
  assert.deepEqual(await (await pending).json(),latest);checks.push('scholarly save after GET headers but before body completion supersedes the stale response');
  const started=versions.get('first');globalThis.fetch=async()=>Response.json(old);const buffered=await fetchCurrentDocument('first',versions,results);const parsing=buffered.json();const newest={...latest,record:{...latest.record,result_key:'newest'}};results.set('first',newest);versions.set('first',2);assert.deepEqual(currentDocumentResult('first',await parsing,started,versions,results),newest);checks.push('synchronous state-write guard catches a save after buffered response return during JSON parsing');
  globalThis.fetch=async()=>Response.json(old);assert.deepEqual(await (await fetchCurrentDocument('first',versions,results)).json(),old);
  checks.push('an unchanged version preserves the newly requested server result instead of pinning an old local cache');
  globalThis.fetch=async()=>new Response('Missing',{status:404});const missing=await fetchCurrentDocument('second',versions,results);assert.equal(missing.status,404);assert.equal(await missing.text(),'Missing');
  assert.equal(results.has('second'),false);checks.push('document identities and HTTP failure bodies remain isolated');
  globalThis.fetch=async()=>{throw Error('Offline');};await assert.rejects(fetchCurrentDocument('first',versions,results),/Offline/);
  checks.push('transport failure stays visible to Workspace recovery');
  console.log(JSON.stringify({checks,passed:checks.length,scope:'Actual Workspace GET helper; synthetic streamed responses.'},null,2));
}finally{globalThis.fetch=actualFetch;}
