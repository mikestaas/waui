// A stopwatch, in AssemblyScript — the same component as
// `crates/demo-stopwatch`, speaking the same `waui:ui/app` world.
//
// It demonstrates the *import* side of the world: the component polls
// `now()` (a host-provided clock) and renders elapsed time, driven by the
// host's periodic `on-tick` calls.
//
// Its diff is state-dependent, like its rendering: while running, every
// tick marks the readout stale; while paused, it marks *nothing* — the
// component simply has no changed nodes.
//
// The UI is a tree in `../ui/tree` (the AssemblyScript twin of the Rust
// demos' `component-ui` crate); this file is state, event handling, and a
// refresh that re-derives the two state-dependent nodes. The readout
// "123 ms" is composed per refresh (an integer plus a literal) — the
// append API keeps the whole label in the tree's reserved buffer.
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
} from '../abi/node';
import { now } from '../abi/root';
import {
  appendInt64,
  appendText,
  beginButton,
  beginColumn,
  beginText,
  changes as lowerChanges,
  end,
  id,
  label,
  labelReset,
  mark,
  reserve,
  setColor,
  setText,
  size,
  style,
  view as lowerView,
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
var built: i32 = 0;
/// Slot handles in the tree (a `begin*` return value).
var readoutSlot: i32 = -1;
var buttonSlot: i32 = -1;

// The stopwatch's whole UI. Built once (lazily, before the first export)
// and updated in place from then on. The readout is built with an empty
// buffer of the size a full u64 reading needs ("18446744073709551615 ms"
// is 23 bytes); the first refresh fills it.
function buildTree(): void {
  beginColumn();
  id(BOX);
  readoutSlot = beginText();
  id(READOUT);
  reserve(24);
  size(SIZE_LARGE); // the size is state-independent; only the color moves
  buttonSlot = beginButton();
  id(BUTTON);
  label('start'); // the initial state's label; refresh rewrites it
  style(COLOR_POSITIVE, SIZE_MEDIUM);
  end();
}

function ensureBuilt(): void {
  if (built == 0) {
    built = 1;
    buildTree();
  }
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

// Both the readout (content and color) and the button (label and color)
// depend on state. The elapsed reading is sampled *once* per refresh, so
// one exported list never mixes two clock readings.
function refresh(): void {
  const e = elapsed();
  labelReset(readoutSlot);
  appendInt64(readoutSlot, e);
  appendText(readoutSlot, ' ms');
  setColor(readoutSlot, running != 0 ? COLOR_ACCENT : COLOR_NEUTRAL);
  setText(buttonSlot, running != 0 ? 'pause' : 'start');
  setColor(buttonSlot, running != 0 ? COLOR_WARNING : COLOR_POSITIVE);
}

export function view(): i32 {
  ensureBuilt();
  refresh();
  return lowerView();
}

export function changes(): i32 {
  ensureBuilt();
  refresh();
  return lowerChanges();
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
  mark(readoutSlot);
  mark(buttonSlot);
}

export function ontick(): void { // → `on-tick`
  // The readout advances only while running.
  if (running != 0) mark(readoutSlot);
}
