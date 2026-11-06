// Generates the Tauri app icons (PNG + ICO) without any external dependency.
const fs = require('fs');
const path = require('path');
const zlib = require('zlib');

const OUT = path.join(__dirname, '..', 'crates', 'reverie-desktop', 'icons');
fs.mkdirSync(OUT, { recursive: true });

const BG = [37, 99, 235];      // #2563EB
const FG = [248, 250, 252];    // #F8FAFC
const ACCENT = [96, 165, 250]; // #60A5FA

// Simple geometric mark: rounded square field with an ascending "swarm" of bars.
function pixel(x, y, size) {
  const r = size * 0.22;
  const inside = (() => {
    const cx = Math.min(Math.max(x, r), size - r);
    const cy = Math.min(Math.max(y, r), size - r);
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r || (x >= r && x <= size - r) || (y >= r && y <= size - r);
  })();
  if (!inside) return [0, 0, 0, 0];

  // Three ascending bars, centred.
  const barW = size * 0.12;
  const gap = size * 0.07;
  const totalW = barW * 3 + gap * 2;
  const startX = (size - totalW) / 2;
  const baseY = size * 0.76;
  const heights = [0.26, 0.4, 0.54];
  for (let i = 0; i < 3; i++) {
    const bx = startX + i * (barW + gap);
    const h = size * heights[i];
    if (x >= bx && x <= bx + barW && y >= baseY - h && y <= baseY) {
      return i === 2 ? [...ACCENT, 255] : [...FG, 255];
    }
  }
  return [...BG, 255];
}

function crc32(buf) {
  let c;
  const table = [];
  for (let n = 0; n < 256; n++) {
    c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  let crc = 0xffffffff;
  for (let i = 0; i < buf.length; i++) crc = table[(crc ^ buf[i]) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const typeBuf = Buffer.from(type, 'ascii');
  const crcBuf = Buffer.alloc(4);
  crcBuf.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])), 0);
  return Buffer.concat([len, typeBuf, data, crcBuf]);
}

function png(size) {
  const raw = Buffer.alloc(size * (size * 4 + 1));
  let o = 0;
  for (let y = 0; y < size; y++) {
    raw[o++] = 0; // filter: none
    for (let x = 0; x < size; x++) {
      const [r, g, b, a] = pixel(x + 0.5, y + 0.5, size);
      raw[o++] = r; raw[o++] = g; raw[o++] = b; raw[o++] = a;
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8;  // bit depth
  ihdr[9] = 6;  // RGBA
  ihdr[10] = 0;
  ihdr[11] = 0;
  ihdr[12] = 0;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', zlib.deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

function ico(size) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(1, 4);

  const xorSize = size * size * 4;
  const andSize = Math.ceil(size / 8) * Math.ceil(size / 8) * 4;
  const dib = Buffer.alloc(40 + xorSize + andSize);
  dib.writeUInt32LE(40, 0);
  dib.writeInt32LE(size, 4);
  dib.writeInt32LE(size * 2, 8);
  dib.writeUInt16LE(1, 12);
  dib.writeUInt16LE(32, 14);
  dib.writeUInt32LE(0, 16);
  dib.writeUInt32LE(xorSize + andSize, 20);

  let o = 40;
  for (let y = size - 1; y >= 0; y--) {
    for (let x = 0; x < size; x++) {
      const [r, g, b, a] = pixel(x + 0.5, y + 0.5, size);
      dib[o++] = b; dib[o++] = g; dib[o++] = r; dib[o++] = a;
    }
  }

  const entry = Buffer.alloc(16);
  entry[0] = size >= 256 ? 0 : size;
  entry[1] = size >= 256 ? 0 : size;
  entry[2] = 0;
  entry[3] = 0;
  entry.writeUInt16LE(1, 4);
  entry.writeUInt16LE(32, 6);
  entry.writeUInt32LE(dib.length, 8);
  entry.writeUInt32LE(22, 12);

  return Buffer.concat([header, entry, dib]);
}

fs.writeFileSync(path.join(OUT, '32x32.png'), png(32));
fs.writeFileSync(path.join(OUT, '128x128.png'), png(128));
fs.writeFileSync(path.join(OUT, '128x128@2x.png'), png(256));
fs.writeFileSync(path.join(OUT, 'icon.png'), png(512));
fs.writeFileSync(path.join(OUT, 'icon.ico'), ico(32));

console.log('icons written to', OUT);
