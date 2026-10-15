//! A terminal host for waui components.
//!
//! Loads a component, renders its `view()` as an ASCII tree, and forwards
//! widget clicks read from standard input. This is a second, trivially
//! different renderer for the same component — the point being that the
//! component doesn't know or care which host it runs in.

use std::io::{self, BufRead, Write};
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

fn main() {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: host-cli <component.wasm>");
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

    println!("waui (terminal host) rendering: {path}");
    println!("type the id of a widget to click it, or q to quit");

    let stdin = io::stdin();
    loop {
        let nodes = app.call_view(&mut store).expect("view() failed");
        let tree = build_tree(&nodes);
        let mut out = String::new();
        render(&tree, 0, &mut out);
        print!("\n{out}");
        print!("> ");
        io::stdout().flush().expect("failed to flush stdout");

        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let line = line.trim();
        if line == "q" {
            break;
        }
        if let Ok(id) = line.parse::<u32>() {
            app.call_on_click(&mut store, id)
                .expect("on-click() failed");
        }
    }
}

/// Render the tree as indented ASCII.
fn render(node: &Tree, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    match node {
        Tree::Text(content) => {
            out.push_str(&format!("{indent}\"{content}\"\n"));
        }
        Tree::Button(id, label) => {
            out.push_str(&format!("{indent}[{id}: {label}]\n"));
        }
        Tree::Row(children) => {
            out.push_str(&format!("{indent}row:\n"));
            for child in children {
                render(child, depth + 1, out);
            }
        }
        Tree::Column(children) => {
            out.push_str(&format!("{indent}column:\n"));
            for child in children {
                render(child, depth + 1, out);
            }
        }
    }
}
