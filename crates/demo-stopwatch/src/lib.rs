//! A stopwatch that uses the host's clock.
//!
//! Demonstrates the *import* side of the world: the component polls `now()`
//! (a host-provided service) and renders elapsed time, driven by the host's
//! periodic `on-tick` calls.
//!
//! Its diff is state-dependent, like its rendering: while running, every
//! tick reports the readout; while paused, it reports *nothing* — the
//! component simply has no changed nodes. Like the counter, its view is
//! written as a nested `component-ui` tree and projected into the
//! contract's flat list at the ABI boundary.

wit_bindgen::generate!({
    world: "app",
    path: "../../wit",
});

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
// `Node` and `Mutation` are emitted by `generate!` at this scope; the
// ergonomic tree and its token mirrors come from the shared component-side
// crate, and the generated records/variants live in the package module.
use component_ui::{Color, Direction, Size, Style, Tree};
use waui::ui::widget as wit;

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
fn readout(elapsed: u64, running: bool) -> Tree {
    Tree::text(
        format!("{elapsed} ms"),
        Style::new(
            if running {
                Color::Accent
            } else {
                Color::Neutral
            },
            Size::Large,
        ),
    )
    .id(2)
}

fn button(running: bool) -> Tree {
    Tree::button(
        if running {
            "pause"
        } else {
            "start"
        },
        Style::new(
            if running {
                Color::Warning
            } else {
                Color::Positive
            },
            Size::Medium,
        ),
    )
    .id(3)
}

/// The stopwatch's whole UI, nested.
fn tree(elapsed: u64, running: bool) -> Tree {
    Tree::column(vec![readout(elapsed, running), button(running)]).id(1)
}

/// The one seam between the ergonomic tree and the generated bindings:
/// each `generate!` is crate-local, so the tree's token mirrors and the
/// ABI's generated types are different, even if identical in shape.
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

struct Stopwatch;

impl Guest for Stopwatch {
    fn view() -> Vec<Node> {
        let running = RUNNING.load(Ordering::Relaxed);
        let elapsed = elapsed();
        // A full snapshot means everything is accounted for.
        mark_exported();
        component_ui::lower(&tree(elapsed, running), to_node)
    }

    fn changes() -> Vec<Mutation> {
        if !EXPORTED.load(Ordering::Relaxed) {
            // First contact with this host: everything is new to it.
            let running = RUNNING.load(Ordering::Relaxed);
            let elapsed = elapsed();
            mark_exported();
            return component_ui::lower(&tree(elapsed, running), to_node)
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
        let elapsed = elapsed();
        // Projected standing alone, so each node's parent is stated
        // explicitly (the lowering walk states it for us inside `view`).
        let mut out = Vec::new();
        if dirty & READOUT != 0 {
            out.push(Mutation::Changed(to_node(&readout(elapsed, running), Some(1))));
        }
        if dirty & BUTTON != 0 {
            out.push(Mutation::Changed(to_node(&button(running), Some(1))));
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
