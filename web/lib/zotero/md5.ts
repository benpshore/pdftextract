// MD5 (RFC 1321) for Zotero's file-upload authorisation, which identifies
// files by MD5. Web Crypto does not offer MD5, so this is a small pure
// implementation; it is not used for anything security-related.

const S = [7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21];
const K = Array.from({ length: 64 }, (_, i) => Math.floor(Math.abs(Math.sin(i + 1)) * 2 ** 32) >>> 0);

function rotl(x: number, c: number): number { return ((x << c) | (x >>> (32 - c))) >>> 0; }

/** Lower-case hexadecimal MD5 digest of `bytes`. */
export function md5Hex(bytes: Uint8Array): string {
  const length = bytes.length;
  const paddedLength = (((length + 8) >>> 6) + 1) << 6;
  const padded = new Uint8Array(paddedLength);
  padded.set(bytes);
  padded[length] = 0x80;
  const view = new DataView(padded.buffer);
  const bitLength = length * 8;
  view.setUint32(paddedLength - 8, bitLength >>> 0, true);
  view.setUint32(paddedLength - 4, Math.floor(bitLength / 2 ** 32), true);
  let a0 = 0x67452301, b0 = 0xefcdab89, c0 = 0x98badcfe, d0 = 0x10325476;
  const m = new Uint32Array(16);
  for (let offset = 0; offset < paddedLength; offset += 64) {
    for (let i = 0; i < 16; i++) m[i] = view.getUint32(offset + i * 4, true);
    let a = a0, b = b0, c = c0, d = d0;
    for (let i = 0; i < 64; i++) {
      let f: number, g: number;
      if (i < 16) { f = (b & c) | (~b & d); g = i; }
      else if (i < 32) { f = (d & b) | (~d & c); g = (5 * i + 1) % 16; }
      else if (i < 48) { f = b ^ c ^ d; g = (3 * i + 5) % 16; }
      else { f = c ^ (b | ~d); g = (7 * i) % 16; }
      const next = (a + f + K[i] + m[g]) >>> 0;
      a = d; d = c; c = b;
      b = (b + rotl(next, S[i])) >>> 0;
    }
    a0 = (a0 + a) >>> 0; b0 = (b0 + b) >>> 0; c0 = (c0 + c) >>> 0; d0 = (d0 + d) >>> 0;
  }
  const out = new Uint8Array(16);
  const outView = new DataView(out.buffer);
  outView.setUint32(0, a0, true); outView.setUint32(4, b0, true); outView.setUint32(8, c0, true); outView.setUint32(12, d0, true);
  return Array.from(out, byte => byte.toString(16).padStart(2, '0')).join('');
}

/** MD5 of a Blob's bytes. */
export async function md5OfBlob(blob: Blob): Promise<string> {
  return md5Hex(new Uint8Array(await blob.arrayBuffer()));
}
