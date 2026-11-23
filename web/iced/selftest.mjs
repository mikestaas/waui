// Headless byte-parity gate for the browser iced host (Node): the wasm
// host's `dump` PNG must be byte-identical to the native `host-iced
// --dump` PNG — same component, same clicks, same fake clock.
//
//   node web/iced/selftest.mjs [host-iced binary] [components dir]
//
// The wasm host runs the component on the Pulley interpreter (the artifact
// is precompiled natively for pulley32 by host-precompile) and renders the
// frame through iced's software `tiny-skia` backend; the native host runs
// it on Cranelift and renders through the same backend. The frame is a
// pure function of the tree, the panels, the canvas size, and the
// registry, and the font database is pinned to the bundled fonts, so the
// two must agree. The scenarios mirror the native golden tests
// (crates/host-iced/tests/dump.rs): the counter's sign colors, the
// stopwatch's clock readout, the todo's rows.

import { existsSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

// Resolve paths relative to the repository root (this file lives in
// web/iced/); absolute paths (a non-default layout) pass through.
const rel = (p) => (p.startsWith('/') ? p : fileURLToPath(new URL(`../../${p}`, import.meta.url)));
const [hostIcedBin, componentsDir] = [
  process.argv[2] ?? 'target/debug/host-iced',
  process.argv[3] ?? 'target/components',
];

const pkgWasm = rel('target/host-iced/pkg/host_iced_bg.wasm');
if (!existsSync(pkgWasm)) {
  console.error('missing the browser host build — run: make host-iced-wasm');
  process.exit(1);
}
if (!existsSync(hostIcedBin)) {
  console.error(`missing the native host binary — run: cargo build -p host-iced`);
  process.exit(1);
}

const iced = await import(rel('target/host-iced/pkg/host_iced.js'));
iced.initSync({ module: readFileSync(pkgWasm) });

// One scenario: the demo, the clicks (widget ids, in order), the
// double-clicks (widget ids, in order), the fake clock (the n-th `now`
// call returns base + n * step; null = wall time), the registry manifest
// (a path, or null), and the icon assets (a directory of files, or null).
// The last two are the desktop's: the browser has no filesystem, so the
// page hands the manifest and the icons over. The entry components' bytes
// are derived from the manifest (the page's background fetch, as bytes).
const scenarios = [
  // The counter: fresh, + + - = 1, and - - - = -3 (the readout joins the
  // minus label in red).
  ['demo-counter', [], null, null, null, null, null],
  ['demo-counter', [2, 2, 4], null, null, null, null, null],
  ['demo-counter', [4, 4, 4], null, null, null, null, null],
  // The stopwatch: paused (never reads the clock), and running on the
  // advancing fake clock (the readout is the fixed 100 ms).
  ['demo-stopwatch', [], null, null, null, null, null],
  ['demo-stopwatch', [3], null, 1000, 50, null, null],
  // The todo: the seeded rows, and the add button's next preset.
  ['demo-todo', [], null, null, null, null, null],
  ['demo-todo', [9], null, null, null, null, null],
  // The desktop: the registry's icon grid (no windows), and one window
  // (a double-click on the counter's tile opens it). The manifest, the
  // icons, and the nine entry components come from the repo; the native
  // host reads the same files through the filesystem.
  ['demo-desktop', [], null, null, null, 'registry.json', 'site/icons'],
  ['demo-desktop', [], [100], null, null, 'registry.json', 'site/icons'],
];

// The desktop's registry manifest text (the native host reads the same file).
function manifestText(p) {
  return p === null ? null : readFileSync(rel(p), 'utf8');
}

// The desktop's icons, base64-encoded, keyed by their path (the `src` the
// component uses). The browser has no filesystem, so the page hands them
// over; `dir` is a directory of files, relative to the repo root.
function assetsJson(dir) {
  if (dir === null) return null;
  const abs = rel(dir);
  const files = readdirSync(abs).filter((f) => f.endsWith('.svg'));
  const obj = {};
  for (const f of files) {
    obj[`${dir}/${f}`] = readFileSync(`${abs}/${f}`).toString('base64');
  }
  return JSON.stringify(obj);
}

// The desktop's entry components, base64-encoded, keyed by the manifest's
// `path` (what the host's open-entry looks up). The browser has no
// filesystem, so the page hands the artifacts over; the selftest reads the
// same precompiled files the page fetches (the manifest's `path` is the
// native component file, the site serves the precompiled one under the
// same base name).
function componentsJson(manifest, componentsDir) {
  if (manifest === null) return null;
  const m = JSON.parse(readFileSync(rel(manifest), 'utf8'));
  const obj = {};
  for (const e of m.entries) {
    const base = e.path
      .split('/')
      .pop()
      .replace(/\.component\.wasm$/, '.cwasm');
    obj[e.path] = readFileSync(`${rel(componentsDir)}/${base}`).toString('base64');
  }
  return JSON.stringify(obj);
}

// The native `--dump` PNG for the scenario: the same events the wasm
// `dump` applies, in the native binary's argv form. It runs from the repo
// root so the manifest's relative paths (and the desktop's icons) resolve.
const dumpCounter = { value: 0 };
function nativeFrame(demo, clicks, doubleClicks, nowBase, nowStep, manifest) {
  const file = `/tmp/waui-host-iced-parity-${process.pid}-${dumpCounter.value++}.png`;
  const args = ['--dump', file];
  if (nowBase !== null) args.push('--now', String(nowBase), String(nowStep));
  for (const id of clicks) args.push('--click', String(id));
  for (const id of doubleClicks ?? []) args.push('--double-click', String(id));
  if (manifest !== null) args.push('--registry', manifest);
  args.push(rel(`${componentsDir}/${demo}.component.wasm`));
  execFileSync(hostIcedBin, args, { cwd: rel('.') });
  const bytes = readFileSync(file);
  rmSync(file, { force: true });
  return bytes;
}

// The first differing bytes, for a failure message.
function pngDiff(want, actual) {
  if (want.length !== actual.length) {
    return `the PNG sizes differ: native ${want.length} bytes, wasm ${actual.length} bytes`;
  }
  let i = 0;
  while (i < want.length && want[i] === actual[i]) i++;
  if (i === want.length) return 'the PNGs are identical (unexpected)';
  const ctx = (n) => [...n].map((b) => b.toString(16).padStart(2, '0')).join(' ');
  return (
    `first difference at byte ${i} of ${want.length}:\n` +
    `  native: ${ctx(want.slice(i, i + 16))}\n` +
    `  wasm:   ${ctx(actual.slice(i, i + 16))}`
  );
}

for (const [demo, clicks, doubleClicks, nowBase, nowStep, manifest, assets] of scenarios) {
  const want = nativeFrame(demo, clicks, doubleClicks, nowBase, nowStep, manifest);
  const artifact = readFileSync(rel(`${componentsDir}/${demo}.cwasm`));
  // The u64 clock parameters are BigInts in the wasm ABI (a JS number is
  // not one); null stays null (the `isLikeNone` branch). The manifest, the
  // icon bytes, and the entry components are strings (or null), straight
  // through.
  const actual = new Uint8Array(
    iced.dump(
      new Uint8Array(artifact),
      nowBase === null ? null : BigInt(nowBase),
      nowStep === null ? null : BigInt(nowStep),
      new Uint32Array(clicks),
      new Uint32Array(doubleClicks ?? []),
      manifestText(manifest),
      assetsJson(assets),
      componentsJson(manifest, componentsDir),
    ),
  );
  assert.ok(
    want.length === actual.length && [...want].every((b, i) => b === actual[i]),
    `${demo} (clicks: [${clicks.join(',')}], double-clicks: [${(doubleClicks ?? []).join(
      ',',
    )}]): the wasm host's PNG differs from the native one:\n${pngDiff(want, actual)}`,
  );
  console.log(`${demo}: ok (clicks: [${clicks.join(', ')}], ${actual.length} bytes)`);
}

console.log('iced selftest: all checks passed');
