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
//! With a registry (`--entry name=path`, repeatable) the host is a
//! multi-window interface: a window manager. A double-clicked tile (a
//! container with an id in the tile range, as the web host's `.tile` marks
//! it) opens its entry as a window — the entry's component running in its
//! own store, drawn as an overlapping bordered box. Windows stack in z-order
//! (newest on top), cascade their placement, and are raised and lowered with
//! tab and shift+tab; esc closes the active window, and quits the app.
//!
//! The loop wakes each component — the app and every window — at the
//! period it requests with `schedule` (0 = never; the 50ms event poll sets
//! the granularity) and, when the `changes` diff is non-empty, refreshes
//! its retained tree — a content-only diff in place, a structural one
//! re-queried from `view` (a diff cannot carry id-less nodes) — and
//! rebuilds; otherwise it redraws.

use std::collections::BTreeMap;
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
use host_shared::{
    build_tree, waui, App, AppImports, ContainerStyle, Entry, EntrySpec, Error, Retained, Tree,
    ViewportSize,
};

// The contract's style tokens, aliased to avoid clashing with ratatui's
// `Color` and `Style`.
use host_shared::waui::ui::widget::{
    Align as WAlign, Color as WColor, Cursor as WCursor, Size as WSize, Stretch as WStretch,
    Style as WStyle,
};

use ratatui::{
    backend::TestBackend,
    layout::{Alignment, Constraint, Layout, Margin, Rect, Size},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame, Terminal,
};
// The halfblocks image encoder: the vendored copy of ratatui-image's
// pure-Rust halfblocks path, for the surfaces the crate does not run on
// (the `--dump` frame and the browser).
mod halfblocks;

// The decoded images and their rasters: the `image` crate's type. `FilterType`
// is the resize filter the native protocol path uses (the halfblocks encoder
// carries its own, in `halfblocks.rs`), so it is native-only.
#[cfg(not(target_arch = "wasm32"))]
use image::imageops::FilterType;
use image::DynamicImage;

/// The component's refresh request, shared between the world-import impl
/// (the writer) and the loop (the reader): the period in ms, 0 = no
/// periodic wake, and the last wake.
#[derive(Default)]
pub struct Wake {
    pub rate_ms: u64,
    pub last_wake_ms: u64,
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
/// gets its own), its retained tree, and its ring. The window's chrome and
/// placement are the app's (the component draws them); the host only seats
/// this component's tree in the app's content slot (the container whose id
/// is the handle).
pub struct Window {
    pub handle: u32,
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
    /// The registry (`--entry`): a non-empty one makes this host a
    /// multi-window host.
    pub entries: Vec<EntrySpec>,
    /// The entry components' bytes, as the page handed them over (the
    /// browser: `run`'s snapshot, then `components-add` as its background
    /// fetches resolve; the native host keeps it empty and reads from the
    /// filesystem). An arc, shared with the windows' hosts and with the
    /// live-host handle the page's `components-add` export writes through.
    pub components: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    /// The image state: the draw mode, the page's assets, and the decoded
    /// images (the registry's icons, keyed by `src`).
    pub image: ImageState,
    /// The open windows, in z-order (last = topmost).
    pub windows: Vec<Window>,
    /// The next window handle (the host's flat id, from [`HANDLE_BASE`]).
    pub next_handle: u32,
}

/// The first window handle. The app uses a window's handle as the id of the
/// content slot it seats the opened component's tree in, so the base is the
/// world's handle base (the component's `open-entry` returns it).
const HANDLE_BASE: u32 = 10_000;

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
            components: Arc::new(Mutex::new(BTreeMap::new())),
            image: ImageState::default(),
            windows: Vec::new(),
            next_handle: HANDLE_BASE,
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
    /// clock, but an empty registry (a window runs a demo, not a window
    /// manager, so the tile rule never fires in it) and its own window list.
    fn window_host(&self) -> Host {
        Host {
            wake: Arc::new(Mutex::new(Wake::default())),
            fake_now: self.fake_now,
            fake_calls: 0,
            engine: self.engine.clone(),
            linker: self.linker.clone(),
            entries: Vec::new(),
            // The same component map (a window never opens entries itself,
            // but its host stays self-contained).
            components: self.components.clone(),
            // The same draw mode (a window's images, if a demo ever has
            // them, render like the app's); a fresh cache and no assets.
            image: ImageState {
                mode: self.image.mode.clone(),
                ..ImageState::default()
            },
            windows: Vec::new(),
            next_handle: HANDLE_BASE,
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
                data: e.data.clone(),
            })
            .collect()
    }
    // This host has no viewport: the unknown size (see the WIT note). A
    // component skips viewport-dependent math (a drag clamp) on `(0, 0)`.
    fn viewport(&mut self) -> ViewportSize {
        ViewportSize { w: 0, h: 0 }
    }
    /// Open a window running the entry with `name`: the entry's component
    /// in a new store, seated in the app's content slot (the container whose
    /// id is the returned handle). The placement and size are the app's (the
    /// component draws the window); the geometry arguments are the
    /// component's request and are ignored here.
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
        let components = self.components.lock().unwrap();
        let component = match load_component(&self.engine, &components, &spec.path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("open-entry: failed to load {}: {e}", spec.path);
                return Err(Error::NotFound);
            }
        };
        drop(components);
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
        self.windows.push(Window {
            handle,
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
    /// Drop the window with `handle` (the component closed it): the host
    /// frees the entry's store. Idempotent — an unknown handle is a no-op.
    fn close_entry(&mut self, handle: u32) {
        self.windows.retain(|w| w.handle != handle);
    }
}

/// Load the entry's component: from the host's component map when it has
/// the bytes (the browser: the page fetches the artifacts and hands them
/// over, in `run` and as its background fetches resolve), else from the
/// filesystem (the native host; the map stays empty there).
fn load_component(
    engine: &Engine,
    components: &BTreeMap<String, Vec<u8>>,
    path: &str,
) -> Result<Component, wasmtime::Error> {
    if let Some(bytes) = components.get(path) {
        return unsafe { Component::deserialize(engine, bytes) };
    }
    #[cfg(not(target_arch = "wasm32"))]
    return Component::from_file(engine, path);
    #[cfg(target_arch = "wasm32")]
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
// --------------------------------------------------------------------------
// Layout sizing: how much terminal space a tree needs.

/// A container the app makes double-clickable: one the component marked
/// `cursor: pointer` (the WIT's double-clickable marker, replacing the old
/// id-range hack). The rule applies only on a multi-window host (a non-empty
/// registry): a standalone demo's containers are not tiles.
fn is_tile(style: &ContainerStyle) -> bool {
    style.cursor == WCursor::Pointer
}

// --------------------------------------------------------------------------
// Images: the registry's icons, decoded per `src` and drawn per surface.

/// How an image node is drawn.
#[derive(Clone)]
pub enum ImageMode {
    /// The native live terminal: ratatui-image, the terminal's graphics
    /// protocol (sixel, kitty, iterm2) or its halfblocks fallback.
    #[cfg(not(target_arch = "wasm32"))]
    Protocol(ratatui_image::picker::Picker),
    /// The `--dump` frame and the browser: the vendored halfblocks encoder
    /// (the crate does not compile for wasm32-unknown-unknown).
    Halfblocks,
}

impl Default for ImageMode {
    fn default() -> Self {
        ImageMode::Halfblocks
    }
}

/// One decoded image, ready to draw at a cell size: the encoded halfblocks
/// (both surfaces), or the built protocol (the native live terminal).
enum ImageData {
    Halfblocks {
        data: Vec<halfblocks::HalfBlock>,
        size: Size,
    },
    #[cfg(not(target_arch = "wasm32"))]
    Protocol(ratatui_image::protocol::Protocol),
}

/// The host's image state: the draw mode, the asset bytes the page provided
/// (the browser; the native host resolves a `src` against the working
/// directory instead), and the decoded images, keyed by `src` and cell size.
/// The decoded result is a pure function of the `src` bytes and the size, so
/// caching it keeps the frame a pure function of the tree.
pub struct ImageState {
    pub mode: ImageMode,
    pub assets: BTreeMap<String, Vec<u8>>,
    rasters: BTreeMap<String, Option<DynamicImage>>,
    data: BTreeMap<(String, u16, u16), Option<ImageData>>,
}

impl Default for ImageState {
    fn default() -> Self {
        Self {
            mode: ImageMode::default(),
            assets: BTreeMap::new(),
            rasters: BTreeMap::new(),
            data: BTreeMap::new(),
        }
    }
}

impl ImageState {
    /// The image's bytes: the page's assets first (the browser), else the
    /// file at `src` (the native host; the registry's icon paths are
    /// relative to the working directory).
    fn bytes(&self, src: &str) -> Option<Vec<u8>> {
        if let Some(bytes) = self.assets.get(src) {
            return Some(bytes.clone());
        }
        // The browser has no filesystem, so only the page's assets.
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::fs::read(src).ok()
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }

    /// The decoded image at its intrinsic size, cached per `src`: the
    /// registry's icons are SVG, which the `image` crate cannot decode, so
    /// resvg rasterizes them; the other formats go through `image`. A failed
    /// decode is cached, so a bad `src` costs one attempt.
    fn raster(&mut self, src: &str) -> Option<DynamicImage> {
        if let Some(img) = self.rasters.get(src) {
            return img.clone();
        }
        let img = self.bytes(src).and_then(|bytes| {
            if src.ends_with(".svg") {
                rasterize_svg(&bytes)
            } else {
                image::load_from_memory(&bytes).ok()
            }
        });
        self.rasters.insert(src.to_string(), img.clone());
        img
    }

    /// The image drawn at `size` cells, built once per (`src`, size).
    fn image(&mut self, src: &str, size: Size) -> Option<&ImageData> {
        let key = (src.to_string(), size.width, size.height);
        if !self.data.contains_key(&key) {
            let built = self.raster(src).and_then(|img| match &self.mode {
                ImageMode::Halfblocks => Some(ImageData::Halfblocks {
                    data: halfblocks::encode(&img, size),
                    size,
                }),
                #[cfg(not(target_arch = "wasm32"))]
                ImageMode::Protocol(picker) => picker
                    .new_protocol(
                        img,
                        size,
                        ratatui_image::Resize::Fit(Some(FilterType::Triangle)),
                    )
                    .ok()
                    .map(ImageData::Protocol),
            });
            self.data.insert(key.clone(), built);
        }
        self.data.get(&key)?.as_ref()
    }
}

/// An SVG at its own size, as RGBA: usvg parses it and resvg rasterizes it.
/// The registry's icons are viewBoxes without a size, so their size is the
/// viewBox's.
fn rasterize_svg(bytes: &[u8]) -> Option<DynamicImage> {
    let options = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(bytes, &options).ok()?;
    let size = tree.size();
    let w = size.width() as u32;
    let h = size.height() as u32;
    if w == 0 || h == 0 {
        return None;
    }
    let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let rgba = pixmap.data().to_vec();
    Some(image::RgbaImage::from_raw(w, h, rgba)?.into())
}

// The tui's geometry scale: the component's pixel geometry (a positioned
// container's `x`/`y`/`w`/`h`, an image's size) mapped onto the terminal's
// character cells — 8px wide, 16px tall, the cell's aspect. `0` leaves a
// dimension to the host's default. The input's width is in characters and
// stays 1:1 (a cell is a character).
const CELL_W: u32 = 8;
const CELL_H: u32 = 16;

/// An image's cell size: its pixel size scaled to the cell metric, or the
/// host's default when a dimension is 0.
fn image_width(width: u32) -> u16 {
    if width > 0 {
        (width / CELL_W).max(1) as u16
    } else {
        8
    }
}

/// An image's cell height: its pixel height scaled to the cell metric, or
/// the host's default when it is 0.
fn image_height(height: u32) -> u16 {
    if height > 0 {
        (height / CELL_H).max(1) as u16
    } else {
        3
    }
}

/// An image's slot in cells.
fn image_slot(width: u32, height: u32) -> Size {
    Size::new(image_width(width), image_height(height))
}

/// A container's box, if it has one: a positioned container (a window) or a
/// tile (the component's `cursor: pointer` marker, in-flow) is a bordered
/// box; the rest flow without a box.
fn is_box(style: &ContainerStyle, has_registry: bool) -> bool {
    style.positioned || (has_registry && is_tile(style))
}

pub fn width_of(node: &Tree, has_registry: bool) -> u16 {
    match node {
        Tree::Text(content, _) => content.chars().count() as u16,
        // A link is its label: the href is the host's, not the layout's.
        Tree::Link(_, label, _, _) => label.chars().count() as u16,
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
        // A decoded image fills its slot; the placeholder, when the `src`
        // does not resolve, is framed within it.
        Tree::Image(_, _, width, _) => image_width(*width),
        // A bordered box (a tile or a window): a positioned box with an
        // explicit width uses it (scaled); the rest size to their widest
        // child plus the two border columns and the padding (the component's
        // pixel padding scaled to cells).
        Tree::Row(_, _, style, children) | Tree::Column(_, _, style, children)
            if is_box(style, has_registry) =>
        {
            if style.positioned && style.w > 0 {
                (style.w / CELL_W).max(1) as u16
            } else {
                let pad = (style.padding / CELL_W) as u16;
                children
                    .iter()
                    .map(|c| width_of(c, false))
                    .max()
                    .unwrap_or(0)
                    + 2
                    + 2 * pad
            }
        }
        // In-flow: a positioned child (a window) is out of flow, so it takes
        // no space in the row/column's extent.
        Tree::Row(_, _, _, children) => children
            .iter()
            .filter(|c| !is_positioned(c))
            .map(|c| width_of(c, has_registry))
            .sum(),
        Tree::Column(_, _, _, children) => children
            .iter()
            .filter(|c| !is_positioned(c))
            .map(|c| width_of(c, has_registry))
            .max()
            .unwrap_or(0),
    }
}

pub fn height_of(node: &Tree, has_registry: bool) -> u16 {
    match node {
        Tree::Text(..) => 1,
        Tree::Link(..) => 1,
        Tree::Button(..) => 3, // border top, label, border bottom
        Tree::Input(..) => 3,  // border top, content, border bottom
        // A decoded image fills its slot; the placeholder, when the `src`
        // does not resolve, is framed within it.
        Tree::Image(_, _, _, height) => image_height(*height),
        // A bordered box (a tile or a window): a positioned box with an
        // explicit height uses it (scaled); the rest size to their tallest
        // child plus the two border rows and the padding (the component's
        // pixel padding scaled to cells).
        Tree::Row(_, _, style, children) | Tree::Column(_, _, style, children)
            if is_box(style, has_registry) =>
        {
            if style.positioned && style.h > 0 {
                (style.h / CELL_H).max(1) as u16
            } else {
                let pad = (style.padding / CELL_H) as u16;
                children
                    .iter()
                    .map(|c| height_of(c, false))
                    .max()
                    .unwrap_or(0)
                    + 2
                    + 2 * pad
            }
        }
        // In-flow: a positioned child (a window) is out of flow, so it takes
        // no space in the row/column's extent.
        Tree::Row(_, _, _, children) => children
            .iter()
            .filter(|c| !is_positioned(c))
            .map(|c| height_of(c, has_registry))
            .max()
            .unwrap_or(0),
        Tree::Column(_, _, _, children) => children
            .iter()
            .filter(|c| !is_positioned(c))
            .map(|c| height_of(c, has_registry))
            .sum(),
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
        WColor::Dim => Color::DarkGray,
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

/// The stack's main-axis offset within the container's main extent: the
/// token's start/center/end. `occupied` is the main space the whole stack
/// takes (the children and their gaps), so `start` reproduces the old flush
/// layout and `center`/`end` place a stack that does not fill the axis.
fn main_offset(align: WAlign, extent: u16, occupied: u16) -> u16 {
    match align {
        WAlign::Start => 0,
        WAlign::Center => extent.saturating_sub(occupied) / 2,
        WAlign::End => extent.saturating_sub(occupied),
    }
}

/// A node's stretch token (containers only; a leaf never stretches).
fn stretch_of(node: &Tree) -> WStretch {
    match node {
        Tree::Row(_, _, style, _) | Tree::Column(_, _, style, _) => style.stretch,
        _ => WStretch::None,
    }
}

/// A node's positioned flag (containers only; a leaf is never positioned).
fn is_positioned(node: &Tree) -> bool {
    matches!(
        node,
        Tree::Row(_, _, s, _) | Tree::Column(_, _, s, _) if s.positioned
    )
}

/// A positioned container's origin in cells (its pixel position scaled).
fn pos_x(node: &Tree) -> u16 {
    match node {
        Tree::Row(_, _, s, _) | Tree::Column(_, _, s, _) => (s.x / CELL_W) as u16,
        _ => 0,
    }
}

/// A positioned container's y in cells (its pixel position scaled).
fn pos_y(node: &Tree) -> u16 {
    match node {
        Tree::Row(_, _, s, _) | Tree::Column(_, _, s, _) => (s.y / CELL_H) as u16,
        _ => 0,
    }
}

/// A node's stacking order (a positioned container's `z`; `0` for the rest).
fn z_of(node: &Tree) -> u32 {
    match node {
        Tree::Row(_, _, s, _) | Tree::Column(_, _, s, _) => s.z,
        _ => 0,
    }
}

/// A container's explicit size in cells (its pixel `w`/`h` scaled); `(0, 0)`
/// for a dimension left to content, and for a leaf (which has no size).
fn explicit_wh(node: &Tree) -> (u16, u16) {
    match node {
        Tree::Row(_, _, s, _) | Tree::Column(_, _, s, _) => {
            let w = if s.w > 0 {
                (s.w / CELL_W).max(1) as u16
            } else {
                0
            };
            let h = if s.h > 0 {
                (s.h / CELL_H).max(1) as u16
            } else {
                0
            };
            (w, h)
        }
        _ => (0, 0),
    }
}

/// A child's explicit `(main, cross)` size in cells, per the container's
/// direction; `0` leaves a dimension open. The main axis is the `direction`
/// axis: width for a row, height for a column.
fn explicit_main_cross(node: &Tree, row: bool) -> (u16, u16) {
    let (w, h) = explicit_wh(node);
    if row {
        (w, h)
    } else {
        (h, w)
    }
}

/// Whether a child stretches the main axis.
fn stretches_main(node: &Tree) -> bool {
    matches!(stretch_of(node), WStretch::Main | WStretch::Both)
}

/// Whether a child stretches the cross axis.
fn stretches_cross(node: &Tree) -> bool {
    matches!(stretch_of(node), WStretch::Cross | WStretch::Both)
}

/// Whether a child's main axis is the container's cross axis: the child's
/// `direction` is orthogonal to the container's. A leaf never stretches.
fn orthogonal(node: &Tree, row: bool) -> bool {
    match node {
        Tree::Row(..) => !row,
        Tree::Column(..) => row,
        _ => false,
    }
}

/// Whether a child fills the container's main axis. The WIT's axes are the
/// child's own (`main` is its `direction` axis): a same-direction child's
/// `main` stretch, or an orthogonal child's `cross` stretch, is the
/// container's main.
fn fills_main(node: &Tree, row: bool) -> bool {
    if orthogonal(node, row) {
        stretches_cross(node)
    } else {
        stretches_main(node)
    }
}

/// Whether a child fills the container's cross axis (the other case).
fn fills_cross(node: &Tree, row: bool) -> bool {
    if orthogonal(node, row) {
        stretches_main(node)
    } else {
        stretches_cross(node)
    }
}

/// A child's content size along the main / cross axes, per the container's
/// direction.
fn content_main_cross(node: &Tree, row: bool, has_registry: bool) -> (u16, u16) {
    let (w, h) = (width_of(node, has_registry), height_of(node, has_registry));
    if row {
        (w, h)
    } else {
        (h, w)
    }
}

/// Each child's rect, in draw order: the in-flow children first (stacked in
/// order along the main axis with the container's `gap`, placed per `align`
/// on both axes), then the positioned children (at their `x`/`y`, scaled
/// to cells) sorted by `z` — the later drawn wins, so a higher `z` sits on
/// top. The container's `padding` insets the whole layout.
///
/// A child's size: its explicit size when set (an explicit size wins over a
/// stretch, as in the web host's flexbox), else the space it fills along a
/// filled axis, else its content size. Children that fill the main axis
/// share its leftover space equally (the menubar's spacer). `0` gap and
/// padding (the demos) reproduce the old flush layout.
fn child_rects(
    area: Rect,
    align: WAlign,
    row: bool,
    style: &ContainerStyle,
    children: &[Tree],
    has_registry: bool,
) -> Vec<(usize, Rect)> {
    // The content area, inset by the container's padding (the component's
    // pixel padding scaled to cells along each axis).
    let pad_x = (style.padding / CELL_W) as u16;
    let pad_y = (style.padding / CELL_H) as u16;
    let inner = Rect::new(
        area.x + pad_x,
        area.y + pad_y,
        area.width.saturating_sub(pad_x * 2),
        area.height.saturating_sub(pad_y * 2),
    );
    let main_size = if row { inner.width } else { inner.height };
    let cross_size = if row { inner.height } else { inner.width };
    // The gap between children, in cells (the component's pixel gap scaled
    // along the main axis).
    let gap = if row {
        (style.gap / CELL_W) as u16
    } else {
        (style.gap / CELL_H) as u16
    };

    let in_flow: Vec<usize> = (0..children.len())
        .filter(|&i| !is_positioned(&children[i]))
        .collect();

    // Pass 1: the main-axis space the non-filling children and the gaps
    // take, so the main-filling children can share what is left.
    let fixed_main: u16 = in_flow
        .iter()
        .map(|&i| {
            let child = &children[i];
            let (ex_main, _) = explicit_main_cross(child, row);
            if ex_main > 0 {
                ex_main
            } else if fills_main(child, row) {
                0
            } else {
                content_main_cross(child, row, has_registry).0
            }
        })
        .sum::<u16>()
        + gap.saturating_mul((in_flow.len().saturating_sub(1)) as u16);
    let remaining = main_size.saturating_sub(fixed_main);
    let n_stretch = in_flow
        .iter()
        .filter(|&&i| {
            let (ex_main, _) = explicit_main_cross(&children[i], row);
            ex_main == 0 && fills_main(&children[i], row)
        })
        .count() as u16;
    let stretch_share = if n_stretch > 0 {
        remaining / n_stretch
    } else {
        0
    };
    // The main space the whole stack takes: the fixed children and the gaps,
    // plus the stretch shares. The `align` token places the stack within the
    // container's main extent on that axis.
    let total_main = fixed_main + stretch_share * n_stretch;

    // Pass 2: place the in-flow children in order, the stack offset per
    // `align` on the main axis.
    let mut out = Vec::with_capacity(children.len());
    let mut main = main_offset(align, main_size, total_main);
    let mut placed_any = false;
    for &i in &in_flow {
        let child = &children[i];
        if placed_any && gap > 0 {
            main += gap;
        }
        placed_any = true;
        let (ex_main, ex_cross) = explicit_main_cross(child, row);
        let (c_main, c_cross) = content_main_cross(child, row, has_registry);
        let main_sz = if ex_main > 0 {
            ex_main
        } else if fills_main(child, row) {
            stretch_share
        } else {
            c_main
        };
        let cross_sz = if ex_cross > 0 {
            ex_cross
        } else if fills_cross(child, row) {
            cross_size
        } else {
            c_cross
        };
        let (w, h) = if row {
            (main_sz, cross_sz)
        } else {
            (cross_sz, main_sz)
        };
        let (x, y) = if row {
            (inner.x + main, inner.y + cross_offset(align, cross_size, h))
        } else {
            (inner.x + cross_offset(align, cross_size, w), inner.y + main)
        };
        main += main_sz;
        out.push((i, Rect::new(x, y, w, h)));
    }

    // The positioned children, sorted by `z` (stable, so equal `z` keeps the
    // component's order); the later drawn wins.
    let mut positioned: Vec<usize> = (0..children.len())
        .filter(|&i| is_positioned(&children[i]))
        .collect();
    positioned.sort_by_key(|&i| z_of(&children[i]));
    for i in positioned {
        let child = &children[i];
        let (w, h) = (
            width_of(child, has_registry),
            height_of(child, has_registry),
        );
        let x = inner.x + pos_x(child).min(inner.width.saturating_sub(w.max(1)));
        let y = inner.y + pos_y(child).min(inner.height.saturating_sub(h.max(1)));
        out.push((i, Rect::new(x, y, w, h)));
    }
    out
}

// --------------------------------------------------------------------------
// Rendering: the tree as ratatui widgets.

/// Map a node to a widget and render it into `area`. `has_registry` enables
/// the tile chrome (a bordered box) on the app's tiles. A content slot (a
/// container whose id is a window's handle) seats the opened component's
/// tree there, centered in the slot's area.
fn render(
    frame: &mut Frame,
    node: &Tree,
    area: Rect,
    focus: Option<u32>,
    has_registry: bool,
    windows: &[Window],
    images: &mut ImageState,
) {
    // A content slot: seat the opened component's tree here (centered). The
    // slot is empty in the app's tree; the window's tree fills it.
    if let Some(i) = match node {
        Tree::Row(id, ..) | Tree::Column(id, ..) => *id,
        _ => None,
    } {
        if let Some(win) = windows.iter().find(|w| w.handle == i) {
            let (cw, ch) = (width_of(&win.tree, false), height_of(&win.tree, false));
            let cw = cw.min(area.width);
            let ch = ch.min(area.height);
            if cw > 0 && ch > 0 {
                let x = area.x + (area.width - cw) / 2;
                let y = area.y + (area.height - ch) / 2;
                render(
                    frame,
                    &win.tree,
                    Rect::new(x, y, cw, ch),
                    focus,
                    false,
                    windows,
                    images,
                );
            }
            return;
        }
    }
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
        // The registry's icons: the decoded image in its slot, or the
        // placeholder when the `src` does not resolve (a display-only
        // marker, dim, like the hint bar).
        Tree::Image(_, src, width, height) => {
            let size = image_slot(*width, *height);
            match images.image(src, size) {
                Some(ImageData::Halfblocks { data, size }) => {
                    halfblocks::render(data, *size, area, frame.buffer_mut());
                }
                #[cfg(not(target_arch = "wasm32"))]
                Some(ImageData::Protocol(protocol)) => {
                    // A protocol image that no longer fits (a shrunken
                    // terminal) leaves the placeholder in its place.
                    if let Some(ph) = protocol.needs_placeholder(area) {
                        render_image_placeholder(frame, ph);
                    } else {
                        frame.render_widget(
                            ratatui_image::Image::new(protocol).allow_clipping(true),
                            area,
                        );
                    }
                }
                None => render_image_placeholder(frame, area),
            }
        }
        // A link: its label, underlined (the host's marker for a URL). The
        // host owns the opening, so it is not focusable.
        Tree::Link(_, label, _, style) => {
            frame.render_widget(
                Paragraph::new(label.clone())
                    .style(node_style(style).add_modifier(Modifier::UNDERLINED))
                    .alignment(Alignment::Center),
                area,
            );
        }
        // A box (a tile or a window): a bordered box around its children,
        // the focus ring on the border when a focused tile.
        Tree::Row(id, align, style, children) if is_box(style, has_registry) => {
            render_box(
                frame,
                id,
                *align,
                style,
                true,
                children,
                area,
                focus,
                has_registry,
                windows,
                images,
            );
        }
        Tree::Column(id, align, style, children) if is_box(style, has_registry) => {
            render_box(
                frame,
                id,
                *align,
                style,
                false,
                children,
                area,
                focus,
                has_registry,
                windows,
                images,
            );
        }
        Tree::Row(_, align, style, children) => {
            let placed = child_rects(area, *align, true, style, children, has_registry);
            for (i, child_area) in placed {
                render(
                    frame,
                    &children[i],
                    child_area,
                    focus,
                    has_registry,
                    windows,
                    images,
                );
            }
        }
        Tree::Column(_, align, style, children) => {
            let placed = child_rects(area, *align, false, style, children, has_registry);
            for (i, child_area) in placed {
                render(
                    frame,
                    &children[i],
                    child_area,
                    focus,
                    has_registry,
                    windows,
                    images,
                );
            }
        }
    }
}

/// A bordered box (a tile or a window): its border (a focus ring in the
/// accent when a focused tile, dim otherwise), its children laid out inside.
/// A terminal has no pixels for the component's background token, so the
/// interior is the terminal default; it is cleared before the children, so a
/// window covers the grid (or a lower window) behind it.
fn render_box(
    frame: &mut Frame,
    id: &Option<u32>,
    align: WAlign,
    style: &ContainerStyle,
    row: bool,
    children: &[Tree],
    area: Rect,
    focus: Option<u32>,
    has_registry: bool,
    windows: &[Window],
    images: &mut ImageState,
) {
    // A window is never the focus target (its content is), so its border is
    // dim; a focused tile takes the ring.
    let focused = *id == focus;
    let border_style = if focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style);
    frame.render_widget(Paragraph::new("").block(block), area);
    // The children inside the border, laid out per the container.
    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    // A terminal cell keeps whatever was last drawn in it, so a positioned
    // box (a window) must clear its interior to cover what is behind it.
    frame.render_widget(Clear, inner);
    let placed = child_rects(inner, align, row, style, children, has_registry);
    for (i, child_area) in placed {
        render(
            frame,
            &children[i],
            child_area,
            focus,
            has_registry,
            windows,
            images,
        );
    }
}

/// The marker for an image whose `src` does not resolve: a dim framed box,
/// the node's size. An image is display-only, so the marker is dim, like
/// the hint bar.
fn render_image_placeholder(frame: &mut Frame, area: Rect) {
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

/// The root's area within `area`: it fills the space along a stretched axis
/// (a `stretch: both` root fills the terminal) and is centered along the
/// rest; a non-stretched root is centered at its content size.
fn root_area(root: &Tree, area: Rect, has_registry: bool) -> Rect {
    let (w, h) = match root {
        Tree::Row(_, _, style, _) => {
            // A row: main = width, cross = height.
            let w = match style.stretch {
                WStretch::Main | WStretch::Both => area.width,
                _ => width_of(root, has_registry).min(area.width),
            };
            let h = match style.stretch {
                WStretch::Cross | WStretch::Both => area.height,
                _ => height_of(root, has_registry).min(area.height),
            };
            (w, h)
        }
        Tree::Column(_, _, style, _) => {
            // A column: main = height, cross = width.
            let w = match style.stretch {
                WStretch::Cross | WStretch::Both => area.width,
                _ => width_of(root, has_registry).min(area.width),
            };
            let h = match style.stretch {
                WStretch::Main | WStretch::Both => area.height,
                _ => height_of(root, has_registry).min(area.height),
            };
            (w, h)
        }
        // A leaf root (a single widget) is centered at its content size.
        _ => (
            width_of(root, has_registry).min(area.width),
            height_of(root, has_registry).min(area.height),
        ),
    };
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

/// Draw one full frame: a hint bar, and the app below it. The open windows
/// are the app's own positioned children (the component draws their chrome
/// and placement); the host seats each in its content slot. `has_registry`
/// marks a multi-window host (the tile chrome applies to its tiles whether
/// or not a window is open); the hint bar's windowing keys apply only while
/// a window is open. The ring's focus highlights its target wherever it sits
/// (the app or a seated window).
pub fn draw(
    frame: &mut Frame,
    app: &Tree,
    app_focus: Option<u32>,
    windows: &[Window],
    _active: usize,
    has_registry: bool,
    images: &mut ImageState,
) {
    let area = frame.area();
    let has_windows = !windows.is_empty();
    // The bar takes its wrapped rows, at most the terminal's.
    let rows = hint_rows(area.width, has_windows).min(area.height.max(1));
    let bars: [Rect; 2] =
        Layout::vertical([Constraint::Length(rows), Constraint::Min(0)]).areas(area);
    frame.render_widget(hint_bar(has_windows), bars[0]);

    // The app: its root fills the space along a stretched axis (the app's
    // root is `stretch: both`) and is centered along the rest.
    let root_area = root_area(app, bars[1], has_registry);
    if root_area.width > 0 && root_area.height > 0 {
        render(
            frame,
            app,
            root_area,
            app_focus,
            has_registry,
            windows,
            images,
        );
    }
}

// --------------------------------------------------------------------------
// Focus: a host-local ring over a surface's buttons, inputs, and (on a
// multi-window host) tiles, in pre-order.

/// One widget the focus ring can move to: its id, whether it is a text
/// input (printable keys type into it; enter/space does not activate it),
/// whether it is a tile (enter opens its entry, not a click), and the
/// surface it belongs to (0 = the app, else a window's handle) — events
/// activate the owning surface's component.
#[derive(Clone, Copy)]
pub struct FocusTarget {
    pub id: u32,
    pub is_input: bool,
    pub is_tile: bool,
    pub owner: u32,
}

/// The ring's targets in pre-order. A content slot (a container whose id is
/// a window's handle) inlines that window's focusables in its place, so one
/// ring spans the app and every open window.
fn focusables(
    node: &Tree,
    out: &mut Vec<FocusTarget>,
    has_registry: bool,
    windows: &[Window],
    owner: u32,
) {
    match node {
        // A button the component gave no id cannot be clicked, so it is
        // not focusable.
        Tree::Button(id, ..) => {
            if let Some(i) = id {
                out.push(FocusTarget {
                    id: *i,
                    is_input: false,
                    is_tile: false,
                    owner,
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
                    owner,
                });
            }
        }
        Tree::Row(id, _, style, children) | Tree::Column(id, _, style, children) => {
            // A content slot (id = a window's handle): inline that window's
            // focusables in its place, owned by the window.
            if let Some(i) = id {
                if let Some(win) = windows.iter().find(|w| w.handle == *i) {
                    focusables(&win.tree, out, false, windows, *i);
                    return;
                }
            }
            // A tile (`cursor: pointer`) is a focusable container; enter on
            // it synthesizes a double-click.
            if has_registry && is_tile(style) {
                if let Some(i) = id {
                    out.push(FocusTarget {
                        id: *i,
                        is_input: false,
                        is_tile: true,
                        owner,
                    });
                }
            }
            for child in children {
                focusables(child, out, has_registry, windows, owner);
            }
        }
        // Text and images are display-only; a link is not focusable either
        // (the host owns the opening).
        Tree::Text(..) | Tree::Image(..) | Tree::Link(..) => {}
    }
}

/// The content of the input with `id`, wherever it sits in the tree.
pub fn input_content(node: &Tree, id: u32) -> Option<&str> {
    match node {
        Tree::Input(iid, content, _, _) if *iid == Some(id) => Some(content),
        Tree::Row(_, _, _, children) | Tree::Column(_, _, _, children) => {
            children.iter().find_map(|c| input_content(c, id))
        }
        _ => None,
    }
}

/// Re-seat the app's ring on its (possibly rebuilt) tree: recompute the
/// targets (the open windows' focusables inlined at their content slots),
/// normalize the saved index, and the focused id.
pub fn reseat(tree: &Tree, has_registry: bool, windows: &[Window], surf: &mut Surface) {
    surf.targets.clear();
    focusables(tree, &mut surf.targets, has_registry, windows, 0);
    if surf.targets.is_empty() {
        surf.focus_idx = 0;
        surf.focus = None;
    } else {
        let n = surf.targets.len();
        surf.focus_idx %= n;
        surf.focus = surf.targets.get(surf.focus_idx).map(|t| t.id);
    }
}

/// The active surface's ring: always the app's. The ring is unified — it
/// spans the app and every open window (each window's focusables inlined at
/// its content slot) — so there is one ring, not one per surface.
pub fn active_surface<'a>(
    _store: &'a mut Store<Host>,
    app: &'a mut Surface,
    _active: usize,
) -> &'a mut Surface {
    app
}

/// Re-derive `active` from the focused target's `owner` without a full
/// re-seat: 0 for the app, else the owning window's 1-based z-order index.
/// A handle with no open window (it was closed) resolves to the app.
pub fn derive_active(store: &Store<Host>, app: &Surface, active: &mut usize) {
    let owner = app.targets.get(app.focus_idx).map(|t| t.owner).unwrap_or(0);
    *active = if owner == 0 {
        0
    } else {
        store
            .data()
            .windows
            .iter()
            .position(|w| w.handle == owner)
            .map(|i| i + 1)
            .unwrap_or(0)
    };
}

/// Move the ring's focus to the first focusable owned by the newest window
/// (after a tile opens it), so the new window is the active surface. Returns
/// whether it moved (a window with no focusables leaves the focus where it
/// was).
pub fn focus_newest_window(app: &mut Surface, windows: &[Window]) -> bool {
    let Some(newest) = windows.last() else {
        return false;
    };
    let handle = newest.handle;
    app.targets
        .iter()
        .position(|t| t.owner == handle)
        .map(|idx| {
            app.focus_idx = idx;
            app.focus = Some(app.targets[idx].id);
        })
        .is_some()
}

/// Re-seat the app's ring on its (possibly rebuilt) tree — the windows'
/// focusables inlined at their content slots — and re-derive `active` from
/// the focused target's owner.
pub fn reseat_active(
    store: &mut Store<Host>,
    tree: &Tree,
    has_registry: bool,
    active: &mut usize,
    app: &mut Surface,
) {
    {
        let windows = &store.data().windows;
        reseat(tree, has_registry, windows, app);
    }
    derive_active(store, app, active);
}

/// The active surface's component and store: the app, or the window's.
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

/// The active surface's tree: the app's, or the window's.
pub fn active_tree<'a>(store: &'a Store<Host>, tree: &'a Tree, active: usize) -> &'a Tree {
    if active == 0 {
        tree
    } else {
        &store.data().windows[active - 1].tree
    }
}

/// The active surface's ring target, as values (a key handler that also
/// needs the store cannot hold a borrow of the surface): the id, whether it
/// is a text input, whether it is a tile, and its owner (0 = the app, else a
/// window's handle). The ring is the app's (unified), so the target is read
/// from it regardless of `active`.
pub fn active_target(
    _store: &mut Store<Host>,
    app: &mut Surface,
    _active: usize,
) -> (u32, bool, bool, u32) {
    app.targets
        .get(app.focus_idx)
        .map(|t| (t.id, t.is_input, t.is_tile, t.owner))
        .unwrap_or((0, false, false, 0))
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
/// A cell's foreground as an SGR code: the sixteen colors by their codes,
/// truecolor by `38;2`, and `None` for a reset (the terminal default) and
/// indexed (a plain text dump has no palette to index into).
fn sgr_fg(c: Color) -> Option<String> {
    Some(match c {
        Color::Black => "\x1b[30m".into(),
        Color::Red => "\x1b[31m".into(),
        Color::Green => "\x1b[32m".into(),
        Color::Yellow => "\x1b[33m".into(),
        Color::Blue => "\x1b[34m".into(),
        Color::Magenta => "\x1b[35m".into(),
        Color::Cyan => "\x1b[36m".into(),
        Color::Gray => "\x1b[37m".into(),
        Color::DarkGray => "\x1b[90m".into(),
        Color::LightRed => "\x1b[91m".into(),
        Color::LightGreen => "\x1b[92m".into(),
        Color::LightYellow => "\x1b[93m".into(),
        Color::LightBlue => "\x1b[94m".into(),
        Color::LightMagenta => "\x1b[95m".into(),
        Color::LightCyan => "\x1b[96m".into(),
        Color::White => "\x1b[97m".into(),
        Color::Reset | Color::Indexed(_) => return None,
        Color::Rgb(r, g, b) => format!("\x1b[38;2;{r};{g};{b}m"),
    })
}

/// A cell's background as an SGR code: the halfblocks cells carry a
/// truecolor background (the lower half of each cell), and most cells have
/// none (`Reset`), so `None`.
fn sgr_bg(c: Color) -> Option<String> {
    Some(match c {
        Color::Black => "\x1b[40m".into(),
        Color::Red => "\x1b[41m".into(),
        Color::Green => "\x1b[42m".into(),
        Color::Yellow => "\x1b[43m".into(),
        Color::Blue => "\x1b[44m".into(),
        Color::Magenta => "\x1b[45m".into(),
        Color::Cyan => "\x1b[46m".into(),
        Color::Gray => "\x1b[47m".into(),
        Color::DarkGray => "\x1b[100m".into(),
        Color::LightRed => "\x1b[101m".into(),
        Color::LightGreen => "\x1b[102m".into(),
        Color::LightYellow => "\x1b[103m".into(),
        Color::LightBlue => "\x1b[104m".into(),
        Color::LightMagenta => "\x1b[105m".into(),
        Color::LightCyan => "\x1b[106m".into(),
        Color::White => "\x1b[107m".into(),
        Color::Reset | Color::Indexed(_) => return None,
        Color::Rgb(r, g, b) => format!("\x1b[48;2;{r};{g};{b}m"),
    })
}

/// The rightmost and bottommost edge (in cells) of any positioned
/// descendant: the dump buffer must fit the app's windows, which sit at
/// their (pixel) positions and sizes, scaled to cells.
fn positioned_extent(node: &Tree) -> (u16, u16) {
    fn walk(node: &Tree, max_x: &mut u16, max_y: &mut u16) {
        match node {
            Tree::Row(_, _, s, children) | Tree::Column(_, _, s, children) => {
                if s.positioned {
                    let x = (s.x / CELL_W) as u16;
                    let y = (s.y / CELL_H) as u16;
                    let w = if s.w > 0 {
                        (s.w / CELL_W).max(1) as u16
                    } else {
                        0
                    };
                    let h = if s.h > 0 {
                        (s.h / CELL_H).max(1) as u16
                    } else {
                        0
                    };
                    *max_x = (*max_x).max(x.saturating_add(w));
                    *max_y = (*max_y).max(y.saturating_add(h));
                }
                for c in children {
                    walk(c, max_x, max_y);
                }
            }
            _ => {}
        }
    }
    let (mut max_x, mut max_y) = (0u16, 0u16);
    walk(node, &mut max_x, &mut max_y);
    (max_x, max_y)
}

/// One full frame as text: rendered into a memory buffer (`TestBackend`)
/// and serialized with ANSI SGR codes for the styled cells, trailing
/// spaces trimmed per row — the same bytes the `--dump` flag prints.
///
/// The buffer fits the app whole (floored at half the hint's length, so the
/// bar wraps to at most two rows) and the windows' extent, or `width`
/// columns when given (the hint bar wraps, as on a narrow terminal). The
/// windows are the app's positioned children, so their extent comes from the
/// app's tree, not from the host. A dump has no pointer to raise a window:
/// the ring seats on the app's first focusable, and the windows' content is
/// inlined at their content slots.
///
/// This is the byte-parity surface between the native and the browser
/// host: a pure function of the tree, the windows, the width, whether a
/// registry is present, and the image state (its mode and the decoded
/// images, which are pure functions of the asset bytes), so the two must
/// render identically.
pub fn dump_frame(
    tree: &Tree,
    windows: &[Window],
    width: Option<u16>,
    has_registry: bool,
    images: &mut ImageState,
) -> String {
    let has_windows = !windows.is_empty();
    // The buffer fits the app whole, floored at half the hint's length, so
    // the bar wraps to at most two rows; it also widens and grows until the
    // app's positioned windows fit.
    let (pos_x, pos_y) = positioned_extent(tree);
    let w = width.unwrap_or_else(|| {
        let base = width_of(tree, has_registry)
            .max((hint_text(has_windows).chars().count() / 2) as u16)
            + 2;
        base.max(pos_x)
    });
    // The bar's wrapped rows, then the app with one margin row above and
    // below, and the windows' extent.
    let hint = hint_rows(w, has_windows);
    let area_h = height_of(tree, has_registry).max(pos_y).saturating_add(2);
    let h = area_h + hint;
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("failed to create terminal");
    // The ring seats on the app's first focusable (a tile); the first target
    // is an app one, so the app is active.
    let mut app_surf = Surface::default();
    reseat(tree, has_registry, windows, &mut app_surf);
    let completed = terminal
        .draw(|f| draw(f, tree, app_surf.focus, windows, 0, has_registry, images))
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
        // The SGR state persists across a row's cells, so a cell with no
        // background clears the one the previous cell set — a stale
        // halfblocks background would otherwise paint the row's tail.
        let mut bg_set = false;
        for x in 0..area.width {
            let (sym, fg, bg, modifier) = match buf.cell((x, y)) {
                Some(c) => (c.symbol(), c.fg, c.bg, c.modifier),
                None => (" ", Color::Reset, Color::Reset, Modifier::empty()),
            };
            if let Some(code) = sgr_fg(fg) {
                line.push_str(&code);
                styled = true;
            }
            let bg_code = sgr_bg(bg);
            if let Some(code) = &bg_code {
                line.push_str(code);
                styled = true;
            } else if bg_set {
                line.push_str("\x1b[49m");
                styled = true;
            }
            bg_set = bg_code.is_some();
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
    //! hint.

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
        let tree = Tree::Column(None, WAlign::Center, ContainerStyle::default(), Vec::new());
        terminal
            .draw(|f| draw(f, &tree, None, &[], 0, false, &mut ImageState::default()))
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
        let tree = Tree::Column(None, WAlign::Center, ContainerStyle::default(), Vec::new());
        terminal
            .draw(|f| draw(f, &tree, None, &[], 0, false, &mut ImageState::default()))
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
}
