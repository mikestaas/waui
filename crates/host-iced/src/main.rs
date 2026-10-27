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

use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use wasmtime::component::*;
use wasmtime::{Engine, Store};

// The world's bindings and the node-list→tree rebuild live in the shared
// host core; this crate is only its renderer.
use host_shared::{build_tree, App, AppImports, Retained, Tree, waui};

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
        *self.wake.lock().unwrap() =
            Wake { rate_ms: period, last_wake_ms: now_ms() };
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
}

#[derive(Debug, Clone)]
enum Message {
    Click(u32),
    Tick,
}

fn main() -> iced::Result {
    iced::application(boot, update, view)
        // Match the static web host: its palette (theme.rs), its centered
        // content, and a modest default window size.
        .theme(theme::theme())
        .style(|_, _| theme::style())
        .title("waui")
        .window_size(iced::Size::new(360.0, 340.0))
        .subscription(subscription)
        .run()
}

fn boot() -> State {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: host-iced <component.wasm>");
            std::process::exit(2);
        }
    };

    let engine = Engine::default();
    let component =
        Component::from_file(&engine, &path).expect("failed to load component");

    let mut linker = Linker::new(&engine);
    App::add_to_linker::<_, HasSelf<_>>(&mut linker, |host| host)
        .expect("failed to link world imports");

    let wake = Arc::new(Mutex::new(Wake::default()));
    let mut store = Store::new(&engine, Host { wake: wake.clone() });
    let app = App::instantiate(&mut store, &component, &linker)
        .expect("failed to instantiate component");

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
    }
}

fn update(state: &mut State, message: Message) {
    // A click always wakes the component. A tick wakes it only when its
    // requested period has elapsed (0 = never); iced's own clock is coarse
    // (50ms), so the gate, not the timer, sets the cadence.
    let woke = match message {
        Message::Click(id) => {
            state
                .app
                .call_on_click(&mut state.store, id)
                .expect("on-click() failed");
            true
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
            due
        }
    };
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
    // Like the web page, fill the window and center the tree, with some
    // room (the page's `#app` padding).
    widget::container(map_tree(&state.tree))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center)
        .padding(24.0)
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
            // stays with iced's theme (a host concern, like the TUI's
            // focus ring).
            let mut label = widget::text(label.as_str()).size(size_pt(style.size));
            if let Some(color) = iced_color(style.color) {
                label = label.color(color);
            }
            // A button with no id is display-only.
            match id {
                Some(id) => widget::button(label)
                    .on_press(Message::Click(*id))
                    .into(),
                None => widget::button(label).into(),
            }
        }
        Tree::Row(children) => {
            widget::row(children.iter().map(map_tree)).spacing(8).into()
        }
        Tree::Column(children) => {
            widget::column(children.iter().map(map_tree)).spacing(8).into()
        }
    }
}
