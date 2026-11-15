//! The desktop: the icon grid, the menu bar, and the set of open windows.
//!
//! The multi-component host's component, on the `app` world. It owns the
//! *logical* state — the grid (built from the host's `list-entries`
//! registry), the menu bar's window count, and which windows are open — and
//! exports it via `view`/`changes`, as any component does. It *requests* a
//! window through the `open-entry` import when a tile is double-clicked, and
//! drops a window from its open set when the host reports a close. The
//! windows themselves (their chrome, geometry, and the embedded demo running
//! in each) are host-drawn, not part of this tree.
//!
//! The tile is a container (an icon, a label, and a caption) with an id: the
//! host makes id'd nodes double-clickable on the desktop, and the component
//! opens the entry a tile represents in response. The tile's presentation
//! comes from the entry's `data` blob (a JSON object, host-defined): the
//! label, the caption, the icon, and the window's default size. A field the
//! blob lacks is rendered without, and a blob that will not parse renders
//! the name only.

wit_bindgen::generate!({
    world: "app",
    path: "../../wit",
});

use component_ui::{Color, Size, Style, Tree};
use std::sync::Mutex;
use waui::ui::widget as wit;

// Project the ergonomic tree onto this crate's generated types.
component_ui::to_node!(wit);

/// The first tile's id; tile `i` is `TILE_BASE + i`.
const TILE_BASE: u32 = 100;
/// Tiles per row of the grid.
const COLS: usize = 3;

/// An entry's parsed `data`: the tile's presentation and the window's
/// default size. The fields this host's registry supplies; a field the blob
/// lacks keeps its fallback.
#[derive(Clone)]
struct EntryMeta {
    /// The entry's name: the open's identifier and the label's fallback.
    name: String,
    /// The tile's display label.
    label: String,
    /// The tile's caption.
    lang: Option<String>,
    /// The tile's icon (the host resolves it to pixels, as an image's src).
    icon: Option<String>,
    /// The window's default size; 0 leaves it to the host's default.
    width: u32,
    height: u32,
}

/// The entries the host can run, fetched once from the `list-entries`
/// registry: each entry's name plus its parsed `data`.
static ENTRIES: Mutex<Option<Vec<EntryMeta>>> = Mutex::new(None);
/// The open windows' handles, as the host assigns them.
static OPEN: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// The current tree: the menu bar (with the window count) and the grid.
fn current_tree() -> Tree {
    {
        let mut entries = ENTRIES.lock().unwrap();
        if entries.is_none() {
            *entries = Some(fetch_entries());
        }
    }
    let entries = ENTRIES.lock().unwrap().clone().unwrap_or_default();
    let n_open = OPEN.lock().unwrap().len();
    tree(&entries, n_open)
}

/// The registry, fetched once: each entry's name plus its parsed `data`.
fn fetch_entries() -> Vec<EntryMeta> {
    list_entries()
        .into_iter()
        .map(|e| meta(&e.name, &e.data))
        .collect()
}

/// Parse an entry's `data` blob into its metadata. The blob is a JSON
/// object, host-defined; the fields this host supplies are `label` (the
/// tile's display label), `lang` (its caption), `icon` (its icon), and
/// `width`/`height` (the window's default size). A field the blob lacks
/// keeps its fallback; a blob that will not parse yields all fallbacks.
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

/// A minimal JSON parser for the flat object the `data` blob is: an object
/// of string/number values. A real component would use a full parser; this
/// one reads the shape this host's registry emits, and anything else
/// (nesting, arrays, a malformed blob) yields an empty object, so the
/// caller's fallbacks apply.
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

/// The desktop's whole UI, nested.
fn tree(entries: &[EntryMeta], n_open: usize) -> Tree {
    let (count, plural) = (n_open.to_string(), if n_open == 1 { "" } else { "s" });
    let menu = Tree::row(vec![
        Tree::text("waui", Style::new(Color::Accent, Size::Large)),
        Tree::text(
            format!("{count} window{plural}"),
            Style::new(Color::Neutral, Size::Small),
        )
        .id(2),
    ])
    .id(1);

    // The grid: a column of rows of `COLS` tiles each, in registry order.
    let rows: Vec<Tree> = entries
        .chunks(COLS)
        .enumerate()
        .map(|(r, chunk)| {
            let tiles = chunk
                .iter()
                .enumerate()
                .map(|(c, e)| tile(e, r * COLS + c))
                .collect();
            Tree::row(tiles).id(10 + r as u32)
        })
        .collect();
    let grid = Tree::column(rows).id(3);

    Tree::column(vec![menu, grid])
}

/// One tile: an icon, a label, and a caption, in a double-clickable column.
/// The icon is 112px, the web desktop's tile size (see `site/desktop.css`).
/// A field the entry's blob lacks is rendered without.
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
    Tree::column(children).id(TILE_BASE + i as u32)
}

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

    /// A tile was double-clicked: open the entry it represents. The
    /// placement is left to the host (0,0 = its default cascade); the size
    /// is the entry's default. A window the host cannot open is dropped.
    fn on_double_click(id: u32) {
        let i = (id - TILE_BASE) as usize;
        let entry = ENTRIES
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|e| e.get(i))
            .cloned();
        let Some(entry) = entry else { return }; // not a tile (or out of range)
        match open_entry(&entry.name, 0, 0, entry.width, entry.height) {
            Ok(handle) => OPEN.lock().unwrap().push(handle),
            Err(_) => {}
        }
    }

    /// A window was closed by the user: drop it from the open set.
    fn on_window_close(window: u32) {
        OPEN.lock().unwrap().retain(|&h| h != window);
    }

    // The plain-app events (see the `app` world): the desktop has no
    // editable input and no clock, and a single click on a tile does nothing
    // (the double-click opens), so all three are ignored.
    fn on_click(_id: u32) {}

    fn on_text(_id: u32, _text: String) {}

    fn on_tick() {}
}

export!(Desktop);
