//! A windowed host for waui components, rendered with [iced](https://iced.rs).
//!
//! The host owns a wasmtime store and a component instance. On every event it
//! forwards the event to the component, re-queries `view()`, rebuilds the
//! widget tree from the flat node list, and maps it onto iced widgets.

use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use wasmtime::component::*;
use wasmtime::{Engine, Store};

wasmtime::component::bindgen!({
    world: "app",
    path: "../../wit",
});

mod tree;

use tree::{build_tree, Tree};

/// The host's implementation of the world's imports.
struct Host;

impl AppImports for Host {
    fn now(&mut self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

// The `widget` interface is pulled into the world (via `use widget.{node}`),
// so the generated bindings require this empty marker impl.
impl waui::ui::widget::Host for Host {}

struct State {
    app: App,
    store: Store<Host>,
    tree: Tree,
}

#[derive(Debug, Clone)]
enum Message {
    Click(u32),
    Tick,
}

fn main() -> iced::Result {
    iced::application(boot, update, view)
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

    let mut store = Store::new(&engine, Host);
    let app = App::instantiate(&mut store, &component, &linker)
        .expect("failed to instantiate component");

    let tree = app
        .call_view(&mut store)
        .expect("failed to get initial view");

    State {
        app,
        store,
        tree: build_tree(&tree),
    }
}

fn update(state: &mut State, message: Message) {
    match message {
        Message::Click(id) => state
            .app
            .call_on_click(&mut state.store, id)
            .expect("on-click() failed"),
        Message::Tick => state
            .app
            .call_on_tick(&mut state.store)
            .expect("on-tick() failed"),
    }
    let nodes = state
        .app
        .call_view(&mut state.store)
        .expect("view() failed");
    state.tree = build_tree(&nodes);
}

fn view(state: &State) -> iced::Element<'_, Message> {
    map_tree(&state.tree)
}

/// Wake the component every 250ms so time-based UIs can advance.
fn subscription(_state: &State) -> iced::Subscription<Message> {
    iced::time::every(Duration::from_millis(250)).map(|_| Message::Tick)
}

/// The one place in this host where the toolkit shows up: map the
/// renderer-agnostic tree onto iced widgets.
fn map_tree(node: &Tree) -> iced::Element<'_, Message> {
    use iced::widget;
    match node {
        Tree::Text(content) => widget::text(content.as_str()).into(),
        Tree::Button(id, label) => widget::button(label.as_str())
            .on_press(Message::Click(*id))
            .into(),
        Tree::Row(children) => {
            widget::row(children.iter().map(map_tree)).spacing(8).into()
        }
        Tree::Column(children) => {
            widget::column(children.iter().map(map_tree)).spacing(8).into()
        }
    }
}
