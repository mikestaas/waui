//! Headless check of the TUI host's frame rendering: run the host binary
//! with `--dump` (one frame into ratatui's `TestBackend`, printed as text
//! with ANSI SGR codes) and assert on the rendered frame — the layout, the
//! semantic colors, and the host's focus ring.
//!
//! Each scenario runs against the three producer languages: the Rust
//! component (cargo + wit-bindgen), its AssemblyScript one (asc + the
//! hand-rolled ABI), and its hand-rolled WAT one. The assertions are shared.
//!
//! The goldens in `tests/goldens/` are the visual regression layer: the
//! whole frame, byte-exact, shared by the three producers (they must render
//! identically, so the shared golden is also the parity check). The frame is
//! text, so the goldens are platform-stable and run everywhere. The
//! stopwatch's running state reads the clock, so it runs on a fake one that
//! advances (`--now <base> <step>`). Regenerate with
//! `UPDATE_GOLDENS=1 cargo test -p host-tui`.

use std::path::{Path, PathBuf};
use std::process::Command;
use waui_test::{
    as_toolchain_available, assert_golden, build_component, build_component_as,
    build_component_wat, component_path, component_path_as, component_path_wat,
    wat_toolchain_available, AS_BUILD_LOCK, RUST_BUILD_LOCK, WAT_BUILD_LOCK,
};

/// The fake clock for the stopwatch's running state: the n-th `now` call
/// returns `base + n * step`. The host's `schedule` call and the readout's
/// `elapsed` sample both read it, so the readout is a fixed 2 * step.
const FAKE_NOW: (u64, u64) = (1_000, 50);

/// Run the host binary in `--dump` mode: one frame, printed as text with
/// ANSI SGR codes for the styled cells. `clicks` are injected before the
/// frame, like a pointer would drive them; `now` is the fake clock's base
/// and step (`--now`), so a frame that reads the clock is deterministic;
/// `width` is the buffer's column count (`--width`); `None` sizes the
/// buffer to the tree, floored so the bar wraps to at most two rows.
fn dump(path: &Path, clicks: &[u32], now: Option<(u64, u64)>, width: Option<u16>) -> String {
    let bin = env!("CARGO_BIN_EXE_host-tui");
    let mut cmd = Command::new(bin);
    cmd.arg("--dump");
    if let Some(w) = width {
        cmd.arg("--width").arg(w.to_string());
    }
    if let Some((base, step)) = now {
        cmd.arg("--now").arg(base.to_string()).arg(step.to_string());
    }
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
// bold), the neutral readout (white bold), and the rest of the palette.
const SGR_CYAN_BOLD: &str = "\x1b[36m\x1b[1m";
const SGR_WHITE_BOLD: &str = "\x1b[97m\x1b[1m";
const SGR_WHITE: &str = "\x1b[97m";
const SGR_RED: &str = "\x1b[31m";
const SGR_GREEN: &str = "\x1b[32m";
const SGR_YELLOW: &str = "\x1b[33m";
const SGR_DIM: &str = "\x1b[2m";

/// The golden file for `name`, under this crate's `tests/goldens/`.
fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(name)
}

/// Compare the frame against its golden: the shared byte-exact check, with
/// the first differing lines in the failure message.
fn assert_frame_golden(frame: &str, name: &str) {
    assert_golden(frame.as_bytes(), &golden(name), frame_diff);
}

/// Save the frame as a per-producer reference golden: the shared golden's
/// name with `-{producer}` before the extension, in the e2e convention
/// (`counter-as-0`). Never compared; the Rust producer's reference is the
/// shared golden itself, so it writes nothing.
fn save_producer_golden(frame: &str, producer: &str, name: &str) {
    if producer == "rust" {
        return;
    }
    let p = golden(name);
    let stem = p.file_stem().unwrap_or_default().to_string_lossy();
    let ext = p.extension().unwrap_or_default().to_string_lossy();
    let file = p.with_file_name(format!("{stem}-{producer}.{ext}"));
    std::fs::write(&file, frame).expect("failed to write the producer golden");
}

/// The first differing lines (and a length note), for a failure message.
fn frame_diff(want: &[u8], actual: &[u8]) -> String {
    let want_text = String::from_utf8_lossy(want);
    let actual_text = String::from_utf8_lossy(actual);
    let want_lines: Vec<&str> = want_text.lines().collect();
    let actual_lines: Vec<&str> = actual_text.lines().collect();
    let mut out = String::new();
    let mut shown = 0;
    for (i, (w, a)) in want_lines.iter().zip(&actual_lines).enumerate() {
        if w != a {
            out.push_str(&format!("line {i}: golden {w:?} | actual {a:?}\n"));
            shown += 1;
            if shown == 5 {
                break;
            }
        }
    }
    if want_lines.len() != actual_lines.len() {
        out.push_str(&format!(
            "the golden has {} lines, the frame has {}\n",
            want_lines.len(),
            actual_lines.len()
        ));
    }
    if out.is_empty() {
        out.push_str("the frames differ, but line by line they match\n");
    }
    out
}

/// The counter's frame: the focus ring on the first button, the readout's
/// color following the value's sign, the hint bar whole, and the whole
/// frame against the golden.
fn assert_counter_frame(path: &Path, producer: &str) {
    // Fresh: 0, neutral, and the "+" button (the first focusable) focused.
    let frame = dump(path, &[], None, None);
    let ls = lines(&frame);
    assert!(
        ls.iter().any(|l| l.plain.contains("q: quit")),
        "the hint bar is clipped: {:?}",
        ls.iter().map(|l| l.plain.as_str()).collect::<Vec<_>>()
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
    assert_frame_golden(&frame, "counter-0.txt");
    save_producer_golden(&frame, producer, "counter-0.txt");

    // + + - = 1: still non-negative, still neutral.
    let frame = dump(path, &[2, 2, 4], None, None);
    let ls = lines(&frame);
    let readout = find(&ls, "1");
    assert!(readout.raw.contains(SGR_WHITE_BOLD));
    assert!(!readout.raw.contains(SGR_RED));
    assert_frame_golden(&frame, "counter-1.txt");
    save_producer_golden(&frame, producer, "counter-1.txt");

    // - - - = -3: the readout takes the negative tone.
    let frame = dump(path, &[4, 4, 4], None, None);
    let ls = lines(&frame);
    let readout = find(&ls, "-3");
    assert!(
        readout.raw.contains(SGR_RED),
        "a negative readout should be red"
    );
    assert_frame_golden(&frame, "counter--3.txt");
    save_producer_golden(&frame, producer, "counter--3.txt");
}

/// The stopwatch's frame: the readout's color and the button's label follow
/// the running state. Paused, the frame never reads the clock; running, it
/// runs on a fake one that advances, so the readout is a fixed 100 ms and
/// the whole frame is the golden.
fn assert_stopwatch_frame(path: &Path, producer: &str) {
    // Paused: 0 ms, neutral, and the start button focused.
    let frame = dump(path, &[], None, None);
    let ls = lines(&frame);
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
    assert_frame_golden(&frame, "stopwatch-paused.txt");
    save_producer_golden(&frame, producer, "stopwatch-paused.txt");

    // Started: the fake clock advances 50 ms per call, so the readout is
    // the fixed 100 ms; the frame takes the running colors (accent readout,
    // warning pause button).
    let frame = dump(path, &[3], Some(FAKE_NOW), None);
    let ls = lines(&frame);
    let readout = ls
        .iter()
        .find(|l| l.plain.ends_with(" ms"))
        .unwrap_or_else(|| panic!("no readout line in the frame"));
    assert_eq!(readout.plain, "100 ms");
    assert!(
        readout.raw.contains(SGR_CYAN_BOLD),
        "a running readout should be accent (cyan bold)"
    );
    let pause = find_containing(&ls, "pause");
    assert!(
        pause.raw.contains(SGR_CYAN_BOLD),
        "the focused pause button should be cyan bold"
    );
    assert_frame_golden(&frame, "stopwatch-running.txt");
    save_producer_golden(&frame, producer, "stopwatch-running.txt");
}

/// The todo's frames: the header's accent, the first row's checkbox
/// carrying the focus ring, the done row's positive checkbox and warning
/// label, the footer's dim count, and the whole frame against the golden.
/// The added state is the add button with an empty field: the next preset
/// label, newest first, the count up one.
fn assert_todo_frame(path: &Path, producer: &str) {
    let frame = dump(path, &[], None, None);
    let ls = lines(&frame);
    assert!(
        ls.iter().any(|l| l.plain.contains("q: quit")),
        "the hint bar is clipped: {:?}",
        ls.iter().map(|l| l.plain.as_str()).collect::<Vec<_>>()
    );
    let header = find(&ls, "todos");
    assert!(
        header.raw.contains(SGR_CYAN_BOLD),
        "the todos header should be accent (cyan bold)"
    );
    let first = find_containing(&ls, "walk the dog");
    assert!(
        first.raw.contains(SGR_CYAN_BOLD),
        "the first row's checkbox should carry the focus ring"
    );
    let second = find_containing(&ls, "buy milk");
    assert!(
        second.raw.contains(SGR_WHITE),
        "an active label should be neutral (white)"
    );
    assert!(
        !second.raw.contains(SGR_CYAN_BOLD),
        "the second row must not carry the focus ring"
    );
    let done = find_containing(&ls, "water plants");
    assert!(
        done.raw.contains(SGR_GREEN),
        "a done checkbox should be positive (green)"
    );
    assert!(
        done.raw.contains(SGR_YELLOW),
        "a done label should be warning (yellow)"
    );
    let footer = find_containing(&ls, "2 left");
    assert!(footer.raw.contains(SGR_DIM), "a small count should be dim");
    assert_frame_golden(&frame, "todo.txt");
    save_producer_golden(&frame, producer, "todo.txt");

    // The add button with an empty field: the next preset ("write code"),
    // at the top of the active column, the count up one.
    let frame = dump(path, &[9], None, None);
    let ls = lines(&frame);
    let new = find_containing(&ls, "write code");
    assert!(
        new.raw.contains(SGR_CYAN_BOLD),
        "the new row's checkbox should carry the focus ring"
    );
    assert!(
        new.raw.contains(SGR_WHITE),
        "an active label should be neutral (white)"
    );
    let done = find_containing(&ls, "water plants");
    assert!(
        done.raw.contains(SGR_GREEN),
        "a done checkbox should be positive (green)"
    );
    let footer = find_containing(&ls, "3 left");
    assert!(footer.raw.contains(SGR_DIM), "a small count should be dim");
    assert_frame_golden(&frame, "todo-added.txt");
    save_producer_golden(&frame, producer, "todo-added.txt");
}

#[test]
fn counter_dump() {
    let _guard = RUST_BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    build_component("demo-counter");
    assert_counter_frame(&component_path("demo-counter"), "rust");
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
    assert_counter_frame(&component_path_as("demo-counter"), "as");
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
    assert_counter_frame(&component_path_wat("demo-counter"), "wat");
}

/// `--width` sizes the dump buffer like a terminal of that width: the
/// hint bar wraps, every word of it stays visible, and no line overruns
/// the width. The default buffer (floored at half the hint's length) is
/// wider than 40 columns, so the flag changes the frame.
#[test]
fn counter_dump_width() {
    let _guard = RUST_BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    build_component("demo-counter");
    let path = component_path("demo-counter");

    let ls = lines(&dump(&path, &[], None, Some(40)));
    let mut words = Vec::new();
    for l in &ls {
        for w in l.plain.split_whitespace() {
            words.push(w.to_string());
        }
    }
    let want = [
        "waui",
        "(tui)",
        "up/down",
        "or",
        "j/k:",
        "focus",
        "enter/space:",
        "click",
        "type:",
        "input",
        "q:",
        "quit",
    ];
    assert!(
        want.iter().all(|w| words.iter().any(|s| s == *w)),
        "the hint bar lost words at 40 columns: {words:?}"
    );
    assert!(
        ls.iter().all(|l| l.plain.chars().count() <= 40),
        "a line overruns the 40-column buffer: {:?}",
        ls.iter().map(|l| l.plain.as_str()).collect::<Vec<_>>()
    );

    let default = lines(&dump(&path, &[], None, None));
    assert!(
        default.iter().any(|l| l.plain.chars().count() > 40),
        "the default buffer should be wider than 40 columns"
    );
}

#[test]
fn stopwatch_dump() {
    let _guard = RUST_BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    build_component("demo-stopwatch");
    assert_stopwatch_frame(&component_path("demo-stopwatch"), "rust");
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
    assert_stopwatch_frame(&component_path_as("demo-stopwatch"), "as");
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
    assert_stopwatch_frame(&component_path_wat("demo-stopwatch"), "wat");
}

#[test]
fn todo_dump() {
    let _guard = RUST_BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    build_component("demo-todo");
    assert_todo_frame(&component_path("demo-todo"), "rust");
}

#[test]
fn todo_dump_as() {
    if !as_toolchain_available() {
        eprintln!("skipping: AssemblyScript toolchain not installed (run `npm install`)");
        return;
    }
    let _guard = AS_BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    build_component_as("demo-todo");
    assert_todo_frame(&component_path_as("demo-todo"), "as");
}

#[test]
fn todo_dump_wat() {
    if !wat_toolchain_available() {
        eprintln!("skipping: wasm-tools not on the PATH");
        return;
    }
    let _guard = WAT_BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    build_component_wat("demo-todo");
    assert_todo_frame(&component_path_wat("demo-todo"), "wat");
}
