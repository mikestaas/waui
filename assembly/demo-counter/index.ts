// The counter, in AssemblyScript — the same component as
// `crates/demo-counter`, speaking the same `waui:ui/app` world.
//
// The UI is written as a *tree* in `../ui/tree` — the AssemblyScript twin
// of the `component-ui` crate the Rust demos use — rather than as a
// hand-ordered flat list with hand-assigned parent ids. This file is
// state, event handling, and a two-line refresh; the projection of the
// tree into the contract's flat node list (and its diff channel) is the
// tree layer's job, on top of the `abi/` emitters.
//
// Behavior mirrors the Rust original exactly:
//   view:    a column (id 1) with the "+" button (id 2, positive), the
//            readout (id 3; its color is state — negative below zero), and
//            the "−" button (id 4, negative).
//   on-click: 2 increments, 4 decrements; each marks the readout stale.
//   changes:  the one stale readout — never the whole tree.
//   on-tick:  nothing (a counter has no clock).
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
} from '../abi/node';
import {
  beginButton,
  beginColumn,
  beginText,
  changes as lowerChanges,
  end,
  id,
  label,
  mark,
  reserve,
  setColor,
  setInt32,
  size,
  style,
  view as lowerView,
} from '../ui/tree';

// The world requires both post functions on the module itself.
export { cabi_post_changes, cabi_post_view };

// Widget ids (the contract's identity convention).
const BOX = 1;
const INC = 2;
const READOUT = 3;
const DEC = 4;

var value: i32 = 0;
var built: i32 = 0;
/// The readout's slot handle in the tree (a `begin*` return value).
var readoutSlot: i32 = -1;

// The counter's whole UI. Built once (lazily, before the first export —
// a lifted component may have no start hook to build in) and updated in
// place from then on. The scope is the tree: children indent under their
// container, and each names it by the id set above it.
function buildTree(): void {
  beginColumn();
  id(BOX);
  beginButton();
  id(INC);
  label('+');
  style(COLOR_POSITIVE, SIZE_MEDIUM);
  readoutSlot = beginText();
  id(READOUT);
  reserve(16); // "-2147483648" is 11 bytes
  size(SIZE_LARGE);
  beginButton();
  id(DEC);
  label('-');
  style(COLOR_NEGATIVE, SIZE_MEDIUM);
  end();
}

function ensureBuilt(): void {
  if (built == 0) {
    built = 1;
    buildTree();
  }
}

// The readout is the only node that depends on `value`; its color is
// state (negative below zero). The buttons and the box are static and so
// need no refresh.
function refresh(): void {
  setInt32(readoutSlot, value);
  setColor(readoutSlot, value < 0 ? COLOR_NEGATIVE : COLOR_NEUTRAL);
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
  if (id == INC) {
    value++;
    mark(readoutSlot);
  } else if (id == DEC) {
    value--;
    mark(readoutSlot);
  }
}

export function ontick(): void {} // → `on-tick`; a counter has no clock
