import { ROOT, MODELS, VERSION } from './manifest.js';
// Session interop follows the MIT-licensed pinned upstream www/pipeline.js.
let docling, ort, digital, layout, rec, detector, dictionary;
const sessions = [];
const execution=[];
const fail = (code,message) => Object.assign(new Error(message),{code});
async function asset(key) {
  const asset = MODELS[key], response = await fetch(ROOT+'models/'+asset.file,{cache:'force-cache'});
  if (!response.ok) throw fail('MODEL_MISSING',`${key} asset ${asset.file}: HTTP ${response.status}. Stage the documented model assets first.`);
  const bytes = await response.arrayBuffer();
  if (bytes.byteLength !== asset.bytes) throw fail('MODEL_INVALID',`${asset.file}: expected ${asset.bytes} bytes, received ${bytes.byteLength}.`);
  const hash = [...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(n=>n.toString(16).padStart(2,'0')).join('');
  if(hash!==asset.sha256) throw fail('MODEL_INVALID',`${asset.file}: SHA-256 mismatch.`);
  return bytes;
}
async function session(key) {
  self.postMessage({event:{phase:'assets',message:`Loading ${key} model (CPU/WASM)`}});
  try {
    const s = await ort.InferenceSession.create(await asset(key),{executionProviders:['wasm'],logSeverityLevel:3});
    sessions.push(s);return s;
  } catch(e) { throw e.code ? e : fail('MODEL_UNSUPPORTED',`${key} CPU/WASM session failed: ${e.message}`); }
}
async function infer(session,feeds,model){
  const start=performance.now();self.postMessage({event:{phase:'model-inference',model,message:`Running ${model} on CPU/WASM`}});
  const out=await session.run(feeds);execution.push({model,provider:'wasm',milliseconds:Math.round(performance.now()-start),outputs:Object.fromEntries(Object.entries(out).map(([name,t])=>[name,{type:t.type,dims:Array.from(t.dims)}]))});return out;
}
async function boot(bytes) {
  docling = await import(ROOT+'docling_wasm.js');
  await docling.default({module_or_path:ROOT+'docling_wasm_bg.wasm'});
  if(docling.version()!==VERSION) throw fail('RUNTIME_VERSION','Unexpected Docling runtime version: '+docling.version());
  ort = await import(ROOT+'ort/ort.wasm.min.mjs');
  ort.env.wasm.numThreads = 1;
  ort.env.wasm.wasmPaths = ROOT+'ort/';
  const l = await session('layout');
  layout = {run:async data=>{
    const out=await infer(l,{pixel_values:new ort.Tensor('float32',data,[1,3,640,640])},'layout');
    const tensor=name=>({data:out[name].data,dims:Array.from(out[name].dims)});
    return {logits:tensor('logits'),boxes:tensor('pred_boxes')};
  }};
  // A rejected digital probe is evidence, not permission to treat a scan as empty success.
  try {digital = new docling.DigitalConverter(new Uint8Array(bytes));}
  catch(e) {self.postMessage({event:{phase:'probe',message:String(e),code:'DIGITAL_PROBE_REJECTED'}});}
  return {version:docling.version(),digitalAvailable:!!digital};
}
async function recognition() {
  if(rec)return;
  dictionary = new TextDecoder().decode(await asset('dictionary'));
  const r=await session('recognition');
  rec={run:async(n,h,w,data)=>{
    const out=await infer(r,{[r.inputNames[0]]:new ort.Tensor('float32',data,[n,3,h,w])},'recognition');
    const t=out[r.outputNames[0]];return {data:t.data,dims:Array.from(t.dims)};
  }};
  digital?.setDict(dictionary);
}
async function detection() {
  if(detector)return;
  const d=await session('detector');
  detector={run:async(h,w,data)=>{
    const out=await infer(d,{[d.inputNames[0]]:new ort.Tensor('float32',data,[1,3,h,w])},'detector');
    const t=out[d.outputNames[0]];return {data:t.data,dims:Array.from(t.dims)};
  }};
}
async function page({rgba,width,height,scale,index,route,ocr,name}) {
  const firstExecution=execution.length;
  if(route==='digital'&&!digital)throw fail('TEXT_LAYER_UNSUPPORTED','Docling could not parse the text layer. Use OCR always to rasterize explicitly.');
  if(ocr!=='off')await recognition();
  let conv=digital;
  if(route==='scanned'){
    if(ocr==='off')throw fail('OCR_REQUIRED',`Page ${index+1} needs OCR; OCR is disabled.`);
    await detection();conv=new docling.ScannedConverter(dictionary);conv.setDetector(detector);
  }
  try {
    if(route==='digital')await conv.add_page(index,new Uint8Array(rgba),width,height,scale,layout,rec);
    else await conv.add_page(new Uint8Array(rgba),width,height,scale,layout,rec);
    return {document:JSON.parse(conv.finish(name,'json','placeholder')),route,ocr:route==='scanned'?'page':ocr==='off'?'off':'pictures',execution:execution.slice(firstExecution)};
  }finally{if(route==='scanned')conv.free();}
}
self.onmessage=async({data})=>{
  try {
    const result=data.operation==='boot'?await boot(data.bytes):await page(data);
    self.postMessage({id:data.id,result});
  }catch(error){self.postMessage({id:data.id,error:{code:error.code||(data.operation==='boot'?'RUNTIME_MISSING':'INFERENCE_FAILED'),message:error.message||String(error)}});}
};
