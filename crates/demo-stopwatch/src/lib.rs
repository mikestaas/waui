//! A stopwatch that uses the host's clock: the component polls `now()` and
//! renders elapsed time.
//!
//! Its diff is state-dependent: while running, the readout changes with the
//! clock, so each `changes` reports it; while paused, it reports nothing.
//! It requests the refresh that makes the running readout visible —
//! `schedule` at 250ms while running, 0 while paused.
//!
//! The view is a nested `component-ui` tree (see that crate for the tree,
//! the diff, and the flat-list projection).

wit_bindgen::generate!({
    world: "app",
    path: "../../wit",
});

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
// `Node` and `Mutation` are emitted by `generate!` at this scope; the
// ergonomic tree and its token mirrors come from the shared component-side
// crate, and the generated records/variants live in the package module.
use component_ui::{ui, Color, Size, Tree};
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

// Project the ergonomic tree onto this crate's generated types.
component_ui::to_node!(wit);

struct Stopwatch;

impl Guest for Stopwatch {
    fn view() -> Vec<Node> {
        let running = RUNNING.load(Ordering::Relaxed);
        let elapsed = elapsed();
        let t = tree(elapsed, running);
        component_ui::note_view(&t);
        component_ui::lower(&t, to_node)
    }

    fn changes() -> Vec<Mutation> {
        // While paused, nothing differs and this is empty.
        component_ui::diff(
            &tree(elapsed(), RUNNING.load(Ordering::Relaxed)),
            |c| match c {
                component_ui::DiffNode::Changed(node, parent, index) => {
                    Mutation::Changed(wit::Changed {
                        node: to_node(node, parent),
                        index,
                    })
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
        schedule(if RUNNING.load(Ordering::Relaxed) {
            REFRESH_PERIOD
        } else {
            0
        });
    }

    fn on_tick() {}

    // No input, so the buffer is ignored; the `String`'s drop frees it.
    fn on_text(_id: u32, _text: String) {}
}

export!(Stopwatch);
