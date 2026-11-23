//! The browser TUI host: the same renderer and focus ring as the native
//! binary (the crate's library), drawn to an xterm.js terminal through a
//! hand-rolled SGR backend (no crossterm). The component is precompiled
//! natively for pulley32 (the `host-precompile` tool) and deserialized here
//! to run on the Pulley interpreter — Cranelift JIT is unavailable in wasm
//! (no executable memory), so the browser never compiles: it only
//! interprets.
//!
//! The page owns the xterm.js terminal (`window.term`); `run` reads it,
//! deserializes the component, and wires the keyboard. `key` and `tick`
//! drive it; `dump` is the headless byte-parity surface (it renders a frame
//! to text, independent of the live terminal, so the Node selftest can
//! compare it against the native `host-tui --dump`).

use std::cell::{Cell, RefCell};
use std::io;
use std::sync::{Arc, Mutex};

use js_sys::{Array, Function, Object, Reflect};
use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::{Cell as RatatuiCell, CellWidth};
use ratatui::layout::{Position, Size};
use ratatui::style::{Color, Modifier, Style};
use ratatui::Terminal;
use wasm_bindgen::prelude::*;

use host_shared::{build_tree, App, Retained, Tree};
use wasmtime::component::*;
use wasmtime::{Engine, Store};

use crate::{
    active_app, active_surface, active_target, active_tree, derive_active, draw, dump_frame,
    focus_newest_window, input_content, now_ms, report_input, reseat, reseat_active, Host, Surface,
    Wake,
};

// --------------------------------------------------------------------------
// The xterm.js backend
// --------------------------------------------------------------------------

/// An xterm.js `Terminal`, reached through `window.term`.
///
/// The backend is a dumb printer: ratatui's `Terminal` owns the double
/// buffer and hands over only the changed cells, in row-major order. The
/// backend moves the cursor (CUP) when a cell is not adjacent to the last
/// one written, resets and re-sets SGR when the style changes, and prints
/// the symbol. Everything is written in one `term.write` per pass.
#[derive(Debug)]
struct XtermBackend {
    term: Object,
    /// Last size read from xterm. `size()` refreshes it on every call
    /// (`Terminal::autoresize` calls it on each draw).
    size: Cell<Size>,
    /// The terminal cursor position as tracked by the backend.
    cursor: Position,
    /// Where the next character will land. `None` forces a CUP before the
    /// next write.
    pen: Option<Position>,
    /// The SGR style in effect on the terminal (what the last reset+set
    /// sequence produced).
    active_style: Style,
}

impl XtermBackend {
    fn new(term: Object, cols: u16, rows: u16) -> Self {
        XtermBackend {
            term,
            size: Cell::new(Size {
                width: cols,
                height: rows,
            }),
            cursor: Position { x: 0, y: 0 },
            pen: Some(Position { x: 0, y: 0 }),
            active_style: Style::default(),
        }
    }

    fn write(&self, text: &str) {
        let f = match Reflect::get(&self.term, &JsValue::from_str("write")) {
            Ok(f) => f,
            Err(_) => return,
        };
        let f: Function = f.unchecked_into();
        let args = Array::from_iter([JsValue::from_str(text)]);
        let _ = Reflect::apply(&f, &self.term, &args);
    }

    /// Read the live size from xterm and cache it.
    fn read_size(&self) -> Size {
        let cols = Reflect::get(&self.term, &JsValue::from_str("cols"))
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(self.size.get().width as f64) as u16;
        let rows = Reflect::get(&self.term, &JsValue::from_str("rows"))
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(self.size.get().height as f64) as u16;
        let size = Size {
            width: cols,
            height: rows,
        };
        self.size.set(size);
        size
    }
}

/// A cell's fields as an absolute `Style` (a cell stores `Color` with
/// `Reset` for "unset", while `Style` uses `Option<Color>`).
fn style_from_cell(cell: &RatatuiCell) -> Style {
    fn opt(c: Color) -> Option<Color> {
        (c != Color::Reset).then_some(c)
    }
    Style {
        fg: opt(cell.fg),
        bg: opt(cell.bg),
        underline_color: opt(cell.underline_color),
        add_modifier: cell.modifier,
        sub_modifier: Modifier::empty(),
    }
}

/// SGR for a color in the given class: 38 (fg), 48 (bg), 58 (underline).
fn sgr_color(c: Color, class: u16) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("{class};2;{r};{g};{b}"),
        Color::Indexed(n) => format!("{class};5;{n}"),
        Color::Black => format!("{}", class - 8),
        Color::Red => format!("{}", class - 7),
        Color::Green => format!("{}", class - 6),
        Color::Yellow => format!("{}", class - 5),
        Color::Blue => format!("{}", class - 4),
        Color::Magenta => format!("{}", class - 3),
        Color::Cyan => format!("{}", class - 2),
        Color::Gray => format!("{}", class - 1),
        Color::DarkGray => format!("{class};0"),
        Color::LightRed => format!("{class};1"),
        Color::LightGreen => format!("{class};2"),
        Color::LightYellow => format!("{class};3"),
        Color::LightBlue => format!("{class};4"),
        Color::LightMagenta => format!("{class};5"),
        Color::LightCyan => format!("{class};6"),
        Color::White => format!("{class};7"),
        Color::Reset => "0".to_string(),
    }
}

/// Emit a full style: reset, then color and modifiers.
fn emit_style(out: &mut String, style: &Style) {
    out.push_str("\x1b[0m");
    if let Some(fg) = style.fg {
        out.push_str(&format!("\x1b[{}m", sgr_color(fg, 38)));
    }
    if let Some(bg) = style.bg {
        out.push_str(&format!("\x1b[{}m", sgr_color(bg, 48)));
    }
    if style.add_modifier.contains(Modifier::UNDERLINED) {
        // 16-color underline has no SGR code; only 256/truecolor do.
        if let Some(uc) = style.underline_color {
            if !matches!(uc, Color::Reset) {
                out.push_str(&format!("\x1b[{}m", sgr_color(uc, 58)));
            }
        }
        out.push_str("\x1b[4m");
    }
    for (bit, code) in [
        (Modifier::BOLD, "1"),
        (Modifier::DIM, "2"),
        (Modifier::ITALIC, "3"),
        (Modifier::SLOW_BLINK, "5"),
        (Modifier::RAPID_BLINK, "6"),
        (Modifier::REVERSED, "7"),
        (Modifier::HIDDEN, "8"),
        (Modifier::CROSSED_OUT, "9"),
    ] {
        if style.add_modifier.contains(bit) {
            out.push_str(&format!("\x1b[{code}m"));
        }
    }
}

impl Backend for XtermBackend {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a RatatuiCell)>,
    {
        let mut out = String::new();
        for (x, y, cell) in content {
            let symbol = cell.symbol();
            let width = cell.cell_width().max(1);
            let style = style_from_cell(cell);

            if self.pen != Some(Position { x, y }) {
                out.push_str(&format!("\x1b[{};{}H", y + 1, x + 1));
            }
            if style != self.active_style {
                emit_style(&mut out, &style);
                self.active_style = style;
            }
            out.push_str(symbol);

            // Writing into the last column wraps the pen to the next row.
            let pen = if x.saturating_add(width) >= self.size.get().width {
                Position {
                    x: 0,
                    y: y.saturating_add(1),
                }
            } else {
                Position { x: x + width, y }
            };
            self.pen = Some(pen);
            self.cursor = pen;
        }
        if !out.is_empty() {
            self.write(&out);
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.write("\x1b[?25l");
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.write("\x1b[?25h");
        Ok(())
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(self.cursor)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        let p = position.into();
        self.write(&format!("\x1b[{};{}H", p.y + 1, p.x + 1));
        self.pen = Some(p);
        self.cursor = p;
        Ok(())
    }

    fn clear(&mut self) -> io::Result<()> {
        self.write("\x1b[2J\x1b[H");
        self.pen = Some(Position { x: 0, y: 0 });
        self.cursor = Position { x: 0, y: 0 };
        Ok(())
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        let seq = match clear_type {
            ClearType::All => "\x1b[2J\x1b[H",
            ClearType::AfterCursor => "\x1b[J",
            ClearType::BeforeCursor => "\x1b[1J",
            ClearType::CurrentLine => "\x1b[2K",
            ClearType::UntilNewLine => "\x1b[K",
        };
        self.write(seq);
        if matches!(clear_type, ClearType::All) {
            self.pen = Some(Position { x: 0, y: 0 });
            self.cursor = Position { x: 0, y: 0 };
        }
        Ok(())
    }

    fn size(&self) -> io::Result<Size> {
        Ok(self.read_size())
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        let size = self.read_size();
        // xterm.js does not expose its pixel size through a stable API.
        Ok(WindowSize {
            columns_rows: size,
            pixels: Size {
                width: 0,
                height: 0,
            },
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        // Writes go straight to xterm; nothing is buffered here.
        Ok(())
    }
}

// --------------------------------------------------------------------------
// The host
// --------------------------------------------------------------------------

/// The browser host's state: the component (its store, app, retained tree),
/// the live xterm terminal, and the interactive loop's local state.
struct WebHost {
    store: Store<Host>,
    app: App,
    retained: Retained,
    tree: Tree,
    wake: Arc<Mutex<Wake>>,
    has_registry: bool,
    terminal: Option<Terminal<XtermBackend>>,
    /// The active surface: 0 = the app, 1.. = a window's z-order index,
    /// derived from the focused target's owner (the ring is unified).
    active: usize,
    app_surf: Surface,
    /// A tile just opened a window: focus its first focusable (so it is the
    /// active surface) on the next re-seat, once its content is in the ring.
    focus_newest_pending: bool,
    /// Whether the app's component was driven since the last `changes`.
    pending: bool,
    click_pending: bool,
    double_click_pending: bool,
    /// The focused input's buffer (the component owns the field, per the
    /// WIT): re-synced from the active surface's tree each pass.
    input_buf: String,
    /// Whether `q` was pressed: the terminal is cleared, and the host stops
    /// redrawing (a page cannot exit, so this is the web's quit).
    quit: bool,
}

// A web page is single-threaded, so a thread-local is a plain global. It
// sidesteps the `Send`/`Sync` bounds a `static OnceLock<Mutex<_>>` would
// require of the wasmtime `Store`/`App`.
thread_local! {
    static HOST: RefCell<Option<WebHost>> = const { RefCell::new(None) };
    /// The live host's component map, as a handle the page's
    /// `components-add` export writes through (the map itself lives in the
    /// host's store data; this is a clone of its arc).
    static COMPONENTS: RefCell<Option<Arc<Mutex<std::collections::BTreeMap<String, Vec<u8>>>>>> =
        const { RefCell::new(None) };
}

/// Run `f` with the live host, if `run` has been called.
fn with_host<R>(f: impl FnOnce(&mut WebHost) -> R) -> Option<R> {
    HOST.with(|h| {
        let mut slot = h.borrow_mut();
        let host = slot.as_mut()?;
        Some(f(host))
    })
}

/// The live xterm terminal, read from `window.term`.
fn open_terminal() -> Result<Terminal<XtermBackend>, String> {
    let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let term = Reflect::get(&window.into(), &JsValue::from_str("term"))
        .map_err(|e| format!("no window.term: {e:?}"))?;
    let term: Object = term.unchecked_into();
    let cols = Reflect::get(&term, &JsValue::from_str("cols"))
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(80.0) as u16;
    let rows = Reflect::get(&term, &JsValue::from_str("rows"))
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(24.0) as u16;
    let backend = XtermBackend::new(term, cols, rows);
    Terminal::new(backend).map_err(|e| format!("terminal: {e}"))
}

/// The key names `handle_key` matches, from xterm's `onKey` write-string.
/// xterm reports the bytes the key writes to a terminal (`"\r"` for
/// enter, `"\u001b[B"` for down-arrow, ...); the native host gets the
/// same bytes parsed into key names by crossterm, so the web host parses
/// them the same way here.
fn key_name(key: &str) -> &str {
    match key {
        "\r" => "Enter",
        "\u{1b}" => "Escape",
        "\u{1b}[A" => "ArrowUp",
        "\u{1b}[B" => "ArrowDown",
        "\u{1b}[C" => "ArrowRight",
        "\u{1b}[D" => "ArrowLeft",
        "\t" => "Tab",
        "\u{7f}" => "Backspace",
        other => other,
    }
}

impl WebHost {
    /// The page's 50ms tick: wake the components whose refresh period has
    /// elapsed, apply the diffs, re-seat, fire activations, and redraw.
    /// After a quit, the host is idle.
    fn tick(&mut self) {
        if self.quit {
            return;
        }
        let now = now_ms();
        {
            let mut w = self.wake.lock().unwrap();
            if w.rate_ms > 0 && now >= w.last_wake_ms + w.rate_ms {
                w.last_wake_ms = now;
                self.app
                    .call_on_tick(&mut self.store)
                    .expect("on-tick() failed");
                self.pending = true;
            }
        }
        for win in self.store.data_mut().windows.iter_mut() {
            let mut w = win.wake.lock().unwrap();
            if w.rate_ms > 0 && now >= w.last_wake_ms + w.rate_ms {
                w.last_wake_ms = now;
                win.app
                    .call_on_tick(&mut win.store)
                    .expect("on-tick() failed");
                win.pending = true;
            }
        }
        self.run_pipeline();
    }

    /// One key (a key name, `shift` the shift state): mutate the focus
    /// ring, the input buffer, and the windows, then run the pipeline and
    /// redraw. After a quit, keys do nothing.
    fn handle_key(&mut self, key: &str, shift: bool) {
        if self.quit {
            return;
        }
        // The app's ring target (the ring is unified), from the last
        // re-seat: it decides what the key does (a 'q' typed into the input
        // types; only outside it quits).
        let (t_id, t_is_input, t_is_tile, _t_owner) =
            active_target(&mut self.store, &mut self.app_surf, self.active);
        match key {
            // Escape: the host quits only when there are no windows (a page
            // cannot exit, so this clears the terminal and stops redrawing);
            // otherwise it is forwarded to the app below (the window
            // manager closes the frontmost). There is no host-level window
            // close.
            "Escape" if self.store.data().windows.is_empty() => {
                self.quit = true;
                if let Some(term) = &mut self.terminal {
                    let _ = term.clear();
                }
            }
            // Move the focus ring to the previous/next focusable widget
            // (wrapping). The ring is unified (the app's), so it spans the
            // app and every open window; `active` follows the focused
            // target's owner.
            "ArrowUp" | "k" => {
                self.app_surf.focus_idx = self.app_surf.focus_idx.wrapping_sub(1);
                derive_active(&self.store, &self.app_surf, &mut self.active);
            }
            "ArrowDown" | "ArrowLeft" | "ArrowRight" | "j" => {
                self.app_surf.focus_idx = self.app_surf.focus_idx.wrapping_add(1);
                derive_active(&self.store, &self.app_surf, &mut self.active);
            }
            // Enter/space activates the focused widget: a tile opens its
            // entry (a synthesized double-activation, the host's input model
            // has no double-click), a button is clicked; an input has no
            // activation.
            "Enter" | " " => {
                if t_is_tile {
                    self.double_click_pending = true;
                } else if !t_is_input {
                    self.click_pending = true;
                }
            }
            // The focused input's buffer: edited here, reported whole through
            // `on-text`, and re-synced from the tree after the component
            // answers.
            "Backspace" => {
                if t_is_input {
                    self.input_buf.pop();
                    let (a, s) = active_app(&mut self.store, &self.app, self.active);
                    report_input(a, s, t_id, &self.input_buf, &mut self.pending);
                }
            }
            // A printable key: typed into the focused input, or 'q' quits.
            _ if key.len() == 1 => {
                if t_is_input {
                    if let Some(c) = key.chars().next() {
                        self.input_buf.push(c);
                    }
                    let (a, s) = active_app(&mut self.store, &self.app, self.active);
                    report_input(a, s, t_id, &self.input_buf, &mut self.pending);
                } else if key == "q" {
                    // Quit: clear the terminal and stop redrawing (a page
                    // cannot exit).
                    self.quit = true;
                    if let Some(term) = &mut self.terminal {
                        let _ = term.clear();
                    }
                }
            }
            // Tab, shift+tab, and the rest: forwarded to the component below
            // (window management is the app's job).
            _ => {}
        }
        // Forward the key to the component (`on-key`), unless a text input is
        // focused (its keys go to `on-text`, per the WIT) or the host is
        // quitting. It always routes to the main component: the window
        // manager on a multi-window host, the demo itself standalone. A
        // component that manages no key-driven UI ignores it.
        if !self.quit && !t_is_input {
            let name = if key == "Tab" && shift {
                "Shift+Tab"
            } else {
                key
            };
            let (a, s) = active_app(&mut self.store, &self.app, 0);
            a.call_on_key(s, name).expect("on-key() failed");
            self.pending = true;
        }
        if !self.quit {
            self.run_pipeline();
        }
    }

    /// Apply the diffs, re-seat the active ring, re-sync the focused input,
    /// fire the pending activation, and redraw.
    fn run_pipeline(&mut self) {
        // Refresh the retained trees when the components report a change
        // (an empty diff skips the rebuild). A content-only diff is applied
        // in place; a structural one re-queries `view` (the diff cannot
        // carry id-less nodes, so the snapshot is the source of truth).
        if self.pending {
            self.pending = false;
            let mutations = self
                .app
                .call_changes(&mut self.store)
                .expect("changes() failed");
            if !mutations.is_empty() {
                if self.retained.is_content_only(&mutations) {
                    self.retained.apply(&mutations);
                } else {
                    let nodes = self.app.call_view(&mut self.store).expect("view() failed");
                    self.retained = Retained::from_view(nodes);
                }
                self.tree = build_tree(self.retained.nodes());
            }
        }
        for win in self.store.data_mut().windows.iter_mut() {
            if win.pending {
                win.pending = false;
                let mutations = win
                    .app
                    .call_changes(&mut win.store)
                    .expect("changes() failed");
                if !mutations.is_empty() {
                    if win.retained.is_content_only(&mutations) {
                        win.retained.apply(&mutations);
                    } else {
                        let nodes = win.app.call_view(&mut win.store).expect("view() failed");
                        win.retained = Retained::from_view(nodes);
                    }
                    win.tree = build_tree(win.retained.nodes());
                }
            }
        }

        // Re-seat the app's ring (a rebuilt tree moves its widgets, and a
        // window that just opened has its content inlined), and re-derive
        // `active` from the focused target's owner.
        reseat_active(
            &mut self.store,
            &self.tree,
            self.has_registry,
            &mut self.active,
            &mut self.app_surf,
        );
        // A tile opened a window this pass: now that its content is in the
        // ring, focus its first focusable (so it is the active surface).
        if self.focus_newest_pending {
            self.focus_newest_pending = false;
            let focused = {
                let wins = &self.store.data().windows;
                focus_newest_window(&mut self.app_surf, wins)
            };
            if focused {
                derive_active(&self.store, &self.app_surf, &mut self.active);
            }
        }
        {
            let (n, t_id, t_is_input, t_is_tile) = {
                let surf = active_surface(&mut self.store, &mut self.app_surf, self.active);
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
                self.click_pending = false;
                self.double_click_pending = false;
            } else {
                if t_is_input {
                    if let Some(content) =
                        input_content(active_tree(&self.store, &self.tree, self.active), t_id)
                    {
                        self.input_buf = content.to_string();
                    }
                }
                if self.click_pending {
                    self.click_pending = false;
                    // An input has no activation; only a button is clicked.
                    if !t_is_input {
                        let (a, s) = active_app(&mut self.store, &self.app, self.active);
                        a.call_on_click(s, t_id).expect("on-click() failed");
                        self.pending = true;
                    }
                }
                if self.double_click_pending {
                    self.double_click_pending = false;
                    if t_is_tile {
                        let before = self.store.data().windows.len();
                        self.app
                            .call_on_double_click(&mut self.store, t_id)
                            .expect("on-double-click() failed");
                        self.pending = true;
                        // The tile's entry opened a window: focus it (so it
                        // is the active surface) on the next re-seat, once
                        // its content is in the ring.
                        if self.store.data().windows.len() > before {
                            self.focus_newest_pending = true;
                        }
                    }
                }
            }
        }

        self.redraw();
    }

    /// Draw the current frame to the live terminal.
    fn redraw(&mut self) {
        let Some(term) = self.terminal.as_mut() else {
            return;
        };
        let tree = &self.tree;
        let focus = self.app_surf.focus;
        let active = self.active;
        let has_registry = self.has_registry;
        let _ = term.draw(|f| {
            let data = self.store.data_mut();
            draw(
                f,
                tree,
                focus,
                &data.windows,
                active,
                has_registry,
                &mut data.image,
            )
        });
    }
}

// --------------------------------------------------------------------------
// The wasm-bindgen exports
// --------------------------------------------------------------------------

/// Apply the page's registry (the desktop): the manifest's entries, the icon
/// bytes, and the entry components' bytes. All are `None` for the
/// non-desktop demos, which carry no registry, no images, and no entries to
/// open.
fn apply_registry(
    host: &mut Host,
    manifest: Option<&str>,
    assets: Option<&str>,
    components: Option<&str>,
) -> Result<(), JsValue> {
    if let Some(text) = manifest {
        host.entries = host_shared::parse_manifest("the page's manifest", text)
            .map_err(|e| JsValue::from_str(&e))?;
    }
    if let Some(json) = assets {
        host.image.assets = host_shared::decode_assets(json).map_err(|e| JsValue::from_str(&e))?;
    }
    if let Some(json) = components {
        let decoded = host_shared::decode_components(json).map_err(|e| JsValue::from_str(&e))?;
        *host.components.lock().unwrap() = decoded;
    }
    Ok(())
}

/// Start the host: deserialize the precompiled component, instantiate it,
/// open the xterm terminal, wire the keyboard, and draw the first frame.
/// `bytes` is the artifact the `host-precompile` tool wrote. `manifest` (the
/// desktop's registry JSON), `assets` (the icons, base64), and `components`
/// (the entry artifacts, base64, keyed by the manifest's `path`) are `None`
/// for the non-desktop demos; the desktop passes `null` for `components` and
/// adds them with [`components_add`] as its background fetches resolve, so
/// the grid's first frame never waits for them.
#[wasm_bindgen]
pub fn run(
    bytes: &[u8],
    manifest: Option<String>,
    assets: Option<String>,
    components: Option<String>,
) -> Result<(), JsValue> {
    let engine = Engine::new(&host_shared::pulley_config())
        .map_err(|e| JsValue::from_str(&format!("engine: {e}")))?;
    let component = unsafe { Component::deserialize(&engine, bytes) }
        .map_err(|e| JsValue::from_str(&format!("deserialize: {e}")))?;
    let mut linker = Linker::new(&engine);
    if let Err(e) = App::add_to_linker::<_, HasSelf<Host>>(&mut linker, |host| host) {
        return Err(JsValue::from_str(&format!("link: {e}")));
    }
    let mut host = Host::default();
    host.engine = engine.clone();
    host.linker = linker.clone();
    apply_registry(
        &mut host,
        manifest.as_deref(),
        assets.as_deref(),
        components.as_deref(),
    )?;
    // The page's `components-add` handle on the live host's component map.
    COMPONENTS.with(|c| *c.borrow_mut() = Some(host.components.clone()));
    let mut store = Store::new(&engine, host);
    let app = App::instantiate(&mut store, &component, &linker)
        .map_err(|e| JsValue::from_str(&format!("instantiate: {e}")))?;
    let wake = store.data().wake.clone();
    let has_registry = !store.data().entries.is_empty();
    // The startup snapshot (the WIT bootstrap).
    let nodes = app
        .call_view(&mut store)
        .map_err(|e| JsValue::from_str(&format!("view: {e}")))?;
    let retained = Retained::from_view(nodes);
    let tree = build_tree(retained.nodes());
    let terminal = open_terminal().map_err(|e| JsValue::from_str(&e))?;

    let mut web_host = WebHost {
        store,
        app,
        retained,
        tree,
        wake,
        has_registry,
        terminal: Some(terminal),
        active: 0,
        app_surf: Surface::default(),
        focus_newest_pending: false,
        pending: false,
        click_pending: false,
        double_click_pending: false,
        input_buf: String::new(),
        quit: false,
    };
    // The first frame: the app's ring on its first focusable.
    reseat(
        &web_host.tree,
        web_host.has_registry,
        &web_host.store.data().windows,
        &mut web_host.app_surf,
    );
    web_host.redraw();
    HOST.with(|h| *h.borrow_mut() = Some(web_host));

    // Wire the keyboard and resize to xterm. The closures read the host
    // back through the thread-local; xterm owns the JS functions, so the
    // Rust `Closure`s are handed over, not kept.
    let on_key = Closure::wrap(Box::new(|ev: JsValue| {
        with_host(|host| {
            let obj = ev.unchecked_ref::<Object>();
            // xterm's `onKey` passes the bytes the key writes, not a key
            // name; parse them into the names the host matches.
            let key = Reflect::get(obj, &JsValue::from_str("key"))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default();
            let key = key_name(&key);
            let shift = Reflect::get(obj, &JsValue::from_str("domEvent"))
                .ok()
                .map(|d| {
                    d.dyn_ref::<web_sys::KeyboardEvent>()
                        .is_some_and(|e| e.shift_key())
                })
                .unwrap_or(false);
            host.handle_key(key, shift);
        });
    }) as Box<dyn FnMut(JsValue)>);
    let on_resize = Closure::wrap(Box::new(|_ev: JsValue| {
        // The backend reads the live size on each draw; a redraw applies it.
        with_host(|host| host.redraw());
    }) as Box<dyn FnMut(JsValue)>);
    let term = HOST.with(|h| {
        h.borrow()
            .as_ref()
            .and_then(|host| host.terminal.as_ref())
            .map(|t| t.backend().term.clone())
    });
    if let Some(term) = term {
        if let Ok(f) = Reflect::get(&term, &JsValue::from_str("onKey")) {
            let f: Function = f.unchecked_into();
            let _ = Reflect::apply(&f, &term, &Array::from_iter([on_key.into_js_value()]));
        }
        if let Ok(f) = Reflect::get(&term, &JsValue::from_str("onResize")) {
            let f: Function = f.unchecked_into();
            let _ = Reflect::apply(&f, &term, &Array::from_iter([on_resize.into_js_value()]));
        }
    }
    Ok(())
}

/// One key, from the page: `key` is a key name (`"Enter"`, `"ArrowUp"`,
/// `"a"`; xterm's write-strings are parsed into names by the `onKey`
/// wiring, not by this export), `shift` the shift state.
#[wasm_bindgen]
pub fn key(key: &str, shift: bool) {
    with_host(|host| host.handle_key(key, shift));
}

/// The page's 50ms tick: wake the components whose refresh period has
/// elapsed, apply the diffs, and redraw.
#[wasm_bindgen]
pub fn tick() {
    with_host(|host| host.tick());
}

/// One entry component's bytes, from the page's background fetch: `path` is
/// the manifest's `path` (what `open-entry` looks up), `b64` the
/// base64-encoded precompiled artifact. The live host's component map grows
/// as the fetches resolve; a double-click that lands before its fetch does
/// fails as a missing file does natively.
#[wasm_bindgen]
pub fn components_add(path: &str, b64: &str) -> Result<(), JsValue> {
    let bytes = host_shared::decode_b64("components", path, b64)?;
    let map = COMPONENTS
        .with(|c| c.borrow().clone())
        .ok_or_else(|| JsValue::from_str("components-add: run first"))?;
    map.lock().unwrap().insert(path.to_string(), bytes);
    Ok(())
}

/// The headless byte-parity surface: a fresh host (so the fake-clock call
/// sequence matches the native `--dump`), the component deserialized from
/// the precompiled artifact, the clicks and double-clicks applied, and one
/// frame rendered to text. `width` sizes the buffer like a terminal of that
/// width; `now_base` and `now_step` fake the host clock (the n-th `now`
/// call returns `now_base + n * now_step`); `clicks` and `double_clicks`
/// are widget ids, in order (a double-clicked tile opens its entry);
/// `components` is the entry artifacts' base64 map, for a desktop that
/// opens one.
#[wasm_bindgen]
pub fn dump(
    bytes: &[u8],
    width: Option<u16>,
    now_base: Option<u64>,
    now_step: Option<u64>,
    clicks: &[u32],
    double_clicks: &[u32],
    manifest: Option<String>,
    assets: Option<String>,
    components: Option<String>,
) -> Result<String, JsValue> {
    let engine = Engine::new(&host_shared::pulley_config())
        .map_err(|e| JsValue::from_str(&format!("engine: {e}")))?;
    let component = unsafe { Component::deserialize(&engine, bytes) }
        .map_err(|e| JsValue::from_str(&format!("deserialize: {e}")))?;
    let mut linker = Linker::new(&engine);
    if let Err(e) = App::add_to_linker::<_, HasSelf<Host>>(&mut linker, |host| host) {
        return Err(JsValue::from_str(&format!("link: {e}")));
    }
    let mut host = Host::default();
    host.engine = engine.clone();
    host.linker = linker.clone();
    host.fake_now = match (now_base, now_step) {
        (Some(base), Some(step)) => Some((base, step)),
        _ => None,
    };
    apply_registry(
        &mut host,
        manifest.as_deref(),
        assets.as_deref(),
        components.as_deref(),
    )?;
    let mut store = Store::new(&engine, host);
    let app = App::instantiate(&mut store, &component, &linker)
        .map_err(|e| JsValue::from_str(&format!("instantiate: {e}")))?;
    let has_registry = !store.data().entries.is_empty();
    // The startup snapshot (the WIT bootstrap).
    let nodes = app
        .call_view(&mut store)
        .map_err(|e| JsValue::from_str(&format!("view: {e}")))?;
    let mut retained = Retained::from_view(nodes);
    let mut tree = build_tree(retained.nodes());
    // The clicks and double-clicks, in order, like a pointer would drive
    // them (a double-clicked tile opens its entry as a window).
    for id in clicks {
        app.call_on_click(&mut store, *id)
            .map_err(|e| JsValue::from_str(&format!("on-click: {e}")))?;
    }
    for id in double_clicks {
        app.call_on_double_click(&mut store, *id)
            .map_err(|e| JsValue::from_str(&format!("on-double-click: {e}")))?;
    }
    // The events may have changed the component's state: re-query the full
    // snapshot, as the native `--dump` does after its injected events.
    if !clicks.is_empty() || !double_clicks.is_empty() {
        let nodes = app
            .call_view(&mut store)
            .map_err(|e| JsValue::from_str(&format!("view: {e}")))?;
        retained = Retained::from_view(nodes);
        tree = build_tree(retained.nodes());
    }
    let data = store.data_mut();
    Ok(dump_frame(
        &tree,
        &data.windows,
        width,
        has_registry,
        &mut data.image,
    ))
}
