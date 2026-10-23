// The counter, in AssemblyScript — the same component as
// `crates/demo-counter`, same `waui:ui/app` world.
//
// The UI is a nested tree in `../ui/tree` (the AS twin of the Rust demos'
// `component-ui` crate). The box and both buttons are static and are rebuilt
// from state on every export, as in the Rust original; the readout is the
// only state-dependent node, so it is a persistent object: its label and
// color are derived from `value` on every build, and the `label()`/`style()`
// modifiers mark it stale only when the value actually changes. The handlers
// mutate state and nothing else; `changes` is a single `diffOf` call.
//
//   view:     a column (id 1) with the "+" button (id 2, positive), the
//             readout (id 3; its color is state — negative below zero), and
//             the "-" button (id 4, negative)
//   on-click: 2 increments, 4 decrements
//   changes:  the stale readout (the whole tree on first contact)
//   on-tick:  nothing (a counter has no clock)
//
// `onclick`/`ontick` are renamed to the world's hyphenated names after
// compilation (see `../rename-exports.mjs`).

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
  Widget,
  button,
  column,
  diffOf,
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
/// The readout, the only state-dependent node: held in a module global so
/// its dirty mark survives between exports. The conservative GC keeps it for
/// the module's lifetime (a few bytes, intended); the box and the buttons
/// stay local to the call that builds them.
var readout = text('', COLOR_NEUTRAL, SIZE_LARGE).id(READOUT);
readout.reserve(16); // "-2147483648" is 11 bytes

/// The readout's label and color derive from `value` (negative below zero);
/// the buttons and the box are static and so need no attention in the diff.
function applyReadout(v: i32): void {
  readout
    .label(`${v}`)
    .style(v < 0 ? COLOR_NEGATIVE : COLOR_NEUTRAL, SIZE_LARGE);
}

/// The counter's whole UI, nested: the structure is rebuilt, the readout is
/// the persistent leaf.
function tree(v: i32): Widget {
  applyReadout(v);
  return column([
    button('+', COLOR_POSITIVE, SIZE_MEDIUM).id(INC),
    readout,
    button('-', COLOR_NEGATIVE, SIZE_MEDIUM).id(DEC),
  ]).id(BOX);
}

export function view(): i32 {
  // A full snapshot accounts for everything (the layer clears the marks).
  return viewOf(tree(value));
}

export function changes(): i32 {
  // The layer holds the bootstrap rule and the diff: first contact exports
  // the whole tree; after, exactly the stale nodes, marks cleared.
  return diffOf(tree(value));
}

export function onclick(id: i32): void { // → `on-click` after the export rename
  const delta = id == INC ? 1 : id == DEC ? -1 : 0;
  // The state transition is all the click does: the next build re-derives
  // the readout, and the modifiers mark it stale only if the value changed.
  if (delta != 0) {
    value += delta;
  }
}

export function ontick(): void {} // → `on-tick`; a counter has no clock
