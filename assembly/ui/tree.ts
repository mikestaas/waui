// The AssemblyScript twin of the Rust `component-ui` crate: the widget tree
// an author writes, projected through the `../abi` emitters into the
// contract's flat, pre-ordered node list and its diff channel.
//
// Nodes are `Widget` objects on the incremental runtime heap (in-module
// TLSF allocator + conservative GC), nested through their `children` arrays —
// the same shape as the Rust `Tree::column(vec![...])`. Tree size is limited
// only by the heap:
//
//   column([
//     button('+', COLOR_POSITIVE, SIZE_MEDIUM).id(2),
//     readout(value),
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
// The diff channel is a stale mark on the node. A demo holds its
// state-dependent nodes as persistent objects (module globals; the
// conservative GC keeps them for the module's lifetime) and its mutation
// handlers set their `dirty` mark. `diffOf(root)` projects the stale nodes —
// the parent taken from the tree structure — as `changed` mutations, then
// clears the marks; a component that has never exported its tree reports the
// whole tree, the host having seen nothing. The Rust `component-ui` has no
// node mark (its demos keep the equivalent as a scalar flag); the exported
// byte stream is the same either way.
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
  /// Stale since the last export: the demo sets it in its mutation handlers,
  /// and `diffOf` consumes and clears it. Only a node the demo holds
  /// persistently keeps the mark between exports; a rebuilt node starts clean.
  dirty: i32 = 0;
  // Materialized by `lower` (valid from projection on):
  labelPtr: i32 = 0; // the partition's base (0 = no label)
  labelLen: i32 = 0; // UTF-8 bytes, clamped to the partition

  constructor(kind: i32, label: string = '', children: Widget[] = []) {
    this.kind = kind;
    this.children = children;
    this.label(label);
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

  /// Set the style in one call (text/buttons only).
  style(colorV: i32, sizeV: i32): this {
    this.color = colorV;
    this.size = sizeV;
    return this;
  }

  /// Clear the stale mark, for chaining. `diffOf` calls this on every node
  /// it projects.
  markClean(): this {
    this.dirty = 0;
    return this;
  }
}

/// A text node.
export function text(
  content: string,
  color: i32 = COLOR_NEUTRAL,
  size: i32 = SIZE_MEDIUM,
): Widget {
  const w = new Widget(TAG_TEXT, content);
  w.color = color;
  w.size = size;
  return w;
}

/// A button.
export function button(
  label: string,
  color: i32 = COLOR_NEUTRAL,
  size: i32 = SIZE_MEDIUM,
): Widget {
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

/// Whether this component has ever exported its whole tree (via `viewOf` or
/// a bootstrap `diffOf`). The WIT's bootstrap rule keys off it: a host that
/// has seen nothing gets the whole tree as `changed` nodes.
var everExported: i32 = 0;

/// Count the nodes of a tree (depth-first pre-order), for sizing the arena.
export function count(w: Widget): i32 {
  let n = 1;
  const c = w.children;
  for (let i = 0; i < c.length; i++) n += count(c[i]);
  return n;
}

/// Count the stale nodes of a tree (depth-first pre-order).
function dirtyCount(w: Widget): i32 {
  let n = w.dirty;
  const c = w.children;
  for (let i = 0; i < c.length; i++) n += dirtyCount(c[i]);
  return n;
}

/// Clear the stale marks of a tree (depth-first pre-order).
function markAllClean(w: Widget): void {
  w.markClean();
  const c = w.children;
  for (let i = 0; i < c.length; i++) markAllClean(c[i]);
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

/// Emit the stale nodes of a tree in depth-first pre-order, each with the
/// parent stated from the structure, and clear their marks. Non-stale nodes
/// are skipped but descended into: a stale child sits under its unmarked
/// parent. Returns the next offset.
function lowerDirty(w: Widget, parent: i32, buf: i32, off: i32): i32 {
  const id = w._id;
  let next = off;
  if (w.dirty != 0) {
    next = emitNode(w, parent, buf, off, 1);
    w.markClean();
  }
  const c = w.children;
  for (let i = 0; i < c.length; i++) {
    next = lowerDirty(c[i], id, buf, next);
  }
  return next;
}

/// Project the whole tree as a `view()` result: every node as a 32-byte node
/// record in depth-first pre-order, behind a list descriptor. A full
/// snapshot accounts for everything: the stale marks are cleared, and the
/// component is marked exported for the bootstrap rule.
export function viewOf(root: Widget): i32 {
  everExported = 1;
  markAllClean(root);
  const n = count(root);
  const buf = arenaAlloc(n * 32);
  lower(root, 0, buf, 0, 0);
  return makeList(buf, n);
}

/// The diff channel for the demo's rebuilt tree. A component that has never
/// exported its tree reports every node as `changed` (the bootstrap rule,
/// held here rather than in each demo); otherwise exactly the stale nodes,
/// in depth-first pre-order, each with the parent taken from the structure.
/// The marks are cleared either way: after the call, nothing is pending.
export function diffOf(root: Widget): i32 {
  const first = everExported == 0;
  const n = first ? count(root) : dirtyCount(root);
  if (first) everExported = 1;
  if (n == 0) return makeList(0, 0); // an empty list's ptr is 0
  const buf = arenaAlloc(n * 36);
  if (first) {
    lower(root, 0, buf, 0, 1);
    markAllClean(root);
  } else {
    lowerDirty(root, 0, buf, 0);
  }
  return makeList(buf, n);
}
