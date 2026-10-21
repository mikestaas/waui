// The ergonomic half of the component side — the AssemblyScript twin of the
// Rust `component-ui` crate: a widget tree an author writes, projected into
// the contract's flat, pre-ordered node list (and its diff channel) by the
// emitters in `../abi`.
//
// One deliberate difference from the Rust layer, forced by the toolchain:
// the tree is not a nested *value*. These components compile with
// `--runtime stub` — no AS heap, no allocator — so there is nothing to nest
// with. The tree is instead a table of fixed-size *slots* in the state
// region (below), built once through a scope (`begin*` / `end` — the source
// indentation is the tree), addressed by slot handle afterwards, and
// projected into the result arena by the same `abi/` emitters the demos
// used to call by hand. Same contract, same output; a per-language
// ergonomics shape, because the languages have different memory models.
//
// Page 0 is split in two. Both halves are bump regions; neither needs a
// runtime:
//
//   [0 ......... RESULT_END)   result arena — reset by `cabi_post_*`
//   [RESULT_END . 64 KiB)      state — built once, never freed
//
// The state region holds the slot table, then a pool of label bytes. A
// projected node record points its string field into the pool — a region
// the host can read anyway (the web glue `subarray`s at the pointer; the
// Rust hosts read store memory) — while the result buffer stays a
// self-contained record run, so one `arenaReset()` is still the entire
// free, and the selftest's leak check still holds.
//
// The tree's own footprint is tiny and static: at most MAX_SLOTS slots
// (SLOT_STRIDE words each) plus whatever label capacity the author
// reserved. It never grows the memory, never moves, and needs none of the
// heap that `--runtime stub` removed. (Access is via raw `store`/`load` at
// computed offsets, never array indexing: under the stub runtime an index
// *write* through a typed array pulls the AS runtime back in as an
// `env.abort` import, and the lift would break.)

import { arenaAlloc, RESULT_END } from '../abi/arena';
import {
  COLOR_NEUTRAL,
  MUT_CHANGED,
  SIZE_MEDIUM,
  TAG_BUTTON,
  TAG_CONTAINER,
  TAG_TEXT,
  emitButton,
  emitContainer,
  emitText,
  makeList,
} from '../abi/node';
import { writeInt32, writeInt64, writeUtf8 } from '../abi/utf8';

// ------------------------------------------------------------------ memory

const SLOT_STRIDE = 10; // words per slot (see the word map below)
const MAX_SLOTS = 32; // also the diff mask's width (it is an i32)

const TABLE_END = RESULT_END + MAX_SLOTS * SLOT_STRIDE * 4;

// Slot table word map, at (STATE slot base) + slot * SLOT_STRIDE:
const W_KIND = 0; // one of the abi TAG_* values
const W_ID = 1; // the contract's node id (0 = none)
const W_PARENT_ID = 2; // the parent's id, resolved at build time (0 = root)
const W_PARENT_SLOT = 3; // the parent's slot index (-1 = root); build-time use
const W_COLOR = 4;
const W_SIZE = 5;
const W_DIRECTION = 6;
const W_LABEL_PTR = 7; // offset into the label pool (or 0)
const W_LABEL_LEN = 8; // current label length, in UTF-8 bytes
const W_LABEL_CAP = 9; // the label buffer's capacity, in bytes

var poolCur = TABLE_END; // bump cursor over the label pool
var slotCount = 0; // slots occupied so far (0..MAX_SLOTS)
var curSlot = -1; // the node a build-time call targets
var curContainer = -1; // the innermost open container (scopes)
var exported = 0; // the protocol: 1 once the tree was first exported
var dirty = -1; // per-slot stale bits; -1 (all) at birth = the bootstrap rule

function slotBase(slot: i32): i32 {
  return RESULT_END + slot * SLOT_STRIDE * 4;
}

function slotWord(slot: i32, w: i32): i32 {
  return load<i32>(slotBase(slot) + w * 4);
}

function setSlotWord(slot: i32, w: i32, v: i32): void {
  store<i32>(slotBase(slot) + w * 4, v);
}

/// Bump-allocate `n` bytes from the label pool, 4-byte aligned.
function poolAlloc(n: i32): i32 {
  const p = poolCur;
  const end = p + ((n + 3) & ~3);
  if (end > 65536) {
    // Out of state room: return the (too small) region's start rather than
    // walk into anything. The author's capacity contract is documented in
    // `label`/`reserve`; a PoC-scale tree is far from this.
    poolCur = 65536;
  } else {
    poolCur = end;
  }
  return p;
}

// ------------------------------------------------------------ the build

function begin(kind: i32, direction: i32): i32 {
  if (slotCount >= MAX_SLOTS) return -1; // dropped; a documented PoC limit
  const slot = slotCount++;
  const parent = curContainer;
  // Every word written, so the slot is fully defined even though the state
  // region is never reused (zero-then-fill, mirroring the abi/ emitters).
  setSlotWord(slot, W_KIND, kind);
  setSlotWord(slot, W_ID, 0);
  setSlotWord(slot, W_PARENT_ID, parent >= 0 ? slotWord(parent, W_ID) : 0);
  setSlotWord(slot, W_PARENT_SLOT, parent);
  setSlotWord(slot, W_COLOR, COLOR_NEUTRAL);
  setSlotWord(slot, W_SIZE, SIZE_MEDIUM);
  setSlotWord(slot, W_DIRECTION, direction);
  setSlotWord(slot, W_LABEL_PTR, 0);
  setSlotWord(slot, W_LABEL_LEN, 0);
  setSlotWord(slot, W_LABEL_CAP, 0);
  curSlot = slot;
  if (kind == TAG_CONTAINER) curContainer = slot;
  return slot;
}

/// Start a text node, as a child of the innermost open container.
export function beginText(): i32 {
  return begin(TAG_TEXT, 0);
}

/// Start a button, as a child of the innermost open container.
export function beginButton(): i32 {
  return begin(TAG_BUTTON, 0);
}

/// Start a row container and open its scope (its children come next).
export function beginRow(): i32 {
  return begin(TAG_CONTAINER, 0);
}

/// Start a column container and open its scope (its children come next).
export function beginColumn(): i32 {
  return begin(TAG_CONTAINER, 1);
}

/// Close the innermost open container. Set a container's `id` *before*
/// `begin`-ing its children: the children name it in their `parent` field,
/// and an id-less parent link would be lost (the same rule the Rust crate
/// asserts in debug builds).
export function end(): void {
  const c = curContainer;
  if (c < 0) return;
  curContainer = slotWord(c, W_PARENT_SLOT);
}

/// Give the current node its contract id (what `on-click` and `changes`
/// key on).
export function id(v: i32): void {
  if (curSlot < 0) return;
  setSlotWord(curSlot, W_ID, v);
}

/// Set the current node's color (text/buttons only; a container has none).
export function color(v: i32): void {
  if (curSlot < 0) return;
  setSlotWord(curSlot, W_COLOR, v);
}

/// Set the current node's size (text/buttons only).
export function size(v: i32): void {
  if (curSlot < 0) return;
  setSlotWord(curSlot, W_SIZE, v);
}

/// Set the current node's color and size.
export function style(colorV: i32, sizeV: i32): void {
  color(colorV);
  size(sizeV);
}

/// Reserve the current node's label buffer (text/buttons only) and write
/// `s` as its initial content. `cap` overrides the reserved size; the
/// default (a UTF-16 char can take up to 4 UTF-8 bytes, plus headroom)
/// fits any ASCII label. Reserve for the *longest* content you will show:
/// updates clamp to the capacity.
export function label(s: string, cap: i32 = -1): void {
  const slot = curSlot;
  if (slot < 0) return;
  const want = cap < 0 ? s.length * 4 + 8 : cap;
  setSlotWord(slot, W_LABEL_PTR, poolAlloc(want));
  setSlotWord(slot, W_LABEL_CAP, want);
  setText(slot, s);
}

/// Reserve the current node's label buffer (text/buttons only) for later
/// dynamic content: `set*`/`append*` after the build.
export function reserve(cap: i32): void {
  const slot = curSlot;
  if (slot < 0) return;
  setSlotWord(slot, W_LABEL_PTR, poolAlloc(cap));
  setSlotWord(slot, W_LABEL_CAP, cap);
  setSlotWord(slot, W_LABEL_LEN, 0);
}

// --------------------------------------------------------- live updates

// Runtime updates address a node by its *slot handle* (a `begin*` return
// value). A label write re-encodes into the slot's reserved buffer; the
// buffer lives in the state region and is simply overwritten — the next
// projection reads whatever is there.

/// The buffer's contents edge (start + capacity).
function labelRoom(slot: i32): i32 {
  return slotWord(slot, W_LABEL_PTR) + slotWord(slot, W_LABEL_CAP);
}

/// Where the next label byte goes (start + current length).
function labelWriteStart(slot: i32): i32 {
  return slotWord(slot, W_LABEL_PTR) + slotWord(slot, W_LABEL_LEN);
}

/// Commit a label write: the writer returns the cursor it reached, already
/// bounded by the buffer's edge (so `p2 <= end` holds); the clamp is a
/// belt. The new length is cursor minus buffer start.
function commitLabelWrite(slot: i32, p2: i32, end: i32): void {
  const clamped = p2 < end ? p2 : end;
  setSlotWord(slot, W_LABEL_LEN, clamped - slotWord(slot, W_LABEL_PTR));
}

/// Empty the node's label (before `append*`).
export function labelReset(slot: i32): void {
  setSlotWord(slot, W_LABEL_LEN, 0);
}

/// Replace the node's label with `s` (UTF-16 in, UTF-8 in the buffer;
/// clamped to the reserved capacity, whole characters only).
export function setText(slot: i32, s: string): void {
  labelReset(slot);
  commitLabelWrite(slot, writeUtf8(labelWriteStart(slot), s, labelRoom(slot)), labelRoom(slot));
}

/// Write `v` as decimal ASCII into the node's label (replacing it).
export function setInt32(slot: i32, v: i32): void {
  labelReset(slot);
  commitLabelWrite(slot, writeInt32(labelWriteStart(slot), v, labelRoom(slot)), labelRoom(slot));
}

/// Write `v` as decimal ASCII into the node's label (replacing it).
export function setInt64(slot: i32, v: i64): void {
  labelReset(slot);
  commitLabelWrite(slot, writeInt64(labelWriteStart(slot), v, labelRoom(slot)), labelRoom(slot));
}

/// Append `s` to the node's label (UTF-16 in, UTF-8 in the buffer).
export function appendText(slot: i32, s: string): void {
  commitLabelWrite(slot, writeUtf8(labelWriteStart(slot), s, labelRoom(slot)), labelRoom(slot));
}

/// Append `v` as decimal ASCII to the node's label.
export function appendInt32(slot: i32, v: i32): void {
  commitLabelWrite(slot, writeInt32(labelWriteStart(slot), v, labelRoom(slot)), labelRoom(slot));
}

/// Append `v` as decimal ASCII to the node's label.
export function appendInt64(slot: i32, v: i64): void {
  commitLabelWrite(slot, writeInt64(labelWriteStart(slot), v, labelRoom(slot)), labelRoom(slot));
}

/// Replace the node's color (a text/button whose hue is state).
export function setColor(slot: i32, v: i32): void {
  setSlotWord(slot, W_COLOR, v);
}

/// Replace the node's size.
export function setSize(slot: i32, v: i32): void {
  setSlotWord(slot, W_SIZE, v);
}

/// Mark the node stale: it will be included in the next `changes()`.
export function mark(slot: i32): void {
  if (slot >= 0 && slot < 32) dirty |= 1 << slot;
}

// -------------------------------------------------------------- lowering

// The one seam the Rust layer draws as a generic projection (`lower<N, F>`)
// is drawn here as a direct call: this file compiles *with* `../abi`, so
// there is no crate boundary to bridge — the slots simply emit through the
// same record emitters the demos used to call by hand.

/// Emit slot `slot` as a 32-byte node record at `rec` — or, with
/// `isMutation`, as the 32-byte node of a 36-byte `changed` mutation (the
/// tag word goes in front, as `abi/node` lays it out).
function emitSlot(rec: i32, slot: i32, isMutation: i32): void {
  const base = rec + isMutation * 4;
  const id_ = slotWord(slot, W_ID);
  const parent = slotWord(slot, W_PARENT_ID);
  // Zero-then-fill, mirroring the abi/ emitters (which re-zero the node's
  // own 32 bytes); the tag word is the mutation's only extra byte run.
  //
  // NB: the bound must be computed before the loop. AssemblyScript (like
  // JavaScript) parses `i < isMutation != 0 ? 36 : 32` as
  // `(i < isMutation) != 0 ? 36 : 32` — a ternary *as* the condition, which
  // is always 32 or 36: the loop would never exit and would run the zero
  // clear off the end of memory. (The same trap as the stopwatch's
  // `&`-vs-`!=` comment; parens/temps are load-bearing here.)
  const size = isMutation != 0 ? 36 : 32;
  for (let i = 0; i < size; i += 4) store<i32>(rec + i, 0);
  if (isMutation != 0) store<i32>(rec, MUT_CHANGED);
  const kind = slotWord(slot, W_KIND);
  if (kind == TAG_TEXT) {
    emitText(
      base, id_, parent,
      slotWord(slot, W_LABEL_PTR), slotWord(slot, W_LABEL_LEN),
      slotWord(slot, W_COLOR), slotWord(slot, W_SIZE),
    );
  } else if (kind == TAG_BUTTON) {
    emitButton(
      base, id_, parent,
      slotWord(slot, W_LABEL_PTR), slotWord(slot, W_LABEL_LEN),
      slotWord(slot, W_COLOR), slotWord(slot, W_SIZE),
    );
  } else {
    emitContainer(base, id_, parent, slotWord(slot, W_DIRECTION));
  }
}

/// Project the whole tree as the `view()` result: every slot as a node,
/// in build order (the contract's depth-first pre-order). Also records the
/// export: a full snapshot means nothing is pending.
export function view(): i32 {
  markExported();
  const buf = arenaAlloc(slotCount * 32);
  for (let i = 0; i < slotCount; i++) emitSlot(buf + i * 32, i, 0);
  return makeList(buf, slotCount);
}

/// Record that the current state was just exported (snapshot or diff).
export function markExported(): void {
  dirty = 0;
  exported = 1;
}

/// Project the diff channel: the stale slots as `changed` mutations, in
/// build order. A fresh instance reports *everything* (the bootstrap rule:
/// a host that never calls `view()` still gets the whole tree); a no-op
/// event reports nothing.
export function changes(): i32 {
  if (exported == 0) {
    markExported();
    const buf = arenaAlloc(slotCount * 36);
    for (let i = 0; i < slotCount; i++) emitSlot(buf + i * 36, i, 1);
    return makeList(buf, slotCount);
  }
  const d = dirty;
  dirty = 0;
  if (d == 0) return makeList(0, 0);
  // NB: parens are load-bearing — AssemblyScript (like JavaScript) binds
  // `!=` tighter than `&`. (See the stopwatch demo's original comment.)
  let count = 0;
  for (let i = 0; i < slotCount; i++) {
    if (((d >> i) & 1) != 0) count++;
  }
  const buf = arenaAlloc(count * 36);
  let k = 0;
  for (let i = 0; i < slotCount; i++) {
    if (((d >> i) & 1) != 0) {
      emitSlot(buf + k * 36, i, 1);
      k++;
    }
  }
  return makeList(buf, count);
}
