//! A stopwatch that uses the host's clock.
//!
//! Demonstrates the *import* side of the world: the component polls `now()`
//! (a host-provided service) and renders elapsed time, driven by the host's
//! periodic `on-tick` calls.
//!
//! Its diff is state-dependent, like its rendering: while running, every
//! tick reports the readout; while paused, it reports *nothing* — the
//! component simply has no changed nodes.

wit_bindgen::generate!({
    world: "app",
    path: "../../wit",
});

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
// `Node` and `Mutation` are emitted by `generate!` at this scope; the
// records they contain come from the package module.
use waui::ui::widget::{Button, Color, Container, Direction, Size, Style, Text};

const NOT_RUNNING: u64 = 0;

static START: AtomicU64 = AtomicU64::new(NOT_RUNNING);
static FROZEN: AtomicU64 = AtomicU64::new(0);
static RUNNING: AtomicBool = AtomicBool::new(false);
/// Which nodes are stale since the last export: bit 0 = the readout,
/// bit 1 = the button.
static DIRTY: AtomicU8 = AtomicU8::new(0);
/// Cleared once the initial tree was exported, via `view` or `changes`.
static EXPORTED: AtomicBool = AtomicBool::new(false);

const READOUT: u8 = 0b01;
const BUTTON: u8 = 0b10;

/// Record that the current state has just been exported (as a full
/// snapshot or as a diff): nothing is pending any more.
fn mark_exported() {
    DIRTY.store(0, Ordering::Relaxed);
    EXPORTED.store(true, Ordering::Relaxed);
}

/// The elapsed milliseconds, as the component currently sees it.
fn elapsed() -> u64 {
    let running = RUNNING.load(Ordering::Relaxed);
    if running {
        let start = START.load(Ordering::Relaxed);
        if start == NOT_RUNNING {
            0
        } else {
            now().saturating_sub(start)
        }
    } else {
        FROZEN.load(Ordering::Relaxed)
    }
}

/// While running the readout glows with the host's accent; the button
/// says what it will do (start = good, pause = careful).
fn readout(elapsed: u64, running: bool) -> Node {
    Node::Text(Text {
        id: 2,
        parent: Some(1),
        content: format!("{elapsed} ms"),
        style: Style {
            color: if running { Color::Accent } else { Color::Neutral },
            size: Size::Large,
        },
    })
}

fn button(running: bool) -> Node {
    Node::Button(Button {
        id: 3,
        parent: Some(1),
        label: if running {
            "pause".to_string()
        } else {
            "start".to_string()
        },
        style: Style {
            color: if running { Color::Warning } else { Color::Positive },
            size: Size::Medium,
        },
    })
}

fn full_view(elapsed: u64, running: bool) -> Vec<Node> {
    vec![
        Node::Container(Container {
            id: 1,
            parent: None,
            direction: Direction::Column,
        }),
        readout(elapsed, running),
        button(running),
    ]
}

struct Stopwatch;

impl Guest for Stopwatch {
    fn view() -> Vec<Node> {
        let running = RUNNING.load(Ordering::Relaxed);
        let elapsed = elapsed();
        // A full snapshot means everything is accounted for.
        mark_exported();
        full_view(elapsed, running)
    }

    fn changes() -> Vec<Mutation> {
        if !EXPORTED.load(Ordering::Relaxed) {
            // First contact with this host: everything is new to it.
            let running = RUNNING.load(Ordering::Relaxed);
            let elapsed = elapsed();
            mark_exported();
            return full_view(elapsed, running)
                .into_iter()
                .map(Mutation::Changed)
                .collect();
        }
        let dirty = DIRTY.swap(0, Ordering::Relaxed);
        if dirty == 0 {
            // Nothing changed (a tick while paused, an unknown click):
            // the host can skip its whole refresh.
            return Vec::new();
        }
        let running = RUNNING.load(Ordering::Relaxed);
        let mut out = Vec::new();
        if dirty & READOUT != 0 {
            out.push(Mutation::Changed(readout(elapsed(), running)));
        }
        if dirty & BUTTON != 0 {
            out.push(Mutation::Changed(button(running)));
        }
        out
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
        // Starting and stopping both change the readout (its color) and
        // the button (its label and color).
        DIRTY.store(READOUT | BUTTON, Ordering::Relaxed);
    }

    fn on_tick() {
        // The readout advances only while running.
        if RUNNING.load(Ordering::Relaxed) {
            DIRTY.fetch_or(READOUT, Ordering::Relaxed);
        }
    }
}

export!(Stopwatch);
