/** WebGPU implementations of the pre-processing kernels. Each stage mirrors a function in
 * `cpu.ts` and returns the same integers. Buffers are created per call and destroyed in
 * `finally`; error scopes turn validation and out-of-memory failures into thrown errors so
 * the caller can fall back to the CPU path. */
import { GRAYSCALE_WGSL, HISTOGRAM_WGSL, PROFILE_WGSL, WORKGROUP_SIZE } from './shaders';
import { DEFAULT_SKEW, otsuThreshold, sampleStride, scoreProfiles, skewTable, type SkewEstimate, type SkewOptions, type SkewTable } from './cpu';
import { limitShortfall } from './capability';

type Pipelines = { grayscale: GPUComputePipeline; histogram: GPUComputePipeline; profiles: GPUComputePipeline };
const pipelineCache = new WeakMap<GPUDevice, Promise<Pipelines>>();
const now = () => (typeof performance === 'object' ? performance.now() : Date.now());

/** Thrown when the device cannot run a stage (limits, validation, out of memory). The CPU path is the remedy. */
export class WebGpuStageError extends Error { constructor(message: string) { super(message); this.name = 'WebGpuStageError'; } }

async function pipelines(device: GPUDevice): Promise<Pipelines> {
  let pending = pipelineCache.get(device);
  if (!pending) {
    pending = (async () => {
      const build = (label: string, code: string) => device.createComputePipelineAsync({ label, layout: 'auto', compute: { module: device.createShaderModule({ label, code }), entryPoint: 'main' } });
      const [grayscale, histogram, profiles] = await Promise.all([build('ocr-grayscale', GRAYSCALE_WGSL), build('ocr-histogram', HISTOGRAM_WGSL), build('ocr-profiles', PROFILE_WGSL)]);
      return { grayscale, histogram, profiles };
    })();
    pipelineCache.set(device, pending);
    pending.catch(() => pipelineCache.delete(device));
  }
  return pending;
}
function roundUp4(bytes: number): number { return (bytes + 3) & ~3; }
function storage(device: GPUDevice, label: string, data: ArrayBufferView, usage = GPUBufferUsage.STORAGE): GPUBuffer {
  const buffer = device.createBuffer({ label, size: roundUp4(data.byteLength), usage, mappedAtCreation: true });
  new Uint8Array(buffer.getMappedRange()).set(new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
  buffer.unmap();
  return buffer;
}
function uniform(device: GPUDevice, label: string, words: Int32Array | Uint32Array): GPUBuffer { return storage(device, label, words, GPUBufferUsage.UNIFORM); }
async function readback(device: GPUDevice, source: GPUBuffer, bytes: number): Promise<ArrayBuffer> {
  const staging = device.createBuffer({ label: 'ocr-readback', size: bytes, usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST });
  try {
    const encoder = device.createCommandEncoder();
    encoder.copyBufferToBuffer(source, 0, staging, 0, bytes);
    device.queue.submit([encoder.finish()]);
    await staging.mapAsync(GPUMapMode.READ);
    const copy = staging.getMappedRange().slice(0);
    staging.unmap();
    return copy;
  } finally { staging.destroy(); }
}
async function guarded<T>(device: GPUDevice, stage: string, work: () => Promise<T>): Promise<T> {
  device.pushErrorScope('out-of-memory'); device.pushErrorScope('validation');
  let result: T | undefined, failure: unknown;
  try { result = await work(); } catch (error) { failure = error; }
  const validation = await device.popErrorScope(), memory = await device.popErrorScope();
  if (memory) throw new WebGpuStageError(`${stage}: out of GPU memory (${memory.message})`);
  if (validation) throw new WebGpuStageError(`${stage}: ${validation.message}`);
  if (failure) throw failure instanceof WebGpuStageError ? failure : new WebGpuStageError(`${stage}: ${(failure as Error).message || String(failure)}`);
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
export async function grayscaleGpu(device: GPUDevice, rgba: Uint8Array | Uint8ClampedArray, pixels: number): Promise<Uint8Array> {
  checkLimits(device, pixels);
  const words = Math.ceil(pixels / 4), buffers: GPUBuffer[] = [];
  try {
    return await guarded(device, 'grayscale', async () => {
      const { grayscale } = await pipelines(device);
      const params = uniform(device, 'ocr-grayscale-params', new Uint32Array([pixels, words, 0, 0]));
      const input = storage(device, 'ocr-rgba', new Uint8Array(rgba.buffer, rgba.byteOffset, pixels * 4));
      const output = device.createBuffer({ label: 'ocr-gray', size: words * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC });
      buffers.push(params, input, output);
      dispatch(device, grayscale, [params, input, output], words);
      return new Uint8Array(await readback(device, output, words * 4), 0, pixels);
    });
  } finally { for (const buffer of buffers) buffer.destroy(); }
}
/** Histogram kernel over luma bytes (uploaded packed four per word). */
export async function histogramGpu(device: GPUDevice, gray: Uint8Array): Promise<Uint32Array> {
  checkLimits(device, gray.length);
  const buffers: GPUBuffer[] = [];
  try {
    return await guarded(device, 'histogram', async () => {
      const { histogram } = await pipelines(device);
      const words = Math.ceil(gray.length / 4);
      const params = uniform(device, 'ocr-histogram-params', new Uint32Array([gray.length, words, 0, 0]));
      const input = storage(device, 'ocr-gray', gray);
      const output = device.createBuffer({ label: 'ocr-histogram', size: 256 * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST });
      buffers.push(params, input, output);
      dispatch(device, histogram, [params, input, output], words);
      return new Uint32Array(await readback(device, output, 256 * 4));
    });
  } finally { for (const buffer of buffers) buffer.destroy(); }
}
/** Projection-profile kernel: angles × height row counts of sampled dark pixels. */
export async function profilesGpu(device: GPUDevice, gray: Uint8Array, width: number, height: number, threshold: number, table: SkewTable, stride: number): Promise<Uint32Array> {
  checkLimits(device, width * height);
  const buffers: GPUBuffer[] = [];
  try {
    return await guarded(device, 'profiles', async () => {
      const { profiles } = await pipelines(device);
      const angles = table.degrees.length, samplesX = Math.ceil(width / stride), samplesY = Math.ceil(height / stride);
      const params = uniform(device, 'ocr-profile-params', new Int32Array([width, height, stride, threshold, samplesX, samplesY, angles, width >> 1, height >> 1, 0, 0, 0]));
      const pairs = new Int32Array(angles * 2);
      for (let a = 0; a < angles; a++) { pairs[a * 2] = table.sinQ[a]; pairs[a * 2 + 1] = table.cosQ[a]; }
      const input = storage(device, 'ocr-gray', gray), sincos = storage(device, 'ocr-skew-table', pairs);
      const output = device.createBuffer({ label: 'ocr-profiles', size: angles * height * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST });
      buffers.push(params, input, sincos, output);
      dispatch(device, profiles, [params, input, sincos, output], samplesX * samplesY);
      return new Uint32Array(await readback(device, output, angles * height * 4));
    });
  } finally { for (const buffer of buffers) buffer.destroy(); }
}

export type GpuStages = { gray: Uint8Array; histogram: Uint32Array; threshold: number; profiles: Uint32Array; skew: SkewEstimate; stride: number; table: SkewTable; timings: { grayscaleMs: number; histogramMs: number; skewMs: number; totalMs: number } };

/** The whole GPU pipeline: the luma buffer stays resident on the device between stages. */
export async function preprocessGpu(device: GPUDevice, rgba: Uint8Array | Uint8ClampedArray, width: number, height: number, options: SkewOptions = {}, signal?: AbortSignal): Promise<GpuStages> {
  const pixels = width * height;
  checkLimits(device, pixels);
  const words = Math.ceil(pixels / 4), table = skewTable(options), stride = sampleStride(width, height, options.maxSamples ?? DEFAULT_SKEW.maxSamples);
  const angles = table.degrees.length, buffers: GPUBuffer[] = [];
  try {
    const start = now();
    const { grayscale, histogram, profiles } = await pipelines(device);
    const grayBuffer = device.createBuffer({ label: 'ocr-gray', size: words * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC });
    buffers.push(grayBuffer);
    const gray = await guarded(device, 'grayscale', async () => {
      const params = uniform(device, 'ocr-grayscale-params', new Uint32Array([pixels, words, 0, 0]));
      const input = storage(device, 'ocr-rgba', new Uint8Array(rgba.buffer, rgba.byteOffset, pixels * 4));
      buffers.push(params, input);
      dispatch(device, grayscale, [params, input, grayBuffer], words);
      return new Uint8Array(await readback(device, grayBuffer, words * 4), 0, pixels);
    });
    const afterGray = now();
    signal?.throwIfAborted();
    const bins = await guarded(device, 'histogram', async () => {
      const params = uniform(device, 'ocr-histogram-params', new Uint32Array([pixels, words, 0, 0]));
      const output = device.createBuffer({ label: 'ocr-histogram', size: 256 * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST });
      buffers.push(params, output);
      dispatch(device, histogram, [params, grayBuffer, output], words);
      return new Uint32Array(await readback(device, output, 256 * 4));
    });
    const threshold = otsuThreshold(bins), afterHistogram = now();
    signal?.throwIfAborted();
    const rows = await guarded(device, 'profiles', async () => {
      const samplesX = Math.ceil(width / stride), samplesY = Math.ceil(height / stride);
      const params = uniform(device, 'ocr-profile-params', new Int32Array([width, height, stride, threshold, samplesX, samplesY, angles, width >> 1, height >> 1, 0, 0, 0]));
      const pairs = new Int32Array(angles * 2);
      for (let a = 0; a < angles; a++) { pairs[a * 2] = table.sinQ[a]; pairs[a * 2 + 1] = table.cosQ[a]; }
      const sincos = storage(device, 'ocr-skew-table', pairs);
      const output = device.createBuffer({ label: 'ocr-profiles', size: angles * height * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST });
      buffers.push(params, sincos, output);
      dispatch(device, profiles, [params, grayBuffer, sincos, output], samplesX * samplesY);
      return new Uint32Array(await readback(device, output, angles * height * 4));
    });
    const skew = scoreProfiles(rows, height, table), end = now();
    return { gray, histogram: bins, threshold, profiles: rows, skew, stride, table, timings: { grayscaleMs: afterGray - start, histogramMs: afterHistogram - afterGray, skewMs: end - afterHistogram, totalMs: end - start } };
  } finally { for (const buffer of buffers) buffer.destroy(); }
}
