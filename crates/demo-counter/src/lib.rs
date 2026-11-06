//! A counter whose UI and logic live entirely in the component.
//!
//! The view is a nested `component-ui` tree (see that crate for the tree,
//! the diff, and the flat-list projection). Only the readout depends on
//! state; the handler mutates state only, and the diff reports it.
//!
//! The counter has no input and never calls `schedule` (it changes only on
//! a click).

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

/// Project the ergonomic tree onto this crate's generated types.
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
        component_ui::note_view(&t);
        component_ui::lower(&t, to_node)
    }

    fn changes() -> Vec<Mutation> {
        // Reports the changed readout (the whole tree on first contact).
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
        // State only: the next build re-derives the readout, the diff reports it.
        if delta != 0 {
            VALUE.fetch_add(delta, Ordering::Relaxed);
        }
    }

    // No input, so the buffer is ignored; the `String`'s drop frees it.
    fn on_text(_id: u32, _text: String) {}

    fn on_tick() {}
}

export!(Counter);
