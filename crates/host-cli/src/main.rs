//! A minimal terminal host for waui components: renders `view()` as an
//! ASCII tree and forwards events read from standard input: a numeric
//! line clicks the widget with that id, any other non-empty line is the
//! whole buffer of the first text input in pre-order (`on-text`).
//!
//! Ignores the diff channel — it re-queries `view()` after every event,
//! which stays correct because `view()` is always the full tree — and
//! ignores `style` (plain ASCII has no palette) and `align` (an indented
//! list has no cross axis to place on). Treats `schedule` as advisory too:
//! it has no timer, so a time-based readout advances only when a click
//! re-queries `view()`.

use std::io::{self, BufRead, Write};
use std::time::{SystemTime, UNIX_EPOCH};
use wasmtime::component::*;
use wasmtime::{Engine, Store};

// The world's bindings and the node-list→tree rebuild live in the shared
// host core; this crate is only its renderer.
use host_shared::{build_tree, waui, App, AppImports, Tree};

/// The host's implementation of the world's imports.
struct Host;

impl AppImports for Host {
    fn now(&mut self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
    // This host has no timer: the request is ignored (see the header).
    fn schedule(&mut self, _period: u64) {}
}

// The `widget` interface is pulled into the world (via `use widget.{...}`),
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
    let component = Component::from_file(&engine, &path).expect("failed to load component");

    let mut linker = Linker::new(&engine);
    App::add_to_linker::<_, HasSelf<_>>(&mut linker, |host| host)
        .expect("failed to link world imports");

    let mut store = Store::new(&engine, Host);
    let app =
        App::instantiate(&mut store, &component, &linker).expect("failed to instantiate component");

    println!("waui (terminal host) rendering: {path}");
    println!("type the id of a widget to click it, text to fill the first input, or q to quit");

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
        } else if !line.is_empty() {
            // A non-numeric line is the whole buffer of the first text
            // input (in pre-order), as `on-text` takes it.
            if let Some(id) = first_input(&tree) {
                app.call_on_text(&mut store, id, line)
                    .expect("on-text() failed");
            }
        }
    }
}

/// The first text input with an id, in depth-first pre-order — the
/// target of a typed (non-numeric) line.
fn first_input(node: &Tree) -> Option<u32> {
    match node {
        Tree::Input(id, _, _) => *id,
        Tree::Row(_, children) | Tree::Column(_, children) => children.iter().find_map(first_input),
        _ => None,
    }
}

/// Render the tree as indented ASCII, ignoring the nodes' `style` (plain
/// ASCII has no palette).
fn render(node: &Tree, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    match node {
        Tree::Text(content, _) => {
            out.push_str(&format!("{indent}\"{content}\"\n"));
        }
        Tree::Button(id, label, _) => {
            // A button with no id is display-only; no id prefix.
            let id = id.map_or_else(String::new, |i| format!("{i}: "));
            out.push_str(&format!("{indent}[{id}{label}]\n"));
        }
        Tree::Input(id, content, _) => {
            // The quotes mark an editable field (a button's label has none);
            // a field with no id is display-only, like an id-less button.
            let id = id.map_or_else(String::new, |i| format!("{i}: "));
            out.push_str(&format!("{indent}[{id}\"{content}\"]\n"));
        }
        Tree::Row(_, children) => {
            out.push_str(&format!("{indent}row:\n"));
            for child in children {
                render(child, depth + 1, out);
            }
        }
        Tree::Column(_, children) => {
            out.push_str(&format!("{indent}column:\n"));
            for child in children {
                render(child, depth + 1, out);
            }
        }
    }
}
