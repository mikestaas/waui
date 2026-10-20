//! A TUI host for waui components.
//!
//! A third renderer for the same component: a real terminal UI (ratatui)
//! that lays the tree out with actual geometry — rows and columns, framed
//! buttons — and keeps a *host-local* focus ring. The component does not
//! know or care which button is highlighted; focus is a host concern.
//!
//! Controls: up/down (or j/k) move focus, enter/space clicks it, q quits.
//!
//! `--dump` renders one frame into an in-memory buffer and prints it with
//! ANSI color, so the tree-to-UI mapping — including the component's
//! semantic styling — can be verified without a terminal.
//!
//! The interactive loop follows the diff protocol: it bootstraps from
//! `view`, then after each click or tick it applies the component's
//! `changes` to the retained tree and rebuilds only when there is
//! something to apply. At rest — no ticks landing, no clicks — the loop
//! makes zero calls into the component; it just redraws the retained
//! tree (which ratatui's frame buffer diffs for free).

use std::io::{self, stdout};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wasmtime::component::*;
use wasmtime::{Engine, Store};

// The world's bindings and the node-list→tree rebuild live in the shared
// host core; this crate is only its renderer.
use host_shared::{build_tree, App, AppImports, Retained, Tree, waui};

// The contract's style tokens, aliased to avoid clashing with ratatui's
// `Color` and `Style`.
use host_shared::waui::ui::widget::{Color as WColor, Size as WSize, Style as WStyle};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use ratatui::{
    backend::{CrosstermBackend, TestBackend},
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};

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

// --------------------------------------------------------------------------
// Layout sizing: how much terminal space a tree needs.

fn width_of(node: &Tree) -> u16 {
    match node {
        Tree::Text(content, _) => content.chars().count() as u16,
        Tree::Button(_, label, _) => label.chars().count() as u16 + 2, // borders
        Tree::Row(children) => children.iter().map(width_of).sum(),
        Tree::Column(children) => children.iter().map(width_of).max().unwrap_or(0),
    }
}

fn height_of(node: &Tree) -> u16 {
    match node {
        Tree::Text(..) => 1,
        Tree::Button(..) => 3, // border top, label, border bottom
        Tree::Row(children) => children.iter().map(height_of).max().unwrap_or(0),
        Tree::Column(children) => children.iter().map(height_of).sum(),
    }
}

// --------------------------------------------------------------------------
// Styling: the component's semantic tokens, mapped onto terminal primitives.
// A TUI can't render `#f7768e`, but it can render "negative" — that is the
// point of tokens instead of pixels.

/// A semantic color as a terminal color.
fn tint(c: WColor) -> Color {
    match c {
        WColor::Neutral => Color::White,
        WColor::Accent => Color::Cyan,
        WColor::Positive => Color::Green,
        WColor::Negative => Color::Red,
        WColor::Warning => Color::Yellow,
    }
}

/// A node's style as a ratatui style: the semantic color, with the
/// semantic size faked as weight (a monospace terminal has no glyph scale).
fn node_style(s: &WStyle) -> Style {
    Style::default()
        .fg(tint(s.color))
        .add_modifier(match s.size {
            WSize::Small => Modifier::DIM,
            WSize::Medium => Modifier::empty(),
            WSize::Large => Modifier::BOLD,
        })
}

// --------------------------------------------------------------------------
// Rendering: the tree as ratatui widgets.

/// Map a node to a widget and render it into `area`.
fn render(frame: &mut Frame, node: &Tree, area: Rect, focus: Option<u32>) {
    match node {
        Tree::Text(content, style) => {
            frame.render_widget(
                Paragraph::new(content.clone())
                    .style(node_style(style))
                    .alignment(Alignment::Center),
                area,
            );
        }
        Tree::Button(id, label, style) => {
            // A display-only button (no id) can never be focused.
            let focused = *id == focus;
            // The component's semantic color is the button's base; the
            // host's focus ring (its own concern) overrides with its accent.
            let style = if focused {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                node_style(style)
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(style);
            frame.render_widget(
                Paragraph::new(label.clone())
                    .style(style)
                    .alignment(Alignment::Center)
                    .block(block),
                area,
            );
        }
        Tree::Row(children) => {
            let layout = Layout::horizontal(
                children
                    .iter()
                    .map(|c| Constraint::Length(width_of(c))),
            );
            for (child, child_area) in children.iter().zip(layout.split(area).iter()) {
                render(frame, child, *child_area, focus);
            }
        }
        Tree::Column(children) => {
            let layout = Layout::vertical(
                children
                    .iter()
                    .map(|c| Constraint::Length(height_of(c))),
            );
            for (child, child_area) in children.iter().zip(layout.split(area).iter()) {
                render(frame, child, *child_area, focus);
            }
        }
    }
}

/// A terminal color as an ANSI SGR foreground code, for `--dump`. Returns
/// `None` for colors a plain text dump cannot represent (reset, truecolor,
/// indexed).
fn sgr_fg(c: Color) -> Option<&'static str> {
    Some(match c {
        Color::Black => "\x1b[30m",
        Color::Red => "\x1b[31m",
        Color::Green => "\x1b[32m",
        Color::Yellow => "\x1b[33m",
        Color::Blue => "\x1b[34m",
        Color::Magenta => "\x1b[35m",
        Color::Cyan => "\x1b[36m",
        Color::Gray => "\x1b[37m",
        Color::DarkGray => "\x1b[90m",
        Color::LightRed => "\x1b[91m",
        Color::LightGreen => "\x1b[92m",
        Color::LightYellow => "\x1b[93m",
        Color::LightBlue => "\x1b[94m",
        Color::LightMagenta => "\x1b[95m",
        Color::LightCyan => "\x1b[96m",
        Color::White => "\x1b[97m",
        Color::Reset | Color::Rgb(..) | Color::Indexed(_) => return None,
    })
}

/// Draw one full frame: a hint bar, then the tree centered below it.
fn draw(frame: &mut Frame, tree: &Tree, focus: Option<u32>) {
    let area = frame.area();
    let bars: [Rect; 2] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    frame.render_widget(
        Paragraph::new(
            " waui (tui)   up/down or j/k: focus   enter/space: click   q: quit ",
        )
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Left),
        bars[0],
    );

    // Center the tree's bounding box in the remaining space.
    let w = width_of(tree).min(bars[1].width);
    let h = height_of(tree).min(bars[1].height);
    if w == 0 || h == 0 {
        return;
    }
    let x = bars[1].x + (bars[1].width - w) / 2;
    let y = bars[1].y + (bars[1].height - h) / 2;
    render(frame, tree, Rect::new(x, y, w, h), focus);
}

// --------------------------------------------------------------------------
// Focus: a host-local ring over the tree's buttons (in pre-order).

fn focusable_ids(node: &Tree, out: &mut Vec<u32>) {
    match node {
        // A button the component gave no id cannot be clicked, so it is
        // not focusable.
        Tree::Button(id, ..) => {
            if let Some(i) = id {
                out.push(*i);
            }
        }
        Tree::Row(children) | Tree::Column(children) => {
            for child in children {
                focusable_ids(child, out);
            }
        }
        Tree::Text(..) => {}
    }
}

// --------------------------------------------------------------------------

/// Restores the terminal on drop (including on panic).
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dump = args.iter().any(|a| a == "--dump");
    let mut clicks: Vec<u32> = Vec::new();
    let mut path: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dump" => {}
            "--click" => {
                if let Some(v) = args.get(i + 1) {
                    clicks.push(
                        v.parse().expect("--click takes a numeric widget id"),
                    );
                    i += 1;
                } else {
                    eprintln!("--click takes a numeric widget id");
                    std::process::exit(2);
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
            eprintln!("usage: host-tui [--dump] [--click <id>] <component.wasm>");
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

    for id in &clicks {
        app.call_on_click(&mut store, *id).expect("on-click() failed");
    }

    if dump {
        // Headless: render one frame into an in-memory buffer and print it.
        let nodes = app.call_view(&mut store).expect("view() failed");
        let tree = build_tree(&nodes);
        let (w, h) = (
            width_of(&tree).max(10) + 2,
            height_of(&tree) + 3,
        );
        let mut terminal = Terminal::new(TestBackend::new(w, h))
            .expect("failed to create terminal");
        let mut focus_ids = Vec::new();
        focusable_ids(&tree, &mut focus_ids);
        let focus = focus_ids.first().copied();
        let completed = terminal.draw(|f| draw(f, &tree, focus)).expect("draw failed");
        let buf = completed.buffer;
        let area = buf.area();
        // Print the buffer as text, prefixed with ANSI escapes for styled
        // cells, so the dump shows the component's semantic styling — the
        // same colors and weight the live terminal would render.
        for y in 0..area.height {
            let mut line = String::new();
            let mut styled = false;
            for x in 0..area.width {
                let (sym, fg, modifier) = match buf.cell((x, y)) {
                    Some(c) => (c.symbol(), c.fg, c.modifier),
                    None => (" ", Color::Reset, Modifier::empty()),
                };
                if let Some(code) = sgr_fg(fg) {
                    line.push_str(code);
                    styled = true;
                }
                if modifier.intersects(Modifier::BOLD | Modifier::DIM) {
                    line.push_str(
                        if modifier.contains(Modifier::BOLD) {
                            "\x1b[1m"
                        } else {
                            "\x1b[2m"
                        },
                    );
                    styled = true;
                }
                line.push_str(sym);
            }
            if styled {
                line.push_str("\x1b[0m");
            }
            // Trim trailing spaces so the dump reads like terminal output.
            println!("{}", line.trim_end());
        }
        return;
    }

    // Interactive.
    enable_raw_mode().expect("failed to enable raw mode");
    let _guard = TerminalGuard;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen).expect("failed to enter alt screen");
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))
        .expect("failed to create terminal");

    let tick = Duration::from_millis(250);
    let poll = Duration::from_millis(50);
    let mut last_tick = SystemTime::now();
    // The retained tree, as the diff protocol maintains it: a full
    // snapshot at startup, then `changes` applied after each event.
    let nodes = app.call_view(&mut store).expect("view() failed");
    let mut retained = Retained::from_view(nodes);
    let mut tree = build_tree(retained.nodes());
    let mut focus_idx = 0usize;
    let mut click_pending = false;
    // Whether the component was driven (click or tick) since the last
    // `changes` call.
    let mut pending = false;
    let mut quit = false;

    while !quit {
        // Read every pending event before acting on them.
        if event::poll(poll).expect("failed to poll events") {
            while event::poll(Duration::ZERO)
                .expect("failed to poll events")
            {
                match event::read().expect("failed to read event") {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => {
                                quit = true;
                            }
                            // Move the focus ring to the previous/next
                            // focusable widget (wrapping). The exact count
                            // is applied after the tree rebuild below.
                            KeyCode::Up | KeyCode::Char('k') => {
                                focus_idx = focus_idx.wrapping_sub(1);
                            }
                            KeyCode::Down
                            | KeyCode::Left
                            | KeyCode::Right
                            | KeyCode::Char('j') => {
                                focus_idx = focus_idx.wrapping_add(1);
                            }
                            KeyCode::Enter | KeyCode::Char(' ') => {
                                click_pending = true;
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }

        if SystemTime::now().duration_since(last_tick).unwrap_or(tick) >= tick
        {
            last_tick = SystemTime::now();
            app.call_on_tick(&mut store).expect("on-tick() failed");
            pending = true;
        }

        // Ask the component what changed; apply the diff to the retained
        // tree and rebuild only when it reported something. At rest this
        // whole block is skipped, so the loop makes no component calls.
        if pending {
            pending = false;
            let mutations = app
                .call_changes(&mut store)
                .expect("changes() failed");
            if retained.apply(&mutations) {
                tree = build_tree(retained.nodes());
            }
        }

        let mut focus_ids = Vec::new();
        focusable_ids(&tree, &mut focus_ids);
        if focus_ids.is_empty() {
            focus_idx = 0;
            click_pending = false;
        } else {
            // `focus_idx` may be one past the end (from wrapping) or
            // stale after a tree change; normalize into the button list.
            let n = focus_ids.len();
            focus_idx = ((focus_idx % n) + n) % n;
            if click_pending {
                click_pending = false;
                app.call_on_click(&mut store, focus_ids[focus_idx])
                    .expect("on-click() failed");
                pending = true;
            }
        }

        let focus = focus_ids.get(focus_idx).copied();
        let _ = terminal
            .draw(|f| draw(f, &tree, focus))
            .expect("failed to draw");
    }

    let _ = terminal.show_cursor().expect("failed to show cursor");
}
