// Timing evidence for one original five-page scholarly PDF; not an accuracy
// benchmark or a timing extrapolation from synthetic pages. No model downloads.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,dirname,extname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {cpus,platform,release} from 'node:os';
const {chromium}=await import(process.env.PLAYWRIGHT_MODULE||'playwright');
const {PDFDocument}=await import(process.env.PDF_LIB_MODULE||'pdf-lib');
const web=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const url='https://aclanthology.org/E17-2068.pdf';
const bytes=process.env.SCHOLARLY_PDF?await readFile(process.env.SCHOLARLY_PDF):await(async()=>{const r=await fetch(url);assert(r.ok,`Scholarly source fetch: HTTP ${r.status}`);return Buffer.from(await r.arrayBuffer());})();
assert.equal((await PDFDocument.load(bytes)).getPageCount(),5,'Original source must contain exactly five pages');
const evidence={complete:false,title:'Bag of Tricks for Efficient Text Classification',authors:'Armand Joulin, Edouard Grave, Piotr Bojanowski, Tomas Mikolov',publication:'EACL 2017, pages 427–431',sourceUrl:url,bytes:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex'),documentPages:5,environment:{platform:platform(),release:release(),cpuModel:cpus()[0]?.model,logicalCpus:cpus().length,node:process.version},config:{engine:'docling',ocr:'auto',pages:'all five original pages',provider:'CPU/WASM',threads:1,modelAssets:'already staged on local fixture server; internet provisioning excluded',timing:'performance.now immediately before createPdfJob through job.result settlement; input fetch and browser startup excluded',cold:'fresh browser/context, no loaded runtime or ONNX session; includes local asset fetch, WASM initialization, model session creation and extraction',warm:'immediately repeated PDF with same worker/model sessions; reparses/renders original PDF; no result cache'},runs:[],externalRequests:[],pageErrors:[]};
const save=async()=>{if(process.env.EVIDENCE_PATH)await writeFile(process.env.EVIDENCE_PATH,JSON.stringify(evidence,null,2)+'\n');};
const server=createServer(async(req,res)=>{try{const path=new URL(req.url,'http://localhost').pathname;if(path==='/'){res.setHeader('Content-Type','text/html');res.end('<!doctype html><title>Scholarly PDF benchmark</title>');return;}const file=resolve(web,'public','.'+path);if(!file.startsWith(resolve(web,'public')+'/'))throw Error('path');const data=path==='/paper.pdf'?bytes:await readFile(file);res.setHeader('Content-Type',({'.js':'text/javascript','.mjs':'text/javascript','.wasm':'application/wasm'}[extname(path)]||'application/octet-stream'));res.setHeader('Content-Length',data.length);res.end(req.method==='HEAD'?undefined:data);}catch{res.statusCode=404;res.end();}});
await new Promise(r=>server.listen(0,'127.0.0.1',r));const origin=`http://127.0.0.1:${server.address().port}`;let browser;
try{
 browser=await chromium.launch({executablePath:process.env.CHROMIUM_PATH,args:['--no-sandbox']});evidence.environment.browser=browser.version();
 const context=await browser.newContext();await context.route('**/*',route=>{if(route.request().url().startsWith(origin+'/'))return route.continue();evidence.externalRequests.push(route.request().url());return route.abort();});
 const page=await context.newPage();page.on('pageerror',e=>evidence.pageErrors.push(e.message));await page.goto(origin);
 for(const mode of ['cold','warm']){
   const result=await page.evaluate(async mode=>{
     const api=await import('/pdf-api/v1/api.js'),file=new File([await(await fetch('/paper.pdf')).arrayBuffer()],'E17-2068.pdf'),progress=[];
     const started=performance.now(),job=api.createPdfJob(file,{ocr:'auto',onProgress:e=>progress.push({elapsedMs:performance.now()-started,...e})}),output=await job.result;
     return {mode,milliseconds:performance.now()-started,output,progress};
   },mode);evidence.runs.push(result);await save();
   console.log('SCHOLARLY PDF TIMING',JSON.stringify({mode,milliseconds:result.milliseconds,status:result.output.status,pages:result.output.pages.length,diagnostics:result.output.diagnostics,routes:result.output.pages.map(p=>({page:p.page,route:p.route,ocr:p.ocr,textCharacters:p.text.length,orderEngine:p.orderEngine})),modelAssets:result.progress.filter(e=>e.phase==='assets').map(e=>e.model)}));
   assert.equal(result.output.source.pageCount,5);assert.equal(result.output.pages.length,5);assert.equal(result.output.status,'partial');assert(result.output.text.includes('Bag of Tricks'));
 }
 assert.equal(evidence.runs[0].output.resources.runtimeId,evidence.runs[1].output.resources.runtimeId);assert(!evidence.runs[1].progress.some(e=>e.phase==='assets'));assert.deepEqual(evidence.externalRequests,[]);assert.deepEqual(evidence.pageErrors,[]);
 evidence.complete=true;await save();
}catch(error){evidence.failure={message:error.message};await save();throw error;}
finally{await browser?.close();await new Promise(r=>server.close(r));}
