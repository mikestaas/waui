// A stopwatch, in AssemblyScript — the same component as
// `crates/demo-stopwatch`, same `waui:ui/app` world. See `../README.md` for
// the AS tree layer, the build pipeline, and the memory layout.
//
// Exercises the world's import side: the component polls `now()` (a host
// clock) and renders elapsed time; it requests its own refresh —
// `schedule` at 250ms while running, 0 while paused — so the host wakes it
// only while running.
//
// Its diff is state-dependent: while running, the readout changes with the
// clock, so each `changes` reports it; while paused, it reports nothing.
// The readout and the button are the state-dependent nodes.
//
// The elapsed reading is sampled once, so one exported list never mixes two
// clock readings.

import { cabi_post_changes, cabi_post_view, cabi_realloc } from '../abi/arena';
import {
  COLOR_ACCENT,
  COLOR_NEUTRAL,
  COLOR_POSITIVE,
  COLOR_WARNING,
  SIZE_LARGE,
  SIZE_MEDIUM,
} from '../abi/node';
import { now, schedule } from '../abi/root';
import { Widget, button, diffOf, h, text, viewOf } from '../ui/tree';

// The canonical-ABI post functions and the on-text allocator (../abi/arena).
export { cabi_post_changes, cabi_post_view, cabi_realloc };

// Widget ids (the contract's identity convention).
const BOX = 1;
const READOUT = 2;
const BUTTON = 3;

/// The requested refresh period while running, in ms.
const REFRESH_PERIOD: i64 = 250;

// Host milliseconds of the start of the current run; 0 = not running (a
// host clock starts far above 0, so it doubles as the sentinel).
var start: i64 = 0;
/// Elapsed milliseconds at the last pause.
var frozen: i64 = 0;
var running: i32 = 0;

/// The two state-dependent nodes. Module globals, so they survive between
/// exports; the box is local to the call.
var readout = text('', COLOR_NEUTRAL, SIZE_LARGE).id(READOUT);
var control = button('start', COLOR_POSITIVE, SIZE_MEDIUM).id(BUTTON);
readout.reserve(24); // "18446744073709551615 ms" is 23 bytes

/// The elapsed milliseconds, as the component currently sees it.
function elapsed(): i64 {
  if (running != 0) {
    if (start == 0) return 0;
    const d = now() - start;
    return d < 0 ? 0 : d; // saturating
  }
  return frozen;
}

/// The readout takes the host's accent color while running; the button's
/// label and color name the action.
function apply(e: i64, r: i32): void {
  readout.label(`${e} ms`).style(r != 0 ? COLOR_ACCENT : COLOR_NEUTRAL, SIZE_LARGE);
  control
    .label(r != 0 ? 'pause' : 'start')
    .style(r != 0 ? COLOR_WARNING : COLOR_POSITIVE, SIZE_MEDIUM);
}

/// The stopwatch's whole UI, nested: the structure is rebuilt, the readout
/// and the button are the persistent leaves.
function tree(e: i64, r: i32): Widget {
  apply(e, r);
  return (
    <column id={BOX}>
      {readout}
      {control}
    </column>
  );
}

export function view(): i32 {
  return viewOf(tree(elapsed(), running));
}

export function changes(): i32 {
  return diffOf(tree(elapsed(), running));
}

export function onclick(id: i32): void {
  // → `on-click` after the export rename
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
  schedule(running != 0 ? REFRESH_PERIOD : 0);
}

// → `on-tick` after the export rename.
export function ontick(): void {}

// → `on-text` after the export rename. The stopwatch has no input, so the
// buffer is unused; it is freed (../abi/arena).
export function ontext(_id: i32, _ptr: i32, _len: i32): void {
  cabi_realloc(_ptr, _len, 0, 1);
}
