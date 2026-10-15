//! A stopwatch that uses the host's clock.
//!
//! Demonstrates the *import* side of the world: the component polls `now()`
//! (a host-provided service) and renders elapsed time, driven by the host's
//! periodic `on-tick` calls.

wit_bindgen::generate!({
    world: "app",
    path: "../../wit",
});

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
// `Node` itself is emitted by `generate!` at this scope; the records it
// contains come from the package module.
use waui::ui::widget::{Button, Container, Direction, Text};

const NOT_RUNNING: u64 = 0;

static START: AtomicU64 = AtomicU64::new(NOT_RUNNING);
static FROZEN: AtomicU64 = AtomicU64::new(0);
static RUNNING: AtomicBool = AtomicBool::new(false);

struct Stopwatch;

impl Guest for Stopwatch {
    fn view() -> Vec<Node> {
        let now = now();
        let running = RUNNING.load(Ordering::Relaxed);
        let elapsed = if running {
            let start = START.load(Ordering::Relaxed);
            if start == NOT_RUNNING {
                0
            } else {
                now.saturating_sub(start)
            }
        } else {
            FROZEN.load(Ordering::Relaxed)
        };

        vec![
            Node::Container(Container {
                id: 1,
                parent: None,
                direction: Direction::Column,
            }),
            Node::Text(Text {
                id: 2,
                parent: Some(1),
                content: format!("{elapsed} ms"),
            }),
            Node::Button(Button {
                id: 3,
                parent: Some(1),
                label: if running {
                    "pause".to_string()
                } else {
                    "start".to_string()
                },
            }),
        ]
    }

    fn on_click(id: u32) {
        if id != 3 {
            return;
        }
        let now = now();
        if RUNNING.swap(false, Ordering::Relaxed) {
            let start = START.load(Ordering::Relaxed);
            if start != NOT_RUNNING {
                FROZEN.store(now.saturating_sub(start), Ordering::Relaxed);
            }
        } else {
            START.store(now, Ordering::Relaxed);
            RUNNING.store(true, Ordering::Relaxed);
        }
    }

    fn on_tick() {}
}

export!(Stopwatch);
