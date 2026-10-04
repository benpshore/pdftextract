#!/usr/bin/env node
/** Local development only: forward PDFs to the existing native CLI on loopback. */
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { constants } from 'node:fs';
import { access, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { isAbsolute, join } from 'node:path';
import { pathToFileURL } from 'node:url';

const hash = bytes => createHash('sha256').update(bytes).digest('hex');
class RuntimeError extends Error {
  constructor(status, message) { super(message); this.status = status; }
}
function boundedInteger(value, fallback, min, max, name) {
  const number = value === undefined ? fallback : Number(value);
  if (!Number.isSafeInteger(number) || number < min || number > max) throw new Error(`Invalid ${name}`);
  return number;
}
function absoluteFile(value, name) {
  if (value && !isAbsolute(value)) throw new Error(`${name} must be an absolute file path`);
  return value || null;
}
export function configuration(env = process.env) {
  let endpoint = null;
  if (env.TPE_GROBID_URL) {
    const url = new URL(env.TPE_GROBID_URL);
    if (url.protocol !== 'http:' || !['127.0.0.1', '[::1]'].includes(url.hostname) || url.username || url.password || url.search || url.hash || !['', '/'].includes(url.pathname)) {
      throw new Error('TPE_GROBID_URL must be a literal loopback HTTP base URL with no credentials, path, query, or fragment');
    }
    endpoint = url.origin;
  }
  const capturedPath = absoluteFile(env.SCHOLARLY_CAPTURED_JSON, 'SCHOLARLY_CAPTURED_JSON');
  const binary = absoluteFile(env.TPE_NATIVE_BIN, 'TPE_NATIVE_BIN');
  if (capturedPath && binary) throw new Error('Choose live native or captured replay explicitly, not both');
  return {
    port: boundedInteger(env.SCHOLARLY_LOCAL_PORT, 8072, 0, 65535, 'SCHOLARLY_LOCAL_PORT'),
    maxInputBytes: boundedInteger(env.SCHOLARLY_MAX_INPUT_BYTES, 32 * 1024 * 1024, 5, 128 * 1024 * 1024, 'SCHOLARLY_MAX_INPUT_BYTES'),
    maxOutputBytes: boundedInteger(env.SCHOLARLY_MAX_OUTPUT_BYTES, 32 * 1024 * 1024, 1024, 128 * 1024 * 1024, 'SCHOLARLY_MAX_OUTPUT_BYTES'),
    timeoutMs: boundedInteger(env.SCHOLARLY_TIMEOUT_MS, 60000, 100, 300000, 'SCHOLARLY_TIMEOUT_MS'),
    binary, endpoint, capturedPath,
    mode: capturedPath ? 'captured-native' : 'live-native',
    resolveIdentifiers: env.TPE_SCHOLARLY_RESOLVE === '1' && !capturedPath,
    // No inherited bearer, proxy, registry contact, or arbitrary command settings.
    childEnv: { PATH: env.PATH || '', ...(endpoint ? { TPE_GROBID_URL: endpoint } : {}) },
  };
}
async function boundedFile(path, limit) {
  const metadata = await stat(path);
  if (!metadata.isFile() || metadata.size > limit) throw new RuntimeError(503, 'Configured runtime file is missing, invalid, or too large');
  const bytes = await readFile(path);
  if (bytes.length > limit) throw new RuntimeError(503, 'Configured runtime file exceeds its limit');
  return bytes;
}
function validateCanonical(bytes, sourceHash, kind) {
  let document;
  try { document = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes)); }
  catch { throw new RuntimeError(502, 'Native output is not one canonical JSON document'); }
  if (kind === 'bibliography') {
    if (document?.sha256 !== sourceHash || !Array.isArray(document.references) || !Array.isArray(document.warnings)) {
      throw new RuntimeError(502, 'Native bibliography does not match the submitted source');
    }
  } else if (!/^[a-f0-9]{64}$/.test(sourceHash) || document?.source_sha256 !== sourceHash || typeof document.raw_tei !== 'string' || document.tei_sha256 !== hash(document.raw_tei) || !Array.isArray(document.citations) || !Array.isArray(document.warnings)) {
    throw new RuntimeError(502, 'Native scholarly output or retained TEI does not match its source hashes');
  }
  return document;
}
async function availability(config) {
  if (config.capturedPath) {
    try {
      const bytes = await boundedFile(config.capturedPath, config.maxOutputBytes);
      const document = JSON.parse(bytes.toString('utf8'));
      validateCanonical(bytes, document.source_sha256, 'grobid');
      return { status: 'ready', mode: config.mode, source_sha256: document.source_sha256, warning: 'Explicit replay of captured native output; no live processing occurs.' };
    } catch { return { status: 'blocked', mode: config.mode, reason: 'Captured native output is missing or invalid' }; }
  }
  if (!config.binary || !config.endpoint) return { status: 'blocked', mode: config.mode, reason: 'Configure TPE_NATIVE_BIN and a loopback TPE_GROBID_URL' };
  try {
    await access(config.binary, constants.X_OK);
    if (!(await stat(config.binary)).isFile()) throw new Error('not a file');
  } catch { return { status: 'blocked', mode: config.mode, reason: 'Configured native executable is unavailable' }; }
  try {
    const response = await fetch(`${config.endpoint}/api/isalive`, { redirect: 'error', signal: AbortSignal.timeout(2000) });
    const reader = response.body?.getReader();
    let content = '';
    if (reader) {
      try {
        while (true) {
          const { value, done } = await reader.read();
          if (done) break;
          content += Buffer.from(value).toString('utf8');
          if (content.length > 64) throw new Error('invalid health response');
        }
      } finally { await reader.cancel().catch(() => {}); reader.releaseLock(); }
    }
    if (!response.ok || content.trim() !== 'true') throw new Error('not ready');
  } catch { return { status: 'blocked', mode: config.mode, reason: 'Configured loopback GROBID service is unavailable' }; }
  return { status: 'ready', mode: config.mode };
}
async function health(config) {
  const state = await availability(config);
  return { ...state, resolver: config.resolveIdentifiers && state.status === 'ready', max_input_bytes: config.maxInputBytes, max_output_bytes: config.maxOutputBytes, timeout_ms: config.timeoutMs };
}
async function readPdf(request, config) {
  if (request.headers['content-type']?.split(';')[0].trim() !== 'application/pdf') throw new RuntimeError(415, 'Expected application/pdf');
  if (Number(request.headers['content-length'] || 0) > config.maxInputBytes) throw new RuntimeError(413, 'PDF exceeds the local input limit');
  let size = 0;
  const chunks = [];
  for await (const chunk of request) {
    size += chunk.length;
    if (size > config.maxInputBytes) throw new RuntimeError(413, 'PDF exceeds the local input limit');
    chunks.push(chunk);
  }
  const bytes = Buffer.concat(chunks);
  if (bytes.subarray(0, 5).toString() !== '%PDF-') throw new RuntimeError(415, 'Expected a PDF header');
  return bytes;
}
function nativeRequest(config, file, kind, signal) {
  const args = kind === 'bibliography'
    ? ['bibliography', file, '--backend', 'lopdf', '--resolve', '--max-bytes', String(config.maxInputBytes)]
    : ['grobid', file, '--consolidation', '0', '--timeout-ms', String(config.timeoutMs), '--max-memory-growth-mib', '512', '--max-input-bytes', String(config.maxInputBytes), '--max-response-bytes', String(config.maxOutputBytes)];
  return new Promise((resolve, reject) => {
    const child = spawn(config.binary, args, { env: config.childEnv, stdio: ['ignore', 'pipe', 'pipe'], detached: true });
    let total = 0, stderrBytes = 0, failure = null, forcedKill;
    const output = [];
    function killGroup(signalName) {
      if (!child.pid) return;
      try { process.kill(-child.pid, signalName); } catch (error) { if (error.code !== 'ESRCH') child.kill(signalName); }
    }
    function stop(error) {
      if (failure) return;
      failure = error;
      killGroup('SIGTERM');
      forcedKill = setTimeout(() => killGroup('SIGKILL'), 250);
      forcedKill.unref();
    }
    const abort = () => stop(new RuntimeError(499, 'Local scholarly request cancelled'));
    const timer = setTimeout(() => stop(new RuntimeError(504, 'Native processing exceeded the local deadline')), config.timeoutMs + 1000);
    signal.addEventListener('abort', abort, { once: true });
    if (signal.aborted) abort();
    child.stdout.on('data', chunk => {
      total += chunk.length;
      if (total > config.maxOutputBytes) stop(new RuntimeError(502, 'Native output exceeded the local limit'));
      else if (!failure) output.push(chunk);
    });
    child.stderr.on('data', chunk => {
      stderrBytes += chunk.length;
      if (stderrBytes > 64 * 1024) stop(new RuntimeError(502, 'Native diagnostics exceeded the local limit'));
    });
    child.once('error', () => { failure ||= new RuntimeError(503, 'Native executable could not be started'); });
    child.once('close', code => {
      if (failure) killGroup('SIGKILL');
      clearTimeout(timer); clearTimeout(forcedKill); signal.removeEventListener('abort', abort);
      if (failure) return reject(failure);
      // Bibliography exits nonzero for partial/not-found records: retain that canonical evidence.
      if (code !== 0 && !(kind === 'bibliography' && total > 0)) return reject(new RuntimeError(502, 'Native processing failed; no scholarly result was published'));
      resolve(Buffer.concat(output));
    });
  });
}
function send(response, status, value, headers = {}) {
  if (response.destroyed) return;
  response.writeHead(status, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff', ...headers });
  response.end(Buffer.isBuffer(value) ? value : JSON.stringify(value));
}
export function createLocalRuntime(config = configuration()) {
  let active = false;
  const controllers = new Set();
  const pending = new Set();
  const server = createServer(async (request, response) => {
    response.setHeader('X-TPE-Scholarly-Mode', config.mode);
    const host = request.headers.host;
    if (request.socket.remoteAddress !== '127.0.0.1' || host !== `127.0.0.1:${server.address().port}` || request.headers.origin || request.headers['sec-fetch-site']) {
      send(response, 403, { error: 'Only the local server bridge may call this runtime' }); return;
    }
    if (request.method === 'GET' && request.url === '/health') { send(response, 200, await health(config)); return; }
    if (request.method !== 'POST' || !['/grobid', '/bibliography'].includes(request.url)) { send(response, 404, { error: 'Not found' }); return; }
    if (request.headers['x-tpe-local-bridge'] !== '1') { send(response, 403, { error: 'Local server bridge header required' }); return; }
    if (active) { send(response, 503, { error: 'Local scholarly runtime is busy' }, { 'Retry-After': '1' }); return; }
    if (request.url === '/bibliography' && !config.resolveIdentifiers) { send(response, 503, { error: 'Native identifier resolution is not enabled' }); return; }
    active = true;
    let finished;
    const complete = new Promise(resolve => { finished = resolve; }); pending.add(complete);
    const controller = new AbortController(); controllers.add(controller);
    const abort = () => { if (!response.writableEnded) controller.abort(); };
    request.once('aborted', abort); response.once('close', abort);
    let directory;
    try {
      const state = await availability(config);
      if (state.status !== 'ready') throw new RuntimeError(503, state.reason);
      const pdf = await readPdf(request, config), sourceHash = hash(pdf);
      let output;
      if (config.capturedPath) {
        if (sourceHash !== state.source_sha256) throw new RuntimeError(409, 'Captured output is only valid for its exact original PDF');
        output = await boundedFile(config.capturedPath, config.maxOutputBytes);
      } else {
        directory = await mkdtemp(join(tmpdir(), 'tpe-scholarly-local-'));
        const file = join(directory, 'source.pdf');
        await writeFile(file, pdf, { mode: 0o600 });
        output = await nativeRequest(config, file, request.url.slice(1), controller.signal);
      }
      validateCanonical(output, sourceHash, request.url.slice(1));
      send(response, 200, output, { 'X-TPE-Scholarly-Mode': config.mode });
    } catch (error) { send(response, error instanceof RuntimeError ? error.status : 500, { error: error instanceof RuntimeError ? error.message : 'Local runtime failed; no result was published' }); }
    finally {
      request.removeListener('aborted', abort); response.removeListener('close', abort);
      try { if (directory) await rm(directory, { recursive: true, force: true }); }
      finally { controllers.delete(controller); active = false; pending.delete(complete); finished(); }
    }
  });
  server.requestTimeout = 15000;
  server.headersTimeout = 10000;
  server.keepAliveTimeout = 1000;
  return {
    server,
    async listen() { await new Promise((resolve, reject) => { server.once('error', reject); server.listen(config.port, '127.0.0.1', resolve); }); return `http://127.0.0.1:${server.address().port}`; },
    async close() {
      for (const controller of controllers) controller.abort();
      await new Promise(resolve => { server.close(resolve); server.closeAllConnections(); });
      await Promise.allSettled([...pending]);
    },
  };
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const runtime = createLocalRuntime();
  const endpoint = await runtime.listen();
  console.log(JSON.stringify({ runtime: 'tpe-scholarly-local', endpoint, ...(await health(configuration())) }));
  for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, async () => { await runtime.close(); process.exit(0); });
}
