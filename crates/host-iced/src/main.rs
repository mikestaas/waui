//! The native iced host binary: a real window (winit) running a waui
//! component on the platform's default engine (Cranelift). The renderer,
//! the panels, and the `--dump` render live in the crate's library (shared
//! with the browser module); this binary is only the argument parsing, the
//! component load, and the interactive event loop.
//!
//! The host's behavior — the bootstrap, the component-requested refresh,
//! the injected clicks, the opened entries, and the headless `--dump` —
//! is documented by the library.
//!
//! `--entry <name=path>` registers an entry the host can run. `--click
//! <id>`, `--double-click <id>`, and `--close <handle>` inject events before
//! the frame or the interactive loop; `--dump <file.png>` renders one frame
//! to a PNG and exits, headless; `--now <base> <step>` fakes the host clock
//! (each call advances `step` ms).

use wasmtime::component::Component;
use wasmtime::Engine;

use host_iced::{
    apply_pending, canvas_size, new_state, render_png, subscription, theme, update, view,
    EntrySpec, Injected, State,
};

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

/// Load the component at the host's path, on the platform's default engine.
fn load(path: &str) -> (Engine, Component) {
    let engine = Engine::default();
    let component = Component::from_file(&engine, path).expect("failed to load component");
    (engine, component)
}

fn main() -> iced::Result {
    let args = parse_args();
    if let Some(file) = &args.dump {
        // Headless: apply the injected events, render one frame to a PNG,
        // and exit — no window, so it runs in CI.
        let (engine, component) = load(&args.path);
        let mut state = new_state(
            &engine,
            &component,
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
    let (engine, component) = load(&args.path);
    new_state(&engine, &component, args.entries, args.injected, args.now)
}

/// Render the current view to a PNG file: the library's render, written
/// where the page's canvas would be.
fn render_dump(state: &State, file: &str) -> iced::Result {
    let (_, _, png) = render_png(state);
    std::fs::write(file, &png).expect("failed to write the dump file");
    Ok(())
}
