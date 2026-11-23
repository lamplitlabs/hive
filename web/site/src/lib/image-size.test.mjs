// The size of a picture, read from its first bytes: what lets a docs page
// keep a picture's place before it has loaded.
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

const { imageSize } = await import("./image-size.ts");
const docs = await import("./docs.ts");

const bytes = (...parts) => Uint8Array.from(parts.flatMap((p) => (typeof p === "string" ? [...p].map((c) => c.charCodeAt(0)) : p)));
const be32 = (n) => [(n >>> 24) & 255, (n >>> 16) & 255, (n >>> 8) & 255, n & 255];
const le16 = (n) => [n & 255, (n >> 8) & 255];
const le24 = (n) => [n & 255, (n >> 8) & 255, (n >> 16) & 255];

test("PNG and GIF say their size in their header", () => {
  const png = bytes([0x89], "PNG", [0x0d, 0x0a, 0x1a, 0x0a], be32(13), "IHDR", be32(1440), be32(900), [8, 6, 0, 0, 0]);
  assert.deepEqual(imageSize(png), { width: 1440, height: 900 });
  assert.deepEqual(imageSize(bytes("GIF89a", le16(320), le16(200), [0, 0, 0])), { width: 320, height: 200 });
});

test("WebP in its three forms", () => {
  const riff = (kind, rest) => bytes("RIFF", [0, 0, 0, 0], "WEBP", kind, [0, 0, 0, 0], rest, new Array(12).fill(0));
  // Lossy: a frame tag, the start code, then 14 bits each.
  assert.deepEqual(imageSize(riff("VP8 ", [0, 0, 0, 0x9d, 0x01, 0x2a, ...le16(1440), ...le16(900)])), { width: 1440, height: 900 });
  // Lossless: one signature byte, then 14 bits of width - 1 and 14 bits of height - 1.
  const packed = (1440 - 1) | ((900 - 1) << 14);
  assert.deepEqual(imageSize(riff("VP8L", [0x2f, packed & 255, (packed >> 8) & 255, (packed >> 16) & 255, (packed >> 24) & 255])), { width: 1440, height: 900 });
  // Extended: flags, three reserved bytes, then 24 bits each, both less one.
  assert.deepEqual(imageSize(riff("VP8X", [0, 0, 0, 0, ...le24(2880 - 1), ...le24(1800 - 1)])), { width: 2880, height: 1800 });
});

test("JPEG keeps its size in the first frame header, after whatever comes before it", () => {
  const app0 = [0xff, 0xe0, 0x00, 0x10, ...new Array(14).fill(0)];
  const sof = [0xff, 0xc0, 0x00, 0x11, 0x08, 0x03, 0x84, 0x05, 0xa0, ...new Array(12).fill(0)];
  assert.deepEqual(imageSize(bytes([0xff, 0xd8], app0, sof)), { width: 1440, height: 900 });
});

test("what is no picture, or is cut short, has no size", () => {
  assert.equal(imageSize(bytes("<svg xmlns='http://www.w3.org/2000/svg'/>")), null);
  assert.equal(imageSize(new Uint8Array(0)), null);
  assert.equal(imageSize(bytes([0x89], "PNG")), null);
  assert.equal(imageSize(bytes([0xff, 0xd8, 0xff])), null);
});

test("every picture of the docs in a format whose header is read has a size the page can keep a place for", () => {
  const root = docs.docsRoot();
  // An AVIF is carried and shown without a size: its header is not read.
  for (const p of docs.listDocPictures(root).filter((f) => /\.(png|webp|gif|jpe?g)$/i.test(f))) {
    const size = imageSize(fs.readFileSync(path.join(root, p)));
    assert.ok(size && size.width > 0 && size.height > 0, `${p}: ${JSON.stringify(size)}`);
  }
});
