/** Visible-control acceptance against the actual production build in local
 * Wrangler. Creates its own disposable database/bucket and browser profiles.
 * Run `pnpm build` first. Does not accept a deployed-site URL. */
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createServer } from 'node:net';
import { mkdtemp, readdir, readFile, rm } from 'node:fs/promises';
import { join, dirname, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
const web=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const {chromium}=await import(process.env.PLAYWRIGHT_MODULE||'playwright');
const temporary=await mkdtemp(join(tmpdir(),'tpe-built-deletion-'));
const fixtureEnv={...process.env,XDG_CONFIG_HOME:join(temporary,'config')};
const args=['--import',join(web,'scripts/sites-env.mjs'),join(web,'node_modules/wrangler/bin/wrangler.js')];
const config=join(web,'dist/server/wrangler.json'),persist=join(temporary,'state');
for(const file of (await readdir(join(web,'drizzle'))).filter(name=>name.endsWith('.sql')).sort()){
 const command=spawnSync(process.execPath,[...args,'d1','execute','DB','--local','--persist-to',persist,'--config',config,'--file',join(web,'drizzle',file)],{cwd:web,encoding:'utf8',env:fixtureEnv});
 assert.equal(command.status,0,command.stderr+command.stdout);
}
const portServer=createServer();await new Promise(r=>portServer.listen(0,'127.0.0.1',r));const port=portServer.address().port;await new Promise(r=>portServer.close(r));
const base=`http://127.0.0.1:${port}`;
const child=spawn(process.execPath,[...args,'dev','--local','--config',config,'--persist-to',persist,'--ip','127.0.0.1','--port',String(port),'--inspector-port','0'],{cwd:web,env:fixtureEnv,stdio:['ignore','pipe','pipe']});
let logs='';child.stdout.on('data',data=>{logs+=data;});child.stderr.on('data',data=>{logs+=data;});
const headers={'oai-authenticated-user-id':'synthetic-deletion-acceptance','oai-authenticated-user-email':'synthetic@example.invalid'};
let browser;
const checks=[];function check(name){checks.push(name);console.log('PASS',name);}
async function waitFor(predicate){for(let i=0;i<300;i++){if(await predicate())return;await new Promise(r=>setTimeout(r,100));}throw Error('Timed out waiting for built app\n'+logs.slice(-2500));}
try{
 await waitFor(async()=>{if(child.exitCode!==null)throw Error(logs);try{return(await fetch(base+'/api/documents',{headers})).ok;}catch{return false;}});
 browser=await chromium.launch({headless:true,...(process.env.CHROMIUM_EXECUTABLE_PATH?{executablePath:process.env.CHROMIUM_EXECUTABLE_PATH}:{})});
 for(const mobile of [false,true]){
  const context=await browser.newContext({extraHTTPHeaders:headers,viewport:mobile?{width:390,height:844}:{width:1280,height:900},isMobile:mobile,hasTouch:mobile});
  const page=await context.newPage(),errors=[];page.on('pageerror',error=>errors.push(String(error)));
  await page.goto(base);await page.locator('#source-paste').waitFor();
  // Real hydration: text input must enable the actual Send button.
  await page.locator('#source-paste').fill('synthetic hydration check');await page.getByRole('button',{name:'Import pasted source',exact:true}).waitFor();assert.ok(await page.getByRole('button',{name:'Import pasted source',exact:true}).isEnabled());await page.locator('#source-paste').fill('');
  const label=mobile?'mobile':'desktop';const names=[`${label}-delete.txt`,`${label}-neighbor.txt`];
  await page.getByRole('button',{name:'Upload',exact:true}).click();
  const [chooser]=await Promise.all([page.waitForEvent('filechooser'),page.getByRole('button',{name:'Add files',exact:true}).click()]);
  await chooser.setFiles(names.map(name=>({name,mimeType:'text/plain',buffer:Buffer.from('Synthetic source '+name)})));
  for(const name of names)await page.locator('.queue-item').filter({hasText:name}).locator('.phase-saved').waitFor({timeout:30000});
  const rows=(await(await context.request.get(base+'/api/documents')).json()).documents;
  const target=rows.find(row=>row.original_name===names[0]),neighbor=rows.find(row=>row.original_name===names[1]);assert.ok(target&&neighbor);
  check(`${label}: real Upload button/picker, two persisted originals and extraction results`);
  await page.getByRole('button',{name:'Saved articles',exact:true}).click();
  const button=page.getByRole('button',{name:'Delete '+names[0],exact:true});
  const box=await button.boundingBox();assert.ok(box&&box.width>20&&box.height>20&&box.x>=0&&box.x+box.width<=(mobile?390:1280));
  assert.ok(await button.evaluate(node=>{const box=node.getBoundingClientRect();return node.contains(document.elementFromPoint(box.x+box.width/2,box.y+box.height/2));}));
  // Opening the article or dismissing the saved menu must never activate Delete.
  let deletes=0;page.on('request',request=>{if(request.method()==='DELETE'&&request.url().includes('/api/documents/'))deletes++;});
  await page.locator('.document-item').filter({hasText:names[0]}).click();await page.locator('.reading').filter({hasText:names[0]}).waitFor();assert.equal(deletes,0);
  await page.getByRole('button',{name:'Saved articles',exact:true}).click();await page.keyboard.press('Escape');assert.equal(deletes,0);
  await page.getByRole('button',{name:'Saved articles',exact:true}).click();
  page.once('dialog',async dialog=>{assert.match(dialog.message(),new RegExp(names[0]));await dialog.dismiss();});await button.click();assert.equal(deletes,0);assert.ok((await context.request.get(base+`/api/documents/${target.id}/original`)).ok());
  check(`${label}: visible hit target, separate Open/Delete, Escape and cancelled confirmation are non-destructive`);
  // Repeated clicks still require explicit confirmation; cancelling both sends no DELETE.
  const dismiss=dialog=>dialog.dismiss();page.on('dialog',dismiss);await button.dblclick();page.off('dialog',dismiss);assert.equal(deletes,0);
  // Keyboard activation reaches the actual native confirmation.
  await button.focus();page.once('dialog',async dialog=>{assert.match(dialog.message(),/cannot be undone/);await dialog.accept();});
  await page.keyboard.press('Enter');
  await waitFor(async()=>!(await(await context.request.get(base+'/api/documents')).json()).documents.some(row=>row.id===target.id));
  await page.waitForFunction(()=>document.querySelector('[role="status"][aria-atomic="true"]')?.textContent.includes('deleted.'));
  assert.equal(deletes,1);assert.equal((await context.request.get(base+`/api/documents/${target.id}`)).status(),404);assert.equal((await context.request.get(base+`/api/documents/${target.id}/original`)).status(),404);
  assert.ok((await context.request.get(base+`/api/documents/${neighbor.id}/original`)).ok());
  assert.equal(await page.locator('.document-item').filter({hasText:names[0]}).count(),0);assert.equal(await page.locator('.queue-item').filter({hasText:names[0]}).count(),0);
  assert.equal(await page.evaluate(()=>document.activeElement?.getAttribute('aria-label')),'Close saved articles');
  check(`${label}: keyboard Delete removes the intended stored document, preserves neighbor and returns focus`);
  await page.reload();await page.locator('#source-paste').waitFor();await page.getByRole('button',{name:'Saved articles',exact:true}).click();
  await page.locator('.document-item').filter({hasText:names[1]}).waitFor();assert.equal(await page.locator('.document-item').filter({hasText:names[0]}).count(),0);assert.equal(await page.locator('.queue-item').filter({hasText:names[0]}).count(),0);
  const local=await page.evaluate(()=>new Promise((resolve,reject)=>{const request=indexedDB.open('tpe-private-workspace',3);request.onsuccess=()=>{const db=request.result,tx=db.transaction(['workspaces','documentDeletions'],'readonly');const snapshot=tx.objectStore('workspaces').get('synthetic-deletion-acceptance'),journal=tx.objectStore('documentDeletions').get('synthetic-deletion-acceptance');tx.oncomplete=()=>{db.close();resolve({snapshot:snapshot.result,journal:journal.result});};tx.onerror=()=>reject(tx.error);};request.onerror=()=>reject(request.error);}));
  assert.ok(!local.snapshot.items.some(item=>item.record?.id===target.id));assert.ok(local.journal.some(item=>item.id===target.id&&!item.pending));
  assert.equal((await context.request.delete(base+`/api/documents/${target.id}`,{data:{confirmDocumentId:target.id}})).status(),204);
  assert.deepEqual(errors,[]);check(`${label}: reload plus raw IndexedDB and repeated server deletion confirm persistence`);await context.close();
 }
 const info=JSON.parse(await readFile(config,'utf8'));const bundle=await readFile(resolve(dirname(config),info.main));
 console.log(JSON.stringify({commit:spawnSync('git',['rev-parse','HEAD'],{cwd:web,encoding:'utf8'}).stdout.trim(),worktreeDirty:!!spawnSync('git',['status','--porcelain'],{cwd:web,encoding:'utf8'}).stdout.trim(),serverBundleSha256:createHash('sha256').update(bundle).digest('hex'),browser:browser.version(),scope:'local production build via Wrangler; synthetic identity and isolated temporary D1/R2; desktop and touch emulation, not deployed Site or Safari',checks}));
}finally{await browser?.close();child.kill('SIGTERM');await new Promise(resolve=>child.exitCode!==null?resolve():child.once('exit',resolve));await rm(temporary,{recursive:true,force:true});}
