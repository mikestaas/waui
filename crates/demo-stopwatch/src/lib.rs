//! A stopwatch that uses the host's clock: the component polls `now()` and
//! renders elapsed time.
//!
//! Its diff is state-dependent: while running, the readout's value changes
//! with the clock, so each `changes` reports it; while paused, it reports
//! nothing. The refresh that makes the running readout visible is requested
//! by the component — `schedule` at 250ms while running, 0 while paused —
//! so the host wakes it only while the readout moves. The view is a nested
//! `component-ui` tree projected into the contract's flat list at the ABI
//! boundary; the diff state lives in `component-ui`, which retains the last
//! export and reports what differs.

wit_bindgen::generate!({
    world: "app",
    path: "../../wit",
});

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
// `Node` and `Mutation` are emitted by `generate!` at this scope; the
// ergonomic tree and its token mirrors come from the shared component-side
// crate, and the generated records/variants live in the package module.
use component_ui::{ui, Color, Direction, Size, Style, Tree};
use waui::ui::widget as wit;

const NOT_RUNNING: u64 = 0;
/// The requested refresh period while running, in ms.
const REFRESH_PERIOD: u64 = 250;

static START: AtomicU64 = AtomicU64::new(NOT_RUNNING);
static FROZEN: AtomicU64 = AtomicU64::new(0);
static RUNNING: AtomicBool = AtomicBool::new(false);

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

/// The readout takes the host's accent color while running; the button's
/// label and color name the action.
fn readout(elapsed: u64, running: bool) -> Tree {
    ui! {
        text content = format!("{elapsed} ms"), id = 2,
            color = if running { Color::Accent } else { Color::Neutral },
            size = Size::Large,
    }
}

fn button(running: bool) -> Tree {
    ui! {
        button label = if running { "pause" } else { "start" }, id = 3,
            color = if running { Color::Warning } else { Color::Positive },
            size = Size::Medium,
    }
}

/// The stopwatch's whole UI, nested.
fn tree(elapsed: u64, running: bool) -> Tree {
    ui! {
        column id = 1, {
            readout(elapsed, running),
            button(running),
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
        let t = tree(elapsed, running);
        // The full snapshot becomes the reference the next diff compares
        // against.
        component_ui::note_view(&t);
        component_ui::lower(&t, to_node)
    }

    fn changes() -> Vec<Mutation> {
        // `component-ui`'s diff against its retained last export; while
        // paused, nothing differs and this is empty.
        component_ui::diff(
            &tree(elapsed(), RUNNING.load(Ordering::Relaxed)),
            |c| match c {
                component_ui::DiffNode::Changed(node, parent) => {
                    Mutation::Changed(to_node(node, parent))
                }
                component_ui::DiffNode::Removed(id) => Mutation::Removed(id),
            },
        )
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
        // The refresh follows `running`: the host wakes the component at
        // the period while running, never while paused.
        schedule(if RUNNING.load(Ordering::Relaxed) {
            REFRESH_PERIOD
        } else {
            0
        });
    }

    // A tick changes no state: while running, the next `changes` re-derives
    // the readout and reports it; while paused, the value is frozen and the
    // diff is empty.
    fn on_tick() {}
}

export!(Stopwatch);
