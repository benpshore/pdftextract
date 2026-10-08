import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,dirname,extname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
const {chromium}=await import(process.env.PLAYWRIGHT_MODULE||'playwright');
const {PDFDocument}=await import(process.env.PDF_LIB_MODULE||'pdf-lib');
const web=resolve(dirname(fileURLToPath(import.meta.url)),'..'),repo=resolve(web,'..');
const native=await readFile(resolve(repo,'tests/fixtures/pdfium-unicode/native.pdf'));
assert.equal(createHash('sha256').update(native).digest('hex'),'099a620bd29179e329704c152808ad8e3e34f0d5388894c43d17fb3340d373c8');
const expected=['Faithful native text remains available.','Existing OCR already reads this sentence.','Numbers 12345 and alpha beta gamma.'];
const fixtures=new Map([['native.pdf',native]]);
for(const name of ['semantic-two','semantic-three'])fixtures.set(name+'.pdf',await readFile(resolve(web,'scripts/fixtures/docling',name+'.pdf')));
const requested=[],blocked=[];
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
let browser;
try{
  browser=await chromium.launch({executablePath:process.env.CHROMIUM_PATH,args:['--no-sandbox']});
  const context=await browser.newContext();await context.route('**/*',async route=>{
    const url=route.request().url();if(url.startsWith(origin+'/')){requested.push(url.slice(origin.length));await route.continue();}else{blocked.push(url);await route.abort();}
  });
  const page=await context.newPage();page.on('console',msg=>{if(msg.text().startsWith('ACTUAL PDF RESULT'))console.log(msg.text());else if(msg.type()==='error')console.error('Browser:',msg.text());});page.on('pageerror',e=>console.error(e));await page.goto(origin);
  // Derive scan pixels from the pre-existing verified public-safe control PDF.
  const jpeg=await page.evaluate(async()=>{
    const pdfjs=await import('/vendor/docling/1.104.2/pdfjs/pdf.mjs');pdfjs.GlobalWorkerOptions.workerSrc='/vendor/docling/1.104.2/pdfjs/pdf.worker.mjs';
    const pdf=await pdfjs.getDocument({data:await(await fetch('/fixtures/native.pdf')).arrayBuffer()}).promise;
    const p=await pdf.getPage(1),viewport=p.getViewport({scale:2}),canvas=document.createElement('canvas');canvas.width=viewport.width;canvas.height=viewport.height;
    await p.render({canvas,canvasContext:canvas.getContext('2d'),viewport}).promise;const value=canvas.toDataURL('image/jpeg',1).split(',')[1];await pdf.destroy();return value;
  });
  const scan=await PDFDocument.create();const image=await scan.embedJpg(Buffer.from(jpeg,'base64'));scan.addPage([612,792]).drawImage(image,{x:0,y:0,width:612,height:792});fixtures.set('scanned.pdf',Buffer.from(await scan.save()));
  const mixed=await PDFDocument.load(native);mixed.addPage((await mixed.copyPages(scan,[0]))[0]);fixtures.set('mixed.pdf',Buffer.from(await mixed.save()));
  const result=await page.evaluate(async expected=>{
    const api=await import('/pdf-api/v1/api.js'),checks=[],runs=[];
    const check=(condition,message)=>{if(!condition)throw new Error(message);checks.push(message);};
    const file=async name=>new File([await(await fetch('/fixtures/'+name)).arrayBuffer()],name,{type:'application/pdf'});
    const discovery=await api.discoverPdf();check(discovery.engine.version==='1.104.2'&&discovery.capabilities.provider.configured==='wasm','Identifiable discovery declares CPU/WASM and schema');
    for(const [name,options] of [['native.pdf',{ocr:'off'}],['scanned.pdf',{}],['mixed.pdf',{}],['semantic-two.pdf',{ocr:'off'}],['semantic-three.pdf',{ocr:'off'}]]){
      const input=await file(name),before=await input.arrayBuffer(),progress=[];
      const job=api.createPdfJob(input,{...options,onProgress:event=>progress.push(event)});
      check(api.getPdfJob(job.id)===job,`${name}: callable job status`);
      const output=await job.result;runs.push({name,output,progress});
      console.log('ACTUAL PDF RESULT',name,JSON.stringify({status:output.status,text:output.text,diagnostics:output.diagnostics}));
      check(output.status==='partial',`${name}: real Docling PDF inference returns explicit partial coverage`);
      check(output.engine.provider==='wasm'&&output.pages.length>0,`${name}: CPU provenance and page output`);
      check(progress.some(p=>p.phase==='inference')&&progress.some(p=>p.phase==='page-complete'),`${name}: real inference progress`);
      check((await input.arrayBuffer()).byteLength===before.byteLength&&new Uint8Array(await input.arrayBuffer()).every((b,i)=>b===new Uint8Array(before)[i]),`${name}: original unchanged`);
      if(name==='native.pdf'||name==='scanned.pdf'||name==='mixed.pdf')for(const p of output.pages)for(const sentence of expected)check(p.text.includes(sentence),`${name}: page ${p.page} expected sentence: ${sentence}`);
      if(name==='scanned.pdf')check(output.pages[0].route==='scanned'&&output.pages[0].ocr==='page','Scan runs ScannedConverter with detector');
      if(name==='mixed.pdf')check(output.pages.map(p=>`${p.page}:${p.route}`).join(',')==='1:digital,2:scanned','Mixed PDF preserves digital/scanned routes and original page identity');
    }
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
    const invalid=await api.createPdfJob(new Uint8Array([1,2,3,4,5])).result;check(invalid.status==='failed'&&invalid.diagnostics[0].code==='INVALID_PDF','Malformed PDF is meaningful failure');
    const bounded=await api.createPdfJob(bytes,{limits:{maxBytes:5}}).result;check(bounded.status==='failed'&&bounded.diagnostics[0].code==='INPUT_LIMIT','Input budget is enforced before allocation and model load');
    const fast=await api.createPdfJob(await file('native.pdf'),{engine:'fast-text',ocr:'off'}).result;
    check(fast.status==='partial'&&expected.every(s=>fast.text.includes(s))&&fast.engine.readingOrder==='unverified','PDF Oxide remains callable fast text with unverified order: '+JSON.stringify(fast.diagnostics));
    const emptyJob=api.createPdfJob(await file('native.pdf'),{ocr:'off'});emptyJob.cancel();const immediate=await emptyJob.result;check(immediate.status==='cancelled','Cancellation during initialization settles job');
    return {checks,runs,cancellation:aborted,subsequent:next,selection,discovery:await api.discoverPdf()};
  },expected);
  const failures=[];
  for(const [asset,code,name] of [['layout_heron_int8.onnx','MODEL_MISSING','native.pdf'],['ocr_det.onnx','MODEL_MISSING','scanned.pdf'],['en_dict.txt','MODEL_INVALID','scanned.pdf']]){
    const pattern='**/models/'+asset;await context.route(pattern,route=>route.fulfill({status:asset==='en_dict.txt'?200:404,body:asset==='en_dict.txt'?'corrupt':'missing'}));
    const failure=await page.evaluate(async name=>{const api=await import('/pdf-api/v1/api.js');return api.createPdfJob(new File([await(await fetch('/fixtures/'+name)).arrayBuffer()],name)).result;},name);
    assert.equal(failure.status,'failed');assert(failure.diagnostics.some(d=>d.code===code),JSON.stringify(failure.diagnostics));failures.push({asset,output:failure});await context.unroute(pattern);
  }
  // Real UI submits the same API and renders the structured result.
  await page.goto(origin+'/pdf-api/v1/index.html');await page.setInputFiles('#file',{name:'native.pdf',mimeType:'application/pdf',buffer:native});await page.selectOption('#ocr','off');await page.click('#run');
  await page.waitForFunction(()=>!document.getElementById('download').disabled,{timeout:120000});
  const ui=await page.evaluate(()=>({status:document.getElementById('status').textContent,text:document.getElementById('text').textContent,result:JSON.parse(document.getElementById('result').textContent)}));
  assert(ui.status.startsWith('partial'));assert(expected.every(s=>ui.text.includes(s)));assert.equal(ui.result.engine.name,'docling.rs-wasm');
  const semantic=[];
  for(const name of ['semantic-two','semantic-three']){
    const wanted=(await readFile(resolve(web,'scripts/fixtures/docling',name+'.txt'),'utf8')).split(/\s+/).filter(Boolean);
    const run=result.runs.find(r=>r.name===name+'.pdf'),actual=run.output.text.split(/\s+/).filter(Boolean);
    const passed=JSON.stringify(actual)===JSON.stringify(wanted);semantic.push({name,passed,expected:wanted,actual});
  }
  assert.deepEqual(blocked,[],'No external network during browser extraction');
  const evidence={browser:browser.version(),...result,failures,ui,semantic,fixtureHashes:Object.fromEntries([...fixtures].map(([n,b])=>[n,createHash('sha256').update(b).digest('hex')])),requests:[...new Set(requested)],externalRequests:blocked};
  if(process.env.EVIDENCE_PATH)await writeFile(process.env.EVIDENCE_PATH,JSON.stringify(evidence,null,2)+'\n');
  console.log(JSON.stringify({checks:result.checks,semantic,provider:result.runs[0].output.engine,externalRequests:blocked},null,2));
  assert(semantic.every(s=>s.passed),'Independent two/three-column order acceptance');
}finally{await browser?.close();await new Promise(r=>server.close(r));}
