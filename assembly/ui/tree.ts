// The AssemblyScript twin of the Rust `component-ui` crate: the widget tree
// an author writes, projected through the `../abi` emitters into the
// contract's flat, pre-ordered node list and its diff channel.
//
// Nodes are `Widget` objects on the incremental runtime heap (in-module
// TLSF allocator + conservative GC), nested through their `children` arrays —
// the same shape as the Rust `Tree::column(vec![...])`. Tree size is limited
// only by the heap. The readout is state-dependent, so it is a persistent
// object (a module global in the demos); the buttons are rebuilt with the
// column:
//
//   var readout = text('', COLOR_NEUTRAL, SIZE_LARGE).id(3);
//   column([
//     button('+', COLOR_POSITIVE, SIZE_MEDIUM).id(2),
//     readout,
//     button('-', COLOR_NEGATIVE, SIZE_MEDIUM).id(4),
//   ]).id(1);
//
// The heap costs one import: the incremental runtime pulls in `env.abort`
// (its OOM path), which `../patch-env-abort.mjs` replaces with an in-module
// `unreachable` after compilation, so the lifted component is import-free.
//
// Page 0 (see `abi/arena`):
//
//   [0 ......... RESULT_END)   result arena, reset by cabi_post_*
//   [RESULT_END . 64 KiB)      label pool, one partition per stable id
//
// A projected record carries a *pointer* into a node's label bytes, and the
// host keeps the result, so the bytes must outlive the tree (which is
// rebuilt from state on every export and the GC reclaims it). The pool is
// raw linear memory the GC never scans: a partition is allocated at the id's
// first build, sized by `reserve`, never moves or grows, and later content
// overwrites it in place, clamped to fit. A labeled node without an id gets
// a fresh partition per build.
//
// The diff channel is a record comparison against the last export, the AS
// twin of the Rust crate's retained `LAST_EXPORTED`: the last export is kept
// as a flat list of records, and `diffOf(root)` reports every id'd node whose
// record — kind, parent, label, style, or direction — differs from it, plus
// every id it no longer contains, as `removed`. Before the first export it
// reports every id'd node (the WIT bootstrap rule). This is the contract's
// add, remove, and re-parent.
//
// This module provides the tree, the pool, and the projection
// (`viewOf`, `diffOf`, `count`).

import { arenaAlloc, RESULT_END } from '../abi/arena';
import {
  COLOR_NEUTRAL,
  DIRECTION_COLUMN,
  DIRECTION_ROW,
  MUT_CHANGED,
  SIZE_MEDIUM,
  TAG_BUTTON,
  TAG_CONTAINER,
  TAG_TEXT,
  emitButton,
  emitContainer,
  emitRemoved,
  emitText,
  makeList,
} from '../abi/node';
import { writeUtf8 } from '../abi/utf8';

// ------------------------------------------------------------------ memory

/// One static-pool partition of label bytes.
class PoolBuf {
  off: i32 = 0; // the partition's base, in the pool region
  cap: i32 = 0; // the partition's size in bytes (and the write clamp)
}

// Node id → its label partition. A module global, so the conservative GC
// keeps every partition for the module's lifetime: stable across rebuilds.
var pool = new Map<i32, PoolBuf>();
var poolCur = RESULT_END; // bump cursor, RESULT_END → the 64 KiB page edge

/// Bump-allocate `n` bytes from the pool, 4-byte aligned. Returns -1 when
/// the request is not positive or the region would overflow the page — the
/// caller then renders an empty label rather than walk into anything.
function poolAlloc(n: i32): i32 {
  if (n <= 0) return -1;
  const p = poolCur;
  const end = p + ((n + 3) & ~3);
  if (end > 65536 || end <= p) return -1;
  poolCur = end;
  return p;
}

// ------------------------------------------------------------------ the tree

/// One node of a widget tree: an object on the runtime heap, nested via
/// `children` — the AS twin of the Rust crate's `Tree`.
export class Widget {
  /// The contract's node id (0 = none); the storage behind the `id()`
  /// builder. Rust lets a field and a method share a name; AssemblyScript
  /// does not, so address the id through `id()`.
  _id: i32 = 0;
  /// One of the `abi/node` TAG_* values.
  kind: i32;
  /// The style (text/buttons only).
  color: i32 = COLOR_NEUTRAL;
  size: i32 = SIZE_MEDIUM;
  /// Containers only.
  direction: i32 = DIRECTION_ROW;
  /// The children, in the contract's depth-first pre-order.
  children: Widget[];
  /// The label's UTF-16 source; materialized to the pool at projection.
  labelSrc: string = '';
  /// The label buffer's requested capacity (consumed at first allocation).
  labelCap: i32 = 0;
  // Materialized by `lower` (valid from projection on):
  labelPtr: i32 = 0; // the partition's base (0 = no label)
  labelLen: i32 = 0; // UTF-8 bytes, clamped to the partition

  constructor(kind: i32, label: string = '', children: Widget[] = []) {
    this.kind = kind;
    this.children = children;
    this.labelSrc = label;
  }

  /// Assign the contract's node id (0 = none) and return `this` for
  /// chaining — the twin of the Rust crate's `.id(n)` builder, so a tree
  /// can be written in one nested expression. A *stable* id across
  /// rebuilds is what makes the label buffer stable.
  id(n: i32): this {
    this._id = n;
    return this;
  }

  /// Replace the label (UTF-16 in; UTF-8 in the pool at projection).
  label(s: string): this {
    this.labelSrc = s;
    return this;
  }

  /// Size the label buffer for the longest content you will show. Only the
  /// *first* allocation of a node's partition honors it — a partition never
  /// moves or grows — and later content clamps to fit.
  reserve(cap: i32): this {
    this.labelCap = cap;
    return this;
  }

  /// Set the style (text/buttons only).
  style(colorV: i32, sizeV: i32): this {
    this.color = colorV;
    this.size = sizeV;
    return this;
  }
}

/// A text node.
export function text(content: string, color: i32 = COLOR_NEUTRAL, size: i32 = SIZE_MEDIUM): Widget {
  const w = new Widget(TAG_TEXT, content);
  w.color = color;
  w.size = size;
  return w;
}

/// A button.
export function button(label: string, color: i32 = COLOR_NEUTRAL, size: i32 = SIZE_MEDIUM): Widget {
  const w = new Widget(TAG_BUTTON, label);
  w.color = color;
  w.size = size;
  return w;
}

/// A row container with its children already nested.
export function row(children: Widget[]): Widget {
  const w = new Widget(TAG_CONTAINER, '', children);
  w.direction = DIRECTION_ROW;
  return w;
}

/// A column container with its children already nested.
export function column(children: Widget[]): Widget {
  const w = new Widget(TAG_CONTAINER, '', children);
  w.direction = DIRECTION_COLUMN;
  return w;
}

// ------------------------------------------------------------- the pool seam

/// Materialize the node's label bytes into the static pool: a partition
/// keyed by the node's id, sized at first allocation (honoring `reserve`),
/// overwritten on every later projection. No-op for an empty label; an
/// exhausted pool renders an empty label rather than corrupt memory.
function materializeLabel(w: Widget): void {
  const src = w.labelSrc;
  if (src.length == 0) {
    w.labelPtr = 0;
    w.labelLen = 0;
    return;
  }
  // A UTF-16 char is at most 4 UTF-8 bytes, plus headroom; the author's
  // reserve may ask for more.
  const content = src.length * 4 + 8;
  const want = w.labelCap > content ? w.labelCap : content;
  // `Map.get` throws on an absent key (the stdlib cannot represent
  // `undefined`, and a throw in wasm is an `abort` trap), so the lookup goes
  // through `has` first. No flow narrowing, so the branch ends in scalars.
  let off: i32;
  let cap: i32;
  if (w._id > 0 && pool.has(w._id)) {
    const existing = pool.get(w._id);
    off = existing.off;
    cap = existing.cap;
  } else {
    off = poolAlloc(want);
    if (off < 0) {
      w.labelPtr = 0;
      w.labelLen = 0;
      return;
    }
    cap = (want + 3) & ~3; // the pool allocated exactly this much
    if (w._id > 0) {
      const buf = new PoolBuf();
      buf.off = off;
      buf.cap = cap;
      pool.set(w._id, buf);
    }
    // id-less: fresh partition per build (documented in the header)
  }
  w.labelPtr = off;
  w.labelLen = writeUtf8(off, src, off + cap) - off;
}

// ------------------------------------------------------------- projection

/// Set once the component has exported its whole tree (`viewOf`, or a
/// bootstrap `diffOf`); the WIT bootstrap rule keys off it.
var everExported: i32 = 0;

/// One node of the last export, as a flat record — the AS twin of the Rust
/// crate's `Flat`. Retained across exports for the diff's record comparison.
class FlatRec {
  kind: i32 = 0;
  _id: i32 = 0;
  parent: i32 = 0;
  color: i32 = COLOR_NEUTRAL;
  size: i32 = SIZE_MEDIUM;
  direction: i32 = DIRECTION_ROW;
  label: string = '';
}

/// The last export, as a flat list of records in depth-first pre-order — the
/// AS twin of the Rust crate's `LAST_EXPORTED`. Replaced on every export; the
/// conservative GC reclaims the previous one.
var lastExport: FlatRec[] = [];

/// Count the nodes of a tree (depth-first pre-order), for sizing the arena.
export function count(w: Widget): i32 {
  let n = 1;
  const c = w.children;
  for (let i = 0; i < c.length; i++) n += count(c[i]);
  return n;
}

/// Flatten the tree into records (depth-first pre-order, every node), for the
/// retained last export.
function flatten(w: Widget, parent: i32, out: FlatRec[]): void {
  const r = new FlatRec();
  r.kind = w.kind;
  r._id = w._id;
  r.parent = parent;
  r.color = w.color;
  r.size = w.size;
  r.direction = w.direction;
  r.label = w.labelSrc;
  out.push(r);
  const id = w._id;
  const c = w.children;
  for (let i = 0; i < c.length; i++) {
    flatten(c[i], id, out);
  }
}

/// Whether `id` is among the records of `recs`.
function presentIn(recs: FlatRec[], id: i32): i32 {
  for (let i = 0; i < recs.length; i++) {
    if (recs[i]._id == id) return 1;
  }
  return 0;
}

/// Whether `w` (with `parent`) is reported `changed`: id'd and, in bootstrap,
/// any; otherwise its record differs from the last export, or its id is new.
/// An id-less node is unaddressable (the WIT identity rule) and never is.
function isChanged(w: Widget, parent: i32, first: i32, last: FlatRec[]): i32 {
  const id = w._id;
  if (id == 0) return 0; // id-less: unaddressable, never reported
  if (first != 0) return 1; // bootstrap: every id'd node
  for (let i = 0; i < last.length; i++) {
    const r = last[i];
    if (r._id != id) continue;
    // Same id: changed only if the record differs. A container's record is
    // its direction (the WIT container carries no style); a leaf's is its
    // label and style. AS compares strings by content.
    let differs = w.kind != r.kind || parent != r.parent;
    if (w.kind == TAG_CONTAINER) {
      differs = differs || w.direction != r.direction;
    } else {
      differs = differs || w.color != r.color || w.size != r.size || w.labelSrc != r.label;
    }
    return differs ? 1 : 0;
  }
  return 1; // id absent from the last export: a new node
}

/// Count the `changed` nodes of a tree (depth-first pre-order).
function countChanged(w: Widget, parent: i32, first: i32, last: FlatRec[]): i32 {
  let n = isChanged(w, parent, first, last);
  const id = w._id;
  const c = w.children;
  for (let i = 0; i < c.length; i++) {
    n += countChanged(c[i], id, first, last);
  }
  return n;
}

/// Count the ids in `last` that are no longer in `newRecs` (the removals).
function countRemoved(last: FlatRec[], newRecs: FlatRec[]): i32 {
  let n = 0;
  for (let i = 0; i < last.length; i++) {
    const id = last[i]._id;
    if (id == 0) continue; // id-less: never removed
    if (presentIn(newRecs, id) == 0) n += 1;
  }
  return n;
}

/// Emit one record for `w` at `buf + off * stride`. With `isMutation`, the
/// record is a 36-byte `changed` mutation (the tag word in front of the
/// 32-byte node). Returns the next offset; stops at the arena's cap rather
/// than walk into the label pool.
function emitNode(w: Widget, parent: i32, buf: i32, off: i32, isMutation: i32): i32 {
  // Computed first: AS (like JS) binds `<` tighter than `?:`, so an
  // unparenthesized `buf + off * isMutation != 0 ? 36 : 32` would parse as
  // a ternary *as* the comparison.
  const stride = isMutation != 0 ? 36 : 32;
  const rec = buf + off * stride;
  if (rec + stride > RESULT_END) return off;
  materializeLabel(w);
  const id = w._id;
  if (isMutation != 0) store<i32>(rec, MUT_CHANGED);
  // A `changed` mutation carries the node one word in (after the tag word);
  // a plain node record *is* the record base. The emitters self-zero their
  // 32 bytes, so the offset must be exactly right: a stray word spills into
  // the neighbor record.
  const node = rec + (isMutation != 0 ? 4 : 0);
  const kind = w.kind;
  if (kind == TAG_TEXT) {
    emitText(node, id, parent, w.labelPtr, w.labelLen, w.color, w.size);
  } else if (kind == TAG_BUTTON) {
    emitButton(node, id, parent, w.labelPtr, w.labelLen, w.color, w.size);
  } else {
    emitContainer(node, id, parent, w.direction);
  }
  return off + 1;
}

/// Emit `w` and its descendants in depth-first pre-order (see `emitNode`).
/// Returns the next offset.
function lower(w: Widget, parent: i32, buf: i32, off: i32, isMutation: i32): i32 {
  const id = w._id;
  let next = emitNode(w, parent, buf, off, isMutation);
  const c = w.children;
  for (let i = 0; i < c.length; i++) {
    next = lower(c[i], id, buf, next, isMutation);
  }
  return next;
}

/// Emit the `changed` nodes of a tree in depth-first pre-order, each with the
/// parent from the structure. Non-changed nodes are skipped but descended
/// into: a changed child sits under its unchanged parent. Returns the next
/// offset.
function lowerChanged(
  w: Widget,
  parent: i32,
  buf: i32,
  off: i32,
  first: i32,
  last: FlatRec[],
): i32 {
  let next = off;
  if (isChanged(w, parent, first, last) != 0) {
    next = emitNode(w, parent, buf, off, 1);
  }
  const id = w._id;
  const c = w.children;
  for (let i = 0; i < c.length; i++) {
    next = lowerChanged(c[i], id, buf, next, first, last);
  }
  return next;
}

/// Emit the `removed` mutations: the ids in `last` no longer in `newRecs`, in
/// the last export's pre-order. Returns the next offset.
function lowerRemoved(last: FlatRec[], newRecs: FlatRec[], buf: i32, off: i32): i32 {
  let next = off;
  for (let i = 0; i < last.length; i++) {
    const id = last[i]._id;
    if (id == 0) continue; // id-less: never removed
    if (presentIn(newRecs, id) == 0) {
      emitRemoved(buf + next * 36, id);
      next += 1;
    }
  }
  return next;
}

/// Project the whole tree as a `view()` result: every node as a 32-byte node
/// record in depth-first pre-order, behind a list descriptor. The full
/// snapshot becomes the reference the next diff compares against (the Rust
/// crate's `note_view`); `everExported` is set.
export function viewOf(root: Widget): i32 {
  everExported = 1;
  const recs = new Array<FlatRec>();
  flatten(root, 0, recs);
  lastExport = recs;
  const n = count(root);
  const buf = arenaAlloc(n * 32);
  lower(root, 0, buf, 0, 0);
  return makeList(buf, n);
}

/// Project the diff channel: the record comparison against the last export.
/// Before the first export, every id'd node is `changed` (the WIT bootstrap
/// rule); otherwise, every id'd node whose record differs is `changed` (in
/// depth-first pre-order), then every id in the last export that is no longer
/// in the tree is `removed` (in the last export's pre-order). The new tree
/// becomes the next reference (the Rust crate's `diff`).
export function diffOf(root: Widget): i32 {
  const first = everExported == 0;
  const last = lastExport; // the reference, captured before it is replaced
  const newRecs = new Array<FlatRec>();
  flatten(root, 0, newRecs);
  if (first) everExported = 1;

  const nChanged = countChanged(root, 0, first, last);
  const nRemoved = first != 0 ? 0 : countRemoved(last, newRecs);
  const n = nChanged + nRemoved;

  lastExport = newRecs; // the new tree is the next reference

  if (n == 0) return makeList(0, 0); // an empty list's ptr is 0
  const buf = arenaAlloc(n * 36);
  let off = lowerChanged(root, 0, buf, 0, first, last);
  if (first == 0) {
    lowerRemoved(last, newRecs, buf, off);
  }
  return makeList(buf, n);
}
