//! A todo list — the structural demo. The contract's diff expresses add,
//! remove, and re-parent, and the unit tests cover both sides in isolation;
//! this is the demo that drives them through a real component.
//!
//! The contract has no text input (the README's roadmap), so the "add"
//! button cycles a preset list of labels instead of a text box.
//!
//! The layout: a column with the "todos" header, the active column, the done
//! column, and a footer (the count, the add button, the clear button). Each
//! todo row is a row with a checkbox, a label, and a delete button.
//!
//! **The ids.** A todo created i-th gets its row 100+4i while active and
//! 500+4i while done; its checkbox, label, and delete keep 101+4i, 102+4i,
//! and 103+4i in both columns. The row's id changes with the column because
//! the contract moves a container by removing the subtree and re-adding it
//! (a host links a container's children in list order, and a re-parented
//! container would leave them behind); the children's ids do not, so a move
//! re-parents them — same id, new parent.
//!
//! **The order.** Both columns are newest-first: the host inserts a new or
//! moved node just after its parent, the top of the column, so the component
//! keeps that order and `view()` and the retained list agree.
//!
//! It never calls `schedule` (it changes only on a click), so the host never
//! wakes it.

wit_bindgen::generate!({
    world: "app",
    path: "../../wit",
});

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;
// `Node` and `Mutation` are emitted by `generate!` at this scope; the
// ergonomic tree and its token mirrors come from the shared component-side
// crate, and the generated records/variants live in the package module.
use component_ui::{ui, Color, Direction, Size, Style, Tree};
use waui::ui::widget as wit;

/// One todo. `idx` is its creation index, the source of its node ids (never
/// reused); `label` is a preset; `done` picks its column and its row's style.
#[derive(Clone, Copy)]
struct Todo {
    idx: u32,
    label: &'static str,
    done: bool,
}

/// The labels the "add" button cycles (the contract has no text input).
const PRESETS: &[&str] = &[
    "buy milk",
    "walk the dog",
    "water plants",
    "write code",
    "ship the demo",
    "take a break",
];

/// The two lists, newest first (see the module docs, *The order*).
static ACTIVE: Mutex<Vec<Todo>> = Mutex::new(Vec::new());
static DONE: Mutex<Vec<Todo>> = Mutex::new(Vec::new());
static INITED: AtomicBool = AtomicBool::new(false);
/// The next creation index; the seeded todos take the first three.
static NEXT: AtomicU32 = AtomicU32::new(3);
/// The next preset label for the add button; the seed consumed the first
/// three.
static PRESET: AtomicUsize = AtomicUsize::new(3);

/// Seed the lists once: the first three presets, the last one done, added in
/// index order (so the newest, 1, sits at the front of the active list).
fn seed(active: &mut Vec<Todo>, done: &mut Vec<Todo>) {
    if INITED.load(Ordering::Relaxed) {
        return;
    }
    active.push(Todo {
        idx: 1,
        label: PRESETS[1],
        done: false,
    });
    active.push(Todo {
        idx: 0,
        label: PRESETS[0],
        done: false,
    });
    done.push(Todo {
        idx: 2,
        label: PRESETS[2],
        done: true,
    });
    INITED.store(true, Ordering::Relaxed);
}

/// A copy of the lists, seeding on first use.
fn snapshot() -> (Vec<Todo>, Vec<Todo>) {
    let mut active = ACTIVE.lock().unwrap();
    let mut done = DONE.lock().unwrap();
    seed(&mut active, &mut done);
    (active.clone(), done.clone())
}

/// One todo's row: the checkbox, the label, and the delete button. The row's
/// id changes with the column (100+4i / 500+4i); the children's do not.
fn row(t: &Todo) -> Tree {
    let base = t.idx * 4;
    ui! {
        row id = base + if t.done { 500 } else { 100 }, {
            button label = if t.done { "✓" } else { "○" },
                   id = 101 + base,
                   color = if t.done { Color::Positive } else { Color::Neutral },
                   size = Size::Medium,
            text content = t.label,
                 id = 102 + base,
                 color = if t.done { Color::Warning } else { Color::Neutral },
                 size = Size::Medium,
            button label = "✕", id = 103 + base, color = Color::Negative, size = Size::Medium,
        }
    }
}

/// The footer: the count, the add button, and the clear button.
fn footer(left: usize) -> Tree {
    ui! {
        row id = 5, {
            text content = format!("{left} left"), id = 6, size = Size::Small,
            button label = "add", id = 7, color = Color::Positive, size = Size::Medium,
            button label = "clear done", id = 8, color = Color::Negative, size = Size::Medium,
        }
    }
}

/// The todo list's whole UI, nested. The columns' rows are spliced into the
/// columns' children (a list child, not one child per row).
fn tree(active: &[Todo], done: &[Todo]) -> Tree {
    ui! {
        column id = 1, {
            text content = "todos", id = 2, color = Color::Accent, size = Size::Large,
            column id = 3, { active.iter().map(row) }
            column id = 4, { done.iter().map(row) }
            footer(active.len()),
        }
    }
}

/// Project the ergonomic tree's tokens onto this crate's generated types:
/// each `generate!` is crate-local, so the two type families are distinct
/// even where identical in shape.
fn to_node(tree: &Tree, parent: Option<u32>) -> Node {
    let style = |s: Style| wit::Style {
        color: match s.color {
            Color::Neutral => wit::Color::Neutral,
            Color::Accent => wit::Color::Accent,
            Color::Positive => wit::Color::Positive,
            Color::Negative => wit::Color::Negative,
            Color::Warning => wit::Color::Warning,
        },
        size: match s.size {
            Size::Small => wit::Size::Small,
            Size::Medium => wit::Size::Medium,
            Size::Large => wit::Size::Large,
        },
    };
    match tree {
        Tree::Text(t) => Node::Text(wit::Text {
            id: t.id,
            parent,
            content: t.content.clone(),
            style: style(t.style),
        }),
        Tree::Button(b) => Node::Button(wit::Button {
            id: b.id,
            parent,
            label: b.label.clone(),
            style: style(b.style),
        }),
        Tree::Container(c) => Node::Container(wit::Container {
            id: c.id,
            parent,
            direction: match c.direction {
                Direction::Row => wit::Direction::Row,
                Direction::Column => wit::Direction::Column,
            },
        }),
    }
}

struct TodoList;

impl Guest for TodoList {
    fn view() -> Vec<Node> {
        let (active, done) = snapshot();
        let t = tree(&active, &done);
        // The full snapshot becomes the reference the next diff compares
        // against.
        component_ui::note_view(&t);
        component_ui::lower(&t, to_node)
    }

    fn changes() -> Vec<Mutation> {
        // Bootstrap, a move, a removal: `component-ui`'s diff against its
        // retained last export.
        let (active, done) = snapshot();
        component_ui::diff(&tree(&active, &done), |c| match c {
            component_ui::DiffNode::Changed(node, parent) => {
                Mutation::Changed(to_node(node, parent))
            }
            component_ui::DiffNode::Removed(id) => Mutation::Removed(id),
        })
    }

    fn on_click(id: u32) {
        let mut active = ACTIVE.lock().unwrap();
        let mut done = DONE.lock().unwrap();
        seed(&mut active, &mut done);
        match id {
            // The add button: the next preset, newest first.
            7 => {
                let idx = NEXT.fetch_add(1, Ordering::Relaxed);
                let p = PRESET.fetch_add(1, Ordering::Relaxed);
                active.insert(
                    0,
                    Todo {
                        idx,
                        label: PRESETS[p % PRESETS.len()],
                        done: false,
                    },
                );
            }
            // The clear button: the done column empties.
            8 => done.clear(),
            // A row's checkbox (101+4i) or delete (103+4i).
            _ => {
                let (idx, remove) = match id % 4 {
                    1 => ((id - 101) / 4, false),
                    3 => ((id - 103) / 4, true),
                    _ => return,
                };
                // The todo in whichever list holds it; `None` if it is gone
                // (a stale click) or deleted.
                let moved = if let Some(pos) = active.iter().position(|t| t.idx == idx) {
                    let mut t = active.remove(pos);
                    if remove {
                        None
                    } else {
                        t.done = true;
                        Some(t)
                    }
                } else if let Some(pos) = done.iter().position(|t| t.idx == idx) {
                    let mut t = done.remove(pos);
                    if remove {
                        None
                    } else {
                        t.done = false;
                        Some(t)
                    }
                } else {
                    None
                };
                if let Some(t) = moved {
                    // Newest first in the column it joins.
                    if t.done {
                        done.insert(0, t);
                    } else {
                        active.insert(0, t);
                    }
                }
            }
        }
    }

    fn on_tick() {}
}

export!(TodoList);
