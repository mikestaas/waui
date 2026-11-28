//! The desktop: a just-an-app window manager.
//!
//! The whole interface is this component's tree: the menu bar, the icon grid,
//! and the set of open windows (their chrome — title bar, close, grip, panel,
//! border, shadow — and their geometry). It draws the windows itself, with the
//! contract's geometry tokens, and drives them through the pointer and key
//! channels: a title-bar drag moves a window, a grip drag resizes it, a press
//! raises it, the close marker (and Escape) closes it, and a double-clicked
//! tile opens its entry.
//!
//! The *content* of an opened window is not this component's: it is a separate
//! component the host runs in its own sandbox. On a double-click this
//! component calls `open-entry` (the host instantiates the entry's component
//! and returns a handle) and draws a window whose content slot is that handle:
//! the host seats the opened component's tree in the slot. Closing a window
//! calls `close-entry`, so the host drops the sandbox. This component only ever
//! sees its own tree; the opened component's tree is the host's to render.
//!
//! The tile's presentation comes from the entry's `data` blob (a JSON object,
//! host-defined): the label, the caption, the icon, and the window's default
//! size. A field the blob lacks is rendered without, and a blob that will not
//! parse renders the name only.

wit_bindgen::generate!({
    world: "app",
    path: "../../wit",
});

use component_ui::{
    Align, Background, Border, Color, Cursor, Overflow, Radius, Shadow, Size, Stretch, Style, Tree,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use waui::ui::widget as wit;

// Project the ergonomic tree onto this crate's generated types.
component_ui::to_node!(wit);

// --------------------------------------------------------------------------
// Layout constants, in the host's units (pixels on the web host).

/// The menu bar's height.
const MENUBAR_H: u32 = 34;
/// The title bar's height.
const TITLEBAR_H: u32 = 36;
/// A window's minimum size.
const MIN_W: u32 = 220;
const MIN_H: u32 = 260;
/// A window's default size, for a dimension the entry's blob does not set.
const DEFAULT_W: u32 = 300;
const DEFAULT_H: u32 = 380;
/// The new-window cascade step.
const CASCADE_STEP: u32 = 28;
/// The first z to assign; a raised window takes the next.
const Z_BASE: u32 = 10;
/// The first tile's id; tile `i` is `TILE_BASE + i`.
const TILE_BASE: u32 = 100;
/// Tiles per row of the grid.
const COLS: usize = 3;

// The window node ids, derived from the host handle. The content slot's id is
// the handle itself (the host seats the opened component's tree there); the
// chrome's ids are offset by a wide stride so they never collide across
// handles (the host assigns handles above the fixed ids' range).
const OFF_WINDOW: u32 = 100_000;
const OFF_TITLEBAR: u32 = 200_000;
const OFF_SPACER: u32 = 300_000;
const OFF_CLOSE: u32 = 400_000;
const OFF_GRIP: u32 = 500_000;

// --------------------------------------------------------------------------
// State.

/// An entry's parsed `data`: the tile's presentation and the window's default
/// size. A field the blob lacks keeps its fallback.
#[derive(Clone)]
struct EntryMeta {
    /// The entry's name: the open's identifier and the label's fallback.
    name: String,
    /// The tile's display label.
    label: String,
    /// The tile's caption.
    lang: Option<String>,
    /// The tile's and title bar's icon (the host resolves it to pixels).
    icon: Option<String>,
    /// The window's default size; 0 leaves it to the host's default.
    width: u32,
    height: u32,
}

/// The registry, fetched once from `list-entries`.
static ENTRIES: Mutex<Option<Vec<EntryMeta>>> = Mutex::new(None);

/// An open window: the host handle (also the content slot's id), the entry's
/// presentation, and the window's geometry and stacking order.
#[derive(Clone)]
struct Window {
    handle: u32,
    label: String,
    icon: Option<String>,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    z: u32,
}

/// The open windows, in the order they were opened.
static WINDOWS: Mutex<Vec<Window>> = Mutex::new(Vec::new());
/// The next z to assign.
static NEXT_Z: AtomicU32 = AtomicU32::new(Z_BASE);
/// The new-window placement counter (wraps after 9).
static CASCADE: AtomicU32 = AtomicU32::new(0);
/// The selected tile's index (its ring); `None` if none.
static SELECTED: Mutex<Option<usize>> = Mutex::new(None);

/// The window being dragged or resized, and the grab offset.
#[derive(Clone, Copy)]
enum Drag {
    /// A title-bar drag: move the window, keeping the grab offset.
    Move { handle: u32, dx: i32, dy: i32 },
    /// A grip resize: grow the window from its origin.
    Resize {
        handle: u32,
        start_x: i32,
        start_y: i32,
        start_w: u32,
        start_h: u32,
    },
}
/// The in-progress drag or resize; `None` when the pointer is idle.
static DRAG: Mutex<Option<Drag>> = Mutex::new(None);

// --------------------------------------------------------------------------
// The registry and its `data` blobs.

/// The current tree: the menu bar, the grid, and the open windows.
fn current_tree() -> Tree {
    {
        let mut entries = ENTRIES.lock().unwrap();
        if entries.is_none() {
            *entries = Some(fetch_entries());
        }
    }
    let entries = ENTRIES.lock().unwrap().clone().unwrap_or_default();
    let windows = WINDOWS.lock().unwrap().clone();
    tree(&entries, &windows)
}

/// The registry, fetched once: each entry's name plus its parsed `data`.
fn fetch_entries() -> Vec<EntryMeta> {
    list_entries()
        .into_iter()
        .map(|e| meta(&e.name, &e.data))
        .collect()
}

/// Parse an entry's `data` blob into its metadata. The blob is a JSON object,
/// host-defined; the fields this host supplies are `label` (the display
/// label), `lang` (the caption), `icon` (the icon), and `width`/`height` (the
/// window's default size). A field the blob lacks keeps its fallback; a blob
/// that will not parse yields all fallbacks.
fn meta(name: &str, data: &str) -> EntryMeta {
    let mut m = EntryMeta {
        name: name.to_string(),
        label: name.to_string(),
        lang: None,
        icon: None,
        width: 0,
        height: 0,
    };
    for (k, v) in parse_object(data) {
        match (k.as_str(), v) {
            ("label", Value::Str(s)) => m.label = s,
            ("lang", Value::Str(s)) => m.lang = Some(s),
            ("icon", Value::Str(s)) => m.icon = Some(s),
            ("width", Value::Num(n)) => m.width = n as u32,
            ("height", Value::Num(n)) => m.height = n as u32,
            _ => {}
        }
    }
    m
}

/// A value in the flat `data` object: a string or a number.
enum Value {
    Str(String),
    Num(f64),
}

/// A minimal JSON parser for the flat object the `data` blob is: an object of
/// string/number values. A real component would use a full parser; this one
/// reads the shape this host's registry emits, and anything else (nesting,
/// arrays, a malformed blob) yields an empty object, so the caller's fallbacks
/// apply.
fn parse_object(data: &str) -> Vec<(String, Value)> {
    let mut p = Parser::new(data);
    let mut out = Vec::new();
    p.skip();
    if !p.eat(b'{') {
        return out;
    }
    loop {
        p.skip();
        if p.eat(b'}') {
            return out;
        }
        let key = match p.string() {
            Some(k) => k,
            None => return out,
        };
        p.skip();
        if !p.eat(b':') {
            return out;
        }
        p.skip();
        let value = if p.peek() == Some(b'"') {
            match p.string() {
                Some(s) => Value::Str(s),
                None => return out,
            }
        } else {
            match p.number() {
                Some(n) => Value::Num(n),
                None => return out,
            }
        };
        out.push((key, value));
        p.skip();
        if p.eat(b',') {
            continue;
        }
        if !p.eat(b'}') {
            return out;
        }
        return out;
    }
}

/// The cursor of [`parse_object`].
struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Parser {
            b: s.as_bytes(),
            i: 0,
        }
    }

    fn skip(&mut self) {
        while matches!(
            self.b.get(self.i),
            Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
        ) {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        let got = self.peek() == Some(c);
        if got {
            self.i += 1;
        }
        got
    }

    /// A JSON string, escapes resolved; multi-byte UTF-8 passes through.
    fn string(&mut self) -> Option<String> {
        if !self.eat(b'"') {
            return None;
        }
        let mut bytes = Vec::new();
        loop {
            let c = *self.b.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(bytes).ok(),
                b'\\' => {
                    let e = *self.b.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'"' => bytes.push(b'"'),
                        b'\\' => bytes.push(b'\\'),
                        b'/' => bytes.push(b'/'),
                        b'b' => bytes.push(0x08),
                        b'f' => bytes.push(0x0c),
                        b'n' => bytes.push(b'\n'),
                        b'r' => bytes.push(b'\r'),
                        b't' => bytes.push(b'\t'),
                        b'u' => {
                            let hex = self.b.get(self.i..self.i + 4)?;
                            self.i += 4;
                            let c = u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                            let mut buf = [0u8; 4];
                            bytes.extend_from_slice(
                                char::from_u32(c)?.encode_utf8(&mut buf).as_bytes(),
                            );
                        }
                        _ => return None,
                    }
                }
                _ => bytes.push(c),
            }
        }
    }

    /// A JSON number, as an f64.
    fn number(&mut self) -> Option<f64> {
        let start = self.i;
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
        ) {
            self.i += 1;
        }
        if self.i == start {
            return None;
        }
        std::str::from_utf8(&self.b[start..self.i])
            .ok()?
            .parse()
            .ok()
    }
}

// --------------------------------------------------------------------------
// The tree.

/// The whole UI, nested: the menu bar, the grid, and the open windows.
fn tree(entries: &[EntryMeta], windows: &[Window]) -> Tree {
    let mut children = vec![menubar(windows.len()), grid_area(entries)];
    for w in windows {
        children.push(window(w));
    }
    Tree::column(children)
        .id(1)
        .align(Align::Start)
        .stretch(Stretch::Both)
        .background(Background::None)
}

/// The menu bar: the name, a spacer, the window count, a hint, and the repo
/// link. The id-less nodes never diff, so only the count (id 4) does.
fn menubar(n_open: usize) -> Tree {
    let (count, plural) = (n_open.to_string(), if n_open == 1 { "" } else { "s" });
    Tree::row(vec![
        Tree::text("waui", Style::new(Color::Accent, Size::Large)),
        Tree::text("wasm ui demos", Style::new(Color::Dim, Size::Small)),
        Tree::row(Vec::new()).id(3).stretch(Stretch::Main),
        Tree::text(
            format!("{count} window{plural}"),
            Style::new(Color::Dim, Size::Small),
        )
        .id(4),
        Tree::text(
            "double-click an icon to open",
            Style::new(Color::Dim, Size::Small),
        ),
        Tree::link(
            "GitHub",
            "https://github.com/mikestaas/waui",
            Style::new(Color::Accent, Size::Small),
        ),
    ])
    .id(2)
    .align(Align::Center)
    .stretch(Stretch::Main)
    .height(MENUBAR_H)
    .z(1000)
    .background(Background::Bar)
    .border(Border::Line)
    .gap(12)
    .padding(16)
}

/// The grid's area: it fills the space below the menu bar and centers the grid.
fn grid_area(entries: &[EntryMeta]) -> Tree {
    let rows: Vec<Tree> = entries
        .chunks(COLS)
        .enumerate()
        .map(|(r, chunk)| {
            let tiles = chunk
                .iter()
                .enumerate()
                .map(|(c, e)| tile(e, r * COLS + c))
                .collect();
            Tree::row(tiles)
                .id(8 + r as u32)
                .align(Align::Center)
                .gap(44)
        })
        .collect();
    // A `cross` stretch, not `main`: with `main` the column fills the area's
    // height, and the contract has no main-axis placement token to center its
    // width, so the grid packs top-left, under the first window.
    let grid = Tree::column(rows)
        .id(7)
        .align(Align::Center)
        .stretch(Stretch::Cross)
        .gap(28);
    Tree::row(vec![grid])
        .id(6)
        .align(Align::Center)
        .stretch(Stretch::Both)
        .overflow(Overflow::Clip)
}

/// One tile: an icon, a label, and a caption, in a double-clickable column.
/// The selected tile takes the ring.
fn tile(e: &EntryMeta, i: usize) -> Tree {
    let mut children = Vec::new();
    if let Some(icon) = &e.icon {
        children.push(Tree::image(icon.clone()).width(112).height(112));
    }
    children.push(Tree::text(
        e.label.clone(),
        Style::new(Color::Neutral, Size::Medium),
    ));
    if let Some(lang) = &e.lang {
        children.push(Tree::text(
            lang.clone(),
            Style::new(Color::Neutral, Size::Small),
        ));
    }
    let selected = *SELECTED.lock().unwrap() == Some(i);
    let mut t = Tree::column(children)
        .id(TILE_BASE + i as u32)
        .align(Align::Center)
        .gap(7)
        .padding(10)
        .radius(Radius::Large)
        .cursor(Cursor::Pointer);
    if selected {
        t = t.border(Border::Ring);
    }
    t
}

/// One window: the title bar, the content slot, and the grip.
fn window(w: &Window) -> Tree {
    let base = w.handle;
    let mut bar = Vec::new();
    // Left padding for the icon: the contract's padding is uniform and
    // container-only, so the leaf image takes an id-less, childless spacer.
    bar.push(Tree::row(Vec::new()).width(8));
    if let Some(icon) = &w.icon {
        bar.push(Tree::image(icon.clone()).width(18).height(18));
    }
    bar.push(Tree::text(
        w.label.clone(),
        Style::new(Color::Neutral, Size::Medium),
    ));
    bar.push(
        Tree::row(Vec::new())
            .id(base + OFF_SPACER)
            .stretch(Stretch::Main),
    );
    // The close marker as a text, not a button: a button node would draw the
    // host's button chrome (fill, border, padding). The container carries the
    // id (the click target) and a size (the hit area); with no chrome tokens
    // it draws nothing but the glyph. The pointer cursor is the affordance.
    // The hosts read that same token as "double-clickable"; a close is a
    // single click and on_double_click ignores close ids, so the coupling is
    // inert here.
    bar.push(
        Tree::row(vec![Tree::text(
            "\u{2715}",
            Style::new(Color::Neutral, Size::Medium),
        )])
        .id(base + OFF_CLOSE)
        .align(Align::Center)
        .width(48)
        .height(32)
        .cursor(Cursor::Pointer),
    );
    let titlebar = Tree::row(bar)
        .id(base + OFF_TITLEBAR)
        .align(Align::Center)
        .stretch(Stretch::Main)
        .height(TITLEBAR_H)
        .background(Background::Bar)
        .border(Border::Line)
        .cursor(Cursor::Grab);

    // The content slot: its id is the handle (the host seats the opened
    // component's tree there). It is empty in this tree.
    let content = Tree::row(Vec::new())
        .id(base)
        .align(Align::Center)
        .stretch(Stretch::Both);

    // The grip, at the window's bottom-right corner.
    let grip = Tree::row(Vec::new())
        .id(base + OFF_GRIP)
        .at(w.w.saturating_sub(17), w.h.saturating_sub(17))
        .width(14)
        .height(14)
        .cursor(Cursor::Resize);

    Tree::column(vec![titlebar, content, grip])
        .id(base + OFF_WINDOW)
        .at(w.x, w.y)
        .width(w.w)
        .height(w.h)
        .z(w.z)
        .align(Align::Start)
        .background(Background::Panel)
        .border(Border::Line)
        .radius(Radius::Large)
        .shadow(Shadow::Raised)
        .overflow(Overflow::Clip)
}

// --------------------------------------------------------------------------
// Window management.

/// The window (handle) and the kind of node with `id`, if it is a window node.
fn window_for_id(windows: &[Window], id: u32) -> Option<(u32, NodeKind)> {
    for w in windows {
        let h = w.handle;
        if id == h {
            return Some((h, NodeKind::Content));
        }
        if id == h + OFF_WINDOW {
            return Some((h, NodeKind::Window));
        }
        if id == h + OFF_TITLEBAR || id == h + OFF_SPACER {
            return Some((h, NodeKind::Titlebar));
        }
        if id == h + OFF_CLOSE {
            return Some((h, NodeKind::Close));
        }
        if id == h + OFF_GRIP {
            return Some((h, NodeKind::Grip));
        }
    }
    None
}

/// Which part of a window a node is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NodeKind {
    Window,
    Titlebar,
    Close,
    Content,
    Grip,
}

/// Open the entry with `index`: request a window from the host (a cascade
/// placement, the entry's default size), and record it. A window the host
/// cannot open is dropped.
fn open_entry_at(index: usize) {
    let entry = ENTRIES
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|e| e.get(index))
        .cloned();
    let Some(entry) = entry else { return };
    let w = if entry.width > 0 {
        entry.width
    } else {
        DEFAULT_W
    };
    let h = if entry.height > 0 {
        entry.height
    } else {
        DEFAULT_H
    };
    let (x, y) = cascade_placement(w, h);
    match open_entry(&entry.name, x, y, w, h) {
        Ok(handle) => {
            let z = NEXT_Z.fetch_add(1, Ordering::SeqCst);
            WINDOWS.lock().unwrap().push(Window {
                handle,
                label: entry.label.clone(),
                icon: entry.icon.clone(),
                x,
                y,
                w,
                h,
                z,
            });
        }
        Err(_) => {}
    }
}

/// A new window's placement: cascade down-right, wrapping after 9, clamped to
/// the viewport. Skipped when the viewport is unknown (`(0, 0)`).
fn cascade_placement(w: u32, h: u32) -> (u32, u32) {
    let n = CASCADE.fetch_add(1, Ordering::SeqCst) % 9;
    let vp = viewport();
    let (vw, vh) = (vp.w, vp.h);
    if vw == 0 && vh == 0 {
        return (24 + n * CASCADE_STEP, 48 + n * CASCADE_STEP);
    }
    let left = clamp_u(
        24 + n * CASCADE_STEP,
        8,
        vw.saturating_sub(w).saturating_sub(8),
    );
    let top = clamp_u(
        48 + n * CASCADE_STEP,
        MENUBAR_H + 4,
        vh.saturating_sub(h).saturating_sub(8),
    );
    (left, top)
}

/// Close the window with `handle`: drop it and tell the host to free the
/// sandbox.
fn close_window(handle: u32) {
    WINDOWS.lock().unwrap().retain(|w| w.handle != handle);
    close_entry(handle);
}

/// Clamp `v` to `[lo, hi]`, tolerating a degenerate range (a window larger than
/// the viewport) by returning the lower bound.
fn clamp_i(v: i32, lo: i32, hi: i32) -> i32 {
    if lo > hi {
        return lo;
    }
    v.clamp(lo, hi)
}

fn clamp_u(v: u32, lo: u32, hi: u32) -> u32 {
    clamp_i(v as i32, lo as i32, hi as i32) as u32
}

// --------------------------------------------------------------------------
// The component.

struct Desktop;

impl Guest for Desktop {
    fn view() -> Vec<Node> {
        let t = current_tree();
        component_ui::note_view(&t);
        component_ui::lower(&t, to_node)
    }

    fn changes() -> Vec<Mutation> {
        component_ui::diff(&current_tree(), |c| match c {
            component_ui::DiffNode::Changed(node, parent, index) => {
                Mutation::Changed(wit::Changed {
                    node: to_node(node, parent),
                    index,
                })
            }
            component_ui::DiffNode::Removed(id) => Mutation::Removed(id),
        })
    }

    // A single click: the close marker closes its window; a tile selects it
    // (its ring).
    fn on_click(id: u32) {
        let handle = {
            let windows = WINDOWS.lock().unwrap();
            window_for_id(&windows, id)
                .filter(|(_, kind)| *kind == NodeKind::Close)
                .map(|(h, _)| h)
        };
        if let Some(handle) = handle {
            close_window(handle);
            return;
        }
        let i = (id - TILE_BASE) as usize;
        let entries = ENTRIES.lock().unwrap();
        let is_tile = entries.as_ref().is_some_and(|e| i < e.len());
        drop(entries);
        if is_tile {
            *SELECTED.lock().unwrap() = Some(i);
        }
    }

    // A double-clicked tile opens its entry.
    fn on_double_click(id: u32) {
        let i = (id - TILE_BASE) as usize;
        let entries = ENTRIES.lock().unwrap();
        let is_tile = entries.as_ref().is_some_and(|e| i < e.len());
        drop(entries);
        if is_tile {
            *SELECTED.lock().unwrap() = Some(i);
            open_entry_at(i);
        }
    }

    // The pointer was pressed over a window node: raise it, and start a drag
    // (title bar) or resize (grip).
    fn on_pointer_down(id: u32, x: u32, y: u32) {
        let mut windows = WINDOWS.lock().unwrap();
        let Some((handle, kind)) = window_for_id(&windows, id) else {
            return;
        };
        let Some(w) = windows.iter_mut().find(|w| w.handle == handle) else {
            return;
        };
        w.z = NEXT_Z.fetch_add(1, Ordering::SeqCst);
        *DRAG.lock().unwrap() = match kind {
            NodeKind::Titlebar => Some(Drag::Move {
                handle,
                dx: x as i32 - w.x as i32,
                dy: y as i32 - w.y as i32,
            }),
            NodeKind::Grip => Some(Drag::Resize {
                handle,
                start_x: x as i32,
                start_y: y as i32,
                start_w: w.w,
                start_h: w.h,
            }),
            // A press on the window, its content, or the close marker only
            // raises (the close is a click, not a drag).
            _ => None,
        };
    }

    // The pointer moved while a button is held: continue the drag or resize.
    fn on_pointer_move(x: u32, y: u32) {
        let drag = DRAG.lock().unwrap().clone();
        let Some(drag) = drag else { return };
        let handle = match &drag {
            Drag::Move { handle, .. } => *handle,
            Drag::Resize { handle, .. } => *handle,
        };
        let (vw, vh) = {
            let vp = viewport();
            (vp.w, vp.h)
        };
        let mut windows = WINDOWS.lock().unwrap();
        let Some(w) = windows.iter_mut().find(|w| w.handle == handle) else {
            return;
        };
        match drag {
            Drag::Move { dx, dy, .. } => {
                let raw_x = x as i32 - dx;
                let raw_y = y as i32 - dy;
                if vw == 0 && vh == 0 {
                    // Unknown viewport: no clamping.
                    w.x = raw_x.max(0) as u32;
                    w.y = raw_y.max(0) as u32;
                } else {
                    w.x = clamp_i(raw_x, 8 - w.w as i32 + 64, vw as i32 - 64).max(0) as u32;
                    w.y = clamp_i(raw_y, MENUBAR_H as i32, vh as i32 - TITLEBAR_H as i32).max(0)
                        as u32;
                }
            }
            Drag::Resize {
                start_x,
                start_y,
                start_w,
                start_h,
                ..
            } => {
                let nw = (start_w as i32 + (x as i32 - start_x)).max(MIN_W as i32) as u32;
                let nh = (start_h as i32 + (y as i32 - start_y)).max(MIN_H as i32) as u32;
                if vw == 0 || vh == 0 {
                    w.w = nw;
                    w.h = nh;
                } else {
                    w.w = nw.min(vw.saturating_sub(w.x).saturating_sub(8).max(MIN_W));
                    w.h = nh.min(vh.saturating_sub(w.y).saturating_sub(8).max(MIN_H));
                }
            }
        }
    }

    // The pointer was released: end the drag or resize.
    fn on_pointer_up(_x: u32, _y: u32) {
        *DRAG.lock().unwrap() = None;
    }

    // A key: Escape closes the frontmost window (the highest z), tab raises
    // it, shift+tab lowers it below the rest (a keyboard host's windowing).
    fn on_key(key: String) {
        let handle = {
            let mut windows = WINDOWS.lock().unwrap();
            let min_z = windows.iter().map(|w| w.z).min().unwrap_or(Z_BASE);
            let Some(top) = windows.iter_mut().max_by_key(|w| w.z) else {
                return;
            };
            let h = top.handle;
            match key.as_str() {
                "Escape" => Some(h),
                "Tab" => {
                    top.z = NEXT_Z.fetch_add(1, Ordering::SeqCst);
                    None
                }
                "Shift+Tab" => {
                    top.z = min_z.saturating_sub(1);
                    None
                }
                _ => None,
            }
        };
        if let Some(handle) = handle {
            close_window(handle);
        }
    }

    // The desktop has no editable input and no clock of its own.
    fn on_text(_id: u32, _text: String) {}
    fn on_tick() {}

    // A host-side close (a host-drawn chrome): drop the window. The just-an-app
    // desktop closes via its own marker, so this is a no-op for it, but a
    // host-chrome component would use it.
    fn on_window_close(window: u32) {
        WINDOWS.lock().unwrap().retain(|w| w.handle != window);
    }
}

export!(Desktop);
