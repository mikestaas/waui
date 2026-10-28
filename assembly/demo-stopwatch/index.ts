// A stopwatch, in AssemblyScript — the same component as
// `crates/demo-stopwatch`, same `waui:ui/app` world.
//
// Exercises the world's import side: the component polls `now()` (a host
// clock) and renders elapsed time; it also requests its own refresh, so the
// host wakes it only while running.
//
// The diff is state-dependent, as in the Rust original: while running, the
// readout's value changes with the clock, so each `changes` reports it;
// while paused, nothing does. The readout and the button are the
// state-dependent nodes: persistent objects, derived on every build, marked
// stale by their modifiers when the value changes. The handlers hold no
// diff state; the running toggle also requests the refresh.
//
// The UI is a nested tree in `../ui/tree`, rebuilt from state per export;
// the elapsed reading is sampled once, so one exported list never mixes two
// clock readings.
//
// `onclick`/`ontick` are renamed to the world's hyphenated names after
// compilation (see `../rename-exports.mjs`).

import { cabi_post_changes, cabi_post_view } from '../abi/arena';
import {
  COLOR_ACCENT,
  COLOR_NEUTRAL,
  COLOR_POSITIVE,
  COLOR_WARNING,
  SIZE_LARGE,
  SIZE_MEDIUM,
} from '../abi/node';
import { now, schedule } from '../abi/root';
import { Widget, button, column, diffOf, text, viewOf } from '../ui/tree';

// The world requires both post functions on the module itself.
export { cabi_post_changes, cabi_post_view };

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

/// The two state-dependent nodes. Module globals, so their dirty marks
/// survive between exports (the conservative GC keeps them for the module's
/// lifetime); the box is local to the call.
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
/// label and color name the action. Both derive from the sampled state, on
/// every build.
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
  return column([readout, control]).id(BOX);
}

export function view(): i32 {
  // `elapsed()` sampled once: one exported list never mixes two readings.
  return viewOf(tree(elapsed(), running));
}

export function changes(): i32 {
  // Same one sample; the layer diffs and clears the marks.
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
  // The refresh follows `running`: the host wakes the component at the
  // period while running, never while paused.
  schedule(running != 0 ? REFRESH_PERIOD : 0);
}

// → `on-tick` after the export rename. A tick changes no state: while
// running, the next `changes` re-derives the readout and reports it; while
// paused, the value is frozen and the diff is empty.
export function ontick(): void {}
