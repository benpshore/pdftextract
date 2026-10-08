// One reusable CPU runtime per API instance. Document state is released after
// each job; interruption destroys the worker because synchronous WASM cannot
// cooperatively observe an AbortSignal. No document buffer lives in this pool.
import { RUNTIME_POLICY } from './manifest.js';

let worker, pending, idleTimer, serial = 0, lease = null, snapshot = null;
export const runtimeState = () => ({ resident: !!worker, leased: !!lease, ...snapshot });

export function disposeRuntime(reason) {
  clearTimeout(idleTimer);
  worker?.terminate();
  worker = null;
  snapshot = null;
  if (pending) {
    pending.reject(reason || Object.assign(new Error('PDF runtime released.'), { code: 'RUNTIME_RELEASED' }));
    pending = null;
  }
  lease = null;
}

export function acquireRuntime(jobId, signal, onEvent) {
  clearTimeout(idleTimer);
  if (lease) throw new Error('PDF runtime is already leased.');
  if (!worker) {
    worker = new Worker('/pdf-api/v1/inference-worker.js', { type: 'module' });
    const owner = worker;
    worker.onmessage = ({ data }) => {
      if (worker !== owner) return;
      if (data.event) {
        if (data.jobId === lease?.jobId) lease.onEvent(data.event);
        return;
      }
      if (data.id !== pending?.id) return;
      const current = pending;
      pending = null;
      if (data.error) current.reject(Object.assign(new Error(data.error.message), { code: data.error.code }));
      else current.resolve(data.result);
    };
    worker.onerror = event => { if (worker === owner) disposeRuntime(Object.assign(new Error(event.message || 'Local PDF runtime failed to load.'), { code: 'RUNTIME_MISSING' })); };
    worker.onmessageerror = () => { if (worker === owner) disposeRuntime(Object.assign(new Error('PDF runtime returned an unreadable message.'), { code: 'RUNTIME_MESSAGE' })); };
  }
  lease = { jobId, onEvent };
  const abort = () => disposeRuntime(signal.reason || new DOMException('PDF cancelled.', 'AbortError'));
  signal.addEventListener('abort', abort, { once: true });
  const call = (operation, data = {}, transfer = []) => new Promise((resolve, reject) => {
    try {
      signal.throwIfAborted();
      if (!worker || lease?.jobId !== jobId) throw new Error('PDF runtime lease ended.');
      if (pending) throw new Error('Concurrent PDF runtime commands are unsupported.');
      const id = ++serial;
      pending = { id, resolve, reject };
      worker.postMessage({ id, jobId, operation, ...data }, transfer);
    } catch (error) { pending = null; reject(error); }
  });
  return {
    call,
    async release(healthy) {
      try {
        if (healthy && worker && !signal.aborted) {
          snapshot = await call('end');
          if (snapshot.rustWasmCapacityBytes > RUNTIME_POLICY.maxRetainedRustWasmBytes) disposeRuntime();
          else idleTimer = setTimeout(() => { if (!lease) disposeRuntime(); }, RUNTIME_POLICY.idleMs);
        } else disposeRuntime();
      } finally {
        signal.removeEventListener('abort', abort);
        lease = null;
      }
    },
  };
}
