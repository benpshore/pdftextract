/** Node lifecycle regressions; browser runs the identical fault body before hardware gates. */
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { transpileSources } from './webgpu-helpers.mjs';
import { runWebGpuFaults } from './webgpu-faults.mjs';
const temporary = await mkdtemp(join(tmpdir(), 'tpe-webgpu-cancellation-'));
try {
  await transpileSources(temporary);
  const kernels = await import(pathToFileURL(join(temporary, 'lib/webgpu/index.js')));
  console.log(JSON.stringify({ environment: 'Node, synthetic WebGPU waits', ...await runWebGpuFaults(kernels, process.env.WEBGPU_FAULT_STAGE ? { stages: [process.env.WEBGPU_FAULT_STAGE], actions: ['abort'], extras: false } : {}) }, null, 2));
} finally { await rm(temporary, { recursive: true, force: true }); }
