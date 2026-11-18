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

import { existsSync, readFileSync, rmSync } from 'node:fs';
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

// One scenario: the demo, the clicks (widget ids, in order), and the fake
// clock (the n-th `now` call returns base + n * step; null = wall time).
const scenarios = [
  // The counter: fresh, + + - = 1, and - - - = -3 (the readout joins the
  // minus label in red).
  ['demo-counter', [], null, null],
  ['demo-counter', [2, 2, 4], null, null],
  ['demo-counter', [4, 4, 4], null, null],
  // The stopwatch: paused (never reads the clock), and running on the
  // advancing fake clock (the readout is the fixed 100 ms).
  ['demo-stopwatch', [], null, null],
  ['demo-stopwatch', [3], 1000, 50],
  // The todo: the seeded rows, and the add button's next preset.
  ['demo-todo', [], null, null],
  ['demo-todo', [9], null, null],
];

// The native `--dump` PNG for the scenario: the same events the wasm
// `dump` applies, in the native binary's argv form.
const dumpCounter = { value: 0 };
function nativeFrame(demo, clicks, nowBase, nowStep) {
  const file = `/tmp/waui-host-iced-parity-${process.pid}-${dumpCounter.value++}.png`;
  const args = ['--dump', file];
  if (nowBase !== null) args.push('--now', String(nowBase), String(nowStep));
  for (const id of clicks) args.push('--click', String(id));
  args.push(rel(`${componentsDir}/${demo}.component.wasm`));
  execFileSync(hostIcedBin, args);
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

for (const [demo, clicks, nowBase, nowStep] of scenarios) {
  const want = nativeFrame(demo, clicks, nowBase, nowStep);
  const artifact = readFileSync(rel(`${componentsDir}/${demo}.cwasm`));
  // The u64 clock parameters are BigInts in the wasm ABI (a JS number is
  // not one); null stays null (the `isLikeNone` branch).
  const actual = new Uint8Array(
    iced.dump(
      new Uint8Array(artifact),
      nowBase === null ? null : BigInt(nowBase),
      nowStep === null ? null : BigInt(nowStep),
      new Uint32Array(clicks),
    ),
  );
  assert.ok(
    want.length === actual.length && [...want].every((b, i) => b === actual[i]),
    `${demo} (clicks: ${clicks.join(',')}): the wasm host's PNG differs from the native one:\n${pngDiff(want, actual)}`,
  );
  console.log(`${demo}: ok (clicks: [${clicks.join(', ')}], ${actual.length} bytes)`);
}

console.log('iced selftest: all checks passed');
