// Generates the extension icons (16/32/48/128) as a small "browsing graph"
// motif — a few connected nodes on the dashboard's dark background. No external
// image deps; a minimal PNG encoder using Node's zlib. Re-run with:
//   node extension/icons/generate-icons.mjs
import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const DIR = dirname(fileURLToPath(import.meta.url));

const BG = [0x11, 0x15, 0x1c];
const EDGE = [0x8a, 0xa0, 0xb6];
const NODE = [0x4f, 0x8e, 0xf7];
const NODE2 = [0x2e, 0xcc, 0x71];

// normalized node positions and the edges connecting them
const NODES = [
  { x: 0.3, y: 0.34, c: NODE },
  { x: 0.72, y: 0.3, c: NODE2 },
  { x: 0.52, y: 0.74, c: NODE },
];
const EDGES = [
  [0, 1],
  [1, 2],
  [0, 2],
];

function render(size) {
  const buf = Buffer.alloc(size * size * 4);
  const set = (x, y, [r, g, b]) => {
    if (x < 0 || y < 0 || x >= size || y >= size) return;
    const i = (y * size + x) * 4;
    buf[i] = r;
    buf[i + 1] = g;
    buf[i + 2] = b;
    buf[i + 3] = 255;
  };
  // background
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) set(x, y, BG);

  const px = (n) => [n.x * size, n.y * size];
  const lw = Math.max(1, size * 0.045);
  // edges
  for (const [a, b] of EDGES) {
    const [ax, ay] = px(NODES[a]);
    const [bx, by] = px(NODES[b]);
    const steps = Math.ceil(Math.hypot(bx - ax, by - ay)) * 2;
    for (let s = 0; s <= steps; s++) {
      const t = s / steps;
      const cx = ax + (bx - ax) * t;
      const cy = ay + (by - ay) * t;
      for (let oy = -lw; oy <= lw; oy++)
        for (let ox = -lw; ox <= lw; ox++)
          if (ox * ox + oy * oy <= lw * lw)
            set(Math.round(cx + ox), Math.round(cy + oy), EDGE);
    }
  }
  // nodes
  const r = Math.max(1.5, size * 0.13);
  for (const n of NODES) {
    const [cx, cy] = px(n);
    for (let y = Math.floor(cy - r); y <= cy + r; y++)
      for (let x = Math.floor(cx - r); x <= cx + r; x++)
        if ((x - cx) ** 2 + (y - cy) ** 2 <= r * r) set(x, y, n.c);
  }
  return buf;
}

function crc32(buf) {
  let c = ~0;
  for (let i = 0; i < buf.length; i++) {
    c ^= buf[i];
    for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1));
  }
  return (~c) >>> 0;
}

function chunk(type, data) {
  const t = Buffer.from(type, "ascii");
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([t, data])), 0);
  return Buffer.concat([len, t, data, crc]);
}

function encodePng(size, rgba) {
  const sig = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  // raw scanlines, filter 0 per row
  const raw = Buffer.alloc((size * 4 + 1) * size);
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0;
    rgba.copy(raw, y * (size * 4 + 1) + 1, y * size * 4, (y + 1) * size * 4);
  }
  const idat = deflateSync(raw);
  return Buffer.concat([
    sig,
    chunk("IHDR", ihdr),
    chunk("IDAT", idat),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

for (const size of [16, 32, 48, 128]) {
  const png = encodePng(size, render(size));
  writeFileSync(join(DIR, `${size}.png`), png);
  console.log(`wrote ${size}.png (${png.length} bytes)`);
}
