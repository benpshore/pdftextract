#!/usr/bin/env node
// Diagnostic only: upstream Linux x64 CLI, pinned local models, benign fixture.
// Does not enable TPE's supervised Docling backend or change any Cargo pin.
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { chmod, mkdir, mkdtemp, open, readFile, rename, rm, stat } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { performance } from 'node:perf_hooks';
import { fileURLToPath } from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const opts = {};
for (let i = 0; i < args.length; i += 2) {
  if (!['--cache', '--output', '--timeout-ms'].includes(args[i]) || !args[i + 1]) {
    throw new Error('usage: node scripts/docling_cpu_probe.mjs --cache DIR --output FILE [--timeout-ms 65000]');
  }
  opts[args[i]] = args[i + 1];
}
if (!opts['--cache'] || !opts['--output']) throw new Error('--cache and --output are required');
if (process.platform !== 'linux' || process.arch !== 'x64') throw new Error('This pinned probe supports Linux x64 only');
const cache = resolve(opts['--cache']);
const output = resolve(opts['--output']);
const timeoutMs = Number(opts['--timeout-ms'] || 65000);
if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 65000) throw new Error('timeout must be 1..65000 ms');
const pins = JSON.parse(await readFile(join(repo, 'docs/validation/docling-runtime-pins.json'), 'utf8'));
const fixture = join(repo, pins.fixture.path);
let activeChild;
let activeDownload;
let interrupted = false;
function killGroup(signal) {
  if (activeChild?.pid) {
    try { process.kill(-activeChild.pid, signal); } catch (e) { if (e.code !== 'ESRCH') throw e; }
  }
}
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => {
  interrupted = true;
  killGroup('SIGKILL');
  activeDownload?.abort();
});
async function hash(path) {
  const digest = createHash('sha256');
  for await (const chunk of createReadStream(path)) digest.update(chunk);
  return digest.digest('hex');
}
async function verify(path, expected) {
  if (await hash(path) !== expected) throw new Error(`SHA-256 mismatch: ${path}`);
}
async function exists(path) {
  try { await stat(path); return true; } catch (e) { if (e.code === 'ENOENT') return false; throw e; }
}
async function command(program, commandArgs, config = {}) {
  if (interrupted) throw new Error('interrupted');
  const child = spawn(program, commandArgs, { detached: true, stdio: ['ignore', 'pipe', 'pipe'], ...config });
  activeChild = child;
  let stdout = '', stderr = '', timedOut = false, outputExceeded = false, maxRssKiB = 0;
  const start = performance.now();
  const collect = (which, data) => {
    if (which === 'stdout') stdout += data; else stderr += data;
    if (stdout.length + stderr.length > 4 * 1024 * 1024) { outputExceeded = true; killGroup('SIGKILL'); }
  };
  child.stdout.on('data', b => collect('stdout', b));
  child.stderr.on('data', b => collect('stderr', b));
  const timeout = setTimeout(() => { timedOut = true; killGroup('SIGKILL'); }, timeoutMs);
  const monitor = setInterval(async () => {
    try {
      const match = (await readFile(`/proc/${child.pid}/status`, 'utf8')).match(/^VmHWM:\s+(\d+)/m);
      if (match) maxRssKiB = Math.max(maxRssKiB, Number(match[1]));
    } catch { /* Process may have exited. */ }
  }, 10);
  try {
    const completion = await new Promise((res, rej) => {
      child.on('error', rej);
      child.on('close', (code, signal) => res({ code, signal }));
    });
    return { ...completion, elapsed_ms: Number((performance.now() - start).toFixed(3)),
      observed_peak_rss_kib: maxRssKiB, timed_out: timedOut, output_exceeded: outputExceeded,
      interrupted, stdout, stderr };
  } finally { clearTimeout(timeout); clearInterval(monitor); activeChild = undefined; }
}
async function provision(artifact) {
  if (interrupted) throw new Error('interrupted');
  const target = join(cache, artifact.path);
  if (await exists(target)) { await verify(target, artifact.sha256); return; }
  await mkdir(dirname(target), { recursive: true });
  const temporary = `${target}.download-${process.pid}`;
  activeDownload = new AbortController();
  try {
    const response = await fetch(artifact.url, { signal: AbortSignal.any([
      activeDownload.signal, AbortSignal.timeout(120000),
    ]) });
    if (!response.ok || !response.body) throw new Error(`Download failed: ${response.status} ${artifact.url}`);
    const fd = await open(temporary, 'wx');
    try {
      for await (const chunk of response.body) {
        if (interrupted) throw new Error('interrupted');
        await fd.writeFile(chunk);
      }
    } finally { await fd.close(); }
    await verify(temporary, artifact.archive_sha256 || artifact.sha256);
    if (artifact.member) {
      // Extract only the manifest's named member; do not expand other paths.
      const unpacked = await command('tar', ['-xzf', temporary, '-C', dirname(target), '--strip-components',
        String(artifact.member.split('/').length - 1), artifact.member]);
      if (unpacked.code !== 0) throw new Error(`Extraction failed: ${unpacked.stderr}`);
      await verify(target, artifact.sha256);
    } else {
      await rename(temporary, target);
    }
    if (artifact.kind === 'executable') await chmod(target, 0o755);
  } finally { activeDownload = undefined; await rm(temporary, { force: true }); }
}

await mkdir(cache, { recursive: true });
await mkdir(dirname(output), { recursive: true });
const labels = ['process-cold', 'repeat-process-warm-filesystem'];
const outputPaths = [output, ...labels.map(label => `${output}.${label}.json`)];
const protectedPaths = new Set([fixture, ...pins.artifacts.map(a => join(cache, a.path))]);
if (outputPaths.some(path => protectedPaths.has(path))) throw new Error('Output collides with a fixture or runtime asset');
// Reserve new files before provisioning. Existing files and symlinks fail closed.
const outputHandles = new Map();
try {
  for (const path of outputPaths) outputHandles.set(path, await open(path, 'wx'));
} catch (error) {
  for (const handle of outputHandles.values()) await handle.close();
  throw error;
}
let workingDirectory;
const report = { schema_version: 1, started_at: new Date().toISOString(),
  scope: 'upstream CLI CPU OCR; not TPE integration or device qualification',
  pins_sha256: await hash(join(repo, 'docs/validation/docling-runtime-pins.json')),
  fixture: pins.fixture, artifacts: [], runs: [] };
try {
  await verify(fixture, pins.fixture.sha256);
  for (const artifact of pins.artifacts) {
    await provision(artifact);
    report.artifacts.push({ ...artifact, bytes: (await stat(join(cache, artifact.path))).size, verified: true });
  }
  const version = await command(join(cache, 'docling-rs'), ['--version']);
  if (version.code !== 0 || !version.stdout.startsWith(`docling-rs ${pins.docling_version} `)) {
    throw new Error(`Version probe failed: ${version.stdout}${version.stderr}`);
  }
  report.version = version.stdout.trim();
  report.limits = { workers: 1, intra_threads: 1, ocr_sessions: 1,
    address_space_bytes: 4294967296, cpu_seconds: 60, wall_ms: timeoutMs, output_bytes: 4194304 };
  const env = { ...process.env, DOCLING_RS_EP: 'cpu', DOCLING_RS_PDF_THREADS: '1',
    DOCLING_RS_PDF_WORKERS: '1', DOCLING_RS_OCR_SESSIONS: '1', DOCLING_RS_NO_GRAPH_CACHE: '1',
    DOCLING_LAYOUT_ONNX: join(cache, 'models/layout_heron_int8.onnx'),
    DOCLING_OCR_REC_ONNX: join(cache, 'models/ocr_rec_en.onnx'),
    DOCLING_OCR_DICT: join(cache, 'models/en_dict.txt'),
    DOCLING_OCR_DET_ONNX: join(cache, 'models/ocr_det.onnx'),
    PDFIUM_DYNAMIC_LIB_PATH: join(cache, 'pdfium/lib'), OMP_NUM_THREADS: '1', OPENBLAS_NUM_THREADS: '1' };
  workingDirectory = await mkdtemp(join(cache, 'probe-cwd-'));
  for (const label of labels) {
    const result = await command('prlimit', ['--as=4294967296', '--cpu=60', '--nofile=128', '--',
      join(cache, 'docling-rs'), '--ocr-engine', 'ppocr', '--ocr-lang', 'en',
      '--force-full-page-ocr', '--no-table-former', '--to', 'json', fixture], { cwd: workingDirectory, env });
    const rawPath = `${output}.${label}.json`;
    await outputHandles.get(rawPath).writeFile(result.stdout);
    let document;
    try { document = JSON.parse(result.stdout); } catch { /* Retain malformed output as failure. */ }
    const phrases = pins.fixture.expected_phrases.map(text => ({ text,
      found: Array.isArray(document?.texts) && document.texts.some(item => item.text === text) }));
    const { stdout: _stdout, ...observations } = result;
    report.runs.push({ label, ...observations, output_file: rawPath, output_sha256: await hash(rawPath),
      rss_method: '10 ms /proc/<pid>/status VmHWM sampling; may miss exit-time peak', expected_phrases: phrases });
    if (result.code !== 0 || phrases.some(p => !p.found)) throw new Error(`OCR fixture failed: ${label}`);
  }
  await verify(fixture, pins.fixture.sha256);
  report.original_preserved = true;
  report.passed = true;
} catch (error) {
  report.passed = false;
  report.error = String(error);
  process.exitCode = interrupted ? 130 : 1;
} finally {
  killGroup('SIGKILL');
  report.finished_at = new Date().toISOString();
  await outputHandles.get(output).writeFile(`${JSON.stringify(report, null, 2)}\n`);
  for (const handle of outputHandles.values()) await handle.close();
  if (workingDirectory) await rm(workingDirectory, { recursive: true });
}
console.log(JSON.stringify({ passed: report.passed, output, runs: report.runs.map(r => ({
  label: r.label, elapsed_ms: r.elapsed_ms, code: r.code,
  phrases_matched: r.expected_phrases.every(p => p.found),
})) }));
