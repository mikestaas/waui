//! Headless contract test: drive the counter component through the same
//! wasmtime host code the real hosts use, and assert on the flat node list
//! its `view()` returns. This is what makes the WIT world testable in CI
//! without a window.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use wasmtime::component::*;
use wasmtime::{Engine, Store};

wasmtime::component::bindgen!({
    world: "app",
    path: "../../wit",
});

struct Host;

impl AppImports for Host {
    fn now(&mut self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

// The `widget` interface is pulled into the world, so the generated
// bindings require this empty marker impl.
impl waui::ui::widget::Host for Host {}

fn waui_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn component_path() -> PathBuf {
    waui_root().join("target/components/demo-counter.component.wasm")
}

/// Build the counter component and lift it, so the test is self-contained.
fn build_component() {
    let root = waui_root();
    let wasm =
        root.join("target/wasm32-unknown-unknown/release/demo_counter.wasm");
    let out =
        root.join("target/components/demo-counter.component.wasm");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .expect("failed to create target/components");
    }

    let status = Command::new("cargo")
        .args([
            "build",
            "-p",
            "demo-counter",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
        ])
        .current_dir(&root)
        .status()
        .expect("failed to run cargo");
    assert!(status.success(), "failed to build demo-counter");

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
        "failed to lift demo-counter into a component"
    );
}

fn node_id(node: &waui::ui::widget::Node) -> u32 {
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
        .find(|n| node_id(n) == id)
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
        .find(|n| node_id(n) == id)
        .and_then(|n| match n {
            waui::ui::widget::Node::Text(t) => Some(&t.style),
            waui::ui::widget::Node::Button(b) => Some(&b.style),
            _ => None,
        })
}

#[test]
fn counter_roundtrip() {
    use waui::ui::widget::{Color, Size};

    build_component();

    let engine = Engine::default();
    let component =
        Component::from_file(&engine, component_path()).expect("failed to load component");

    let mut linker = Linker::new(&engine);
    App::add_to_linker::<_, HasSelf<_>>(&mut linker, |host| host)
        .expect("failed to link world imports");

    let mut store = Store::new(&engine, Host);
    let app = App::instantiate(&mut store, &component, &linker)
        .expect("failed to instantiate component");

    // Fresh counter starts at zero: the number is the host's default
    // color at its "large" size, and the buttons ask for their own
    // semantics.
    let nodes = app.call_view(&mut store).expect("view failed");
    assert_eq!(text_with_id(&nodes, 3), Some("0"));
    let style = style_of(&nodes, 3).expect("text node carries a style");
    assert_eq!(style.color, Color::Neutral);
    assert_eq!(style.size, Size::Large);
    assert_eq!(style_of(&nodes, 2).unwrap().color, Color::Positive);
    assert_eq!(style_of(&nodes, 4).unwrap().color, Color::Negative);

    // + + -
    app.call_on_click(&mut store, 2).expect("on-click failed");
    app.call_on_click(&mut store, 2).expect("on-click failed");
    app.call_on_click(&mut store, 4).expect("on-click failed");

    let nodes = app.call_view(&mut store).expect("view failed");
    assert_eq!(text_with_id(&nodes, 3), Some("1"));
    // Still non-negative: still the neutral color.
    assert_eq!(style_of(&nodes, 3).unwrap().color, Color::Neutral);

    // - - drives the value to -1: the component asks for the host's
    // "negative" tone, however the host chooses to render it.
    app.call_on_click(&mut store, 4).expect("on-click failed");
    app.call_on_click(&mut store, 4).expect("on-click failed");

    let nodes = app.call_view(&mut store).expect("view failed");
    assert_eq!(text_with_id(&nodes, 3), Some("-1"));
    assert_eq!(style_of(&nodes, 3).unwrap().color, Color::Negative);
}
