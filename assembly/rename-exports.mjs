// rename-exports.mjs — rename entries in a core wasm module's export section.
//
// Why: the `waui:ui/app` world names two functions with hyphens (`on-click`,
// `on-tick`), and WIT uses those names verbatim as raw wasm export names.
// AssemblyScript identifiers cannot contain hyphens, so a component written
// in such a language exports `onclick`/`ontick` and must be renamed here,
// after compilation, before `wasm-tools component embed` + `new` type the
// lift against the WIT world.
//
// The export section is the only section touched; every other section is
// preserved byte-for-byte.
//
//   node rename-exports.mjs <in.wasm> <out.wasm> <old>=<new> ...

import { readFileSync, writeFileSync } from 'node:fs';

function readLeb(buf, pos) {
  let value = 0,
    shift = 0;
  for (;;) {
    const b = buf[pos++];
    value |= (b & 0x7f) << shift;
    if (!(b & 0x80)) return [value, pos];
    shift += 7;
  }
}

function leb(value) {
  const out = [];
  do {
    let b = value & 0x7f;
    value >>>= 7;
    if (value !== 0) b |= 0x80;
    out.push(b);
  } while (value !== 0);
  return out;
}

const [inFile, outFile, ...pairs] = process.argv.slice(2);
if (!inFile || !outFile || pairs.length === 0) {
  console.error('usage: node rename-exports.mjs <in.wasm> <out.wasm> <old>=<new> ...');
  process.exit(2);
}
const renames = new Map();
for (const pair of pairs) {
  const eq = pair.indexOf('=');
  if (eq < 0) {
    console.error(`bad pair (want old=new): ${pair}`);
    process.exit(2);
  }
  renames.set(pair.slice(0, eq), pair.slice(eq + 1));
}

const buf = readFileSync(inFile);
if (buf.subarray(0, 4).toString('latin1') !== '\0asm') {
  console.error(`${inFile}: not a wasm binary`);
  process.exit(1);
}

// Walk the top-level sections, keeping each section's raw bytes.
const sections = [];
let pos = 8;
while (pos < buf.length) {
  const id = buf[pos];
  let size;
  [size, pos] = readLeb(buf, pos + 1);
  sections.push({ id, content: Uint8Array.from(buf.subarray(pos, pos + size)) });
  pos += size;
}

let renamed = 0;
for (const section of sections) {
  if (section.id !== 7 /* export */) continue;
  const content = section.content;
  let p = 0;
  let count;
  [count, p] = readLeb(content, p);
  const entries = [];
  for (let i = 0; i < count; i++) {
    let nlen;
    [nlen, p] = readLeb(content, p);
    const name = Buffer.from(content.subarray(p, p + nlen)).toString('utf8');
    p += nlen;
    const kind = content[p++];
    let index;
    [index, p] = readLeb(content, p);
    const next = renames.get(name);
    if (next !== undefined) renamed++;
    entries.push({ name: next ?? name, kind, index });
  }
  const bytes = [...leb(count)];
  for (const { name, kind, index } of entries) {
    const nameBytes = Buffer.from(name, 'utf8');
    bytes.push(...leb(nameBytes.length), ...nameBytes, kind, ...leb(index));
  }
  section.content = Uint8Array.from(bytes);
}

if (renamed !== renames.size) {
  console.error(
    `${inFile}: renamed ${renamed}/${renames.size} exports — is the module's export list as expected?`,
  );
  process.exit(1);
}

const out = [buf.subarray(0, 8)];
for (const { id, content } of sections)
  out.push(Uint8Array.of(id), Uint8Array.from(leb(content.length)), content);
writeFileSync(outFile, Buffer.concat(out.map((b) => Buffer.from(b))));
console.log(
  `${inFile} -> ${outFile} (renamed: ${[...renames].map(([o, n]) => `${o}=${n}`).join(', ')})`,
);
