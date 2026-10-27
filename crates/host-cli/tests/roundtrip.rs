//! Headless contract test: drive the counter and the stopwatch through the
//! same wasmtime host code the real hosts use, and assert on the flat node
//! list `view()` returns, its style, the diff channel, and the refresh
//! request (`schedule`: a 250ms period while the stopwatch runs, 0 while
//! it pauses; the counter never calls it).
//!
//! Each scenario runs against both producer languages: the Rust component
//! (cargo + wit-bindgen) and its AssemblyScript one (asc + the hand-rolled
//! ABI). The assertions are shared.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use wasmtime::component::*;
use wasmtime::{Engine, Store};

wasmtime::component::bindgen!({
    world: "app",
    path: "../../wit",
});

/// Records the component's refresh requests (the world's `schedule`
/// import), for the assertions below.
#[derive(Default)]
struct Host {
    last_period: u64,
    calls: u64,
}

impl AppImports for Host {
    fn now(&mut self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
    fn schedule(&mut self, period: u64) {
        self.last_period = period;
        self.calls += 1;
    }
}

// The `widget` interface is pulled into the world, so the generated
// bindings require this empty marker impl.
impl waui::ui::widget::Host for Host {}

fn waui_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn component_path(demo: &str) -> PathBuf {
    waui_root().join("target/components").join(format!("{demo}.component.wasm"))
}

fn component_path_as(demo: &str) -> PathBuf {
    waui_root().join("target/components").join(format!("{demo}-as.component.wasm"))
}

/// Build the component and lift it, so the test is self-contained. Cargo
/// names the artifact with `_` where the crate name has `-`.
fn build_component(demo: &str) {
    let root = waui_root();
    let wasm = root
        .join("target/wasm32-unknown-unknown/release")
        .join(format!("{}{}", demo.replace('-', "_"), ".wasm"));
    let out = component_path(demo);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .expect("failed to create target/components");
    }

    let status = Command::new("cargo")
        .args([
            "build",
            "-p",
            demo,
            "--release",
            "--target",
            "wasm32-unknown-unknown",
        ])
        .current_dir(&root)
        .status()
        .expect("failed to run cargo");
    assert!(status.success(), "failed to build {demo}");

    let status = Command::new("wasm-tools")
        .args([
            "component",
            "new",
            wasm.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
        ])
        .status()
        .expect("failed to run wasm-tools");
    assert!(
        status.success(),
        "failed to lift {demo} into a component"
    );
}

// The AssemblyScript build needs node + the local npm toolchain (asc);
// without it the AS tests skip rather than fail.
fn as_toolchain_available() -> bool {
    waui_root()
        .join("node_modules/.bin/asc")
        .exists()
        && Command::new("node").arg("--version").output().is_ok_and(|o| o.status.success())
}

// The tests share their built artifacts on disk, and run on parallel
// threads: the build *and* load window must be serialized per artifact
// file, so that one test's lift cannot clobber the file while a sibling
// test is reading it.
static RUST_BUILD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
static AS_BUILD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Build the AssemblyScript component and lift it:
/// asc (the incremental runtime) -> the `env.abort` patch -> export rename
/// -> component embed (the WIT) -> component new.
/// (Callers hold `AS_BUILD_LOCK` across the build *and* the load, so a
/// sibling test cannot lift over the file while we read it.)
fn build_component_as(demo: &str) {
    let root = waui_root();
    std::fs::create_dir_all(root.join("target/assembly"))
        .expect("failed to create target/assembly");
    std::fs::create_dir_all(root.join("target/components"))
        .expect("failed to create target/components");

    let src = format!("assembly/{demo}/index.ts");
    let raw = root.join("target/assembly").join(format!("{demo}.raw.wasm"));
    let core = root.join("target/assembly").join(format!("{demo}.core.wasm"));
    let renamed =
        root.join("target/assembly").join(format!("{demo}-as.wasm"));
    let embedded = root
        .join("target/assembly")
        .join(format!("{demo}-as.embedded.wasm"));
    let out = component_path_as(demo);

    let mut steps: Vec<Command> = Vec::new();
    {
        let mut c = Command::new("npx");
        c.args([
            "asc",
            src.as_str(),
            "--outFile",
            raw.to_str().unwrap(),
        ]);
        c.args(["-O", "--noAssert", "--runtime", "incremental", "--memoryBase", "65536", "--initialMemory", "2"]);
        steps.push(c);
    }
    {
        // The incremental runtime's one import (`env.abort`, its OOM path)
        // is replaced with an in-module `unreachable` so the lifted
        // component is import-free again (see assembly/patch-env-abort.mjs).
        let mut c = Command::new("node");
        c.args(["assembly/patch-env-abort.mjs", raw.to_str().unwrap(), core.to_str().unwrap()]);
        steps.push(c);
    }
    {
        let mut c = Command::new("node");
        c.args(["assembly/rename-exports.mjs", core.to_str().unwrap(), renamed.to_str().unwrap(), "onclick=on-click", "ontick=on-tick"]);
        steps.push(c);
    }
    {
        let mut c = Command::new("wasm-tools");
        c.args(["component", "embed", "wit/", renamed.to_str().unwrap(), "--output", embedded.to_str().unwrap()]);
        steps.push(c);
    }
    {
        let mut c = Command::new("wasm-tools");
        c.args(["component", "new", embedded.to_str().unwrap(), "--output", out.to_str().unwrap()]);
        steps.push(c);
    }
    for step in &mut steps {
        step.current_dir(&root);
        let status = step.status().expect("failed to run a build tool");
        assert!(
            status.success(),
            "failed to build the AssemblyScript {demo}"
        );
    }
}

/// Link and instantiate a component against our world, returning the live
/// binding. The engine and store stay in the caller (the store borrows the
/// engine, so only the owned `App` can be returned).
fn instantiate(engine: &Engine, store: &mut Store<Host>, path: &Path) -> App {
    let component =
        Component::from_file(engine, path).expect("failed to load component");

    let mut linker = Linker::new(engine);
    App::add_to_linker::<_, HasSelf<_>>(&mut linker, |host| host)
        .expect("failed to link world imports");

    App::instantiate(store, &component, &linker)
        .expect("failed to instantiate component")
}

fn node_id(node: &waui::ui::widget::Node) -> Option<u32> {
    match node {
        waui::ui::widget::Node::Text(t) => t.id,
        waui::ui::widget::Node::Button(b) => b.id,
        waui::ui::widget::Node::Container(c) => c.id,
    }
}

/// Find a node by id and return the content of a text node (if that's what
/// it is).
fn text_with_id(nodes: &[waui::ui::widget::Node], id: u32) -> Option<&str> {
    nodes
        .iter()
        .find(|n| node_id(n) == Some(id))
        .and_then(|n| match n {
            waui::ui::widget::Node::Text(t) => Some(t.content.as_str()),
            _ => None,
        })
}

/// Find a node by id and return its semantic style (text and button nodes
/// carry one; containers do not).
fn style_of(
    nodes: &[waui::ui::widget::Node],
    id: u32,
) -> Option<&waui::ui::widget::Style> {
    nodes
        .iter()
        .find(|n| node_id(n) == Some(id))
        .and_then(|n| match n {
            waui::ui::widget::Node::Text(t) => Some(&t.style),
            waui::ui::widget::Node::Button(b) => Some(&b.style),
            _ => None,
        })
}

/// The counter's full contract: initial rendering, click-driven content and
/// color changes, and the diff channel. Language-agnostic: it runs against
/// whichever producer language built the component.
fn assert_counter_roundtrip(app: &mut App, store: &mut Store<Host>) {
    use waui::ui::widget::{Color, Size};

    // Fresh counter starts at zero: the number is the host's default
    // color at its "large" size, and the buttons ask for their own
    // semantics.
    let nodes = app.call_view(&mut *store).expect("view failed");
    assert_eq!(text_with_id(&nodes, 3), Some("0"));
    let style = style_of(&nodes, 3).expect("text node carries a style");
    assert_eq!(style.color, Color::Neutral);
    assert_eq!(style.size, Size::Large);
    assert_eq!(style_of(&nodes, 2).unwrap().color, Color::Positive);
    assert_eq!(style_of(&nodes, 4).unwrap().color, Color::Negative);

    // + + -
    app.call_on_click(&mut *store, 2).expect("on-click failed");
    app.call_on_click(&mut *store, 2).expect("on-click failed");
    app.call_on_click(&mut *store, 4).expect("on-click failed");

    let nodes = app.call_view(&mut *store).expect("view failed");
    assert_eq!(text_with_id(&nodes, 3), Some("1"));
    // Still non-negative: still the neutral color.
    assert_eq!(style_of(&nodes, 3).unwrap().color, Color::Neutral);

    // - - drives the value to -1: the component asks for the host's
    // "negative" tone, however the host chooses to render it.
    app.call_on_click(&mut *store, 4).expect("on-click failed");
    app.call_on_click(&mut *store, 4).expect("on-click failed");

    let nodes = app.call_view(&mut *store).expect("view failed");
    assert_eq!(text_with_id(&nodes, 3), Some("-1"));
    assert_eq!(style_of(&nodes, 3).unwrap().color, Color::Negative);

    // The diff channel: the component reports only what it changed. A
    // `view()` call above reset its dirty state, so the next click is the
    // only pending change.
    app.call_on_click(&mut *store, 4).expect("on-click failed"); // 0 -> ...-2
    let mutations = app.call_changes(&mut *store).expect("changes failed");
    assert_eq!(mutations.len(), 1, "one click should dirty exactly one node");
    match &mutations[0] {
        waui::ui::widget::Mutation::Changed(node) => {
            assert_eq!(node_id(node), Some(3));
            if let waui::ui::widget::Node::Text(t) = node {
                assert_eq!(t.content, "-2");
                assert_eq!(t.style.color, Color::Negative);
                assert_eq!(t.style.size, Size::Large);
            } else {
                panic!("expected the text readout, got another node kind");
            }
        }
        waui::ui::widget::Mutation::Removed(_) => {
            panic!("the counter never removes nodes")
        }
    }

    // No-op events (an unknown click, a tick) produce an empty diff.
    app.call_on_click(&mut *store, 99).expect("on-click failed");
    assert!(
        app.call_changes(&mut *store).expect("changes failed").is_empty(),
        "an unknown click must produce no changes"
    );
    app.call_on_tick(&mut *store).expect("on-tick failed");
    assert!(
        app.call_changes(&mut *store).expect("changes failed").is_empty(),
        "a no-op tick must produce no changes"
    );

    // And climbing back over zero: the color flips with the content, and
    // it is still one mutation — the readout.
    app.call_on_click(&mut *store, 2).expect("on-click failed"); // -2 -> -1
    app.call_on_click(&mut *store, 2).expect("on-click failed"); // -1 -> 0
    let mutations = app.call_changes(&mut *store).expect("changes failed");
    assert_eq!(mutations.len(), 1);
    match &mutations[0] {
        waui::ui::widget::Mutation::Changed(node) => {
            if let waui::ui::widget::Node::Text(t) = node {
                assert_eq!(t.content, "0");
                assert_eq!(t.style.color, Color::Neutral);
            } else {
                panic!("expected the text readout, got another node kind");
            }
        }
        _ => panic!("expected a changed node"),
    }

    // The counter changes only on a click: it never requests a refresh, so
    // a host that honors `schedule` never wakes it.
    assert_eq!(store.data().calls, 0, "a counter never calls schedule");
}

/// A host that never calls view() still gets the whole tree from the first
/// changes() call: before anything has been exported, every node is new to
/// the host.
fn assert_changes_bootstrap(app: &mut App, store: &mut Store<Host>) {
    let mutations = app.call_changes(&mut *store).expect("changes failed");
    assert_eq!(
        mutations.len(),
        4,
        "first-ever changes() should export the whole tree"
    );
    assert!(
        mutations
            .iter()
            .all(|m| matches!(m, waui::ui::widget::Mutation::Changed(_))),
        "bootstrap mutations should all be `changed`"
    );

    // A second call, with nothing changed, is empty.
    assert!(
        app.call_changes(&mut *store).expect("changes failed").is_empty(),
        "second changes() with no events should be empty"
    );
}

/// The stopwatch's contract: the refresh request — a 250ms period while
/// running, cancelled on pause — and the wake behavior it drives.
fn assert_stopwatch_roundtrip(app: &mut App, store: &mut Store<Host>) {
    use std::thread::sleep;
    use std::time::Duration;
    use waui::ui::widget::Color;

    // Fresh: paused, reading 0 ms — and no refresh requested: a host that
    // honors `schedule` never wakes a paused stopwatch.
    let nodes = app.call_view(&mut *store).expect("view failed");
    assert_eq!(text_with_id(&nodes, 2), Some("0 ms"));
    assert_eq!(
        store.data().calls,
        0,
        "a paused stopwatch requests no refresh"
    );

    // Start: the component requests the 250ms refresh.
    app.call_on_click(&mut *store, 3).expect("on-click failed");
    app.call_changes(&mut *store).expect("changes failed");
    assert_eq!(
        store.data().last_period, 250,
        "a running stopwatch requests a 250ms refresh"
    );

    // The host-side consequence: time passing shows up, on a wake, as the
    // one changed node — the readout.
    sleep(Duration::from_millis(400));
    app.call_on_tick(&mut *store).expect("on-tick failed");
    let mutations = app.call_changes(&mut *store).expect("changes failed");
    assert_eq!(
        mutations.len(),
        1,
        "a running wake should report the readout only"
    );
    match &mutations[0] {
        waui::ui::widget::Mutation::Changed(node) => {
            if let waui::ui::widget::Node::Text(t) = node {
                let ms = t
                    .content
                    .strip_suffix(" ms")
                    .unwrap()
                    .parse::<u64>()
                    .unwrap();
                assert!(
                    ms >= 200 && ms < 2000,
                    "unexpected elapsed time: {ms} ms"
                );
                assert_eq!(t.style.color, Color::Accent);
            } else {
                panic!("expected the text readout, got another node kind");
            }
        }
        _ => panic!("expected a changed node"),
    }

    // Pause: the readout and the button change, and the refresh is
    // cancelled...
    app.call_on_click(&mut *store, 3).expect("on-click failed");
    let mutations = app.call_changes(&mut *store).expect("changes failed");
    assert_eq!(
        mutations.len(),
        2,
        "pausing changes the readout and the button"
    );
    assert_eq!(
        store.data().last_period, 0,
        "a paused stopwatch cancels the refresh"
    );

    // ...and a further wake is empty (zero diff traffic).
    app.call_on_tick(&mut *store).expect("on-tick failed");
    assert!(
        app.call_changes(&mut *store).expect("changes failed").is_empty(),
        "a paused stopwatch must produce no changes"
    );
}

#[test]
fn counter_roundtrip() {
    let engine = Engine::default();
    let mut store = Store::new(&engine, Host::default());
    let mut app = {
        let _guard = RUST_BUILD_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        build_component("demo-counter");
        instantiate(&engine, &mut store, &component_path("demo-counter"))
    };
    assert_counter_roundtrip(&mut app, &mut store);
}

#[test]
fn changes_bootstraps_without_view() {
    let engine = Engine::default();
    let mut store = Store::new(&engine, Host::default());
    let mut app = {
        let _guard = RUST_BUILD_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        build_component("demo-counter");
        instantiate(&engine, &mut store, &component_path("demo-counter"))
    };
    assert_changes_bootstrap(&mut app, &mut store);
}

// The same two contracts, against the AssemblyScript clone of the counter.

#[test]
fn counter_roundtrip_as() {
    if !as_toolchain_available() {
        eprintln!("skipping: AssemblyScript toolchain not installed (run `npm install`)");
        return;
    }
    let engine = Engine::default();
    let mut store = Store::new(&engine, Host::default());
    let mut app = {
        let _guard = AS_BUILD_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        build_component_as("demo-counter");
        instantiate(&engine, &mut store, &component_path_as("demo-counter"))
    };
    assert_counter_roundtrip(&mut app, &mut store);
}

#[test]
fn changes_bootstraps_without_view_as() {
    if !as_toolchain_available() {
        eprintln!("skipping: AssemblyScript toolchain not installed (run `npm install`)");
        return;
    }
    let engine = Engine::default();
    let mut store = Store::new(&engine, Host::default());
    let mut app = {
        let _guard = AS_BUILD_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        build_component_as("demo-counter");
        instantiate(&engine, &mut store, &component_path_as("demo-counter"))
    };
    assert_changes_bootstrap(&mut app, &mut store);
}

// And the stopwatch's refresh-request contract, both languages.

#[test]
fn stopwatch_roundtrip() {
    let engine = Engine::default();
    let mut store = Store::new(&engine, Host::default());
    let mut app = {
        let _guard = RUST_BUILD_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        build_component("demo-stopwatch");
        instantiate(&engine, &mut store, &component_path("demo-stopwatch"))
    };
    assert_stopwatch_roundtrip(&mut app, &mut store);
}

#[test]
fn stopwatch_roundtrip_as() {
    if !as_toolchain_available() {
        eprintln!("skipping: AssemblyScript toolchain not installed (run `npm install`)");
        return;
    }
    let engine = Engine::default();
    let mut store = Store::new(&engine, Host::default());
    let mut app = {
        let _guard = AS_BUILD_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        build_component_as("demo-stopwatch");
        instantiate(&engine, &mut store, &component_path_as("demo-stopwatch"))
    };
    assert_stopwatch_roundtrip(&mut app, &mut store);
}
