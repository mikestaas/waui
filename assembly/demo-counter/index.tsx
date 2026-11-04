// The counter, in AssemblyScript — the same component as
// `crates/demo-counter`, same `waui:ui/app` world.
//
// The UI is a nested tree in `../ui/tree` (the AS twin of the Rust demos'
// `component-ui` crate), rebuilt from state on every export. The box and
// buttons are static. The readout is the only state-dependent node, so it is
// a persistent object: derived from `value` on every build. The handlers
// mutate state only; the diff compares records. It never calls `schedule`, so
// the host never wakes it.
//
//   view:     a column (id 1) with the "+" button (id 2, positive), the
//             readout (id 3; its color is state — negative below zero), and
//             the "-" button (id 4, negative)
//   on-click: 2 increments, 4 decrements
//   on-text:  nothing (a counter has no input)
//   changes:  the changed readout (the whole tree on first contact)
//   on-tick:  nothing (a counter has no clock)
//
// The tree is written in JSX; `../jsx.mjs` lowers it to `h` calls before
// `asc` compiles it (asc's parser has no JSX). `onclick`/`ontext`/`ontick`
// are renamed to the world's hyphenated names after compilation (see
// `../rename-exports.mjs`).

import { cabi_post_changes, cabi_post_view, cabi_realloc } from '../abi/arena';
import {
  COLOR_NEGATIVE,
  COLOR_NEUTRAL,
  COLOR_POSITIVE,
  SIZE_LARGE,
  SIZE_MEDIUM,
} from '../abi/node';
import { Widget, diffOf, h, text, viewOf } from '../ui/tree';

// The world requires both post functions on the module itself, and
// `cabi_realloc` is the canonical allocator both hosts use for the `on-text`
// string buffer (see `../abi/arena`).
export { cabi_post_changes, cabi_post_view, cabi_realloc };

// Widget ids (the contract's identity convention).
const BOX = 1;
const INC = 2;
const READOUT = 3;
const DEC = 4;

var value: i32 = 0;
/// The readout, the only state-dependent node. A module global, so it
/// survives between exports (the conservative GC keeps it for the module's
/// lifetime); the box and buttons are local to the call.
var readout = text('', COLOR_NEUTRAL, SIZE_LARGE).id(READOUT);
readout.reserve(16); // "-2147483648" is 11 bytes

/// The readout's label and color, derived from `value` (negative below zero).
function applyReadout(v: i32): void {
  readout.label(`${v}`).style(v < 0 ? COLOR_NEGATIVE : COLOR_NEUTRAL, SIZE_LARGE);
}

/// The counter's whole UI, nested: the structure is rebuilt, the readout is
/// the persistent leaf.
function tree(v: i32): Widget {
  applyReadout(v);
  return (
    <column id={BOX}>
      <button id={INC} label="+" color={COLOR_POSITIVE} size={SIZE_MEDIUM} />
      {readout}
      <button id={DEC} label="-" color={COLOR_NEGATIVE} size={SIZE_MEDIUM} />
    </column>
  );
}

export function view(): i32 {
  // viewOf records the snapshot (the next diff's reference) and sets
  // everExported.
  return viewOf(tree(value));
}

export function changes(): i32 {
  // diffOf holds the bootstrap rule: the whole tree on first contact,
  // otherwise the changed nodes and the removed ids.
  return diffOf(tree(value));
}

export function onclick(id: i32): void {
  // → `on-click` after the export rename
  const delta = id == INC ? 1 : id == DEC ? -1 : 0;
  // State only: the next build re-derives the readout, and the diff reports
  // it.
  if (delta != 0) {
    value += delta;
  }
}

// → `on-text` after the export rename. The counter has no input, so the
// host's buffer is ignored (the export is required on every component).
export function ontext(_id: i32, _ptr: i32, _len: i32): void {
  // The buffer is a string parameter the callee owns; free it so the scratch
  // region returns to its base (the Rust twin drops its String).
  cabi_realloc(_ptr, _len, 0, 1);
}

export function ontick(): void {} // → `on-tick`; a counter has no clock
