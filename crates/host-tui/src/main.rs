//! The native TUI host binary: a real terminal (crossterm) running a waui
//! component on the platform's default engine (Cranelift). The renderer,
//! focus ring, and `--dump` frame live in the crate's library (shared with
//! the browser module); this binary is only the argument parsing, the
//! component load, and the interactive event loop.
//!
//! With a registry (`--entry name=path`, repeatable) the host is a desktop:
//! a multi-document interface of overlapping windows (see the library docs).
//!
//! Controls: up/down (or j/k) move the active surface's focus ring (the
//! desktop, or the topmost window), enter/space activates the focused
//! widget (enter on a tile opens its entry), printable keys type into the
//! focused text input, tab raises and shift+tab lowers the active window,
//! esc closes it, q quits.
//! `--dump` renders one frame with ANSI color and prints it, for headless
//! verification of the tree-to-UI mapping; `--width <n>` sizes the `--dump`
//! buffer to n columns (the hint bar wraps, as on a narrow terminal);
//! `--click <id>`, `--double-click <id>`, and `--close <handle>` inject
//! events before the frame or the interactive loop; `--now <base> <step>`
//! fakes the host clock (each call advances `step` ms), so a `--dump` frame
//! that reads the clock is deterministic.

use std::io::{self, stdout};
use std::time::Duration;

use wasmtime::component::*;
use wasmtime::{Engine, Store};

// The world's bindings and the node-list→tree rebuild live in the shared
// host core; this crate is only its renderer.
use host_shared::{build_tree, App, Retained};
// The renderer, focus ring, and `--dump` frame, shared with the browser
// module.
use host_tui::{
    active_app, active_surface, active_target, active_tree, draw, dump_frame, input_content,
    now_ms, report_input, reseat_active, EntrySpec, Host, Surface,
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

/// Restores the terminal on drop (including on panic).
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dump = args.iter().any(|a| a == "--dump");
    let mut width: Option<u16> = None;
    // Events injected before the frame or the interactive loop, in argument
    // order, like a pointer and keyboard would drive them.
    enum Injected {
        Click(u32),
        DoubleClick(u32),
        Close(u32),
    }
    let mut injected: Vec<Injected> = Vec::new();
    let mut entries: Vec<EntrySpec> = Vec::new();
    let mut fake_now: Option<(u64, u64)> = None;
    let mut path: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dump" => {}
            "--width" => {
                if let Some(v) = args.get(i + 1) {
                    let w: u16 = v.parse().expect("--width takes a numeric column count");
                    if w == 0 {
                        eprintln!("--width must be at least 1");
                        std::process::exit(2);
                    }
                    width = Some(w);
                    i += 1;
                } else {
                    eprintln!("--width takes a numeric column count");
                    std::process::exit(2);
                }
            }
            "--click" => {
                if let Some(v) = args.get(i + 1) {
                    injected.push(Injected::Click(
                        v.parse().expect("--click takes a numeric widget id"),
                    ));
                    i += 1;
                } else {
                    eprintln!("--click takes a numeric widget id");
                    std::process::exit(2);
                }
            }
            "--double-click" => {
                if let Some(v) = args.get(i + 1) {
                    injected.push(Injected::DoubleClick(
                        v.parse().expect("--double-click takes a numeric widget id"),
                    ));
                    i += 1;
                } else {
                    eprintln!("--double-click takes a numeric widget id");
                    std::process::exit(2);
                }
            }
            "--close" => {
                if let Some(v) = args.get(i + 1) {
                    injected.push(Injected::Close(
                        v.parse().expect("--close takes a numeric window handle"),
                    ));
                    i += 1;
                } else {
                    eprintln!("--close takes a numeric window handle");
                    std::process::exit(2);
                }
            }
            "--entry" => {
                // `name=path`: a registry entry (the name is the tile's
                // label fallback; the path is the component to open).
                match args.get(i + 1) {
                    Some(v) if v.contains('=') => {
                        let (name, p) = v.split_once('=').unwrap();
                        if !name.is_empty() && !p.is_empty() {
                            entries.push(EntrySpec {
                                name: name.to_string(),
                                path: p.to_string(),
                            });
                            i += 1;
                        } else {
                            eprintln!("--entry takes name=path");
                            std::process::exit(2);
                        }
                    }
                    _ => {
                        eprintln!("--entry takes name=path");
                        std::process::exit(2);
                    }
                }
            }
            "--now" => {
                let (b, s) = (args.get(i + 1), args.get(i + 2));
                match (b, s) {
                    (Some(b), Some(s)) => {
                        fake_now = Some((
                            b.parse().expect("--now takes a numeric base and step"),
                            s.parse().expect("--now takes a numeric base and step"),
                        ));
                        i += 2;
                    }
                    _ => {
                        eprintln!("--now takes a numeric base and step (milliseconds)");
                        std::process::exit(2);
                    }
                }
            }
            other => {
                if !other.starts_with('-') {
                    path = Some(other.to_string());
                }
            }
        }
        i += 1;
    }
    let path = match path {
        Some(p) => p,
        None => {
            eprintln!(
                "usage: host-tui [--dump] [--width <n>] [--entry name=path] [--click <id>] [--double-click <id>] [--close <handle>] [--now <base> <step>] <component.wasm>"
            );
            std::process::exit(2);
        }
    };

    let engine = Engine::default();
    let component = Component::from_file(&engine, &path).expect("failed to load component");

    let mut linker = Linker::new(&engine);
    App::add_to_linker::<_, HasSelf<_>>(&mut linker, |host| host)
        .expect("failed to link world imports");

    let mut host = Host::default();
    host.engine = engine.clone();
    host.linker = linker.clone();
    host.entries = entries;
    host.fake_now = fake_now;
    let mut store = Store::new(&engine, host);
    let app =
        App::instantiate(&mut store, &component, &linker).expect("failed to instantiate component");
    // The loop's handle on the component's refresh request.
    let wake = store.data().wake.clone();
    // A non-empty registry makes this host a desktop: the tile rule and the
    // windowing keys apply.
    let has_registry = !store.data().entries.is_empty();

    // The startup snapshot (the WIT bootstrap): the host builds its
    // retained tree from view. The desktop fetches its registry on first
    // view, so this precedes the injected events — a double-click on a
    // tile looks up the registry the first view fetched.
    let nodes = app.call_view(&mut store).expect("view() failed");
    let mut retained = Retained::from_view(nodes);
    let mut tree = build_tree(retained.nodes());

    // The injected events, in argument order, like a pointer and keyboard
    // would drive them. A close drops the window and tells the component
    // (it drops the handle and updates the count).
    for inj in &injected {
        match inj {
            Injected::Click(id) => {
                app.call_on_click(&mut store, *id)
                    .expect("on-click() failed");
            }
            Injected::DoubleClick(id) => {
                app.call_on_double_click(&mut store, *id)
                    .expect("on-double-click() failed");
            }
            Injected::Close(handle) => {
                let w = store.data_mut();
                w.windows.retain(|win| win.handle != *handle);
                app.call_on_window_close(&mut store, *handle)
                    .expect("on-window-close() failed");
            }
        }
    }
    // The events may have changed the component's state (a double-click
    // opens a window, a click changes a readout): re-query the full
    // snapshot. view is always the whole tree, so this stays correct; the
    // diff is an optimization the interactive loop uses instead.
    if !injected.is_empty() {
        let nodes = app.call_view(&mut store).expect("view() failed");
        retained = Retained::from_view(nodes);
        tree = build_tree(retained.nodes());
    }

    if dump {
        // Headless: render one frame into a memory buffer and print it.
        // `--width` sizes the buffer like a terminal of that width; by
        // default it fits the desktop whole and the windows' cascade.
        print!(
            "{}",
            dump_frame(&tree, &mut store.data_mut().windows, width, has_registry)
        );
        return;
    }

    // Interactive.
    enable_raw_mode().expect("failed to enable raw mode");
    let _guard = TerminalGuard;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen).expect("failed to enter alt screen");
    let mut terminal =
        Terminal::new(CrosstermBackend::new(stdout)).expect("failed to create terminal");

    let poll = Duration::from_millis(50);
    // `retained` and `tree` are the startup snapshot above, as the diff
    // protocol maintains them: `changes` applied after each event.
    // The active surface: 0 = the desktop, 1.. = a window's z-order index.
    let mut active = 0usize;
    let mut desktop_surf = Surface::default();
    // Whether the desktop's component was driven (a click, a double-click,
    // or a tick) since the last `changes` call.
    let mut pending = false;
    // The active surface's activation, fired after the targets re-seat.
    let mut click_pending = false;
    let mut double_click_pending = false;
    let mut quit = false;
    // The host's copy of the focused input's buffer (the component owns the
    // field, per the WIT): re-synced from the active surface's tree each
    // iteration; only the keystrokes between syncs are local.
    let mut input_buf = String::new();

    while !quit {
        // Read every pending event before acting on them.
        if event::poll(poll).expect("failed to poll events") {
            while event::poll(Duration::ZERO).expect("failed to poll events") {
                match event::read().expect("failed to read event") {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        let shift = key.modifiers.intersects(KeyModifiers::SHIFT);
                        // The active surface's ring target, from the last
                        // re-seat: it decides what the key does (a 'q'
                        // typed into the input types; only outside it
                        // quits).
                        let (t_id, t_is_input, t_is_tile) =
                            active_target(&mut store, &mut desktop_surf, active);
                        match key.code {
                            KeyCode::Esc => {
                                if active > 0 {
                                    // Close the active window: drop it and
                                    // tell the desktop (it drops the
                                    // handle and updates the count).
                                    let w = store.data_mut();
                                    let win = w.windows.remove(active - 1);
                                    app.call_on_window_close(&mut store, win.handle)
                                        .expect("on-window-close() failed");
                                    pending = true;
                                    active = (active - 1).min(store.data().windows.len());
                                } else {
                                    quit = true;
                                }
                                reseat_active(
                                    &mut store,
                                    &tree,
                                    has_registry,
                                    &mut active,
                                    &mut desktop_surf,
                                );
                            }
                            // Raise the next surface (wrapping to the
                            // desktop); shift+tab lowers the active window
                            // to the bottom, and the surface below it takes
                            // its slot, so the active index holds.
                            KeyCode::Tab => {
                                if shift {
                                    if active > 0 {
                                        let w = store.data_mut();
                                        let win = w.windows.remove(active - 1);
                                        w.windows.insert(0, win);
                                    }
                                } else {
                                    let n = store.data().windows.len();
                                    if n > 0 {
                                        active = (active + 1) % (n + 1);
                                        if active > 0 {
                                            let w = store.data_mut();
                                            let win = w.windows.remove(active - 1);
                                            w.windows.push(win);
                                            active = store.data().windows.len();
                                        }
                                    }
                                }
                                reseat_active(
                                    &mut store,
                                    &tree,
                                    has_registry,
                                    &mut active,
                                    &mut desktop_surf,
                                );
                            }
                            // Move the focus ring to the previous/next
                            // focusable widget (wrapping). The exact count
                            // is applied after the tree rebuild below.
                            KeyCode::Up | KeyCode::Char('k') => {
                                let surf = active_surface(&mut store, &mut desktop_surf, active);
                                surf.focus_idx = surf.focus_idx.wrapping_sub(1);
                            }
                            KeyCode::Down | KeyCode::Left | KeyCode::Right | KeyCode::Char('j') => {
                                let surf = active_surface(&mut store, &mut desktop_surf, active);
                                surf.focus_idx = surf.focus_idx.wrapping_add(1);
                            }
                            // Enter/space activates the focused widget: a
                            // tile opens its entry (a synthesized
                            // double-activation, the host's input model has
                            // no double-click), a button is clicked; an
                            // input has no activation.
                            KeyCode::Enter | KeyCode::Char(' ') => {
                                if t_is_tile {
                                    double_click_pending = true;
                                } else if !t_is_input {
                                    click_pending = true;
                                }
                            }
                            // The focused input's buffer: edited here,
                            // reported whole through `on-text`, and
                            // re-synced from the tree after the component
                            // answers.
                            KeyCode::Backspace => {
                                if t_is_input {
                                    input_buf.pop();
                                    let (a, s) = active_app(&mut store, &app, active);
                                    report_input(a, s, t_id, &input_buf, &mut pending);
                                }
                            }
                            KeyCode::Char(c) => {
                                if t_is_input {
                                    input_buf.push(c);
                                    let (a, s) = active_app(&mut store, &app, active);
                                    report_input(a, s, t_id, &input_buf, &mut pending);
                                } else if c == 'q' {
                                    quit = true;
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }

        // The components' refresh requests (0 = never): wake when the
        // period has elapsed; each `schedule` call restarts it.
        let now = now_ms();
        {
            let mut w = wake.lock().unwrap();
            if w.rate_ms > 0 && now >= w.last_wake_ms + w.rate_ms {
                w.last_wake_ms = now;
                app.call_on_tick(&mut store).expect("on-tick() failed");
                pending = true;
            }
        }
        for win in store.data_mut().windows.iter_mut() {
            let mut w = win.wake.lock().unwrap();
            if w.rate_ms > 0 && now >= w.last_wake_ms + w.rate_ms {
                w.last_wake_ms = now;
                win.app
                    .call_on_tick(&mut win.store)
                    .expect("on-tick() failed");
                win.pending = true;
            }
        }

        // Apply the diffs to the retained trees; rebuild only when one
        // reported something (an empty diff skips the rebuild).
        if pending {
            pending = false;
            let mutations = app.call_changes(&mut store).expect("changes() failed");
            if retained.apply(&mutations) {
                tree = build_tree(retained.nodes());
            }
        }
        for win in store.data_mut().windows.iter_mut() {
            if win.pending {
                win.pending = false;
                let mutations = win
                    .app
                    .call_changes(&mut win.store)
                    .expect("changes() failed");
                if win.retained.apply(&mutations) {
                    win.tree = build_tree(win.retained.nodes());
                }
            }
        }

        // Re-seat the active surface's ring (a rebuilt tree moves its
        // widgets), re-sync the focused input, and fire the pending
        // activation.
        reseat_active(
            &mut store,
            &tree,
            has_registry,
            &mut active,
            &mut desktop_surf,
        );
        {
            let (n, t_id, t_is_input, t_is_tile) = {
                let surf = active_surface(&mut store, &mut desktop_surf, active);
                (
                    surf.targets.len(),
                    surf.targets.get(surf.focus_idx).map(|t| t.id).unwrap_or(0),
                    surf.targets
                        .get(surf.focus_idx)
                        .map(|t| t.is_input)
                        .unwrap_or(false),
                    surf.targets
                        .get(surf.focus_idx)
                        .map(|t| t.is_tile)
                        .unwrap_or(false),
                )
            };
            if n == 0 {
                click_pending = false;
                double_click_pending = false;
            } else {
                if t_is_input {
                    if let Some(content) = input_content(active_tree(&store, &tree, active), t_id) {
                        input_buf = content.to_string();
                    }
                }
                if click_pending {
                    click_pending = false;
                    // An input has no activation; only a button is clicked.
                    if !t_is_input {
                        let (a, s) = active_app(&mut store, &app, active);
                        a.call_on_click(s, t_id).expect("on-click() failed");
                        pending = true;
                    }
                }
                if double_click_pending {
                    double_click_pending = false;
                    if t_is_tile {
                        let before = store.data().windows.len();
                        app.call_on_double_click(&mut store, t_id)
                            .expect("on-double-click() failed");
                        pending = true;
                        // The tile's entry opened a window: it is the new
                        // active surface.
                        if store.data().windows.len() > before {
                            active = store.data().windows.len();
                        }
                    }
                }
            }
        }

        let _ = terminal
            .draw(|f| {
                draw(
                    f,
                    &tree,
                    desktop_surf.focus,
                    &store.data().windows,
                    active,
                    has_registry,
                )
            })
            .expect("failed to draw");
    }

    let _ = terminal.show_cursor().expect("failed to show cursor");
}
