// Actual API lifecycle/length evidence. Optional exposed GC is diagnostic only:
// production and timed extraction runs never call it.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,dirname,extname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {browserMemorySampler} from './browser-process-memory.mjs';
import {workerMemoryProbe} from './worker-memory-probe.mjs';
const playwright=await import(process.env.PLAYWRIGHT_MODULE||'playwright');
const {PDFDocument}=await import(process.env.PDF_LIB_MODULE||'pdf-lib');
const web=resolve(dirname(fileURLToPath(import.meta.url)),'..'),kind=process.env.BROWSER||'chromium';
assert(['chromium','webkit'].includes(kind));
const counts=(process.env.LIFECYCLE_COUNTS??'1,8,12').split(',').filter(Boolean).map(Number),cycles=Number(process.env.LIFECYCLE_CYCLES??3);
assert(counts.every(n=>[1,8,12,16].includes(n))&&Number.isInteger(cycles)&&cycles>=0&&cycles<=3);
const native=await readFile(resolve(web,'../tests/fixtures/pdfium-unicode/native.pdf'));
assert.equal(createHash('sha256').update(native).digest('hex'),'099a620bd29179e329704c152808ad8e3e34f0d5388894c43d17fb3340d373c8');
const sentence='Faithful native text remains available. Existing OCR already reads this sentence. Numbers 12345 and alpha beta gamma.';
const fixtures=new Map([['native.pdf',native]]),checks=[],runs=[],errors=[],external=[];
const evidence={complete:false,browserKind:kind,checks,runs,errors,external,forcedGc:'Only separately labelled diagnostic checkpoints; never during an extraction.',fixtureDescription:'Five copies of the verified native text strip per page, rasterized into separate JPEG images with alternating x offsets. Independent expected text is fixed before extraction. Synthetic scan length/ownership control, not real-world OCR qualification.'};
const save=async()=>{if(process.env.EVIDENCE_PATH)await writeFile(process.env.EVIDENCE_PATH,JSON.stringify(evidence,null,2)+'\n');};
const check=(value,message)=>{assert(value,message);checks.push(message);console.log('LIFECYCLE CHECK',message);};
const server=createServer(async(req,res)=>{
  try{
    const path=new URL(req.url,'http://localhost').pathname;
    if(path==='/'){res.setHeader('Content-Type','text/html');res.end('<!doctype html><title>PDF lifecycle evidence</title>');return;}
    const file=resolve(web,'public','.'+path);if(!file.startsWith(resolve(web,'public')+'/'))throw Error('path');
    const data=fixtures.get(path.slice(1))||await readFile(file);
    res.setHeader('Content-Type',({'.js':'text/javascript','.mjs':'text/javascript','.wasm':'application/wasm'}[extname(path)]||'application/octet-stream'));res.setHeader('Content-Length',data.length);res.end(req.method==='HEAD'?undefined:data);
  }catch{res.statusCode=404;res.end();}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));const origin=`http://127.0.0.1:${server.address().port}`;
let launch,browser,memory;
try{
  launch=await playwright[kind].launchServer(kind==='chromium'?{executablePath:process.env.CHROMIUM_PATH,args:['--no-sandbox','--js-flags=--expose-gc']}:{});
  browser=await playwright[kind].connect(launch.wsEndpoint());memory=browserMemorySampler(launch.process().pid);
  evidence.browserVersion=browser.version();evidence.processMemoryMethod=memory.method;
  const context=await browser.newContext();
  await context.route('**/*',route=>{if(route.request().url().startsWith(origin+'/'))return route.continue();external.push(route.request().url());return route.abort();});
  await context.route('**/pdf-api/v1/inference-worker.js',async route=>{
    let source=await readFile(resolve(web,'public/pdf-api/v1/inference-worker.js'),'utf8');
    const needle="  if (!documentOpen) throw fail('RUNTIME_STATE', 'No PDF document is open.');";
    assert(source.includes(needle));
    source=source.replace(needle,"  globalThis.__pageBuffers.push({ref:new WeakRef(rgba),bytes:rgba.byteLength,index});\n"+needle);
    await route.fulfill({contentType:'text/javascript',body:`(${workerMemoryProbe.toString()})();\nglobalThis.__pageBuffers=[];\n${source}`});
  });
  const page=await context.newPage();page.on('pageerror',e=>errors.push(e.message));await page.goto(origin);
  await page.exposeFunction('markLifecycleMemory',event=>memory.mark(event));
  await page.exposeFunction('recordLifecyclePage',sample=>{runs.at(-1)?.pageCompletions.push({...sample,nearestProcessSample:memory.checkpoint()});});
  // Crop the three lines from actual PDF.js text bounds, then repeat those pixels
  // at readable original scale. All 16 images differ, defeating same-image reuse.
  const jpegs=await page.evaluate(async()=>{
    const pdfjs=await import('/vendor/docling/1.104.2/pdfjs/pdf.mjs');pdfjs.GlobalWorkerOptions.workerSrc='/vendor/docling/1.104.2/pdfjs/pdf.worker.mjs';
    const pdf=await pdfjs.getDocument({data:await(await fetch('/native.pdf')).arrayBuffer()}).promise,p=await pdf.getPage(1),v=p.getViewport({scale:2});
    const canvas=document.createElement('canvas');canvas.width=v.width;canvas.height=v.height;await p.render({canvas,canvasContext:canvas.getContext('2d'),viewport:v}).promise;
    const text=await p.getTextContent(),ys=text.items.filter(t=>t.str).map(t=>v.height-t.transform[5]*2);
    const top=Math.max(0,Math.floor(Math.min(...ys)-45)),bottom=Math.min(v.height,Math.ceil(Math.max(...ys)+15));
    const result=[];
    for(let i=0;i<16;i++){
      const out=document.createElement('canvas');out.width=v.width;out.height=v.height;const c=out.getContext('2d');c.fillStyle='white';c.fillRect(0,0,out.width,out.height);
      for(let row=0;row<5;row++)c.drawImage(canvas,0,top,canvas.width,bottom-top,(i%2?8:0),50+row*280,canvas.width,bottom-top);
      // Distinct, non-text corner mark does not affect the independent oracle.
      c.fillStyle='#777';c.fillRect(10+i*3,out.height-20,2,2);
      result.push(out.toDataURL('image/jpeg',0.95).split(',')[1]);out.width=out.height=0;
    }
    canvas.width=canvas.height=0;await pdf.destroy();return result;
  });
  for(const count of [1,8,12,16]){
    const pdf=await PDFDocument.create();for(let i=0;i<count;i++)pdf.addPage([612,792]).drawImage(await pdf.embedJpg(Buffer.from(jpegs[i],'base64')),{x:0,y:0,width:612,height:792});fixtures.set(`scan-${count}.pdf`,Buffer.from(await pdf.save()));
  }
  evidence.fixtures=Object.fromEntries([...fixtures].map(([name,b])=>[name,{bytes:b.length,sha256:createHash('sha256').update(b).digest('hex')}]));
  const expected=Array(5).fill(sentence).join(' ');
  async function run(label,name,count,{cancelPage}={}){
    const record={label,name,pageCompletions:[]};runs.push(record);await memory.begin(label);
    const result=await page.evaluate(async({name,cancelPage,expected})=>{
      const api=await import('/pdf-api/v1/api.js'),input=new File([await(await fetch('/'+name)).arrayBuffer()],name),events=[];
      let currentPage=0;const started=performance.now();
      const job=api.createPdfJob(input,{onProgress:e=>{void globalThis.markLifecycleMemory(e);if(e.page)currentPage=e.page;if(e.phase==='page-complete')void globalThis.recordLifecyclePage({page:e.page,elapsedMs:performance.now()-started});if(e.testOnlyWasmMemory)events.push({phase:e.phase,model:e.model,page:currentPage,memory:e.testOnlyWasmMemory});if(cancelPage&&currentPage===cancelPage&&e.phase==='model-inference')api.cancelPdfJob(e.jobId);}});
      const output=await job.result;
      return {status:output.status,jobId:job.id,diagnostics:output.diagnostics,resources:output.resources,serializedResultBytes:new TextEncoder().encode(JSON.stringify(output)).length,milliseconds:performance.now()-started,
        pages:output.pages.map(p=>({page:p.page,route:p.route,ocr:p.ocr,text:p.text,exact:p.text.replace(/\s+/g,' ').trim()===expected,order:p.order,layout:p.layout,diagnostics:p.diagnostics,execution:p.execution,resources:p.resources})),events,runtime:(await api.discoverPdf()).runtime};
    },{name,cancelPage,expected});
    record.output=result;record.memory=await memory.end();await save();
    console.log(JSON.stringify({label,status:result.status,pages:result.pages.length,ms:Math.round(result.milliseconds),peakRssMiB:record.memory.peakRssBytes/1048576,endPssMiB:record.memory.endPssBytes/1048576,rustMiB:result.pages.at(-1)?.resources.rustWasmCapacityBytes/1048576}));
    check(result.status===(cancelPage?'cancelled':'partial'),`${label}: expected terminal status`);
    check(result.pages.length===(cancelPage?cancelPage-1:count),`${label}: original completed-page count`);
    check(result.pages.every((p,i)=>p.page===i+1&&p.ocr==='page'&&p.diagnostics.some(d=>d.code==='COVERAGE_UNVERIFIED')),`${label}: page identity, OCR provenance and incomplete coverage`);
    // This control exposed an upstream OCR duplication on the initial block.
    // Keep the independent expectation and failures visible while measuring
    // length/ownership; this is not an assertion that OCR accuracy passed.
    record.textOracle={expectedPerPage:expected,allPagesExact:result.pages.every(p=>p.exact),mismatchedPages:result.pages.filter(p=>!p.exact).map(p=>p.page)};
    console.log('DENSE SCAN ORACLE',JSON.stringify({label,...record.textOracle}));
    if(cancelPage)check(!result.runtime.resident,`${label}: interrupted model worker terminated`);
    return result;
  }
  async function diagnostic(label,dispose=false){
    const record={label,diagnosticOnly:true};runs.push(record);await memory.begin(label);
    if(dispose)await page.evaluate(async()=>{(await import('/pdf-api/v1/api.js')).disposePdfRuntime();});
    await page.waitForTimeout(1500);
    record.beforeCollection=await memory.end();
    record.workersBeforeCollection=[];
    for(const worker of page.workers())try{record.workersBeforeCollection.push(await worker.evaluate(()=>({url:self.location.pathname,livePageBuffers:(globalThis.__pageBuffers||[]).filter(x=>x.ref.deref()).map(({ref,...rest})=>rest)})));}catch{/* disposal raced worker notification */}
    await memory.begin(label+':diagnostic-gc');
    record.workers=[];
    for(const worker of page.workers())try{record.workers.push(await worker.evaluate(()=>{const didCollect=typeof globalThis.gc==='function';if(didCollect)globalThis.gc();const refs=globalThis.__pageBuffers||[];return {url:self.location.pathname,didCollect,observedPageBuffers:refs.length,livePageBuffers:refs.filter(x=>x.ref.deref()).map(({ref,...rest})=>rest)};}));}catch(error){record.workers.push({terminated:true,message:error.message.slice(0,160)});}
    record.main=await page.evaluate(async()=>{const didCollect=typeof globalThis.gc==='function';if(didCollect)globalThis.gc();return {didCollect,observerPayloadRetained:!!globalThis.__observerWeak?.deref(),lateSubscriberPayloadRetained:!!globalThis.__lateObserverWeak?.deref(),runtime:(await(await import('/pdf-api/v1/api.js')).discoverPdf()).runtime};});
    await page.waitForTimeout(500);record.afterCollection=await memory.end();await save();console.log(JSON.stringify(record));return record;
  }
  for(const count of counts){await run(`length-${count}`,`scan-${count}.pdf`,count);await diagnostic(`length-${count}:settled`);}
  for(let i=0;i<cycles;i++){
    await run(`cycle-${i}:extract`,'scan-1.pdf',1);
    await run(`cycle-${i}:cancel-page-three`,'scan-8.pdf',8,{cancelPage:3});
    await run(`cycle-${i}:retry`,'scan-1.pdf',1);
    const released=await diagnostic(`cycle-${i}:dispose`,true);check(!released.main.runtime.resident,`cycle-${i}: explicit dispose removes model owner`);
  }
  // Probe a progress closure's reachability independently of RSS/capacity.
  await page.evaluate(async()=>{
    const api=await import('/pdf-api/v1/api.js');
    async function withObserver(){const payload={buffer:new ArrayBuffer(4*1024*1024)};globalThis.__observerWeak=new WeakRef(payload);const input=new File([await(await fetch('/native.pdf')).arrayBuffer()],'native.pdf');const job=api.createPdfJob(input,{ocr:'off',onProgress:()=>payload.buffer.byteLength});await job.result;(function lateSubscriber(){const latePayload={buffer:new ArrayBuffer(4*1024*1024)};globalThis.__lateObserverWeak=new WeakRef(latePayload);job.subscribe(()=>latePayload.buffer.byteLength);})();}
    await withObserver();
  });
  const observer=await diagnostic('completed-observer-probe',true);
  evidence.observerProbe=observer.main.observerPayloadRetained;
  if(observer.main.didCollect)check(!evidence.observerProbe,'Completed options do not retain the progress closure payload after diagnostic GC');
  if(observer.main.didCollect)check(!observer.main.lateSubscriberPayloadRetained,'Completed jobs do not retain late subscriber payloads after diagnostic GC');
  const ids=runs.filter(r=>r.output).map(r=>r.output.jobId);
  evidence.retainedJobHandles=await page.evaluate(async ids=>{const api=await import('/pdf-api/v1/api.js');return ids.filter(id=>!!api.getPdfJob(id)).length;},ids);
  check(evidence.retainedJobHandles<=8,'Completed job lookup remains bounded to eight handles');
  check(errors.length===0,'No unhandled browser errors');check(external.length===0,'No external job requests');
  evidence.complete=true;await save();console.log(JSON.stringify({complete:true,browser:kind,checks:checks.length,observerPayloadRetained:evidence.observerProbe}));
}catch(error){evidence.failure={message:error.message,stack:error.stack};await save();throw error;}
finally{memory?.stop();await browser?.close();await launch?.close();await new Promise(r=>server.close(r));}
