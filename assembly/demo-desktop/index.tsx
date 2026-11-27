// The desktop, in AssemblyScript — the same component as `crates/demo-desktop`,
// same `waui:ui/app` world. See `../README.md` for the AS tree layer, the
// build pipeline, and the memory layout.
//
// A just-an-app window manager. The whole interface is this component's tree:
// the menu bar, the icon grid, and the set of open windows (their chrome —
// title bar, close, grip, panel, border, shadow — and their geometry). It draws
// the windows itself, with the contract's geometry tokens, and drives them
// through the pointer and key channels: a title-bar drag moves a window, a grip
// drag resizes it, a press raises it, the close marker (and Escape) closes it,
// and a double-clicked tile opens its entry.
//
// The *content* of an opened window is not this component's: it is a separate
// component the host runs in its own sandbox. On a double-click this component
// calls `open-entry` (the host instantiates the entry's component and returns a
// handle) and draws a window whose content slot is that handle: the host seats
// the opened component's tree in the slot. Closing a window calls `close-entry`,
// so the host drops the sandbox.
//
// The tile's presentation comes from the entry's `data` blob (a JSON object,
// host-defined): the label, the caption, the icon, and the window's default
// size. A field the blob lacks is rendered without, and a blob that will not
// parse renders the name only.
//
// **The ids.** The root is 1, the menu bar 2 (its spacer 3, its count 4), the
// grid area 6, the grid 7, the rows 8+r, and the tiles 100+i. A window's nodes
// are derived from the host handle: the content slot is the handle itself, and
// the chrome is offset by a wide stride (window +100000, title bar +200000,
// spacer +300000, close +400000, grip +500000) so they never collide across
// handles. A tile's icon, label, and caption are id-less (display-only); the
// title bar's label is id-less too (the host makes id'd nodes addressable),
// and the close glyph is id-less (its id rides on its container).
//
// It never calls `schedule` (it changes only on a pointer or key event).

import { arenaAlloc, cabi_post_changes, cabi_post_view, cabi_realloc } from '../abi/arena';
import {
  ALIGN_CENTER,
  ALIGN_START,
  BACKGROUND_BAR,
  BACKGROUND_NONE,
  BACKGROUND_PANEL,
  BORDER_LINE,
  BORDER_NONE,
  BORDER_RING,
  COLOR_ACCENT,
  COLOR_DIM,
  COLOR_NEUTRAL,
  CURSOR_GRAB,
  CURSOR_POINTER,
  CURSOR_RESIZE,
  DIRECTION_COLUMN,
  DIRECTION_ROW,
  OVERFLOW_CLIP,
  RADIUS_LARGE,
  SHADOW_RAISED,
  SIZE_LARGE,
  SIZE_MEDIUM,
  SIZE_SMALL,
  STRETCH_BOTH,
  STRETCH_CROSS,
  STRETCH_MAIN,
} from '../abi/node';
import { readUtf8, writeUtf8 } from '../abi/utf8';
import { closeEntry, listEntries, openEntry, viewport } from '../abi/windowing';
import { column, diffOf, image, link, row, text, viewOf, Widget } from '../ui/tree';

// The canonical-ABI post functions and the string allocator (../abi/arena).
export { cabi_post_changes, cabi_post_view, cabi_realloc };

// ---------------------------------------------------------------- constants

// Layout, in the host's units (pixels on the web host).
const MENUBAR_H = 34; // the menu bar's height
const TITLEBAR_H = 36; // the title bar's height
const MIN_W = 220; // a window's minimum size
const MIN_H = 260;
const DEFAULT_W = 300; // a window's default size (a dimension the blob lacks)
const DEFAULT_H = 380;
const CASCADE_STEP = 28; // the new-window cascade step
const Z_BASE = 10; // the first z to assign; a raised window takes the next
const TILE_BASE = 100; // the first tile's id; tile i is TILE_BASE + i
const COLS = 3; // tiles per row of the grid
const TILE_PX = 112; // the tile's icon size

// The window node ids, derived from the host handle (see the header).
const OFF_WINDOW = 100000;
const OFF_TITLEBAR = 200000;
const OFF_SPACER = 300000;
const OFF_CLOSE = 400000;
const OFF_GRIP = 500000;

// Which part of a window a node is.
const KIND_WINDOW = 0;
const KIND_TITLEBAR = 1;
const KIND_CLOSE = 2;
const KIND_CONTENT = 3;
const KIND_GRIP = 4;

// The in-progress drag or resize.
const DRAG_MOVE = 0; // a title-bar drag: move the window
const DRAG_RESIZE = 1; // a grip resize: grow the window from its origin

// ------------------------------------------------------------------ state

/// An entry's parsed `data`: the tile's presentation and the window's default
/// size. A field the blob lacks keeps its fallback (an empty string = absent).
class EntryMeta {
  name: string = ''; // the entry's name: the open's identifier, the label's fallback
  label: string = ''; // the tile's display label
  lang: string = ''; // the tile's caption (empty = absent)
  icon: string = ''; // the tile's and title bar's icon (empty = absent)
  width: i32 = 0; // the window's default size; 0 leaves it to the host's default
  height: i32 = 0;
}

/// An open window: the host handle (also the content slot's id), the entry's
/// presentation, and the window's geometry and stacking order.
class Win {
  handle: i32 = 0;
  label: string = '';
  icon: string = '';
  x: i32 = 0;
  y: i32 = 0;
  w: i32 = 0;
  h: i32 = 0;
  z: i32 = 0;
}

/// The in-progress drag or resize; the fields are used per `kind`.
class Drag {
  kind: i32 = DRAG_MOVE;
  handle: i32 = 0;
  dx: i32 = 0; // Move: the grab offset
  dy: i32 = 0;
  startX: i32 = 0; // Resize: the grab origin and the window's start size
  startY: i32 = 0;
  startW: i32 = 0;
  startH: i32 = 0;
}

// Module globals, so they survive between exports (wasm is single-threaded, so
// no locks are needed).
var entries: EntryMeta[] = new Array<EntryMeta>();
var inited: i32 = 0;
var windows: Win[] = new Array<Win>();
var nextZ: i32 = Z_BASE;
var cascade: i32 = 0; // the new-window placement counter (wraps after 9)
var selected: i32 = -1; // the selected tile's index; -1 = none
var drag: Drag | null = null;
var vpW: i32 = 0; // the host viewport, from the last `viewport` query
var vpH: i32 = 0;

// ---------------------------------------------------------------- the registry

/// The cursor of a flat JSON object (the `data` blob): a string of string and
/// number values. A real component would use a full parser; this one reads
/// the shape this host's registry emits, and anything else (nesting, arrays,
/// a malformed blob) stops the parse, so the caller's fallbacks apply.
class JsonParser {
  s: string;
  i: i32 = 0;
  constructor(s: string) {
    this.s = s;
  }

  skip(): void {
    while (this.i < this.s.length) {
      const c = this.s.charCodeAt(this.i);
      if (c == 32 || c == 9 || c == 10 || c == 13) this.i++;
      else break;
    }
  }

  peek(): i32 {
    if (this.i >= this.s.length) return -1;
    return this.s.charCodeAt(this.i);
  }

  eat(c: i32): boolean {
    if (this.peek() == c) {
      this.i++;
      return true;
    }
    return false;
  }

  /// A JSON string, escapes resolved. The registry is ASCII, so the UTF-16
  /// code units are the characters.
  string(): string {
    if (!this.eat(34 /* " */)) return '';
    let out = '';
    while (this.i < this.s.length) {
      const c = this.s.charCodeAt(this.i);
      this.i++;
      if (c == 34 /* " */) break; // closing quote
      if (c == 92 /* \ */) {
        const e = this.s.charCodeAt(this.i);
        this.i++;
        if (e == 34) out += '"';
        else if (e == 92) out += '\\';
        else if (e == 47) out += '/';
        else if (e == 98) out += '\b';
        else if (e == 102) out += '\f';
        else if (e == 110) out += '\n';
        else if (e == 114) out += '\r';
        else if (e == 116) out += '\t';
        else if (e == 117) {
          const cp = hexToCode(this.s.substring(this.i, this.i + 4));
          this.i += 4;
          if (cp >= 0) out += String.fromCharCode(cp);
        }
        // an unknown escape is skipped
      } else {
        out += String.fromCharCode(c);
      }
    }
    return out;
  }

  /// A JSON number, as an i32 (the registry's sizes are small). The cursor
  /// advances over the whole token (as the Rust parser does); the value is
  /// the token's integer part.
  number(): i32 {
    const start = this.i;
    while (this.i < this.s.length) {
      const c = this.s.charCodeAt(this.i);
      if ((c >= 48 && c <= 57) || c == 45 || c == 43 || c == 46 || c == 101 || c == 69) {
        this.i++;
      } else break;
    }
    if (this.i == start) return -1; // not a number
    const tok = this.s.substring(start, this.i);
    let v = 0;
    let p = 0;
    let minus = 0;
    if (p < tok.length && tok.charCodeAt(p) == 45 /* - */) {
      minus = 1;
      p++;
    }
    while (p < tok.length) {
      const c = tok.charCodeAt(p);
      if (c < 48 || c > 57) break;
      v = v * 10 + (c - 48);
      p++;
    }
    return minus != 0 ? -v : v;
  }
}

/// The four hex digits of a `\uXXXX` escape as a code point, or -1.
function hexToCode(hex: string): i32 {
  let v = 0;
  for (let i = 0; i < 4; i++) {
    const c = i < hex.length ? hex.charCodeAt(i) : -1;
    let d: i32;
    if (c >= 48 && c <= 57) d = c - 48;
    else if (c >= 65 && c <= 70) d = c - 65 + 10;
    else if (c >= 97 && c <= 102) d = c - 97 + 10;
    else return -1;
    v = v * 16 + d;
  }
  return v;
}

/// Parse an entry's `data` blob into its metadata, in place. The blob is a
/// JSON object, host-defined; the fields this host supplies are `label` (the
/// tile's display label), `lang` (its caption), `icon` (its icon), and
/// `width`/`height` (the window's default size). A field the blob lacks keeps
/// its fallback; a blob that will not parse keeps the fallbacks parsed so far.
function parseMeta(m: EntryMeta, data: string): void {
  const p = new JsonParser(data);
  p.skip();
  if (!p.eat(123 /* { */)) return; // not an object: the fallbacks
  for (;;) {
    p.skip();
    if (p.eat(125 /* } */)) break; // done
    const key = p.string();
    p.skip();
    if (!p.eat(58 /* : */)) return; // malformed: keep what was parsed
    p.skip();
    if (p.peek() == 34 /* " */) {
      const v = p.string();
      if (key == 'label') m.label = v;
      else if (key == 'lang') m.lang = v;
      else if (key == 'icon') m.icon = v;
    } else {
      const v = p.number();
      if (v >= 0) {
        if (key == 'width') m.width = v;
        else if (key == 'height') m.height = v;
      }
    }
    p.skip();
    if (p.eat(44 /* , */)) continue; // the next pair
    if (!p.eat(125 /* } */)) return; // malformed: keep what was parsed
    break;
  }
}

/// The registry, fetched once: each entry's name plus its parsed `data`. The
/// host writes the records and strings into the module's scratch region
/// (through `cabi_realloc`), so the component copies them into retained
/// strings and frees the scratch.
function fetchEntries(): EntryMeta[] {
  const out: EntryMeta[] = new Array<EntryMeta>();
  const desc = arenaAlloc(8);
  listEntries(desc);
  const ptr = load<i32>(desc + 0);
  const len = load<i32>(desc + 4);
  for (let i = 0; i < len; i++) {
    const o = ptr + i * 16;
    const name = readUtf8(load<i32>(o + 0), load<i32>(o + 4));
    const data = readUtf8(load<i32>(o + 8), load<i32>(o + 12));
    const m = new EntryMeta();
    m.name = name;
    m.label = name; // the fallback display label
    parseMeta(m, data);
    out.push(m);
  }
  // The scratch is a bump region the host filled; a free returns it to base.
  cabi_realloc(0, 0, 1, 0);
  return out;
}

/// The current tree: the registry is fetched on the first build, and the grid
/// and the count are derived from it and the open set.
function currentTree(): Widget {
  if (inited == 0) {
    inited = 1;
    entries = fetchEntries();
  }
  return tree(entries, windows);
}

// ---------------------------------------------------------------- the tree

/// The whole UI, nested: the menu bar, the grid, and the open windows.
function tree(entries: EntryMeta[], windows: Win[]): Widget {
  const kids: Widget[] = new Array<Widget>();
  kids.push(menubar(windows.length));
  kids.push(gridArea(entries));
  for (let i = 0; i < windows.length; i++) kids.push(window(windows[i]));
  return column(kids).id(1).align(ALIGN_START).stretch(STRETCH_BOTH).background(BACKGROUND_NONE);
}

/// The menu bar: the name, a spacer, the window count, a hint, and the repo
/// link. The id-less nodes never diff, so only the count (id 4) does.
function menubar(nOpen: i32): Widget {
  const plural = nOpen == 1 ? '' : 's';
  const kids: Widget[] = new Array<Widget>();
  kids.push(text('waui', COLOR_ACCENT, SIZE_LARGE));
  kids.push(text('wasm ui demos', COLOR_DIM, SIZE_SMALL));
  kids.push(row([]).id(3).stretch(STRETCH_MAIN));
  kids.push(text(`${nOpen} window${plural}`, COLOR_DIM, SIZE_SMALL).id(4).reserve(16));
  kids.push(text('double-click an icon to open', COLOR_DIM, SIZE_SMALL));
  kids.push(link('GitHub', 'https://github.com/mikestaas/waui', COLOR_ACCENT, SIZE_SMALL));
  return row(kids)
    .id(2)
    .align(ALIGN_CENTER)
    .stretch(STRETCH_MAIN)
    .height(MENUBAR_H)
    .z(1000)
    .background(BACKGROUND_BAR)
    .border(BORDER_LINE)
    .gap(12)
    .padding(16);
}

/// The grid's area: it fills the space below the menu bar and centers the grid.
function gridArea(entries: EntryMeta[]): Widget {
  const rows: Widget[] = new Array<Widget>();
  const nRows = (entries.length + COLS - 1) / COLS;
  for (let r = 0; r < nRows; r++) {
    const tiles: Widget[] = new Array<Widget>();
    for (let c = 0; c < COLS; c++) {
      const i = r * COLS + c;
      if (i < entries.length) tiles.push(tile(entries[i], i));
    }
    rows.push(
      row(tiles)
        .id(8 + r)
        .align(ALIGN_CENTER)
        .gap(44),
    );
  }
  const grid = column(rows).id(7).align(ALIGN_CENTER).stretch(STRETCH_CROSS).gap(28);
  const area: Widget[] = [grid];
  return row(area).id(6).align(ALIGN_CENTER).stretch(STRETCH_BOTH).overflow(OVERFLOW_CLIP);
}

/// One tile: an icon, a label, and a caption, in a double-clickable column.
/// The selected tile takes the ring. A field the entry's blob lacks is
/// rendered without.
function tile(e: EntryMeta, i: i32): Widget {
  const kids: Widget[] = new Array<Widget>();
  if (e.icon.length != 0) kids.push(image(e.icon, TILE_PX, TILE_PX));
  kids.push(text(e.label, COLOR_NEUTRAL, SIZE_MEDIUM));
  if (e.lang.length != 0) kids.push(text(e.lang, COLOR_NEUTRAL, SIZE_SMALL));
  let t = column(kids)
    .id(TILE_BASE + i)
    .align(ALIGN_CENTER)
    .gap(7)
    .padding(10)
    .radius(RADIUS_LARGE)
    .cursor(CURSOR_POINTER);
  if (selected == i) t = t.border(BORDER_RING);
  return t;
}

/// One window: the title bar, the content slot, and the grip. The content
/// slot's id is the handle (the host seats the opened component's tree there);
/// it is empty in this tree.
function window(w: Win): Widget {
  const base = w.handle;
  // The title bar.
  const bar: Widget[] = new Array<Widget>();
  if (w.icon.length != 0) bar.push(image(w.icon, 18, 18));
  bar.push(text(w.label, COLOR_NEUTRAL, SIZE_MEDIUM)); // id-less: the label
  bar.push(
    row([])
      .id(base + OFF_SPACER)
      .stretch(STRETCH_MAIN),
  );
  // The close marker as a text, not a button: a button node would draw the
  // host's button chrome (fill, border, padding). The container carries the
  // id (the click target) and a size (the hit area); with no chrome tokens
  // it draws nothing but the glyph. The pointer cursor is the affordance.
  // The hosts read that same token as "double-clickable"; a close is a
  // single click and on_double_click ignores close ids, so the coupling is
  // inert here.
  bar.push(
    row([text('✕', COLOR_NEUTRAL, SIZE_MEDIUM)])
      .id(base + OFF_CLOSE)
      .align(ALIGN_CENTER)
      .width(48)
      .height(32)
      .cursor(CURSOR_POINTER),
  );
  const titlebar = row(bar)
    .id(base + OFF_TITLEBAR)
    .align(ALIGN_CENTER)
    .stretch(STRETCH_MAIN)
    .height(TITLEBAR_H)
    .background(BACKGROUND_BAR)
    .border(BORDER_LINE)
    .cursor(CURSOR_GRAB);

  // The content slot: its id is the handle, and it is empty in this tree.
  const content = row([]).id(base).align(ALIGN_CENTER).stretch(STRETCH_BOTH);

  // The grip, at the window's bottom-right corner.
  const grip = row([])
    .id(base + OFF_GRIP)
    .at(w.w - 17, w.h - 17)
    .width(14)
    .height(14)
    .cursor(CURSOR_RESIZE);

  const kids: Widget[] = [titlebar, content, grip];
  return column(kids)
    .id(base + OFF_WINDOW)
    .at(w.x, w.y)
    .width(w.w)
    .height(w.h)
    .z(w.z)
    .align(ALIGN_START)
    .background(BACKGROUND_PANEL)
    .border(BORDER_LINE)
    .radius(RADIUS_LARGE)
    .shadow(SHADOW_RAISED)
    .overflow(OVERFLOW_CLIP);
}

// --------------------------------------------------------- window management

/// `a - b`, saturating at 0 (the host's units never go negative).
function ssub(a: i32, b: i32): i32 {
  return a < b ? 0 : a - b;
}

/// Clamp `v` to `[lo, hi]`, tolerating a degenerate range (a window larger
/// than the viewport) by returning the lower bound.
function clampI(v: i32, lo: i32, hi: i32): i32 {
  if (lo > hi) return lo;
  if (v < lo) return lo;
  if (v > hi) return hi;
  return v;
}

function clampU(v: i32, lo: i32, hi: i32): i32 {
  return clampI(v, lo, hi);
}

/// The host's viewport, in the host's units, into `vpW`/`vpH`. A host with no
/// viewport reports (0, 0), which the component treats as unknown. The return
/// buffer is arena-allocated (the canonical ABI's return region); it is freed
/// with the rest of the arena on the next post.
function refreshViewport(): void {
  const d = arenaAlloc(8);
  viewport(d);
  vpW = load<i32>(d + 0);
  vpH = load<i32>(d + 4);
}

/// The index of the window whose node has `id`, or -1 if `id` is not a window
/// node (a tile, a menu-bar node, or a node of another, closed window).
function findWindowNode(id: i32): i32 {
  for (let i = 0; i < windows.length; i++) {
    const h = windows[i].handle;
    if (
      id == h ||
      id == h + OFF_WINDOW ||
      id == h + OFF_TITLEBAR ||
      id == h + OFF_SPACER ||
      id == h + OFF_CLOSE ||
      id == h + OFF_GRIP
    ) {
      return i;
    }
  }
  return -1;
}

/// Which part of `w` the node with `id` is.
function nodeKind(id: i32, w: Win): i32 {
  const h = w.handle;
  if (id == h) return KIND_CONTENT;
  if (id == h + OFF_WINDOW) return KIND_WINDOW;
  if (id == h + OFF_TITLEBAR || id == h + OFF_SPACER) return KIND_TITLEBAR;
  if (id == h + OFF_CLOSE) return KIND_CLOSE;
  if (id == h + OFF_GRIP) return KIND_GRIP;
  return KIND_WINDOW;
}

/// Open the entry with `index`: request a window from the host (a cascade
/// placement, the entry's default size), and record it. A window the host
/// cannot open is dropped.
function openEntryAt(index: i32): void {
  if (index < 0 || index >= entries.length) return;
  const entry = entries[index];
  const w = entry.width > 0 ? entry.width : DEFAULT_W;
  const h = entry.height > 0 ? entry.height : DEFAULT_H;
  const place = cascadePlacement(w, h);
  const x = place[0];
  const y = place[1];
  // The name buffer is in the module's own memory (the host reads it); it is
  // arena-allocated and dies with the next post.
  const cap = entry.name.length * 4; // a UTF-16 char is at most 4 UTF-8 bytes
  const nameBuf = arenaAlloc(cap);
  const len = writeUtf8(nameBuf, entry.name, nameBuf + cap) - nameBuf;
  const retPtr = arenaAlloc(8);
  openEntry(nameBuf, len, x, y, w, h, retPtr);
  if (load<i8>(retPtr + 0) != 0) return; // an error: the window was not opened
  const win = new Win();
  win.handle = load<i32>(retPtr + 4); // the host's window handle
  win.label = entry.label;
  win.icon = entry.icon;
  win.x = x;
  win.y = y;
  win.w = w;
  win.h = h;
  win.z = nextZ;
  nextZ += 1;
  windows.push(win);
}

/// A new window's placement: cascade down-right, wrapping after 9, clamped to
/// the viewport. Skipped when the viewport is unknown (`(0, 0)`). Returns the
/// placement as a two-element array (x, y).
function cascadePlacement(w: i32, h: i32): i32[] {
  const n = cascade % 9;
  cascade += 1;
  refreshViewport();
  const vw = vpW;
  const vh = vpH;
  if (vw == 0 && vh == 0) {
    return [24 + n * CASCADE_STEP, 48 + n * CASCADE_STEP];
  }
  const left = clampU(24 + n * CASCADE_STEP, 8, ssub(ssub(vw, w), 8));
  const top = clampU(48 + n * CASCADE_STEP, MENUBAR_H + 4, ssub(ssub(vh, h), 8));
  return [left, top];
}

/// Close the window with `handle`: drop it and tell the host to free the
/// sandbox.
function closeWindow(handle: i32): void {
  for (let i = 0; i < windows.length; i++) {
    if (windows[i].handle == handle) {
      windows.splice(i, 1);
      break;
    }
  }
  closeEntry(handle);
}

// ---------------------------------------------------------------- the world

export function view(): i32 {
  return viewOf(currentTree());
}

export function changes(): i32 {
  return diffOf(currentTree());
}

/// A single click: the close marker closes its window; a tile selects it (its
/// ring).
export function onclick(id: i32): void {
  const wi = findWindowNode(id);
  if (wi >= 0 && nodeKind(id, windows[wi]) == KIND_CLOSE) {
    closeWindow(windows[wi].handle);
    return;
  }
  const i = id - TILE_BASE;
  if (i >= 0 && i < entries.length) {
    selected = i;
  }
}

/// A double-clicked tile opens its entry.
export function ondoubleclick(id: i32): void {
  const i = id - TILE_BASE;
  if (i < 0 || i >= entries.length) return; // not a tile (or out of range)
  selected = i;
  openEntryAt(i);
}

/// The pointer was pressed over a window node: raise it, and start a drag
/// (title bar) or resize (grip).
export function onpointerdown(id: i32, x: i32, y: i32): void {
  const wi = findWindowNode(id);
  if (wi < 0) return;
  const w = windows[wi];
  const handle = w.handle;
  w.z = nextZ;
  nextZ += 1;
  const kind = nodeKind(id, w);
  if (kind == KIND_TITLEBAR) {
    const d = new Drag();
    d.kind = DRAG_MOVE;
    d.handle = handle;
    d.dx = x - w.x;
    d.dy = y - w.y;
    drag = d;
  } else if (kind == KIND_GRIP) {
    const d = new Drag();
    d.kind = DRAG_RESIZE;
    d.handle = handle;
    d.startX = x;
    d.startY = y;
    d.startW = w.w;
    d.startH = w.h;
    drag = d;
  } else {
    // A press on the window, its content, or the close marker only raises
    // (the close is a click, not a drag).
    drag = null;
  }
}

/// The pointer moved while a button is held: continue the drag or resize.
export function onpointermove(x: i32, y: i32): void {
  const d = drag;
  if (d == null) return;
  const wi = findWindowNode(d.handle);
  if (wi < 0) return;
  const w = windows[wi];
  refreshViewport();
  const vw = vpW;
  const vh = vpH;
  if (d.kind == DRAG_MOVE) {
    const rawX = x - d.dx;
    const rawY = y - d.dy;
    if (vw == 0 && vh == 0) {
      // Unknown viewport: no clamping.
      w.x = rawX > 0 ? rawX : 0;
      w.y = rawY > 0 ? rawY : 0;
    } else {
      const cx = clampI(rawX, 8 - w.w + 64, vw - 64);
      const cy = clampI(rawY, MENUBAR_H, vh - TITLEBAR_H);
      w.x = cx > 0 ? cx : 0;
      w.y = cy > 0 ? cy : 0;
    }
  } else {
    const nw = d.startW + (x - d.startX) > MIN_W ? d.startW + (x - d.startX) : MIN_W;
    const nh = d.startH + (y - d.startY) > MIN_H ? d.startH + (y - d.startY) : MIN_H;
    if (vw == 0 || vh == 0) {
      w.w = nw;
      w.h = nh;
    } else {
      // The window cannot grow past the viewport (minus a margin), floored
      // at the minimum: w.w = min(nw, max(vw - w.x - 8, MIN_W)).
      const innerW = ssub(ssub(vw, w.x), 8);
      const capW = innerW > MIN_W ? innerW : MIN_W;
      w.w = nw < capW ? nw : capW;
      const innerH = ssub(ssub(vh, w.y), 8);
      const capH = innerH > MIN_H ? innerH : MIN_H;
      w.h = nh < capH ? nh : capH;
    }
  }
}

/// The pointer was released: end the drag or resize.
export function onpointerup(_x: i32, _y: i32): void {
  drag = null;
}

/// A key: Escape closes the frontmost window (the highest z), tab raises it,
/// shift+tab lowers it below the rest (a keyboard host's windowing). The key
/// name is a string in the module's memory (the host allocates it), so the
/// buffer is freed (../abi/arena).
export function onkey(ptr: i32, len: i32): void {
  const key = readUtf8(ptr, len);
  cabi_realloc(ptr, len, 1, 0);
  if (windows.length == 0) return;
  // The frontmost window (the highest z) and the lowest z.
  let topIdx = 0;
  let topZ = windows[0].z;
  let minZ = windows[0].z;
  for (let i = 1; i < windows.length; i++) {
    if (windows[i].z > topZ) {
      topZ = windows[i].z;
      topIdx = i;
    }
    if (windows[i].z < minZ) minZ = windows[i].z;
  }
  const h = windows[topIdx].handle;
  if (key == 'Escape') {
    closeWindow(h);
  } else if (key == 'Tab') {
    windows[topIdx].z = nextZ;
    nextZ += 1;
  } else if (key == 'Shift+Tab') {
    windows[topIdx].z = minZ > 0 ? minZ - 1 : 0;
  }
}

// The desktop has no editable input and no clock of its own.
export function ontext(_id: i32, ptr: i32, len: i32): void {
  cabi_realloc(ptr, len, 1, 0);
}

export function ontick(): void {}

/// A host-side close (a host-drawn chrome): drop the window. The just-an-app
/// desktop closes via its own marker, so this is a no-op for it, but a
/// host-chrome component would use it.
export function onwindowclose(window: i32): void {
  closeWindow(window);
}
