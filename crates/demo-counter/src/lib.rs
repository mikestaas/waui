//! A counter whose UI and logic live entirely in the component.
//!
//! The host is a plain renderer: it draws what `view()` returns and
//! forwards clicks via `on-click`.
//!
//! The view is a nested `component-ui` tree projected into the contract's
//! flat node list at the ABI boundary.
//!
//! The diff: only the readout depends on state, so after an event the
//! component reports exactly that node (the whole tree on first contact).

wit_bindgen::generate!({
    world: "app",
    path: "../../wit",
});

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
// `Node` and `Mutation` are emitted by `generate!` at this scope; the
// ergonomic tree and its token mirrors come from the shared component-side
// crate, and the generated records/variants live in the package module.
use component_ui::{Color, Direction, Size, Style, Tree};
use waui::ui::widget as wit;

static VALUE: AtomicI32 = AtomicI32::new(0);
/// Set when the state changed since it was last exported.
static DIRTY: AtomicBool = AtomicBool::new(false);
/// Cleared once the initial tree was exported, via `view` or `changes`.
static EXPORTED: AtomicBool = AtomicBool::new(false);

/// Record that the current state has just been exported (as a full
/// snapshot or as a diff): nothing is pending any more.
fn mark_exported() {
    DIRTY.store(false, Ordering::Relaxed);
    EXPORTED.store(true, Ordering::Relaxed);
}

/// The readout is the only node that depends on `VALUE`; its color is
/// state (negative below zero).
fn readout(value: i32) -> Tree {
    Tree::text(
        value.to_string(),
        Style::new(
            if value < 0 {
                Color::Negative
            } else {
                Color::Neutral
            },
            Size::Large,
        ),
    )
    .id(3)
}

/// The counter's whole UI, nested.
fn tree(value: i32) -> Tree {
    Tree::column(vec![
        Tree::button("+", Style::new(Color::Positive, Size::Medium)).id(2),
        readout(value),
        Tree::button("-", Style::new(Color::Negative, Size::Medium)).id(4),
    ])
    .id(1)
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

struct Counter;

impl Guest for Counter {
    fn view() -> Vec<Node> {
        let value = VALUE.load(Ordering::Relaxed);
        // A full snapshot means everything is accounted for.
        mark_exported();
        component_ui::lower(&tree(value), to_node)
    }

    fn changes() -> Vec<Mutation> {
        if !EXPORTED.load(Ordering::Relaxed) {
            // First contact with this host: it has seen nothing, so
            // everything is new to it. (A host may bootstrap from
            // `changes` alone.)
            let value = VALUE.load(Ordering::Relaxed);
            mark_exported();
            return component_ui::lower(&tree(value), to_node)
                .into_iter()
                .map(Mutation::Changed)
                .collect();
        }
        if !DIRTY.load(Ordering::Relaxed) {
            // A no-op event (a tick, a click on an unknown id): there is
            // nothing for the host to do.
            return Vec::new();
        }
        DIRTY.store(false, Ordering::Relaxed);
        // The buttons and the box are static; only the readout changed.
        // Projected standing alone, its parent is stated explicitly (inside
        // `view`, the lowering walk states it).
        let value = VALUE.load(Ordering::Relaxed);
        vec![Mutation::Changed(to_node(&readout(value), Some(1)))]
    }

    fn on_click(id: u32) {
        let delta = match id {
            2 => 1,
            4 => -1,
            _ => 0,
        };
        if delta != 0 {
            VALUE.fetch_add(delta, Ordering::Relaxed);
            DIRTY.store(true, Ordering::Relaxed);
        }
    }

    fn on_tick() {}
}

export!(Counter);
