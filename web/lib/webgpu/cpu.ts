/** Pure-JavaScript reference implementations of the OCR pre-processing kernels and
 * the host-side maths both paths share (Otsu threshold, skew table and scoring).
 * Integer arithmetic matches `shaders.ts` exactly, so GPU and CPU results are
 * compared for equality, not approximate agreement. */

export type SkewTable = { degrees: Float64Array; sinQ: Int32Array; cosQ: Int32Array; zeroIndex: number };
export type SkewEstimate = { degrees: number; index: number; scores: Float64Array; confidence: number };
export type SkewOptions = { maxDegrees?: number; stepDegrees?: number; maxSamples?: number };

export const DEFAULT_SKEW: Required<SkewOptions> = { maxDegrees: 5, stepDegrees: 0.25, maxSamples: 1_000_000 };
const FIXED_ONE = 65536;

/** BT.601 integer luma of one RGBA pixel; identical to the WGSL `luma`. */
export function luma(r: number, g: number, b: number): number { return ((r * 299 + g * 587 + b * 114 + 500) / 1000) | 0; }

/** RGBA bytes → 8-bit luma per pixel (packed-word layout of the GPU readback is the same byte order). */
export function grayscaleCpu(rgba: Uint8Array | Uint8ClampedArray, pixels: number): Uint8Array {
  if (rgba.length < pixels * 4) throw new RangeError('RGBA buffer is shorter than the pixel count.');
  const gray = new Uint8Array(pixels);
  for (let index = 0, at = 0; index < pixels; index++, at += 4) gray[index] = luma(rgba[at], rgba[at + 1], rgba[at + 2]);
  return gray;
}

/** 256-bin luma histogram. */
export function histogramCpu(gray: Uint8Array): Uint32Array {
  const histogram = new Uint32Array(256);
  for (let index = 0; index < gray.length; index++) histogram[gray[index]]++;
  return histogram;
}

/** Otsu's threshold (maximum between-class variance). Pixels with luma <= threshold are "dark".
 * Returns 0 for a flat image so an empty page never becomes all-dark. */
export function otsuThreshold(histogram: Uint32Array): number {
  let total = 0, sum = 0;
  for (let level = 0; level < 256; level++) { total += histogram[level]; sum += level * histogram[level]; }
  if (total === 0) return 0;
  let background = 0, backgroundSum = 0, best = 0, threshold = 0;
  for (let level = 0; level < 256; level++) {
    background += histogram[level];
    if (background === 0) continue;
    const foreground = total - background;
    if (foreground === 0) break;
    backgroundSum += level * histogram[level];
    const meanBackground = backgroundSum / background, meanForeground = (sum - backgroundSum) / foreground;
    const variance = background * foreground * (meanBackground - meanForeground) ** 2;
    if (variance > best) { best = variance; threshold = level; }
  }
  return threshold;
}

/** Candidate angles (clockwise-positive, canvas convention) as 16.16 fixed-point sine/cosine pairs.
 * The host computes the table once; both kernels consume the same integers. */
export function skewTable(options: SkewOptions = {}): SkewTable {
  const { maxDegrees, stepDegrees } = { ...DEFAULT_SKEW, ...options };
  if (!(maxDegrees > 0) || !(stepDegrees > 0) || maxDegrees > 45) throw new RangeError('Skew search range must be 0 < step <= max <= 45 degrees.');
  const steps = Math.round(maxDegrees / stepDegrees), count = steps * 2 + 1;
  const degrees = new Float64Array(count), sinQ = new Int32Array(count), cosQ = new Int32Array(count);
  for (let index = 0; index < count; index++) {
    const value = (index - steps) * stepDegrees, radians = value * Math.PI / 180;
    // Rotating by -φ makes lines tilted by φ horizontal, so `degrees[index]` reports the tilt itself.
    degrees[index] = value; sinQ[index] = -Math.round(Math.sin(radians) * FIXED_ONE); cosQ[index] = Math.round(Math.cos(radians) * FIXED_ONE);
  }
  return { degrees, sinQ, cosQ, zeroIndex: steps };
}

/** Sampling stride so that at most `maxSamples` pixels enter the projection kernels. */
export function sampleStride(width: number, height: number, maxSamples: number): number {
  return Math.max(1, Math.ceil(Math.sqrt(width * height / Math.max(1, maxSamples))));
}

/** Row projection profiles of the dark pixels for every candidate angle (angles × height, row-major). */
export function profilesCpu(gray: Uint8Array, width: number, height: number, threshold: number, table: SkewTable, stride: number): Uint32Array {
  const angles = table.degrees.length, profiles = new Uint32Array(angles * height);
  const centreX = width >> 1, centreY = height >> 1;
  for (let y = 0; y < height; y += stride) {
    const dy = y - centreY;
    for (let x = 0; x < width; x += stride) {
      if (gray[y * width + x] > threshold) continue;
      const dx = x - centreX;
      for (let a = 0; a < angles; a++) {
        const row = ((dx * table.sinQ[a] + dy * table.cosQ[a] + 32768) >> 16) + centreY;
        if (row >= 0 && row < height) profiles[a * height + row]++;
      }
    }
  }
  return profiles;
}

/** Score each angle by the sum of squared differences between neighbouring rows
 * (sharp text lines give a spiky profile) and choose the best; ties prefer the smaller angle.
 * `confidence` is best score / score at zero degrees (1 means no evidence of skew). */
export function scoreProfiles(profiles: Uint32Array, height: number, table: SkewTable): SkewEstimate {
  const angles = table.degrees.length, scores = new Float64Array(angles);
  let index = table.zeroIndex;
  for (let a = 0; a < angles; a++) {
    let score = 0;
    for (let row = a * height, end = row + height - 1; row < end; row++) { const delta = profiles[row] - profiles[row + 1]; score += delta * delta; }
    scores[a] = score;
    if (score > scores[index] || score === scores[index] && Math.abs(table.degrees[a]) < Math.abs(table.degrees[index])) index = a;
  }
  const zero = scores[table.zeroIndex];
  return { degrees: table.degrees[index], index, scores, confidence: zero > 0 ? scores[index] / zero : scores[index] > 0 ? Infinity : 1 };
}

export type CpuStages = { gray: Uint8Array; histogram: Uint32Array; threshold: number; profiles: Uint32Array; skew: SkewEstimate; stride: number; table: SkewTable; timings: { grayscaleMs: number; histogramMs: number; skewMs: number; totalMs: number } };

/** The whole CPU pipeline in one call, with per-stage timings. */
export function preprocessCpu(rgba: Uint8Array | Uint8ClampedArray, width: number, height: number, options: SkewOptions = {}): CpuStages {
  const now = () => (typeof performance === 'object' ? performance.now() : Date.now());
  const pixels = width * height, table = skewTable(options), stride = sampleStride(width, height, options.maxSamples ?? DEFAULT_SKEW.maxSamples);
  const start = now(), gray = grayscaleCpu(rgba, pixels), afterGray = now();
  const histogram = histogramCpu(gray), threshold = otsuThreshold(histogram), afterHistogram = now();
  const profiles = profilesCpu(gray, width, height, threshold, table, stride), skew = scoreProfiles(profiles, height, table), end = now();
  return { gray, histogram, threshold, profiles, skew, stride, table, timings: { grayscaleMs: afterGray - start, histogramMs: afterHistogram - afterGray, skewMs: end - afterHistogram, totalMs: end - start } };
}
