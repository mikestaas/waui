// A stopwatch, in AssemblyScript — the same component as
// `crates/demo-stopwatch`, same `waui:ui/app` world.
//
// Exercises the world's import side: the component polls `now()` (a host
// clock) and renders elapsed time, driven by the host's `on-tick`.
//
// The diff is state-dependent, as in the Rust original: while running, every
// tick dirties the readout; while paused, nothing. The dirty state is a bit
// mask over the two state-dependent nodes, and `changes` re-projects the
// dirty subtrees from state.
//
// The UI is a nested tree in `../ui/tree`, rebuilt from state per export;
// the elapsed reading is sampled once, so one exported list never mixes two
// clock readings.
//
// `onclick`/`ontick` are renamed to the world's hyphenated names after
// compilation (see `../rename-exports.mjs`).

import {
  cabi_post_changes,
  cabi_post_view,
} from '../abi/arena';
import {
  COLOR_ACCENT,
  COLOR_NEUTRAL,
  COLOR_POSITIVE,
  COLOR_WARNING,
  SIZE_LARGE,
  SIZE_MEDIUM,
  makeList,
} from '../abi/node';
import { now } from '../abi/root';
import {
  Widget,
  button,
  column,
  mutationsOf,
  mutationsOfAll,
  text,
  viewOf,
} from '../ui/tree';

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

const BIT_READOUT = 1;
const BIT_BUTTON = 2;

/// Record that the current state has just been exported (as a full
/// snapshot or as a diff): nothing is pending any more.
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

// The readout takes the host's accent color while running; the button's
// label and color say what it will do.
function readout(e: i64, running: i32): Widget {
  return text(
    `${e} ms`,
    running != 0 ? COLOR_ACCENT : COLOR_NEUTRAL,
    SIZE_LARGE,
  ).id(READOUT).reserve(24); // "18446744073709551615 ms" is 23 bytes
}

function control(running: i32): Widget {
  return button(
    running != 0 ? 'pause' : 'start',
    running != 0 ? COLOR_WARNING : COLOR_POSITIVE,
    SIZE_MEDIUM,
  ).id(BUTTON);
}

/// The stopwatch's whole UI, nested.
function tree(e: i64, running: i32): Widget {
  return column([readout(e, running), control(running)]).id(BOX);
}

export function view(): i32 {
  const t = tree(elapsed(), running);
  // A full snapshot means everything is accounted for.
  markExported();
  return viewOf(t);
}

export function changes(): i32 {
  if (exported == 0) {
    // First contact with this host: everything is new to it.
    const t = tree(elapsed(), running);
    markExported();
    return mutationsOf(t, 0);
  }
  const d = dirty;
  dirty = 0;
  if (d == 0) {
    // Nothing changed (a tick while paused, an unknown click): the host can
    // skip its whole refresh.
    return makeList(0, 0);
  }
  const e = elapsed(); // one reading for the whole projection
  // Gather exactly the dirty subtrees.
  let k = 0;
  if ((d & BIT_READOUT) != 0) k++;
  if ((d & BIT_BUTTON) != 0) k++;
  const roots = new Array<Widget>(k);
  let w = 0;
  if ((d & BIT_READOUT) != 0) roots[w++] = readout(e, running);
  if ((d & BIT_BUTTON) != 0) roots[w++] = control(running);
  // Projected standing alone, each states its parent explicitly (inside
  // `view`, the lowering walk states it).
  return mutationsOfAll(roots, BOX);
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
  dirty = BIT_READOUT | BIT_BUTTON;
}

export function ontick(): void { // → `on-tick`
  // The readout advances only while running.
  if (running != 0) dirty |= BIT_READOUT;
}
