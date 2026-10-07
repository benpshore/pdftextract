/** WebGPU promises have no native cancellation. Bound our wait, observe late rejections,
 * and dispose resources delivered after the caller has stopped waiting. */
export type GpuWaitOptions = { signal?: AbortSignal; deadline?: number; timeoutMs?: number };
export const GPU_WAIT_TIMEOUT_MS = 10_000;
const now = () => performance.now();

export class WebGpuTimeoutError extends Error {
  constructor(stage: string) { super(`${stage}: WebGPU deadline exceeded`); this.name = 'WebGpuTimeoutError'; }
}

/** Nested stages reuse one deadline; a caller may shorten, but cannot disable, the budget. */
export function gpuWaitOptions(options: GpuWaitOptions = {}): GpuWaitOptions {
  const timeout = options.timeoutMs ?? GPU_WAIT_TIMEOUT_MS;
  if (!Number.isFinite(timeout) || timeout <= 0) throw new RangeError('WebGPU timeout must be a positive finite number.');
  return { signal: options.signal, deadline: Math.min(options.deadline ?? Infinity, now() + Math.min(timeout, GPU_WAIT_TIMEOUT_MS)) };
}

export function checkGpuWait(options: GpuWaitOptions, stage: string): void {
  options.signal?.throwIfAborted();
  if (now() >= options.deadline!) throw new WebGpuTimeoutError(stage);
}

export function waitForGpu<T>(start: () => Promise<T>, options: GpuWaitOptions, stage: string, disposeLate?: (value: T) => void): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    let finished = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const cleanup = () => { clearTimeout(timer); options.signal?.removeEventListener('abort', abort); };
    const fail = (error: unknown) => { if (!finished) { finished = true; cleanup(); reject(error); } };
    const abort = () => fail(options.signal!.reason);
    try {
      checkGpuWait(options, stage);
      options.signal?.addEventListener('abort', abort, { once: true });
      timer = setTimeout(() => fail(new WebGpuTimeoutError(stage)), Math.max(0, options.deadline! - now()));
      const pending = start();
      pending.then(value => {
        // Check again: a late resolution must not beat an expired timer in the task queue.
        try { checkGpuWait(options, stage); } catch (error) { fail(error); }
        if (finished) { try { disposeLate?.(value); } catch { /* best-effort teardown */ } return; }
        finished = true; cleanup(); resolve(value);
      }, fail);
    } catch (error) { fail(error); }
  });
}
