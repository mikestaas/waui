//! Headless check of the TUI host's frame rendering: run the host binary
//! with `--dump` (one frame into ratatui's `TestBackend`, printed as text
//! with ANSI SGR codes) and assert on the rendered frame — the layout, the
//! semantic colors, and the host's focus ring.
//!
//! Each scenario runs against both producer languages: the Rust component
//! (cargo + wit-bindgen) and its AssemblyScript one (asc + the hand-rolled
//! ABI). The assertions are shared.

use std::path::Path;
use std::process::Command;
use waui_test::{
    as_toolchain_available, build_component, build_component_as, build_component_wat,
    component_path, component_path_as, component_path_wat, wat_toolchain_available, AS_BUILD_LOCK,
    RUST_BUILD_LOCK, WAT_BUILD_LOCK,
};

/// Run the host binary in `--dump` mode: one frame, printed as text with
/// ANSI SGR codes for the styled cells. `clicks` are injected before the
/// frame, like a pointer would drive them.
fn dump(path: &Path, clicks: &[u32]) -> String {
    let bin = env!("CARGO_BIN_EXE_host-tui");
    let mut cmd = Command::new(bin);
    cmd.arg("--dump");
    for id in clicks {
        cmd.arg("--click").arg(id.to_string());
    }
    cmd.arg(path);
    let out = cmd.output().expect("failed to run the host-tui binary");
    assert!(
        out.status.success(),
        "host-tui --dump failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("the dump is not valid utf-8")
}

/// Strip ANSI escape sequences (the SGR color/weight codes), keeping the
/// visible characters.
fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A frame line: the visible text (trimmed) and the raw bytes (with the
/// SGR codes the color assertions check).
struct Line {
    plain: String,
    raw: String,
}

fn lines(frame: &str) -> Vec<Line> {
    frame
        .lines()
        .map(|l| Line {
            plain: strip_ansi(l).trim().to_string(),
            raw: l.to_string(),
        })
        .collect()
}

fn find<'a>(lines: &'a [Line], want: &str) -> &'a Line {
    lines
        .iter()
        .find(|l| l.plain == want)
        .unwrap_or_else(|| panic!("no line with {want:?} in the frame"))
}

fn find_containing<'a>(lines: &'a [Line], want: &str) -> &'a Line {
    lines
        .iter()
        .find(|l| l.plain.contains(want))
        .unwrap_or_else(|| panic!("no line containing {want:?} in the frame"))
}

// The semantic colors, as the host's SGR codes: the focused override (cyan
// bold), the neutral readout (white bold), and negative (red).
const SGR_CYAN_BOLD: &str = "\x1b[36m\x1b[1m";
const SGR_WHITE_BOLD: &str = "\x1b[97m\x1b[1m";
const SGR_RED: &str = "\x1b[31m";

/// The counter's frame: the focus ring on the first button, the readout's
/// color following the value's sign, and the hint bar whole.
fn assert_counter_frame(path: &Path) {
    // Fresh: 0, neutral, and the "+" button (the first focusable) focused.
    let frame = dump(path, &[]);
    let ls = lines(&frame);
    assert!(
        ls[0].plain.contains("q: quit"),
        "the hint bar is clipped: {:?}",
        ls[0].plain
    );
    let plus = find(&ls, "│+│");
    assert!(
        plus.raw.contains(SGR_CYAN_BOLD),
        "the focused + button should be cyan bold"
    );
    let readout = find(&ls, "0");
    assert!(
        readout.raw.contains(SGR_WHITE_BOLD),
        "a non-negative readout should be neutral (white bold)"
    );
    assert!(!readout.raw.contains(SGR_RED));
    let minus = find(&ls, "│-│");
    assert!(minus.raw.contains(SGR_RED), "the - button should be red");
    assert!(
        !minus.raw.contains(SGR_CYAN_BOLD),
        "the - button must not carry the focus ring"
    );

    // + + - = 1: still non-negative, still neutral.
    let ls = lines(&dump(path, &[2, 2, 4]));
    let readout = find(&ls, "1");
    assert!(readout.raw.contains(SGR_WHITE_BOLD));
    assert!(!readout.raw.contains(SGR_RED));

    // - - - = -3: the readout takes the negative tone.
    let ls = lines(&dump(path, &[4, 4, 4]));
    let readout = find(&ls, "-3");
    assert!(
        readout.raw.contains(SGR_RED),
        "a negative readout should be red"
    );
}

/// The stopwatch's frame: the readout's color and the button's label follow
/// the running state; the readout's value is a small elapsed time after a
/// start (it is measured, so only its order of magnitude is asserted).
fn assert_stopwatch_frame(path: &Path) {
    // Paused: 0 ms, neutral, and the start button focused.
    let ls = lines(&dump(path, &[]));
    let readout = ls
        .iter()
        .find(|l| l.plain.ends_with(" ms"))
        .unwrap_or_else(|| panic!("no readout line in the frame"));
    assert_eq!(readout.plain, "0 ms");
    assert!(
        readout.raw.contains(SGR_WHITE_BOLD),
        "a paused readout should be neutral (white bold)"
    );
    let start = find_containing(&ls, "start");
    assert!(
        start.raw.contains(SGR_CYAN_BOLD),
        "the focused start button should be cyan bold"
    );

    // Started: the readout takes the accent tone, the button reads pause,
    // and the value is a small elapsed time (the frame is drawn moments
    // after the click).
    let ls = lines(&dump(path, &[3]));
    let readout = ls
        .iter()
        .find(|l| l.plain.ends_with(" ms"))
        .unwrap_or_else(|| panic!("no readout line in the frame"));
    let ms = readout
        .plain
        .trim_end_matches(" ms")
        .parse::<u64>()
        .unwrap_or_else(|_| panic!("readout {:?} is not a millisecond value", readout.plain));
    assert!(
        ms < 50,
        "the frame is drawn moments after the start: {ms} ms"
    );
    assert!(
        readout.raw.contains(SGR_CYAN_BOLD),
        "a running readout should be accent (cyan bold)"
    );
    let pause = find_containing(&ls, "pause");
    assert!(
        pause.raw.contains(SGR_CYAN_BOLD),
        "the focused pause button should be cyan bold"
    );
}

#[test]
fn counter_dump() {
    let _guard = RUST_BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    build_component("demo-counter");
    assert_counter_frame(&component_path("demo-counter"));
}

#[test]
fn counter_dump_as() {
    if !as_toolchain_available() {
        eprintln!("skipping: AssemblyScript toolchain not installed (run `npm install`)");
        return;
    }
    let _guard = AS_BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    build_component_as("demo-counter");
    assert_counter_frame(&component_path_as("demo-counter"));
}

#[test]
fn counter_dump_wat() {
    if !wat_toolchain_available() {
        eprintln!("skipping: wasm-tools not on the PATH");
        return;
    }
    let _guard = WAT_BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    build_component_wat("demo-counter");
    assert_counter_frame(&component_path_wat("demo-counter"));
}

#[test]
fn stopwatch_dump() {
    let _guard = RUST_BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    build_component("demo-stopwatch");
    assert_stopwatch_frame(&component_path("demo-stopwatch"));
}

#[test]
fn stopwatch_dump_as() {
    if !as_toolchain_available() {
        eprintln!("skipping: AssemblyScript toolchain not installed (run `npm install`)");
        return;
    }
    let _guard = AS_BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    build_component_as("demo-stopwatch");
    assert_stopwatch_frame(&component_path_as("demo-stopwatch"));
}

#[test]
fn stopwatch_dump_wat() {
    if !wat_toolchain_available() {
        eprintln!("skipping: wasm-tools not on the PATH");
        return;
    }
    let _guard = WAT_BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    build_component_wat("demo-stopwatch");
    assert_stopwatch_frame(&component_path_wat("demo-stopwatch"));
}
