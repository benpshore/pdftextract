// MIT session interop adapted from pinned upstream www/pipeline.js.
import { ROOT, MODELS, VERSION } from './manifest.js';
import {cropForOcr} from './regions.js';

// Measured against default prepacking with real layout/detector/recognizer runs.
// Avoid retaining packed weight copies; keep the same models and CPU provider.
const SESSION_OPTIONS = { executionProviders: ['wasm'], logSeverityLevel: 3, extra: { session: { disable_prepacking: '1' } } };

let docling, wasm, ort, digital, layout, rec, detector, dictionary, jobId;
let documentOpen = false, execution = [], parses = 0, peakRustBytes = 0;
const runtimeId = crypto.randomUUID();
const loaded = new Map();
const fail = (code, message) => Object.assign(new Error(message), { code });
const status = event => self.postMessage({ jobId, event });
const hash = async bytes => [...new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))].map(n => n.toString(16).padStart(2, '0')).join('');
function memory() {
  const bytes = wasm?.memory.buffer.byteLength || 0;
  peakRustBytes = Math.max(bytes, peakRustBytes);
  return bytes;
}
function metrics() {
  return { runtimeId, rustWasmCapacityBytes: memory(), peakRustWasmCapacityBytes: peakRustBytes,
    rustPdfParses: parses, modelAssets: Object.fromEntries(loaded),sessionOptions:SESSION_OPTIONS,
    memoryScope: 'Rust linear-memory capacity only; excludes ONNX, PDF.js, JS heap and browser overhead' };
}
async function asset(key) {
  const spec = MODELS[key];
  const response = await fetch(ROOT + 'models/' + spec.file, { cache: 'force-cache' });
  if (!response.ok) throw fail('MODEL_MISSING', `${spec.file}: HTTP ${response.status}. Stage the documented model assets first.`);
  const contentLength = Number(response.headers.get('Content-Length'));
  if (contentLength && contentLength !== spec.bytes) throw fail('MODEL_INVALID', `${spec.file}: unexpected Content-Length.`);
  // Fixed-size staging rejects oversized/short chunked responses before session creation.
  const bytes = new Uint8Array(spec.bytes);
  const reader = response.body.getReader();
  let offset = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      if (offset + value.length > bytes.length) throw fail('MODEL_INVALID', `${spec.file}: asset exceeds its pinned size.`);
      bytes.set(value, offset); offset += value.length;
    }
  } finally { await reader.cancel().catch(() => {}); reader.releaseLock(); }
  if (offset !== spec.bytes || await hash(bytes) !== spec.sha256) throw fail('MODEL_INVALID', `${spec.file}: size or SHA-256 mismatch.`);
  return bytes;
}
async function session(key) {
  status({ phase: 'assets', model: key, message: `Loading ${key} model (CPU/WASM)` });
  const started = performance.now();
  try {
    const s = await ort.InferenceSession.create(await asset(key), structuredClone(SESSION_OPTIONS));
    loaded.set(key, { ...MODELS[key], provider: 'wasm', sessionId: crypto.randomUUID(), loadMilliseconds: Math.round(performance.now() - started) });
    status({phase:'model-ready',model:key,message:`${key} session ready`,resources:metrics()});
    return s;
  } catch (error) { throw error.code ? error : fail('MODEL_UNSUPPORTED', `${key} CPU/WASM session failed: ${error.message}`); }
}
async function infer(session, feeds, model) {
  const started = performance.now(); memory();
  status({ phase: 'model-inference', model, message: `Running ${model} on CPU/WASM` });
  const out = await session.run(feeds);
  const inferenceMilliseconds = Math.round(performance.now() - started);
  const same = (actual, expected) => actual.length === expected.length && actual.every((n, i) => n === expected[i]);
  const input = Object.values(feeds)[0];
  const first = out[session.outputNames[0]];
  if (model === 'layout' && (!out.logits || !out.pred_boxes || !same(out.logits.dims, [1, 300, 17]) || !same(out.pred_boxes.dims, [1, 300, 4]))) throw fail('MODEL_OUTPUT_INVALID', 'Layout output dimensions differ from the pinned model contract.');
  if (model === 'detector' && !same(first.dims, [1, 1, input.dims[2], input.dims[3]])) throw fail('MODEL_OUTPUT_INVALID', 'Detector output dimensions differ from the input image.');
  if (model === 'recognition' && (first.dims.length !== 3 || first.dims[0] !== input.dims[0] || first.dims[2] !== 97)) throw fail('MODEL_OUTPUT_INVALID', 'English recognition output dimensions differ from its pinned dictionary.');
  const outputs = {};
  for (const [name, tensor] of Object.entries(out)) {
    if (tensor.type !== 'float32') throw fail('MODEL_OUTPUT_INVALID', `${model}/${name}: expected float32 output.`);
    const data = tensor.data;
    let min = Infinity, max = -Infinity;
    for (const n of data) { if (!Number.isFinite(n)) throw fail('MODEL_OUTPUT_INVALID', `${model}/${name}: non-finite output.`); min = Math.min(min, n); max = Math.max(max, n); }
    outputs[name] = { type: tensor.type, dims: Array.from(tensor.dims), elements: data.length, min, max,
      sha256: await hash(new Uint8Array(data.buffer, data.byteOffset, data.byteLength)) };
  }
  execution.push({ model, provider: 'wasm', sessionId: loaded.get(model).sessionId, milliseconds: inferenceMilliseconds, outputs });
  status({phase:'model-complete',model,message:`${model} inference complete`,resources:metrics()});
  memory(); return out;
}
async function begin(id) {
  if (documentOpen) throw fail('RUNTIME_STATE', 'Previous PDF document was not released.');
  jobId = id; parses = 0; execution = []; peakRustBytes = 0;
  if (!docling) {
    docling = await import(ROOT + 'docling_wasm.js');
    wasm = await docling.default({ module_or_path: ROOT + 'docling_wasm_bg.wasm' });
    if (docling.version() !== VERSION) throw fail('RUNTIME_VERSION', 'Unexpected Docling runtime version: ' + docling.version());
    ort = await import(ROOT + 'ort/ort.wasm.min.mjs');
    ort.env.wasm.numThreads = 1;
    ort.env.wasm.proxy = false;
    ort.env.wasm.wasmPaths = ROOT + 'ort/';
  }
  documentOpen = true;
  // No PDF parse or ONNX model load until a selected page needs it.
  return { version: docling.version(), ...metrics() };
}
function parseDigital(bytes) {
  if (!documentOpen || digital || parses) throw fail('RUNTIME_STATE', 'A digital PDF may be parsed once per document.');
  parses++;
  try { digital = new docling.DigitalConverter(new Uint8Array(bytes)); }
  catch (error) { throw fail('TEXT_LAYER_UNSUPPORTED', `${String(error)}. Select OCR always to rasterize explicitly.`); }
  memory();
  return { pageCount: digital.page_count(), ...metrics() };
}
async function ensureLayout() {
  if (layout) return;
  const s = await session('layout');
  layout = { run: async data => {
    const out = await infer(s, { pixel_values: new ort.Tensor('float32', data, [1, 3, 640, 640]) }, 'layout');
    const tensor = name => ({ data: out[name].data, dims: Array.from(out[name].dims) });
    return { logits: tensor('logits'), boxes: tensor('pred_boxes') };
  } };
}
async function ensureRecognition() {
  if (rec) return;
  dictionary = new TextDecoder().decode(await asset('dictionary'));
  loaded.set('dictionary', MODELS.dictionary);
  const s = await session('recognition');
  rec = { run: async (n, h, w, data) => {
    const out = await infer(s, { [s.inputNames[0]]: new ort.Tensor('float32', data, [n, 3, h, w]) }, 'recognition');
    const t = out[s.outputNames[0]]; return { data: t.data, dims: Array.from(t.dims) };
  } };
}
async function ensureDetector() {
  if (detector) return;
  const s = await session('detector');
  detector = { run: async (h, w, data) => {
    const out = await infer(s, { [s.inputNames[0]]: new ort.Tensor('float32', data, [1, 3, h, w]) }, 'detector');
    const t = out[s.outputNames[0]]; return { data: t.data, dims: Array.from(t.dims) };
  } };
}
async function page({ rgba, width, height, scale, index, route, ocr, regions=[],nativeCells=[],name, maxOutputBytes }) {
  if (!documentOpen) throw fail('RUNTIME_STATE', 'No PDF document is open.');
  if (route === 'digital' && !digital) throw fail('RUNTIME_STATE', 'The digital PDF has not been parsed.');
  const started = performance.now();
  execution = [];
  await ensureLayout();
  if (route==='scanned'||regions.length) await ensureRecognition();
  let conv = digital;
  if (route === 'scanned') {
    if (ocr === 'off') throw fail('OCR_REQUIRED', `Page ${index + 1} needs OCR; OCR is disabled.`);
    await ensureDetector(); conv = new docling.ScannedConverter(dictionary); conv.setDetector(detector);
  }
  try {
    if (route === 'digital') await conv.add_page(index, new Uint8Array(rgba), width, height, scale, layout, undefined);
    else await conv.add_page(new Uint8Array(rgba), width, height, scale, layout, rec);
    const json = conv.finish(name, 'json', 'placeholder');
    memory();
    let resultBytes=new TextEncoder().encode(json).byteLength,peakCropRgbaBytes=0;
    if (resultBytes > maxOutputBytes) throw fail('OUTPUT_LIMIT', 'Docling page JSON exceeded the result budget.');
    const regionResults=[];
    for(const region of regions){
      await ensureDetector();
      const crop=cropForOcr(new Uint8Array(rgba),width,region,nativeCells,scale);
      peakCropRgbaBytes=Math.max(peakCropRgbaBytes,crop.pixels.byteLength);
      const converter=new docling.ScannedConverter(dictionary);converter.setDetector(detector);
      try{
        status({phase:'region-ocr',region:region.id,message:`Recognizing raster region ${region.id} without native text`});
        await converter.add_page(crop.pixels,crop.width,crop.height,scale,layout,rec);
        const regionJson=converter.finish(name,'json','placeholder');
        resultBytes+=new TextEncoder().encode(regionJson).length;
        if(resultBytes>maxOutputBytes)throw fail('OUTPUT_LIMIT','Native and raster region JSON exceeded the result budget.');
        regionResults.push({...region,maskedNativeCells:crop.maskedNativeCells,document:JSON.parse(regionJson)});
      }finally{converter.free();}
    }
    return { document: JSON.parse(json),regions:regionResults,route,ocr:route==='scanned'?'page':regions.length?'regions':'off',
      execution, resources: { ...metrics(), pageMilliseconds: Math.round(performance.now() - started), rgbaBytes: rgba.byteLength,peakCropRgbaBytes,ocrRegionCount:regions.length } };
  } finally { if (route === 'scanned') conv.free(); }
}
function end() {
  digital?.free(); digital = undefined; documentOpen = false;
  const result = metrics(); execution = []; jobId = undefined; return result;
}
let busy = false;
self.onmessage = async ({ data }) => {
  if (busy) { self.postMessage({ id: data.id, error: { code: 'RUNTIME_STATE', message: 'Concurrent command rejected.' } }); return; }
  busy = true;
  try {
    if (data.operation !== 'begin' && data.jobId !== jobId) throw fail('RUNTIME_STATE', 'PDF job identity mismatch.');
    let result;
    switch (data.operation) {
      case 'begin': result = await begin(data.jobId); break;
      case 'digital': result = parseDigital(data.bytes); break;
      case 'page': result = await page(data); break;
      case 'end': result = end(); break;
      default: throw fail('RUNTIME_STATE', 'Unknown PDF operation.');
    }
    self.postMessage({ id: data.id, result });
  } catch (error) {
    self.postMessage({ id: data.id, error: { code: error.code || (data.operation === 'begin' ? 'RUNTIME_MISSING' : 'INFERENCE_FAILED'), message: (error.message || String(error)).slice(0, 2000) } });
  } finally { busy = false; }
};
