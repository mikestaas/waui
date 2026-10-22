// The counter, in AssemblyScript — the same component as
// `crates/demo-counter`, speaking the same `waui:ui/app` world.
//
// The UI is written as a nested *tree* in `../ui/tree` — the AssemblyScript
// twin of the `component-ui` crate the Rust demos use: real objects on the
// runtime heap, nested the same way the Rust demo writes them. This file is
// state, event handling, and the two projection calls; the diff is the
// demo's own bookkeeping, exactly as in the Rust original.
//
// Behavior mirrors the Rust original exactly:
//   view:     a column (id 1) with the "+" button (id 2, positive), the
//             readout (id 3; its color is state — negative below zero), and
//             the "−" button (id 4, negative).
//   on-click: 2 increments, 4 decrements; each marks the readout stale.
//   changes:  the one stale readout — never the whole tree.
//   on-tick:  nothing (a counter has no clock).
//
// The tree is rebuilt from state on every export (the Rust model — state is
// the source of truth); the tree layer projects it, and the label bytes it
// points at come from the tree's static pool, so a rebuild never moves what
// a projected record points at.
//
// The two exports whose world names contain hyphens (`on-click`, `on-tick`)
// are authored here as `onclick`/`ontick` — AssemblyScript identifiers
// cannot contain hyphens — and renamed in the module after compilation
// (see `../rename-exports.mjs`).

import {
  cabi_post_changes,
  cabi_post_view,
} from '../abi/arena';
import {
  COLOR_NEGATIVE,
  COLOR_NEUTRAL,
  COLOR_POSITIVE,
  SIZE_LARGE,
  SIZE_MEDIUM,
  makeList,
} from '../abi/node';
import {
  Widget,
  button,
  column,
  mutationsOf,
  text,
  viewOf,
} from '../ui/tree';

// The world requires both post functions on the module itself.
export { cabi_post_changes, cabi_post_view };

// Widget ids (the contract's identity convention).
const BOX = 1;
const INC = 2;
const READOUT = 3;
const DEC = 4;

var value: i32 = 0;
/// The readout is stale (it is the only state-dependent node).
var dirty: i32 = 0;
/// Cleared once the initial tree was exported, via `view` or `changes`.
var exported: i32 = 0;

/// Record that the current state has just been exported (as a full
/// snapshot or as a diff): nothing is pending any more.
function markExported(): void {
  dirty = 0;
  exported = 1;
}

// The readout is the only node that depends on `value`; its color is state
// (negative below zero). The buttons and the box are static and so need no
// attention in the diff.
function readout(v: i32): Widget {
  const w = text(`${v}`, v < 0 ? COLOR_NEGATIVE : COLOR_NEUTRAL, SIZE_LARGE);
  w.id = READOUT;
  w.reserve(16); // "-2147483648" is 11 bytes
  return w;
}

/// The counter's whole UI, nested — the same expression the Rust demo writes.
function tree(v: i32): Widget {
  const inc = button('+', COLOR_POSITIVE, SIZE_MEDIUM);
  inc.id = INC;
  const dec = button('-', COLOR_NEGATIVE, SIZE_MEDIUM);
  dec.id = DEC;
  const box = column([inc, readout(v), dec]);
  box.id = BOX;
  return box;
}

export function view(): i32 {
  const t = tree(value);
  // A full snapshot means everything is accounted for.
  markExported();
  return viewOf(t);
}

export function changes(): i32 {
  if (exported == 0) {
    // First contact with this host: it has seen nothing, so everything is
    // new to it. (A host may bootstrap from `changes` alone.)
    const t = tree(value);
    markExported();
    return mutationsOf(t, 0);
  }
  if (dirty == 0) {
    // A no-op event (a tick, a click on an unknown id): there is nothing
    // for the host to do.
    return makeList(0, 0);
  }
  dirty = 0;
  // Projected standing alone, so its parent is stated explicitly (the
  // lowering walk states it for us inside `view`).
  return mutationsOf(readout(value), BOX);
}

export function onclick(id: i32): void { // → `on-click` after the export rename
  const delta = id == INC ? 1 : id == DEC ? -1 : 0;
  if (delta != 0) {
    value += delta;
    dirty = 1;
  }
}

export function ontick(): void {} // → `on-tick`; a counter has no clock
