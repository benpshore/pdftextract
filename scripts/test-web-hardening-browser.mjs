#!/usr/bin/env node
/** Engineering integration fixtures against the real local app and disposable D1/R2.
 * No route interception, production credentials, deployment or formal-audit claim.
 * PLAYWRIGHT_MODULE and CHROMIUM_EXECUTABLE select an already installed browser.
 */
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import fs from 'node:fs/promises';
import {createRequire} from 'node:module';
import {createServer as reserveServer} from 'node:net';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),web=path.join(root,'web');
const req=createRequire(path.join(web,'package.json'));
const temporary=await fs.mkdtemp(path.join(os.tmpdir(),'tpe-hardening-browser-'));
const output=path.resolve(process.env.HARDENING_QA_OUTPUT||'/tmp/pdftextract-hardening-browser');
const stateRoot=path.join(temporary,'state'),checks=[],browserErrors=[],externalRequests=[],responses=[];
let platform,server,browser,baseUrl,chromiumVersion;
const pass=name=>{checks.push(name);console.log('PASS '+name);};
await fs.mkdir(output,{recursive:true});
for(const[name,value]of Object.entries({CLOUDFLARE_CF_FETCH_ENABLED:'false',WRANGLER_SEND_METRICS:'false',WRANGLER_WRITE_LOGS:'false',WRANGLER_LOG_PATH:path.join(temporary,'logs'),WRANGLER_REGISTRY_PATH:path.join(temporary,'registry'),MINIFLARE_REGISTRY_PATH:path.join(temporary,'registry')}))process.env[name]=value;
process.chdir(web);
const bindingConfig={name:'tpe-hardening-browser-fixture',main:path.join(web,'build/sites-worker.ts'),compatibility_date:'2026-05-15',compatibility_flags:['nodejs_compat','global_fetch_strictly_public'],d1_databases:[{binding:'DB',database_name:'hardening-fixture',database_id:'00000000-0000-4000-8000-000000000000'}],r2_buckets:[{binding:'BUCKET',bucket_name:'hardening-fixture'}]};
const configPath=path.join(temporary,'wrangler.json');await fs.writeFile(configPath,JSON.stringify(bindingConfig));
try{
  const {getPlatformProxy}=req('wrangler');
  platform=await getPlatformProxy({configPath,envFiles:[],persist:{path:path.join(stateRoot,'v3')},remoteBindings:false});
  for(const file of (await fs.readdir(path.join(web,'drizzle'))).filter(n=>n.endsWith('.sql')).sort()){
    for(const statement of (await fs.readFile(path.join(web,'drizzle',file),'utf8')).split('--> statement-breakpoint'))if(statement.trim())await platform.env.DB.exec(statement.replace(/\n/g,' '));
  }
  await platform.dispose();platform=null;
  const reserve=reserveServer();await new Promise(r=>reserve.listen(0,'127.0.0.1',r));const port=reserve.address().port;await new Promise(r=>reserve.close(r));
  const {createServer}=await import(pathToFileURL(req.resolve('vite')));
  const {default:vinext}=await import(pathToFileURL(path.join(web,'node_modules/vinext/dist/index.js')));
  const {cloudflare}=await import(pathToFileURL(path.join(web,'node_modules/@cloudflare/vite-plugin/dist/index.mjs')));
  const {sites}=await import(pathToFileURL(path.join(web,'build/sites-vite-plugin.ts')));
  server=await createServer({configFile:false,root:web,cacheDir:path.join(temporary,'node_modules/.vite'),server:{host:'127.0.0.1',port,strictPort:true,fs:{deny:['.env','.env.*','*.{crt,pem}','**/.git/**','**/.sites-runtime/**']}},plugins:[vinext(),sites({mockAuth:true}),cloudflare({config:bindingConfig,persistState:{path:stateRoot},remoteBindings:false,tunnel:false,inspectorPort:false,viteEnvironment:{name:'rsc',childEnvironments:['ssr']}})]});
  await server.listen();baseUrl=`http://127.0.0.1:${server.httpServer.address().port}`;
  for(const headers of [{},{'oai-authenticated-user-id':'spoofed'}])assert.equal((await fetch(baseUrl+'/api/capture',{method:'POST',headers:{'Content-Type':'application/json',...headers},body:JSON.stringify({url:'https://source.fixture.invalid/'})})).status,401);
  pass('Signed-out and spoofed capture requests remain unauthorized');
  const {chromium}=req(process.env.PLAYWRIGHT_MODULE||'playwright');
  browser=await chromium.launch({headless:true,executablePath:process.env.CHROMIUM_EXECUTABLE||'/usr/bin/chromium',args:['--no-sandbox','--disable-dev-shm-usage']});chromiumVersion=browser.version();
  const context=await browser.newContext({viewport:{width:1280,height:900}}),page=await context.newPage();
  page.on('pageerror',e=>browserErrors.push(String(e)));
  page.on('request',r=>{if(/^https?:/.test(r.url())&&!r.url().startsWith(baseUrl+'/'))externalRequests.push(r.url());});
  page.on('response',r=>{if(r.url().startsWith(baseUrl+'/api/'))responses.push({method:r.request().method(),path:new URL(r.url()).pathname,status:r.status()});});
  await page.goto(baseUrl+'/signin-with-chatgpt?return_to=%2F',{waitUntil:'networkidle',timeout:120000});
  const textarea=page.locator('#source-paste');await textarea.waitFor({timeout:30000});
  assert.match(await page.locator('[role=note]').innerText(),/fetching is disabled/);
  await textarea.fill('https://source.fixture.invalid/article');await page.getByRole('button',{name:'Import pasted source'}).click();
  assert.equal(await textarea.inputValue(),'https://source.fixture.invalid/article');assert.match(await page.getByRole('alert').innerText(),/disabled/);
  assert.equal(responses.filter(r=>r.path==='/api/capture').length,0);
  const rejected=await context.request.post(baseUrl+'/api/capture',{data:{url:'https://source.fixture.invalid/article'}});assert.equal(rejected.status(),503);assert.match(await rejected.text(),/disabled/);
  pass('Disabled URL intake preserves its draft; authenticated direct capture returns 503');
  const paragraph='An independently authored local study preserves measured values, Unicode α and a source link. '.repeat(10);
  const html=`<!doctype html><html><head><title>Local study</title><link rel="canonical" href="https://source.fixture.invalid/study"><link rel="alternate" type="application/rss+xml" href="https://source.fixture.invalid/feed"><link rel="stylesheet" href="https://resource.fixture.invalid/style.css"><meta name="citation_title" content="Local study"></head><body><article><h1>Local study</h1><p>${paragraph}<a href="https://source.fixture.invalid/paper">Source evidence</a></p><figure><img src="https://resource.fixture.invalid/figure.png"><figcaption>Retained caption.</figcaption></figure><iframe src="https://resource.fixture.invalid/frame"></iframe><script>window.__fixtureExecuted=true</script></article></body></html>`;
  await page.locator('input[aria-label="Choose source files"]').setInputFiles({name:'local-study.html',mimeType:'text/html',buffer:Buffer.from(html)});
  await page.waitForFunction(()=>[...document.querySelectorAll('.queue-item')].some(e=>e.textContent.includes('local-study.html')&&e.textContent.includes('Saved')),{},{timeout:60000});
  await page.locator('.queue-item').filter({hasText:'local-study.html'}).locator('.queue-open').click();
  await page.waitForFunction(()=>document.querySelector('.reading')?.textContent.includes('measured values'));
  assert.equal(await page.locator('.reading img,.reading iframe,.reading script').count(),0);
  assert.match(await page.locator('.reading').innerText(),/Retained caption/);
  assert.equal(await page.evaluate(()=>window.__fixtureExecuted),undefined);
  const list=await context.request.get(baseUrl+'/api/documents');const rows=(await list.json()).documents;assert.equal(rows.length,1);const record=rows[0];
  const saved=await context.request.get(baseUrl+`/api/documents/${record.id}`),result=(await saved.json()).result;
  assert.equal(result.metadata.canonical,'https://source.fixture.invalid/study');assert.equal(result.metadata.feeds[0].url,'https://source.fixture.invalid/feed');assert.equal(result.metadata.meta.citation_title,'Local study');
  const original=await context.request.get(baseUrl+`/api/documents/${record.id}/original`);assert.equal(createHash('sha256').update(await original.body()).digest('hex'),createHash('sha256').update(html).digest('hex'));
  const remoteAsset=await context.request.post(baseUrl+`/api/documents/${record.id}/assets`,{data:{url:'https://resource.fixture.invalid/figure.png'}});assert.equal(remoteAsset.status(),503);
  pass('Local HTML saves exact originals and readable text, captions, links and canonical/feed metadata; remote assets remain disabled');
  await textarea.fill('Pasted text remains a local source, including Unicode 日本語.');await page.getByRole('button',{name:'Import pasted source'}).click();
  await page.waitForFunction(()=>[...document.querySelectorAll('.queue-item')].some(e=>e.textContent.includes('Pasted text.txt')&&e.textContent.includes('Saved')),{},{timeout:60000});
  await page.locator('.queue-item').filter({hasText:'Pasted text.txt'}).locator('.queue-open').click();assert.match(await page.locator('.reading').innerText(),/日本語/);
  await page.reload({waitUntil:'networkidle'});await page.waitForFunction(()=>document.querySelector('.reading')?.textContent.includes('日本語'));
  pass('Paste, save, read and reload use the real authenticated local app/storage');
  assert.deepEqual(externalRequests,[]);assert.deepEqual(browserErrors,[]);
  pass('No browser external-resource requests or runtime errors');
  await page.screenshot({path:path.join(output,'local-hardening-desktop.png'),fullPage:true});
  await fs.writeFile(path.join(output,'report.json'),JSON.stringify({checks,chromiumVersion,baseUrl,browserErrors,externalRequests,responses,scope:'Engineering synthetic local-app interactions only; local sign-in middleware, no deployed identity/own-zone/rebinding/accuracy or formal audit qualification.'},null,2));
}finally{
  await browser?.close();await server?.close();await platform?.dispose();await fs.rm(temporary,{recursive:true,force:true});
}
