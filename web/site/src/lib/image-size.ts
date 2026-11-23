// The pixel size of a picture, read from the first bytes of the file: enough
// for a page to reserve the picture's place before it loads. PNG, WebP, GIF
// and JPEG; anything else is null and the page shows it without a size. Pure,
// so the node tests feed it bytes.

export type ImageSize = { width: number; height: number };

const ascii = (b: Uint8Array, at: number, n: number) => String.fromCharCode(...b.subarray(at, at + n));
const u16be = (b: Uint8Array, at: number) => (b[at] << 8) | b[at + 1];
const u16le = (b: Uint8Array, at: number) => b[at] | (b[at + 1] << 8);
const u24le = (b: Uint8Array, at: number) => b[at] | (b[at + 1] << 8) | (b[at + 2] << 16);
const u32be = (b: Uint8Array, at: number) => ((b[at] << 24) | (b[at + 1] << 16) | (b[at + 2] << 8) | b[at + 3]) >>> 0;

/** How many leading bytes of a file `imageSize` wants at most: a JPEG may keep its size behind its metadata. */
export const IMAGE_HEADER_BYTES = 256 * 1024;

export function imageSize(b: Uint8Array): ImageSize | null {
  const ok = (width: number, height: number) => (width > 0 && height > 0 ? { width, height } : null);

  // PNG: the signature, then the IHDR chunk.
  if (b.length >= 24 && u32be(b, 0) === 0x89504e47 && ascii(b, 12, 4) === "IHDR") return ok(u32be(b, 16), u32be(b, 20));

  // GIF: the logical screen, little-endian.
  if (b.length >= 10 && /^GIF8[79]a$/.test(ascii(b, 0, 6))) return ok(u16le(b, 6), u16le(b, 8));

  // WebP: a RIFF container with one of three first chunks.
  if (b.length >= 30 && ascii(b, 0, 4) === "RIFF" && ascii(b, 8, 4) === "WEBP") {
    const kind = ascii(b, 12, 4);
    if (kind === "VP8X") return ok(1 + u24le(b, 24), 1 + u24le(b, 27));
    if (kind === "VP8 " && b[23] === 0x9d && b[24] === 0x01 && b[25] === 0x2a) return ok(u16le(b, 26) & 0x3fff, u16le(b, 28) & 0x3fff);
    if (kind === "VP8L" && b[20] === 0x2f) {
      const bits = (b[21] | (b[22] << 8) | (b[23] << 16) | (b[24] << 24)) >>> 0;
      return ok(1 + (bits & 0x3fff), 1 + ((bits >>> 14) & 0x3fff));
    }
    return null;
  }

  // JPEG: walk the segments to the first start-of-frame.
  if (b.length >= 4 && b[0] === 0xff && b[1] === 0xd8) {
    let at = 2;
    while (at + 9 < b.length) {
      if (b[at] !== 0xff) return null;
      const marker = b[at + 1];
      if (marker === 0xff) {
        at += 1;
        continue;
      }
      const frame = marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc;
      if (frame) return ok(u16be(b, at + 7), u16be(b, at + 5));
      at += 2 + u16be(b, at + 2);
    }
  }
  return null;
}
