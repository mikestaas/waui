//! The platform-neutral core of the iced host: the wasmtime `Host` (clock,
//! refresh request, registry, and panels), the tree-to-iced renderer, and
//! the `--dump` render.
//!
//! The renderer and the render are pure functions of the tree, the panels,
//! the canvas size, and the registry, so they are shared by the native
//! binary (a real window, via winit) and the browser module (a canvas, via
//! iced's Web runtime). What differs per platform is only the engine
//! (Cranelift natively, the Pulley interpreter in the browser), the clock,
//! and how a component is loaded. [`render_png`] is the shared byte-parity
//! surface between the two.
//!
//! A windowed host for waui components, rendered with [iced](https://iced.rs).
//!
//! The host bootstraps from `view()` (a full snapshot), then — on every
//! event it forwards to the component — checks the component's `changes()`
//! diff: a content-only diff is applied in place, a structural one
//! re-queries `view()` (the diff cannot carry id-less nodes, so the
//! snapshot is the source of truth), and a rebuild follows either; a tick
//! that changes nothing skips the rebuild.
//!
//! Time-based refresh is component-requested: `schedule` sets the period,
//! and because iced's `time::every` takes a fixed period, the host keeps a
//! coarse 50ms clock and wakes the component only when that period has
//! elapsed (0 = never).
//!
//! `--click <id>` injects clicks on the first tick after startup, so the
//! windowed host can be driven and screenshotted without a pointer; the
//! click takes the same path as a pointer event.
//!
//! Text inputs render as editable fields; each edit forwards the field's
//! whole buffer through `on-text`, and the component's answer (it is the
//! source of truth for the buffer) lands through the diff.
//!
//! `--entry <name=path>` registers an entry the host can run; a non-empty
//! registry lets this host run multi-component apps. An opened entry runs
//! in its own store, and its tree is seated in a slot of the launching
//! component's tree (the slot's id is the entry's handle): the launching
//! component draws the window's chrome and geometry with the contract's
//! tokens and drives them through the pointer channel, and this host
//! forwards presses and held-button movement (`on-pointer-down` /
//! `on-pointer-move` / `on-pointer-up`) and double-activations
//! (`on-double-click`).
//!
//! This host has no keyboard channel (iced has no raw key event), so it
//! never emits `on-key`; windowing a component binds to keys (a close on
//! Escape) is available only on hosts with a keyboard (the TUI, the web
//! page).
//!
//! `--dump <file.png>` renders one frame to a PNG and exits, headless: the
//! window's own render path with the surface replaced by iced's software
//! (`tiny-skia`) backend, so no window, display, or GPU is needed and the
//! check runs in CI. `--click`, `--double-click`, and `--close` apply
//! before the frame, as in the window. `--now <base> <step>` fakes the
//! host clock (each call advances `step` ms), so a `--dump` frame that
//! reads the clock is deterministic.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

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
    build_tree, waui, App, AppImports, Entry, EntrySpec, Error, Retained, Tree, ViewportSize,
};

// The contract's style tokens, aliased to avoid clashing with `iced::Color`.
use host_shared::waui::ui::widget::{
    Align as WAlign, Background as WBackground, Border as WBorder, Color as WColor,
    Cursor as WCursor, Radius as WRadius, Shadow as WShadow, Size as WSize, Stretch as WStretch,
};
use host_shared::ContainerStyle;

// The `Widget` trait and the layout types for the panel's custom widget.
use iced_runtime::core as iced_core;

pub mod theme;
#[cfg(target_arch = "wasm32")]
pub mod web;

/// The component's refresh request, shared between the world-import impl
/// (the writer) and the tick handler (the reader): the period in ms, 0 =
/// no periodic wake, and the last wake.
#[derive(Default)]
pub struct Wake {
    pub rate_ms: u64,
    pub last_wake_ms: u64,
}

/// An open panel: the entry's component running in its own store (the
/// world's imports take the store's data, not a store handle, so a panel
/// gets its own) and its retained tree. The panel has no geometry of its
/// own: the opening component draws the window's chrome and the host seats
/// the panel's tree in the component's content slot (see [`content_rect`]).
pub struct Panel {
    pub handle: u32,
    pub store: Store<Host>,
    pub app: App,
    pub retained: Retained,
    pub tree: Tree,
    pub wake: Arc<Mutex<Wake>>,
    /// Whether the panel's component was driven (a click, a text edit, or a
    /// tick) since the last `changes` call.
    pub pending: bool,
}

/// The host's implementation of the world's imports. The world's imports
/// take the store's data, not a store handle, so a panel this host opens
/// carries its own store and its own copy of the panels it opens.
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
    /// The registry (`--entry`): a non-empty one lets the host run
    /// multi-component apps.
    pub entries: Vec<EntrySpec>,
    /// The image assets the page provided (the browser; the native host
    /// resolves a `src` against the filesystem instead): path to bytes, for
    /// the registry's icons.
    pub assets: BTreeMap<String, Vec<u8>>,
    /// The open panels, in the order they were opened.
    pub panels: Vec<Panel>,
    /// The next panel handle. The base is above the component's own id
    /// space (a component's fixed ids stay low; its content slot takes the
    /// handle as its id, so the two spaces never collide).
    pub next_handle: u32,
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

// The first panel handle: above the component's own ids (its fixed ids and
// its tiles stay in the low range; its content slot takes a handle as its
// id, so the host's handles and the component's ids never collide).
pub const HANDLE_BASE: u32 = 10_000;

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
            assets: BTreeMap::new(),
            panels: Vec::new(),
            next_handle: HANDLE_BASE,
        }
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

    /// The store's data for a panel this host opens: the same engine,
    /// linker, and clock, but an empty registry (a panel runs a demo, not a
    /// multi-component app, so the tile rule never fires in it) and its own
    /// panel list.
    fn window_host(&self) -> Host {
        Host {
            wake: Arc::new(Mutex::new(Wake::default())),
            fake_now: self.fake_now,
            fake_calls: 0,
            engine: self.engine.clone(),
            linker: self.linker.clone(),
            entries: Vec::new(),
            // A panel's images (if a demo ever has them) resolve against the
            // same page assets as the app.
            assets: self.assets.clone(),
            panels: Vec::new(),
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
    // The host's viewport: its canvas, in the host's units (pixels). A
    // component's viewport-dependent math (a drag clamp) uses it.
    fn viewport(&mut self) -> ViewportSize {
        let size = canvas_size(!self.entries.is_empty());
        ViewportSize {
            w: size.width as u32,
            h: size.height as u32,
        }
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
        self.panels.push(Panel {
            handle,
            store,
            app,
            retained,
            tree,
            wake,
            pending: false,
        });
        Ok(handle)
    }
    /// Drop the panel with `handle` (the component closed it): the host frees
    /// the entry's store. Idempotent — an unknown handle is a no-op.
    fn close_entry(&mut self, handle: u32) {
        self.panels.retain(|p| p.handle != handle);
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

pub struct State {
    pub app: App,
    pub store: Store<Host>,
    /// The retained node list, as the diff protocol maintains it.
    pub retained: Retained,
    pub tree: Tree,
    pub wake: Arc<Mutex<Wake>>,
    /// Events from `--click`, `--double-click`, and `--close`, in argument
    /// order, applied on the first tick after startup.
    pub pending: Vec<Injected>,
    /// The pointer's last known position, in canvas coordinates (a
    /// full-canvas area reports it on every move; `on_press` carries none).
    pub pointer_pos: (f32, f32),
    /// The press in progress: the surface and node the pointer was pressed
    /// over, and the press position (a release within a few pixels of it is
    /// a click; the held movement drives `on-pointer-move`).
    pub press: Option<(Surface, u32, f32, f32)>,
}

/// Which surface an event landed on: the main app, or a panel (by handle).
/// The main app's and a panel's trees are separate components with
/// separate id spaces, so a node id alone does not name its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Main,
    Panel(u32),
}

#[derive(Debug, Clone)]
pub enum Message {
    /// A button with `id` was activated (a press released over it).
    Click(Surface, u32),
    /// The input with `id` now holds `text`: the field's whole buffer,
    /// after the user's latest edit.
    Text(Surface, u32, String),
    /// A node of the main app was double-activated (a native double-click,
    /// or `--double-click`).
    DoubleClick(u32),
    /// The pointer was pressed over the node with `id`.
    PointerDown(Surface, u32),
    /// The pointer was released.
    PointerUp,
    /// The pointer moved to `(x, y)`, in canvas coordinates.
    PointerMove(f32, f32),
    Tick,
}

/// An event injected from the command line, in argument order: the same
/// actions the pointer drives, for a host that can be driven without one.
#[derive(Clone)]
pub enum Injected {
    Click(u32),
    DoubleClick(u32),
    Close(u32),
}

// The default window size, and the `--dump` canvas, for a host that runs
// no entries.
pub const WINDOW_WIDTH: f32 = 360.0;
pub const WINDOW_HEIGHT: f32 = 340.0;

// The app canvas, for a host that runs entries (a multi-component app):
// the window and the dump render the same size, so a `--dump` frame is the
// window's frame.
pub const APP_WIDTH: f32 = 960.0;
pub const APP_HEIGHT: f32 = 600.0;

/// The window and dump canvas: the app's when the host runs entries, else
/// the standalone default (the existing goldens' size).
pub fn canvas_size(has_registry: bool) -> iced::Size {
    if has_registry {
        iced::Size::new(APP_WIDTH, APP_HEIGHT)
    } else {
        iced::Size::new(WINDOW_WIDTH, WINDOW_HEIGHT)
    }
}

/// A container the host makes double-clickable: one the component marked
/// `cursor: pointer` (the WIT's double-clickable marker, replacing the old
/// id-range hack). The rule applies only when the host runs entries (a
/// non-empty registry): a standalone app's containers are not tiles.
fn is_tile(style: &ContainerStyle) -> bool {
    style.cursor == WCursor::Pointer
}

/// Whether a node is a positioned container (a window).
fn is_positioned(node: &Tree) -> bool {
    matches!(
        node,
        Tree::Row(_, _, s, _) | Tree::Column(_, _, s, _) if s.positioned
    )
}

/// A node's stacking order (a positioned container's `z`; `0` for the rest).
fn z_of(node: &Tree) -> u32 {
    match node {
        Tree::Row(_, _, s, _) | Tree::Column(_, _, s, _) => s.z,
        _ => 0,
    }
}

/// Instantiate the component and build the initial retained node list and
/// widget tree from its `view()` snapshot.
pub fn new_state(
    engine: &Engine,
    component: &Component,
    entries: Vec<EntrySpec>,
    injected: Vec<Injected>,
    fake_now: Option<(u64, u64)>,
    assets: BTreeMap<String, Vec<u8>>,
) -> State {
    let mut linker = Linker::new(engine);
    App::add_to_linker::<_, HasSelf<_>>(&mut linker, |host| host)
        .expect("failed to link world imports");

    let wake = Arc::new(Mutex::new(Wake::default()));
    let mut store = Store::new(
        engine,
        Host {
            wake: wake.clone(),
            fake_now,
            fake_calls: 0,
            engine: engine.clone(),
            linker: linker.clone(),
            entries,
            assets,
            panels: Vec::new(),
            next_handle: HANDLE_BASE,
        },
    );
    let app =
        App::instantiate(&mut store, component, &linker).expect("failed to instantiate component");

    let retained = Retained::from_view(
        app.call_view(&mut store)
            .expect("failed to get initial view"),
    );
    let tree = build_tree(retained.nodes());

    State {
        app,
        store,
        retained,
        tree,
        wake,
        pending: injected,
        pointer_pos: (0.0, 0.0),
        press: None,
    }
}

/// Apply the `--click`, `--double-click`, and `--close` injections, in
/// argument order, and the components' resulting diffs, rebuilding the
/// trees that changed. The windowed host does this on the first tick; the
/// dump path does it before rendering. The startup snapshot precedes it
/// (a multi-component app fetches its registry on first view, so a
/// double-click looks up the registry that view fetched).
pub fn apply_pending(state: &mut State) {
    if state.pending.is_empty() {
        return;
    }
    for inj in state.pending.drain(..) {
        match inj {
            Injected::Click(id) => {
                state
                    .app
                    .call_on_click(&mut state.store, id)
                    .expect("on-click() failed");
            }
            Injected::DoubleClick(id) => {
                state
                    .app
                    .call_on_double_click(&mut state.store, id)
                    .expect("on-double-click() failed");
            }
            Injected::Close(handle) => {
                state.store.data_mut().panels.retain(|p| p.handle != handle);
                state
                    .app
                    .call_on_window_close(&mut state.store, handle)
                    .expect("on-window-close() failed");
            }
        }
    }
    diff_main(state);
    diff_panels(state);
}

/// The main app's refresh: rebuild its tree when the component reports a
/// change (an empty diff skips the rebuild).
///
/// A content-only diff is applied in place; a structural diff re-queries
/// `view`. The split follows the WIT identity note: a diff cannot carry
/// id-less nodes, so a structural change (which may add or move one — a
/// window's title-bar label) must come from the snapshot, which is always
/// the full tree (the WIT's re-query fallback).
fn diff_main(state: &mut State) {
    let mutations = state
        .app
        .call_changes(&mut state.store)
        .expect("changes() failed");
    if mutations.is_empty() {
        return;
    }
    if state.retained.is_content_only(&mutations) {
        state.retained.apply(&mutations);
    } else {
        let nodes = state
            .app
            .call_view(&mut state.store)
            .expect("view() failed");
        state.retained = Retained::from_view(nodes);
    }
    state.tree = build_tree(state.retained.nodes());
}

/// The panels' refresh: rebuild each panel that was driven, with the same
/// content-only/structural split as [`diff_main`].
fn diff_panels(state: &mut State) {
    let driven = state
        .store
        .data()
        .panels
        .iter()
        .map(|p| p.pending)
        .collect::<Vec<_>>();
    if driven.iter().any(|&d| d) {
        let panels = &mut state.store.data_mut().panels;
        for (p, &d) in panels.iter_mut().zip(&driven) {
            if !d {
                continue;
            }
            p.pending = false;
            let mutations = p.app.call_changes(&mut p.store).expect("changes() failed");
            if mutations.is_empty() {
                continue;
            }
            if p.retained.is_content_only(&mutations) {
                p.retained.apply(&mutations);
            } else {
                let nodes = p.app.call_view(&mut p.store).expect("view() failed");
                p.retained = Retained::from_view(nodes);
            }
            p.tree = build_tree(p.retained.nodes());
        }
    }
}

/// The dump must render identically on every machine. Iced's font database
/// holds the system fonts too, and the bundled Fira Sans lacks the todo's
/// checkbox and delete symbols, so those glyphs would fall back to a system
/// font that differs per platform. Prune the database to the bundled fonts
/// and load the symbol font that covers the glyphs Fira lacks.
///
/// The guard pins it once per process: natively the dump is a one-shot,
/// and in the browser the live window and the dumps share the global
/// database, so a re-pin would duplicate the symbol font. On wasm32 the
/// prune is a no-op (fontdb loads no system fonts there); the window keeps
/// the system fonts natively.
fn pin_dump_fonts() {
    static PINNED: Once = Once::new();
    PINNED.call_once(|| {
        let mut system = iced_graphics::text::font_system()
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let db = system.raw().db_mut();
        // Keep only the bundled fonts (loaded from bytes); drop the system
        // fonts (loaded from files).
        let drop: Vec<fontdb::ID> = db
            .faces()
            .filter(|face| !matches!(face.source, fontdb::Source::Binary(_)))
            .map(|face| face.id)
            .collect();
        for id in drop {
            db.remove_face(id);
        }
        db.load_font_source(fontdb::Source::Binary(std::sync::Arc::new(
            include_bytes!("../fonts/waui-symbols.ttf").to_vec(),
        )));
    });
}

/// The headless renderer: the window's own `Renderer` with the surface
/// replaced by iced's software (`tiny-skia`) backend, so no window,
/// display, or GPU is needed.
fn create_headless_renderer() -> Option<iced::Renderer> {
    use iced_core::renderer::Headless;

    // The `Headless` `new`, fully qualified: the single-backend
    // `iced_tiny_skia::Renderer` (the wasm32 build) also has an inherent
    // 2-argument `new`, which would otherwise shadow it. The trait's `new`
    // is an `async fn` that resolves to `Option<Self>` on every backend,
    // so one `block_on` drives it.
    let renderer = <iced::Renderer as Headless>::new(
        iced::Font::with_name("Fira Sans"),
        16.0.into(),
        Some("tiny-skia"),
    );
    #[cfg(not(target_arch = "wasm32"))]
    {
        // The executor blocks on the renderer's creation (synchronous with
        // the tiny-skia backend; no GPU is touched).
        use iced::executor::Executor;
        let executor = iced::executor::Default::new().expect("failed to create an executor");
        executor.block_on(renderer)
    }
    #[cfg(target_arch = "wasm32")]
    {
        // The `Executor` trait has no `block_on` on wasm32 (it would need
        // an event loop); the tiny-skia backend is a synchronous
        // constructor, so a plain one drives it to completion.
        futures::executor::block_on(renderer)
    }
}

/// Render the current view to a PNG, headless: no window, no display, no
/// GPU.
///
/// Text renders with the bundled Fira Sans (the `fira-sans` feature)
/// rather than a system font: a CI machine may have none, and the dump
/// must be stable to be a useful check. `pin_dump_fonts` prunes the font
/// database so the few glyphs Fira lacks also come from bundled bytes;
/// the window keeps the system fonts.
///
/// The render is the window's own path with the surface replaced by iced's
/// `Headless` renderer: the same `UserInterface` layout and draw, then
/// `screenshot` rasterizes the recorded layers into a pixel buffer. The
/// buffer is unpremultiplied RGBA at the canvas's logical size, 1x.
pub fn render_png(state: &State) -> (u32, u32, Vec<u8>) {
    use iced_runtime::core::renderer::Headless;

    pin_dump_fonts();

    let mut renderer = create_headless_renderer().expect("no headless renderer");

    let size = canvas_size(!state.store.data().entries.is_empty());

    // Layout (text shaping runs through the renderer) and draw, the same
    // path the window's event loop uses.
    let mut ui = iced_runtime::user_interface::UserInterface::build(
        view(state),
        size,
        iced_runtime::user_interface::Cache::new(),
        &mut renderer,
    );
    let style = theme::style();
    ui.draw(
        &mut renderer,
        &theme::theme(),
        &iced_runtime::core::renderer::Style {
            text_color: style.text_color,
        },
        iced::mouse::Cursor::Unavailable,
    );

    let width = size.width as u32;
    let height = size.height as u32;
    let rgba = renderer.screenshot(iced::Size::new(width, height), 1.0, style.background_color);

    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut frame = encoder
        .write_header()
        .expect("failed to write the PNG header");
    frame
        .write_image_data(&rgba)
        .expect("failed to write the PNG");
    drop(frame);
    (width, height, png)
}

/// Drive the component of `surface` (the main app, or the panel with
/// `handle`); a panel it drives is marked for a diff.
fn drive_app(state: &mut State, surface: Surface, drive: impl Fn(&mut App, &mut Store<Host>)) {
    match surface {
        Surface::Main => drive(&mut state.app, &mut state.store),
        Surface::Panel(handle) => {
            let panels = &mut state.store.data_mut().panels;
            if let Some(p) = panels.iter_mut().find(|p| p.handle == handle) {
                drive(&mut p.app, &mut p.store);
                p.pending = true;
            }
        }
    }
}

pub fn update(state: &mut State, message: Message) {
    // `--click`, `--double-click`, and `--close` injections take the same
    // path as the pointer: applied on the first tick.
    apply_pending(state);

    // An event always wakes the component it landed on. A tick wakes a
    // component only when its requested period has elapsed (0 = never);
    // iced's own clock is coarse (50ms), so the gate, not the timer, sets
    // the cadence.
    let mut main_woke = false;
    match message {
        Message::Click(surface, id) => {
            drive_app(state, surface, |app, store| {
                app.call_on_click(store, id).expect("on-click() failed");
            });
            main_woke |= surface == Surface::Main;
            // A button captures its release, so the canvas never sees it:
            // end any tracked press here, or the component's drag state
            // outlives the button.
            if let Some((s, _, _, _)) = state.press.take() {
                let (x, y) = state.pointer_pos;
                drive_app(state, s, |app, store| {
                    app.call_on_pointer_up(store, x as u32, y as u32)
                        .expect("on-pointer-up() failed");
                });
                main_woke |= s == Surface::Main;
            }
        }
        // An edit always wakes the component: it re-derives the field's
        // content from the buffer it stores, and the diff carries it.
        Message::Text(surface, id, text) => {
            drive_app(state, surface, |app, store| {
                app.call_on_text(store, id, &text)
                    .expect("on-text() failed");
            });
            main_woke |= surface == Surface::Main;
        }
        // A node of the main app was double-activated: the app opens the
        // entry's panel through the open-entry import.
        Message::DoubleClick(id) => {
            state
                .app
                .call_on_double_click(&mut state.store, id)
                .expect("on-double-click() failed");
            main_woke = true;
        }
        // A press over a node: report it (the component raises a window and
        // starts a drag or resize) and track it for the release.
        Message::PointerDown(surface, id) => {
            let (x, y) = state.pointer_pos;
            state.press = Some((surface, id, x, y));
            drive_app(state, surface, |app, store| {
                app.call_on_pointer_down(store, id, x as u32, y as u32)
                    .expect("on-pointer-down() failed");
            });
            main_woke |= surface == Surface::Main;
        }
        // A held-button movement: the pointer's position, to the surface
        // that owns the press (a window drag or resize).
        Message::PointerMove(x, y) => {
            state.pointer_pos = (x, y);
            if let Some((surface, _, _, _)) = state.press {
                drive_app(state, surface, |app, store| {
                    app.call_on_pointer_move(store, x as u32, y as u32)
                        .expect("on-pointer-move() failed");
                });
                main_woke |= surface == Surface::Main;
            }
        }
        // A release: end the tracked press; a press released where it
        // started (within a few pixels) is a click, a drag is not.
        Message::PointerUp => {
            if let Some((surface, id, px, py)) = state.press.take() {
                let (x, y) = state.pointer_pos;
                drive_app(state, surface, |app, store| {
                    app.call_on_pointer_up(store, x as u32, y as u32)
                        .expect("on-pointer-up() failed");
                });
                main_woke |= surface == Surface::Main;
                if (x - px).abs() + (y - py).abs() <= 4.0 {
                    drive_app(state, surface, |app, store| {
                        app.call_on_click(store, id).expect("on-click() failed");
                    });
                    main_woke |= surface == Surface::Main;
                }
            }
        }
        Message::Tick => {
            let mut w = state.wake.lock().unwrap();
            let due = w.rate_ms > 0 && now_ms() >= w.last_wake_ms + w.rate_ms;
            if due {
                w.last_wake_ms = now_ms();
            }
            if due {
                state
                    .app
                    .call_on_tick(&mut state.store)
                    .expect("on-tick() failed");
            }
            main_woke |= due;
            // Each panel: its component's own requested period (a running
            // stopwatch), gated the same way.
            for p in state.store.data_mut().panels.iter_mut() {
                let mut w = p.wake.lock().unwrap();
                let due = w.rate_ms > 0 && now_ms() >= w.last_wake_ms + w.rate_ms;
                if due {
                    w.last_wake_ms = now_ms();
                }
                if due {
                    p.app.call_on_tick(&mut p.store).expect("on-tick() failed");
                    p.pending = true;
                }
            }
        }
    }
    if main_woke {
        diff_main(state);
    }
    diff_panels(state);
}

/// Whether the main app's tree has a content slot for `handle` (a container
/// the app gave the id): the panel seats there when it does.
fn slot_in_tree(node: &Tree, handle: u32) -> bool {
    match node {
        Tree::Row(id, _, _, children) | Tree::Column(id, _, _, children) => {
            *id == Some(handle) || children.iter().any(|c| slot_in_tree(c, handle))
        }
        _ => false,
    }
}

pub fn view(state: &State) -> iced::Element<'_, Message> {
    use iced::alignment;
    use iced::widget;
    use iced::Length;
    let host = state.store.data();
    let has_registry = !host.entries.is_empty();
    // Fill the canvas and center the app. A standalone app (no registry)
    // keeps the page's 2rem padding: a cosmetic margin, and the size the
    // committed goldens were drawn at. A multi-component app has no padding:
    // its canvas is the component's drawing area whole, so `viewport` (the
    // canvas) is what the component's clamping math expects.
    let mut main = widget::container(map_tree(
        &state.tree,
        Surface::Main,
        has_registry,
        &host.panels,
        &host.assets,
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(alignment::Horizontal::Center)
    .align_y(alignment::Vertical::Center);
    if !has_registry {
        main = main.padding(32.0);
    }
    // A full-canvas area: the pointer's position on every move (a press
    // carries none), and the release that no node below reports (a button
    // captures its own).
    let main = widget::MouseArea::new(main)
        .on_move(|p| Message::PointerMove(p.x, p.y))
        .on_release(Message::PointerUp);
    let mut layers = vec![main.into()];
    // A panel the app did not seat (a host-chrome component, which leaves
    // the window's chrome to the host): centered, without chrome.
    for panel in &host.panels {
        if !slot_in_tree(&state.tree, panel.handle) {
            let content = widget::container(map_tree(
                &panel.tree,
                Surface::Panel(panel.handle),
                false,
                &EMPTY_PANELS,
                &host.assets,
            ))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(alignment::Horizontal::Center)
            .align_y(alignment::Vertical::Center);
            layers.push(widget::stack(vec![content.into()]).into());
        }
    }
    // A host with no panels is the standalone view of today: the stack with
    // its single layer draws the layer alone.
    widget::stack(layers).into()
}

/// A positioned container at a fixed position on the canvas: its content
/// laid out at `(x, y)`, clamped to the canvas. The stack sizes to its base
/// layer (the app), so the positioned container's size only places it;
/// everything else delegates to the content.
struct PanelWidget<'a, Message> {
    content: iced::Element<'a, Message>,
    x: f32,
    y: f32,
}

impl<'a, Message> iced_core::Widget<Message, iced::Theme, iced::Renderer>
    for PanelWidget<'a, Message>
where
    Message: 'a,
{
    fn size(&self) -> iced_core::Size<iced_core::Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> iced_core::Size<iced_core::Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut iced_core::widget::Tree,
        renderer: &iced::Renderer,
        limits: &iced_core::layout::Limits,
    ) -> iced_core::layout::Node {
        let mut node = self.content.as_widget_mut().layout(tree, renderer, limits);
        // Place the panel at its cascade position, clamped to the canvas
        // (a panel wider than the canvas lands at the left edge).
        let max = limits.max();
        let x = self
            .x
            .max(0.0)
            .min((max.width - node.size().width).max(0.0));
        let y = self
            .y
            .max(0.0)
            .min((max.height - node.size().height).max(0.0));
        node.move_to_mut(iced_core::Point::new(x, y));
        node
    }

    fn draw(
        &self,
        tree: &iced_core::widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &iced_core::renderer::Style,
        layout: iced_core::Layout<'_>,
        cursor: iced_core::mouse::Cursor,
        viewport: &iced_core::Rectangle,
    ) {
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn tag(&self) -> iced_core::widget::tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> iced_core::widget::tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<iced_core::widget::Tree> {
        self.content.as_widget().children()
    }

    fn diff(&self, tree: &mut iced_core::widget::Tree) {
        self.content.as_widget().diff(tree);
    }

    fn operate(
        &mut self,
        tree: &mut iced_core::widget::Tree,
        layout: iced_core::Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn iced_core::widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut iced_core::widget::Tree,
        event: &iced_core::Event,
        layout: iced_core::Layout<'_>,
        cursor: iced_core::mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn iced_core::Clipboard,
        shell: &mut iced_core::Shell<'_, Message>,
        viewport: &iced_core::Rectangle,
    ) {
        self.content.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &iced_core::widget::Tree,
        layout: iced_core::Layout<'_>,
        cursor: iced_core::mouse::Cursor,
        viewport: &iced_core::Rectangle,
        renderer: &iced::Renderer,
    ) -> iced_core::mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut iced_core::widget::Tree,
        layout: iced_core::Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &iced_core::Rectangle,
        translation: iced_core::Vector,
    ) -> Option<iced_core::overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

/// iced's `time::every` takes a fixed period, so the host keeps a coarse
/// 50ms clock; the component's requested period gates the wake (see the
/// tick arm of `update`).
pub fn subscription(_state: &State) -> iced::Subscription<Message> {
    iced::time::every(Duration::from_millis(50)).map(|_| Message::Tick)
}

/// A semantic color in this host's palette (the web page's Tokyo Night
/// values, from `theme.rs`). `neutral` maps to nothing: iced's own theme
/// color is the default foreground.
fn iced_color(c: WColor) -> Option<iced::Color> {
    Some(match c {
        WColor::Neutral => return None,
        WColor::Accent => iced::Color::from_rgb8(0x7a, 0xa2, 0xf7),
        WColor::Positive => iced::Color::from_rgb8(0x9e, 0xce, 0x6a),
        WColor::Negative => iced::Color::from_rgb8(0xf7, 0x76, 0x8e),
        WColor::Warning => iced::Color::from_rgb8(0xe0, 0xaf, 0x68),
        WColor::Dim => iced::Color::from_rgb8(0x56, 0x5f, 0x89),
    })
}

/// A semantic size on iced's point scale.
fn size_pt(s: WSize) -> f32 {
    match s {
        WSize::Small => 13.0,
        WSize::Medium => 17.0,
        WSize::Large => 30.0,
    }
}

/// A semantic background in this host's palette (the web page's values, from
/// `theme.rs`). `none` maps to nothing: the host's own background shows
/// through.
fn iced_background(b: WBackground) -> Option<iced::Background> {
    Some(match b {
        WBackground::None => return None,
        WBackground::Panel => iced::Background::Color(iced::Color::from_rgb8(0x24, 0x28, 0x3b)),
        WBackground::Bar => iced::Background::Color(iced::Color::from_rgb8(0x1e, 0x21, 0x30)),
    })
}

/// A corner radius in this host's pixels.
fn iced_radius(r: WRadius) -> iced::border::Radius {
    match r {
        WRadius::None => iced::border::radius(0.0),
        WRadius::Small => iced::border::radius(6.0),
        WRadius::Large => iced::border::radius(10.0),
    }
}

/// A semantic border in this host's palette; `none` is a zero-width
/// (invisible) border.
fn iced_border(b: WBorder, radius: WRadius) -> iced::Border {
    match b {
        WBorder::None => iced::Border {
            color: iced::Color::TRANSPARENT,
            width: 0.0,
            radius: iced_radius(radius),
        },
        WBorder::Line => iced::Border {
            color: iced::Color::from_rgb8(0x41, 0x48, 0x68),
            width: 1.0,
            radius: iced_radius(radius),
        },
        WBorder::Ring => iced::Border {
            color: iced::Color::from_rgb8(0x7a, 0xa2, 0xf7),
            width: 2.0,
            radius: iced_radius(radius),
        },
    }
}

/// A semantic shadow; `none` is iced's default (no shadow).
fn iced_shadow(s: WShadow) -> iced::Shadow {
    match s {
        WShadow::None => iced::Shadow::default(),
        WShadow::Raised => iced::Shadow {
            color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.5),
            offset: iced::Vector::new(0.0, 18.0),
            blur_radius: 50.0,
        },
    }
}

/// A cursor token as iced's cursor shape; `default` maps to nothing (the
/// content's own shape shows through). Iced has no generic resize shape, so
/// `resize` takes the corner-grip one (a drag that grows both axes).
fn iced_cursor(c: WCursor) -> Option<iced::mouse::Interaction> {
    Some(match c {
        WCursor::Default => return None,
        WCursor::Pointer => iced::mouse::Interaction::Pointer,
        WCursor::Grab => iced::mouse::Interaction::Grab,
        WCursor::Grabbing => iced::mouse::Interaction::Grabbing,
        WCursor::Resize => iced::mouse::Interaction::ResizingDiagonallyDown,
    })
}

/// Whether a container carries any visible chrome (a background, a border,
/// or a shadow): a container with none of them is drawn as a plain
/// row/column, as before.
fn has_chrome(style: &ContainerStyle) -> bool {
    style.background != WBackground::None
        || style.border != WBorder::None
        || style.shadow != WShadow::None
}

/// A container's chrome as a iced container style (its background, border,
/// radius, and shadow).
fn container_style(style: &ContainerStyle) -> iced::widget::container::Style {
    iced::widget::container::Style {
        text_color: None,
        background: iced_background(style.background),
        border: iced_border(style.border, style.radius),
        shadow: iced_shadow(style.shadow),
        snap: false,
    }
}

/// The contract's cross-axis token on a column's (horizontal) axis.
fn column_align(a: WAlign) -> iced::alignment::Horizontal {
    match a {
        WAlign::Start => iced::alignment::Horizontal::Left,
        WAlign::Center => iced::alignment::Horizontal::Center,
        WAlign::End => iced::alignment::Horizontal::Right,
    }
}

/// The contract's cross-axis token on a row's (vertical) axis.
fn row_align(a: WAlign) -> iced::alignment::Vertical {
    match a {
        WAlign::Start => iced::alignment::Vertical::Top,
        WAlign::Center => iced::alignment::Vertical::Center,
        WAlign::End => iced::alignment::Vertical::Bottom,
    }
}

/// A container's id (`None` for a container the component did not give one).
/// `map_tree` threads the `surface` (the component a node belongs to — the
/// main app's and a panel's id spaces overlap) and the `panels` (a content
/// slot seats the panel its id names) through to the container.
fn id_of(node: &Tree) -> Option<u32> {
    match node {
        Tree::Row(id, _, _, _) | Tree::Column(id, _, _, _) => *id,
        _ => None,
    }
}

/// Whether a node is a row container.
fn is_row(node: &Tree) -> bool {
    matches!(node, Tree::Row(..))
}

/// A container: its in-flow children in a row/column (the component's
/// `gap`/`padding`/`align`/`stretch`), its chrome, and its positioned
/// children (a window's grip) stacked on top at their `(x, y)`. A content
/// slot (a container an opened entry's handle is the id of) seats the
/// entry's tree. An id'd container is addressable (a press over it is a
/// pointer event; a tile, `cursor: pointer`, is double-clickable). A
/// container with all-default tokens (the demos) renders as a plain
/// row/column, as before.
///
/// A positioned container (a window) is *not* wrapped here: [`map_tree`]
/// wraps it in the host's fixed-position widget.
fn map_container<'a>(
    node: &'a Tree,
    surface: Surface,
    has_registry: bool,
    row: bool,
    panels: &'a [Panel],
    assets: &'a BTreeMap<String, Vec<u8>>,
) -> iced::Element<'a, Message> {
    use iced::widget;
    use iced::Length;
    let style = match node {
        Tree::Row(_, _, s, _) | Tree::Column(_, _, s, _) => *s,
        _ => unreachable!("a container is a row or a column"),
    };
    let align = match node {
        Tree::Row(_, a, _, _) | Tree::Column(_, a, _, _) => *a,
        _ => unreachable!(),
    };
    let children = match node {
        Tree::Row(_, _, _, c) | Tree::Column(_, _, _, c) => c,
        _ => unreachable!(),
    };

    // The content slot (a container the app gave an opened entry's handle as
    // its id): the entry's tree seats there, in the slot's place.
    let slot = id_of(node).and_then(|id| panels.iter().find(|p| p.handle == id));

    // The in-flow children, in a row/column. The component's `gap` (its
    // pixels, 1:1 on this host) or the host's default (8px) when it is `0`;
    // its `padding` when set.
    let in_flow: Vec<iced::Element<'_, Message>> = if let Some(panel) = slot {
        vec![map_tree(
            &panel.tree,
            Surface::Panel(panel.handle),
            false,
            &EMPTY_PANELS,
            assets,
        )]
    } else {
        children
            .iter()
            .filter(|c| !is_positioned(c))
            .map(|c| map_tree(c, surface, has_registry, panels, assets))
            .collect()
    };
    let spacing = if style.gap > 0 { style.gap as f32 } else { 8.0 };
    let padding = if style.padding > 0 {
        style.padding as f32
    } else {
        0.0
    };
    // For a row the main axis is the width, for a column the height.
    // The row/column's size: the component's explicit `w`/`h` (its pixels,
    // 1:1 on this host) when set, else the stretch (a `Fill` axis), else
    // content. An explicit size wins on the same axis (a container that sets
    // both is sized, not filled). For a row the main axis is the width, for
    // a column the height; the explicit `w`/`h` are always width/height.
    let (w_len, h_len) = if row {
        let mut w = Length::Shrink;
        let mut h = Length::Shrink;
        if matches!(style.stretch, WStretch::Main | WStretch::Both) {
            w = Length::Fill;
        }
        if matches!(style.stretch, WStretch::Cross | WStretch::Both) {
            h = Length::Fill;
        }
        if style.w > 0 {
            w = Length::Fixed(style.w as f32);
        }
        if style.h > 0 {
            h = Length::Fixed(style.h as f32);
        }
        (w, h)
    } else {
        let mut w = Length::Shrink;
        let mut h = Length::Shrink;
        if matches!(style.stretch, WStretch::Main | WStretch::Both) {
            h = Length::Fill;
        }
        if matches!(style.stretch, WStretch::Cross | WStretch::Both) {
            w = Length::Fill;
        }
        if style.w > 0 {
            w = Length::Fixed(style.w as f32);
        }
        if style.h > 0 {
            h = Length::Fixed(style.h as f32);
        }
        (w, h)
    };
    let mut base: iced::Element<'_, Message> = if row {
        let mut r = widget::row(in_flow)
            .spacing(spacing)
            .align_y(row_align(align));
        if padding > 0.0 {
            r = r.padding(padding);
        }
        r = r.width(w_len).height(h_len);
        r.into()
    } else {
        let mut c = widget::column(in_flow)
            .spacing(spacing)
            .align_x(column_align(align));
        if padding > 0.0 {
            c = c.padding(padding);
        }
        c = c.width(w_len).height(h_len);
        c.into()
    };

    // The chrome, when the container has any (a background, a border, or a
    // shadow). A container with none of them is drawn as a plain row/column.
    // The chrome sizes to its content (the row/column, which is already
    // sized), so the background spans it.
    if has_chrome(&style) {
        base = widget::container(base)
            .style(move |_: &iced::Theme| container_style(&style))
            .into();
    }

    // A node with an id is addressable: a press over it is reported
    // (`on-pointer-down`, the component raises a window and starts a drag or
    // resize), a release ends the press (and, near the press, is a click),
    // and a tile (`cursor: pointer`) is double-clickable. The cursor token
    // sets the shape.
    if let Some(id) = id_of(node) {
        let mut area = widget::MouseArea::new(base)
            .on_press(Message::PointerDown(surface, id))
            .on_release(Message::PointerUp);
        if has_registry && is_tile(&style) {
            area = area.on_double_click(Message::DoubleClick(id));
        }
        if let Some(cursor) = iced_cursor(style.cursor) {
            area = area.interaction(cursor);
        }
        base = area.into();
    }

    // The positioned children (a window, a grip): stacked on top at their
    // `(x, y)`. The stack sizes to its base layer (the row/column, already
    // sized), so the positioned children place without resizing it.
    let positioned: Vec<&Tree> = children.iter().filter(|c| is_positioned(c)).collect();
    if positioned.is_empty() {
        return base;
    }
    let mut layers = vec![base];
    let mut pos: Vec<(u32, &Tree)> = positioned.iter().map(|c| (z_of(c), *c)).collect();
    pos.sort_by_key(|&(z, _)| z);
    for (_, c) in pos {
        layers.push(map_tree(c, surface, has_registry, panels, assets));
    }
    widget::stack(layers).into()
}

/// A positioned container (a window): its content (its base and its own
/// positioned children) at its `(x, y)`, clamped to the canvas — the host's
/// fixed-position widget, reused for the component's own windows.
fn positioned_element<'a>(
    node: &'a Tree,
    surface: Surface,
    has_registry: bool,
    panels: &'a [Panel],
    assets: &'a BTreeMap<String, Vec<u8>>,
) -> iced::Element<'a, Message> {
    let (x, y) = match node {
        Tree::Row(_, _, s, _) | Tree::Column(_, _, s, _) => (s.x as f32, s.y as f32),
        _ => (0.0, 0.0),
    };
    iced::Element::new(PanelWidget {
        content: map_container(node, surface, has_registry, is_row(node), panels, assets),
        x,
        y,
    })
}

/// No panel: a panel's tree seats nothing (a panel runs a demo, not a
/// multi-component app). A `static` so the empty slice outlives the element
/// it feeds.
static EMPTY_PANELS: [Panel; 0] = [];

/// The placeholder for an image the host cannot draw (the browser module, or
/// a native `src` with no file): a box, the node's size (a host default when
/// it is 0), a dim marker, the host's standard border.
fn image_placeholder(w: f32, h: f32) -> iced::Element<'static, Message> {
    use iced::widget;
    let marker = widget::text("image")
        .size(14.0)
        .color(iced::Color::from_rgb8(0x56, 0x5f, 0x89));
    widget::container(marker)
        .center_x(iced::Length::Fixed(w))
        .center_y(iced::Length::Fixed(h))
        .style(|_: &iced::Theme| iced::widget::container::Style {
            text_color: None,
            background: None,
            border: iced::Border {
                color: iced::Color::from_rgb8(0x41, 0x48, 0x68),
                width: 1.0,
                radius: iced::border::radius(6.0),
            },
            shadow: iced::Shadow::default(),
            snap: false,
        })
        .into()
}

/// Map the tree onto iced widgets. `surface` names the component a node
/// belongs to (the main app's and a panel's id spaces overlap), and `panels`
/// seats a panel whose handle is a content slot's id.
fn map_tree<'a>(
    node: &'a Tree,
    surface: Surface,
    has_registry: bool,
    panels: &'a [Panel],
    assets: &'a BTreeMap<String, Vec<u8>>,
) -> iced::Element<'a, Message> {
    use iced::widget;
    match node {
        Tree::Text(content, style) => {
            let mut text = widget::text(content.as_str()).size(size_pt(style.size));
            if let Some(color) = iced_color(style.color) {
                text = text.color(color);
            }
            text.into()
        }
        Tree::Button(id, label, style) => {
            // The label carries the component's style; the button chrome
            // matches the web page (theme.rs) — a host concern, like the
            // TUI's focus ring. The page's 6px/16px padding.
            let mut label = widget::text(label.as_str()).size(size_pt(style.size));
            if let Some(color) = iced_color(style.color) {
                label = label.color(color);
            }
            let mut button = widget::button(label)
                .padding(iced::Padding {
                    top: 6.0,
                    right: 16.0,
                    bottom: 6.0,
                    left: 16.0,
                })
                .style(theme::button_style);
            // A button with no id is display-only.
            if let Some(id) = id {
                button = button.on_press(Message::Click(surface, *id));
            }
            button.into()
        }
        Tree::Input(id, content, width, style) => {
            // The field's chrome matches the web page (theme.rs), as the
            // button's does; the field's text takes the node's semantic
            // color (the page's style class does the same).
            let value = iced_color(style.color).unwrap_or_else(|| theme::style().text_color);
            // A fixed field width: the component's character count on the
            // field's font scale (a character is about half an em), or the
            // host's own default when it is `0`. Content past it scrolls (a
            // host concern, like the button's chrome).
            let px = if *width > 0 {
                *width as f32 * size_pt(style.size) * 0.5
            } else {
                80.0
            };
            let mut input = widget::text_input("", content.as_str())
                .width(iced::Length::Fixed(px))
                .padding(iced::Padding {
                    top: 6.0,
                    right: 16.0,
                    bottom: 6.0,
                    left: 16.0,
                })
                .size(size_pt(style.size))
                .style(theme::input_style(value));
            // iced disables a text input without an `on_input`; a field
            // with no id is unaddressable (the WIT identity note), so that
            // is exactly the right rendering for it.
            if let Some(id) = id {
                input = input.on_input(move |text| Message::Text(surface, *id, text));
            }
            input.into()
        }
        // An image node: the host's rendering of the `src`. Natively iced's
        // `svg` rasterizes the file on demand (the `--dump` path does so
        // synchronously), and a missing file falls back to the placeholder
        // box. In the browser there is no filesystem: the page's assets (the
        // registry's icons, base64) are rasterized from memory, and a `src`
        // with no asset falls back to the placeholder.
        Tree::Image(_, src, width, height) => {
            let w = if *width > 0 { *width as f32 } else { 64.0 };
            let h = if *height > 0 { *height as f32 } else { 64.0 };
            #[cfg(not(target_arch = "wasm32"))]
            {
                if std::path::Path::new(src).is_file() {
                    widget::svg(iced_core::svg::Handle::from_path(src.as_str()))
                        .width(iced::Length::Fixed(w))
                        .height(iced::Length::Fixed(h))
                        .into()
                } else {
                    image_placeholder(w, h)
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                match assets.get(src) {
                    Some(bytes) => widget::svg(iced_core::svg::Handle::from_memory(bytes.clone()))
                        .width(iced::Length::Fixed(w))
                        .height(iced::Length::Fixed(h))
                        .into(),
                    None => image_placeholder(w, h),
                }
            }
        }
        // A container: its in-flow children, its chrome, and its positioned
        // children (see `map_container`). A positioned one (a window) is the
        // host's fixed-position widget at its `(x, y)`.
        Tree::Row(..) | Tree::Column(..) => {
            if is_positioned(node) {
                positioned_element(node, surface, has_registry, panels, assets)
            } else {
                map_container(node, surface, has_registry, is_row(node), panels, assets)
            }
        }
    }
}
