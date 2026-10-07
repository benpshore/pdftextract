/** WebGPU implementations of the pre-processing kernels. Each stage mirrors a function in
 * `cpu.ts` and returns the same integers. Buffers are created per call and destroyed in
 * `finally`; error scopes turn validation and out-of-memory failures into thrown errors so
 * the caller can fall back to the CPU path. */
import { GRAYSCALE_WGSL, HISTOGRAM_WGSL, PROFILE_WGSL, WORKGROUP_SIZE } from './shaders';
import { DEFAULT_SKEW, otsuThreshold, sampleStride, scoreProfiles, skewTable, type SkewEstimate, type SkewOptions, type SkewTable } from './cpu';
import { limitShortfall } from './capability';
import { checkGpuWait, gpuWaitOptions, waitForGpu, WebGpuTimeoutError, type GpuWaitOptions } from './wait';

type Pipelines = { grayscale: GPUComputePipeline; histogram: GPUComputePipeline; profiles: GPUComputePipeline };
const pipelineCache = new WeakMap<GPUDevice, Promise<Pipelines>>();
const now = () => (typeof performance === 'object' ? performance.now() : Date.now());

/** Thrown when the device cannot run a stage (limits, validation, out of memory). The CPU path is the remedy. */
export class WebGpuStageError extends Error { constructor(message: string) { super(message); this.name = 'WebGpuStageError'; } }

async function pipelines(device: GPUDevice, wait: GpuWaitOptions): Promise<Pipelines> {
  checkGpuWait(wait, 'createComputePipelineAsync');
  let pending = pipelineCache.get(device);
  if (!pending) {
    pending = (async () => {
      const build = (label: string, code: string) => device.createComputePipelineAsync({ label, layout: 'auto', compute: { module: device.createShaderModule({ label, code }), entryPoint: 'main' } });
      const [grayscale, histogram, profiles] = await Promise.all([build('ocr-grayscale', GRAYSCALE_WGSL), build('ocr-histogram', HISTOGRAM_WGSL), build('ocr-profiles', PROFILE_WGSL)]);
      return { grayscale, histogram, profiles };
    })();
    pipelineCache.set(device, pending);
    pending.catch(() => { if (pipelineCache.get(device) === pending) pipelineCache.delete(device); });
  }
  try { return await waitForGpu(() => pending!, wait, 'createComputePipelineAsync'); }
  catch (error) { if (pipelineCache.get(device) === pending) pipelineCache.delete(device); throw error; }
}
function roundUp4(bytes: number): number { return (bytes + 3) & ~3; }
function track(buffers: GPUBuffer[], buffer: GPUBuffer): GPUBuffer { buffers.push(buffer); return buffer; }
function storage(device: GPUDevice, label: string, data: ArrayBufferView, buffers: GPUBuffer[], usage = GPUBufferUsage.STORAGE): GPUBuffer {
  const buffer = track(buffers, device.createBuffer({ label, size: roundUp4(data.byteLength), usage, mappedAtCreation: true }));
  new Uint8Array(buffer.getMappedRange()).set(new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
  buffer.unmap();
  return buffer;
}
function uniform(device: GPUDevice, label: string, words: Int32Array | Uint32Array, buffers: GPUBuffer[]): GPUBuffer { return storage(device, label, words, buffers, GPUBufferUsage.UNIFORM); }
async function readback(device: GPUDevice, source: GPUBuffer, bytes: number, wait: GpuWaitOptions): Promise<ArrayBuffer> {
  const staging = device.createBuffer({ label: 'ocr-readback', size: bytes, usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST });
  try {
    const encoder = device.createCommandEncoder();
    encoder.copyBufferToBuffer(source, 0, staging, 0, bytes);
    device.queue.submit([encoder.finish()]);
    await waitForGpu(() => staging.mapAsync(GPUMapMode.READ), wait, 'mapAsync');
    const copy = staging.getMappedRange().slice(0);
    staging.unmap();
    return copy;
  } finally { staging.destroy(); }
}
async function guarded<T>(device: GPUDevice, stage: string, wait: GpuWaitOptions, work: () => Promise<T>): Promise<T> {
  checkGpuWait(wait, stage);
  device.pushErrorScope('out-of-memory'); device.pushErrorScope('validation');
  let result: T | undefined, failure: unknown, failed = false;
  try { result = await work(); } catch (error) { failure = error; failed = true; }
  // Pop both scopes synchronously before yielding, even when work failed. Observe their
  // promises without waiting on them after abort; scope cleanup may itself stall.
  const errors = Promise.all([device.popErrorScope(), device.popErrorScope()]);
  errors.catch(() => {});
  if (failed) {
    if (wait.signal?.aborted) throw wait.signal.reason;
    throw failure instanceof WebGpuStageError || failure instanceof WebGpuTimeoutError ? failure : new WebGpuStageError(`${stage}: ${(failure as Error)?.message || String(failure)}`);
  }
  const [validation, memory] = await waitForGpu(() => errors, wait, 'popErrorScope');
  if (memory) throw new WebGpuStageError(`${stage}: out of GPU memory (${memory.message})`);
  if (validation) throw new WebGpuStageError(`${stage}: ${validation.message}`);
  return result as T;
}
function dispatch(device: GPUDevice, pipeline: GPUComputePipeline, entries: GPUBuffer[], items: number): void {
  const workgroups = Math.ceil(items / WORKGROUP_SIZE);
  if (workgroups > device.limits.maxComputeWorkgroupsPerDimension) throw new WebGpuStageError(`${workgroups} workgroups exceed maxComputeWorkgroupsPerDimension`);
  const group = device.createBindGroup({ layout: pipeline.getBindGroupLayout(0), entries: entries.map((buffer, binding) => ({ binding, resource: { buffer } })) });
  const encoder = device.createCommandEncoder(), pass = encoder.beginComputePass();
  pass.setPipeline(pipeline); pass.setBindGroup(0, group); pass.dispatchWorkgroups(workgroups); pass.end();
  device.queue.submit([encoder.finish()]);
}
function checkLimits(device: GPUDevice, pixels: number): void {
  const shortfall = limitShortfall(device.limits, pixels);
  if (shortfall) throw new WebGpuStageError(`adapter limits too small: ${shortfall}`);
}

/** Grayscale kernel: returns one luma byte per pixel. */
export async function grayscaleGpu(device: GPUDevice, rgba: Uint8Array | Uint8ClampedArray, pixels: number, options: GpuWaitOptions = {}): Promise<Uint8Array> {
  const wait = gpuWaitOptions(options);
  checkLimits(device, pixels);
  const words = Math.ceil(pixels / 4), buffers: GPUBuffer[] = [];
  try {
    return await guarded(device, 'grayscale', wait, async () => {
      const { grayscale } = await pipelines(device, wait);
      const params = uniform(device, 'ocr-grayscale-params', new Uint32Array([pixels, words, 0, 0]), buffers);
      const input = storage(device, 'ocr-rgba', new Uint8Array(rgba.buffer, rgba.byteOffset, pixels * 4), buffers);
      const output = track(buffers, device.createBuffer({ label: 'ocr-gray', size: words * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC }));
      dispatch(device, grayscale, [params, input, output], words);
      return new Uint8Array(await readback(device, output, words * 4, wait), 0, pixels);
    });
  } finally { for (const buffer of buffers) buffer.destroy(); }
}
/** Histogram kernel over luma bytes (uploaded packed four per word). */
export async function histogramGpu(device: GPUDevice, gray: Uint8Array, options: GpuWaitOptions = {}): Promise<Uint32Array> {
  const wait = gpuWaitOptions(options);
  checkLimits(device, gray.length);
  const buffers: GPUBuffer[] = [];
  try {
    return await guarded(device, 'histogram', wait, async () => {
      const { histogram } = await pipelines(device, wait);
      const words = Math.ceil(gray.length / 4);
      const params = uniform(device, 'ocr-histogram-params', new Uint32Array([gray.length, words, 0, 0]), buffers);
      const input = storage(device, 'ocr-gray', gray, buffers);
      const output = track(buffers, device.createBuffer({ label: 'ocr-histogram', size: 256 * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST }));
      dispatch(device, histogram, [params, input, output], words);
      return new Uint32Array(await readback(device, output, 256 * 4, wait));
    });
  } finally { for (const buffer of buffers) buffer.destroy(); }
}
/** Projection-profile kernel: angles × height row counts of sampled dark pixels. */
export async function profilesGpu(device: GPUDevice, gray: Uint8Array, width: number, height: number, threshold: number, table: SkewTable, stride: number, options: GpuWaitOptions = {}): Promise<Uint32Array> {
  const wait = gpuWaitOptions(options);
  checkLimits(device, width * height);
  const buffers: GPUBuffer[] = [];
  try {
    return await guarded(device, 'profiles', wait, async () => {
      const { profiles } = await pipelines(device, wait);
      const angles = table.degrees.length, samplesX = Math.ceil(width / stride), samplesY = Math.ceil(height / stride);
      const params = uniform(device, 'ocr-profile-params', new Int32Array([width, height, stride, threshold, samplesX, samplesY, angles, width >> 1, height >> 1, 0, 0, 0]), buffers);
      const pairs = new Int32Array(angles * 2);
      for (let a = 0; a < angles; a++) { pairs[a * 2] = table.sinQ[a]; pairs[a * 2 + 1] = table.cosQ[a]; }
      const input = storage(device, 'ocr-gray', gray, buffers), sincos = storage(device, 'ocr-skew-table', pairs, buffers);
      const output = track(buffers, device.createBuffer({ label: 'ocr-profiles', size: angles * height * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST }));
      dispatch(device, profiles, [params, input, sincos, output], samplesX * samplesY);
      return new Uint32Array(await readback(device, output, angles * height * 4, wait));
    });
  } finally { for (const buffer of buffers) buffer.destroy(); }
}

export type GpuStages = { gray: Uint8Array; histogram: Uint32Array; threshold: number; profiles: Uint32Array; skew: SkewEstimate; stride: number; table: SkewTable; timings: { grayscaleMs: number; histogramMs: number; skewMs: number; totalMs: number } };

/** The whole GPU pipeline: the luma buffer stays resident on the device between stages. */
export async function preprocessGpu(device: GPUDevice, rgba: Uint8Array | Uint8ClampedArray, width: number, height: number, options: SkewOptions = {}, signal?: AbortSignal, waitOptions: GpuWaitOptions = {}): Promise<GpuStages> {
  const wait = gpuWaitOptions({ ...waitOptions, signal: signal ?? waitOptions.signal });
  const pixels = width * height;
  checkLimits(device, pixels);
  const words = Math.ceil(pixels / 4), table = skewTable(options), stride = sampleStride(width, height, options.maxSamples ?? DEFAULT_SKEW.maxSamples);
  const angles = table.degrees.length, buffers: GPUBuffer[] = [];
  try {
    const start = now();
    const { grayscale, histogram, profiles } = await pipelines(device, wait);
    const grayBuffer = track(buffers, device.createBuffer({ label: 'ocr-gray', size: words * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC }));
    const gray = await guarded(device, 'grayscale', wait, async () => {
      const params = uniform(device, 'ocr-grayscale-params', new Uint32Array([pixels, words, 0, 0]), buffers);
      const input = storage(device, 'ocr-rgba', new Uint8Array(rgba.buffer, rgba.byteOffset, pixels * 4), buffers);
      dispatch(device, grayscale, [params, input, grayBuffer], words);
      return new Uint8Array(await readback(device, grayBuffer, words * 4, wait), 0, pixels);
    });
    const afterGray = now();
    signal?.throwIfAborted();
    const bins = await guarded(device, 'histogram', wait, async () => {
      const params = uniform(device, 'ocr-histogram-params', new Uint32Array([pixels, words, 0, 0]), buffers);
      const output = track(buffers, device.createBuffer({ label: 'ocr-histogram', size: 256 * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST }));
      dispatch(device, histogram, [params, grayBuffer, output], words);
      return new Uint32Array(await readback(device, output, 256 * 4, wait));
    });
    const threshold = otsuThreshold(bins), afterHistogram = now();
    signal?.throwIfAborted();
    const rows = await guarded(device, 'profiles', wait, async () => {
      const samplesX = Math.ceil(width / stride), samplesY = Math.ceil(height / stride);
      const params = uniform(device, 'ocr-profile-params', new Int32Array([width, height, stride, threshold, samplesX, samplesY, angles, width >> 1, height >> 1, 0, 0, 0]), buffers);
      const pairs = new Int32Array(angles * 2);
      for (let a = 0; a < angles; a++) { pairs[a * 2] = table.sinQ[a]; pairs[a * 2 + 1] = table.cosQ[a]; }
      const sincos = storage(device, 'ocr-skew-table', pairs, buffers);
      const output = track(buffers, device.createBuffer({ label: 'ocr-profiles', size: angles * height * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST }));
      dispatch(device, profiles, [params, grayBuffer, sincos, output], samplesX * samplesY);
      return new Uint32Array(await readback(device, output, angles * height * 4, wait));
    });
    const skew = scoreProfiles(rows, height, table), end = now();
    return { gray, histogram: bins, threshold, profiles: rows, skew, stride, table, timings: { grayscaleMs: afterGray - start, histogramMs: afterHistogram - afterGray, skewMs: end - afterHistogram, totalMs: end - start } };
  } finally { for (const buffer of buffers) buffer.destroy(); }
}
