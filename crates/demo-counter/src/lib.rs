//! A counter whose UI and logic live entirely in the component.
//!
//! The host is a dumb renderer: it draws whatever `view()` returns and
//! forwards clicks via `on-click`. Nothing about this counter (its state,
//! its layout, its buttons) exists in the host.
//!
//! The counter also exercises the diff channel: only the readout depends
//! on its state, so after an event it reports exactly one changed node —
//! never the whole tree.

wit_bindgen::generate!({
    world: "app",
    path: "../../wit",
});

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
// `Node` and `Mutation` are emitted by `generate!` at this scope; the
// records they contain come from the package module.
use waui::ui::widget::{Button, Color, Container, Direction, Size, Style, Text};

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

/// The readout is the only node that depends on `VALUE`.
fn readout(value: i32) -> Node {
    Node::Text(Text {
        id: 3,
        parent: Some(1),
        content: value.to_string(),
        // The number is the star of the show; its color is state.
        style: Style {
            color: if value < 0 { Color::Negative } else { Color::Neutral },
            size: Size::Large,
        },
    })
}

fn full_view(value: i32) -> Vec<Node> {
    vec![
        Node::Container(Container {
            id: 1,
            parent: None,
            direction: Direction::Column,
        }),
        Node::Button(Button {
            id: 2,
            parent: Some(1),
            label: "+".to_string(),
            style: Style { color: Color::Positive, size: Size::Medium },
        }),
        readout(value),
        Node::Button(Button {
            id: 4,
            parent: Some(1),
            label: "-".to_string(),
            style: Style { color: Color::Negative, size: Size::Medium },
        }),
    ]
}

struct Counter;

impl Guest for Counter {
    fn view() -> Vec<Node> {
        let value = VALUE.load(Ordering::Relaxed);
        // A full snapshot means everything is accounted for.
        mark_exported();
        full_view(value)
    }

    fn changes() -> Vec<Mutation> {
        if !EXPORTED.load(Ordering::Relaxed) {
            // First contact with this host: it has seen nothing, so
            // everything is new to it. (A host may bootstrap from
            // `changes` alone.)
            let value = VALUE.load(Ordering::Relaxed);
            mark_exported();
            return full_view(value)
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
        vec![Mutation::Changed(readout(
            VALUE.load(Ordering::Relaxed),
        ))]
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
