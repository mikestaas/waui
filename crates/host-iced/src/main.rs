//! A windowed host for waui components, rendered with [iced](https://iced.rs).
//!
//! The host bootstraps from `view()` (a full snapshot), then — on every
//! event it forwards to the component — applies the component's `changes()`
//! diff to its retained tree and rebuilds only when the diff is non-empty;
//! a tick that changes nothing skips the rebuild.
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
//! registry makes this host a desktop. On a desktop, a container whose id
//! is in the tile range is a double-clickable tile, and a double-click
//! (native, or `--double-click <id>`) opens a floating panel running the
//! entry's component in its own store: the host's own chrome (a title bar
//! with the entry's name and a close button), a cascade placement, and the
//! entry's default size. The close button drops the panel and reports
//! `on-window-close`; a press on a panel raises it — drag, resize, and
//! raise are host-internal (the WIT), and the panel's events never reach
//! the desktop below.
//!
//! `--dump <file.png>` renders one frame to a PNG and
//! exits, headless: the window's own render path with the surface replaced
//! by iced's software (`tiny-skia`) backend, so no window, display, or GPU
//! is needed and the check runs in CI. `--click`, `--double-click`, and
//! `--close` apply before the frame, as in the window. `--now <base>
//! <step>` fakes the host clock (each call advances `step` ms), so a
//! `--dump` frame that reads the clock is deterministic.

use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use wasmtime::component::*;
use wasmtime::{Engine, Store};

// The world's bindings and the node-list→tree rebuild live in the shared
// host core; this crate is only its renderer.
use host_shared::{build_tree, waui, App, AppImports, Entry, Error, Retained, Tree};

// The contract's style tokens, aliased to avoid clashing with `iced::Color`.
use host_shared::waui::ui::widget::{Align as WAlign, Color as WColor, Size as WSize};

// The `Widget` trait and the layout types for the panel's custom widget.
use iced_runtime::core as iced_core;

mod theme;

/// The component's refresh request, shared between the world-import impl
/// (the writer) and the tick handler (the reader): the period in ms, 0 =
/// no periodic wake, and the last wake.
#[derive(Default)]
struct Wake {
    rate_ms: u64,
    last_wake_ms: u64,
}

/// A launchable entry (`--entry <name=path>`): the registry's `data` blob
/// is always `{}`: the host supplies no per-entry metadata, so a tile
/// renders the name only, as the component degrades a blob it cannot
/// parse.
#[derive(Clone)]
struct EntrySpec {
    name: String,
    path: String,
}

/// An open panel: the entry's component running in its own store (the
/// world's imports take the store's data, not a store handle, so a panel
/// gets its own), its retained tree, its cascade placement, and its size.
struct Panel {
    handle: u32,
    title: String,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    store: Store<Host>,
    app: App,
    retained: Retained,
    tree: Tree,
    wake: Arc<Mutex<Wake>>,
    /// Whether the panel's component was driven (a click, a text edit, or a
    /// tick) since the last `changes` call.
    pending: bool,
}

/// The host's implementation of the world's imports. The world's imports
/// take the store's data, not a store handle, so a panel this host opens
/// carries its own store and its own copy of the panels it opens.
struct Host {
    wake: Arc<Mutex<Wake>>,
    /// A fake clock (`--now <base> <step>`): the n-th call to `now` returns
    /// `base + n * step`, so a `--dump` frame that reads the clock shows a
    /// fixed elapsed. `None` = wall time.
    fake_now: Option<(u64, u64)>,
    /// The fake clock's call count.
    fake_calls: u64,
    engine: Engine,
    linker: Linker<Host>,
    /// The registry (`--entry`): a non-empty one makes this host a desktop.
    entries: Vec<EntrySpec>,
    /// The open panels, in z-order (last = topmost).
    panels: Vec<Panel>,
    /// The next panel handle (the host's flat id, from 1).
    next_handle: u32,
}

impl Default for Host {
    fn default() -> Self {
        let engine = Engine::default();
        let linker = Linker::new(&engine);
        Host {
            wake: Arc::new(Mutex::new(Wake::default())),
            fake_now: None,
            fake_calls: 0,
            engine,
            linker,
            entries: Vec::new(),
            panels: Vec::new(),
            next_handle: 1,
        }
    }
}

impl Host {
    /// The host clock: the fake value when set, else wall time.
    fn clock(&mut self) -> u64 {
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
    /// linker, and clock, but an empty registry (a panel runs a demo, not
    /// a desktop, so the tile rule never fires in it) and its own panel
    /// list.
    fn window_host(&self) -> Host {
        Host {
            wake: Arc::new(Mutex::new(Wake::default())),
            fake_now: self.fake_now,
            fake_calls: 0,
            engine: self.engine.clone(),
            linker: self.linker.clone(),
            entries: Vec::new(),
            panels: Vec::new(),
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
    fn open_entry(&mut self, name: String, x: u32, y: u32, w: u32, h: u32) -> Result<u32, Error> {
        let Some(spec) = self.entries.iter().find(|e| e.name == name).cloned() else {
            return Err(Error::NotFound);
        };
        let component = match Component::from_file(&self.engine, &spec.path) {
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
        let n = self.panels.len();
        // A 0 placement cascades (down-right, wrapping); a 0 size takes
        // the host's default.
        let (x, y) = if x == 0 && y == 0 {
            let o = (n % 9) as f32 * CASCADE_STEP;
            (CASCADE_X + o, CASCADE_Y + o)
        } else {
            (x as f32, y as f32)
        };
        let width = if w > 0 { w as f32 } else { DEFAULT_W };
        let height = if h > 0 { h as f32 } else { DEFAULT_H };
        let handle = self.next_handle;
        self.next_handle = self.next_handle.saturating_add(1);
        self.panels.push(Panel {
            handle,
            title: spec.name,
            x,
            y,
            width,
            height,
            store,
            app,
            retained,
            tree,
            wake,
            pending: false,
        });
        Ok(handle)
    }
}

// The host clock in ms (wall time, as the world's `now` import promises).
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// The `widget` interface is pulled into the world (via `use widget.{node}`),
// so the generated bindings require this empty marker impl.
impl waui::ui::widget::Host for Host {}

struct State {
    app: App,
    store: Store<Host>,
    /// The retained node list, as the diff protocol maintains it.
    retained: Retained,
    tree: Tree,
    wake: Arc<Mutex<Wake>>,
    /// Events from `--click`, `--double-click`, and `--close`, in argument
    /// order, applied on the first tick after startup.
    pending: Vec<Injected>,
}

/// Which surface an event landed on: the desktop, or a panel (by handle).
/// The desktop's and a panel's trees are separate components with separate
/// id spaces, so a button id alone does not name its target.
#[derive(Debug, Clone, Copy)]
enum Surface {
    Desktop,
    Panel(u32),
}

#[derive(Debug, Clone)]
enum Message {
    Click(Surface, u32),
    /// The input with `id` now holds `text`: the field's whole buffer,
    /// after the user's latest edit.
    Text(Surface, u32, String),
    /// A desktop tile was double-clicked (native double-click, or
    /// `--double-click`): the desktop opens the entry's panel.
    DoubleClick(u32),
    /// A panel's close button was pressed.
    ClosePanel(u32),
    /// A press on a panel (outside its buttons): raise it to the top.
    RaisePanel(u32),
    Tick,
}

/// An event injected from the command line, in argument order: the same
/// actions the pointer drives, for a host that can be driven without one.
#[derive(Clone)]
enum Injected {
    Click(u32),
    DoubleClick(u32),
    Close(u32),
}

// The default window size, and the `--dump` canvas, for a host that runs
// no entries.
const WINDOW_WIDTH: f32 = 360.0;
const WINDOW_HEIGHT: f32 = 340.0;

// The desktop's canvas: the window and the dump render the same size, so a
// `--dump` frame is the window's frame.
const DESKTOP_WIDTH: f32 = 960.0;
const DESKTOP_HEIGHT: f32 = 600.0;

/// The window and dump canvas: the desktop's when the host runs entries,
/// else the standalone default (the existing goldens' size).
fn canvas_size(has_registry: bool) -> iced::Size {
    if has_registry {
        iced::Size::new(DESKTOP_WIDTH, DESKTOP_HEIGHT)
    } else {
        iced::Size::new(WINDOW_WIDTH, WINDOW_HEIGHT)
    }
}

// A panel the component did not size: the host's default.
const DEFAULT_W: f32 = 300.0;
const DEFAULT_H: f32 = 380.0;

// New panels cascade down-right (the web host's geometry), wrapping after
// nine.
const CASCADE_X: f32 = 24.0;
const CASCADE_Y: f32 = 48.0;
const CASCADE_STEP: f32 = 28.0;

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

/// The host's arguments: `--entry`, `--click`, `--double-click`, and
/// `--close` flags, an optional `--dump <file>`, an optional `--now <base>
/// <step>`, and the component path. A bad argument exits with usage.
struct Args {
    path: String,
    entries: Vec<EntrySpec>,
    injected: Vec<Injected>,
    dump: Option<String>,
    now: Option<(u64, u64)>,
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut entries = Vec::new();
    let mut injected = Vec::new();
    let mut dump = None;
    let mut now = None;
    let mut path = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--entry" => match args.get(i + 1) {
                Some(v) => {
                    let (name, p) = v.split_once('=').unwrap_or_else(|| {
                        eprintln!("--entry takes name=path");
                        std::process::exit(2);
                    });
                    entries.push(EntrySpec {
                        name: name.to_string(),
                        path: p.to_string(),
                    });
                    i += 1;
                }
                None => {
                    eprintln!("--entry takes name=path");
                    std::process::exit(2);
                }
            },
            "--click" => match args.get(i + 1) {
                Some(v) => {
                    injected.push(Injected::Click(
                        v.parse().expect("--click takes a numeric widget id"),
                    ));
                    i += 1;
                }
                None => {
                    eprintln!("--click takes a numeric widget id");
                    std::process::exit(2);
                }
            },
            "--double-click" => match args.get(i + 1) {
                Some(v) => {
                    injected.push(Injected::DoubleClick(
                        v.parse().expect("--double-click takes a numeric widget id"),
                    ));
                    i += 1;
                }
                None => {
                    eprintln!("--double-click takes a numeric widget id");
                    std::process::exit(2);
                }
            },
            "--close" => match args.get(i + 1) {
                Some(v) => {
                    injected.push(Injected::Close(
                        v.parse().expect("--close takes a numeric window handle"),
                    ));
                    i += 1;
                }
                None => {
                    eprintln!("--close takes a numeric window handle");
                    std::process::exit(2);
                }
            },
            "--dump" => match args.get(i + 1) {
                Some(v) => {
                    dump = Some(v.clone());
                    i += 1;
                }
                None => {
                    eprintln!("--dump takes a file path");
                    std::process::exit(2);
                }
            },
            "--now" => match (args.get(i + 1), args.get(i + 2)) {
                (Some(b), Some(s)) => {
                    now = Some((
                        b.parse().expect("--now takes a numeric base and step"),
                        s.parse().expect("--now takes a numeric base and step"),
                    ));
                    i += 2;
                }
                _ => {
                    eprintln!("--now takes a numeric base and step (milliseconds)");
                    std::process::exit(2);
                }
            },
            other if !other.starts_with('-') => path = Some(other.to_string()),
            _ => {}
        }
        i += 1;
    }
    let path = match path {
        Some(p) => p,
        None => {
            eprintln!(
                "usage: host-iced [--entry <name=path>]... [--click <id>] [--double-click <id>] [--close <handle>] [--dump <file.png>] [--now <base> <step>] <component.wasm>"
            );
            std::process::exit(2);
        }
    };
    Args {
        path,
        entries,
        injected,
        dump,
        now,
    }
}

fn main() -> iced::Result {
    let args = parse_args();
    if let Some(file) = &args.dump {
        // Headless: apply the injected events, render one frame to a PNG,
        // and exit — no window, so it runs in CI.
        let mut state = new_state(
            &args.path,
            args.entries.clone(),
            args.injected.clone(),
            args.now,
        );
        apply_pending(&mut state);
        return render_dump(&state, file);
    }
    iced::application(boot, update, view)
        // Match the static web host: its palette (theme.rs), its centered
        // content, and a modest default window size.
        .theme(theme::theme())
        .style(|_, _| theme::style())
        .title("waui")
        .window_size(canvas_size(!args.entries.is_empty()))
        .subscription(subscription)
        .run()
}

// Parsed again here because iced's boot takes no parameters; the result
// matches `main`'s parse of the same arguments.
fn boot() -> State {
    let args = parse_args();
    new_state(&args.path, args.entries, args.injected, args.now)
}

/// Instantiate the component and build the initial retained node list and
/// widget tree from its `view()` snapshot.
fn new_state(
    path: &str,
    entries: Vec<EntrySpec>,
    injected: Vec<Injected>,
    fake_now: Option<(u64, u64)>,
) -> State {
    let engine = Engine::default();
    let component = Component::from_file(&engine, path).expect("failed to load component");

    let mut linker = Linker::new(&engine);
    App::add_to_linker::<_, HasSelf<_>>(&mut linker, |host| host)
        .expect("failed to link world imports");

    let wake = Arc::new(Mutex::new(Wake::default()));
    let mut store = Store::new(
        &engine,
        Host {
            wake: wake.clone(),
            fake_now,
            fake_calls: 0,
            engine: engine.clone(),
            linker: linker.clone(),
            entries,
            panels: Vec::new(),
            next_handle: 1,
        },
    );
    let app =
        App::instantiate(&mut store, &component, &linker).expect("failed to instantiate component");

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
    }
}

/// Apply the `--click`, `--double-click`, and `--close` injections, in
/// argument order, and the components' resulting diffs, rebuilding the
/// trees that changed. The windowed host does this on the first tick; the
/// dump path does it before rendering. The startup snapshot precedes it
/// (the desktop fetches its registry on first view, so a double-click
/// looks up the registry that view fetched).
fn apply_pending(state: &mut State) {
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
    diff_desktop(state);
    diff_panels(state);
}

/// The desktop's diff: rebuild its tree when the component reports a
/// change (an empty diff skips the rebuild).
fn diff_desktop(state: &mut State) {
    let mutations = state
        .app
        .call_changes(&mut state.store)
        .expect("changes() failed");
    if state.retained.apply(&mutations) {
        state.tree = build_tree(state.retained.nodes());
    }
}

/// The panels' diffs: rebuild each panel that was driven.
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
            if p.retained.apply(&mutations) {
                p.tree = build_tree(p.retained.nodes());
            }
        }
    }
}

/// Drive the panel with `handle` (a click or a text edit) and mark it for
/// a diff.
fn wake_panel(state: &mut State, handle: u32, drive: impl Fn(&mut Panel)) {
    let panels = &mut state.store.data_mut().panels;
    if let Some(p) = panels.iter_mut().find(|p| p.handle == handle) {
        drive(p);
        p.pending = true;
    }
}

/// The dump must render identically on every machine. Iced's font database
/// holds the system fonts too, and the bundled Fira Sans lacks the todo's
/// checkbox and delete symbols, so those glyphs would fall back to a system
/// font that differs per platform. Prune the database to the bundled fonts
/// and load the symbol font that covers the glyphs Fira lacks.
/// The interactive window keeps the system fonts; only the dump prunes.
fn pin_dump_fonts() {
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
}

/// Render the current view to a PNG file, headless: no window, no display,
/// no GPU.
///
/// The render is the window's own path with the surface replaced by iced's
/// `Headless` renderer (the software `tiny-skia` backend): the same
/// `UserInterface` layout and draw, then `screenshot` rasterizes the
/// recorded layers into a pixel buffer.
///
/// Text renders with the bundled Fira Sans (the `fira-sans` feature)
/// rather than a system font: a CI machine may have none, and the dump
/// must be stable to be a useful check. `pin_dump_fonts` prunes the font
/// database so the few glyphs Fira lacks also come from bundled bytes;
/// the window keeps the system fonts.
fn render_dump(state: &State, file: &str) -> iced::Result {
    use iced_runtime::core::renderer::Headless;

    pin_dump_fonts();

    // The executor blocks on the renderer's creation (synchronous with the
    // tiny-skia backend; no GPU is touched).
    use iced::executor::Executor;
    let executor = iced::executor::Default::new().expect("failed to create an executor");
    let mut renderer = executor
        .block_on(iced::Renderer::new(
            iced::Font::with_name("Fira Sans"),
            16.0.into(),
            Some("tiny-skia"),
        ))
        .expect("no headless renderer");

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

    // Rasterize the layers at the window's logical size; the buffer is
    // unpremultiplied RGBA.
    let width = size.width as u32;
    let height = size.height as u32;
    let rgba = renderer.screenshot(iced::Size::new(width, height), 1.0, style.background_color);

    let png = std::fs::File::create(file).expect("failed to create the dump file");
    let mut writer = std::io::BufWriter::new(png);
    let mut encoder = png::Encoder::new(&mut writer, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut frame = encoder
        .write_header()
        .expect("failed to write the PNG header");
    frame
        .write_image_data(&rgba)
        .expect("failed to write the PNG");
    Ok(())
}

fn update(state: &mut State, message: Message) {
    // `--click`, `--double-click`, and `--close` injections take the same
    // path as the pointer and keyboard: applied on the first tick.
    apply_pending(state);

    // A click, a text edit, or a double-click always wakes the component it
    // landed on. A tick wakes a component only when its requested period
    // has elapsed (0 = never); iced's own clock is coarse (50ms), so the
    // gate, not the timer, sets the cadence.
    let mut desktop_woke = false;
    match message {
        Message::Click(Surface::Desktop, id) => {
            state
                .app
                .call_on_click(&mut state.store, id)
                .expect("on-click() failed");
            desktop_woke = true;
        }
        Message::Click(Surface::Panel(handle), id) => {
            wake_panel(state, handle, |p| {
                p.app
                    .call_on_click(&mut p.store, id)
                    .expect("on-click() failed");
            });
        }
        // An edit always wakes the component: it re-derives the field's
        // content from the buffer it stores, and the diff carries it.
        Message::Text(Surface::Desktop, id, text) => {
            state
                .app
                .call_on_text(&mut state.store, id, &text)
                .expect("on-text() failed");
            desktop_woke = true;
        }
        Message::Text(Surface::Panel(handle), id, text) => {
            wake_panel(state, handle, |p| {
                p.app
                    .call_on_text(&mut p.store, id, &text)
                    .expect("on-text() failed");
            });
        }
        // A tile was double-clicked: the desktop opens the entry's panel
        // through the open-entry import, and its menu bar counts it.
        Message::DoubleClick(id) => {
            state
                .app
                .call_on_double_click(&mut state.store, id)
                .expect("on-double-click() failed");
            desktop_woke = true;
        }
        // The close button drops the panel and tells the desktop (it drops
        // the handle from its open set and updates its count).
        Message::ClosePanel(handle) => {
            state.store.data_mut().panels.retain(|p| p.handle != handle);
            state
                .app
                .call_on_window_close(&mut state.store, handle)
                .expect("on-window-close() failed");
            desktop_woke = true;
        }
        // Raise is host-internal (the WIT): z-order only, no component
        // wake, no diff.
        Message::RaisePanel(handle) => {
            let panels = &mut state.store.data_mut().panels;
            if let Some(i) = panels.iter().position(|p| p.handle == handle) {
                let panel = panels.remove(i);
                panels.push(panel);
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
            desktop_woke |= due;
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
    if desktop_woke {
        diff_desktop(state);
    }
    diff_panels(state);
}

fn view(state: &State) -> iced::Element<'_, Message> {
    use iced::alignment;
    use iced::widget;
    use iced::Length;
    let host = state.store.data();
    let has_registry = !host.entries.is_empty();
    // Like the page, fill the window and center the desktop, with the
    // page's `#app` padding (2rem).
    let mut layers = vec![
        widget::container(map_tree(&state.tree, Surface::Desktop, has_registry))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(alignment::Horizontal::Center)
            .align_y(alignment::Vertical::Center)
            .padding(32.0)
            .into(),
    ];
    // The open panels, in z-order (bottom to top): the stack draws them in
    // that order, and its update runs them top-down, so a top panel's
    // opaque bounds keep its press from reaching the desktop below.
    for panel in &host.panels {
        layers.push(panel_element(panel));
    }
    // A host with no panels is the standalone view of today: the stack
    // with its single layer draws the layer alone, at the layer's size.
    widget::stack(layers).into()
}

/// One panel's element: its chrome (a title bar with the entry's name and
/// a close button, its content centered below) at its cascade position,
/// opaque to the desktop below, raised by a press.
fn panel_element(panel: &Panel) -> iced::Element<'_, Message> {
    use iced::alignment;
    use iced::widget;
    use iced::Length;
    let title = widget::text(&panel.title)
        .size(13.0)
        .color(iced::Color::from_rgb8(0xc0, 0xca, 0xf5));
    let close = widget::button(widget::text("×").size(16.0))
        .padding(iced::Padding {
            top: 5.0,
            right: 8.0,
            bottom: 5.0,
            left: 8.0,
        })
        .style(theme::close_style)
        .on_press(Message::ClosePanel(panel.handle));
    let bar = widget::container(
        widget::row([
            title.into(),
            widget::space().width(Length::Fill).into(),
            close.into(),
        ])
        .spacing(8.0)
        .height(36.0)
        .padding(iced::Padding {
            top: 0.0,
            right: 8.0,
            bottom: 0.0,
            left: 12.0,
        })
        .align_y(alignment::Vertical::Center),
    )
    .width(Length::Fill)
    .style(theme::titlebar_style);
    let content = widget::container(map_tree(&panel.tree, Surface::Panel(panel.handle), false))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center)
        .padding(16.0);
    let body = widget::container(
        widget::column([bar.into(), content.into()])
            .width(Length::Fixed(panel.width))
            .height(Length::Fixed(panel.height)),
    )
    .style(theme::panel_style);
    // A press on the panel (a button captures its own; the rest raises the
    // panel), then the opaque wrapper: a press in the panel's bounds never
    // reaches the desktop below, and its empty area levitates no hover.
    let interactive = widget::MouseArea::new(body).on_press(Message::RaisePanel(panel.handle));
    iced::Element::new(PanelWidget {
        content: widget::opaque(interactive),
        x: panel.x,
        y: panel.y,
    })
}

/// A panel at a fixed position on the canvas: its content laid out at
/// (x, y), clamped to the canvas. The stack sizes to its base layer (the
/// desktop), so the panel's size only places it; everything else
/// delegates to the content.
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
fn subscription(_state: &State) -> iced::Subscription<Message> {
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

/// Map the tree onto iced widgets. `surface` names the component a button
/// or field belongs to (the desktop's and a panel's id spaces overlap),
/// and `has_registry` marks a desktop: its tiles are framed and
/// double-clickable, its containers are not.
fn map_tree(node: &Tree, surface: Surface, has_registry: bool) -> iced::Element<'_, Message> {
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
        // This host draws no pixels: a placeholder box, the node's size
        // (a host default when it is 0), a dim marker, the host's standard
        // border.
        Tree::Image(_, _, width, height) => {
            let w = if *width > 0 { *width as f32 } else { 64.0 };
            let h = if *height > 0 { *height as f32 } else { 64.0 };
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
        // A tile on a desktop: a framed, double-clickable container (the
        // web marks the same range `.tile`). A single click does nothing
        // (the WIT: the double-click activates).
        Tree::Row(id, align, children) | Tree::Column(id, align, children)
            if has_registry && is_tile(*id) =>
        {
            let content: iced::Element<'_, Message> = match node {
                Tree::Row(_, align, children) => {
                    widget::row(children.iter().map(|c| map_tree(c, surface, has_registry)))
                        .spacing(8)
                        .align_y(row_align(*align))
                        .into()
                }
                Tree::Column(_, align, children) => {
                    widget::column(children.iter().map(|c| map_tree(c, surface, has_registry)))
                        .spacing(8)
                        .align_x(column_align(*align))
                        .into()
                }
                _ => unreachable!("a tile is a row or a column"),
            };
            let id = id.unwrap();
            widget::MouseArea::new(
                widget::container(content)
                    .padding(16.0)
                    .style(theme::tile_style),
            )
            .on_double_click(Message::DoubleClick(id))
            .into()
        }
        // The component's cross-axis token; a host default (start) would
        // draw the contract's default (center) wrong.
        Tree::Row(_, align, children) => {
            widget::row(children.iter().map(|c| map_tree(c, surface, has_registry)))
                .spacing(8)
                .align_y(row_align(*align))
                .into()
        }
        Tree::Column(_, align, children) => {
            widget::column(children.iter().map(|c| map_tree(c, surface, has_registry)))
                .spacing(8)
                .align_x(column_align(*align))
                .into()
        }
    }
}
