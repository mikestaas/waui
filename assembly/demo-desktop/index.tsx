// The desktop, in AssemblyScript — the same component as `crates/demo-desktop`,
// same `waui:ui/app` world. See `../README.md` for the AS tree layer, the
// build pipeline, and the memory layout.
//
// The multi-component host's component: it owns the *logical* state — the
// grid (built from the host's `list-entries` registry), the menu bar's window
// count, and which windows are open — and exports it via `view`/`changes`, as
// any component does. It *requests* a window through the `open-entry` import
// when a tile is double-clicked, and drops a window from its open set when the
// host reports a close. The windows themselves (their chrome, geometry, and
// the embedded demo running in each) are host-drawn, not part of this tree.
//
//   view:     a centered column: the menu bar (id 1: "waui" accent-large, and
//             the window count id 2, neutral-small) and the grid (id 3: a
//             column of rows of three tiles). Each tile (id 100+i) is a column
//             of an icon (112px, id-less), a label (neutral-medium, id-less),
//             and a caption (neutral-small, id-less).
//   on-double-click: a tile opens its entry (the host places it; the size is
//             the entry's default)
//   on-window-close: a closed window is dropped from the open set
//   on-click / on-text / on-tick: nothing (no input, no clock; a single
//             click on a tile does nothing)
//   changes:  the diff (the window count; the whole tree on first contact)
//
// **The registry.** The entries are fetched once from the host's
// `list-entries` (a wasi:filesystem `readdir`), on the first tree build. Each
// entry's `data` is a JSON object of host-defined metadata: the tile's label,
// caption, icon, and the window's default size. A field the blob lacks is
// rendered without, and a blob that will not parse renders the name only.
// The host writes the registry into the module's scratch region (through
// `cabi_realloc`), so the component copies it into retained strings and frees
// the scratch.
//
// **The ids.** The menu bar is 1 (its count 2), the grid 3, the rows 10+r,
// and the tiles 100+i. A tile's icon, label, and caption are id-less: the
// host makes id'd nodes double-clickable, and the id-less leaves are
// display-only.
//
// It never calls `schedule` (it changes only on a double-click or a close).

import { arenaAlloc, cabi_post_changes, cabi_post_view, cabi_realloc } from '../abi/arena';
import { COLOR_ACCENT, COLOR_NEUTRAL, SIZE_LARGE, SIZE_MEDIUM, SIZE_SMALL } from '../abi/node';
import { readUtf8, writeUtf8 } from '../abi/utf8';
import { listEntries, openEntry } from '../abi/windowing';
import { column, diffOf, h, image, row, text, viewOf, Widget } from '../ui/tree';

// The canonical-ABI post functions and the string allocator (../abi/arena).
export { cabi_post_changes, cabi_post_view, cabi_realloc };

// Widget ids (the contract's identity convention).
const MENU = 1;
const COUNT = 2;
const GRID = 3;
const ROW_BASE = 10; // row r is ROW_BASE + r
const TILE_BASE = 100; // tile i is TILE_BASE + i
const COLS = 3; // tiles per row of the grid
const TILE_PX = 112; // the web desktop's tile size (see site/desktop.css)

/// An entry's parsed `data`: the tile's presentation and the window's default
/// size. The fields this host's registry supplies; a field the blob lacks
/// keeps its fallback (an empty string = absent).
class EntryMeta {
  /// The entry's name: the open's identifier and the label's fallback.
  name: string = '';
  /// The tile's display label.
  label: string = '';
  /// The tile's caption (empty = absent).
  lang: string = '';
  /// The tile's icon (empty = absent; the host resolves it to pixels).
  icon: string = '';
  /// The window's default size; 0 leaves it to the host's default.
  width: i32 = 0;
  height: i32 = 0;
}

// The entries the host can run, fetched once from `list-entries`; and the
// open windows' handles, as the host assigns them. Module globals, so they
// survive between exports.
var entries: EntryMeta[] = new Array<EntryMeta>();
var open: i32[] = new Array<i32>();
var inited: i32 = 0;

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
          // \uXXXX: a code point (the registry does not use it, but a
          // surrogate pair in a label would).
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

/// Parse an entry's `data` blob into its metadata. The blob is a JSON object,
/// host-defined; the fields this host supplies are `label` (the tile's display
/// label), `lang` (its caption), `icon` (its icon), and `width`/`height` (the
/// window's default size). A field the blob lacks keeps its fallback; a blob
/// that will not parse keeps the fallbacks parsed so far.
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

// ---------------------------------------------------------------- the tree

/// The desktop's whole UI, nested. The structure is rebuilt on every export;
/// the entries (and the open set) are the retained state.
function tree(entries: EntryMeta[], nOpen: i32): Widget {
  const plural = nOpen == 1 ? '' : 's';
  const menu = (
    <row id={MENU}>
      <text label="waui" color={COLOR_ACCENT} size={SIZE_LARGE} />
      <text id={COUNT} label={`${nOpen} window${plural}`} color={COLOR_NEUTRAL} size={SIZE_SMALL} />
    </row>
  );

  // The grid: a column of rows of `COLS` tiles each, in registry order.
  const rows: Widget[] = new Array<Widget>();
  const nRows = (entries.length + COLS - 1) / COLS;
  for (let r = 0; r < nRows; r++) {
    const tiles: Widget[] = new Array<Widget>();
    for (let c = 0; c < COLS; c++) {
      const i = r * COLS + c;
      if (i < entries.length) tiles.push(tile(entries[i], i));
    }
    rows.push(<row id={ROW_BASE + r}>{tiles}</row>);
  }
  const grid = <column id={GRID}>{rows}</column>;

  return <column>{[menu, grid]}</column>;
}

/// One tile: an icon, a label, and a caption, in a double-clickable column.
/// The icon is `TILE_PX` square, the web desktop's tile size. A field the
/// entry's blob lacks is rendered without.
function tile(e: EntryMeta, i: i32): Widget {
  const kids: Widget[] = new Array<Widget>();
  if (e.icon.length != 0) kids.push(image(e.icon, TILE_PX, TILE_PX));
  kids.push(text(e.label, COLOR_NEUTRAL, SIZE_MEDIUM));
  if (e.lang.length != 0) kids.push(text(e.lang, COLOR_NEUTRAL, SIZE_SMALL));
  return <column id={TILE_BASE + i}>{kids}</column>;
}

/// The current tree: the registry is fetched on the first build, and the grid
/// and the count are derived from it and the open set.
function currentTree(): Widget {
  if (inited == 0) {
    inited = 1;
    entries = fetchEntries();
  }
  return tree(entries, open.length);
}

// ---------------------------------------------------------------- the world

export function view(): i32 {
  return viewOf(currentTree());
}

export function changes(): i32 {
  return diffOf(currentTree());
}

/// A tile was double-clicked: open the entry it represents. The placement is
/// left to the host (0, 0 = its default cascade); the size is the entry's
/// default. A window the host cannot open is dropped.
export function ondoubleclick(id: i32): void {
  const i = id - TILE_BASE;
  if (i < 0 || i >= entries.length) return; // not a tile (or out of range)
  const entry = entries[i];
  // The name buffer is in the module's own memory (the host reads it); it is
  // arena-allocated and dies with the next post.
  const cap = entry.name.length * 4; // a UTF-16 char is at most 4 UTF-8 bytes
  const nameBuf = arenaAlloc(cap);
  const len = writeUtf8(nameBuf, entry.name, nameBuf + cap) - nameBuf;
  const retPtr = arenaAlloc(8);
  openEntry(nameBuf, len, 0, 0, entry.width, entry.height, retPtr);
  if (load<i8>(retPtr + 0) != 0) return; // an error: the window was not opened
  open.push(load<i32>(retPtr + 4)); // the host's window handle
}

/// A window was closed by the user: drop it from the open set.
export function onwindowclose(window: i32): void {
  for (let i = 0; i < open.length; i++) {
    if (open[i] == window) {
      open.splice(i, 1);
      break;
    }
  }
}

// The plain-app events (see the `app` world): the desktop has no editable
// input and no clock, and a single click on a tile does nothing (the
// double-click opens), so all three are ignored. `on-text` frees the buffer
// the host allocated (../abi/arena).
export function onclick(_id: i32): void {}

export function ontext(_id: i32, ptr: i32, len: i32): void {
  cabi_realloc(ptr, len, 1, 0);
}

export function ontick(): void {}
