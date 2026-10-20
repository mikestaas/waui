// Emitters for the canonical-ABI encoding of the `waui:ui/widget` types —
// the byte-level mirror of what `wit-bindgen` emits for the Rust
// components, and of what the hosts' decoders (e.g. `web/glue.js`) read.
//
//   `node` (a variant; 32 bytes, the widest case):
//     [0]       tag: 0 = text, 1 = button, 2 = container
//     [4]       id option flag        [8..12)   id
//     [12]      parent option flag    [16..20)  parent
//     [20..24)  text/button: string pointer | container: direction byte
//     [24..28)  text/button: string length (UTF-8 bytes)
//     [28]      text/button: style.color    [29] style.size
//
//   `mutation` (a variant; 36 bytes):
//     [0..4)    tag: 0 = changed, 1 = removed
//     [4..)     changed: a 32-byte `node` | removed: the node's id (u32)
//
// Enum values are the WIT case order in `wit/widget.wit`. Records are
// zeroed before their fields are written: the arena is reused, and a host
// must never see stale bytes in padding.

import { arenaAlloc } from './arena';

// `waui:ui/widget` token values (WIT enum case order).
export const TAG_TEXT = 0;
export const TAG_BUTTON = 1;
export const TAG_CONTAINER = 2;
export const COLOR_NEUTRAL = 0;
export const COLOR_ACCENT = 1;
export const COLOR_POSITIVE = 2;
export const COLOR_NEGATIVE = 3;
export const COLOR_WARNING = 4;
export const SIZE_SMALL = 0;
export const SIZE_MEDIUM = 1;
export const SIZE_LARGE = 2;
export const DIRECTION_ROW = 0;
export const DIRECTION_COLUMN = 1;
export const MUT_CHANGED = 0;
export const MUT_REMOVED = 1;

// A node id of 0 means "none" (the WIT option is not set). Real ids start
// at 1; a root's parent is 0.

function zero(rec: i32, size: i32): void {
  for (let i = 0; i < size; i += 4) store<i32>(rec + i, 0);
}

function putId(rec: i32, id: i32, parent: i32): void {
  if (id != 0) {
    store<i8>(rec + 4, 1);
    store<i32>(rec + 8, id);
  }
  if (parent != 0) {
    store<i8>(rec + 12, 1);
    store<i32>(rec + 16, parent);
  }
}

/// A `text` node record at `rec` (32 bytes).
export function emitText(
  rec: i32, id: i32, parent: i32,
  str: i32, len: i32, color: i32, size: i32,
): void {
  zero(rec, 32);
  store<i8>(rec + 0, TAG_TEXT);
  putId(rec, id, parent);
  store<i32>(rec + 20, str);
  store<i32>(rec + 24, len);
  store<i8>(rec + 28, color);
  store<i8>(rec + 29, size);
}

/// A `button` node record at `rec` (32 bytes).
export function emitButton(
  rec: i32, id: i32, parent: i32,
  str: i32, len: i32, color: i32, size: i32,
): void {
  zero(rec, 32);
  store<i8>(rec + 0, TAG_BUTTON);
  putId(rec, id, parent);
  store<i32>(rec + 20, str);
  store<i32>(rec + 24, len);
  store<i8>(rec + 28, color);
  store<i8>(rec + 29, size);
}

/// A `container` node record at `rec` (32 bytes).
export function emitContainer(rec: i32, id: i32, parent: i32, direction: i32): void {
  zero(rec, 32);
  store<i8>(rec + 0, TAG_CONTAINER);
  putId(rec, id, parent);
  store<i8>(rec + 20, direction);
}

// The `changed` mutation's payload is a full node record at byte 4, so it
// is written by the same node emitters, one word in.

/// A `changed` mutation carrying a `text` node, at `rec` (36 bytes).
export function emitChangedText(
  rec: i32, id: i32, parent: i32,
  str: i32, len: i32, color: i32, size: i32,
): void {
  zero(rec, 36);
  store<i32>(rec + 0, MUT_CHANGED);
  emitText(rec + 4, id, parent, str, len, color, size);
}

/// A `changed` mutation carrying a `button` node, at `rec` (36 bytes).
export function emitChangedButton(
  rec: i32, id: i32, parent: i32,
  str: i32, len: i32, color: i32, size: i32,
): void {
  zero(rec, 36);
  store<i32>(rec + 0, MUT_CHANGED);
  emitButton(rec + 4, id, parent, str, len, color, size);
}

/// A `changed` mutation carrying a `container` node, at `rec` (36 bytes).
export function emitChangedContainer(rec: i32, id: i32, parent: i32, direction: i32): void {
  zero(rec, 36);
  store<i32>(rec + 0, MUT_CHANGED);
  emitContainer(rec + 4, id, parent, direction);
}

/// A `removed` mutation (the node's id) at `rec` (36 bytes).
export function emitRemoved(rec: i32, id: i32): void {
  zero(rec, 36);
  store<i32>(rec + 0, MUT_REMOVED);
  store<i32>(rec + 4, id);
}

// A `list` result in the canonical ABI is a pointer to an 8-byte
// {ptr, len} descriptor in the callee's memory: `ptr` the contiguous item
// buffer, `len` the item count. The host returns it with the matching
// `cabi_post_*`. Use 0 for an empty list's ptr.
export function makeList(buf: i32, len: i32): i32 {
  const d = arenaAlloc(8);
  store<i32>(d + 0, buf);
  store<i32>(d + 4, len);
  return d;
}
