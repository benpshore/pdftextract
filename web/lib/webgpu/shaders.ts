/** WGSL compute kernels for OCR pre-processing. Every kernel uses integer
 * arithmetic only, with the same formulas as `cpu.ts`, so a GPU result is
 * byte-for-byte identical to the CPU fallback (no floating-point drift).
 * Grayscale output and input are packed four 8-bit pixels per u32 word in
 * little-endian pixel order, so a readback is directly a Uint8Array. */

export const WORKGROUP_SIZE = 256;

/** RGBA8 (one u32 per pixel, R in the low byte) → packed 8-bit luma, BT.601 integer weights. */
export const GRAYSCALE_WGSL = /* wgsl */ `
struct Params { pixels: u32, words: u32, pad0: u32, pad1: u32 }
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> rgba: array<u32>;
@group(0) @binding(2) var<storage, read_write> gray: array<u32>;
fn luma(px: u32) -> u32 {
  let r = px & 0xffu;
  let g = (px >> 8u) & 0xffu;
  let b = (px >> 16u) & 0xffu;
  return (r * 299u + g * 587u + b * 114u + 500u) / 1000u;
}
@compute @workgroup_size(${WORKGROUP_SIZE})
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
  let word = id.x;
  if (word >= params.words) { return; }
  var packed = 0u;
  for (var k = 0u; k < 4u; k++) {
    let index = word * 4u + k;
    if (index < params.pixels) { packed |= luma(rgba[index]) << (8u * k); }
  }
  gray[word] = packed;
}
`;

/** 256-bin histogram of packed luma: workgroup-shared atomics, then one global atomicAdd per bin per workgroup. */
export const HISTOGRAM_WGSL = /* wgsl */ `
struct Params { pixels: u32, words: u32, pad0: u32, pad1: u32 }
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> gray: array<u32>;
@group(0) @binding(2) var<storage, read_write> histogram: array<atomic<u32>, 256>;
var<workgroup> local: array<atomic<u32>, 256>;
@compute @workgroup_size(${WORKGROUP_SIZE})
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
  atomicStore(&local[lid.x], 0u);
  workgroupBarrier();
  let word = id.x;
  if (word < params.words) {
    let packed = gray[word];
    let count = min(4u, params.pixels - word * 4u);
    for (var k = 0u; k < count; k++) { atomicAdd(&local[(packed >> (8u * k)) & 0xffu], 1u); }
  }
  workgroupBarrier();
  let value = atomicLoad(&local[lid.x]);
  if (value > 0u) { atomicAdd(&histogram[lid.x], value); }
}
`;

/** Projection profiles for skew estimation: every sampled dark pixel (luma <= threshold)
 * is rotated about the image centre by each candidate angle using 16.16 fixed-point
 * sine/cosine supplied by the host, and its destination row is counted with an atomic add. */
export const PROFILE_WGSL = /* wgsl */ `
struct Params {
  width: u32, height: u32, stride: u32, threshold: u32,
  samplesX: u32, samplesY: u32, angles: u32, centreX: i32,
  centreY: i32, pad0: u32, pad1: u32, pad2: u32,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> gray: array<u32>;
@group(0) @binding(2) var<storage, read> table: array<vec2<i32>>;
@group(0) @binding(3) var<storage, read_write> profiles: array<atomic<u32>>;
@compute @workgroup_size(${WORKGROUP_SIZE})
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
  let sample = id.x;
  if (sample >= params.samplesX * params.samplesY) { return; }
  let x = (sample % params.samplesX) * params.stride;
  let y = (sample / params.samplesX) * params.stride;
  let index = y * params.width + x;
  let luma = (gray[index >> 2u] >> (8u * (index & 3u))) & 0xffu;
  if (luma > params.threshold) { return; }
  let dx = i32(x) - params.centreX;
  let dy = i32(y) - params.centreY;
  for (var a = 0u; a < params.angles; a++) {
    let sc = table[a];
    let row = ((dx * sc.x + dy * sc.y + 32768) >> 16u) + params.centreY;
    if (row >= 0 && row < i32(params.height)) { atomicAdd(&profiles[a * params.height + u32(row)], 1u); }
  }
}
`;
