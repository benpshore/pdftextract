#!/usr/bin/env node
/**
 * Full local app browser QA: actual vinext routes, workerd D1/R2, local Sites
 * sign-in middleware, browser PDF worker and citation UI. No API interception.
 * The scholarly bridge may replay captured native output; this is explicitly
 * distinct from running GROBID/native extraction during this browser test.
 *
 * node scripts/test-web-scholarly-browser.mjs [--probe-local-stack]
 * PLAYWRIGHT_MODULE and CHROMIUM_EXECUTABLE override the browser installation.
 * SCHOLARLY_QA_OUTPUT defaults to /tmp/pdftextract-scholarly-browser-qa.
 * SCHOLARLY_QA_RUNTIME_URL explicitly selects an existing literal loopback
 * runtime; otherwise the harness starts an isolated captured-native replay.
 * Only disposable local storage and independently authored fixtures are used.
 */
import assert from 'node:assert/strict';
import {createHash, randomUUID} from 'node:crypto';
import fs from 'node:fs/promises';
import {createRequire} from 'node:module';
import {createServer as createNetServer} from 'node:net';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';

const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const web=path.join(root,'web');
const require=createRequire(import.meta.url);
const webRequire=createRequire(path.join(web,'package.json'));
const output=path.resolve(process.env.SCHOLARLY_QA_OUTPUT||'/tmp/pdftextract-scholarly-browser-qa');
const temporary=await fs.mkdtemp(path.join(os.tmpdir(),'tpe-scholarly-browser-'));
const stateRoot=path.join(temporary,'state');
const checks=[],browserErrors=[],responses=[];
const pass=name=>{checks.push(name);console.log('PASS '+name);};
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
const probeOnly=process.argv.includes('--probe-local-stack');
const probeUpload=process.argv.includes('--probe-upload');
const fixtureDirectory=path.join(web,'tests/fixtures/scholarly');
const foreignId=randomUUID();
const sourcePdf=await fs.readFile(path.join(fixtureDirectory,'synthetic-scholarly.pdf'));
const nativeJson=await fs.readFile(path.join(fixtureDirectory,'grobid-pr206-fixed-live.json'),'utf8');
const nativeResult=JSON.parse(nativeJson);
assert.equal(sha(sourcePdf),nativeResult.source_sha256,'Captured native output must match the exact fixture PDF');
const externalRuntime=process.env.SCHOLARLY_QA_RUNTIME_URL;
if(externalRuntime){const url=new URL(externalRuntime);assert.equal(url.protocol,'http:');assert.equal(url.hostname,'127.0.0.1');assert.equal(url.pathname,'/');assert.equal(url.username+url.password+url.search+url.hash,'');}
let runtimeMode='captured-native';
await fs.mkdir(output,{recursive:true});
for(const [name,value] of Object.entries({
  CLOUDFLARE_CF_FETCH_ENABLED:'false',WRANGLER_SEND_METRICS:'false',WRANGLER_WRITE_LOGS:'false',
  WRANGLER_LOG_PATH:path.join(temporary,'logs'),WRANGLER_REGISTRY_PATH:path.join(temporary,'registry'),
  MINIFLARE_REGISTRY_PATH:path.join(temporary,'registry'),
}))process.env[name]=value;
process.chdir(web);
await import(pathToFileURL(path.join(web,'scripts/copy-pdf-wasm.mjs')).href);
await import(pathToFileURL(path.join(web,'scripts/copy-ocr-assets.mjs')).href);

const bindingConfig={
  name:'tpe-scholarly-browser-fixture',main:path.join(web,'build/sites-worker.ts'),
  compatibility_date:'2026-05-15',compatibility_flags:['nodejs_compat'],
  d1_databases:[{binding:'DB',database_name:'scholarly-browser-fixture',database_id:'00000000-0000-4000-8000-000000000000'}],
  r2_buckets:[{binding:'BUCKET',bucket_name:'scholarly-browser-fixture'}],
};
const configPath=path.join(temporary,'wrangler.json');
await fs.writeFile(configPath,JSON.stringify(bindingConfig));
const {getPlatformProxy}=webRequire('wrangler');
let platform,server,browser,runtime;
let baseUrl,chromiumVersion;
try{
  // The plugin adds /v3; getPlatformProxy accepts the effective directory.
  platform=await getPlatformProxy({configPath,envFiles:[],persist:{path:path.join(stateRoot,'v3')},remoteBindings:false});
  for(const file of (await fs.readdir(path.join(web,'drizzle'))).filter(name=>name.endsWith('.sql')).sort()){
    const migration=await fs.readFile(path.join(web,'drizzle',file),'utf8');
    for(const statement of migration.split('--> statement-breakpoint'))if(statement.trim())await platform.env.DB.exec(statement.replace(/\n/g,' '));
  }
  await platform.env.DB.prepare('INSERT INTO documents (id,owner,title,kind,original_name,mime,status,engine,sha256,bytes,created_at,search_text) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)').bind(foreignId,'another-synthetic-owner','Foreign synthetic document.pdf','pdf','Foreign synthetic document.pdf','application/pdf','uploaded','',sha(sourcePdf),sourcePdf.length,'2026-10-04T00:00:00Z','').run();
  await platform.env.BUCKET.put(`${foreignId}/original`,sourcePdf,{httpMetadata:{contentType:'application/pdf'}});
  await platform.dispose();platform=null;

  const auxiliaryWorkers=[];
  if(!probeOnly){
    let runtimeUrl=externalRuntime;
    if(!runtimeUrl){
      const {configuration,createLocalRuntime}=await import('./scholarly-local-runtime.mjs');
      runtime=createLocalRuntime(configuration({SCHOLARLY_LOCAL_PORT:'0',SCHOLARLY_CAPTURED_JSON:path.join(fixtureDirectory,'grobid-pr206-fixed-live.json')}));
      runtimeUrl=await runtime.listen();
    }
    const health=await (await fetch(runtimeUrl+'/health')).json();assert.equal(health.status,'ready',JSON.stringify(health));assert.ok(['live-native','captured-native'].includes(health.mode));runtimeMode=health.mode;
    bindingConfig.services=[{binding:'SCHOLARLY',service:'tpe-scholarly-local-fixture'}];
    auxiliaryWorkers.push({config:{name:'tpe-scholarly-local-fixture',main:path.join(web,'build/scholarly-local-worker.mjs'),compatibility_date:'2026-05-15',vars:{SCHOLARLY_LOCAL_URL:runtimeUrl}}});
  }
  // Vite treats port 0 as its default; reserve a loopback port explicitly.
  const reserve=createNetServer();await new Promise(resolve=>reserve.listen(0,'127.0.0.1',resolve));
  const port=reserve.address().port;await new Promise(resolve=>reserve.close(resolve));

  const {createServer}=await import(pathToFileURL(webRequire.resolve('vite')).href);
  const {default:vinext}=await import(pathToFileURL(path.join(web,'node_modules/vinext/dist/index.js')).href);
  const {cloudflare}=await import(pathToFileURL(path.join(web,'node_modules/@cloudflare/vite-plugin/dist/index.mjs')).href);
  const {sites}=await import(pathToFileURL(path.join(web,'build/sites-vite-plugin.ts')).href);
  server=await createServer({
    configFile:false,root:web,cacheDir:path.join(temporary,'node_modules/.vite'),
    server:{host:'127.0.0.1',port,strictPort:true,fs:{deny:['.env','.env.*','*.{crt,pem}','**/.git/**','**/.sites-runtime/**']}},
    plugins:[vinext(),sites({mockAuth:true}),cloudflare({
      config:bindingConfig,persistState:{path:stateRoot},remoteBindings:false,tunnel:false,inspectorPort:false,auxiliaryWorkers,
      viteEnvironment:{name:'rsc',childEnvironments:['ssr']},
    })],
  });
  await server.listen();
  const address=server.httpServer.address();assert.equal(typeof address,'object');
  baseUrl=`http://127.0.0.1:${address.port}`;
  console.log('Local full app: '+baseUrl);
  const signedOut=await fetch(baseUrl+'/api/documents');assert.equal(signedOut.status,401,await signedOut.text());
  const spoofed=await fetch(baseUrl+'/api/documents',{headers:{'oai-authenticated-user-id':'forged','oai-authenticated-user-email':'forged@fixture.invalid'}});assert.equal(spoofed.status,401,await spoofed.text());
  pass('actual development auth rejects signed-out and spoofed identity requests');

  const {chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
  browser=await chromium.launch({headless:true,executablePath:process.env.CHROMIUM_EXECUTABLE||'/usr/bin/chromium',args:['--no-sandbox','--disable-dev-shm-usage']});
  chromiumVersion=browser.version();
  const context=await browser.newContext({viewport:{width:1280,height:900},acceptDownloads:true});
  const page=await context.newPage();page.on('pageerror',error=>browserErrors.push(String(error)));
  page.on('response',response=>{if(response.url().startsWith(baseUrl+'/api/'))responses.push({method:response.request().method(),path:new URL(response.url()).pathname,status:response.status()});});
  await page.goto(baseUrl+'/signin-with-chatgpt?return_to=%2F',{waitUntil:'networkidle',timeout:120000});
  await page.getByText('Upload',{exact:true}).waitFor({timeout:30000});
  const listed=await context.request.get(baseUrl+'/api/documents');assert.equal(listed.status(),200,await listed.text());assert.deepEqual((await listed.json()).documents,[]);
  pass('real local sign-in renders Workspace and reads the migrated isolated D1 library');
  await page.screenshot({path:path.join(output,'full-app-empty-desktop.png'),fullPage:true});

  if(!probeOnly){
    const originalRead=await context.request.get(baseUrl+`/api/documents/${foreignId}/original`);assert.equal(originalRead.status(),404);
    for(const suffix of ['scholarly','scholarly/evidence?format=native-json']){
      const denied=await context.request.get(baseUrl+`/api/documents/${foreignId}/${suffix}`);assert.equal(denied.status(),404,await denied.text());
    }
    const deniedWrite=await context.request.post(baseUrl+`/api/documents/${foreignId}/scholarly`,{data:{baseResultKey:null}});assert.equal(deniedWrite.status(),404,await deniedWrite.text());
    pass('actual document/scholarly/evidence routes reject the separately seeded foreign owner');
    await page.getByLabel('Choose source files',{exact:true}).setInputFiles(path.join(fixtureDirectory,'synthetic-scholarly.pdf'));
    await page.locator('.queue-item .phase-saved').first().waitFor({timeout:120000});
    const library=await (await context.request.get(baseUrl+'/api/documents')).json();
    assert.equal(library.documents.length,1);const record=library.documents[0];
    assert.equal(record.sha256,sha(sourcePdf));assert.equal(record.original_name,'synthetic-scholarly.pdf');
    const opened=await context.request.get(baseUrl+`/api/documents/${record.id}`);assert.equal(opened.status(),200);const before=await opened.json();
    assert.equal(before.result.engine,'pdf-oxide-wasm 0.3.77');assert.match(before.result.text,/Synthetic Study of Reproducible Document Processing/);
    const storedOriginal=await context.request.get(baseUrl+`/api/documents/${record.id}/original`);assert.equal(sha(await storedOriginal.body()),sha(sourcePdf));
    pass('real browser PDF Worker and multipart API save a readable result with exact original bytes in local D1/R2');
    const health=await context.request.get(baseUrl+`/api/documents/${record.id}/scholarly`);assert.equal(health.status(),200);const availability=await health.json();assert.equal(availability.available,true,JSON.stringify(availability));assert.equal(availability.mode,runtimeMode);
    pass(`real scholarly route reaches explicit ${runtimeMode} loopback bridge through its service binding`);
    if(!probeUpload){
      await page.locator('.reader-secondary > details').filter({has:page.locator('summary', {hasText:/^Citations$/})}).locator('summary').first().click();
      await page.getByRole('button',{name:'Extract references',exact:true}).click();
      await page.getByText('Reference result saved.',{exact:false}).waitFor({timeout:120000});
      const after=await (await context.request.get(baseUrl+`/api/documents/${record.id}`)).json();
      assert.notEqual(after.record.result_key,before.record.result_key);
      for(const field of ['text','markdown','html','title','engine'])assert.deepEqual(after.result[field],before.result[field]);
      assert.equal(after.result.bibliography.references.items.length,2);
      assert.equal(after.result.bibliography.mentions.state,'unavailable');
      assert.equal(after.result.bibliography.source.original_name,'synthetic-scholarly.pdf');
      assert.equal(after.result.metadata.scholarly.mode,runtimeMode);
      assert(after.result.warnings.some(warning=>/coordinate/i.test(warning)));
      await page.getByRole('article',{name:'Supplied reference'}).first().waitFor();
      assert.equal(await page.getByRole('article',{name:'Supplied reference'}).count(),2);
      pass('mounted controls save two reported references while preserving reading, filename and missing-coordinate/in-text evidence');
      for(const [format,expected] of [['native-json',runtimeMode==='captured-native'?nativeJson:null],['tei',runtimeMode==='captured-native'?JSON.parse(nativeJson).raw_tei:null]]){
        const evidence=await context.request.get(baseUrl+`/api/documents/${record.id}/scholarly/evidence?format=${format}`);
        assert.equal(evidence.status(),200);assert.equal(evidence.headers()['content-type'],'application/octet-stream');
        const body=await evidence.text();if(expected!==null)assert.equal(body,expected);
        if(format==='native-json'){const actual=JSON.parse(body);assert.equal(actual.source_sha256,sha(sourcePdf));assert.equal(sha(actual.raw_tei),actual.tei_sha256);}
      }
      pass('authenticated passive evidence downloads retain exact native JSON/raw TEI with source and TEI hashes');
      const downloadPromise=page.waitForEvent('download');await page.getByRole('button',{name:'Download CSV',exact:true}).click();
      const csvDownload=await downloadPromise;await csvDownload.saveAs(path.join(output,'references.csv'));
      const csv=await fs.readFile(path.join(output,'references.csv'),'utf8');assert.match(csv,/"type","id"/);assert.equal(csv.trim().split('\n').length,3);
      await page.getByLabel('Citation view',{exact:true}).selectOption('mentions');await page.getByText('In-text mentions unavailable.',{exact:true}).waitFor();
      await page.getByLabel('Citation view',{exact:true}).selectOption('csv');assert.equal(await page.getByRole('article',{name:/CSV row reference/}).count(),2);
      await page.screenshot({path:path.join(output,'scholarly-desktop.png'),fullPage:true});
      await page.reload({waitUntil:'networkidle'});await page.getByRole('heading',{name:before.result.title,exact:true}).waitFor();
      const reloaded=await (await context.request.get(baseUrl+`/api/documents/${record.id}`)).json();assert.equal(reloaded.record.result_key,after.record.result_key);
      assert.equal(reloaded.result.bibliography.references.items.length,2);
      pass('reference CSV, explicit unavailable mentions, CSV row view and reload retain the saved scholarly attachment');
    }
  }
  assert.deepEqual(browserErrors,[]);pass('no browser runtime errors');
  const report={checks,chromiumVersion,baseUrl,browserErrors,responses,mode:probeOnly?'local-stack-probe':probeUpload?runtimeMode+'-upload-probe':runtimeMode,fixtures:{pdf:sha(sourcePdf),capturedNativeJson:sha(nativeJson)},scope:'Actual local app routes, development sign-in, browser PDF WASM Worker and workerd D1/R2. '+(runtimeMode==='captured-native'?'Scholarly extraction explicitly replays captured PR206 native output for its exact independently authored PDF; no live native/GROBID execution. ':'Scholarly requests use an explicitly configured loopback live native/GROBID runtime. ')+'No production access or deployment; one synthetic PDF does not establish general scholarly extraction accuracy.',sources:{}};
  for(const file of ['web/app/workspace.tsx','web/app/globals.css','web/build/sites-vite-plugin.ts','web/build/sites-worker.ts','web/components/scholarly-controls.tsx','web/lib/scholarly-adapter.ts','web/lib/scholarly-service.ts','web/lib/scholarly-client.ts','web/lib/scholarly-naming.ts','scripts/test-web-scholarly-browser.mjs'])report.sources[file]=sha(await fs.readFile(path.join(root,file)));
  await fs.writeFile(path.join(output,'report.json'),JSON.stringify(report,null,2)+'\n');
  console.log(JSON.stringify({passed:checks.length,report:path.join(output,'report.json')}));
}catch(error){
  if(browser){const pages=browser.contexts().flatMap(context=>context.pages());for(const page of pages){await page.screenshot({path:path.join(output,'failure.png'),fullPage:true}).catch(()=>{});await fs.writeFile(path.join(output,'failure-body.txt'),await page.locator('body').innerText().catch(()=>''));}}
  await fs.writeFile(path.join(output,'failure.json'),JSON.stringify({error:String(error),stack:error.stack,checks,browserErrors,responses},null,2)+'\n');
  throw error;
}finally{
  await browser?.close();await server?.close();await platform?.dispose();await runtime?.close();
  await fs.rm(temporary,{recursive:true,force:true});
}
