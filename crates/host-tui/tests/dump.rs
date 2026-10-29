//! Headless check of the TUI host's frame rendering: run the host binary
//! with `--dump` (one frame into ratatui's `TestBackend`, printed as text
//! with ANSI SGR codes) and assert on the rendered frame — the layout, the
//! semantic colors, and the host's focus ring.
//!
//! Each scenario runs against both producer languages: the Rust component
//! (cargo + wit-bindgen) and its AssemblyScript one (asc + the hand-rolled
//! ABI). The assertions are shared.

use std::path::{Path, PathBuf};
use std::process::Command;

fn waui_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn component_path(demo: &str) -> PathBuf {
    waui_root()
        .join("target/components")
        .join(format!("{demo}.component.wasm"))
}

fn component_path_as(demo: &str) -> PathBuf {
    waui_root()
        .join("target/components")
        .join(format!("{demo}-as.component.wasm"))
}

/// Build the component and lift it, so the test is self-contained. Cargo
/// names the artifact with `_` where the crate name has `-`.
fn build_component(demo: &str) {
    let root = waui_root();
    let wasm = root
        .join("target/wasm32-unknown-unknown/release")
        .join(format!("{}{}", demo.replace('-', "_"), ".wasm"));
    let out = component_path(demo);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("failed to create target/components");
    }

    let status = Command::new("cargo")
        .args([
            "build",
            "-p",
            demo,
            "--release",
            "--target",
            "wasm32-unknown-unknown",
        ])
        .current_dir(&root)
        .status()
        .expect("failed to run cargo");
    assert!(status.success(), "failed to build {demo}");

    let status = Command::new("wasm-tools")
        .args([
            "component",
            "new",
            wasm.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
        ])
        .current_dir(&root)
        .status()
        .expect("failed to run wasm-tools");
    assert!(status.success(), "failed to lift {demo} into a component");
}

// The AssemblyScript build needs node + the local npm toolchain (asc);
// without it the AS tests skip rather than fail.
fn as_toolchain_available() -> bool {
    waui_root().join("node_modules/.bin/asc").exists()
        && Command::new("node")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
}

// The tests share their built artifacts on disk, and run on parallel
// threads: the build *and* the load window must be serialized per artifact
// file, so that one test's lift cannot clobber the file while a sibling
// test is reading it.
static RUST_BUILD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
static AS_BUILD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Build the AssemblyScript component and lift it:
/// asc (the incremental runtime) -> the `env.abort` patch -> export rename
/// -> component embed (the WIT) -> component new.
/// (Callers hold `AS_BUILD_LOCK` across the build *and* the load, so a
/// sibling test cannot lift over the file while we read it.)
fn build_component_as(demo: &str) {
    let root = waui_root();
    std::fs::create_dir_all(root.join("target/assembly"))
        .expect("failed to create target/assembly");
    std::fs::create_dir_all(root.join("target/components"))
        .expect("failed to create target/components");

    let src = format!("assembly/{demo}/index.ts");
    let raw = root
        .join("target/assembly")
        .join(format!("{demo}.raw.wasm"));
    let core = root
        .join("target/assembly")
        .join(format!("{demo}.core.wasm"));
    let renamed = root.join("target/assembly").join(format!("{demo}-as.wasm"));
    let embedded = root
        .join("target/assembly")
        .join(format!("{demo}-as.embedded.wasm"));
    let out = component_path_as(demo);

    let mut steps: Vec<Command> = Vec::new();
    {
        let mut c = Command::new("npx");
        c.args(["asc", src.as_str(), "--outFile", raw.to_str().unwrap()]);
        c.args([
            "-O",
            "--noAssert",
            "--runtime",
            "incremental",
            "--memoryBase",
            "65536",
            "--initialMemory",
            "2",
        ]);
        steps.push(c);
    }
    {
        // The incremental runtime's one import (`env.abort`, its OOM path)
        // is replaced with an in-module `unreachable` so the lifted
        // component is import-free again (see assembly/patch-env-abort.mjs).
        let mut c = Command::new("node");
        c.args([
            "assembly/patch-env-abort.mjs",
            raw.to_str().unwrap(),
            core.to_str().unwrap(),
        ]);
        steps.push(c);
    }
    {
        let mut c = Command::new("node");
        c.args([
            "assembly/rename-exports.mjs",
            core.to_str().unwrap(),
            renamed.to_str().unwrap(),
            "onclick=on-click",
            "ontick=on-tick",
        ]);
        steps.push(c);
    }
    {
        let mut c = Command::new("wasm-tools");
        c.args([
            "component",
            "embed",
            "wit/",
            renamed.to_str().unwrap(),
            "--output",
            embedded.to_str().unwrap(),
        ]);
        steps.push(c);
    }
    {
        let mut c = Command::new("wasm-tools");
        c.args([
            "component",
            "new",
            embedded.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
        ]);
        steps.push(c);
    }
    for step in &mut steps {
        step.current_dir(&root);
        let status = step.status().expect("failed to run a build tool");
        assert!(
            status.success(),
            "failed to build the AssemblyScript {demo}"
        );
    }
}

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
