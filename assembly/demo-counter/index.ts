// The counter, in AssemblyScript — the same component as
// `crates/demo-counter`, speaking the same `waui:ui/app` world.
//
// No `wit-bindgen` exists for AssemblyScript, so instead of generated
// bindings this file writes the canonical ABI by hand through the shared
// layer in `../abi`: a flat, pre-ordered node list with parent ids (the
// `waui:ui/widget` contract), a diff channel, and no imports at all — the
// counter never calls a clock.
//
// Behavior mirrors the Rust original exactly:
//   view:    a column (id 1) with the "+" button (id 2, positive), the
//            readout (id 3; its color is state — negative below zero), and
//            the "−" button (id 4, negative).
//   on-click: 2 increments, 4 decrements; each dirties the readout only.
//   changes:  the one changed readout — never the whole tree.
//   on-tick:  nothing (a counter has no clock).
//
// The two exports whose world names contain hyphens (`on-click`, `on-tick`)
// are authored here as `onclick`/`ontick` — AssemblyScript identifiers
// cannot contain hyphens — and renamed in the module after compilation
// (see `../rename-exports.mjs`).

import {
  arenaAlloc,
  cabi_post_changes,
  cabi_post_view,
} from '../abi/arena';
import {
  COLOR_NEGATIVE,
  COLOR_NEUTRAL,
  COLOR_POSITIVE,
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
import { writeInt32, writeUtf8 } from '../abi/utf8';

// The world requires both post functions on the module itself.
export { cabi_post_changes, cabi_post_view };

// Widget ids (the contract's identity convention).
const BOX = 1;
const INC = 2;
const READOUT = 3;
const DEC = 4;

var value: i32 = 0;
/// Set when the state changed since it was last exported.
var dirty: i32 = 0;
/// Cleared once the initial tree was exported, via `view` or `changes`.
var exported: i32 = 0;

function markExported(): void {
  dirty = 0;
  exported = 1;
}

// The readout is the only node that depends on `value`; its color is state
// (negative below zero).
function emitReadout(rec: i32, parent: i32): void {
  const v = value;
  const str = arenaAlloc(16); // "-2147483648" is 11 bytes
  const len = writeInt32(str, v) - str;
  emitText(rec, READOUT, parent, str, len, v < 0 ? COLOR_NEGATIVE : COLOR_NEUTRAL, SIZE_LARGE);
}

function emitPlus(rec: i32, parent: i32): void {
  const str = arenaAlloc(8);
  const len = writeUtf8(str, '+') - str;
  emitButton(rec, INC, parent, str, len, COLOR_POSITIVE, SIZE_MEDIUM);
}

function emitMinus(rec: i32, parent: i32): void {
  const str = arenaAlloc(8);
  const len = writeUtf8(str, '-') - str;
  emitButton(rec, DEC, parent, str, len, COLOR_NEGATIVE, SIZE_MEDIUM);
}

export function view(): i32 {
  // A full snapshot means everything is accounted for.
  markExported();
  const nodes = arenaAlloc(4 * 32);
  emitContainer(nodes + 0 * 32, BOX, 0, DIRECTION_COLUMN);
  emitPlus(nodes + 1 * 32, BOX);
  emitReadout(nodes + 2 * 32, BOX);
  emitMinus(nodes + 3 * 32, BOX);
  return makeList(nodes, 4);
}

export function changes(): i32 {
  if (exported == 0) {
    // First contact with this host: it has seen nothing, so everything is
    // new to it. (A host may bootstrap from `changes` alone.)
    markExported();
    const buf = arenaAlloc(4 * 36);
    emitChangedContainer(buf + 0 * 36, BOX, 0, DIRECTION_COLUMN);
    const s1 = arenaAlloc(8);
    const l1 = writeUtf8(s1, '+') - s1;
    emitChangedButton(buf + 1 * 36, INC, BOX, s1, l1, COLOR_POSITIVE, SIZE_MEDIUM);
    emitChangedReadout(buf + 2 * 36, BOX);
    const s2 = arenaAlloc(8);
    const l2 = writeUtf8(s2, '-') - s2;
    emitChangedButton(buf + 3 * 36, DEC, BOX, s2, l2, COLOR_NEGATIVE, SIZE_MEDIUM);
    return makeList(buf, 4);
  }
  if (dirty == 0) {
    // A no-op event (a tick, a click on an unknown id): there is nothing
    // for the host to do.
    return makeList(0, 0);
  }
  dirty = 0;
  // The buttons and the box are static; only the readout changed.
  const buf = arenaAlloc(36);
  emitChangedReadout(buf, BOX);
  return makeList(buf, 1);
}

// The readout projected standing alone, as a `changed` mutation.
function emitChangedReadout(rec: i32, parent: i32): void {
  const v = value;
  const str = arenaAlloc(16);
  const len = writeInt32(str, v) - str;
  emitChangedText(rec, READOUT, parent, str, len, v < 0 ? COLOR_NEGATIVE : COLOR_NEUTRAL, SIZE_LARGE);
}

export function onclick(id: i32): void { // → `on-click` after the export rename
  if (id == INC) {
    value++;
    dirty = 1;
  } else if (id == DEC) {
    value--;
    dirty = 1;
  }
}

export function ontick(): void {} // → `on-tick`; a counter has no clock
