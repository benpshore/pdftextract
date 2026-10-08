import {VERSION,COMMIT,ROOT,MODELS,LIMITS} from './manifest.js';
import {orderTextCells,textCells} from './geometry-order.js';
export const API_VERSION='1';
export const schemas=Object.freeze({
  input:{$schema:'https://json-schema.org/draft/2020-12/schema',type:'object',properties:{engine:{enum:['docling','fast-text']},ocr:{enum:['auto','always','off']},pages:{type:'array',uniqueItems:true,minItems:1,maxItems:20,items:{type:'integer',minimum:1}},limits:{type:'object',additionalProperties:false,properties:Object.fromEntries(Object.keys(LIMITS).map(key=>[key,{type:'integer',minimum:1,maximum:LIMITS[key]}]))}},additionalProperties:false},
  result:{$schema:'https://json-schema.org/draft/2020-12/schema',type:'object',required:['apiVersion','jobId','status','source','engine','pages','text','diagnostics'],properties:{apiVersion:{const:'1'},jobId:{type:'string'},status:{enum:['partial','failed','cancelled']},source:{type:['object','null']},engine:{type:'object'},pages:{type:'array',items:{type:'object',required:['page','text','order','layout','diagnostics']}},text:{type:'string'},diagnostics:{type:'array',items:{type:'object',required:['code','message']}}}},
});
const jobs=new Map();let active,lastExecution=null;
const problem=(code,message)=>Object.assign(new Error(message),{code});
const sha=async bytes=>[...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(n=>n.toString(16).padStart(2,'0')).join('');
export async function discoverPdf() {
  const paths=['docling_wasm.js','docling_wasm_bg.wasm','pdfjs/pdf.mjs','pdfjs/pdf.worker.mjs','ort/ort.wasm.min.mjs','ort/ort-wasm-simd-threaded.mjs','ort/ort-wasm-simd-threaded.wasm',...Object.values(MODELS).map(m=>'models/'+m.file)];
  const assets=await Promise.all(paths.map(async path=>{try{const r=await fetch(ROOT+path,{method:'HEAD'});return {path:ROOT+path,available:r.ok,bytes:Number(r.headers.get('Content-Length'))||null};}catch{return {path:ROOT+path,available:false};}}));
  return {apiVersion:API_VERSION,engine:{name:'docling.rs-wasm',version:VERSION,commit:COMMIT},interface:'browser JavaScript ES module',operations:['discoverPdf','createPdfJob','getPdfJob','cancelPdfJob'],schemas,limits:LIMITS,
    capabilities:{embeddedText:true,fastText:{engine:'pdf-oxide-wasm 0.3.77',readingOrder:'unverified',layout:false,ocr:false},layout:{model:'layout_heron_int8.onnx',available:assets.find(a=>a.path.endsWith(MODELS.layout.file)).available},ocr:{languages:['en'],detectorRequired:true,available:['recognition','detector','dictionary'].every(key=>assets.find(a=>a.path.endsWith(MODELS[key].file)).available)},tables:'geometric; TableFormer unsupported',provider:{configured:'wasm',device:'CPU',threads:1,webgpu:'unsupported',lastExecution},cancellation:'owned inference worker terminated; pdf.js render cancelled',progress:true,http:false,wasi:false},assets};
}
export function getPdfJob(id){return jobs.get(id)||null;}
export function cancelPdfJob(id){const job=jobs.get(id);if(!job)return false;job.cancel();return true;}
function optionsFor(options){
  if(Object.keys(options).some(k=>!['engine','ocr','pages','limits','signal','onProgress'].includes(k)))throw problem('INVALID_OPTIONS','Unknown option.');
  const engine=options.engine??'docling';if(!['docling','fast-text'].includes(engine))throw problem('INVALID_OPTIONS','engine must be docling or fast-text.');
  const ocr=options.ocr??'auto';if(!['auto','always','off'].includes(ocr))throw problem('INVALID_OPTIONS','ocr must be auto, always, or off.');
  const limits={...LIMITS,...options.limits};
  for(const [key,value] of Object.entries(limits))if(!(key in LIMITS)||!Number.isSafeInteger(value)||value<1||value>LIMITS[key])throw problem('INVALID_OPTIONS',`Invalid ${key} limit; limits can only lower the documented ceilings.`);
  if(options.pages&&(!Array.isArray(options.pages)||!options.pages.length||options.pages.length>limits.maxPages||options.pages.some(p=>!Number.isSafeInteger(p)||p<1)||new Set(options.pages).size!==options.pages.length))throw problem('INVALID_OPTIONS','pages must contain distinct positive page numbers within the page budget.');
  if(engine==='fast-text'&&(ocr!=='off'||options.pages))throw problem('UNSUPPORTED_OPTIONS','Fast text supports OCR off and all pages only.');
  return {engine,ocr,limits,pages:options.pages?.slice().sort((a,b)=>a-b)};
}
export function createPdfJob(input,options={}) {
  const config=optionsFor(options);
  if(active)throw problem('BUSY','One PDF job is already running. Cancel or await it before starting another.');
  const controller=new AbortController(),listeners=new Set();if(options.onProgress)listeners.add(options.onProgress);
  const job={id:crypto.randomUUID(),status:'queued',progress:null,result:null,cancel:()=>{if(['queued','running'].includes(job.status))controller.abort();},subscribe:listener=>{listeners.add(listener);return()=>listeners.delete(listener);}};
  jobs.set(job.id,job);if(jobs.size>8)jobs.delete(jobs.keys().next().value);active=job;
  const relay=()=>controller.abort();options.signal?.addEventListener('abort',relay,{once:true});if(options.signal?.aborted)controller.abort();
  const emit=event=>{job.progress={jobId:job.id,...event};for(const listener of listeners){try{listener(job.progress);}catch{/* observers do not control the operation */}}};
  job.result=(config.engine==='fast-text'?runFast:run)(input,config,controller,emit,job).finally(()=>{active=null;options.signal?.removeEventListener('abort',relay);});return job;
}
function ordered(document){
  const blocks=[],refs=new Set();
  const resolve=ref=>{const [collection,index]=ref.replace(/^#\//,'').split('/');return document[collection]?.[Number(index)];};
  const visit=item=>{if(!item)return;if(item.$ref){if(refs.has(item.$ref))return;refs.add(item.$ref);item=resolve(item.$ref);if(!item)return;}let text=item.text;if(item.label==='table')text=(item.data?.table_cells||[]).slice().sort((a,b)=>a.start_row_offset_idx-b.start_row_offset_idx||a.start_col_offset_idx-b.start_col_offset_idx).map(c=>c.text||'').join('\n');if(typeof text==='string'||item.label==='picture')blocks.push({id:item.self_ref,text:text||'',label:item.label,provenance:item.prov||[]});for(const child of item.children||[])visit(child);};
  visit(document.body);
  // Missing body links are explicit coverage uncertainty, never silently dropped.
  const unlinked=(document.texts||[]).filter(t=>!blocks.some(b=>b.id===t.self_ref));
  return {blocks,unlinked};
}
async function run(input,{ocr,pages:selected,limits},controller,emit,job){
  const signal=controller.signal;
  let worker,loading,pdf,render,timeout,rejectRpc;const output={apiVersion:API_VERSION,jobId:job.id,status:'failed',source:null,engine:{name:'docling.rs-wasm',version:VERSION,commit:COMMIT,renderer:'pdfjs-dist 5.4.624',runtime:'onnxruntime-web 1.24.3',provider:'wasm',device:'CPU',threads:1},pages:[],text:'',diagnostics:[]};
  const abort=()=>{render?.cancel();worker?.terminate();void loading?.destroy().catch(()=>{});rejectRpc?.(problem(signal.reason?.code||'CANCELLED','PDF job interrupted.'));};signal.addEventListener('abort',abort,{once:true});
  const wait=promise=>new Promise((resolve,reject)=>{const interrupted=()=>reject(signal.reason||problem('CANCELLED','PDF job interrupted.'));if(signal.aborted){interrupted();return;}signal.addEventListener('abort',interrupted,{once:true});Promise.resolve(promise).then(resolve,reject).finally(()=>signal.removeEventListener('abort',interrupted));});
  try{
    signal.throwIfAborted();job.status='running';timeout=setTimeout(()=>controller.abort(problem('TIMEOUT','PDF exceeded the execution time budget.')),limits.timeoutMs);
    const size=input instanceof Blob?input.size:input?.byteLength;
    if(!Number.isSafeInteger(size)||size<5||size>limits.maxBytes)throw problem('INPUT_LIMIT',`PDF must be between 5 and ${limits.maxBytes} bytes.`);
    if(!(input instanceof Blob||input instanceof ArrayBuffer||input instanceof Uint8Array))throw problem('INVALID_INPUT','Input must be a File, Blob, Uint8Array, or ArrayBuffer.');
    const bytes=input instanceof Blob?new Uint8Array(await wait(input.arrayBuffer())):new Uint8Array(input instanceof ArrayBuffer?input.slice(0):input.slice().buffer);
    output.source={name:input.name||'document.pdf',bytes:bytes.byteLength,sha256:await sha(bytes),preserved:true};
    if(new TextDecoder().decode(bytes.slice(0,5))!=='%PDF-')throw problem('INVALID_PDF','Input does not start with a PDF header.');
    // No third-party PDF bytes, models, scripts or fonts are fetched during jobs.
    emit({phase:'opening',completed:0,message:'Opening PDF'});
    const pdfjs=await wait(import(ROOT+'pdfjs/pdf.mjs'));signal.throwIfAborted();
    pdfjs.GlobalWorkerOptions.workerSrc=ROOT+'pdfjs/pdf.worker.mjs';
    loading=pdfjs.getDocument({data:bytes.slice(),isEvalSupported:false,useSystemFonts:true,cMapUrl:ROOT+'pdfjs/cmaps/',cMapPacked:true,standardFontDataUrl:ROOT+'pdfjs/standard_fonts/',wasmUrl:ROOT+'pdfjs/wasm/'});
    loading.onPassword=()=>{loading.destroy();};pdf=await wait(loading.promise);signal.throwIfAborted();
    if(pdf.numPages>limits.maxPages&&!selected)throw problem('PAGE_LIMIT',`PDF has ${pdf.numPages} pages; select at most ${limits.maxPages} pages explicitly.`);
    const indices=selected||Array.from({length:pdf.numPages},(_,i)=>i+1);
    if(indices.some(i=>i>pdf.numPages))throw problem('INVALID_PAGES','Selected page exceeds the PDF page count.');
    output.source.pageCount=pdf.numPages;output.source.selectedPages=indices;
    worker=new Worker('/pdf-api/v1/inference-worker.js',{type:'module'});let serial=0;
    const rpc=(operation,data,transfer=[])=>new Promise((resolve,reject)=>{
      signal.throwIfAborted();const id=++serial;rejectRpc=reject;
      worker.onmessage=({data})=>{if(data.event){emit({...data.event,completed:output.pages.length,total:indices.length});return;}if(data.id!==id)return;rejectRpc=null;data.error?reject(problem(data.error.code,data.error.message)):resolve(data.result);};
      worker.onerror=e=>reject(problem('RUNTIME_MISSING',e.message||'Inference worker could not load its local runtime assets.'));worker.postMessage({id,operation,...data},transfer);
    });
    const probe=await rpc('boot',{bytes:bytes.buffer},[bytes.buffer]);
    for(const pageNo of indices){
      signal.throwIfAborted();emit({phase:'rasterizing',page:pageNo,completed:output.pages.length,total:indices.length,message:`Rendering page ${pageNo}`});
      const page=await wait(pdf.getPage(pageNo)),text=await wait(page.getTextContent());
      const chars=text.items.reduce((n,item)=>n+(item.str||'').replace(/\s/g,'').length,0);
      const route=ocr==='always'||chars<20?'scanned':'digital';
      if(route==='scanned'&&ocr==='off')throw problem('OCR_REQUIRED',`Page ${pageNo} has no substantial text layer. Enable OCR.`);
      if(route==='digital'&&!probe.digitalAvailable)throw problem('TEXT_LAYER_UNSUPPORTED',`Page ${pageNo}: Docling digital probe rejected; choose OCR always.`);
      const viewport=page.getViewport({scale:2}),width=Math.ceil(viewport.width),height=Math.ceil(viewport.height);
      if(width*height>limits.maxPixels)throw problem('PIXEL_LIMIT',`Page ${pageNo} exceeds ${limits.maxPixels} rendered pixels at required scale 2.`);
      const canvas=document.createElement('canvas');canvas.width=width;canvas.height=height;const context=canvas.getContext('2d',{willReadFrequently:true});
      render=page.render({canvas,canvasContext:context,viewport});await wait(render.promise);render=null;signal.throwIfAborted();
      const rgba=context.getImageData(0,0,width,height).data;canvas.width=canvas.height=0;
      emit({phase:'inference',page:pageNo,completed:output.pages.length,total:indices.length,message:`${route==='scanned'?'OCR and layout':'Text and layout'} on page ${pageNo} (CPU)`});
      const result=await rpc('page',{rgba:rgba.buffer,width,height,scale:2,index:pageNo-1,route,ocr,name:output.source.name},[rgba.buffer]);
      const order=ordered(result.document),diagnostics=[];
      if(order.unlinked.length)diagnostics.push({code:'UNLINKED_TEXT',message:'Docling text items were not linked from document body.',items:order.unlinked});
      let blocks=order.blocks.map((b,index)=>({...b,order:index,provenance:b.provenance.map(p=>({...p,page_no:pageNo}))}));
      const originalCells=textCells(text.items,page.getViewport({scale:1})),geometry=orderTextCells(originalCells);
      let orderEngine='docling.rs-wasm document body';
      if(route==='digital'&&geometry.columns>1){
        const multiset=value=>Array.from(value.replace(/\s/g,'')).sort().join('');
        if(multiset(blocks.map(b=>b.text).join(''))===multiset(originalCells.map(c=>c.text).join(''))&&!(result.document.tables||[]).length){
          blocks=geometry.cells.map((cell,index)=>({id:cell.id,text:cell.text,label:'text',order:index,provenance:[{page_no:pageNo,bbox:{l:cell.bbox[0],t:cell.bbox[1],r:cell.bbox[2],b:cell.bbox[3],coord_origin:'TOPLEFT'}}]}));
          orderEngine='pdf-api repeated-gutter geometry v1 (PDF.js cells)';
          diagnostics.push({code:'GEOMETRIC_ORDER',message:`${geometry.columns} columns inferred from repeated whitespace gutters. Docling merged the text region; independent geometry supplies provisional order. Original Docling output and PDF.js cells are retained.`});
        }else diagnostics.push({code:'ORDER_UNRESOLVED',message:'Multiple text columns detected, but model text differs from embedded cells or includes tables. Automatic repair cannot conserve both; inspect original evidence.'});
      }
      const pageText=blocks.map(b=>b.text).join('\n');
      if(!pageText.trim())diagnostics.push({code:'NO_TEXT',message:'No text recovered. This can be blank content or unsupported recognition; completeness is unverified.'});
      diagnostics.push({code:'COVERAGE_UNVERIFIED',message:'Model output is not a guarantee that every region or glyph was recovered.'});
      const annotations=await page.getAnnotations();
      const links=annotations.filter(a=>typeof a.url==='string').map(a=>({url:a.url,label:a.contentsObj?.str||'',rect:a.rect}));
      output.pages.push({page:pageNo,width:viewport.width/2,height:viewport.height/2,rotation:viewport.rotation,route,ocr:result.ocr,status:'partial',text:pageText,orderEngine,order:blocks.map(b=>b.id),layout:blocks,originalTextCells:originalCells,docling:result.document,doclingPageMap:Object.fromEntries(Object.keys(result.document.pages||{}).map(key=>[key,pageNo])),execution:result.execution,links,diagnostics});
      if(new TextEncoder().encode(JSON.stringify(output)).length>limits.maxOutputBytes)throw problem('OUTPUT_LIMIT','Structured output exceeded the output byte budget.');
      page.cleanup();emit({phase:'page-complete',page:pageNo,completed:output.pages.length,total:indices.length,message:`Page ${pageNo} complete`});
    }
    output.status='partial';output.engine.models=Object.fromEntries(Object.entries(MODELS).filter(([key])=>key==='layout'||key==='dictionary'&&ocr!=='off'||output.pages.some(p=>p.execution.some(e=>e.model===key))).map(([key,m])=>[key,m]));lastExecution={provider:'wasm',version:VERSION,pages:output.pages.length,completedAt:new Date().toISOString()};
  }catch(error){const timedOut=signal.reason?.code==='TIMEOUT';output.status=signal.aborted&&!timedOut?'cancelled':'failed';output.diagnostics.push({code:timedOut?'TIMEOUT':signal.aborted?'CANCELLED':error.code||'PDF_FAILED',message:error.message||String(error)});}
  finally{clearTimeout(timeout);signal.removeEventListener('abort',abort);worker?.terminate();await (pdf?.destroy()||loading?.destroy())?.catch(()=>{});}
  output.text=output.pages.map(p=>p.text).join('\n\n');job.status=output.status;emit({phase:output.status,completed:output.pages.length,message:output.status});return output;
}

async function runFast(input,{limits},controller,emit,job){
  const signal=controller.signal,output={apiVersion:API_VERSION,jobId:job.id,status:'failed',source:null,engine:{name:'pdf-oxide-wasm',version:'0.3.77',provider:'wasm',device:'CPU',readingOrder:'unverified'},pages:[],text:'',diagnostics:[]};let worker,timer;
  try{
    signal.throwIfAborted();job.status='running';const size=input instanceof Blob?input.size:input?.byteLength;
    if(!size||size>limits.maxBytes)throw problem('INPUT_LIMIT','Fast-text PDF input exceeds byte budget.');
    const bytes=input instanceof Blob?await input.arrayBuffer():input instanceof ArrayBuffer?input.slice(0):input.slice().buffer;
    output.source={name:input.name||'document.pdf',bytes:size,sha256:await sha(bytes),preserved:true};
    const result=await new Promise((resolve,reject)=>{
      worker=new Worker('/pdf-worker.js',{type:'module'});
      const abort=()=>{worker.terminate();reject(signal.reason||problem('CANCELLED','PDF cancelled.'));};signal.addEventListener('abort',abort,{once:true});
      timer=setTimeout(()=>controller.abort(problem('TIMEOUT','Fast text exceeded time budget.')),limits.timeoutMs);
      worker.onmessage=({data})=>{if(data.total>limits.maxPages){worker.terminate();reject(problem('PAGE_LIMIT','Fast text exceeds page budget.'));return;}if(data.progress!==undefined)emit({phase:'fast-text',completed:data.progress,total:data.total,message:'Extracting unverified fast text'});else{signal.removeEventListener('abort',abort);data.error?reject(problem('FAST_TEXT_FAILED',data.error)):resolve(data.result);}};
      worker.onerror=e=>reject(problem('RUNTIME_MISSING',e.message));if(signal.aborted){abort();return;}worker.postMessage({bytes,name:output.source.name},[bytes]);
    });
    output.pages=result.pages.map(p=>({...p,route:'fast-text',status:'partial',order:[],layout:[],diagnostics:[{code:'ORDER_UNVERIFIED',message:'PDF Oxide fast text does not establish reading order, layout or OCR.'}]}));output.text=result.text;
    if(new TextEncoder().encode(JSON.stringify(output)).length>limits.maxOutputBytes)throw problem('OUTPUT_LIMIT','Fast text exceeds output budget.');
    output.status='partial';
  }catch(error){output.status=signal.aborted&&signal.reason?.code!=='TIMEOUT'?'cancelled':'failed';output.diagnostics.push({code:error.code||'FAST_TEXT_FAILED',message:error.message||String(error)});}
  finally{clearTimeout(timer);worker?.terminate();}job.status=output.status;emit({phase:output.status,completed:output.pages.length,message:output.status});return output;
}
