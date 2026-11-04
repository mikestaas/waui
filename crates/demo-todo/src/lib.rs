//! A todo list — the structural demo. The contract's diff expresses add,
//! remove, and re-parent, and the unit tests cover both sides in isolation;
//! this is the demo that drives them through a real component.
//!
//! The footer's "add" button uses the text input: it adds a todo with the
//! input's text and clears the field, or — when the field is empty — cycles
//! a preset list of labels. The input's text is component state: the host
//! reports each edit through `on-text` (the whole current buffer), the
//! component stores it, and the input node's content re-derives from it.
//!
//! The layout: a column with the "todos" header, the active column, the done
//! column, and a footer (the count, the input, the add button, the clear
//! button). Each todo row is a row with a checkbox, a label, and a delete
//! button.
//!
//! **The ids.** A todo created i-th gets its row 100+4i while active and
//! 500+4i while done; its checkbox, label, and delete keep 101+4i, 102+4i,
//! and 103+4i in both columns. The row's id changes with the column because
//! the contract moves a container by removing the subtree and re-adding it
//! (a host links a container's children in list order, and a re-parented
//! container would leave them behind); the children's ids do not, so a move
//! re-parents them — same id, new parent. The footer's input is 9.
//!
//! **The order.** Both columns are newest-first: the component keeps that
//! order and the host places a new or moved node at its index among the
//! parent's children, so `view()` and the retained list agree.
//!
//! It never calls `schedule` (it changes only on a click or a text edit), so
//! the host never wakes it.

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
/// reused); `label` is the typed text or a preset; `done` picks its column
/// and its row's style.
#[derive(Clone)]
struct Todo {
    idx: u32,
    label: String,
    done: bool,
}

/// The labels the "add" button cycles when the input is empty.
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
/// The input's text: the component's copy of the buffer the host reports
/// through `on-text`.
static INPUT: Mutex<String> = Mutex::new(String::new());

/// Seed the lists once: the first three presets, the last one done, added in
/// index order (so the newest, 1, sits at the front of the active list).
fn seed(active: &mut Vec<Todo>, done: &mut Vec<Todo>) {
    if INITED.load(Ordering::Relaxed) {
        return;
    }
    active.push(Todo {
        idx: 1,
        label: PRESETS[1].to_string(),
        done: false,
    });
    active.push(Todo {
        idx: 0,
        label: PRESETS[0].to_string(),
        done: false,
    });
    done.push(Todo {
        idx: 2,
        label: PRESETS[2].to_string(),
        done: true,
    });
    INITED.store(true, Ordering::Relaxed);
}

/// Copies of the lists and the input's text, seeding on first use.
fn snapshot() -> (Vec<Todo>, Vec<Todo>, String) {
    let mut active = ACTIVE.lock().unwrap();
    let mut done = DONE.lock().unwrap();
    seed(&mut active, &mut done);
    let input = INPUT.lock().unwrap().clone();
    (active.clone(), done.clone(), input)
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
            text content = t.label.clone(),
                 id = 102 + base,
                 color = if t.done { Color::Warning } else { Color::Neutral },
                 size = Size::Medium,
            button label = "✕", id = 103 + base, color = Color::Negative, size = Size::Medium,
        }
    }
}

/// The footer: the count, the input, the add button, and the clear button.
fn footer(left: usize, input: &str) -> Tree {
    ui! {
        row id = 5, {
            text content = format!("{left} left"), id = 6, size = Size::Small,
            input content = input, id = 9, size = Size::Medium,
            button label = "add", id = 7, color = Color::Positive, size = Size::Medium,
            button label = "clear done", id = 8, color = Color::Negative, size = Size::Medium,
        }
    }
}

/// The todo list's whole UI, nested. The columns' rows are spliced into the
/// columns' children (a list child, not one child per row).
fn tree(active: &[Todo], done: &[Todo], input: &str) -> Tree {
    ui! {
        column id = 1, {
            text content = "todos", id = 2, color = Color::Accent, size = Size::Large,
            column id = 3, { active.iter().map(row) }
            column id = 4, { done.iter().map(row) }
            footer(active.len(), input),
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
        Tree::Input(i) => Node::Input(wit::Input {
            id: i.id,
            parent,
            content: i.content.clone(),
            style: style(i.style),
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
        let (active, done, input) = snapshot();
        let t = tree(&active, &done, &input);
        // The full snapshot becomes the reference the next diff compares
        // against.
        component_ui::note_view(&t);
        component_ui::lower(&t, to_node)
    }

    fn changes() -> Vec<Mutation> {
        // Bootstrap, a move, a removal: `component-ui`'s diff against its
        // retained last export.
        let (active, done, input) = snapshot();
        component_ui::diff(&tree(&active, &done, &input), |c| match c {
            component_ui::DiffNode::Changed(node, parent, index) => {
                Mutation::Changed(wit::Changed {
                    node: to_node(node, parent),
                    index,
                })
            }
            component_ui::DiffNode::Removed(id) => Mutation::Removed(id),
        })
    }

    fn on_click(id: u32) {
        let mut active = ACTIVE.lock().unwrap();
        let mut done = DONE.lock().unwrap();
        seed(&mut active, &mut done);
        match id {
            // The add button: the input's text if the user typed any, else
            // the next preset; newest first.
            7 => {
                let idx = NEXT.fetch_add(1, Ordering::Relaxed);
                let label = {
                    let mut input = INPUT.lock().unwrap();
                    if input.is_empty() {
                        let p = PRESET.fetch_add(1, Ordering::Relaxed);
                        PRESETS[p % PRESETS.len()].to_string()
                    } else {
                        std::mem::take(&mut *input)
                    }
                };
                active.insert(
                    0,
                    Todo {
                        idx,
                        label,
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

    // The input (9): the host's whole current buffer. The component is the
    // source of truth for the field, so it stores the text verbatim; the
    // input node's content re-derives from it on the next export. `text` is
    // a string parameter the callee owns, allocated through the guest's
    // `cabi_realloc`, so the `String`'s drop is a valid free.
    fn on_text(id: u32, text: String) {
        if id == 9 {
            *INPUT.lock().unwrap() = text;
        }
    }

    fn on_tick() {}
}

export!(TodoList);
