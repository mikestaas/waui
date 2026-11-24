// The AssemblyScript twin of the Rust `component-ui` crate: the widget tree
// an author writes, projected through the `../abi` emitters into the
// contract's flat, pre-ordered node list and its diff channel.
//
// The design lives in `../README.md`: nodes on the incremental runtime heap,
// the JSX factory, the page-0 label pool, and the diff (a record comparison
// against the retained last export).
//
// This module provides the tree, the pool, the JSX factory (`h`), and the
// projection (`viewOf`, `diffOf`, `count`).

import { arenaAlloc, arenaAllocFit, HOST_SCRATCH_BASE, RESULT_END } from '../abi/arena';
import {
  ALIGN_CENTER,
  BACKGROUND_NONE,
  BORDER_NONE,
  COLOR_NEUTRAL,
  CURSOR_DEFAULT,
  DIRECTION_COLUMN,
  DIRECTION_ROW,
  MUT_CHANGED,
  OVERFLOW_CLIP,
  RADIUS_NONE,
  SHADOW_NONE,
  SIZE_MEDIUM,
  STRETCH_NONE,
  TAG_BUTTON,
  TAG_CONTAINER,
  TAG_IMAGE,
  TAG_INPUT,
  TAG_LINK,
  TAG_TEXT,
  emitButton,
  emitContainer,
  emitImage,
  emitInput,
  emitLink,
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
// Bump cursor, RESULT_END → the scratch base (the pool's edge).
var poolCur = RESULT_END;

/// Bump-allocate `n` bytes from the pool, 4-byte aligned. Returns -1 when
/// the request is not positive or the region would overflow the pool (into
/// the host's scratch region) — the caller then renders an empty label
/// rather than walk into anything.
function poolAlloc(n: i32): i32 {
  if (n <= 0) return -1;
  const p = poolCur;
  const end = p + ((n + 3) & ~3);
  if (end > HOST_SCRATCH_BASE || end <= p) return -1;
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
  /// The style (text, button, and input only).
  color: i32 = COLOR_NEUTRAL;
  size: i32 = SIZE_MEDIUM;
  /// Containers only.
  direction: i32 = DIRECTION_ROW;
  /// Containers only: the placement of the children, on both axes. The
  /// storage is `_align`, set with `.align()` (Rust's `Tree` has an `align`
  /// builder): AssemblyScript cannot name a field and a method alike the way
  /// Rust can.
  _align: i32 = ALIGN_CENTER;
  /// The width: an input's field width in characters, an image's width, or a
  /// container's `w` — in the host's units for the latter two (all `0` = the
  /// host's default). The storage is `_width`, set with `.width()`:
  /// AssemblyScript cannot name a field and a method alike the way Rust can.
  _width: i32 = 0;
  /// An image's height, or a container's `h` — in the host's units
  /// (`0` = the host's default). The storage is `_height`, set with `.height()`.
  _height: i32 = 0;
  // Containers only: the geometry and the chrome tokens. A `0`/token-default
  // leaves the dimension to the host, as with the style tokens. The emitter
  // reads these per kind, so a stray one on a leaf is inert.
  _positioned: i32 = 0;
  _x: i32 = 0;
  _y: i32 = 0;
  _z: i32 = 0;
  _gap: i32 = 0;
  _padding: i32 = 0;
  _stretch: i32 = STRETCH_NONE;
  _overflow: i32 = OVERFLOW_CLIP;
  _background: i32 = BACKGROUND_NONE;
  _border: i32 = BORDER_NONE;
  _radius: i32 = RADIUS_NONE;
  _shadow: i32 = SHADOW_NONE;
  _cursor: i32 = CURSOR_DEFAULT;
  /// The children, in the contract's depth-first pre-order.
  children: Widget[];
  /// The label's (or an image's `src`) UTF-16 source; materialized to the
  /// pool (an id'd node) or the result arena (an id-less one) at projection.
  labelSrc: string = '';
  /// The label buffer's requested capacity (consumed at first allocation).
  labelCap: i32 = 0;
  // Materialized by `lower` (valid from projection on):
  labelPtr: i32 = 0; // the partition's base (0 = no label)
  labelLen: i32 = 0; // UTF-8 bytes, clamped to the partition
  /// A link's `href` UTF-16 source; materialized to the result arena at
  /// projection (see `materializeHref`).
  hrefSrc: string = '';
  // Materialized by `lower` (valid from projection on):
  hrefPtr: i32 = 0; // 0 = no href
  hrefLen: i32 = 0; // UTF-8 bytes

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

  /// Replace the href (a link only; UTF-16 in, UTF-8 in the arena at
  /// projection).
  href(s: string): this {
    this.hrefSrc = s;
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

  /// Replace a container's placement, both axes (a no-op on a leaf).
  align(v: i32): this {
    this._align = v;
    return this;
  }

  /// Set the width: an input's field width in characters, or an image's
  /// width in the host's units (both 0 = the host's default).
  width(n: i32): this {
    this._width = n;
    return this;
  }

  /// Set the image's height, or a container's `h`, in the host's units
  /// (0 = the host's default).
  height(n: i32): this {
    this._height = n;
    return this;
  }

  /// Place a container at `(x, y)` relative to its parent, marking it
  /// positioned. Inert on a leaf.
  at(x: i32, y: i32): this {
    this._x = x;
    this._y = y;
    this._positioned = 1;
    return this;
  }

  /// Replace a container's stacking order (`0` = none). Inert on a leaf.
  z(n: i32): this {
    this._z = n;
    return this;
  }

  /// Replace a container's child gap, in the host's units. Inert on a leaf.
  gap(n: i32): this {
    this._gap = n;
    return this;
  }

  /// Replace a container's padding, in the host's units. Inert on a leaf.
  padding(n: i32): this {
    this._padding = n;
    return this;
  }

  /// Replace a container's fill behavior. Inert on a leaf.
  stretch(v: i32): this {
    this._stretch = v;
    return this;
  }

  /// Replace a container's overflow behavior. Inert on a leaf.
  overflow(v: i32): this {
    this._overflow = v;
    return this;
  }

  /// Replace a container's background. Inert on a leaf.
  background(v: i32): this {
    this._background = v;
    return this;
  }

  /// Replace a container's border. Inert on a leaf.
  border(v: i32): this {
    this._border = v;
    return this;
  }

  /// Replace a container's corner radius. Inert on a leaf.
  radius(v: i32): this {
    this._radius = v;
    return this;
  }

  /// Replace a container's shadow. Inert on a leaf.
  shadow(v: i32): this {
    this._shadow = v;
    return this;
  }

  /// Replace a container's cursor. Inert on a leaf.
  cursor(v: i32): this {
    this._cursor = v;
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

/// A single-line text input.
export function input(
  content: string,
  color: i32 = COLOR_NEUTRAL,
  size: i32 = SIZE_MEDIUM,
): Widget {
  const w = new Widget(TAG_INPUT, content);
  w.color = color;
  w.size = size;
  return w;
}

/// An image. The `src` is a path or asset name the host resolves to pixels;
/// the width and height are the host's units, `0` = the host's default.
export function image(src: string, width: i32 = 0, height: i32 = 0): Widget {
  const w = new Widget(TAG_IMAGE, src);
  w._width = width;
  w._height = height;
  return w;
}

/// A hyperlink. `href` is the URL the host opens (a new tab or window on the
/// web host, the operating system's default handler elsewhere); the host owns
/// the opening and never reports the activation through `on-click`.
export function link(
  label: string,
  href: string,
  color: i32 = COLOR_NEUTRAL,
  size: i32 = SIZE_MEDIUM,
): Widget {
  const w = new Widget(TAG_LINK, label);
  w.hrefSrc = href;
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

// ------------------------------------------------------------------ the JSX

/// The props bag of a JSX element. Each field is one attribute the factory
/// understands; an attribute absent from the element takes the field's
/// default. AssemblyScript coerces the element's object literal to this
/// class, so a typo'd attribute name is a compile error.
export class Props {
  id: i32 = 0;
  label: string = '';
  href: string = ''; // honored only by `link`
  color: i32 = COLOR_NEUTRAL;
  size: i32 = SIZE_MEDIUM;
  reserve: i32 = 0;
  direction: i32 = DIRECTION_ROW; // honored only by `container`
  align: i32 = ALIGN_CENTER; // honored by `row`/`column`/`container`
  stretch: i32 = STRETCH_NONE; // honored by `row`/`column`/`container`
  width: i32 = 0; // honored by `input` (characters) and `image` (host units)
  height: i32 = 0; // honored only by `image` (host units)
}

/// The JSX factory that `../jsx.mjs` emits calls to (its `pragma`): the tag
/// picks the constructor, the props bag (null when the element has no
/// attributes) is applied on top, and the children — a widget or a spliced
/// list — normalize to a `Widget[]` (see `../README.md`).
export function h(tag: string, props: Props | null, ...children: Object[]): Widget {
  // The children, normalized to a list. The common case — every child a
  // widget — reuses the rest array as is (a Widget and a Widget[] are both
  // object references, so the cast is safe); a spliced list child copies
  // into a fresh array.
  let allWidgets = true;
  for (let i = 0; i < children.length; i++) {
    if (!(children[i] instanceof Widget)) {
      allWidgets = false;
      break;
    }
  }
  let kids: Widget[] = allWidgets ? changetype<Widget[]>(children) : new Array<Widget>();
  if (!allWidgets) {
    for (let i = 0; i < children.length; i++) {
      const c = children[i];
      if (c instanceof Widget) {
        kids.push(c as Widget);
      } else {
        const list = c as Widget[];
        for (let j = 0; j < list.length; j++) kids.push(list[j]);
      }
    }
  }
  let w: Widget;
  if (tag == 'text') {
    w = text('');
  } else if (tag == 'button') {
    w = button('');
  } else if (tag == 'input') {
    w = input('');
  } else if (tag == 'image') {
    w = image('');
  } else if (tag == 'link') {
    w = link('', '');
  } else if (tag == 'row') {
    w = row(kids);
  } else if (tag == 'column') {
    w = column(kids);
  } else if (tag == 'container') {
    w = new Widget(TAG_CONTAINER, '', kids);
  } else {
    throw new Error('unknown tag: ' + tag);
  }
  if (props != null) {
    if (props.id != 0) w = w.id(props.id);
    if (props.label.length != 0) w = w.label(props.label);
    if (props.href.length != 0) w = w.href(props.href);
    if (props.color != COLOR_NEUTRAL || props.size != SIZE_MEDIUM) {
      w = w.style(props.color, props.size);
    }
    if (props.reserve != 0) w = w.reserve(props.reserve);
    // Only `container` takes its direction from the props: `row`/`column`
    // fix it in their constructor, and applying the props default here would
    // silently flip them. `align` and `stretch` are orthogonal to direction,
    // so all three container tags take them (the props defaults match the
    // factory defaults, so an unstated one is a no-op).
    if (tag == 'container') w.direction = props.direction;
    if (tag == 'row' || tag == 'column' || tag == 'container') {
      w._align = props.align;
      w._stretch = props.stretch;
    }
    // The WIT's `width` is an input field (characters) or an image (host
    // units); a `width` on any other tag is ignored, as in the Rust `ui!`.
    // `height` is an image field only.
    if (tag == 'input' || tag == 'image') w = w.width(props.width);
    if (tag == 'image') w = w.height(props.height);
  }
  return w;
}

// ------------------------------------------------------------- the label seam

/// Materialize the node's label bytes (an image's `src`) where the host will
/// read them from.
///
/// An *id'd* node takes a static **pool** partition keyed by its id: sized at
/// first allocation (honoring `reserve`), never moves or grows, and later
/// content overwrites it in place, clamped to fit. The address is stable
/// across rebuilds, so a changing label of a stable node reuses its bytes.
///
/// An *id-less* node allocates from the **result arena** instead: fresh each
/// build, and freed with the rest of the result when the host posts it
/// (`cabi_post_*`). This is the per-build lifetime the Rust layer gives its
/// id-less strings (heap-allocated, freed with the tree), and it keeps a
/// changing id-less label from growing the pool until it is exhausted.
///
/// No-op for an empty label; an exhausted pool or arena renders an empty
/// label rather than corrupt memory.
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

  if (w._id > 0) {
    // A stable id: the pool partition, keyed by the id. `Map.get` throws on
    // an absent key (the stdlib cannot represent `undefined`, and a throw in
    // wasm is an `abort` trap), so the lookup goes through `has` first. No
    // flow narrowing, so the branch ends in scalars.
    let off: i32;
    let cap: i32;
    if (pool.has(w._id)) {
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
      const buf = new PoolBuf();
      buf.off = off;
      buf.cap = cap;
      pool.set(w._id, buf);
    }
    w.labelPtr = off;
    w.labelLen = writeUtf8(off, src, off + cap) - off;
    return;
  }

  // No id: the result arena, fresh per build (freed on the host's post).
  const off = arenaAllocFit(want);
  if (off < 0) {
    w.labelPtr = 0;
    w.labelLen = 0;
    return;
  }
  const cap = (want + 7) & ~7; // the arena allocated exactly this much
  w.labelPtr = off;
  w.labelLen = writeUtf8(off, src, off + cap) - off;
}

/// Materialize a link's `href` bytes.
///
/// Unlike the label, the href always takes the **result arena**: fresh each
/// build, freed with the rest of the result on the host's post. The label's
/// pool is keyed by node id and the label already occupies that id's
/// partition, so the href cannot share it; a changing href of a stable link
/// re-allocates per build, which the arena's per-build lifetime makes safe.
///
/// No-op for an empty href; an exhausted arena renders an empty href rather
/// than corrupt memory.
function materializeHref(w: Widget): void {
  const src = w.hrefSrc;
  if (src.length == 0) {
    w.hrefPtr = 0;
    w.hrefLen = 0;
    return;
  }
  const want = src.length * 4 + 8;
  const off = arenaAllocFit(want);
  if (off < 0) {
    w.hrefPtr = 0;
    w.hrefLen = 0;
    return;
  }
  const cap = (want + 7) & ~7; // the arena allocated exactly this much
  w.hrefPtr = off;
  w.hrefLen = writeUtf8(off, src, off + cap) - off;
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
  align: i32 = ALIGN_CENTER;
  width: i32 = 0; // inputs (characters), images and containers (host units)
  height: i32 = 0; // images and containers (host units)
  positioned: i32 = 0; // containers only
  x: i32 = 0;
  y: i32 = 0;
  z: i32 = 0;
  gap: i32 = 0;
  padding: i32 = 0;
  stretch: i32 = STRETCH_NONE;
  overflow: i32 = OVERFLOW_CLIP;
  background: i32 = BACKGROUND_NONE;
  border: i32 = BORDER_NONE;
  radius: i32 = RADIUS_NONE;
  shadow: i32 = SHADOW_NONE;
  cursor: i32 = CURSOR_DEFAULT;
  label: string = ''; // the content, or an image's `src`
  href: string = ''; // a link's target
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
  r.align = w._align;
  r.width = w._width;
  r.height = w._height;
  r.positioned = w._positioned;
  r.x = w._x;
  r.y = w._y;
  r.z = w._z;
  r.gap = w._gap;
  r.padding = w._padding;
  r.stretch = w._stretch;
  r.overflow = w._overflow;
  r.background = w._background;
  r.border = w._border;
  r.radius = w._radius;
  r.shadow = w._shadow;
  r.cursor = w._cursor;
  r.label = w.labelSrc;
  r.href = w.hrefSrc;
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
    // its direction, align, geometry, and chrome tokens (it carries no
    // style); an image's is its `src`, its width, and its height (it carries
    // no style); a text/button/input's is its label, its style, and — for an
    // input — its width; a link's is its label, its href, and its style. AS
    // compares strings by content.
    let differs = w.kind != r.kind || parent != r.parent;
    if (w.kind == TAG_CONTAINER) {
      differs = differs || w.direction != r.direction || w._align != r.align;
      differs = differs || w._positioned != r.positioned || w._x != r.x || w._y != r.y;
      differs = differs || w._width != r.width || w._height != r.height || w._z != r.z;
      differs = differs || w._gap != r.gap || w._padding != r.padding;
      differs = differs || w._stretch != r.stretch || w._overflow != r.overflow;
      differs = differs || w._background != r.background || w._border != r.border;
      differs = differs || w._radius != r.radius || w._shadow != r.shadow || w._cursor != r.cursor;
    } else if (w.kind == TAG_IMAGE) {
      differs = differs || w.labelSrc != r.label || w._width != r.width || w._height != r.height;
    } else {
      differs = differs || w.color != r.color || w.size != r.size || w.labelSrc != r.label;
      // The WIT's `width` is an input field; the other leaves carry 0.
      if (w.kind == TAG_INPUT) differs = differs || w._width != r.width;
      if (w.kind == TAG_LINK) differs = differs || w.hrefSrc != r.href;
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
/// record is a 68-byte `changed` mutation (the tag word, the 60-byte node,
/// and the index). Returns the next offset; stops at the arena's cap rather
/// than walk into the label pool.
function emitNode(w: Widget, parent: i32, index: i32, buf: i32, off: i32, isMutation: i32): i32 {
  // Computed first: AS (like JS) binds `<` tighter than `?:`, so an
  // unparenthesized `buf + off * isMutation != 0 ? 68 : 60` would parse as
  // a ternary *as* the comparison.
  const stride = isMutation != 0 ? 68 : 60;
  const rec = buf + off * stride;
  if (rec + stride > RESULT_END) return off;
  materializeLabel(w);
  if (w.kind == TAG_LINK) materializeHref(w);
  const id = w._id;
  if (isMutation != 0) store<i32>(rec, MUT_CHANGED);
  // A `changed` mutation carries the node one word in (after the tag word);
  // a plain node record *is* the record base. The emitters self-zero their
  // 60 bytes, so the offset must be exactly right: a stray word spills into
  // the neighbor record.
  const node = rec + (isMutation != 0 ? 4 : 0);
  const kind = w.kind;
  if (kind == TAG_TEXT) {
    emitText(node, id, parent, w.labelPtr, w.labelLen, w.color, w.size);
  } else if (kind == TAG_BUTTON) {
    emitButton(node, id, parent, w.labelPtr, w.labelLen, w.color, w.size);
  } else if (kind == TAG_INPUT) {
    emitInput(node, id, parent, w.labelPtr, w.labelLen, w.color, w.size, w._width);
  } else if (kind == TAG_IMAGE) {
    emitImage(node, id, parent, w.labelPtr, w.labelLen, w._width, w._height);
  } else if (kind == TAG_LINK) {
    emitLink(node, id, parent, w.labelPtr, w.labelLen, w.hrefPtr, w.hrefLen, w.color, w.size);
  } else {
    emitContainer(
      node,
      id,
      parent,
      w.direction,
      w._align,
      w._positioned,
      w._x,
      w._y,
      w._width,
      w._height,
      w._z,
      w._gap,
      w._padding,
      w._stretch,
      w._overflow,
      w._background,
      w._border,
      w._radius,
      w._shadow,
      w._cursor,
    );
  }
  // The `changed` mutation's index (the node's 0-based position among its
  // parent's children) sits after the 60-byte node; a plain node has none.
  if (isMutation != 0) store<i32>(rec + 64, index);
  return off + 1;
}

/// Emit `w` and its descendants in depth-first pre-order (see `emitNode`).
/// `index` is `w`'s 0-based position among its parent's children. Returns
/// the next offset.
function lower(w: Widget, parent: i32, index: i32, buf: i32, off: i32, isMutation: i32): i32 {
  const id = w._id;
  let next = emitNode(w, parent, index, buf, off, isMutation);
  const c = w.children;
  for (let i = 0; i < c.length; i++) {
    next = lower(c[i], id, i, buf, next, isMutation);
  }
  return next;
}

/// Emit the `changed` nodes of a tree in depth-first pre-order, each with the
/// parent and index from the structure. Non-changed nodes are skipped but
/// descended into: a changed child sits under its unchanged parent. Returns
/// the next offset.
function lowerChanged(
  w: Widget,
  parent: i32,
  index: i32,
  buf: i32,
  off: i32,
  first: i32,
  last: FlatRec[],
): i32 {
  let next = off;
  if (isChanged(w, parent, first, last) != 0) {
    next = emitNode(w, parent, index, buf, off, 1);
  }
  const id = w._id;
  const c = w.children;
  for (let i = 0; i < c.length; i++) {
    next = lowerChanged(c[i], id, i, buf, next, first, last);
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
      emitRemoved(buf + next * 68, id);
      next += 1;
    }
  }
  return next;
}

/// Project the whole tree as a `view()` result: every node as a 60-byte node
/// record in depth-first pre-order, behind a list descriptor. The snapshot
/// becomes the next diff's reference (see `../README.md`).
export function viewOf(root: Widget): i32 {
  everExported = 1;
  const recs = new Array<FlatRec>();
  flatten(root, 0, recs);
  lastExport = recs;
  const n = count(root);
  const buf = arenaAlloc(n * 60);
  lower(root, 0, 0, buf, 0, 0);
  return makeList(buf, n);
}

/// Project the diff channel: the record comparison against the last export,
/// with the bootstrap rule and the ordering in `../README.md`. The new tree
/// becomes the next reference.
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
  const buf = arenaAlloc(n * 68);
  let off = lowerChanged(root, 0, 0, buf, 0, first, last);
  if (first == 0) {
    lowerRemoved(last, newRecs, buf, off);
  }
  return makeList(buf, n);
}
