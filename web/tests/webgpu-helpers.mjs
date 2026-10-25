/** Shared helpers for the WebGPU pre-processing tests: transpile the TypeScript sources with the
 * project's own TypeScript (no bundler), rewrite relative imports, and build deterministic fixtures. */
import { readFile, writeFile, mkdir, readdir } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
export const web = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const ts = require(join(web, 'node_modules/typescript'));

/** Transpile lib/image-ocr.ts and lib/webgpu/*.ts into `<out>/lib/...js` ES modules. */
export async function transpileSources(out) {
  const sources = ['lib/image-ocr.ts'];
  for (const name of await readdir(join(web, 'lib/webgpu'))) if (name.endsWith('.ts') && !name.endsWith('.d.ts')) sources.push(`lib/webgpu/${name}`);
  for (const source of sources) {
    const text = await readFile(join(web, source), 'utf8');
    const { outputText, diagnostics } = ts.transpileModule(text, { fileName: source, reportDiagnostics: true, compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } });
    if (diagnostics?.length) throw new Error(`${source}: ${diagnostics.map(item => ts.flattenDiagnosticMessageText(item.messageText, '\n')).join('; ')}`);
    const rewritten = outputText.replace(/(from\s+|import\()(['"])(\.{1,2}\/[^'"]+)\2/g, (match, lead, quote, path) => `${lead}${quote}${path.endsWith('.js') ? path : `${path}.js`}${quote}`);
    const target = join(out, source.replace(/\.ts$/, '.js'));
    await mkdir(dirname(target), { recursive: true });
    await writeFile(target, rewritten);
  }
  return sources;
}

/** Deterministic pseudo-random bytes (xorshift32) for colour-noise fixtures. */
export function noise(length, seed = 0x9e3779b9) {
  const bytes = new Uint8Array(length);
  let state = seed >>> 0 || 1;
  for (let index = 0; index < length; index++) { state ^= state << 13; state >>>= 0; state ^= state >>> 17; state ^= state << 5; state >>>= 0; bytes[index] = state & 255; }
  return bytes;
}

/** A synthetic "page": white RGBA with dark text-like line segments tilted by `degrees` (clockwise
 * positive in canvas coordinates), rasterised without anti-aliasing so both kernels see the same pixels. */
export function tiltedLines(width, height, degrees, options = {}) {
  const { lineGap = 24, thickness = 3, margin = 0.12, ink = 20, paper = 245, seed = 7 } = options;
  const rgba = new Uint8Array(width * height * 4).fill(paper);
  for (let index = 3; index < rgba.length; index += 4) rgba[index] = 255;
  const radians = degrees * Math.PI / 180, cos = Math.cos(radians), sin = Math.sin(radians);
  const cx = width / 2, cy = height / 2, grain = noise(width * height, seed);
  const put = (x, y) => { if (x < 0 || y < 0 || x >= width || y >= height) return; const at = (y * width + x) * 4; rgba[at] = rgba[at + 1] = rgba[at + 2] = ink + (grain[y * width + x] & 15); };
  for (let line = -height / 2 + lineGap; line < height / 2 - lineGap; line += lineGap) {
    for (let t = -width / 2 + width * margin; t < width / 2 - width * margin; t += 0.5) {
      // Word gaps make the profile text-like rather than a solid band.
      if (Math.floor((t + width) / 37) % 5 === 0) continue;
      for (let k = 0; k < thickness; k++) {
        const x = Math.round(cx + t * cos - (line + k) * sin), y = Math.round(cy + t * sin + (line + k) * cos);
        put(x, y);
      }
    }
  }
  return rgba;
}

export function maxAbsDiff(a, b) {
  if (a.length !== b.length) return Infinity;
  let worst = 0;
  for (let index = 0; index < a.length; index++) { const diff = Math.abs(a[index] - b[index]); if (diff > worst) worst = diff; }
  return worst;
}

/** Minimal PNG decoder for the committed fixtures: 8-bit RGB/RGBA, non-interlaced. Returns RGBA bytes. */
export async function decodePng(bytes) {
  const { inflateSync } = await import('node:zlib');
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (view.getUint32(0) !== 0x89504e47) throw new Error('not a PNG');
  const width = view.getUint32(16), height = view.getUint32(20), depth = bytes[24], colour = bytes[25], interlace = bytes[28];
  if (depth !== 8 || interlace !== 0 || colour !== 2 && colour !== 6) throw new Error(`unsupported PNG (depth ${depth}, colour ${colour}, interlace ${interlace})`);
  const chunks = [];
  for (let at = 8; at + 12 <= bytes.length;) {
    const length = view.getUint32(at), kind = String.fromCharCode(...bytes.subarray(at + 4, at + 8));
    if (kind === 'IDAT') chunks.push(bytes.subarray(at + 8, at + 8 + length));
    at += 12 + length;
    if (kind === 'IEND') break;
  }
  const channels = colour === 6 ? 4 : 3, stride = width * channels, raw = inflateSync(Buffer.concat(chunks));
  const rgba = new Uint8Array(width * height * 4);
  let previous = new Uint8Array(stride);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)], line = Uint8Array.from(raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1)));
    for (let x = 0; x < stride; x++) {
      const a = x >= channels ? line[x - channels] : 0, b = previous[x], c = x >= channels ? previous[x - channels] : 0;
      let predictor = 0;
      if (filter === 1) predictor = a; else if (filter === 2) predictor = b; else if (filter === 3) predictor = (a + b) >> 1;
      else if (filter === 4) { const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c); predictor = pa <= pb && pa <= pc ? a : pb <= pc ? b : c; }
      line[x] = (line[x] + predictor) & 255;
    }
    for (let x = 0; x < width; x++) { const from = x * channels, to = (y * width + x) * 4; rgba[to] = line[from]; rgba[to + 1] = line[from + 1]; rgba[to + 2] = line[from + 2]; rgba[to + 3] = channels === 4 ? line[from + 3] : 255; }
    previous = line;
  }
  return { width, height, rgba };
}
