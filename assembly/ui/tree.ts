// The ergonomic half of the component side — the AssemblyScript twin of the
// Rust `component-ui` crate: a widget tree an author writes, projected into
// the contract's flat, pre-ordered node list (and its diff channel) by the
// emitters in `../abi`.
//
// The tree is a real nested *value*, exactly like the Rust crate's: nodes are
// `Widget` objects on the incremental runtime's heap (TLSF allocator +
// conservative GC, all in-module), nested through their `children` arrays.
// There is no slot table, so there is no slot cap — a tree's size is limited
// only by the heap — and the demos author it the way the Rust demos do:
//
//   const box = column([
//     button('+', COLOR_POSITIVE, SIZE_MEDIUM),   // id 2
//     readout(value),                             // id 3
//     button('-', COLOR_NEGATIVE, SIZE_MEDIUM),   // id 4
//   ]);
//   box.id = 1;
//
// One price of the real heap: the module is no longer import-free. The
// incremental runtime pulls in a single `env.abort` import (its OOM path),
// which `../patch-env-abort.mjs` replaces with an in-module `unreachable`
// after compilation — behaviorally identical (both trap), so the lifted
// component is import-free again and the WIT world stays untouched.
//
// Page 0 keeps two bump regions (see `abi/arena`):
//
//   [0 ......... RESULT_END)   result arena — reset by `cabi_post_*`
//   [RESULT_END . 64 KiB)      label pool — one partition per stable id
//
// The tree's *structure* lives on the heap (page 1 and up), but a node's
// label *bytes* must outlive the tree: a projected record carries a
// *pointer* into them, and the tree is rebuilt from state on every export —
// bytes that died with the previous tree would dangle in the host's copy of
// the result. The pool is raw linear memory the GC never touches, so a
// partition stays put for the module's lifetime. A node that keeps its id
// claims the same partition on every rebuild: the first allocation sizes it
// (honoring `reserve`), later writes clamp to fit — reserve for the longest
// content you will show. A labeled node *without* an id gets a fresh
// partition per build; a changing label that never carries a stable id would
// grow the pool until it is exhausted (which then renders an empty label
// rather than corrupting anything).
//
// The diff channel follows the Rust demos exactly: the *demo* owns the dirty
// state (a flag, or a small bit mask over its state-dependent nodes) and
// re-projects the dirty subtrees from state. The bookkeeping is the demo's,
// not the layer's — the Rust `component-ui` crate provides no diff
// machinery either, only the tree and its projection. `view()` is always
// the full truth; `changes()` is the bootstrap (the whole tree, on first
// contact) and then the demo's diff.

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

// A node id → its label partition. Retained by this module global for the
// module's lifetime, so a buffer stays put across rebuilds. (An instance of
// `Map` is a runtime object — fine: these components now compile with the
// incremental runtime, which has a heap.)
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

/// One node of a widget tree: a real object on the runtime heap, nested via
/// `children` — the AS twin of the Rust crate's `Tree`.
export class Widget {
  /// The contract's node id (0 = none). A *stable* id across rebuilds is
  /// what makes the label buffer stable.
  id: i32 = 0;
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
    this.label(label);
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
  // A UTF-16 char takes at most 4 UTF-8 bytes, plus headroom; the author's
  // reserve may ask for more.
  const content = src.length * 4 + 8;
  const want = w.labelCap > content ? w.labelCap : content;
  // (AssemblyScript does no flow narrowing: a `PoolBuf | null` variable
  // assigned inside an `if` is still nullable after it, so the branch ends
  // in plain scalars.)
  //
  // The lookup must go through `has` first: unlike JavaScript, the
  // AssemblyScript stdlib's `Map.get` *throws* on an absent key (it cannot
  // represent `undefined`) — a throw is an `abort` trap in wasm, so a
  // missing key would kill the call. `has` is a pure bool; `get` on a key
  // `has` confirmed is safe.
  let off: i32;
  let cap: i32;
  if (w.id > 0 && pool.has(w.id)) {
    const existing = pool.get(w.id);
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
    if (w.id > 0) {
      const buf = new PoolBuf();
      buf.off = off;
      buf.cap = cap;
      pool.set(w.id, buf);
    }
    // id-less: fresh partition per build (documented in the header)
  }
  w.labelPtr = off;
  w.labelLen = writeUtf8(off, src, off + cap) - off;
}

// ------------------------------------------------------------- projection

/// Count the nodes of a tree (depth-first pre-order), for sizing the arena.
export function count(w: Widget): i32 {
  let n = 1;
  const c = w.children;
  for (let i = 0; i < c.length; i++) n += count(c[i]);
  return n;
}

/// Emit `w` and its descendants as records at `buf + off * stride`, in the
/// contract's depth-first pre-order. With `isMutation`, each record is a
/// 36-byte `changed` mutation (the tag word in front of the 32-byte node).
/// Returns the next offset.
///
/// The walk stops when it reaches the arena's cap instead of running on:
/// the records would otherwise walk into the label pool. A too-big tree
/// therefore renders *less* rather than corrupts.
function lower(w: Widget, parent: i32, buf: i32, off: i32, isMutation: i32): i32 {
  // The stride must be computed first: AssemblyScript (like JavaScript)
  // binds `<` tighter than `?:`, so an unparenthesized `buf + off *
  // isMutation != 0 ? 36 : 32` would parse as a ternary-as-condition.
  const stride = isMutation != 0 ? 36 : 32;
  const rec = buf + off * stride;
  if (rec + stride > RESULT_END) return off;
  materializeLabel(w);
  const id = w.id;
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
  let next = off + 1;
  const c = w.children;
  for (let i = 0; i < c.length; i++) {
    next = lower(c[i], id, buf, next, isMutation);
  }
  return next;
}

/// Project the whole tree as a `view()` result: every node as a 32-byte node
/// record in depth-first pre-order, behind a list descriptor. Always the
/// full truth.
export function viewOf(root: Widget): i32 {
  const n = count(root);
  const buf = arenaAlloc(n * 32);
  lower(root, 0, buf, 0, 0);
  return makeList(buf, n);
}

/// Project one subtree as `changed` mutations (the diff channel). Its root's
/// parent is stated explicitly: projected standing alone, it must name its
/// box (the `view` walk would otherwise state it for it).
export function mutationsOf(root: Widget, parent: i32): i32 {
  const n = count(root);
  const buf = arenaAlloc(n * 36);
  lower(root, parent, buf, 0, 1);
  return makeList(buf, n);
}

/// Project a *set* of subtrees — the demo's dirty nodes, each with the same
/// explicit parent — as one `changed` mutation list.
export function mutationsOfAll(roots: Widget[], parent: i32): i32 {
  let n = 0;
  for (let i = 0; i < roots.length; i++) n += count(roots[i]);
  if (n == 0) return makeList(0, 0); // an empty list's ptr is 0
  const buf = arenaAlloc(n * 36);
  let off = 0;
  for (let i = 0; i < roots.length; i++) {
    off = lower(roots[i], parent, buf, off, 1);
  }
  return makeList(buf, n);
}
