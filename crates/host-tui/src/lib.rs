//! The platform-neutral core of the TUI host: the wasmtime `Host` (clock,
//! refresh request, registry, and windows), the tree-to-ratatui renderer,
//! the host-local focus ring, and the `--dump` frame printer.
//!
//! The renderer and the focus ring are pure functions of the tree and the
//! host's local state, so they are shared by the native binary (a real
//! terminal, via crossterm) and the browser module (an xterm.js terminal,
//! via the `web` module). What differs per platform is only the I/O: the
//! engine (Cranelift natively, the Pulley interpreter in the browser), the
//! clock, and how a component is loaded. [`dump_frame`] is the shared
//! byte-parity surface between the two.
//!
//! A TUI host for waui components: real terminal layout (ratatui) — rows
//! and columns, framed buttons — plus a host-local focus ring. The
//! component has no focus concept; which button is highlighted is a host
//! concern.
//!
//! With a registry (`--entry name=path`, repeatable) the host is a desktop:
//! a multi-document interface. A double-clicked tile (a container with an
//! id in the tile range, as the web host's `.tile` marks it) opens its
//! entry as a window — the entry's component running in its own store,
//! drawn as an overlapping bordered box. Windows stack in z-order (newest
//! on top), cascade their placement, and are raised and lowered with tab
//! and shift+tab; esc closes the active window, and quits on the desktop.
//!
//! The loop wakes each component — the desktop and every window — at the
//! period it requests with `schedule` (0 = never; the 50ms event poll sets
//! the granularity) and rebuilds its retained tree only when the diff is
//! non-empty; otherwise it redraws.

use std::sync::{Arc, Mutex};

// The host clock: wall time natively, `performance.now()` in the browser.
// `web_time` mirrors the `std::time` API, so the one `now_ms` below is
// uniform across platforms.
#[cfg(not(target_arch = "wasm32"))]
use std::time;
#[cfg(target_arch = "wasm32")]
use web_time as time;

use wasmtime::component::*;
use wasmtime::{Engine, Store};

// The world's bindings and the node-list→tree rebuild live in the shared
// host core; this crate is only its renderer.
use host_shared::{build_tree, waui, App, AppImports, Entry, Error, Retained, Tree};

// The contract's style tokens, aliased to avoid clashing with ratatui's
// `Color` and `Style`.
use host_shared::waui::ui::widget::{
    Align as WAlign, Color as WColor, Size as WSize, Style as WStyle,
};

use ratatui::{
    backend::TestBackend,
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame, Terminal,
};

/// The component's refresh request, shared between the world-import impl
/// (the writer) and the loop (the reader): the period in ms, 0 = no
/// periodic wake, and the last wake.
#[derive(Default)]
pub struct Wake {
    pub rate_ms: u64,
    pub last_wake_ms: u64,
}

/// A registry entry: the name (`open-entry`'s key, the tile's label
/// fallback) and the component's file path. The `data` blob is always
/// `{}`: the host supplies no per-entry metadata, so a tile renders the
/// name only, as the component degrades a blob it cannot parse.
#[derive(Clone)]
pub struct EntrySpec {
    pub name: String,
    pub path: String,
}

/// One surface's focus ring: its targets (in pre-order), the ring's
/// position among them, and the focused id.
#[derive(Default, Clone)]
pub struct Surface {
    pub focus_idx: usize,
    pub targets: Vec<FocusTarget>,
    pub focus: Option<u32>,
}

/// An open window: the entry's component running in its own store (the
/// world's imports take the store's data, not a store handle, so a window
/// gets its own), its retained tree, its cascade placement, and its ring.
pub struct Window {
    pub handle: u32,
    pub title: String,
    pub x: u16,
    pub y: u16,
    pub store: Store<Host>,
    pub app: App,
    pub retained: Retained,
    pub tree: Tree,
    pub wake: Arc<Mutex<Wake>>,
    /// Whether the window's component was driven (a tick) since the last
    /// `changes` call.
    pub pending: bool,
    pub surface: Surface,
}

/// The host's implementation of the world's imports. The world's imports
/// take the store's data, not a store handle, so a window this host opens
/// carries its own store and its own copy of the windows it opens.
pub struct Host {
    pub wake: Arc<Mutex<Wake>>,
    /// A fake clock (`--now <base> <step>`): the n-th call to `now` returns
    /// `base + n * step`, so a `--dump` frame that reads the clock shows a
    /// fixed elapsed. `None` = wall time.
    pub fake_now: Option<(u64, u64)>,
    /// The fake clock's call count.
    pub fake_calls: u64,
    pub engine: Engine,
    pub linker: Linker<Host>,
    /// The registry (`--entry`): a non-empty one makes this host a desktop.
    pub entries: Vec<EntrySpec>,
    /// The open windows, in z-order (last = topmost).
    pub windows: Vec<Window>,
    /// The next window handle (the host's flat id, from 1).
    pub next_handle: u32,
}

impl Default for Host {
    fn default() -> Self {
        let engine = engine_default();
        let linker = Linker::new(&engine);
        Host {
            wake: Arc::new(Mutex::new(Wake::default())),
            fake_now: None,
            fake_calls: 0,
            engine,
            linker,
            entries: Vec::new(),
            windows: Vec::new(),
            next_handle: 1,
        }
    }
}

/// The host's engine: the platform default natively (Cranelift for the
/// host's architecture); the shared pulley32 config in the browser, where
/// Cranelift JIT is unavailable and the component runs on the Pulley
/// interpreter.
fn engine_default() -> Engine {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Engine::default()
    }
    #[cfg(target_arch = "wasm32")]
    {
        Engine::new(&host_shared::pulley_config()).expect("failed to create the pulley engine")
    }
}

impl Host {
    /// The host clock: the fake value when set, else wall time.
    pub fn clock(&mut self) -> u64 {
        match self.fake_now {
            Some((base, step)) => {
                let t = base + self.fake_calls * step;
                self.fake_calls += 1;
                t
            }
            None => now_ms(),
        }
    }

    /// The host for a window this host opens: the same engine, linker, and
    /// clock, but an empty registry (a window runs a demo, not a desktop,
    /// so the tile rule never fires in it) and its own window list.
    fn window_host(&self) -> Host {
        Host {
            wake: Arc::new(Mutex::new(Wake::default())),
            fake_now: self.fake_now,
            fake_calls: 0,
            engine: self.engine.clone(),
            linker: self.linker.clone(),
            entries: Vec::new(),
            windows: Vec::new(),
            next_handle: 1,
        }
    }
}

impl AppImports for Host {
    fn now(&mut self) -> u64 {
        self.clock()
    }
    fn schedule(&mut self, period: u64) {
        *self.wake.lock().unwrap() = Wake {
            rate_ms: period,
            last_wake_ms: self.clock(),
        };
    }
    fn list_entries(&mut self) -> Vec<Entry> {
        self.entries
            .iter()
            .map(|e| Entry {
                name: e.name.clone(),
                data: "{}".to_string(),
            })
            .collect()
    }
    /// Open a window running the entry with `name`: the entry's component
    /// in a new store, drawn as an overlapping box. The placement cascades
    /// (0,0 = the host's default); a pixel size is meaningless in a
    /// terminal, so the window is sized by its content.
    fn open_entry(
        &mut self,
        name: String,
        _x: u32,
        _y: u32,
        _w: u32,
        _h: u32,
    ) -> Result<u32, Error> {
        let Some(spec) = self.entries.iter().find(|e| e.name == name).cloned() else {
            return Err(Error::NotFound);
        };
        let component = match load_component(&self.engine, &spec.path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("open-entry: failed to load {}: {e}", spec.path);
                return Err(Error::NotFound);
            }
        };
        let mut store = Store::new(&self.engine, self.window_host());
        let app = match App::instantiate(&mut store, &component, &self.linker) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("open-entry: failed to instantiate {}: {e}", spec.name);
                return Err(Error::NotFound);
            }
        };
        let nodes = match app.call_view(&mut store) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("open-entry: view() failed in {}: {e}", spec.name);
                return Err(Error::NotFound);
            }
        };
        let retained = Retained::from_view(nodes);
        let tree = build_tree(retained.nodes());
        let wake = store.data().wake.clone();
        let handle = self.next_handle;
        self.next_handle = self.next_handle.saturating_add(1);
        let n = self.windows.len();
        self.windows.push(Window {
            handle,
            title: spec.name,
            x: cascade_x(n),
            y: cascade_y(n),
            store,
            app,
            retained,
            tree,
            wake,
            pending: false,
            surface: Surface::default(),
        });
        Ok(handle)
    }
}

/// Load the component at `path`. The native host reads it from the
/// filesystem; the browser host has no filesystem, so it cannot load a
/// registry entry's component by path (the standalone demo's registry is
/// empty, so this is unreachable there).
#[cfg(not(target_arch = "wasm32"))]
fn load_component(engine: &Engine, path: &str) -> Result<Component, wasmtime::Error> {
    Component::from_file(engine, path)
}

#[cfg(target_arch = "wasm32")]
fn load_component(_engine: &Engine, _path: &str) -> Result<Component, wasmtime::Error> {
    Err(wasmtime::Error::msg(
        "file loading is unavailable in the browser",
    ))
}

// The host clock in ms (wall time, as the world's `now` import promises).
pub fn now_ms() -> u64 {
    time::SystemTime::now()
        .duration_since(time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// The `widget` interface is pulled into the world (via `use widget.{node}`),
// so the generated bindings require this empty marker impl.
impl waui::ui::widget::Host for Host {}

// --------------------------------------------------------------------------
// Window placement and sizing: the host's cascade and box geometry.

// The cascade, in cells (the web host's 28px step, scaled to a terminal):
// a new window steps down-right from the first, wrapping after 9.
const CASCADE_X: u16 = 10;
const CASCADE_Y: u16 = 1;
const CASCADE_STEP_X: u16 = 5;
const CASCADE_STEP_Y: u16 = 2;

fn cascade_x(n: usize) -> u16 {
    CASCADE_X + (n % 9) as u16 * CASCADE_STEP_X
}

fn cascade_y(n: usize) -> u16 {
    CASCADE_Y + (n % 9) as u16 * CASCADE_STEP_Y
}

/// A window's box size: its content (the demo's tree) with one cell of
/// padding, its title row, and its border. The title row floors the width
/// (the title plus the close marker).
fn window_size(win: &Window) -> (u16, u16) {
    let cw = width_of(&win.tree, false);
    let ch = height_of(&win.tree, false);
    let title = win.title.chars().count() as u16;
    let inner_w = cw.saturating_add(2).max(title.saturating_add(3));
    (inner_w.saturating_add(2), ch.saturating_add(4))
}

// --------------------------------------------------------------------------
// Layout sizing: how much terminal space a tree needs.

/// The tile range: the web host marks a node `.tile` when its id is at
/// least this (the desktop's tile base); a container in the range is a
/// double-clickable tile.
const TILE_BASE: u32 = 100;

/// A container the desktop makes double-clickable: an id in the tile range.
/// The rule is host-local (the WIT has no double-clickable marker), and it
/// applies only on a desktop (a non-empty registry): a standalone demo's
/// id'd containers (the todo's rows) are not tiles.
fn is_tile(id: Option<u32>) -> bool {
    id.is_some_and(|i| i >= TILE_BASE)
}

pub fn width_of(node: &Tree, desktop: bool) -> u16 {
    match node {
        Tree::Text(content, _) => content.chars().count() as u16,
        Tree::Button(_, label, _) => label.chars().count() as u16 + 2, // borders
        // Framed like a button: the field's width — the component's
        // character count, or the content's when it is `0` — plus its two
        // border columns. Content past the width clips, like a field.
        Tree::Input(_, content, width, _) => {
            let inner = if *width > 0 {
                *width as u16
            } else {
                content.chars().count() as u16
            };
            inner + 2
        }
        // A framed placeholder (a terminal has no pixels): the node's width
        // in cells — a host default when it is 0 — plus its two border
        // columns.
        Tree::Image(_, _, width, _) => {
            let inner = if *width > 0 { *width as u16 } else { 8 };
            inner + 2
        }
        // A tile is a bordered box: its widest child plus the two border
        // columns.
        Tree::Row(id, _, children) if desktop && is_tile(*id) => {
            children
                .iter()
                .map(|c| width_of(c, false))
                .max()
                .unwrap_or(0)
                + 2
        }
        Tree::Column(id, _, children) if desktop && is_tile(*id) => {
            children
                .iter()
                .map(|c| width_of(c, false))
                .max()
                .unwrap_or(0)
                + 2
        }
        Tree::Row(_, _, children) => children.iter().map(|c| width_of(c, desktop)).sum(),
        Tree::Column(_, _, children) => children
            .iter()
            .map(|c| width_of(c, desktop))
            .max()
            .unwrap_or(0),
    }
}

pub fn height_of(node: &Tree, desktop: bool) -> u16 {
    match node {
        Tree::Text(..) => 1,
        Tree::Button(..) => 3, // border top, label, border bottom
        Tree::Input(..) => 3,  // border top, content, border bottom
        // The placeholder's height in cells, a host default when it is 0,
        // plus its two border rows.
        Tree::Image(_, _, _, height) => {
            let inner = if *height > 0 { *height as u16 } else { 3 };
            inner + 2
        }
        Tree::Row(id, _, children) if desktop && is_tile(*id) => {
            children
                .iter()
                .map(|c| height_of(c, false))
                .max()
                .unwrap_or(0)
                + 2
        }
        Tree::Column(id, _, children) if desktop && is_tile(*id) => {
            children
                .iter()
                .map(|c| height_of(c, false))
                .max()
                .unwrap_or(0)
                + 2
        }
        Tree::Row(_, _, children) => children
            .iter()
            .map(|c| height_of(c, desktop))
            .max()
            .unwrap_or(0),
        Tree::Column(_, _, children) => children.iter().map(|c| height_of(c, desktop)).sum(),
    }
}

// --------------------------------------------------------------------------
// Styling: the component's semantic tokens, mapped onto terminal primitives.

/// A semantic color as a terminal color.
fn tint(c: WColor) -> Color {
    match c {
        WColor::Neutral => Color::White,
        WColor::Accent => Color::Cyan,
        WColor::Positive => Color::Green,
        WColor::Negative => Color::Red,
        WColor::Warning => Color::Yellow,
    }
}

/// A node's style as a ratatui style: the semantic color, with the
/// semantic size faked as weight (a monospace terminal has no glyph scale).
fn node_style(s: &WStyle) -> Style {
    Style::default()
        .fg(tint(s.color))
        .add_modifier(match s.size {
            WSize::Small => Modifier::DIM,
            WSize::Medium => Modifier::empty(),
            WSize::Large => Modifier::BOLD,
        })
}

/// A child's cross-axis offset within the container's cross extent: the
/// token's start/center/end. The extent is the container's content size
/// along that axis (the widest/tallest child), so `center` reproduces the
/// old full-width-rect behavior and `start`/`end` shift the child.
fn cross_offset(align: WAlign, extent: u16, child: u16) -> u16 {
    match align {
        WAlign::Start => 0,
        WAlign::Center => extent.saturating_sub(child) / 2,
        WAlign::End => extent.saturating_sub(child),
    }
}

/// Each child's rect: the main axis stacks children in order (flush from the
/// container's origin); the cross axis places each child per `align`.
fn child_rects(
    area: Rect,
    align: WAlign,
    row: bool,
    children: &[Tree],
    desktop: bool,
) -> Vec<Rect> {
    let mut rects = Vec::with_capacity(children.len());
    let mut main = 0u16;
    for child in children {
        let (w, h) = (width_of(child, desktop), height_of(child, desktop));
        let (x, y) = if row {
            (area.x + main, area.y + cross_offset(align, area.height, h))
        } else {
            (area.x + cross_offset(align, area.width, w), area.y + main)
        };
        main += if row { w } else { h };
        rects.push(Rect::new(x, y, w, h));
    }
    rects
}

// --------------------------------------------------------------------------
// Rendering: the tree as ratatui widgets.

/// Map a node to a widget and render it into `area`. `desktop` enables the
/// tile chrome (a bordered box) on the desktop's tiles.
fn render(frame: &mut Frame, node: &Tree, area: Rect, focus: Option<u32>, desktop: bool) {
    match node {
        Tree::Text(content, style) => {
            frame.render_widget(
                Paragraph::new(content.clone())
                    .style(node_style(style))
                    .alignment(Alignment::Center),
                area,
            );
        }
        Tree::Button(id, label, style) => {
            // A display-only button (no id) can never be focused.
            let focused = *id == focus;
            // The component's semantic color is the button's base; the
            // host's focus ring (its own concern) overrides with its accent.
            let style = if focused {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                node_style(style)
            };
            let block = Block::default().borders(Borders::ALL).border_style(style);
            frame.render_widget(
                Paragraph::new(label.clone())
                    .style(style)
                    .alignment(Alignment::Center)
                    .block(block),
                area,
            );
        }
        Tree::Input(id, content, _, style) => {
            // A display-only field (no id) can never be focused. The
            // content is the component's copy of the buffer, so it is
            // left-aligned, like a text field.
            let focused = *id == focus;
            let style = if focused {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                node_style(style)
            };
            let block = Block::default().borders(Borders::ALL).border_style(style);
            frame.render_widget(
                Paragraph::new(content.clone()).style(style).block(block),
                area,
            );
        }
        // A terminal has no pixels: a dim placeholder box, the node's size.
        // An image is display-only, so the marker is dim, like the hint bar.
        Tree::Image(..) => {
            let style = Style::default().fg(Color::DarkGray);
            let block = Block::default().borders(Borders::ALL).border_style(style);
            frame.render_widget(
                Paragraph::new("image")
                    .style(style)
                    .alignment(Alignment::Center)
                    .block(block),
                area,
            );
        }
        // A tile: a bordered box around its children (the web host's
        // `.tile` chrome), the focus ring on the border when focused.
        Tree::Row(id, align, children) if desktop && is_tile(*id) => {
            render_tile(frame, id, *align, children, area, focus);
        }
        Tree::Column(id, align, children) if desktop && is_tile(*id) => {
            render_tile(frame, id, *align, children, area, focus);
        }
        Tree::Row(_, align, children) => {
            let areas = child_rects(area, *align, true, children, desktop);
            for (child, child_area) in children.iter().zip(areas) {
                render(frame, child, child_area, focus, desktop);
            }
        }
        Tree::Column(_, align, children) => {
            let areas = child_rects(area, *align, false, children, desktop);
            for (child, child_area) in children.iter().zip(areas) {
                render(frame, child, child_area, focus, desktop);
            }
        }
    }
}

/// One tile: a bordered box (the focus ring on the border when focused),
/// its children stacked inside.
fn render_tile(
    frame: &mut Frame,
    id: &Option<u32>,
    align: WAlign,
    children: &[Tree],
    area: Rect,
    focus: Option<u32>,
) {
    let focused = *id == focus;
    let style = if focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default().borders(Borders::ALL).border_style(style);
    frame.render_widget(Paragraph::new("").block(block), area);
    // The children inside the border, stacked as a column.
    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let areas = child_rects(inner, align, false, children, false);
    for (child, child_area) in children.iter().zip(areas) {
        render(frame, child, child_area, focus, false);
    }
}

/// One window: a bordered box, its title bar (the entry's name and the
/// close marker), the demo's tree centered in the content, and the focus
/// ring when the window is the active surface.
fn draw_window(frame: &mut Frame, win: &Window, area: Rect, active: bool) {
    let focus = if active { win.surface.focus } else { None };
    draw_window_box(frame, &win.title, &win.tree, area, focus, active);
}

/// A window's box: its border (the accent when active), its title bar (the
/// name and the close marker, on the top border row), and its content (the
/// tree centered, with one cell of padding).
pub fn draw_window_box(
    frame: &mut Frame,
    title: &str,
    tree: &Tree,
    area: Rect,
    focus: Option<u32>,
    active: bool,
) {
    let border = if active { Color::Cyan } else { Color::DarkGray };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border));
    frame.render_widget(Paragraph::new("").block(block), area);

    // The title bar: the name (bold when active, dim otherwise) and the
    // close marker, on the top border row.
    let title_style = Style::default()
        .fg(if active { Color::Cyan } else { Color::DarkGray })
        .add_modifier(if active {
            Modifier::BOLD
        } else {
            Modifier::DIM
        });
    let title_w = title.chars().count() as u16 + 2;
    // The bar's inner width, reserving the last cell for the close marker.
    let inner = area.width.saturating_sub(2);
    let mut line = Line::default();
    line.push_span(Span::styled(format!(" {} ", title), title_style));
    for _ in 0..inner.saturating_sub(title_w).saturating_sub(1) {
        line.push_span(Span::raw(" "));
    }
    line.push_span(Span::styled("×", title_style));
    frame.render_widget(
        Paragraph::new(line),
        Rect::new(area.x + 1, area.y, area.width.saturating_sub(2), 1),
    );

    // The content: one cell of padding, the tree centered in the rest.
    let content = Rect::new(
        area.x + 1,
        area.y + 2,
        area.width.saturating_sub(2),
        area.height.saturating_sub(3),
    );
    let cw = width_of(tree, false).min(content.width);
    let ch = height_of(tree, false).min(content.height);
    if cw > 0 && ch > 0 {
        let x = content.x + (content.width - cw) / 2;
        let y = content.y + (content.height - ch) / 2;
        render(frame, tree, Rect::new(x, y, cw, ch), focus, false);
    }
}

// --------------------------------------------------------------------------
// The hint bar: the host's key legend, per whether a window is open.

/// Its length floors the default `--dump` buffer's width (half of it), so
/// the bar wraps to at most two rows.
pub const HINT: &str =
    " waui (tui)   up/down or j/k: focus   enter/space: click   type: input   q: quit ";
pub const HINT_WINDOWS: &str = " waui (tui)   up/down or j/k: focus   enter: click/open   type: input   tab: raise   shift+tab: lower   esc: close   q: quit ";

pub fn hint_text(has_windows: bool) -> &'static str {
    if has_windows {
        HINT_WINDOWS
    } else {
        HINT
    }
}

/// The hint bar widget: dim, left-aligned, wrapping. `trim: false` keeps
/// the hint's leading space, so a wide terminal renders the bar unchanged.
fn hint_bar(has_windows: bool) -> Paragraph<'static> {
    Paragraph::new(hint_text(has_windows))
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: false })
}

/// The bar's row count at a width: as many rows as the hint wraps to, one
/// on a wide terminal.
pub fn hint_rows(width: u16, has_windows: bool) -> u16 {
    hint_bar(has_windows).line_count(width).max(1) as u16
}

/// Draw one full frame: a hint bar, the desktop centered below it, and the
/// open windows (in z-order, bottom to top) in their cascade positions,
/// clipped to the terminal. `has_registry` marks a desktop (the tile
/// chrome applies to its tiles whether or not a window is open); the hint
/// bar's windowing keys apply only while a window is open.
pub fn draw(
    frame: &mut Frame,
    desktop: &Tree,
    desktop_focus: Option<u32>,
    windows: &[Window],
    active: usize,
    has_registry: bool,
) {
    let area = frame.area();
    let has_windows = !windows.is_empty();
    // The bar takes its wrapped rows, at most the terminal's.
    let rows = hint_rows(area.width, has_windows).min(area.height.max(1));
    let bars: [Rect; 2] =
        Layout::vertical([Constraint::Length(rows), Constraint::Min(0)]).areas(area);
    frame.render_widget(hint_bar(has_windows), bars[0]);

    // The desktop: centered in the remaining space. Its ring only when it
    // is the active surface.
    let desktop_focus = if active == 0 { desktop_focus } else { None };
    let w = width_of(desktop, has_registry).min(bars[1].width);
    let h = height_of(desktop, has_registry).min(bars[1].height);
    if w > 0 && h > 0 {
        let x = bars[1].x + (bars[1].width - w) / 2;
        let y = bars[1].y + (bars[1].height - h) / 2;
        render(
            frame,
            desktop,
            Rect::new(x, y, w, h),
            desktop_focus,
            has_registry,
        );
    }

    // The windows, in z-order (the topmost last, over the desktop).
    for (i, win) in windows.iter().enumerate() {
        let (ww, wh) = window_size(win);
        let x = bars[1].x + win.x.min(bars[1].width.saturating_sub(ww.max(1)));
        let y = bars[1].y + win.y.min(bars[1].height.saturating_sub(wh.max(1)));
        draw_window(frame, win, Rect::new(x, y, ww, wh), i + 1 == active);
    }
}

// --------------------------------------------------------------------------
// Focus: a host-local ring over a surface's buttons, inputs, and (on a
// desktop) tiles, in pre-order.

/// One widget the focus ring can move to: its id, whether it is a text
/// input (printable keys type into it; enter/space does not activate it),
/// and whether it is a tile (enter opens its entry, not a click).
#[derive(Clone, Copy)]
pub struct FocusTarget {
    pub id: u32,
    pub is_input: bool,
    pub is_tile: bool,
}

fn focusables(node: &Tree, out: &mut Vec<FocusTarget>, desktop: bool) {
    match node {
        // A button the component gave no id cannot be clicked, so it is
        // not focusable.
        Tree::Button(id, ..) => {
            if let Some(i) = id {
                out.push(FocusTarget {
                    id: *i,
                    is_input: false,
                    is_tile: false,
                });
            }
        }
        // An id-less input is unaddressable (the WIT identity note), so it
        // is not focusable either.
        Tree::Input(id, ..) => {
            if let Some(i) = id {
                out.push(FocusTarget {
                    id: *i,
                    is_input: true,
                    is_tile: false,
                });
            }
        }
        Tree::Row(id, _, children) | Tree::Column(id, _, children) => {
            // A tile is a focusable container (the desktop's
            // double-clickable node); enter on it opens its entry.
            if desktop && is_tile(*id) {
                if let Some(i) = id {
                    out.push(FocusTarget {
                        id: *i,
                        is_input: false,
                        is_tile: true,
                    });
                }
            }
            for child in children {
                focusables(child, out, desktop);
            }
        }
        // An image is display-only; it is not focusable.
        Tree::Text(..) | Tree::Image(..) => {}
    }
}

/// The content of the input with `id`, wherever it sits in the tree.
pub fn input_content(node: &Tree, id: u32) -> Option<&str> {
    match node {
        Tree::Input(iid, content, _, _) if *iid == Some(id) => Some(content),
        Tree::Row(_, _, children) | Tree::Column(_, _, children) => {
            children.iter().find_map(|c| input_content(c, id))
        }
        _ => None,
    }
}

/// Re-seat one surface's ring on its (possibly rebuilt) tree: recompute
/// the targets, normalize the saved index, and the focused id.
pub fn reseat(tree: &Tree, desktop: bool, surf: &mut Surface) {
    surf.targets.clear();
    focusables(tree, &mut surf.targets, desktop);
    if surf.targets.is_empty() {
        surf.focus_idx = 0;
        surf.focus = None;
    } else {
        let n = surf.targets.len();
        surf.focus_idx %= n;
        surf.focus = surf.targets.get(surf.focus_idx).map(|t| t.id);
    }
}

/// The active surface: the desktop, or a window by its z-order index.
pub fn active_surface<'a>(
    store: &'a mut Store<Host>,
    desktop: &'a mut Surface,
    active: usize,
) -> &'a mut Surface {
    if active == 0 {
        desktop
    } else {
        &mut store.data_mut().windows[active - 1].surface
    }
}

/// Re-seat the active surface's ring on its tree (after a rebuild or a
/// surface switch).
pub fn reseat_active(
    store: &mut Store<Host>,
    tree: &Tree,
    has_registry: bool,
    active: &mut usize,
    desktop: &mut Surface,
) {
    if *active == 0 {
        reseat(tree, has_registry, desktop);
    } else if let Some(win) = store.data_mut().windows.get_mut(*active - 1) {
        reseat(&win.tree, false, &mut win.surface);
    } else {
        // The active window was closed; the desktop is active.
        *active = 0;
        reseat(tree, has_registry, desktop);
    }
}

/// The active surface's component and store: the desktop, or the window's.
pub fn active_app<'a>(
    store: &'a mut Store<Host>,
    app: &'a App,
    active: usize,
) -> (&'a App, &'a mut Store<Host>) {
    if active == 0 {
        (app, store)
    } else {
        let win = &mut store.data_mut().windows[active - 1];
        (&win.app, &mut win.store)
    }
}

/// The active surface's tree: the desktop's, or the window's.
pub fn active_tree<'a>(store: &'a Store<Host>, tree: &'a Tree, active: usize) -> &'a Tree {
    if active == 0 {
        tree
    } else {
        &store.data().windows[active - 1].tree
    }
}

/// The active surface's ring target, as values (a key handler that also
/// needs the store cannot hold a borrow of the surface).
pub fn active_target(
    store: &mut Store<Host>,
    desktop: &mut Surface,
    active: usize,
) -> (u32, bool, bool) {
    let surf = active_surface(store, desktop, active);
    surf.targets
        .get(surf.focus_idx)
        .map(|t| (t.id, t.is_input, t.is_tile))
        .unwrap_or((0, false, false))
}

/// Report the focused input's whole buffer to the component (`on-text`)
/// and mark the diff pending: the component re-derives the field's content
/// from what it stored, and the next `changes` carries it back.
pub fn report_input(app: &App, store: &mut Store<Host>, id: u32, buf: &str, pending: &mut bool) {
    app.call_on_text(store, id, buf).expect("on-text() failed");
    *pending = true;
}

// --------------------------------------------------------------------------
// The `--dump` frame: the byte-parity surface.

/// A terminal color as an ANSI SGR foreground code, for the dump. Returns
/// `None` for colors a plain text dump cannot represent (reset, truecolor,
/// indexed).
fn sgr_fg(c: Color) -> Option<&'static str> {
    Some(match c {
        Color::Black => "\x1b[30m",
        Color::Red => "\x1b[31m",
        Color::Green => "\x1b[32m",
        Color::Yellow => "\x1b[33m",
        Color::Blue => "\x1b[34m",
        Color::Magenta => "\x1b[35m",
        Color::Cyan => "\x1b[36m",
        Color::Gray => "\x1b[37m",
        Color::DarkGray => "\x1b[90m",
        Color::LightRed => "\x1b[91m",
        Color::LightGreen => "\x1b[92m",
        Color::LightYellow => "\x1b[93m",
        Color::LightBlue => "\x1b[94m",
        Color::LightMagenta => "\x1b[95m",
        Color::LightCyan => "\x1b[96m",
        Color::White => "\x1b[97m",
        Color::Reset | Color::Rgb(..) | Color::Indexed(_) => return None,
    })
}

/// One full frame as text: rendered into a memory buffer (`TestBackend`)
/// and serialized with ANSI SGR codes for the styled cells, trailing
/// spaces trimmed per row — the same bytes the `--dump` flag prints.
///
/// The buffer fits the desktop whole (floored at half the hint's length,
/// so the bar wraps to at most two rows) and the windows' cascade extent,
/// or `width` columns when given (the hint bar wraps, as on a narrow
/// terminal). A dump has no pointer to raise a window: the topmost is
/// active, with its ring on its first focusable.
///
/// This is the byte-parity surface between the native and the browser
/// host: a pure function of the tree, the windows, the width, and whether
/// a registry is present, so the two must render identically.
pub fn dump_frame(
    tree: &Tree,
    windows: &mut [Window],
    width: Option<u16>,
    has_registry: bool,
) -> String {
    // The topmost window is active: re-seat its ring on its first focusable.
    if let Some(win) = windows.last_mut() {
        reseat(&win.tree, false, &mut win.surface);
    }
    let has_windows = !windows.is_empty();
    // The buffer fits the desktop whole, floored at half the hint's length,
    // so the bar wraps to at most two rows.
    let w = width.unwrap_or_else(|| {
        let base = width_of(tree, has_registry)
            .max((hint_text(has_windows).chars().count() / 2) as u16)
            + 2;
        // The desktop is centered and the windows sit at their cascade
        // positions, so the buffer also widens until a window no longer
        // overlaps the desktop (a real terminal is wider still; the dump
        // shows the frame without the overlap).
        let no_overlap = windows
            .iter()
            .map(|win| 2 * (win.x + window_size(win).0) + width_of(tree, has_registry))
            .max()
            .unwrap_or(0);
        base.max(no_overlap)
    });
    // The bar's wrapped rows, then the desktop with one margin row above
    // and below, and the windows' cascade extent.
    let hint = hint_rows(w, has_windows);
    let area_h = height_of(tree, has_registry).saturating_add(2).max(
        windows
            .iter()
            .map(|win| win.y + window_size(win).1 + 2)
            .max()
            .unwrap_or(0),
    );
    let h = area_h + hint;
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("failed to create terminal");
    let mut desktop_surf = Surface::default();
    reseat(tree, has_registry, &mut desktop_surf);
    let active = if has_windows { windows.len() } else { 0 };
    let completed = terminal
        .draw(|f| draw(f, tree, desktop_surf.focus, &windows, active, has_registry))
        .expect("draw failed");
    let buf = completed.buffer;
    let area = buf.area();
    // Print the buffer as text, prefixed with ANSI escapes for styled
    // cells, so the frame shows the component's semantic styling — the
    // same colors and weight the live terminal would render.
    let mut out = String::new();
    for y in 0..area.height {
        let mut line = String::new();
        let mut styled = false;
        for x in 0..area.width {
            let (sym, fg, modifier) = match buf.cell((x, y)) {
                Some(c) => (c.symbol(), c.fg, c.modifier),
                None => (" ", Color::Reset, Modifier::empty()),
            };
            if let Some(code) = sgr_fg(fg) {
                line.push_str(code);
                styled = true;
            }
            if modifier.intersects(Modifier::BOLD | Modifier::DIM) {
                line.push_str(if modifier.contains(Modifier::BOLD) {
                    "\x1b[1m"
                } else {
                    "\x1b[2m"
                });
                styled = true;
            }
            line.push_str(sym);
        }
        if styled {
            line.push_str("\x1b[0m");
        }
        // Trim trailing spaces so the frame reads like terminal output.
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

// The browser module (the xterm.js backend and the wasm-bindgen exports).
// It is part of this crate, so it reaches the private renderer above.
#[cfg(target_arch = "wasm32")]
pub mod web;

#[cfg(test)]
mod tests {
    //! The frame is a pure function of (tree, focus, windows, terminal
    //! size), so a `TestBackend` pins the host-local behavior the component
    //! goldens do not reach: the hint bar on a terminal narrower than the
    //! hint, and the window's chrome.

    use super::*;

    /// The words visible in the buffer, in reading order (top to bottom,
    /// left to right).
    fn visible_words(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buf = terminal.backend().buffer();
        let area = buf.area();
        let mut words = Vec::new();
        for y in 0..area.height {
            let line: String = (0..area.width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                .collect();
            for word in line.split_whitespace() {
                words.push(word.to_string());
            }
        }
        words
    }

    /// A terminal narrower than the hint: the bar wraps and every word of
    /// the hint stays visible — none of it clipped.
    #[test]
    fn the_hint_bar_wraps_on_a_narrow_terminal() {
        let mut terminal =
            Terminal::new(TestBackend::new(40, 12)).expect("failed to create terminal");
        let tree = Tree::Column(None, WAlign::Center, Vec::new());
        terminal
            .draw(|f| draw(f, &tree, None, &[], 0, false))
            .expect("draw failed");
        let want: Vec<String> = HINT.split_whitespace().map(str::to_string).collect();
        assert_eq!(
            visible_words(&terminal),
            want,
            "the hint bar lost words on a 40-column terminal"
        );
    }

    /// A wide terminal: the bar stays one row, byte-identical to the
    /// pre-wrap behavior, and takes no further rows.
    #[test]
    fn the_hint_bar_is_one_row_on_a_wide_terminal() {
        let mut terminal =
            Terminal::new(TestBackend::new(100, 10)).expect("failed to create terminal");
        let tree = Tree::Column(None, WAlign::Center, Vec::new());
        terminal
            .draw(|f| draw(f, &tree, None, &[], 0, false))
            .expect("draw failed");
        let buf = terminal.backend().buffer();
        let row = |y: u16| -> String {
            (0..buf.area().width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                .collect()
        };
        // `trim_end` on both sides: the row is padded to the buffer width
        // with unstyled spaces. The leading space of the hint is asserted.
        assert_eq!(row(0).trim_end(), HINT.trim_end());
        assert!(
            (1..buf.area().height).all(|y| row(y).trim().is_empty()),
            "the bar should take exactly one row on a wide terminal"
        );
    }

    /// A window: its title bar (the entry's name and the close marker) and
    /// its border, the demo's tree centered in the content.
    #[test]
    fn a_window_draws_its_title_and_border() {
        let mut terminal =
            Terminal::new(TestBackend::new(40, 16)).expect("failed to create terminal");
        let tree = Tree::Column(
            None,
            WAlign::Center,
            vec![Tree::Text(
                "0".to_string(),
                WStyle {
                    color: WColor::Neutral,
                    size: WSize::Large,
                },
            )],
        );
        terminal
            .draw(|f| draw_window_box(f, "counter", &tree, Rect::new(4, 2, 14, 8), None, true))
            .expect("draw failed");
        let words = visible_words(&terminal);
        assert!(
            words.iter().any(|w| w == "counter"),
            "the title bar should show the entry's name: {words:?}"
        );
        assert!(
            words.iter().any(|w| w.contains('×')),
            "the title bar should show the close marker: {words:?}"
        );
    }
}
