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
//! `--dump <file.png>` renders one frame to a PNG and exits, headless: the
//! window's own render path with the surface replaced by iced's software
//! (`tiny-skia`) backend, so no window, display, or GPU is needed and the
//! check runs in CI. `--click` applies before the frame, as in the window.

use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use wasmtime::component::*;
use wasmtime::{Engine, Store};

// The world's bindings and the node-list→tree rebuild live in the shared
// host core; this crate is only its renderer.
use host_shared::{build_tree, waui, App, AppImports, Retained, Tree};

// The contract's style tokens, aliased to avoid clashing with `iced::Color`.
use host_shared::waui::ui::widget::{Color as WColor, Size as WSize};

mod theme;

/// The component's refresh request, shared between the world-import impl
/// (the writer) and the tick handler (the reader): the period in ms, 0 =
/// no periodic wake, and the last wake.
#[derive(Default)]
struct Wake {
    rate_ms: u64,
    last_wake_ms: u64,
}

/// The host's implementation of the world's imports.
#[derive(Clone, Default)]
struct Host {
    wake: Arc<Mutex<Wake>>,
}

impl AppImports for Host {
    fn now(&mut self) -> u64 {
        now_ms()
    }
    fn schedule(&mut self, period: u64) {
        *self.wake.lock().unwrap() = Wake {
            rate_ms: period,
            last_wake_ms: now_ms(),
        };
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
    /// Clicks from `--click`, applied on the first tick after startup.
    pending_clicks: Vec<u32>,
}

#[derive(Debug, Clone)]
enum Message {
    Click(u32),
    Tick,
}

// The default window size, and the `--dump` canvas.
const WINDOW_WIDTH: f32 = 360.0;
const WINDOW_HEIGHT: f32 = 340.0;

/// The host's arguments: `--click <id>` flags, an optional `--dump <file>`,
/// and the component path. A bad argument exits with usage.
struct Args {
    path: String,
    clicks: Vec<u32>,
    dump: Option<String>,
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut clicks = Vec::new();
    let mut dump = None;
    let mut path = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--click" => match args.get(i + 1) {
                Some(v) => {
                    clicks.push(v.parse().expect("--click takes a numeric widget id"));
                    i += 1;
                }
                None => {
                    eprintln!("--click takes a numeric widget id");
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
            other if !other.starts_with('-') => path = Some(other.to_string()),
            _ => {}
        }
        i += 1;
    }
    let path = match path {
        Some(p) => p,
        None => {
            eprintln!("usage: host-iced [--click <id>] [--dump <file.png>] <component.wasm>");
            std::process::exit(2);
        }
    };
    Args { path, clicks, dump }
}

fn main() -> iced::Result {
    let args = parse_args();
    if let Some(file) = &args.dump {
        // Headless: apply the injected clicks, render one frame to a PNG,
        // and exit — no window, so it runs in CI.
        let mut state = new_state(&args.path, args.clicks.clone());
        apply_pending_clicks(&mut state);
        return render_dump(&state, file);
    }
    iced::application(boot, update, view)
        // Match the static web host: its palette (theme.rs), its centered
        // content, and a modest default window size.
        .theme(theme::theme())
        .style(|_, _| theme::style())
        .title("waui")
        .window_size(iced::Size::new(WINDOW_WIDTH, WINDOW_HEIGHT))
        .subscription(subscription)
        .run()
}

// Parsed again here because iced's boot takes no parameters; the result
// matches `main`'s parse of the same arguments.
fn boot() -> State {
    let args = parse_args();
    new_state(&args.path, args.clicks)
}

/// Instantiate the component and build the initial retained node list and
/// widget tree from its `view()` snapshot.
fn new_state(path: &str, clicks: Vec<u32>) -> State {
    let engine = Engine::default();
    let component = Component::from_file(&engine, path).expect("failed to load component");

    let mut linker = Linker::new(&engine);
    App::add_to_linker::<_, HasSelf<_>>(&mut linker, |host| host)
        .expect("failed to link world imports");

    let wake = Arc::new(Mutex::new(Wake::default()));
    let mut store = Store::new(&engine, Host { wake: wake.clone() });
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
        pending_clicks: clicks,
    }
}

/// Apply the `--click` injections and the component's resulting diff,
/// rebuilding the tree when the diff is non-empty. The windowed host does
/// this on the first tick; the dump path does it before rendering.
fn apply_pending_clicks(state: &mut State) {
    if state.pending_clicks.is_empty() {
        return;
    }
    for id in state.pending_clicks.drain(..) {
        state
            .app
            .call_on_click(&mut state.store, id)
            .expect("on-click() failed");
    }
    let mutations = state
        .app
        .call_changes(&mut state.store)
        .expect("changes() failed");
    if state.retained.apply(&mutations) {
        state.tree = build_tree(state.retained.nodes());
    }
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
/// must be stable to be a useful check. The window keeps the system font;
/// only this renderer's default changes.
fn render_dump(state: &State, file: &str) -> iced::Result {
    use iced_runtime::core::renderer::Headless;

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

    let size = iced::Size::new(WINDOW_WIDTH, WINDOW_HEIGHT);

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
    let width = WINDOW_WIDTH as u32;
    let height = WINDOW_HEIGHT as u32;
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
    // A click always wakes the component. A tick wakes it only when its
    // requested period has elapsed (0 = never); iced's own clock is coarse
    // (50ms), so the gate, not the timer, sets the cadence.
    // `--click` injections take the same path as pointer clicks.
    let mut woke = !state.pending_clicks.is_empty();
    apply_pending_clicks(state);
    match message {
        Message::Click(id) => {
            state
                .app
                .call_on_click(&mut state.store, id)
                .expect("on-click() failed");
            woke = true;
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
            woke |= due;
        }
    }
    if woke {
        // The diff channel: rebuild only when the component reports
        // something changed (an empty diff skips the rebuild).
        let mutations = state
            .app
            .call_changes(&mut state.store)
            .expect("changes() failed");
        if state.retained.apply(&mutations) {
            state.tree = build_tree(state.retained.nodes());
        }
    }
}

fn view(state: &State) -> iced::Element<'_, Message> {
    use iced::alignment;
    use iced::widget;
    use iced::Length;
    // Like the web page, fill the window and center the tree, with the
    // page's `#app` padding (2rem).
    widget::container(map_tree(&state.tree))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center)
        .padding(32.0)
        .into()
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

/// Map the tree onto iced widgets.
fn map_tree(node: &Tree) -> iced::Element<'_, Message> {
    use iced::alignment;
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
                button = button.on_press(Message::Click(*id));
            }
            button.into()
        }
        // The web page centers a column's children (align-items: center)
        // and a row's; iced's default is start (top/left).
        Tree::Row(children) => widget::row(children.iter().map(map_tree))
            .spacing(8)
            .align_y(alignment::Vertical::Center)
            .into(),
        Tree::Column(children) => widget::column(children.iter().map(map_tree))
            .spacing(8)
            .align_x(alignment::Horizontal::Center)
            .into(),
    }
}
