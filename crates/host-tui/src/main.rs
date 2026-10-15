//! A TUI host for waui components.
//!
//! A third renderer for the same component: a real terminal UI (ratatui)
//! that lays the tree out with actual geometry — rows and columns, framed
//! buttons — and keeps a *host-local* focus ring. The component does not
//! know or care which button is highlighted; focus is a host concern.
//!
//! Controls: up/down (or j/k) move focus, enter/space clicks it, q quits.
//!
//! `--dump` renders one frame into an in-memory buffer and prints it, so
//! the tree-to-UI mapping can be verified without a terminal.

use std::io::{self, stdout};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wasmtime::component::*;
use wasmtime::{Engine, Store};

wasmtime::component::bindgen!({
    world: "app",
    path: "../../wit",
});

mod tree;
use tree::{build_tree, Tree};

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
        Tree::Text(content) => content.chars().count() as u16,
        Tree::Button(_, label) => label.chars().count() as u16 + 2, // borders
        Tree::Row(children) => children.iter().map(width_of).sum(),
        Tree::Column(children) => children.iter().map(width_of).max().unwrap_or(0),
    }
}

fn height_of(node: &Tree) -> u16 {
    match node {
        Tree::Text(_) => 1,
        Tree::Button(..) => 3, // border top, label, border bottom
        Tree::Row(children) => children.iter().map(height_of).max().unwrap_or(0),
        Tree::Column(children) => children.iter().map(height_of).sum(),
    }
}

// --------------------------------------------------------------------------
// Rendering: the tree as ratatui widgets.

/// Map a node to a widget and render it into `area`.
fn render(frame: &mut Frame, node: &Tree, area: Rect, focus: Option<u32>) {
    match node {
        Tree::Text(content) => {
            frame.render_widget(
                Paragraph::new(content.clone())
                    .style(Style::default().fg(Color::White))
                    .alignment(Alignment::Center),
                area,
            );
        }
        Tree::Button(id, label) => {
            let focused = Some(*id) == focus;
            let style = if focused {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
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
        Tree::Button(id, _) => out.push(*id),
        Tree::Row(children) | Tree::Column(children) => {
            for child in children {
                focusable_ids(child, out);
            }
        }
        Tree::Text(_) => {}
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
        for y in 0..area.height {
            let mut line = String::new();
            for x in 0..area.width {
                line.push_str(
                    buf.cell((x, y))
                        .map(|c| c.symbol())
                        .unwrap_or(" "),
                );
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
    let mut focus_idx = 0usize;
    let mut click_pending = false;
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
        }

        let nodes = app.call_view(&mut store).expect("view() failed");
        let tree = build_tree(&nodes);
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
            }
        }

        let focus = focus_ids.get(focus_idx).copied();
        let _ = terminal
            .draw(|f| draw(f, &tree, focus))
            .expect("failed to draw");
    }

    let _ = terminal.show_cursor().expect("failed to show cursor");
}
