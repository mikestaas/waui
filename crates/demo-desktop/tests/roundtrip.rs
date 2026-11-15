//! Headless contract test for the desktop component: drive it through the
//! same wasmtime host path the real hosts use, and assert on the grid it
//! builds from the `list-entries` registry, the window count, and the
//! open/close round-trip (a double-click opens a window, a close drops it).
//!
//! The host implements the `app` world's imports: `list-entries` (the
//! registry) and `open-entry` (which assigns a handle and records the
//! call). `now` and `schedule` are stubbed: the desktop has no clock.

use std::sync::Mutex;
use wasmtime::component::*;
use wasmtime::{Engine, Store};
use waui_test::{build_component, component_path, RUST_BUILD_LOCK};

wasmtime::component::bindgen!({
    world: "app",
    path: "../../wit",
});

/// The host's side of the `app` world.
#[derive(Default)]
struct Host {
    /// The registry `list-entries` returns.
    entries: Vec<Entry>,
    /// The next window handle to assign.
    next: u32,
    /// The (name, width, height) of each `open-entry` call, in order.
    opens: Mutex<Vec<(String, u32, u32)>>,
}

impl AppImports for Host {
    // The desktop has no clock: the stubs are never called.
    fn now(&mut self) -> u64 {
        0
    }
    fn schedule(&mut self, _period: u64) {}

    fn list_entries(&mut self) -> Vec<Entry> {
        self.entries.clone()
    }

    fn open_entry(&mut self, name: String, _x: u32, _y: u32, w: u32, h: u32) -> Result<u32, Error> {
        // A name the host does not recognize fails, like wasi's open-at.
        if !self.entries.iter().any(|e| e.name == name) {
            return Err(Error::NotFound);
        }
        let handle = self.next;
        self.next += 1;
        self.opens.lock().unwrap().push((name, w, h));
        Ok(handle)
    }
}

// The `widget` interface is pulled into the world, so the generated
// bindings require this empty marker impl.
impl waui::ui::widget::Host for Host {}

/// The nine entries, in the site's order (three demos, three languages):
/// the name is the unique identifier, the `data` blob the tile's
/// presentation.
fn nine_entries() -> Vec<Entry> {
    let names = ["Counter", "Stopwatch", "Todo"];
    let langs = ["Rust", "AssemblyScript", "WAT"];
    let mut out = Vec::new();
    for lang in langs {
        for name in &names {
            out.push(Entry {
                name: format!("{name}-{lang}"),
                data: format!(
                    r#"{{"label":"{name}","lang":"{lang}","icon":"icons/{name}-{lang}.svg","width":300,"height":380}}"#
                ),
            });
        }
    }
    out
}

/// Link and instantiate the component against our world.
fn instantiate(engine: &Engine, store: &mut Store<Host>, path: &std::path::Path) -> App {
    let component = Component::from_file(engine, path).expect("failed to load component");
    let mut linker = Linker::new(engine);
    App::add_to_linker::<_, HasSelf<_>>(&mut linker, |host| host)
        .expect("failed to link world imports");
    App::instantiate(store, &component, &linker).expect("failed to instantiate")
}

/// The node's id, if it has one.
fn node_id(node: &waui::ui::widget::Node) -> Option<u32> {
    match node {
        waui::ui::widget::Node::Text(t) => t.id,
        waui::ui::widget::Node::Button(b) => b.id,
        waui::ui::widget::Node::Input(i) => i.id,
        waui::ui::widget::Node::Image(i) => i.id,
        waui::ui::widget::Node::Container(c) => c.id,
    }
}

/// The widget with the given id, as a one-line summary, for assertions.
fn node_summary(node: &waui::ui::widget::Node) -> String {
    match node {
        waui::ui::widget::Node::Text(t) => format!("text:{:?}:{}", t.id, t.content),
        waui::ui::widget::Node::Button(b) => format!("button:{:?}:{}", b.id, b.label),
        waui::ui::widget::Node::Input(i) => format!("input:{:?}:{}", i.id, i.content),
        waui::ui::widget::Node::Image(i) => format!("image:{:?}:{}", i.id, i.src),
        waui::ui::widget::Node::Container(c) => {
            format!("container:{:?}: {:?}", c.id, c.direction)
        }
    }
}

#[test]
fn desktop_grid_and_window_roundtrip() {
    let engine = Engine::default();
    let mut store = Store::new(
        &engine,
        Host {
            entries: nine_entries(),
            ..Default::default()
        },
    );
    let mut app = {
        let _guard = RUST_BUILD_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        build_component("demo-desktop");
        instantiate(&engine, &mut store, &component_path("demo-desktop"))
    };

    // The initial view: the menu bar (0 windows) and the 3x3 grid.
    let nodes = app.call_view(&mut store).expect("view failed");
    let summaries: Vec<String> = nodes.iter().map(node_summary).collect();
    // The menu bar: "waui" and "0 windows".
    assert!(summaries.iter().any(|s| s == "text:None:waui"));
    assert!(summaries.iter().any(|s| s == "text:Some(2):0 windows"));
    // The nine tiles, each an image + name + lang.
    assert!(summaries
        .iter()
        .any(|s| s == "image:None:icons/Counter-Rust.svg"));
    assert!(summaries.iter().any(|s| s == "text:None:Counter"));
    assert!(summaries.iter().any(|s| s == "text:None:Rust"));
    assert!(summaries
        .iter()
        .any(|s| s == "image:None:icons/Todo-WAT.svg"));

    // Nothing has changed since the view: an empty diff.
    assert!(app
        .call_changes(&mut store)
        .expect("changes failed")
        .is_empty());

    // Double-click the first tile (Counter, Rust): it opens a window.
    app.call_on_double_click(&mut store, 100)
        .expect("on-double-click failed");
    let opens = store.data().opens.lock().unwrap();
    assert_eq!(opens.len(), 1, "one tile opened one window");
    // The open is by the entry's name; the size is the blob's default.
    assert_eq!(opens[0].0, "Counter-Rust");
    assert_eq!((opens[0].1, opens[0].2), (300, 380));
    drop(opens);

    // The count is now 1: the diff reports the changed count node.
    let mutations = app.call_changes(&mut store).expect("changes failed");
    assert_eq!(
        mutations.len(),
        1,
        "opening a window changes the count only"
    );
    match &mutations[0] {
        waui::ui::widget::Mutation::Changed(c) => {
            assert_eq!(node_id(&c.node), Some(2), "the changed node is the count");
        }
        other => panic!("expected a changed count node, got {other:?}"),
    }
    // And the full view agrees: "1 window" (singular).
    let nodes = app.call_view(&mut store).expect("view failed");
    let summaries: Vec<String> = nodes.iter().map(node_summary).collect();
    assert!(summaries.iter().any(|s| s == "text:Some(2):1 window"));

    // Open a second window, then close the first: the count returns to 1.
    app.call_on_double_click(&mut store, 101)
        .expect("on-double-click failed");
    app.call_on_window_close(&mut store, 0)
        .expect("on-window-close failed");
    let nodes = app.call_view(&mut store).expect("view failed");
    let summaries: Vec<String> = nodes.iter().map(node_summary).collect();
    assert!(summaries.iter().any(|s| s == "text:Some(2):1 window"));

    // A double-click on a non-tile id is a no-op (no window, no diff).
    app.call_on_double_click(&mut store, 999)
        .expect("on-double-click failed");
    assert!(
        app.call_changes(&mut store)
            .expect("changes failed")
            .is_empty(),
        "a non-tile double-click must produce no changes"
    );
}
