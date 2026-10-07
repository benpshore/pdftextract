/** OCR pre-processing entry point: grayscale, Otsu threshold and projection-profile skew
 * estimation on WebGPU when it is available, with a CPU fallback that returns identical
 * integers. The first GPU run is cross-checked against the CPU path once per page load
 * and timed; a mismatch or a much slower (software) adapter demotes the session to CPU.
 * Processing stays on the device; the only output besides the result is one console line. */
import { acquireWebGpuDevice, detectWebGpu, webGpuFailureReason, statusLineFor, type WebGpuStatus } from './capability';
import { preprocessCpu, type CpuStages, type SkewOptions } from './cpu';
import { preprocessGpu, type GpuStages } from './gpu';
import { gpuWaitOptions } from './wait';

export type { WebGpuStatus, WebGpuAdapterSummary, WebGpuLimits, BrowserHints } from './capability';
export { detectWebGpu, acquireWebGpuDevice, releaseWebGpuDevice, resetWebGpuState, limitShortfall, requiredLimits } from './capability';
export { grayscaleCpu, histogramCpu, otsuThreshold, profilesCpu, scoreProfiles, skewTable, sampleStride, preprocessCpu, luma, DEFAULT_SKEW } from './cpu';
export type { SkewTable, SkewEstimate, SkewOptions, CpuStages } from './cpu';
export { grayscaleGpu, histogramGpu, profilesGpu, preprocessGpu, WebGpuStageError } from './gpu';
export type { GpuStages } from './gpu';
export { GRAYSCALE_WGSL, HISTOGRAM_WGSL, PROFILE_WGSL, WORKGROUP_SIZE } from './shaders';

export type PreprocessMode = 'auto' | 'cpu' | 'webgpu';
export type PreprocessTimings = { grayscaleMs: number; histogramMs: number; skewMs: number; totalMs: number };
/** Outcome of the one-time GPU-versus-CPU comparison on the first image of the session. */
export type CrossCheck = { identical: boolean; gpuMs: number; cpuMs: number; decision: 'webgpu' | 'cpu'; reason: string };
export type PreprocessResult = {
  /** Which path produced `gray`, `threshold` and `skew`. */
  runtime: 'webgpu' | 'cpu';
  /** Why the CPU path ran when `webgpu` was wanted, or null. */
  fallbackReason: string | null;
  gray: Uint8Array; histogram: Uint32Array; threshold: number;
  skew: { degrees: number; confidence: number; stride: number };
  timings: PreprocessTimings;
  status: WebGpuStatus;
  crossCheck: CrossCheck | null;
};
export type PreprocessOptions = { /** Total WebGPU wait budget, default/max 10 seconds. */ timeoutMs?: number; mode?: PreprocessMode; signal?: AbortSignal; skew?: SkewOptions; /** Defaults to true: compare GPU and CPU once per session. */ crossCheck?: boolean; log?: (line: string) => void };

/** A software adapter slower than this many times the CPU path is not worth the readbacks. */
const SLOWER_FACTOR = 2;
let verdict: CrossCheck | null = null;

/** The result of the session's one-time comparison, if it has run. */
export function preprocessVerdict(): CrossCheck | null { return verdict; }
/** Forget the session verdict (tests). */
export function resetPreprocessVerdict(): void { verdict = null; }

function same(a: ArrayLike<number>, b: ArrayLike<number>): boolean {
  if (a.length !== b.length) return false;
  for (let index = 0; index < a.length; index++) if (a[index] !== b[index]) return false;
  return true;
}
function identical(gpu: GpuStages, cpu: CpuStages): boolean { return same(gpu.gray, cpu.gray) && same(gpu.histogram, cpu.histogram) && gpu.threshold === cpu.threshold && same(gpu.profiles, cpu.profiles) && gpu.skew.index === cpu.skew.index; }
function finish(runtime: 'webgpu' | 'cpu', stages: GpuStages | CpuStages, status: WebGpuStatus, fallbackReason: string | null, crossCheck: CrossCheck | null): PreprocessResult {
  return { runtime, fallbackReason, gray: stages.gray, histogram: stages.histogram, threshold: stages.threshold, skew: { degrees: stages.skew.degrees, confidence: stages.skew.confidence, stride: stages.stride }, timings: stages.timings, status, crossCheck };
}

/** Pre-process one RGBA working image for OCR. Never throws for a GPU problem: the CPU path is the fallback. */
export async function preprocessForOcr(rgba: Uint8Array | Uint8ClampedArray, width: number, height: number, options: PreprocessOptions = {}): Promise<PreprocessResult> {
  const mode = options.mode ?? 'auto', signal = options.signal, log = options.log ?? ((line: string) => console.info(line));
  if (!Number.isInteger(width) || !Number.isInteger(height) || width <= 0 || height <= 0) throw new RangeError('Working image dimensions must be positive integers.');
  if (rgba.length < width * height * 4) throw new RangeError('RGBA buffer is shorter than width × height × 4.');
  signal?.throwIfAborted();
  const wait = gpuWaitOptions({ signal, timeoutMs: options.timeoutMs });
  const status = await detectWebGpu({ workingPixels: width * height, ...wait });
  signal?.throwIfAborted();
  const cpuOnly = (reason: string | null, crossCheck: CrossCheck | null = verdict) => finish('cpu', preprocessCpu(rgba, width, height, options.skew), status, reason, crossCheck);
  if (mode === 'cpu') return cpuOnly(null);
  if (status.state !== 'available') return cpuOnly(status.reason ?? 'WebGPU unavailable');
  if (verdict && !verdict.identical) return cpuOnly(verdict.reason);
  if (mode === 'auto' && verdict?.decision === 'cpu') return cpuOnly(verdict.reason);
  const acquired = await acquireWebGpuDevice({ workingPixels: width * height, ...wait });
  signal?.throwIfAborted();
  if (!acquired) return cpuOnly(webGpuFailureReason() ?? 'WebGPU device could not be created');
  let gpu: GpuStages;
  try { gpu = await preprocessGpu(acquired.device, rgba, width, height, options.skew, signal, wait); } catch (error) {
    if (signal?.aborted) throw signal.reason;
    return cpuOnly(`WebGPU stage failed: ${(error as Error).message}`);
  }
  signal?.throwIfAborted();
  const liveStatus = { ...acquired.status, statusLine: statusLineFor(acquired.status) };
  if (!verdict && options.crossCheck !== false) {
    const cpu = preprocessCpu(rgba, width, height, options.skew), match = identical(gpu, cpu);
    const slow = gpu.timings.totalMs > cpu.timings.totalMs * SLOWER_FACTOR;
    verdict = {
      identical: match, gpuMs: gpu.timings.totalMs, cpuMs: cpu.timings.totalMs,
      decision: match && (!slow || mode === 'webgpu') ? 'webgpu' : 'cpu',
      reason: !match ? 'WebGPU results differed from the CPU reference; the CPU path is used for this session' : slow ? `WebGPU pre-processing took ${gpu.timings.totalMs.toFixed(1)} ms against ${cpu.timings.totalMs.toFixed(1)} ms on the CPU${status.adapter?.fallback ? ' (software fallback adapter)' : ''}; the CPU path is used for this session` : `WebGPU pre-processing ${gpu.timings.totalMs.toFixed(1)} ms, CPU ${cpu.timings.totalMs.toFixed(1)} ms, results identical`,
    };
    log(`[ocr] WebGPU pre-processing check on a ${width}×${height} image: ${verdict.reason}. Adapter: ${[status.adapter?.vendor, status.adapter?.architecture, status.adapter?.device].filter(Boolean).join(' ') || 'unknown'}${status.adapter?.fallback ? ' (fallback)' : ''}.`);
    if (!match) return finish('cpu', cpu, liveStatus, verdict.reason, verdict);
    if (verdict.decision === 'cpu' && mode === 'auto') return finish('cpu', cpu, liveStatus, verdict.reason, verdict);
  }
  return finish('webgpu', gpu, liveStatus, null, verdict);
}

/** One plain sentence describing the acceleration state for an accessible status line. */
export async function accelerationStatusLine(): Promise<string> {
  const status = await detectWebGpu();
  if (status.state === 'available' && verdict?.decision === 'cpu') return `WebGPU available but not used: ${verdict.reason}. Text recognition runs on the CPU (WASM). Nothing leaves this device.`;
  return status.statusLine;
}
