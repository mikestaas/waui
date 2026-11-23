// Emitters for the canonical-ABI encoding of the `waui:ui/widget` types —
// the byte-level mirror of what `wit-bindgen` emits for the Rust
// components, and of what the hosts' decoders (e.g. `web/glue.js`) read.
//
//   `node` (a variant; 60 bytes, the widest case — the container):
//     [0]       tag: 0 = text, 1 = button, 2 = container, 3 = input,
//               4 = image, 5 = link
//     [4]       id option flag        [8..12)   id
//     [12]      parent option flag    [16..20)  parent
//     text/button/input: [20..24) string pointer, [24..28) string length,
//                         [28] style.color, [29] style.size
//     image:      [20..24) src pointer, [24..28) src length,
//                 [28..32) width, [32..36) height
//     link:       [20..24) label pointer, [24..28) label length,
//                 [28..32) href pointer, [32..36) href length,
//                 [36] style.color, [37] style.size
//     container:  [20] direction, [21] align, [22] positioned,
//                 [24..28) x, [28..32) y, [32..36) w, [36..40) h,
//                 [40..44) z, [44..48) gap, [48..52) padding,
//                 [52] stretch, [53] overflow, [54] background, [55] border,
//                 [56] radius, [57] shadow, [58] cursor
//
//   `mutation` (a variant; 68 bytes):
//     [0..4)    tag: 0 = changed, 1 = removed
//     [4..64)   changed: a 60-byte `node` | removed: the node's id (u32)
//     [64..68)  changed: the node's index (u32) | removed: padding
//
// Enum values are the WIT case order in `wit/widget.wit`. Records are
// zeroed before their fields are written: the arena is reused, and a host
// must never see stale bytes in padding.

import { arenaAlloc } from './arena';

// `waui:ui/widget` token values (WIT enum case order).
export const TAG_TEXT = 0;
export const TAG_BUTTON = 1;
export const TAG_CONTAINER = 2;
export const TAG_INPUT = 3;
export const TAG_IMAGE = 4;
export const TAG_LINK = 5;
export const COLOR_NEUTRAL = 0;
export const COLOR_ACCENT = 1;
export const COLOR_POSITIVE = 2;
export const COLOR_NEGATIVE = 3;
export const COLOR_WARNING = 4;
export const COLOR_DIM = 5;
export const SIZE_SMALL = 0;
export const SIZE_MEDIUM = 1;
export const SIZE_LARGE = 2;
export const DIRECTION_ROW = 0;
export const DIRECTION_COLUMN = 1;
export const ALIGN_START = 0;
export const ALIGN_CENTER = 1;
export const ALIGN_END = 2;
export const STRETCH_NONE = 0;
export const STRETCH_MAIN = 1;
export const STRETCH_CROSS = 2;
export const STRETCH_BOTH = 3;
export const OVERFLOW_CLIP = 0;
export const OVERFLOW_SCROLL = 1;
export const BACKGROUND_NONE = 0;
export const BACKGROUND_PANEL = 1;
export const BACKGROUND_BAR = 2;
export const BORDER_NONE = 0;
export const BORDER_LINE = 1;
export const BORDER_RING = 2;
export const RADIUS_NONE = 0;
export const RADIUS_SMALL = 1;
export const RADIUS_LARGE = 2;
export const SHADOW_NONE = 0;
export const SHADOW_RAISED = 1;
export const CURSOR_DEFAULT = 0;
export const CURSOR_POINTER = 1;
export const CURSOR_GRAB = 2;
export const CURSOR_GRABBING = 3;
export const CURSOR_RESIZE = 4;
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

/// A `text` node record at `rec` (60 bytes).
export function emitText(
  rec: i32,
  id: i32,
  parent: i32,
  str: i32,
  len: i32,
  color: i32,
  size: i32,
): void {
  zero(rec, 60);
  store<i8>(rec + 0, TAG_TEXT);
  putId(rec, id, parent);
  store<i32>(rec + 20, str);
  store<i32>(rec + 24, len);
  store<i8>(rec + 28, color);
  store<i8>(rec + 29, size);
}

/// A `button` node record at `rec` (60 bytes).
export function emitButton(
  rec: i32,
  id: i32,
  parent: i32,
  str: i32,
  len: i32,
  color: i32,
  size: i32,
): void {
  zero(rec, 60);
  store<i8>(rec + 0, TAG_BUTTON);
  putId(rec, id, parent);
  store<i32>(rec + 20, str);
  store<i32>(rec + 24, len);
  store<i8>(rec + 28, color);
  store<i8>(rec + 29, size);
}

/// An `input` node record at `rec` (60 bytes). The width is in characters.
export function emitInput(
  rec: i32,
  id: i32,
  parent: i32,
  str: i32,
  len: i32,
  color: i32,
  size: i32,
  width: i32,
): void {
  zero(rec, 60);
  store<i8>(rec + 0, TAG_INPUT);
  putId(rec, id, parent);
  store<i32>(rec + 20, str);
  store<i32>(rec + 24, len);
  store<i8>(rec + 28, color);
  store<i8>(rec + 29, size);
  store<i32>(rec + 32, width);
}

/// An `image` node record at `rec` (60 bytes). An image carries no style:
/// its `[28..36)` is the width and the height, not the two style bytes.
export function emitImage(
  rec: i32,
  id: i32,
  parent: i32,
  str: i32,
  len: i32,
  width: i32,
  height: i32,
): void {
  zero(rec, 60);
  store<i8>(rec + 0, TAG_IMAGE);
  putId(rec, id, parent);
  store<i32>(rec + 20, str);
  store<i32>(rec + 24, len);
  store<i32>(rec + 28, width);
  store<i32>(rec + 32, height);
}

/// A `link` node record at `rec` (60 bytes). Its `[28..36)` is the href,
/// not the two style bytes (as with an image); the style sits at `[36]` and
/// `[37]`.
export function emitLink(
  rec: i32,
  id: i32,
  parent: i32,
  labelPtr: i32,
  labelLen: i32,
  hrefPtr: i32,
  hrefLen: i32,
  color: i32,
  size: i32,
): void {
  zero(rec, 60);
  store<i8>(rec + 0, TAG_LINK);
  putId(rec, id, parent);
  store<i32>(rec + 20, labelPtr);
  store<i32>(rec + 24, labelLen);
  store<i32>(rec + 28, hrefPtr);
  store<i32>(rec + 32, hrefLen);
  store<i8>(rec + 36, color);
  store<i8>(rec + 37, size);
}

/// A `container` node record at `rec` (60 bytes). The 19 WIT fields, in the
/// record's order: the two option ids, then direction, align, positioned,
/// the x/y/w/h/z geometry, gap and padding, and the seven chrome tokens.
export function emitContainer(
  rec: i32,
  id: i32,
  parent: i32,
  direction: i32,
  align: i32,
  positioned: i32,
  x: i32,
  y: i32,
  w: i32,
  h: i32,
  z: i32,
  gap: i32,
  padding: i32,
  stretch: i32,
  overflow: i32,
  background: i32,
  border: i32,
  radius: i32,
  shadow: i32,
  cursor: i32,
): void {
  zero(rec, 60);
  store<i8>(rec + 0, TAG_CONTAINER);
  putId(rec, id, parent);
  store<i8>(rec + 20, direction);
  store<i8>(rec + 21, align);
  store<i8>(rec + 22, positioned);
  store<i32>(rec + 24, x);
  store<i32>(rec + 28, y);
  store<i32>(rec + 32, w);
  store<i32>(rec + 36, h);
  store<i32>(rec + 40, z);
  store<i32>(rec + 44, gap);
  store<i32>(rec + 48, padding);
  store<i8>(rec + 52, stretch);
  store<i8>(rec + 53, overflow);
  store<i8>(rec + 54, background);
  store<i8>(rec + 55, border);
  store<i8>(rec + 56, radius);
  store<i8>(rec + 57, shadow);
  store<i8>(rec + 58, cursor);
}

// The `changed` mutation's payload is a full node record at byte 4, so it
// is written by the same node emitters, one word in. The index (the node's
// 0-based position among its parent's children) sits after the 60-byte node.

/// A `changed` mutation carrying a `text` node, at `rec` (68 bytes).
export function emitChangedText(
  rec: i32,
  id: i32,
  parent: i32,
  str: i32,
  len: i32,
  color: i32,
  size: i32,
  index: i32,
): void {
  zero(rec, 68);
  store<i32>(rec + 0, MUT_CHANGED);
  emitText(rec + 4, id, parent, str, len, color, size);
  store<i32>(rec + 64, index);
}

/// A `changed` mutation carrying a `button` node, at `rec` (68 bytes).
export function emitChangedButton(
  rec: i32,
  id: i32,
  parent: i32,
  str: i32,
  len: i32,
  color: i32,
  size: i32,
  index: i32,
): void {
  zero(rec, 68);
  store<i32>(rec + 0, MUT_CHANGED);
  emitButton(rec + 4, id, parent, str, len, color, size);
  store<i32>(rec + 64, index);
}

/// A `changed` mutation carrying an `input` node, at `rec` (68 bytes).
export function emitChangedInput(
  rec: i32,
  id: i32,
  parent: i32,
  str: i32,
  len: i32,
  color: i32,
  size: i32,
  width: i32,
  index: i32,
): void {
  zero(rec, 68);
  store<i32>(rec + 0, MUT_CHANGED);
  emitInput(rec + 4, id, parent, str, len, color, size, width);
  store<i32>(rec + 64, index);
}

/// A `changed` mutation carrying an `image` node, at `rec` (68 bytes).
export function emitChangedImage(
  rec: i32,
  id: i32,
  parent: i32,
  str: i32,
  len: i32,
  width: i32,
  height: i32,
  index: i32,
): void {
  zero(rec, 68);
  store<i32>(rec + 0, MUT_CHANGED);
  emitImage(rec + 4, id, parent, str, len, width, height);
  store<i32>(rec + 64, index);
}

/// A `changed` mutation carrying a `link` node, at `rec` (68 bytes).
export function emitChangedLink(
  rec: i32,
  id: i32,
  parent: i32,
  labelPtr: i32,
  labelLen: i32,
  hrefPtr: i32,
  hrefLen: i32,
  color: i32,
  size: i32,
  index: i32,
): void {
  zero(rec, 68);
  store<i32>(rec + 0, MUT_CHANGED);
  emitLink(rec + 4, id, parent, labelPtr, labelLen, hrefPtr, hrefLen, color, size);
  store<i32>(rec + 64, index);
}

/// A `changed` mutation carrying a `container` node, at `rec` (68 bytes).
export function emitChangedContainer(
  rec: i32,
  id: i32,
  parent: i32,
  direction: i32,
  align: i32,
  positioned: i32,
  x: i32,
  y: i32,
  w: i32,
  h: i32,
  z: i32,
  gap: i32,
  padding: i32,
  stretch: i32,
  overflow: i32,
  background: i32,
  border: i32,
  radius: i32,
  shadow: i32,
  cursor: i32,
  index: i32,
): void {
  zero(rec, 68);
  store<i32>(rec + 0, MUT_CHANGED);
  emitContainer(
    rec + 4,
    id,
    parent,
    direction,
    align,
    positioned,
    x,
    y,
    w,
    h,
    z,
    gap,
    padding,
    stretch,
    overflow,
    background,
    border,
    radius,
    shadow,
    cursor,
  );
  store<i32>(rec + 64, index);
}

/// A `removed` mutation (the node's id) at `rec` (68 bytes).
export function emitRemoved(rec: i32, id: i32): void {
  zero(rec, 68);
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
