//! A counter whose UI and logic live entirely in the component.
//!
//! The host is a plain renderer: it draws what `view()` returns and
//! forwards clicks via `on-click`. `on-text` is the world's text-input
//! channel; the counter has no input, so it is a no-op here (the world
//! requires the export on every component).
//!
//! The view is a nested `component-ui` tree projected into the contract's
//! flat node list at the ABI boundary.
//!
//! The diff: only the readout depends on state, so after an event the
//! component reports that node (the whole tree on first contact). The diff
//! state lives in `component-ui`, which retains the last export and reports
//! what differs; the handler mutates state only.
//!
//! It never calls `schedule` (it changes only on a click), so the host
//! never wakes it.

wit_bindgen::generate!({
    world: "app",
    path: "../../wit",
});

use std::sync::atomic::{AtomicI32, Ordering};
// `Node` and `Mutation` are emitted by `generate!` at this scope; the
// ergonomic tree and its token mirrors come from the shared component-side
// crate, and the generated records/variants live in the package module.
use component_ui::{ui, Align, Color, Direction, Size, Style, Tree};
use waui::ui::widget as wit;

static VALUE: AtomicI32 = AtomicI32::new(0);

/// The readout is the only node that depends on `VALUE`; its color is
/// state (negative below zero).
fn readout(value: i32) -> Tree {
    ui! {
        text content = value.to_string(), id = 3,
            color = if value < 0 { Color::Negative } else { Color::Neutral },
            size = Size::Large,
    }
}

/// The counter's whole UI, nested.
fn tree(value: i32) -> Tree {
    ui! {
        column id = 1, {
            button label = "+", id = 2, color = Color::Positive, size = Size::Medium,
            readout(value),
            button label = "-", id = 4, color = Color::Negative, size = Size::Medium,
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
            align: match c.align {
                Align::Start => wit::Align::Start,
                Align::Center => wit::Align::Center,
                Align::End => wit::Align::End,
            },
        }),
    }
}

struct Counter;

impl Guest for Counter {
    fn view() -> Vec<Node> {
        let value = VALUE.load(Ordering::Relaxed);
        let t = tree(value);
        // The full snapshot becomes the reference the next diff compares
        // against.
        component_ui::note_view(&t);
        component_ui::lower(&t, to_node)
    }

    fn changes() -> Vec<Mutation> {
        // Bootstrap, a value change, a removal: `component-ui`'s diff
        // against its retained last export.
        component_ui::diff(&tree(VALUE.load(Ordering::Relaxed)), |c| match c {
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
        let delta = match id {
            2 => 1,
            4 => -1,
            _ => 0,
        };
        // State only: the next build re-derives the readout, and the diff
        // compares.
        if delta != 0 {
            VALUE.fetch_add(delta, Ordering::Relaxed);
        }
    }

    // The world's text-input channel: the counter has no input, so the
    // host's buffer is ignored (the export is required on every component).
    // The buffer is a string parameter the callee owns; the `String`'s drop
    // frees it (it came through the guest's `cabi_realloc`).
    fn on_text(_id: u32, _text: String) {}

    fn on_tick() {}
}

export!(Counter);
