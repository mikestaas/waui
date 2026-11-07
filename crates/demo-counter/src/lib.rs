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
use component_ui::{ui, Color, Size, Tree};
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

// Project the ergonomic tree onto this crate's generated types.
component_ui::to_node!(wit);

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
