/** Shared Node/Chromium fault injection. These are lifecycle tests, not GPU-quality evidence. */
export async function runWebGpuFaults(kernels, settings = {}) {
  const checks = [], timings = [];
  const check = (condition, message) => { if (!condition) throw new Error(message); checks.push(message); };
  const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
  const tick = () => new Promise(resolve => setTimeout(resolve, 0));
  const watchdog = async (promise, label) => {
    let timer;
    try { return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`${label} stayed pending after watchdog`)), 1000); })]); }
    finally { clearTimeout(timer); }
  };
  const saved = new Map();
  const override = (object, name, value) => {
    if (!saved.has(object)) saved.set(object, new Map());
    const properties = saved.get(object);
    if (!properties.has(name)) properties.set(name, Object.getOwnPropertyDescriptor(object, name));
    Object.defineProperty(object, name, { configurable: true, value });
  };
  const limits = { maxBufferSize: 256 << 20, maxStorageBufferBindingSize: 128 << 20, maxComputeWorkgroupsPerDimension: 65535, maxComputeInvocationsPerWorkgroup: 256, maxComputeWorkgroupSizeX: 256, maxComputeWorkgroupStorageSize: 16384 };
  const rgba = new Uint8Array(4 * 4 * 4).fill(200);
  const stages = ['adapter', 'fallback-adapter', 'acquisition-adapter', 'adapter-info', 'device', 'pipeline', 'map-1', 'map-2', 'map-3', 'error-scope'];

  function fixture(stage) {
    const entered = deferred(), pending = deferred(), lost = deferred();
    const buffers = []; let adapters = 0, maps = 0, scopes = 0, destroyed = 0, deviceRequests = 0;
    const stall = () => { entered.resolve(); return pending.promise; };
    const device = {
      limits, features: new Set(), lost: lost.promise,
      destroy() { destroyed++; lost.resolve({ reason: 'destroyed', message: 'test teardown' }); },
      queue: { submit() {} }, pushErrorScope() { scopes++; },
      popErrorScope() { scopes--; return stage === 'error-scope' ? stall() : Promise.resolve(null); },
      createShaderModule() { return {}; },
      createComputePipelineAsync() { return stage === 'pipeline' ? stall() : Promise.resolve({ getBindGroupLayout() { return {}; } }); },
      createBindGroup() { return {}; },
      createCommandEncoder() { return { copyBufferToBuffer() {}, finish() { return {}; }, beginComputePass() { return { setPipeline() {}, setBindGroup() {}, dispatchWorkgroups() {}, end() {} }; } }; },
      createBuffer({ size, label }) {
        const buffer = { label, destroyed: 0, lateReads: 0, memory: new ArrayBuffer(size),
          getMappedRange() { if (this.destroyed) this.lateReads++; return this.memory; },
          unmap() {}, destroy() { this.destroyed++; },
          mapAsync() { maps++; return stage === `map-${maps}` ? stall() : Promise.resolve(); },
        };
        buffers.push(buffer); return buffer;
      },
    };
    const adapter = { info: stage === 'adapter-info' ? undefined : {}, limits, features: new Set(),
      requestAdapterInfo: stall,
      requestDevice() { deviceRequests++; return stage === 'device' ? stall() : Promise.resolve(device); },
    };
    const gpu = { requestAdapter() {
      adapters++;
      if (stage === 'adapter' || stage === 'fallback-adapter' && adapters === 2 || stage === 'acquisition-adapter' && adapters === 3) return stall();
      if (stage === 'fallback-adapter' && adapters === 1) return Promise.resolve(null);
      return Promise.resolve(adapter);
    } };
    override(navigator, 'gpu', gpu);
    return { entered, pending, device, adapter, buffers, lost, counts: () => ({ scopes, destroyed, deviceRequests, maps, adapters }) };
  }

  try {
    override(globalThis, 'GPUBufferUsage', { MAP_READ: 1, COPY_DST: 2, STORAGE: 4, COPY_SRC: 8, UNIFORM: 16 });
    override(globalThis, 'GPUMapMode', { READ: 1 });
    for (const stage of settings.stages ?? stages) {
      for (const action of settings.actions ?? ['abort', 'deadline']) {
        kernels.resetWebGpuState(); kernels.resetPreprocessVerdict();
        const fake = fixture(stage), controller = new AbortController();
        // Attach rejection observation before triggering abort (also covers non-Error reasons).
        const operation = kernels.preprocessForOcr(rgba, 4, 4, { mode: 'webgpu', signal: controller.signal, timeoutMs: action === 'deadline' ? 60 : 5000, crossCheck: false, log() {} })
          .then(value => ({ value }), error => ({ error }));
        await watchdog(fake.entered.promise, `${stage} entry`);
        const start = performance.now(), reason = { stage, action };
        if (action === 'abort') controller.abort(reason);
        const outcome = await watchdog(operation, `${stage} ${action}`), elapsedMs = performance.now() - start;
        timings.push({ stage, action, elapsedMs });
        if (action === 'abort') check(outcome.error === reason && elapsedMs < 250, `${stage}: abort returns promptly with the exact signal reason`);
        else check(outcome.value?.runtime === 'cpu' && /deadline exceeded/.test(outcome.value.fallbackReason) && elapsedMs < 500, `${stage}: deadline returns promptly with explicit CPU fallback`);
        check(fake.buffers.every(buffer => buffer.destroyed === 1) && fake.counts().scopes === 0, `${stage}/${action}: every allocated buffer destroyed once and error scopes popped`);
        // Resolve abandoned work: no continuation may allocate/read or publish a late device.
        const before = fake.buffers.length;
        fake.pending.resolve(stage === 'device' ? fake.device : stage.includes('adapter') && stage !== 'adapter-info' ? fake.adapter : stage === 'pipeline' ? { getBindGroupLayout() { return {}; } } : null);
        await tick(); await tick();
        check(fake.buffers.length === before && fake.buffers.every(buffer => buffer.lateReads === 0), `${stage}/${action}: late settlement cannot resume buffer work`);
        if (stage === 'device') check(fake.counts().destroyed === 1, `device/${action}: late device destroyed instead of cached`);
        kernels.resetWebGpuState(); await tick();
        if (fake.counts().deviceRequests) check(fake.counts().destroyed === 1, `${stage}/${action}: acquired session device released exactly once`);
      }
    }

    if (settings.extras === false) return { checks, timings, qualityEvidence: false };

    // Rejected abandoned promises must remain observed in both Node and Chromium.
    for (const stage of ['adapter', 'device', 'pipeline', 'map-1', 'error-scope']) {
      kernels.resetWebGpuState();
      const fake = fixture(stage), controller = new AbortController();
      const result = kernels.preprocessForOcr(rgba, 4, 4, { signal: controller.signal, crossCheck: false }).catch(error => error);
      await watchdog(fake.entered.promise, stage); controller.abort(); await watchdog(result, stage);
      fake.pending.reject(new Error(`late ${stage} rejection`));
      await tick(); await tick();
      kernels.resetWebGpuState();
      check(true, `${stage}: late rejection observed without resuming work`);
    }

    // Independent pending acquisitions: abort one caller, let the other finish and reuse.
    kernels.resetWebGpuState();
    const fake = fixture('device'), requests = [], bothEntered = deferred();
    fake.adapter.requestDevice = () => { const request = deferred(); requests.push(request); if (requests.length === 2) bothEntered.resolve(); return request.promise; };
    const controller = new AbortController();
    const first = kernels.acquireWebGpuDevice({ signal: controller.signal }).catch(error => error);
    const second = kernels.acquireWebGpuDevice();
    await watchdog(bothEntered.promise, 'concurrent acquisition');
    controller.abort(); check((await watchdog(first, 'first acquisition')).name === 'AbortError', 'Concurrent acquisition: one caller cancels promptly');
    requests[1].resolve(fake.device);
    const acquired = await watchdog(second, 'second acquisition');
    const late = { destroyCalls: 0, destroy() { this.destroyCalls++; } }; requests[0].resolve(late); await tick();
    check(acquired.device === fake.device && late.destroyCalls === 1 && fake.counts().destroyed === 0, 'Concurrent acquisition: surviving caller owns live device and canceled caller destroys its late device');
    check((await kernels.acquireWebGpuDevice()).device === fake.device, 'Concurrent acquisition: successful device remains reusable');
    kernels.resetWebGpuState(); await tick();

    // Two successful requests converge on one cached device; destroy the duplicate.
    const duplicate = fixture('device'), duplicateRequests = [], duplicateEntered = deferred();
    duplicate.adapter.requestDevice = () => { const request = deferred(); duplicateRequests.push(request); if (duplicateRequests.length === 2) duplicateEntered.resolve(); return request.promise; };
    const one = kernels.acquireWebGpuDevice(), two = kernels.acquireWebGpuDevice();
    await watchdog(duplicateEntered.promise, 'duplicate acquisition');
    duplicateRequests[0].resolve(duplicate.device); const kept = await watchdog(one, 'kept device');
    const redundant = { destroyCalls: 0, destroy() { this.destroyCalls++; } }; duplicateRequests[1].resolve(redundant);
    check((await watchdog(two, 'duplicate device')).device === kept.device && redundant.destroyCalls === 1, 'Concurrent successes reuse one cached device and destroy the duplicate');
    kernels.resetWebGpuState();

    // Teardown during requestDevice must prevent the old request from repopulating cache.
    const teardown = fixture('device'), old = kernels.acquireWebGpuDevice();
    await watchdog(teardown.entered.promise, 'teardown acquisition'); kernels.resetWebGpuState();
    teardown.pending.resolve(teardown.device);
    check(await watchdog(old, 'teardown acquisition') === null && teardown.counts().destroyed === 1, 'Teardown invalidates in-flight acquisition and destroys the resulting device');
    // A cached old device can report loss after reset and a new acquisition.
    const previous = fixture('ready'); previous.device.destroy = () => {};
    await kernels.acquireWebGpuDevice(); kernels.resetWebGpuState();
    const fresh = fixture('ready');
    const live = await kernels.acquireWebGpuDevice(); previous.lost.resolve({ reason: 'unknown', message: 'old loss' }); await tick();
    check((await kernels.acquireWebGpuDevice()).device === live.device && fresh.counts().destroyed === 0, 'Old device loss cannot clear a newer cached device');
    kernels.resetWebGpuState();

    // All adapter probes, acquisition and GPU work must share one total budget.
    const budget = fixture('device');
    navigator.gpu.requestAdapter = () => new Promise(resolve => setTimeout(() => resolve(budget.adapter), 25));
    const started = performance.now();
    const bounded = await watchdog(kernels.preprocessForOcr(rgba, 4, 4, { timeoutMs: 130, crossCheck: false }), 'total deadline');
    check(bounded.runtime === 'cpu' && /requestDevice.*deadline exceeded/.test(bounded.fallbackReason) && performance.now() - started < 190, 'Adapter discovery, acquisition and device wait share one total deadline');
    budget.pending.resolve(budget.device); await tick();
    check(budget.counts().destroyed === 1, 'Total deadline: late device destroyed');
    kernels.resetWebGpuState();

    // A synchronous allocation failure must also release allocations already acquired.
    const allocation = fixture('ready'), createBuffer = allocation.device.createBuffer;
    allocation.device.createBuffer = descriptor => { if (allocation.buffers.length === 2) throw new Error('injected allocation failure'); return createBuffer(descriptor); };
    const failure = await kernels.preprocessForOcr(rgba, 4, 4, { crossCheck: false });
    check(failure.runtime === 'cpu' && /injected allocation failure/.test(failure.fallbackReason) && allocation.buffers.length === 2 && allocation.buffers.every(buffer => buffer.destroyed === 1), 'Mid-allocation failure releases each earlier buffer exactly once');
    return { checks, timings, qualityEvidence: false };
  } finally {
    kernels.resetWebGpuState(); kernels.resetPreprocessVerdict();
    for (const [object, properties] of saved) for (const [name, descriptor] of properties) {
      if (descriptor) Object.defineProperty(object, name, descriptor); else delete object[name];
    }
  }
}
