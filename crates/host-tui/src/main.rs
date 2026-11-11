//! A TUI host for waui components: real terminal layout (ratatui) — rows
//! and columns, framed buttons — plus a host-local focus ring. The
//! component has no focus concept; which button is highlighted is a host
//! concern.
//!
//! Controls: up/down (or j/k) move focus, enter/space clicks, printable
//! keys type into the focused text input, q quits.
//! `--dump` renders one frame with ANSI color and prints it, for headless
//! verification of the tree-to-UI mapping; `--click <id>` injects clicks
//! before the interactive loop; `--now <base> <step>` fakes the host clock
//! (each call advances `step` ms), so a `--dump` frame that reads the clock
//! is deterministic.
//!
//! The loop wakes the component — `on-tick` + `changes` — at the period
//! the component requests with `schedule` (0 = never; the 50ms event poll
//! sets the granularity) and rebuilds the retained tree only when the diff
//! is non-empty; otherwise it redraws.

use std::io::{self, stdout};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wasmtime::component::*;
use wasmtime::{Engine, Store};

// The world's bindings and the node-list→tree rebuild live in the shared
// host core; this crate is only its renderer.
use host_shared::{build_tree, waui, App, AppImports, Retained, Tree};

// The contract's style tokens, aliased to avoid clashing with ratatui's
// `Color` and `Style`.
use host_shared::waui::ui::widget::{
    Align as WAlign, Color as WColor, Size as WSize, Style as WStyle,
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{CrosstermBackend, TestBackend},
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};

/// The component's refresh request, shared between the world-import impl
/// (the writer) and the loop (the reader): the period in ms, 0 = no
/// periodic wake, and the last wake.
#[derive(Default)]
struct Wake {
    rate_ms: u64,
    last_wake_ms: u64,
}

/// The host's implementation of the world's imports.
#[derive(Clone, Default)]
struct Host {
    wake: Arc<Mutex<Wake>>,
    /// A fake clock (`--now <base> <step>`): the n-th call to `now` returns
    /// `base + n * step`, so a `--dump` frame that reads the clock shows a
    /// fixed elapsed. `None` = wall time.
    fake_now: Option<(u64, u64)>,
    /// The fake clock's call count.
    fake_calls: u64,
}

impl Host {
    /// The host clock: the fake value when set, else wall time.
    fn clock(&mut self) -> u64 {
        match self.fake_now {
            Some((base, step)) => {
                let t = base + self.fake_calls * step;
                self.fake_calls += 1;
                t
            }
            None => now_ms(),
        }
    }
}

impl AppImports for Host {
    fn now(&mut self) -> u64 {
        self.clock()
    }
    fn schedule(&mut self, period: u64) {
        *self.wake.lock().unwrap() = Wake {
            rate_ms: period,
            last_wake_ms: self.clock(),
        };
    }
}

// The host clock in ms (wall time, as the world's `now` import promises).
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
        // Framed like a button: the content plus its two border columns.
        Tree::Input(_, content, _) => content.chars().count() as u16 + 2,
        Tree::Row(_, children) => children.iter().map(width_of).sum(),
        Tree::Column(_, children) => children.iter().map(width_of).max().unwrap_or(0),
    }
}

fn height_of(node: &Tree) -> u16 {
    match node {
        Tree::Text(..) => 1,
        Tree::Button(..) => 3, // border top, label, border bottom
        Tree::Input(..) => 3,  // border top, content, border bottom
        Tree::Row(_, children) => children.iter().map(height_of).max().unwrap_or(0),
        Tree::Column(_, children) => children.iter().map(height_of).sum(),
    }
}

// --------------------------------------------------------------------------
// Styling: the component's semantic tokens, mapped onto terminal primitives.

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

/// A child's cross-axis offset within the container's cross extent: the
/// token's start/center/end. The extent is the container's content size
/// along that axis (the widest/tallest child), so `center` reproduces the
/// old full-width-rect behavior and `start`/`end` shift the child.
fn cross_offset(align: WAlign, extent: u16, child: u16) -> u16 {
    match align {
        WAlign::Start => 0,
        WAlign::Center => extent.saturating_sub(child) / 2,
        WAlign::End => extent.saturating_sub(child),
    }
}

/// Each child's rect: the main axis stacks children in order (flush from the
/// container's origin); the cross axis places each child per `align`.
fn child_rects(area: Rect, align: WAlign, row: bool, children: &[Tree]) -> Vec<Rect> {
    let mut rects = Vec::with_capacity(children.len());
    let mut main = 0u16;
    for child in children {
        let (w, h) = (width_of(child), height_of(child));
        let (x, y) = if row {
            (area.x + main, area.y + cross_offset(align, area.height, h))
        } else {
            (area.x + cross_offset(align, area.width, w), area.y + main)
        };
        main += if row { w } else { h };
        rects.push(Rect::new(x, y, w, h));
    }
    rects
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
            let block = Block::default().borders(Borders::ALL).border_style(style);
            frame.render_widget(
                Paragraph::new(label.clone())
                    .style(style)
                    .alignment(Alignment::Center)
                    .block(block),
                area,
            );
        }
        Tree::Input(id, content, style) => {
            // A display-only field (no id) can never be focused. The
            // content is the component's copy of the buffer, so it is
            // left-aligned, like a text field.
            let focused = *id == focus;
            let style = if focused {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                node_style(style)
            };
            let block = Block::default().borders(Borders::ALL).border_style(style);
            frame.render_widget(
                Paragraph::new(content.clone()).style(style).block(block),
                area,
            );
        }
        Tree::Row(align, children) => {
            let areas = child_rects(area, *align, true, children);
            for (child, child_area) in children.iter().zip(areas) {
                render(frame, child, child_area, focus);
            }
        }
        Tree::Column(align, children) => {
            let areas = child_rects(area, *align, false, children);
            for (child, child_area) in children.iter().zip(areas) {
                render(frame, child, child_area, focus);
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

/// The hint bar's text. Its length sets the `--dump` buffer's minimum
/// width, so the verification frame shows the bar whole.
const HINT: &str =
    " waui (tui)   up/down or j/k: focus   enter/space: click   type: input   q: quit ";

/// Draw one full frame: a hint bar, then the tree centered below it.
fn draw(frame: &mut Frame, tree: &Tree, focus: Option<u32>) {
    let area = frame.area();
    let bars: [Rect; 2] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    frame.render_widget(
        Paragraph::new(HINT)
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
// Focus: a host-local ring over the tree's buttons and inputs (in pre-order).

/// One widget the focus ring can move to: its id, and whether it is a text
/// input (printable keys type into it; enter/space does not activate it).
#[derive(Clone, Copy)]
struct FocusTarget {
    id: u32,
    is_input: bool,
}

fn focusables(node: &Tree, out: &mut Vec<FocusTarget>) {
    match node {
        // A button the component gave no id cannot be clicked, so it is
        // not focusable.
        Tree::Button(id, ..) => {
            if let Some(i) = id {
                out.push(FocusTarget {
                    id: *i,
                    is_input: false,
                });
            }
        }
        // An id-less input is unaddressable (the WIT identity note), so it
        // is not focusable either.
        Tree::Input(id, ..) => {
            if let Some(i) = id {
                out.push(FocusTarget {
                    id: *i,
                    is_input: true,
                });
            }
        }
        Tree::Row(_, children) | Tree::Column(_, children) => {
            for child in children {
                focusables(child, out);
            }
        }
        Tree::Text(..) => {}
    }
}

/// The content of the input with `id`, wherever it sits in the tree.
fn input_content(node: &Tree, id: u32) -> Option<&str> {
    match node {
        Tree::Input(iid, content, _) if *iid == Some(id) => Some(content),
        Tree::Row(_, children) | Tree::Column(_, children) => {
            children.iter().find_map(|c| input_content(c, id))
        }
        _ => None,
    }
}

/// Report the focused input's whole buffer to the component (`on-text`)
/// and mark the diff pending: the component re-derives the field's content
/// from what it stored, and the next `changes` carries it back.
fn report_input(app: &App, store: &mut Store<Host>, id: u32, buf: &str, pending: &mut bool) {
    app.call_on_text(store, id, buf).expect("on-text() failed");
    *pending = true;
}

// --------------------------------------------------------------------------

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
    let mut clicks: Vec<u32> = Vec::new();
    let mut fake_now: Option<(u64, u64)> = None;
    let mut path: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dump" => {}
            "--click" => {
                if let Some(v) = args.get(i + 1) {
                    clicks.push(v.parse().expect("--click takes a numeric widget id"));
                    i += 1;
                } else {
                    eprintln!("--click takes a numeric widget id");
                    std::process::exit(2);
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
                "usage: host-tui [--dump] [--click <id>] [--now <base> <step>] <component.wasm>"
            );
            std::process::exit(2);
        }
    };

    let engine = Engine::default();
    let component = Component::from_file(&engine, &path).expect("failed to load component");

    let mut linker = Linker::new(&engine);
    App::add_to_linker::<_, HasSelf<_>>(&mut linker, |host| host)
        .expect("failed to link world imports");

    let mut store = Store::new(&engine, Host::default());
    store.data_mut().fake_now = fake_now;
    let app =
        App::instantiate(&mut store, &component, &linker).expect("failed to instantiate component");
    // The loop's handle on the component's refresh request.
    let wake = store.data().wake.clone();

    for id in &clicks {
        app.call_on_click(&mut store, *id)
            .expect("on-click() failed");
    }

    if dump {
        // Headless: render one frame into an in-memory buffer and print it.
        // The buffer fits the tree and the hint bar (a narrow tree would
        // otherwise clip the bar, which spans the full buffer width).
        let nodes = app.call_view(&mut store).expect("view() failed");
        let tree = build_tree(&nodes);
        let w = (width_of(&tree).max(10) + 2).max((HINT.chars().count() + 2) as u16);
        let h = height_of(&tree) + 3;
        let mut terminal =
            Terminal::new(TestBackend::new(w, h)).expect("failed to create terminal");
        let mut targets = Vec::new();
        focusables(&tree, &mut targets);
        let focus = targets.first().map(|t| t.id);
        let completed = terminal
            .draw(|f| draw(f, &tree, focus))
            .expect("draw failed");
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
                    line.push_str(if modifier.contains(Modifier::BOLD) {
                        "\x1b[1m"
                    } else {
                        "\x1b[2m"
                    });
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
    let mut terminal =
        Terminal::new(CrosstermBackend::new(stdout)).expect("failed to create terminal");

    let poll = Duration::from_millis(50);
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
    // The ring's targets, in pre-order: the key handler reads the previous
    // iteration's list, and the loop recomputes it after every rebuild.
    let mut targets: Vec<FocusTarget> = Vec::new();
    // The host's copy of the focused input's buffer (the component owns the
    // field, per the WIT): re-synced from the tree each iteration; only the
    // keystrokes between syncs are local.
    let mut input_buf = String::new();

    while !quit {
        // Read every pending event before acting on them.
        if event::poll(poll).expect("failed to poll events") {
            while event::poll(Duration::ZERO).expect("failed to poll events") {
                match event::read().expect("failed to read event") {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        // The ring's current target, from the last rebuild:
                        // it decides what printable keys do (a 'q' typed
                        // into the input types; only outside it quits).
                        let focused_input = targets.get(focus_idx).filter(|t| t.is_input);
                        match key.code {
                            KeyCode::Esc => {
                                quit = true;
                            }
                            // Move the focus ring to the previous/next
                            // focusable widget (wrapping). The exact count
                            // is applied after the tree rebuild below.
                            KeyCode::Up | KeyCode::Char('k') => {
                                focus_idx = focus_idx.wrapping_sub(1);
                            }
                            KeyCode::Down | KeyCode::Left | KeyCode::Right | KeyCode::Char('j') => {
                                focus_idx = focus_idx.wrapping_add(1);
                            }
                            // Enter/space activates a button; an input has
                            // no activation.
                            KeyCode::Enter | KeyCode::Char(' ') => {
                                if focused_input.is_none() {
                                    click_pending = true;
                                }
                            }
                            // The focused input's buffer: edited here,
                            // reported whole through `on-text`, and re-synced
                            // from the tree after the component answers.
                            KeyCode::Backspace => {
                                if let Some(t) = focused_input {
                                    input_buf.pop();
                                    report_input(&app, &mut store, t.id, &input_buf, &mut pending);
                                }
                            }
                            KeyCode::Char(c) => {
                                if let Some(t) = focused_input {
                                    input_buf.push(c);
                                    report_input(&app, &mut store, t.id, &input_buf, &mut pending);
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

        // The component's refresh request (0 = never): wake when its
        // period has elapsed; each `schedule` call restarts the period.
        let now = now_ms();
        {
            let mut w = wake.lock().unwrap();
            if w.rate_ms > 0 && now >= w.last_wake_ms + w.rate_ms {
                w.last_wake_ms = now;
                app.call_on_tick(&mut store).expect("on-tick() failed");
                pending = true;
            }
        }

        // Apply the diff to the retained tree; rebuild only when it
        // reported something (an empty diff skips the rebuild).
        if pending {
            pending = false;
            let mutations = app.call_changes(&mut store).expect("changes() failed");
            if retained.apply(&mutations) {
                tree = build_tree(retained.nodes());
            }
        }

        // Recomputed from the tree so a rebuilt tree (a moved or removed
        // widget) re-seats the ring.
        targets.clear();
        focusables(&tree, &mut targets);
        if targets.is_empty() {
            focus_idx = 0;
            click_pending = false;
        } else {
            // `focus_idx` may be one past the end (from wrapping) or
            // stale after a tree change; normalize into the target list.
            let n = targets.len();
            focus_idx = ((focus_idx % n) + n) % n;
            let target = &targets[focus_idx];
            // Re-sync the host's copy from the tree (the component's answer).
            if target.is_input {
                if let Some(content) = input_content(&tree, target.id) {
                    input_buf = content.to_string();
                }
            }
            if click_pending {
                click_pending = false;
                // An input has no activation; only a button is clicked.
                if !target.is_input {
                    app.call_on_click(&mut store, target.id)
                        .expect("on-click() failed");
                    pending = true;
                }
            }
        }

        let focus = targets.get(focus_idx).map(|t| t.id);
        let _ = terminal
            .draw(|f| draw(f, &tree, focus))
            .expect("failed to draw");
    }

    let _ = terminal.show_cursor().expect("failed to show cursor");
}
