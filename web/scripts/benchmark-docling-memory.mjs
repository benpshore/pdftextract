// Real PDF/inference memory comparison. Each profile gets a fresh owned browser.
// Session-option experiments are injected into the test worker only.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,dirname,extname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {browserMemorySampler} from './browser-process-memory.mjs';
import {workerMemoryProbe} from './worker-memory-probe.mjs';
const {chromium}=await import(process.env.PLAYWRIGHT_MODULE||'playwright');
const {PDFDocument}=await import(process.env.PDF_LIB_MODULE||'pdf-lib');
const web=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const native=await readFile(resolve(web,'../tests/fixtures/pdfium-unicode/native.pdf'));
assert.equal(createHash('sha256').update(native).digest('hex'),'099a620bd29179e329704c152808ad8e3e34f0d5388894c43d17fb3340d373c8');
const expected='Faithful native text remains available. Existing OCR already reads this sentence. Numbers 12345 and alpha beta gamma.';
const profiles={default:{},noPrepack:{extra:{session:{disable_prepacking:'1'}}},noPrepackKeepRenderer:{extra:{session:{disable_prepacking:'1'}}}};
const chosen=(process.env.MEMORY_PROFILES||'default,noPrepack').split(',');
for(const name of chosen)assert(Object.hasOwn(profiles,name),'Unknown memory profile');
const fixtures=new Map([['native.pdf',native]]),results=[];
const server=createServer(async(req,res)=>{
  try{
    const path=new URL(req.url,'http://localhost').pathname;
    if(path==='/'){res.setHeader('Content-Type','text/html');res.end('<!doctype html><title>PDF memory benchmark</title>');return;}
    const fixture=fixtures.get(path.slice(1));const data=fixture||await readFile(resolve(web,'public','.'+path));
    res.setHeader('Content-Type',({'.js':'text/javascript','.mjs':'text/javascript','.wasm':'application/wasm'}[extname(path)]||'application/octet-stream'));res.setHeader('Content-Length',data.length);res.end(data);
  }catch{res.statusCode=404;res.end();}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));const origin=`http://127.0.0.1:${server.address().port}`;
try{
  for(const profile of chosen){
    const launch=await chromium.launchServer({executablePath:process.env.CHROMIUM_PATH,args:['--no-sandbox']}),browser=await chromium.connect(launch.wsEndpoint());
    const memory=browserMemorySampler(launch.process().pid),errors=[];
    try{
      const context=await browser.newContext();
      await context.route('**/*',route=>route.request().url().startsWith(origin+'/')?route.continue():route.abort());
      if(profile==='noPrepackKeepRenderer')await context.route('**/pdf-api/v1/api.js',async route=>{
        const source=await readFile(resolve(web,'public/pdf-api/v1/api.js'),'utf8'),needle='if(pageNo===indices.at(-1))';
        assert(source.includes(needle),'Pinned renderer-retention experiment changed');
        await route.fulfill({contentType:'text/javascript',body:source.replace(needle,'if(false)')});
      });
      await context.route('**/pdf-api/v1/inference-worker.js',async route=>{
        let source=await readFile(resolve(web,'public/pdf-api/v1/inference-worker.js'),'utf8');
        const needle=/^const SESSION_OPTIONS = .+;$/m;
        assert(needle.test(source),'Pinned benchmark injection point changed');
        source=source.replace(needle,'const SESSION_OPTIONS = '+JSON.stringify({executionProviders:['wasm'],logSeverityLevel:3,...profiles[profile]})+';');
        await route.fulfill({contentType:'text/javascript',body:`(${workerMemoryProbe.toString()})();\n${source}`});
      });
      const page=await context.newPage();page.on('pageerror',e=>errors.push(e.message));
      await page.exposeFunction('beginMemory',label=>memory.begin(label));await page.exposeFunction('endMemory',()=>memory.end());await page.exposeFunction('markMemory',e=>memory.mark(e));
      await page.goto(origin);
      const jpeg=await page.evaluate(async()=>{
        const pdfjs=await import('/vendor/docling/1.104.2/pdfjs/pdf.mjs');pdfjs.GlobalWorkerOptions.workerSrc='/vendor/docling/1.104.2/pdfjs/pdf.worker.mjs';
        const pdf=await pdfjs.getDocument({data:await(await fetch('/native.pdf')).arrayBuffer()}).promise,p=await pdf.getPage(1),viewport=p.getViewport({scale:2}),canvas=document.createElement('canvas');canvas.width=viewport.width;canvas.height=viewport.height;
        await p.render({canvas,canvasContext:canvas.getContext('2d'),viewport}).promise;const jpeg=canvas.toDataURL('image/jpeg',1).split(',')[1];canvas.width=canvas.height=0;await pdf.destroy();return jpeg;
      });
      const scan=await PDFDocument.create();scan.addPage([612,792]).drawImage(await scan.embedJpg(Buffer.from(jpeg,'base64')),{x:0,y:0,width:612,height:792});fixtures.set('scanned.pdf',Buffer.from(await scan.save()));
      const mixed=await PDFDocument.create(),mixedPage=mixed.addPage([612,1584]);mixedPage.drawPage((await mixed.embedPdf(native))[0],{x:0,y:792,width:612,height:792});mixedPage.drawImage(await mixed.embedJpg(Buffer.from(jpeg,'base64')),{x:0,y:0,width:612,height:792});fixtures.set('mixed.pdf',Buffer.from(await mixed.save()));
      const runs=await page.evaluate(async expected=>{
        const api=await import('/pdf-api/v1/api.js'),runs=[];
        for(const [name,ocr] of [['native.pdf','off'],['native.pdf','off'],['scanned.pdf','auto'],['mixed.pdf','auto']]){
          const input=new File([await(await fetch('/'+name)).arrayBuffer()],name),progress=[];
          await globalThis.beginMemory(name);const started=performance.now();
          const output=await api.createPdfJob(input,{ocr,onProgress:e=>{progress.push(e);void globalThis.markMemory(e);}}).result;
          const sample=await globalThis.endMemory();
          const wanted=name==='mixed.pdf'?expected+' '+expected:expected;
          if(output.status!=='partial'||output.text.replace(/\s+/g,' ').trim()!==wanted)throw new Error(JSON.stringify({name,output}));
          runs.push({name,milliseconds:performance.now()-started,output,progress,memory:sample});
        }
        await globalThis.beginMemory('explicit-release');api.disposePdfRuntime();await new Promise(r=>setTimeout(r,1500));runs.push({name:'explicit-release',memory:await globalThis.endMemory()});
        return runs;
      },expected);
      assert.deepEqual(errors,[]);results.push({profile,sessionOptions:profiles[profile],rendererRetainedUntilJobEnd:profile==='noPrepackKeepRenderer',browser:browser.version(),method:memory.method,runs});
      if(process.env.EVIDENCE_PATH)await writeFile(process.env.EVIDENCE_PATH,JSON.stringify({results},null,2)+'\n');
      console.log(JSON.stringify({profile,runs:runs.map(r=>({name:r.name,milliseconds:Math.round(r.milliseconds||0),peakRssMiB:r.memory.peakRssBytes/1048576,peakPssMiB:r.memory.peakPssBytes/1048576,endPssMiB:r.memory.endPssBytes/1048576,wasm:r.output?.pages[0].resources.testOnlyWasmMemory}))}));
    }finally{memory.stop();await browser.close();await launch.close();}
  }
}finally{await new Promise(r=>server.close(r));}
