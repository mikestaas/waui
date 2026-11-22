//! Headless contract test for the desktop component: drive it through the
//! same wasmtime host path the real hosts use, and assert on the grid it
//! builds from the `list-entries` registry, the window count, and the
//! open/close round-trip (a double-click opens a window, a close drops it).
//!
//! The host implements the `app` world's imports: `list-entries` (the
//! registry) and `open-entry` (which assigns a handle and records the
//! call). `now` and `schedule` are stubbed: the desktop has no clock.
//! `viewport` reports the unknown size (the cascade skips its clamp) and
//! `close-entry` is a no-op (the host tracks no sandboxes).

use std::sync::Mutex;
use wasmtime::component::*;
use wasmtime::{Engine, Store};
use waui_test::{
    as_toolchain_available, build_component, build_component_as, build_component_wat,
    component_path, component_path_as, component_path_wat, wat_toolchain_available, AS_BUILD_LOCK,
    RUST_BUILD_LOCK, WAT_BUILD_LOCK,
};

wasmtime::component::bindgen!({
    world: "app",
    path: "../../wit",
});

/// The host's side of the `app` world.
struct Host {
    /// The registry `list-entries` returns.
    entries: Vec<Entry>,
    /// The next window handle to assign. The hosts' base (10_000): the
    /// desktop's content slot takes a handle as its id, so a zero handle
    /// would serialize as an id-less node and drop out of the diff.
    next: u32,
    /// The (name, width, height) of each `open-entry` call, in order.
    opens: Mutex<Vec<(String, u32, u32)>>,
}

impl Default for Host {
    fn default() -> Self {
        Host {
            entries: Vec::new(),
            next: 10_000,
            opens: Mutex::new(Vec::new()),
        }
    }
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
    // The desktop closes via its own button, which calls this; a host-side
    // close (`on-window-close`) frees the sandbox before the call, so there
    // is nothing to free here.
    fn close_entry(&mut self, _handle: u32) {}
    // The cascade clamps to the viewport; the unknown size skips the clamp
    // (see the WIT note).
    fn viewport(&mut self) -> ViewportSize {
        ViewportSize { w: 0, h: 0 }
    }
}

// The `widget` interface is pulled into the world, so the generated
// bindings require this empty marker impl.
impl waui::ui::widget::Host for Host {}

/// The registry, as the hosts read it: the manifest at the repo root
/// (registry.json) through the shared reader, so this test parses the same
/// file the native hosts do.
fn manifest_entries() -> Vec<Entry> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("registry.json");
    host_shared::read_manifest(path.to_str().unwrap())
        .expect("failed to read registry.json")
        .into_iter()
        .map(|s| Entry {
            name: s.name,
            data: s.data,
        })
        .collect()
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

/// The desktop's contract, against one producer's component.
fn assert_desktop_roundtrip(app: &mut App, mut store: &mut Store<Host>) {
    // The initial view: the menu bar (0 windows) and the 3x3 grid.
    let nodes = app.call_view(&mut store).expect("view failed");
    let summaries: Vec<String> = nodes.iter().map(node_summary).collect();
    // The menu bar: "waui" and "0 windows".
    assert!(summaries.iter().any(|s| s == "text:None:waui"));
    assert!(summaries.iter().any(|s| s == "text:Some(4):0 windows"));
    // The nine tiles, each an image + label + lang (the manifest's
    // presentation), the icon the manifest's canonical path.
    assert!(summaries
        .iter()
        .any(|s| s == "image:None:site/icons/counter-rust.svg"));
    assert!(summaries.iter().any(|s| s == "text:None:Counter"));
    assert!(summaries.iter().any(|s| s == "text:None:Rust"));
    assert!(summaries
        .iter()
        .any(|s| s == "image:None:site/icons/todo-wat.svg"));

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
    assert_eq!(opens[0].0, "counter-rust");
    assert_eq!((opens[0].1, opens[0].2), (300, 380));
    drop(opens);

    // The diff: the count changes, the tile takes the selection ring, and
    // the window's six id'd nodes are added (window, title bar, spacer,
    // close, content slot, grip). The title-bar label is id-less, so the
    // diff cannot carry it (the WIT identity note) — the view does.
    let mutations = app.call_changes(&mut store).expect("changes failed");
    assert_eq!(
        mutations.len(),
        8,
        "the open's diff: the count, the tile, and the window's six id'd nodes"
    );
    let ids: Vec<Option<u32>> = mutations
        .iter()
        .map(|m| match m {
            waui::ui::widget::Mutation::Changed(c) => node_id(&c.node),
            waui::ui::widget::Mutation::Removed(_) => None,
        })
        .collect();
    assert!(ids.contains(&Some(4)), "the count is in the diff");
    assert!(ids.contains(&Some(100)), "the tile's ring is in the diff");
    // The window's nodes, under the handle the host assigned (10_000).
    for id in [10_000u32, 110_000, 210_000, 310_000, 410_000, 510_000] {
        assert!(
            ids.contains(&Some(id)),
            "the diff is missing window node {id}"
        );
    }
    // And the full view agrees: "1 window" (singular), and it carries the
    // id-less label the diff could not (the title bar's child).
    let nodes = app.call_view(&mut store).expect("view failed");
    let summaries: Vec<String> = nodes.iter().map(node_summary).collect();
    assert!(summaries.iter().any(|s| s == "text:Some(4):1 window"));
    let has_label = nodes.iter().any(|n| {
        matches!(
            n,
            waui::ui::widget::Node::Text(t)
                if t.content == "Counter" && t.parent == Some(210_000)
        )
    });
    assert!(has_label, "the view carries the id-less title-bar label");

    // Open a second window, then close the first: the count returns to 1.
    app.call_on_double_click(&mut store, 101)
        .expect("on-double-click failed");
    app.call_on_window_close(&mut store, 10_000)
        .expect("on-window-close failed");
    let nodes = app.call_view(&mut store).expect("view failed");
    let summaries: Vec<String> = nodes.iter().map(node_summary).collect();
    assert!(summaries.iter().any(|s| s == "text:Some(4):1 window"));

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

#[test]
fn desktop_grid_and_window_roundtrip() {
    let engine = Engine::default();
    let mut store = Store::new(
        &engine,
        Host {
            entries: manifest_entries(),
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
    assert_desktop_roundtrip(&mut app, &mut store);
}

// The same contract, against the AssemblyScript clone of the desktop.

#[test]
fn desktop_grid_and_window_roundtrip_as() {
    if !as_toolchain_available() {
        eprintln!("skipping: AssemblyScript toolchain not installed (run `npm install`)");
        return;
    }
    let engine = Engine::default();
    let mut store = Store::new(
        &engine,
        Host {
            entries: manifest_entries(),
            ..Default::default()
        },
    );
    let mut app = {
        let _guard = AS_BUILD_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        build_component_as("demo-desktop");
        instantiate(&engine, &mut store, &component_path_as("demo-desktop"))
    };
    assert_desktop_roundtrip(&mut app, &mut store);
}

// The same contract, against the hand-rolled WAT clone of the desktop.

#[test]
fn desktop_grid_and_window_roundtrip_wat() {
    if !wat_toolchain_available() {
        eprintln!("skipping: wasm-tools not on the PATH");
        return;
    }
    let engine = Engine::default();
    let mut store = Store::new(
        &engine,
        Host {
            entries: manifest_entries(),
            ..Default::default()
        },
    );
    let mut app = {
        let _guard = WAT_BUILD_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        build_component_wat("demo-desktop");
        instantiate(&engine, &mut store, &component_path_wat("demo-desktop"))
    };
    assert_desktop_roundtrip(&mut app, &mut store);
}
