//! A counter whose UI and logic live entirely in the component.
//!
//! The host is a dumb renderer: it draws whatever `view()` returns and
//! forwards clicks via `on-click`. Nothing about this counter (its state,
//! its layout, its buttons) exists in the host.

wit_bindgen::generate!({
    world: "app",
    path: "../../wit",
});

use std::sync::atomic::{AtomicI32, Ordering};
// `Node` itself is emitted by `generate!` at this scope; the records it
// contains come from the package module.
use waui::ui::widget::{Button, Color, Container, Direction, Size, Style, Text};

static VALUE: AtomicI32 = AtomicI32::new(0);

struct Counter;

impl Guest for Counter {
    fn view() -> Vec<Node> {
        let value = VALUE.load(Ordering::Relaxed);
        vec![
            Node::Container(Container {
                id: 1,
                parent: None,
                direction: Direction::Column,
            }),
            // The number is the star of the show; its color is state.
            Node::Button(Button {
                id: 2,
                parent: Some(1),
                label: "+".to_string(),
                style: Style { color: Color::Positive, size: Size::Medium },
            }),
            Node::Text(Text {
                id: 3,
                parent: Some(1),
                content: value.to_string(),
                style: Style {
                    color: if value < 0 {
                        Color::Negative
                    } else {
                        Color::Neutral
                    },
                    size: Size::Large,
                },
            }),
            Node::Button(Button {
                id: 4,
                parent: Some(1),
                label: "-".to_string(),
                style: Style { color: Color::Negative, size: Size::Medium },
            }),
        ]
    }

    fn on_click(id: u32) {
        let delta = match id {
            2 => 1,
            4 => -1,
            _ => 0,
        };
        VALUE.fetch_add(delta, Ordering::Relaxed);
    }

    fn on_tick() {}
}

export!(Counter);
