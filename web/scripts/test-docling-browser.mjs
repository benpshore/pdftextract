import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,dirname,extname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {browserMemorySampler} from './browser-process-memory.mjs';
import {columnDerivatives} from './fixtures/docling/column-derivatives.mjs';
import {workerMemoryProbe} from './worker-memory-probe.mjs';
const {chromium}=await import(process.env.PLAYWRIGHT_MODULE||'playwright');
const {PDFDocument,StandardFonts}=await import(process.env.PDF_LIB_MODULE||'pdf-lib');
const web=resolve(dirname(fileURLToPath(import.meta.url)),'..'),repo=resolve(web,'..');
const native=await readFile(resolve(repo,'tests/fixtures/pdfium-unicode/native.pdf'));
assert.equal(createHash('sha256').update(native).digest('hex'),'099a620bd29179e329704c152808ad8e3e34f0d5388894c43d17fb3340d373c8');
const expected=['Faithful native text remains available.','Existing OCR already reads this sentence.','Numbers 12345 and alpha beta gamma.'];
const fixtures=new Map([['native.pdf',native]]);
for(const [name,sha] of [['semantic-two','fd5204522a871834507c26d75202dba08d0eb175e5e303f4c75582011d1f8c14'],['semantic-three','07832050f7553fcaa7e33cbb565e2236aac17243c2eec09ded69afc22f48d75a']]){
  const bytes=await readFile(resolve(web,'scripts/fixtures/docling',name+'.pdf'));assert.equal(createHash('sha256').update(bytes).digest('hex'),sha);fixtures.set(name+'.pdf',bytes);
}
const requested=[],blocked=[],pageErrors=[],partialRuns=[];
const server=createServer(async(req,res)=>{
  try{
    const path=new URL(req.url,'http://localhost').pathname;
    if(path==='/'){res.setHeader('Content-Type','text/html');res.end('<!doctype html><title>Docling PDF runtime evidence</title>');return;}
    if(path.startsWith('/fixtures/')){const bytes=fixtures.get(path.slice(10));if(bytes){res.setHeader('Content-Type','application/pdf');res.end(bytes);return;}}
    if(path.startsWith('/pdf-api/')||path.startsWith('/vendor/')||path==='/pdf-worker.js'){
      const file=resolve(web,'public','.'+path);if(!file.startsWith(resolve(web,'public')+'/'))throw new Error('Invalid path');
      const data=await readFile(file);res.setHeader('Content-Type',({'.js':'text/javascript','.mjs':'text/javascript','.wasm':'application/wasm','.html':'text/html'}[extname(file)]||'application/octet-stream'));res.setHeader('Content-Length',data.length);res.end(req.method==='HEAD'?undefined:data);return;
    }
  }catch{/* missing asset => explicit 404 */}res.statusCode=404;res.end();
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));const origin=`http://127.0.0.1:${server.address().port}`;
let browser,browserServer,memory;
try{
  browserServer=await chromium.launchServer({executablePath:process.env.CHROMIUM_PATH,args:['--no-sandbox']});
  browser=await chromium.connect(browserServer.wsEndpoint());
  memory=browserMemorySampler(browserServer.process().pid);
  const context=await browser.newContext();await context.route('**/*',async route=>{
    const url=route.request().url();if(url.startsWith(origin+'/')){requested.push(url.slice(origin.length));await route.continue();}else{blocked.push(url);await route.abort();}
  });
  if(process.env.MEMORY_PROBE==='1')await context.route('**/pdf-api/v1/inference-worker.js',async route=>{
    const source=await readFile(resolve(web,'public/pdf-api/v1/inference-worker.js'),'utf8');
    await route.fulfill({contentType:'text/javascript',body:`(${workerMemoryProbe.toString()})();\n${source}`});
  });
  const page=await context.newPage();await page.exposeFunction('beginResourceSample',label=>memory.begin(label));await page.exposeFunction('endResourceSample',()=>memory.end());await page.exposeFunction('recordMemoryPhase',event=>memory.mark(event));page.on('console',msg=>{if(msg.text().startsWith('ACTUAL PDF RESULT'))console.log(msg.text());else if(msg.text().startsWith('PDF CHECK'))console.log(msg.text());else if(msg.type()==='error')console.error('Browser:',msg.text());});page.on('pageerror',e=>{pageErrors.push(e.message);console.error(e);});await page.goto(origin);
  await page.exposeFunction('recordRun',async run=>{partialRuns.push(run);if(process.env.EVIDENCE_PATH)await writeFile(process.env.EVIDENCE_PATH,JSON.stringify({complete:false,browser:browser.version(),runs:partialRuns,pageErrors},null,2));});
  // Derive scan pixels from the pre-existing verified public-safe control PDF.
  const jpeg=await page.evaluate(async()=>{
    const pdfjs=await import('/vendor/docling/1.104.2/pdfjs/pdf.mjs');pdfjs.GlobalWorkerOptions.workerSrc='/vendor/docling/1.104.2/pdfjs/pdf.worker.mjs';
    const pdf=await pdfjs.getDocument({data:await(await fetch('/fixtures/native.pdf')).arrayBuffer()}).promise;
    const p=await pdf.getPage(1),viewport=p.getViewport({scale:2}),canvas=document.createElement('canvas');canvas.width=viewport.width;canvas.height=viewport.height;
    await p.render({canvas,canvasContext:canvas.getContext('2d'),viewport}).promise;const value=canvas.toDataURL('image/jpeg',1).split(',')[1];await pdf.destroy();return value;
  });
  const scan=await PDFDocument.create();const image=await scan.embedJpg(Buffer.from(jpeg,'base64'));scan.addPage([612,792]).drawImage(image,{x:0,y:0,width:612,height:792});fixtures.set('scanned.pdf',Buffer.from(await scan.save()));
  const six=await PDFDocument.create();const original=await PDFDocument.load(native);for(let i=0;i<6;i++)six.addPage((await six.copyPages(original,[0]))[0]);fixtures.set('six-pages.pdf',Buffer.from(await six.save()));
  const mixed=await PDFDocument.load(native);mixed.addPage((await mixed.copyPages(scan,[0]))[0]);fixtures.set('mixed.pdf',Buffer.from(await mixed.save()));
  // Genuine same-page mixture: verified vector text above its raster derivative.
  const samePage=await PDFDocument.create(),samePageCanvas=samePage.addPage([612,1584]);
  samePageCanvas.drawPage((await samePage.embedPdf(native))[0],{x:0,y:792,width:612,height:792});
  samePageCanvas.drawImage(await samePage.embedJpg(Buffer.from(jpeg,'base64')),{x:0,y:0,width:612,height:792});
  fixtures.set('same-page.pdf',Buffer.from(await samePage.save()));
  const overlay=await PDFDocument.create(),overlayPage=overlay.addPage([612,792]);
  overlayPage.drawImage(await overlay.embedJpg(Buffer.from(jpeg,'base64')),{x:0,y:0,width:612,height:792});
  overlayPage.drawPage((await overlay.embedPdf(native))[0],{x:0,y:0,width:612,height:792});
  fixtures.set('overlay.pdf',Buffer.from(await overlay.save()));
  const pair=await PDFDocument.load(native),pairImage=await pair.embedJpg(Buffer.from(jpeg,'base64'));
  pair.getPage(0).drawImage(pairImage,{x:0,y:0,width:300,height:388});pair.getPage(0).drawImage(pairImage,{x:312,y:0,width:300,height:388});
  fixtures.set('two-images.pdf',Buffer.from(await pair.save()));
  const columnCases=await columnDerivatives(PDFDocument,StandardFonts,fixtures,expected);
  for(const fixture of columnCases)fixtures.set(fixture.name,Buffer.from(fixture.bytes));
  const result=await page.evaluate(async({expected,columnCases})=>{
    const api=await import('/pdf-api/v1/api.js'),checks=[],runs=[];
    const check=(condition,message)=>{if(!condition)throw new Error(message);checks.push(message);console.log('PDF CHECK',message);};
    const file=async name=>new File([await(await fetch('/fixtures/'+name)).arrayBuffer()],name,{type:'application/pdf'});
    const discovery=await api.discoverPdf();check(discovery.engine.version==='1.104.2'&&discovery.capabilities.provider.configured==='wasm','Identifiable discovery declares CPU/WASM and schema');
    for(const [name,options] of [['native.pdf',{ocr:'off'}],['native.pdf',{ocr:'off'}],['scanned.pdf',{}],['mixed.pdf',{}],['six-pages.pdf',{ocr:'off'}],['semantic-two.pdf',{ocr:'off'}],['semantic-three.pdf',{ocr:'off'}],['same-page.pdf',{}],['same-page.pdf',{ocr:'always'}],['overlay.pdf',{}],...columnCases.map(c=>[c.name,{ocr:'off'}])]){
      const input=await file(name),before=await input.arrayBuffer(),progress=[];
      await globalThis.beginResourceSample(`${name}:${runs.length}`);
      const started=performance.now();
      const job=api.createPdfJob(input,{...options,onProgress:event=>{progress.push(event);void globalThis.recordMemoryPhase(event);}});
      let busy=false;try{api.createPdfJob(input);}catch(error){busy=error.code==='BUSY';}check(busy,`${name}: concurrent job rejected before allocating another runtime`);
      check(api.getPdfJob(job.id)===job,`${name}: callable job status`);
      const output=await job.result;const memory=await globalThis.endResourceSample();runs.push({name,output,progress,milliseconds:Math.round(performance.now()-started),memory});
      await globalThis.recordRun(runs.at(-1));
      console.log('ACTUAL PDF RESULT',name,JSON.stringify({status:output.status,text:output.text,diagnostics:output.diagnostics}));
      check(output.status==='partial',`${name}: real Docling PDF inference returns explicit partial coverage`);
      check(output.engine.provider==='wasm'&&output.pages.length>0,`${name}: CPU provenance and page output`);
      check(progress.some(p=>p.phase==='inference')&&progress.some(p=>p.phase==='page-complete'),`${name}: real inference progress`);
      check((await input.arrayBuffer()).byteLength===before.byteLength&&new Uint8Array(await input.arrayBuffer()).every((b,i)=>b===new Uint8Array(before)[i]),`${name}: original unchanged`);
      if(['native.pdf','scanned.pdf','mixed.pdf','six-pages.pdf','overlay.pdf'].includes(name))for(const p of output.pages)check(p.text.replace(/\s+/g,' ').trim()===expected.join(' '),`${name}: page ${p.page} exact independent expected text`);
      check(output.resources.rustPdfParses===(name==='scanned.pdf'||options.ocr==='always'?0:1),`${name}: zero scan parses or one digital Rust parse`);
      check(output.resources.pdfOxideDocuments===0,`${name}: PDF Oxide is not loaded in a Docling job`);
      check(output.resources.pdfJsClosedBeforeFinalInference===true,`${name}: renderer lifetime ends before final-page inference`);
      if(name==='scanned.pdf')check(output.pages[0].route==='scanned'&&output.pages[0].ocr==='page','Scan runs ScannedConverter with detector');
      if(name==='mixed.pdf')check(output.pages.map(p=>`${p.page}:${p.route}`).join(',')==='1:digital,2:scanned','Mixed PDF preserves digital/scanned routes and original page identity');
      if(name==='same-page.pdf'&&options.ocr==='always')check(output.text.replace(/\s+/g,' ').trim()===[...expected,...expected].join(' '),'Same-page vector/raster mixture: OCR always recovers both exact independent text copies');
      if(name==='same-page.pdf'&&options.ocr!=='always'){
        check(output.text.replace(/\s+/g,' ').trim()===[...expected,...expected].join(' '),'Same-page AUTO recovers native and raster content exactly without whole-page OCR');
        check(output.pages[0].route==='digital'&&output.pages[0].ocr==='regions'&&output.pages[0].ocrRegions.length===1,'Same-page AUTO retains native document and recognizes one raster region');
        check(output.pages[0].orderedBlocks.some(b=>b.source==='native text')&&output.pages[0].orderedBlocks.some(b=>b.source==='raster-region OCR'),'Mixed result keeps distinct native and raster provenance');
      }
      if(name==='overlay.pdf')check(output.pages[0].ocrRegions[0].maskedNativeCells.length===3,'Spatial native-text masks prevent duplicate recognition of an overlapping raster text layer');
      const column=columnCases.find(c=>c.name===name);
      if(column)check(output.text.replace(/\s+/g,' ').trim()===column.expected.join(' '),`${name}: independent exact order for ${column.purpose}`);
    }
    check(runs[0].output.resources.runtimeId===runs[1].output.resources.runtimeId,'Successful jobs reuse the same owned model worker');
    check(runs[0].output.pages[0].execution[0].sessionId===runs[1].output.pages[0].execution[0].sessionId,'Warm job reuses the existing layout ONNX session');
    check(!runs[1].progress.some(e=>e.phase==='assets'),'Warm job does not load model assets again');
    check(runs[0].output.pages[0].execution[0].outputs.logits.sha256===runs[1].output.pages[0].execution[0].outputs.logits.sha256,'Repeated actual input yields identical layout tensor fingerprint');
    check(runs[0].output.pages[0].execution[0].outputs.logits.sha256!==runs.find(r=>r.name==='semantic-two.pdf').output.pages[0].execution[0].outputs.logits.sha256,'Different PDF pixels produce a different actual ONNX output');
    // These independent semantic witnesses remain acceptance gates even if models fail them.
    const input=await file('scanned.pdf');
    const missing=api.createPdfJob(input,{ocr:'off'});const m=await missing.result;check(m.status==='failed'&&m.diagnostics.some(d=>d.code==='OCR_REQUIRED'),'OCR off on a scan is meaningful failure');
    const abort=api.createPdfJob(input,{onProgress:event=>{if(event.phase==='model-inference')api.cancelPdfJob(event.jobId);}});
    const aborted=await abort.result;check(aborted.status==='cancelled','Cancellation terminates an actual rendered PDF operation');
    const next=await api.createPdfJob(await file('native.pdf'),{ocr:'off'}).result;
    check(next.status==='partial'&&expected.every(s=>next.text.includes(s)),'Successful subsequent PDF after interruption');
    const selection=await api.createPdfJob(await file('mixed.pdf'),{pages:[2]}).result;
    check(selection.pages[0]?.page===2&&selection.pages[0].layout.every(b=>b.provenance.every(p=>p.page_no===2)),'Page selection remaps provenance to original page 2');
    const timeout=await api.createPdfJob(await file('native.pdf'),{ocr:'off',limits:{timeoutMs:5}}).result;
    check(timeout.status==='failed'&&timeout.diagnostics.some(d=>d.code==='TIMEOUT'),'Time budget produces failed TIMEOUT instead of empty success');
    const bytes=await(await file('native.pdf')).arrayBuffer();
    let invalidSignal=false;try{api.createPdfJob(bytes,{signal:{}});}catch(error){invalidSignal=error.code==='INVALID_OPTIONS';}check(invalidSignal,'Invalid signal cannot strand the shared active-job slot');
    for(const [name,options,code] of [
      ['six-pages.pdf',{limits:{maxPages:2}},'PAGE_LIMIT'],
      ['six-pages.pdf',{pages:[1],limits:{maxDocumentPages:2}},'DOCUMENT_PAGE_LIMIT'],
      ['native.pdf',{limits:{maxPixels:100}},'PIXEL_LIMIT'],
      ['native.pdf',{limits:{maxTextCells:1}},'TEXT_LIMIT'],
      ['two-images.pdf',{limits:{maxOcrRegions:1}},'REGION_LIMIT'],
      ['six-pages.pdf',{ocr:'off',limits:{maxOutputBytes:16384}},'OUTPUT_LIMIT'],
      ['six-pages.pdf',{engine:'fast-text',ocr:'off',limits:{maxPages:2}},'PAGE_LIMIT'],
    ]){
      const limited=await api.createPdfJob(await file(name),options).result;
      check(limited.status==='failed'&&limited.diagnostics.some(d=>d.code===code),`${code} is enforced by the actual PDF path`);
      if(code==='OUTPUT_LIMIT')check(new TextEncoder().encode(JSON.stringify(limited)).length<=16384,'Failure result withholds overflowing payload and stays inside the output budget');
    }
    const invalid=await api.createPdfJob(new Uint8Array([1,2,3,4,5])).result;check(invalid.status==='failed'&&invalid.diagnostics[0].code==='INVALID_PDF','Malformed PDF is meaningful failure');
    const bounded=await api.createPdfJob(bytes,{limits:{maxBytes:5}}).result;check(bounded.status==='failed'&&bounded.diagnostics[0].code==='INPUT_LIMIT','Input budget is enforced before allocation and model load');
    const fast=await api.createPdfJob(await file('native.pdf'),{engine:'fast-text',ocr:'off'}).result;
    check(fast.status==='partial'&&expected.every(s=>fast.text.includes(s))&&fast.engine.readingOrder==='unverified','PDF Oxide remains callable fast text with unverified order: '+JSON.stringify(fast.diagnostics));
    const emptyJob=api.createPdfJob(await file('native.pdf'),{ocr:'off'});emptyJob.cancel();const immediate=await emptyJob.result;check(immediate.status==='cancelled','Cancellation during initialization settles job');
    return {checks,runs,cancellation:aborted,subsequent:next,selection,discovery:await api.discoverPdf()};
  },{expected,columnCases:columnCases.map(({bytes,...rest})=>rest)});
  const failures=[];
  for(const [asset,code,name] of [['layout_heron_int8.onnx','MODEL_MISSING','native.pdf'],['ocr_det.onnx','MODEL_MISSING','scanned.pdf'],['en_dict.txt','MODEL_INVALID','scanned.pdf']]){
    await page.evaluate(async()=>{(await import('/pdf-api/v1/api.js')).disposePdfRuntime();});
    const pattern='**/models/'+asset;await context.route(pattern,route=>route.fulfill({status:asset==='en_dict.txt'?200:404,body:asset==='en_dict.txt'?'corrupt':'missing'}));
    const failure=await page.evaluate(async name=>{const api=await import('/pdf-api/v1/api.js');return api.createPdfJob(new File([await(await fetch('/fixtures/'+name)).arrayBuffer()],name)).result;},name);
    assert.equal(failure.status,'failed');assert(failure.diagnostics.some(d=>d.code===code),JSON.stringify(failure.diagnostics));failures.push({asset,output:failure});await context.unroute(pattern);
  }
  await page.evaluate(async()=>{(await import('/pdf-api/v1/api.js')).disposePdfRuntime();});
  await context.route('**/pdf-api/v1/inference-worker.js',route=>route.fulfill({contentType:'text/javascript',body:"throw new Error('Injected worker fault');"}));
  const crash=await page.evaluate(async()=>{const api=await import('/pdf-api/v1/api.js');return api.createPdfJob(new File([await(await fetch('/fixtures/native.pdf')).arrayBuffer()],'native.pdf')).result;});
  assert.equal(crash.status,'failed');assert(crash.diagnostics.some(d=>d.code==='RUNTIME_MISSING'));failures.push({asset:'injected worker fault',output:crash});await context.unroute('**/pdf-api/v1/inference-worker.js');
  // Real UI submits the same API and renders the structured result.
  await page.goto(origin+'/pdf-api/v1/index.html');await page.setInputFiles('#file',{name:'native.pdf',mimeType:'application/pdf',buffer:native});await page.selectOption('#ocr','off');await page.click('#run');
  await page.waitForFunction(()=>!document.getElementById('download').disabled,{timeout:120000});
  const ui=await page.evaluate(()=>({status:document.getElementById('status').textContent,text:document.getElementById('text').textContent,result:JSON.parse(document.getElementById('result').textContent)}));
  assert(ui.status.startsWith('partial'));assert(expected.every(s=>ui.text.includes(s)));assert.equal(ui.result.engine.name,'docling.rs-wasm');
  if(process.env.EVIDENCE_PATH)await page.screenshot({path:process.env.EVIDENCE_PATH.replace(/\.json$/,'')+'.png',fullPage:true});
  await page.waitForFunction(async()=>!(await(await import('/pdf-api/v1/api.js')).discoverPdf()).runtime.resident,null,{timeout:35000,polling:1000});
  ui.idleRuntimeReleased=true;
  const semantic=[];
  for(const name of ['semantic-two','semantic-three']){
    const wanted=(await readFile(resolve(web,'scripts/fixtures/docling',name+'.txt'),'utf8')).split(/\s+/).filter(Boolean);
    const run=result.runs.find(r=>r.name===name+'.pdf'),actual=run.output.text.split(/\s+/).filter(Boolean);
    const passed=JSON.stringify(actual)===JSON.stringify(wanted);semantic.push({name,passed,expected:wanted,actual});
  }
  assert.deepEqual(blocked,[],'No external network during browser extraction');
  assert.deepEqual(pageErrors,[],'No unhandled page errors during interruption, stream limits or recovery');
  const evidence={complete:true,browser:browser.version(),memoryProbe:process.env.MEMORY_PROBE==='1',memoryMethod:memory.method,memorySamples:memory.results,pageErrors,...result,failures,ui,semantic,fixtureHashes:Object.fromEntries([...fixtures].map(([n,b])=>[n,createHash('sha256').update(b).digest('hex')])),requests:[...new Set(requested)],externalRequests:blocked};
  if(process.env.EVIDENCE_PATH)await writeFile(process.env.EVIDENCE_PATH,JSON.stringify(evidence,null,2)+'\n');
  console.log(JSON.stringify({checks:result.checks,semantic,provider:result.runs[0].output.engine,externalRequests:blocked},null,2));
  assert(semantic.every(s=>s.passed),'Independent two/three-column order acceptance');
}finally{memory?.stop();await browser?.close();await browserServer?.close();await new Promise(r=>server.close(r));}
