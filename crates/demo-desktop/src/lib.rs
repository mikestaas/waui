//! The desktop: the icon grid, the menu bar, and the set of open windows.
//!
//! The multi-component host's component, on the `app` world. It owns the
//! *logical* state — the grid (built from the host's `list-demos` registry),
//! the menu bar's window count, and which windows are open — and exports it
//! via `view`/`changes`, as any component does. It *requests* a window
//! through the `open-window` import when a tile is double-clicked, and drops
//! a window from its open set when the host reports a close. The windows
//! themselves (their chrome, geometry, and the embedded demo running in each)
//! are host-drawn, not part of this tree.
//!
//! The tile is a container (an icon, a name, and a language) with an id: the
//! host makes id'd nodes double-clickable on the desktop, and the component
//! opens the demo a tile represents in response.

wit_bindgen::generate!({
    world: "app",
    path: "../../wit",
});

use component_ui::{Color, Size, Style, Tree};
use std::sync::Mutex;
use waui::ui::widget as wit;

// Project the ergonomic tree onto this crate's generated types.
component_ui::to_node!(wit);

/// The first tile's id; tile `i` is `TILE_BASE + i`.
const TILE_BASE: u32 = 100;
/// Tiles per row of the grid.
const COLS: usize = 3;

/// The demos the host can run, fetched once from the `list-demos` registry.
static DEMOS: Mutex<Option<Vec<Demo>>> = Mutex::new(None);
/// The open windows' handles, as the host assigns them.
static OPEN: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// The current tree: the menu bar (with the window count) and the grid.
fn current_tree() -> Tree {
    {
        let mut demos = DEMOS.lock().unwrap();
        if demos.is_none() {
            *demos = Some(list_demos());
        }
    }
    let demos = DEMOS.lock().unwrap().clone().unwrap_or_default();
    let n_open = OPEN.lock().unwrap().len();
    tree(&demos, n_open)
}

/// The desktop's whole UI, nested.
fn tree(demos: &[Demo], n_open: usize) -> Tree {
    let (count, plural) = (n_open.to_string(), if n_open == 1 { "" } else { "s" });
    let menu = Tree::row(vec![
        Tree::text("waui", Style::new(Color::Accent, Size::Large)),
        Tree::text(
            format!("{count} window{plural}"),
            Style::new(Color::Neutral, Size::Small),
        )
        .id(2),
    ])
    .id(1);

    // The grid: a column of rows of `COLS` tiles each, in `list-demos` order.
    let rows: Vec<Tree> = demos
        .chunks(COLS)
        .enumerate()
        .map(|(r, chunk)| {
            let tiles = chunk
                .iter()
                .enumerate()
                .map(|(c, d)| tile(d, r * COLS + c))
                .collect();
            Tree::row(tiles).id(10 + r as u32)
        })
        .collect();
    let grid = Tree::column(rows).id(3);

    Tree::column(vec![menu, grid])
}

/// One tile: an icon, a name, and a language, in a double-clickable column.
/// The icon is 112px, the web desktop's tile size (see `site/desktop.css`).
fn tile(demo: &Demo, i: usize) -> Tree {
    Tree::column(vec![
        Tree::image(demo.icon.clone()).width(112).height(112),
        Tree::text(demo.name.clone(), Style::new(Color::Neutral, Size::Medium)),
        Tree::text(demo.lang.clone(), Style::new(Color::Neutral, Size::Small)),
    ])
    .id(TILE_BASE + i as u32)
}

struct Desktop;

impl Guest for Desktop {
    fn view() -> Vec<Node> {
        let t = current_tree();
        component_ui::note_view(&t);
        component_ui::lower(&t, to_node)
    }

    fn changes() -> Vec<Mutation> {
        component_ui::diff(&current_tree(), |c| match c {
            component_ui::DiffNode::Changed(node, parent, index) => {
                Mutation::Changed(wit::Changed {
                    node: to_node(node, parent),
                    index,
                })
            }
            component_ui::DiffNode::Removed(id) => Mutation::Removed(id),
        })
    }

    /// A tile was double-clicked: open the demo it represents. The placement
    /// is left to the host (0,0 = its default cascade); the size is the
    /// demo's default. A window the host cannot open is dropped.
    fn on_double_click(id: u32) {
        let i = (id - TILE_BASE) as usize;
        let demo = DEMOS
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|d| d.get(i))
            .cloned();
        let Some(demo) = demo else { return }; // not a tile (or out of range)
        match open_window(
            &demo.src,
            &demo.name,
            0,
            0,
            demo.default_width,
            demo.default_height,
        ) {
            Ok(handle) => OPEN.lock().unwrap().push(handle),
            Err(_) => {}
        }
    }

    /// A window was closed by the user: drop it from the open set.
    fn on_window_close(window: u32) {
        OPEN.lock().unwrap().retain(|&h| h != window);
    }

    // The plain-app events (see the `app` world): the desktop has no
    // editable input and no clock, and a single click on a tile does nothing
    // (the double-click opens), so all three are ignored.
    fn on_click(_id: u32) {}

    fn on_text(_id: u32, _text: String) {}

    fn on_tick() {}
}

export!(Desktop);
