// Headless byte-parity gate for the browser TUI host (Node): the wasm
// host's `dump` frame must be byte-identical to the native `host-tui
// --dump` frame — same component, same clicks, same fake clock.
//
//   node web/tui/selftest.mjs [host-tui binary] [components dir]
//
// The wasm host runs the component on the Pulley interpreter (the artifact
// is precompiled natively for pulley32 by host-precompile); the native host
// runs it on Cranelift. The frame is a pure function of the tree, the
// windows, the width, and the registry, so the two must agree. The scenarios
// mirror the native golden tests (crates/host-tui/tests/dump.rs): the
// counter's sign colors, the stopwatch's clock readout, the todo's rows.

import { existsSync, readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

// Resolve paths relative to the repository root (this file lives in
// web/tui/); absolute paths (a non-default layout) pass through.
const rel = (p) => (p.startsWith('/') ? p : fileURLToPath(new URL(`../../${p}`, import.meta.url)));
const [hostTuiBin, componentsDir] = [
  process.argv[2] ?? 'target/debug/host-tui',
  process.argv[3] ?? 'target/components',
];

const pkgWasm = rel('target/host-tui/pkg/host_tui_bg.wasm');
if (!existsSync(pkgWasm)) {
  console.error('missing the browser host build — run: make host-tui-wasm');
  process.exit(1);
}
if (!existsSync(hostTuiBin)) {
  console.error(`missing the native host binary — run: cargo build -p host-tui`);
  process.exit(1);
}

const tui = await import(rel('target/host-tui/pkg/host_tui.js'));
tui.initSync({ module: readFileSync(pkgWasm) });

// One scenario: the demo, the clicks (widget ids, in order), the fake clock
// (the n-th `now` call returns base + n * step; null = wall time), and the
// buffer width (null = the frame's own size).
const scenarios = [
  // The counter: fresh, + + - = 1, - - - = -3, and a 40-column wrap.
  ['demo-counter', [], null, null, null],
  ['demo-counter', [2, 2, 4], null, null, null],
  ['demo-counter', [4, 4, 4], null, null, null],
  ['demo-counter', [], null, null, 40],
  // The stopwatch: paused (never reads the clock), and running on the
  // advancing fake clock (the readout is the fixed 100 ms).
  ['demo-stopwatch', [], null, null, null],
  ['demo-stopwatch', [3], 1000, 50, null],
  // The todo: the seeded rows, and the add button's next preset.
  ['demo-todo', [], null, null, null],
  ['demo-todo', [9], null, null, null],
];

// The native `--dump` frame for the scenario: the same events the wasm
// `dump` applies, in the native binary's argv form.
function nativeFrame(demo, clicks, nowBase, nowStep, width) {
  const args = ['--dump'];
  if (width !== null) args.push('--width', String(width));
  if (nowBase !== null) args.push('--now', String(nowBase), String(nowStep));
  for (const id of clicks) args.push('--click', String(id));
  args.push(rel(`${componentsDir}/${demo}.component.wasm`));
  const out = execFileSync(hostTuiBin, args);
  return new TextDecoder().decode(out);
}

// The first differing lines, for a failure message.
function frameDiff(want, actual) {
  const wantLines = want.split('\n');
  const actualLines = actual.split('\n');
  const lines = [];
  for (let i = 0; i < Math.max(wantLines.length, actualLines.length); i++) {
    if (wantLines[i] !== actualLines[i]) {
      lines.push(
        `line ${i}:\n  native: ${JSON.stringify(wantLines[i])}\n  wasm:   ${JSON.stringify(actualLines[i])}`,
      );
      if (lines.length === 5) break;
    }
  }
  if (wantLines.length !== actualLines.length) {
    lines.push(
      `the native frame has ${wantLines.length} lines, the wasm frame has ${actualLines.length}`,
    );
  }
  return lines.join('\n');
}

for (const [demo, clicks, nowBase, nowStep, width] of scenarios) {
  const want = nativeFrame(demo, clicks, nowBase, nowStep, width);
  const artifact = readFileSync(rel(`${componentsDir}/${demo}.cwasm`));
  // The u64 clock parameters are BigInts in the wasm ABI (a JS number is
  // not one); null stays null (the `isLikeNone` branch).
  const actual = tui.dump(
    new Uint8Array(artifact),
    width,
    nowBase === null ? null : BigInt(nowBase),
    nowStep === null ? null : BigInt(nowStep),
    new Uint32Array(clicks),
  );
  assert.equal(
    actual,
    want,
    `${demo} (clicks: ${clicks.join(',')}): the wasm host's frame differs from the native one:\n${frameDiff(want, actual)}`,
  );
  console.log(`${demo}: ok (clicks: [${clicks.join(', ')}])`);
}

console.log('tui selftest: all checks passed');
