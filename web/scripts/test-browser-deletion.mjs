/** Production Workspace/IndexedDB/API with disposable local D1/R2. Synthetic
 * owner, sources and PDF worker; never contacts a deployed app or real profile. */
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
const web=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const req=createRequire(join(web,'package.json')),reqWrangler=createRequire(req.resolve('wrangler/package.json'));
const {build}=reqWrangler('esbuild'),{Miniflare,Log,LogLevel}=reqWrangler('miniflare');
const {chromium}=await import(process.env.PLAYWRIGHT_MODULE||'playwright');
const checks=[];const check=name=>{checks.push(name);console.log('PASS',name);};
const client=await build({
 stdin:{contents:`import React from 'react';import {createRoot} from 'react-dom/client';import Workspace from './app/workspace';import * as storage from './lib/workspace-storage';window.fixture={storage};createRoot(document.getElementById('root')).render(<Workspace userId="fixture-owner"/>);`,resolveDir:web,loader:'tsx'},
 absWorkingDir:web,bundle:true,format:'esm',platform:'browser',write:false,tsconfig:join(web,'tsconfig.json'),define:{'process.env.NODE_ENV':'"production"'},
});
const routes=[['documents','./app/api/documents/route'],['document','./app/api/documents/[id]/route'],['uploads','./app/api/uploads/route'],['upload','./app/api/uploads/[id]/route'],['original','./app/api/documents/[id]/original/route'],['media','./app/api/documents/[id]/media/route'],['asset','./app/api/documents/[id]/assets/[asset]/route']];
const worker=await build({
 stdin:{contents:routes.map(([name,path])=>`import * as ${name} from '${path}';`).join('\n')+`
export default {async fetch(request){
 const path=new URL(request.url).pathname.split('/').filter(Boolean);
 const route=path[1]==='uploads'?(path[2]?upload:uploads):!path[2]?documents:path[3]==='original'?original:path[3]==='media'?media:path[3]==='assets'?asset:document;
 const handler=route[request.method];return handler?handler(request,{params:Promise.resolve({id:path[2],asset:path[4]})}):new Response('No fixture route',{status:404});
}};`,resolveDir:web,loader:'ts'},
 absWorkingDir:web,bundle:true,format:'esm',platform:'browser',write:false,external:['cloudflare:workers'],tsconfig:join(web,'tsconfig.json'),
 plugins:[{name:'synthetic-owner',setup(b){b.onResolve({filter:/chatgpt-auth$/},()=>({path:'fixture-auth',namespace:'fixture'}));b.onLoad({filter:/.*/,namespace:'fixture'},()=>({contents:'export async function getChatGPTUser(){return {userId:"fixture-owner"};}'}));}}],
});
const mf=new Miniflare({modules:true,script:worker.outputFiles[0].text,compatibilityDate:'2026-05-15',r2Buckets:['BUCKET'],d1Databases:['DB'],log:new Log(LogLevel.ERROR)});
const db=await mf.getD1Database('DB'),bucket=await mf.getR2Bucket('BUCKET');
for(const file of ['0000_clean_apocalypse.sql','0001_tense_lord_hawal.sql','0002_document_deletions.sql']){
 const sql=(await readFile(join(web,'drizzle',file),'utf8')).replaceAll('--> statement-breakpoint','');
 for(const statement of sql.split(';').filter(s=>s.trim()))await db.prepare(statement).run();
}
let base,deleteMode='normal',deleteRequests=0,releaseDelete,deleteHeld=false;
const server=createServer(async(request,response)=>{
 try{
  const url=new URL(request.url,base);
  if(url.pathname==='/'){response.setHeader('Content-Type','text/html');response.end('<!doctype html><html><body><div id="root"></div><script type="module" src="/client.js"></script></body></html>');return;}
  if(url.pathname==='/client.js'){response.setHeader('Content-Type','text/javascript');response.end(client.outputFiles[0].text);return;}
  if(url.pathname==='/pdf-worker.js'){response.setHeader('Content-Type','text/javascript');response.end('self.onmessage=()=>{self.postMessage({progress:1,total:2});setTimeout(()=>self.postMessage({title:"late PDF",text:"late PDF text",links:[],warnings:[],engine:"synthetic worker",status:"ready"}),30000);};');return;}
  const chunks=[];for await(const chunk of request)chunks.push(chunk);const body=Buffer.concat(chunks);
  if(request.method==='DELETE'&&url.pathname.startsWith('/api/documents/')){
   deleteRequests++;
   if(deleteMode==='offline'){response.writeHead(503);response.end('Synthetic server unreachable; retry deletion.');return;}
   if(deleteMode==='hold-before'){deleteHeld=true;await new Promise(resolve=>{releaseDelete=resolve;});deleteHeld=false;}
  }
  const reply=await mf.dispatchFetch(url.href,{method:request.method,headers:request.headers,...(body.length?{body}:{})});
  if(request.method==='DELETE'&&deleteMode==='lose-response'){request.socket.destroy();return;}
  response.writeHead(reply.status,Object.fromEntries(reply.headers));response.end(Buffer.from(await reply.arrayBuffer()));
 }catch(error){response.writeHead(500);response.end(String(error));}
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));base=`http://127.0.0.1:${server.address().port}`;
const browser=await chromium.launch({headless:true,...(process.env.CHROMIUM_EXECUTABLE_PATH?{executablePath:process.env.CHROMIUM_EXECUTABLE_PATH}:{})});
const context=await browser.newContext();const errors=[];
context.on('page',page=>page.on('pageerror',error=>errors.push(String(error))));const page=await context.newPage();
async function waitFor(predicate){for(let i=0;i<200;i++){if(await predicate())return;await new Promise(r=>setTimeout(r,25));}throw Error('Timed out waiting for fixture condition');}
async function boot(page){await page.goto(base);await page.locator('#source-paste').waitFor();await page.waitForFunction(()=>window.fixture?.storage);}
async function upload(page,name,body,kind='text/plain'){
 await page.locator('input[aria-label="Choose source files"]').setInputFiles({name,mimeType:kind,buffer:Buffer.from(body)});
 await page.locator('.queue-item').filter({hasText:name}).locator('.phase-saved').waitFor();
 const row=await db.prepare('SELECT * FROM documents WHERE original_name=?').bind(name).first();assert.ok(row);return row;
}
async function confirmDelete(page,button,accept=true){page.once('dialog',async dialog=>{assert.match(dialog.message(),/Permanently delete/);assert.match(dialog.message(),/cannot be undone/);await(accept?dialog.accept():dialog.dismiss());});await button.click();}
async function rowGone(id){await waitFor(async()=>!await db.prepare('SELECT id FROM documents WHERE id=?').bind(id).first());}
async function assertBytesGone(id){assert.equal((await bucket.list({prefix:`${id}/`})).objects.length,0);}
async function journal(page){return page.evaluate(()=>window.fixture.storage.readWorkspaceDeletions('fixture-owner'));}
async function complete(page,id){await waitFor(async()=>(await journal(page)).some(e=>e.id===id&&!e.pending));}
const openSaved=page=>page.getByRole('button',{name:'Saved articles',exact:true}).click();
const closeSaved=page=>page.getByRole('button',{name:'Close saved articles',exact:true}).click();
const retry=page=>page.getByRole('region',{name:'Pending document deletions'}).getByRole('button',{name:'Retry delete'});
try{
 await boot(page);
 const a=await upload(page,'delete-A.txt','synthetic document A'),b=await upload(page,'keep-B.txt','synthetic neighbor B');
 await openSaved(page);await confirmDelete(page,page.getByRole('button',{name:'Delete delete-A.txt',exact:true}),false);
 assert.equal(deleteRequests,0);assert.ok(await bucket.head(`${a.id}/original`));check('irreversible confirmation cancellation changes nothing');
 const stale=await page.evaluate(()=>window.fixture.storage.readWorkspace('fixture-owner'));
 await bucket.put(`${b.id}/assets/independent`,'independent asset');
 await confirmDelete(page,page.getByRole('button',{name:'Delete delete-A.txt',exact:true}));await complete(page,a.id);await rowGone(a.id);await assertBytesGone(a.id);
 assert.ok(await bucket.head(`${b.id}/original`));assert.ok(await bucket.head(`${b.id}/assets/independent`));
 assert.equal(await page.locator('.document-list').getByText('delete-A.txt',{exact:true}).count(),0);assert.equal(await page.locator('.queue-item').filter({hasText:'delete-A.txt'}).count(),0);check('confirmed UI deletion removes only intended D1/R2 record, selection and queue');
 assert.equal((await context.request.delete(`${base}/api/documents/${a.id}`,{data:{confirmDocumentId:a.id}})).status(),204);check('repeat DELETE is idempotent');
 await page.evaluate(async old=>{await window.fixture.storage.writeWorkspace('different-owner',old);await window.fixture.storage.writeWorkspace('fixture-owner',old);},stale);
 assert.ok(!(await page.evaluate(()=>window.fixture.storage.readWorkspace('fixture-owner'))).items.some(item=>item.record?.id===a.id));
 assert.ok((await page.evaluate(()=>window.fixture.storage.readWorkspace('different-owner'))).items.some(item=>item.record?.id===a.id));
 await page.reload();await page.locator('.queue-item').waitFor();assert.equal(await page.locator('.queue-item').filter({hasText:'delete-A.txt'}).count(),0);check('real IndexedDB rejects stale snapshots across reload and preserves other owners');
 await page.goto(`${base}/?document=${a.id}`);await page.locator('#source-paste').waitFor();await page.waitForFunction(()=>!new URL(location.href).searchParams.has('document'));assert.equal(await page.locator('.reading').count(),0);check('deleted document history URL cannot reopen cached content');
 const c=await upload(page,'two-tabs-C.txt','two tab source');const page2=await context.newPage();await boot(page2);
 await openSaved(page2);await page2.locator('.document-item').filter({hasText:'two-tabs-C.txt'}).click();await page2.locator('.reading').filter({hasText:'two tab source'}).waitFor();
 await openSaved(page);await confirmDelete(page,page.getByRole('button',{name:'Delete two-tabs-C.txt',exact:true}));await complete(page,c.id);
 await page2.waitForFunction(()=>!document.querySelector('.reading')?.textContent.includes('two tab source'));assert.equal(await page2.locator('.queue-item').filter({hasText:'two-tabs-C.txt'}).count(),0);check('BroadcastChannel clears the other tab reader and queue');await page2.close();await closeSaved(page);
 const d=await upload(page,'interrupted-D.txt','interrupted request');await openSaved(page);deleteMode='hold-before';
 await confirmDelete(page,page.getByRole('button',{name:'Delete interrupted-D.txt',exact:true}));await waitFor(()=>deleteHeld);
 await page.reload();await retry(page).waitFor();deleteMode='normal';releaseDelete();await rowGone(d.id);await retry(page).click();await complete(page,d.id);check('intent before request survives reload; repeated cleanup confirms completion');
 const e=await upload(page,'lost-response-E.txt','lost response');await openSaved(page);deleteMode='lose-response';
 await confirmDelete(page,page.getByRole('button',{name:'Delete lost-response-E.txt',exact:true}));await rowGone(e.id);
 await page.locator('.saved-panel [role="alert"]').filter({hasText:'not confirmed complete'}).waitFor();assert.ok((await journal(page)).find(x=>x.id===e.id).pending);
 deleteMode='normal';await page.reload();await retry(page).click();await complete(page,e.id);check('lost success response remains visibly pending and recovers after reload');
 const f=await upload(page,'quota-F.txt','quota fixture');await openSaved(page);
 await page.evaluate(()=>{const proto=IDBObjectStore.prototype,original=proto.put;window.fixture.restorePut=()=>{proto.put=original;};proto.put=function(...args){if(this.name==='documentDeletions')throw new DOMException('Synthetic quota failure','QuotaExceededError');return original.apply(this,args);};});
 const before=deleteRequests;await confirmDelete(page,page.getByRole('button',{name:'Delete quota-F.txt',exact:true}));await page.locator('.saved-panel [role="alert"]').filter({hasText:'not confirmed complete'}).waitFor();
 assert.equal(deleteRequests,before);assert.ok(await bucket.head(`${f.id}/original`));await page.evaluate(()=>window.fixture.restorePut());check('failed IndexedDB intent prevents server deletion and retains retryable document');await closeSaved(page);
 await page.locator('input[aria-label="Choose source files"]').setInputFiles({name:'worker-G.pdf',mimeType:'application/pdf',buffer:Buffer.from('%PDF-1.7\nsynthetic worker test')});
 await page.locator('.queue-item').filter({hasText:'worker-G.pdf'}).getByText('Extracting page 1 of 2.').waitFor();
 const g=await db.prepare('SELECT id FROM documents WHERE original_name=?').bind('worker-G.pdf').first();assert.ok(g);
 await openSaved(page);await page.getByRole('button',{name:'Search',exact:true}).click();await confirmDelete(page,page.getByRole('button',{name:'Delete worker-G.pdf',exact:true}));await complete(page,g.id);await assertBytesGone(g.id);await closeSaved(page);
 await upload(page,'after-worker-H.txt','pump continues after deletion');assert.equal(await db.prepare('SELECT id FROM documents WHERE id=?').bind(g.id).first(),null);check('deletion aborts an active PDF worker and the queue continues');
 assert.deepEqual(errors,[]);check('no uncaught browser errors');console.log(JSON.stringify({browser:browser.version(),scope:'synthetic Chromium with production UI/IndexedDB/API and disposable D1/R2',checks}));
}finally{releaseDelete?.();await browser.close();await new Promise(resolve=>server.close(resolve));await mf.dispose();}
