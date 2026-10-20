// A stopwatch, in AssemblyScript — the same component as
// `crates/demo-stopwatch`, speaking the same `waui:ui/app` world.
//
// It demonstrates the *import* side of the world: the component polls
// `now()` (a host-provided clock) and renders elapsed time, driven by the
// host's periodic `on-tick` calls.
//
// Its diff is state-dependent, like its rendering: while running, every
// tick reports the readout; while paused, it reports *nothing* — the
// component simply has no changed nodes.
//
// `onclick`/`ontick` are renamed to the world's hyphenated names after
// compilation (see `../rename-exports.mjs`).

import {
  arenaAlloc,
  cabi_post_changes,
  cabi_post_view,
} from '../abi/arena';
import {
  COLOR_ACCENT,
  COLOR_NEUTRAL,
  COLOR_POSITIVE,
  COLOR_WARNING,
  DIRECTION_COLUMN,
  SIZE_LARGE,
  SIZE_MEDIUM,
  emitButton,
  emitChangedButton,
  emitChangedContainer,
  emitChangedText,
  emitContainer,
  emitText,
  makeList,
} from '../abi/node';
import { writeInt64, writeUtf8 } from '../abi/utf8';
import { now } from '../abi/root';

// The world requires both post functions on the module itself.
export { cabi_post_changes, cabi_post_view };

// Widget ids (the contract's identity convention).
const BOX = 1;
const READOUT = 2;
const BUTTON = 3;

// Host milliseconds of the start of the current run; 0 = not running (a
// host clock starts far above 0, so it doubles as the sentinel).
var start: i64 = 0;
/// Elapsed milliseconds at the last pause.
var frozen: i64 = 0;
var running: i32 = 0;
/// Which nodes are stale since the last export: bit 0 = the readout,
/// bit 1 = the button.
var dirty: i32 = 0;
/// Cleared once the initial tree was exported, via `view` or `changes`.
var exported: i32 = 0;

const D_READOUT = 0b01;
const D_BUTTON = 0b10;

function markExported(): void {
  dirty = 0;
  exported = 1;
}

/// The elapsed milliseconds, as the component currently sees it.
function elapsed(): i64 {
  if (running != 0) {
    if (start == 0) return 0;
    const d = now() - start;
    return d < 0 ? 0 : d; // saturating
  }
  return frozen;
}

// While running the readout glows with the host's accent; the button says
// what it will do (start = good, pause = careful). The elapsed reading is
// sampled once per export (in `view`/`changes`) and passed in, like the
// Rust original: one consistent clock reading per list.
function emitReadout(rec: i32, parent: i32, e: i64): void {
  const str = arenaAlloc(24); // "18446744073709551615 ms" is 23 bytes
  let p = writeInt64(str, e);
  p = writeUtf8(p, ' ms');
  const len = p - str;
  emitText(rec, READOUT, parent, str, len, running != 0 ? COLOR_ACCENT : COLOR_NEUTRAL, SIZE_LARGE);
}

function emitStartPause(rec: i32, parent: i32): void {
  const isRunning = running != 0;
  const str = arenaAlloc(8);
  const len = writeUtf8(str, isRunning ? 'pause' : 'start') - str;
  emitButton(rec, BUTTON, parent, str, len, isRunning ? COLOR_WARNING : COLOR_POSITIVE, SIZE_MEDIUM);
}

export function view(): i32 {
  // A full snapshot means everything is accounted for.
  markExported();
  const e = elapsed();
  const nodes = arenaAlloc(3 * 32);
  emitContainer(nodes + 0 * 32, BOX, 0, DIRECTION_COLUMN);
  emitReadout(nodes + 1 * 32, BOX, e);
  emitStartPause(nodes + 2 * 32, BOX);
  const desc = makeList(nodes, 3);
  return desc;
}

export function changes(): i32 {
  if (exported == 0) {
    // First contact with this host: everything is new to it.
    markExported();
    const e = elapsed();
    const buf = arenaAlloc(3 * 36);
    emitChangedContainer(buf + 0 * 36, BOX, 0, DIRECTION_COLUMN);
    emitChangedReadout(buf + 1 * 36, BOX, e);
    emitChangedStartPause(buf + 2 * 36, BOX);
    const desc = makeList(buf, 3);
    return desc;
  }
  const d = dirty;
  dirty = 0;
  if (d == 0) {
    // Nothing changed (a tick while paused, an unknown click): the host
    // can skip its whole refresh.
    const desc = makeList(0, 0);
    return desc;
  }
  const e = elapsed();
  // NB: parens are load-bearing. AssemblyScript (like JavaScript) binds
  // `!=` tighter than `&`, so an unparenthesized `d & BIT != 0` tests the
  // *constant* side of the comparison, not the bit. Rust binds `&` tighter,
  // which is how this exact expression looks in the original.
  const count = ((d & D_READOUT) != 0 ? 1 : 0) + ((d & D_BUTTON) != 0 ? 1 : 0);
  const buf = arenaAlloc(count * 36);
  let k = 0;
  if ((d & D_READOUT) != 0) {
    emitChangedReadout(buf + k * 36, BOX, e);
    k++;
  }
  if ((d & D_BUTTON) != 0) {
    emitChangedStartPause(buf + k * 36, BOX);
    k++;
  }
  const desc = makeList(buf, count);
  return desc;
}

// The readout projected standing alone, as a `changed` mutation.
function emitChangedReadout(rec: i32, parent: i32, e: i64): void {
  const str = arenaAlloc(24);
  let p = writeInt64(str, e);
  p = writeUtf8(p, ' ms');
  const len = p - str;
  emitChangedText(rec, READOUT, parent, str, len, running != 0 ? COLOR_ACCENT : COLOR_NEUTRAL, SIZE_LARGE);
}

// The button projected standing alone, as a `changed` mutation.
function emitChangedStartPause(rec: i32, parent: i32): void {
  const isRunning = running != 0;
  const str = arenaAlloc(8);
  const len = writeUtf8(str, isRunning ? 'pause' : 'start') - str;
  emitChangedButton(rec, BUTTON, parent, str, len, isRunning ? COLOR_WARNING : COLOR_POSITIVE, SIZE_MEDIUM);
}

export function onclick(id: i32): void { // → `on-click` after the export rename
  if (id != BUTTON) return;
  const t = now();
  if (running != 0) {
    running = 0;
    if (start != 0) {
      const d = t - start;
      frozen = d < 0 ? 0 : d;
    }
  } else {
    start = t;
    running = 1;
  }
  // Starting and stopping both change the readout (its color) and the
  // button (its label and color).
  dirty = D_READOUT | D_BUTTON;
}

export function ontick(): void { // → `on-tick`
  // The readout advances only while running.
  if (running != 0) dirty |= D_READOUT;
}
