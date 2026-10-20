/** Adapter lifecycle tests with explicit decoder/canvas/worker fakes; no OCR quality claim. */
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import ts from 'typescript';

const temporary = await mkdtemp(join(tmpdir(), 'tpe-ocr-lifecycle-'));
const checks = [];
const check = (condition, name) => { assert(condition, name); checks.push(name); };
const tick = () => new Promise(resolve => setTimeout(resolve, 0));
try {
  const source = await readFile(new URL('../lib/image-ocr.ts', import.meta.url), 'utf8');
  await writeFile(join(temporary, 'ocr.mjs'), ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText);
  const { recognizeImage, OCR_TIMEOUT_MS } = await import(pathToFileURL(join(temporary, 'ocr.mjs')));
  const png = await readFile(new URL('./fixtures/ocr-still.png', import.meta.url));
  const file = new File([png], 'synthetic.png', { type: 'image/png' });
  let stall, active = 0, bitmapClosed = 0, calls = [], lateDecode, lateEncode, instances = [], outputs;
  const reset = () => { stall = undefined; bitmapClosed = 0; calls = []; lateDecode = undefined; lateEncode = undefined; instances = []; outputs = [{ text: 'fixture 10.1000/example', confidence: 90 }]; };
  const bitmap = () => ({ width: 1500, height: 340, close() { bitmapClosed++; } });
  globalThis.location = { origin: 'https://local.invalid' };
  globalThis.createImageBitmap = () => stall === 'decode' ? new Promise(resolve => { lateDecode = resolve; }) : Promise.resolve(bitmap());
  globalThis.document = { createElement: () => ({ width: 0, height: 0, getContext: () => ({ fillRect() {}, drawImage() {} }), toBlob: callback => { if (stall === 'encode') lateEncode = callback; else callback(new Blob([png])); } }) };
  globalThis.Worker = class {
    constructor() { active++; this.closed = false; instances.push(this); }
    postMessage(packet) {
      calls.push(packet.action);
      if (stall === 'post') throw new Error('post failed');
      if (stall === packet.action) return;
      queueMicrotask(() => {
        this.onmessage?.({ data: { status: 'progress', data: { status: packet.action, progress: 0.5 } } });
        this.onmessage?.({ data: { status: 'resolve', jobId: packet.jobId, data: packet.action === 'recognize' ? outputs.shift() : {} } });
      });
    }
    terminate() { assert(!this.closed, 'terminate must be idempotent'); this.closed = true; active--; }
  };
  reset();
  const progress = [], ordinary = await recognizeImage(file, event => progress.push(event));
  check(ordinary.text.includes('fixture') && ordinary.metadata.ocr.attempts.length === 1, 'Nonempty first pass is retained without a second pass');
  check(active === 0 && bitmapClosed === 1, 'Normal completion disposes owned worker and bitmap');
  check(progress.at(-1).progress === 1 && progress.every((event, i) => !i || event.progress >= progress[i - 1].progress), 'Progress is monotonic and reaches completion only after output validation');
  reset(); outputs = [{ text: ' ', confidence: 90 }, { text: 'sparse recovery', confidence: 80 }];
  const sparse = await recognizeImage(file);
  check(sparse.text === 'sparse recovery' && sparse.metadata.ocr.attempts[0].confidence === null && sparse.metadata.ocr.attempts[1].segmentation === 'sparse', 'Only an empty first pass triggers sparse recovery with explicit attempt evidence');
  reset(); outputs = [{ text: '', confidence: 90 }, { text: '', confidence: 80 }];
  const empty = await recognizeImage(file);
  check(empty.metadata.ocr.outcome === 'empty' && empty.metadata.ocr.confidence === null && empty.warnings.some(w => w.includes('No text was recognized')) && calls.filter(c => c === 'recognize').length === 2, 'Empty output remains honest and work stops after two passes');
  for (const stage of ['decode', 'encode', 'load', 'loadLanguage', 'initialize', 'recognize']) {
    reset(); stall = stage;
    await assert.rejects(recognizeImage(file, undefined, undefined, { timeoutMs: 40 }), error => error.name === 'TimeoutError' && error.message.includes('original remains saved'));
    check(active === 0, `Deadline settles stalled ${stage} and terminates any worker`);
    if (lateDecode) { lateDecode(bitmap()); await tick(); check(bitmapClosed === 1, 'A decoder result arriving after timeout is closed'); }
    if (lateEncode) { lateEncode(new Blob([png])); await tick(); }
    reset(); stall = stage;
    const controller = new AbortController(), reason = new Error(`Exact ${stage} cancellation`);
    const promise = recognizeImage(file, undefined, controller.signal);
    const rejection = assert.rejects(promise, error => error === reason);
    await tick(); controller.abort(reason); await rejection;
    check(active === 0, `Cancellation settles stalled ${stage} with caller reason and disposes worker`);
    if (lateDecode) { lateDecode(bitmap()); await tick(); }
    if (lateEncode) { lateEncode(null); await tick(); }
    reset(); const next = await recognizeImage(file);
    check(next.text.includes('fixture') && active === 0, `Fresh OCR succeeds after ${stage} cancellation`);
  }
  reset(); stall = 'load';
  const cancelled = new AbortController(), events = [];
  const pending = recognizeImage(file, event => events.push(event), cancelled.signal);
  const rejection = assert.rejects(pending, error => error.name === 'AbortError');
  await tick(); const old = instances[0]; cancelled.abort(); await rejection;
  old.onmessage({ data: { status: 'resolve', jobId: 'image-1', data: {} } }); await tick();
  old.onmessage({ data: { status: 'progress', data: { status: 'late success', progress: 1 } } });
  check(!events.some(event => event.progress === 1) && active === 0, 'Late worker success cannot resurrect a cancelled operation');
  reset(); stall = 'post'; await assert.rejects(recognizeImage(file), /post failed/);
  check(active === 0, 'Synchronous worker post failure disposes the worker');
  reset(); outputs = [null]; await assert.rejects(recognizeImage(file), /no valid text result/);
  check(active === 0, 'Malformed output is an error with no invented text');
  reset(); stall = 'load';
  const broken = recognizeImage(file), failure = assert.rejects(broken, /failed to load/);
  await tick(); instances[0].onerror({ message: 'OCR assets failed to load' }); await failure;
  check(active === 0, 'Worker asset failure is explicit and releases resources');
  for (const timeoutMs of [0, -1, Infinity, NaN, OCR_TIMEOUT_MS + 1]) await assert.rejects(recognizeImage(file, undefined, undefined, { timeoutMs }), /OCR timeout must/);
  check(active === 0, 'Callers may shorten but cannot remove the total deadline');
  console.log(JSON.stringify({ environment: 'Node; decoder/canvas/worker are test fakes', checks }, null, 2));
} finally { await rm(temporary, { recursive: true, force: true }); }
