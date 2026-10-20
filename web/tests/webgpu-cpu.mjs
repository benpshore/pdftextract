/** Node regression for the CPU fallback kernels, the shared maths and the detection path without
 * WebGPU. Runs from `web/`: `node tests/webgpu-cpu.mjs`. No network, no browser. */
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { pathToFileURL } from 'node:url';
import { web, transpileSources, tiltedLines, noise, decodePng, maxAbsDiff } from './webgpu-helpers.mjs';
const temporary = await mkdtemp(join(tmpdir(), 'tpe-webgpu-cpu-'));
const checks = [];
const check = (condition, message) => { assert(condition, message); checks.push(message); };
try {
  await transpileSources(temporary);
  const kernels = await import(pathToFileURL(join(temporary, 'lib/webgpu/index.js')));
  const { grayscaleCpu, histogramCpu, otsuThreshold, profilesCpu, scoreProfiles, skewTable, sampleStride, preprocessCpu, preprocessForOcr, detectWebGpu, accelerationStatusLine, limitShortfall, luma, DEFAULT_SKEW } = kernels;

  // Detection without WebGPU (Node has navigator but no navigator.gpu).
  const status = await detectWebGpu();
  check(status.state === 'unavailable' && typeof status.reason === 'string' && status.reason.length > 0, `Detection reports unavailable with a reason: ${status.reason}`);
  check(status.statusLine.includes('CPU') && status.statusLine.includes('Nothing leaves this device'), 'Status line names the CPU path and the on-device guarantee');
  check((await accelerationStatusLine()) === status.statusLine, 'Acceleration status line matches detection when no verdict exists');
  check(limitShortfall({ maxBufferSize: 1 << 20, maxStorageBufferBindingSize: 1 << 20, maxComputeWorkgroupsPerDimension: 65535, maxComputeInvocationsPerWorkgroup: 256, maxComputeWorkgroupSizeX: 256, maxComputeWorkgroupStorageSize: 16384 }, 4_000_000)?.includes('maxStorageBufferBindingSize'), 'Limit check names the first limit that is too small');
  check(limitShortfall({ maxBufferSize: 256 << 20, maxStorageBufferBindingSize: 128 << 20, maxComputeWorkgroupsPerDimension: 65535, maxComputeInvocationsPerWorkgroup: 256, maxComputeWorkgroupSizeX: 256, maxComputeWorkgroupStorageSize: 16384 }, 4_000_000) === null, 'Default WebGPU limits cover the 4,000,000-pixel working image');

  // Grayscale and histogram on colour noise.
  const width = 613, height = 211, pixels = width * height, rgba = noise(pixels * 4, 42);
  const gray = grayscaleCpu(rgba, pixels);
  let formulaOk = true;
  for (let index = 0; index < pixels; index++) if (gray[index] !== luma(rgba[index * 4], rgba[index * 4 + 1], rgba[index * 4 + 2])) { formulaOk = false; break; }
  check(formulaOk && gray.length === pixels, 'Grayscale applies the BT.601 integer formula to every pixel');
  check(luma(255, 255, 255) === 255 && luma(0, 0, 0) === 0 && luma(255, 0, 0) === 76 && luma(0, 255, 0) === 150 && luma(0, 0, 255) === 29, 'Luma weights match the documented integer coefficients');
  const histogram = histogramCpu(gray);
  check(histogram.length === 256 && histogram.reduce((sum, bin) => sum + bin, 0) === pixels, 'Histogram has 256 bins that sum to the pixel count');
  const bimodal = new Uint32Array(256); bimodal[40] = 1000; bimodal[200] = 3000;
  const threshold = otsuThreshold(bimodal);
  check(threshold >= 40 && threshold < 200, `Otsu separates a bimodal histogram (threshold ${threshold})`);
  check(otsuThreshold(new Uint32Array(256)) === 0, 'Otsu of an empty histogram is 0, never all-dark');
  const flat = new Uint32Array(256); flat[128] = 500;
  check(otsuThreshold(flat) === 0, 'Otsu of a flat image is 0');

  // Skew table and stride.
  const table = skewTable();
  check(table.degrees.length === 41 && table.degrees[table.zeroIndex] === 0 && table.sinQ[table.zeroIndex] === 0 && table.cosQ[table.zeroIndex] === 65536, 'Default skew table spans ±5° in 0.25° steps with exact zero');
  check(table.sinQ[0] === -table.sinQ[40] && table.cosQ[0] === table.cosQ[40], 'Skew table is symmetric');
  check(sampleStride(2000, 2000, DEFAULT_SKEW.maxSamples) === 2 && sampleStride(100, 100, 1_000_000) === 1, 'Sampling stride bounds the projection work');
  assert.throws(() => skewTable({ maxDegrees: 0 }), RangeError);

  // Skew estimation recovers known tilts on synthetic text-like pages.
  for (const tilt of [0, 1.5, -2.25, 3]) {
    const page = tiltedLines(900, 500, tilt);
    const stages = preprocessCpu(page, 900, 500);
    check(Math.abs(stages.skew.degrees - tilt) <= 0.25, `Projection-profile skew recovers ${tilt}° (estimated ${stages.skew.degrees}°, confidence ${stages.skew.confidence.toFixed(2)})`);
    if (tilt !== 0) check(stages.skew.confidence > 1.05, `Tilted page has a confident estimate (${stages.skew.confidence.toFixed(2)})`);
    check(stages.threshold >= 20 && stages.threshold < 245, `Otsu threshold lies between the ink range and the paper level (${stages.threshold})`);
  }
  const straight = preprocessCpu(tiltedLines(900, 500, 0), 900, 500);
  check(straight.skew.degrees === 0 && straight.skew.confidence === 1, 'A straight page scores best at exactly zero');

  // Profiles are deterministic and only count dark samples.
  const page = tiltedLines(300, 200, 1);
  const grayPage = grayscaleCpu(page, 300 * 200), t = otsuThreshold(histogramCpu(grayPage));
  const profiles = profilesCpu(grayPage, 300, 200, t, table, 1), again = profilesCpu(grayPage, 300, 200, t, table, 1);
  check(maxAbsDiff(profiles, again) === 0 && profiles.length === 41 * 200, 'Profiles are deterministic with shape angles × height');
  let dark = 0; for (const value of grayPage) if (value <= t) dark++;
  check(profiles.subarray(table.zeroIndex * 200, (table.zeroIndex + 1) * 200).reduce((sum, bin) => sum + bin, 0) === dark, 'Zero-degree profile counts every dark pixel exactly once');
  check(scoreProfiles(profiles, 200, table).index === scoreProfiles(again, 200, table).index, 'Scoring is deterministic');

  // The committed OCR fixture through the CPU path.
  const fixture = await decodePng(new Uint8Array(await readFile(join(web, 'scripts/fixtures/ocr-still.png'))));
  const real = preprocessCpu(fixture.rgba, fixture.width, fixture.height);
  check(fixture.width === 1500 && fixture.height === 340 && real.gray.length === 1500 * 340, 'Fixture PNG decodes to the documented dimensions');
  check(Math.abs(real.skew.degrees) <= 0.25 && real.threshold > 0, `Fixture text is straight (skew ${real.skew.degrees}°, threshold ${real.threshold}, ${real.timings.totalMs.toFixed(1)} ms on the CPU)`);

  // The public entry point falls back to the CPU and says why.
  const result = await preprocessForOcr(fixture.rgba, fixture.width, fixture.height, { mode: 'auto', log: () => {} });
  check(result.runtime === 'cpu' && result.fallbackReason === status.reason && result.crossCheck === null, 'preprocessForOcr falls back to the CPU with the detection reason');
  check(maxAbsDiff(result.gray, real.gray) === 0 && result.threshold === real.threshold && result.skew.degrees === real.skew.degrees, 'Fallback result equals the reference CPU stages');
  const forced = await preprocessForOcr(fixture.rgba, fixture.width, fixture.height, { mode: 'cpu' });
  check(forced.runtime === 'cpu' && forced.fallbackReason === null, 'mode cpu runs the CPU path without a fallback reason');
  await assert.rejects(preprocessForOcr(fixture.rgba, 0, 1), RangeError);
  const controller = new AbortController(); controller.abort();
  await assert.rejects(preprocessForOcr(fixture.rgba, fixture.width, fixture.height, { signal: controller.signal }), error => error.name === 'AbortError');
  check(true, 'Invalid dimensions and pre-aborted signals reject before any work');

  // image-ocr.ts exports the status line and the flag without touching the DOM.
  const ocr = await import(pathToFileURL(join(temporary, 'lib/image-ocr.js')));
  check(ocr.getOcrAcceleration() === 'auto', 'Acceleration defaults to auto (on when available)');
  ocr.setOcrAcceleration('off'); check((await ocr.ocrAccelerationStatus()).includes('off'), 'Status line reflects the off setting');
  ocr.setOcrAcceleration('cpu'); check((await ocr.ocrAccelerationStatus()).includes('CPU by setting'), 'Status line reflects the cpu setting');
  ocr.setOcrAcceleration('auto'); check((await ocr.ocrAccelerationStatus()) === status.statusLine, 'Status line in auto mode is the detection status line');
  console.log(JSON.stringify({ environment: 'Node, no WebGPU: CPU fallback, shared maths and detection only', checks }, null, 2));
} finally { await rm(temporary, { recursive: true, force: true }); }
