/** WebGPU capability detection and device ownership for the OCR pre-processing path.
 * Detection never throws: every failure becomes an explicit `unavailable` status with a
 * reason the UI can show. Safari 26 / iOS 26 and Chrome differences are handled here. */

export type WebGpuAdapterSummary = { vendor: string; architecture: string; device: string; description: string; fallback: boolean };
export type WebGpuLimits = { maxBufferSize: number; maxStorageBufferBindingSize: number; maxComputeWorkgroupsPerDimension: number; maxComputeInvocationsPerWorkgroup: number; maxComputeWorkgroupSizeX: number; maxComputeWorkgroupStorageSize: number };
export type BrowserHints = { engine: 'webkit' | 'blink' | 'gecko' | 'unknown'; ios: boolean; safari: boolean; chrome: boolean; worker: boolean };
export type WebGpuStatus = {
  /** `available`: a device can be created and the limits cover the OCR working image. */
  state: 'available' | 'unavailable' | 'lost';
  reason: string | null;
  secureContext: boolean;
  browser: BrowserHints;
  adapter: WebGpuAdapterSummary | null;
  features: string[];
  limits: WebGpuLimits | null;
  /** Human-readable notes (limit checks, fallback adapter, Safari specifics). */
  notes: string[];
  /** One plain sentence for an accessible status line; no colour or icon carries meaning. */
  statusLine: string;
};
export type DetectOptions = { workingPixels?: number; powerPreference?: 'low-power' | 'high-performance' };

const OCR_WORKING_PIXELS = 4_000_000;
const WORKGROUP = 256;

function hints(): BrowserHints {
  const agent = typeof navigator === 'object' && typeof navigator.userAgent === 'string' ? navigator.userAgent : '';
  const worker = typeof WorkerGlobalScope !== 'undefined';
  const chrome = /Chrome\/|CriOS\//.test(agent), gecko = /Gecko\/\d|Firefox\//.test(agent) && !chrome;
  const webkitFamily = /AppleWebKit\//.test(agent), ios = /iPhone|iPad|iPod/.test(agent) || webkitFamily && typeof navigator === 'object' && navigator.maxTouchPoints > 1 && /Macintosh/.test(agent);
  // Every iOS browser uses WebKit; Chrome on iOS reports CriOS but still runs WebKit's WebGPU.
  const engine = ios || webkitFamily && !chrome && /Safari\//.test(agent) ? 'webkit' : chrome ? 'blink' : gecko ? 'gecko' : webkitFamily ? 'webkit' : 'unknown';
  return { engine, ios, safari: engine === 'webkit' && !chrome, chrome, worker };
}
function limitsOf(adapter: GPUAdapter): WebGpuLimits {
  const limits = adapter.limits;
  return { maxBufferSize: limits.maxBufferSize, maxStorageBufferBindingSize: limits.maxStorageBufferBindingSize, maxComputeWorkgroupsPerDimension: limits.maxComputeWorkgroupsPerDimension, maxComputeInvocationsPerWorkgroup: limits.maxComputeInvocationsPerWorkgroup, maxComputeWorkgroupSizeX: limits.maxComputeWorkgroupSizeX, maxComputeWorkgroupStorageSize: limits.maxComputeWorkgroupStorageSize };
}
async function adapterSummary(adapter: GPUAdapter): Promise<WebGpuAdapterSummary> {
  // Safari 26 exposes `adapter.info`; older Chrome shipped the now-removed `requestAdapterInfo()`.
  let info: GPUAdapterInfo | undefined = adapter.info;
  if (!info && typeof adapter.requestAdapterInfo === 'function') info = await adapter.requestAdapterInfo().catch(() => undefined);
  return { vendor: info?.vendor || '', architecture: info?.architecture || '', device: info?.device || '', description: info?.description || '', fallback: Boolean(info?.isFallbackAdapter ?? adapter.isFallbackAdapter ?? false) };
}
/** Bytes and dispatch sizes the kernels need for a working image of `pixels` pixels. */
export function requiredLimits(pixels: number): { storageBytes: number; workgroups: number } {
  return { storageBytes: pixels * 4, workgroups: Math.ceil(pixels / WORKGROUP) };
}
/** Why the adapter limits cannot process `pixels` pixels, or null when they can. */
export function limitShortfall(limits: WebGpuLimits, pixels: number): string | null {
  const need = requiredLimits(pixels);
  if (limits.maxStorageBufferBindingSize < need.storageBytes) return `maxStorageBufferBindingSize ${limits.maxStorageBufferBindingSize} < ${need.storageBytes} bytes needed for ${pixels} pixels`;
  if (limits.maxBufferSize < need.storageBytes) return `maxBufferSize ${limits.maxBufferSize} < ${need.storageBytes} bytes`;
  if (limits.maxComputeWorkgroupsPerDimension < need.workgroups) return `maxComputeWorkgroupsPerDimension ${limits.maxComputeWorkgroupsPerDimension} < ${need.workgroups}`;
  if (limits.maxComputeInvocationsPerWorkgroup < WORKGROUP || limits.maxComputeWorkgroupSizeX < WORKGROUP) return `compute workgroup size ${WORKGROUP} unsupported`;
  if (limits.maxComputeWorkgroupStorageSize < 256 * 4) return 'workgroup storage below 1 KiB';
  return null;
}
export function statusLineFor(status: Omit<WebGpuStatus, 'statusLine'>): string {
  const where = status.adapter ? `${[status.adapter.vendor, status.adapter.architecture, status.adapter.device, status.adapter.description].filter(Boolean).join(' ') || 'unnamed adapter'}${status.adapter.fallback ? ', software fallback adapter' : ''}` : '';
  if (status.state === 'available') return `WebGPU available (${where}): image pre-processing for OCR runs on the GPU; text recognition runs on the CPU (WASM). Nothing leaves this device.`;
  if (status.state === 'lost') return `WebGPU device was lost${status.reason ? ` (${status.reason})` : ''}: image pre-processing for OCR runs on the CPU until the page reloads.`;
  return `WebGPU unavailable${status.reason ? ` (${status.reason})` : ''}: image pre-processing and OCR run on the CPU. Nothing leaves this device.`;
}

let cached: { device: GPUDevice; status: WebGpuStatus } | null = null, lostReason: string | null = null, deviceFailure: string | null = null, acquiring: Promise<{ device: GPUDevice; status: WebGpuStatus } | null> | null = null;

/** Probe `navigator.gpu`, request an adapter (then a fallback adapter), check limits. Never throws. */
export async function detectWebGpu(options: DetectOptions = {}): Promise<WebGpuStatus> {
  const pixels = options.workingPixels ?? OCR_WORKING_PIXELS, browser = hints();
  const secureContext = typeof isSecureContext === 'boolean' ? isSecureContext : false;
  const base = { secureContext, browser, adapter: null, features: [], limits: null, notes: [] as string[] };
  const unavailable = (reason: string, extra: Partial<WebGpuStatus> = {}): WebGpuStatus => { const status = { ...base, ...extra, state: 'unavailable' as const, reason }; return { ...status, statusLine: statusLineFor(status) }; };
  if (cached) return cached.status;
  if (deviceFailure) return unavailable(deviceFailure);
  if (lostReason) { const status = { ...base, state: 'lost' as const, reason: lostReason }; return { ...status, statusLine: statusLineFor(status) }; }
  const gpu = typeof navigator === 'object' ? navigator.gpu : undefined;
  if (!gpu) return unavailable(typeof isSecureContext === 'boolean' && !isSecureContext ? 'navigator.gpu requires a secure context (https or localhost)' : 'navigator.gpu is not exposed by this browser');
  let adapter: GPUAdapter | null = null;
  try { adapter = await gpu.requestAdapter(options.powerPreference ? { powerPreference: options.powerPreference } : {}); } catch (error) { base.notes.push(`requestAdapter threw: ${(error as Error).message}`); }
  if (!adapter) {
    // Chrome returns null on blocklisted GPUs; a fallback adapter (SwiftShader) may still exist.
    try { adapter = await gpu.requestAdapter({ forceFallbackAdapter: true }); } catch (error) { base.notes.push(`fallback requestAdapter threw: ${(error as Error).message}`); }
    if (adapter) base.notes.push('only a fallback (software) adapter is available');
  }
  if (!adapter) return unavailable('no WebGPU adapter (GPU blocklisted, driver unsupported, or disabled by browser policy)');
  const summary = await adapterSummary(adapter), limits = limitsOf(adapter), features = [...adapter.features].sort();
  const shortfall = limitShortfall(limits, pixels);
  if (shortfall) return unavailable(`adapter limits too small: ${shortfall}`, { adapter: summary, limits, features });
  if (summary.fallback) base.notes.push('fallback adapter: compute runs in software, so a first-use timing comparison decides whether it is used');
  if (browser.engine === 'webkit') base.notes.push('WebKit (Safari 26 / iOS 26): default limits only, no optional features requested; the device is released when the page is hidden for long');
  const ok = { ...base, state: 'available' as const, reason: null, adapter: summary, limits, features, notes: [...base.notes, `limits cover a ${pixels.toLocaleString('en-US')}-pixel working image`] };
  return { ...ok, statusLine: statusLineFor(ok) };
}

/** Create (once) and share the compute device. Returns null, with the status explaining why, when unavailable. */
export async function acquireWebGpuDevice(options: DetectOptions = {}): Promise<{ device: GPUDevice; status: WebGpuStatus } | null> {
  if (cached) return cached;
  if (acquiring) return acquiring;
  acquiring = (async () => {
    const status = await detectWebGpu(options);
    if (status.state !== 'available') return null;
    const gpu = navigator.gpu!;
    let adapter: GPUAdapter | null = null;
    try { adapter = await gpu.requestAdapter(options.powerPreference ? { powerPreference: options.powerPreference } : {}) ?? await gpu.requestAdapter({ forceFallbackAdapter: true }); } catch { adapter = null; }
    if (!adapter) return null;
    try {
      // Request nothing optional: Safari rejects unknown features/limits and the kernels need only defaults.
      const device = await adapter.requestDevice({ label: 'tpe-ocr-preprocess' });
      device.lost.then(info => { lostReason = `${info.reason}: ${info.message || 'no message'}`; cached = null; }).catch(() => { cached = null; });
      cached = { device, status };
      return cached;
    } catch (error) { deviceFailure = `requestDevice failed: ${(error as Error).message || 'unknown error'}`; return null; }
  })();
  try { return await acquiring; } finally { acquiring = null; }
}
/** Destroy the shared device (tests and page teardown). */
export function releaseWebGpuDevice(): void { try { cached?.device.destroy(); } catch { /* already destroyed */ } cached = null; }
/** Forget a recorded device loss so the next acquisition tries again (tests). */
export function resetWebGpuState(): void { releaseWebGpuDevice(); lostReason = null; deviceFailure = null; }
