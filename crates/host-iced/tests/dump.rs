//! Headless check of the iced host's render: run the host binary with
//! `--dump` (one frame through iced's software `tiny-skia` renderer,
//! written as a PNG) and assert on the pixels — the theme's background,
//! the button chrome, the semantic colors, and the centered layout.
//!
//! The render is headless (no window, display, or GPU), so it runs in CI.
//! Text renders with the bundled Fira Sans, and the rasterizer is pure
//! software, so the pixels are platform-stable and the goldens run
//! everywhere.
//!
//! Each scenario runs against the three producer languages: the Rust
//! component, its AssemblyScript one, and its hand-rolled WAT one. The
//! counter's content is deterministic, so the three producers must render
//! byte-identical PNGs; the stopwatch's readout is wall-clock time, so its
//! running state runs on a fake clock that advances (`--now <base> <step>`).
//!
//! The goldens in `tests/goldens/` are the visual regression layer: the
//! whole frame, byte-exact, shared by the three producers (the shared
//! golden is also the parity check). Regenerate with
//! `UPDATE_GOLDENS=1 cargo test -p host-iced`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use waui_test::{
    as_toolchain_available, assert_golden, build_component, build_component_as,
    build_component_wat, component_path, component_path_as, component_path_wat,
    wat_toolchain_available, AS_BUILD_LOCK, RUST_BUILD_LOCK, WAT_BUILD_LOCK,
};

// The fake clock for the stopwatch's running state: the n-th `now` call
// returns `base + n * step`. The host's `schedule` call and the readout's
// `elapsed` sample both read it, so the readout is a fixed 2 * step.
const FAKE_NOW: (u64, u64) = (1_000, 50);

// The dump file: unique per call, so parallel tests do not collide.
static DUMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Run the host binary in `--dump` mode and read the PNG back: the decoded
/// RGBA pixels (the property assertions) and the raw bytes (the golden
/// compares them). `clicks` are injected before the frame, like a pointer
/// would drive them; `now` is the fake clock's base and step (`--now`), so
/// a frame that reads the clock is deterministic.
fn dump(path: &Path, clicks: &[u32], now: Option<(u64, u64)>) -> (u32, u32, Vec<u8>, Vec<u8>) {
    let bin = env!("CARGO_BIN_EXE_host-iced");
    let file = std::env::temp_dir().join(format!(
        "waui-host-iced-dump-{}-{}.png",
        std::process::id(),
        DUMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let mut cmd = Command::new(bin);
    cmd.arg("--dump").arg(&file);
    if let Some((base, step)) = now {
        cmd.arg("--now").arg(base.to_string()).arg(step.to_string());
    }
    for id in clicks {
        cmd.arg("--click").arg(id.to_string());
    }
    cmd.arg(path);
    let out = cmd.output().expect("failed to run the host-iced binary");
    assert!(
        out.status.success(),
        "host-iced --dump failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bytes = std::fs::read(&file).expect("failed to read the dump file");
    std::fs::remove_file(&file).ok();
    let (width, height, buf) = decode_png(&bytes);
    (width, height, buf, bytes)
}

/// Decode a PNG into raw RGBA pixels (one row-major frame).
fn decode_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut decoder = png::Decoder::new(&bytes[..]);
    let (width, height) = decoder
        .read_header_info()
        .expect("the dump is not a valid PNG")
        .size();
    let mut reader = decoder.read_info().expect("the dump is not a valid PNG");
    let mut buf = vec![0u8; width as usize * height as usize * 4];
    let output = reader
        .next_frame(&mut buf)
        .expect("the dump is not a valid PNG");
    assert_eq!(
        (output.width, output.height),
        (width, height),
        "the decoded frame size"
    );
    (width, height, buf)
}

/// The pixel at (x, y) as RGBA.
fn px(buf: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let i = (y * width + x) as usize * 4;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

/// Whether a pixel is within `tol` (per channel) of `color`.
fn near(p: [u8; 4], color: [u8; 3], tol: u32) -> bool {
    (p[0] as i32 - color[0] as i32).abs() <= tol as i32
        && (p[1] as i32 - color[1] as i32).abs() <= tol as i32
        && (p[2] as i32 - color[2] as i32).abs() <= tol as i32
}

/// The number of pixels within `tol` (per channel) of `color`.
fn count_near(buf: &[u8], width: u32, height: u32, color: [u8; 3], tol: u32) -> u32 {
    let mut n = 0u32;
    for y in 0..height {
        for x in 0..width {
            if near(px(buf, width, x, y), color, tol) {
                n += 1;
            }
        }
    }
    n
}

/// The mean x of the pixels within `tol` of `color`, for a centeredness
/// check; `None` when there are none.
fn centroid_x(buf: &[u8], width: u32, height: u32, color: [u8; 3], tol: u32) -> Option<f32> {
    let mut sum = 0.0f64;
    let mut n = 0u32;
    for y in 0..height {
        for x in 0..width {
            if near(px(buf, width, x, y), color, tol) {
                sum += x as f64;
                n += 1;
            }
        }
    }
    (n > 0).then(|| (sum / n as f64) as f32)
}

// The web page's palette (web/index.html), as the host must draw it.
const BACKGROUND: [u8; 3] = [0x1a, 0x1b, 0x26];
const BUTTON_FILL: [u8; 3] = [0x24, 0x28, 0x3b];
const NEUTRAL: [u8; 3] = [0xc0, 0xca, 0xf5];
const POSITIVE: [u8; 3] = [0x9e, 0xce, 0x6a];
const NEGATIVE: [u8; 3] = [0xf7, 0x76, 0x8e];
const ACCENT: [u8; 3] = [0x7a, 0xa2, 0xf7];
const WARNING: [u8; 3] = [0xe0, 0xaf, 0x68];

// The dump canvas (host-iced's `WINDOW_WIDTH`/`WINDOW_HEIGHT`), at 1x.
const WIDTH: u32 = 360;
const HEIGHT: u32 = 340;

/// The dump canvas and the flat, non-antialiased regions: the theme's
/// background (exact at the edges) and the button fill (present).
fn assert_background(width: u32, height: u32, buf: &[u8]) {
    assert_eq!((width, height), (WIDTH, HEIGHT), "the dump canvas size");

    // The background: the corners and the edge midpoints, exact.
    for (x, y) in [
        (0u32, 0u32),
        (width - 1, 0),
        (0, height - 1),
        (width - 1, height - 1),
        (width / 2, 0),
        (width / 2, height - 1),
    ] {
        assert_eq!(
            px(buf, width, x, y),
            [BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255],
            "background at ({x}, {y})"
        );
    }

    let fill = count_near(buf, width, height, BUTTON_FILL, 8);
    assert!(
        fill > 100,
        "the button fill is missing or too small: {fill} px"
    );
}

/// The button fill's centroid sits on the canvas's center line: a demo of
/// centered content keeps its buttons there. The todo's footer input (a
/// fixed 80px field) shifts the fill off-center, so it does not take this
/// check; its layout is the golden's.
fn assert_centered(width: u32, height: u32, buf: &[u8]) {
    let cx = centroid_x(buf, width, height, BUTTON_FILL, 8).expect("no button fill pixels");
    assert!(
        (cx - width as f32 / 2.0).abs() <= 10.0,
        "the buttons are not centered: centroid x {cx}"
    );
}

/// The golden file for `name`, under this crate's `tests/goldens/`.
fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(name)
}

/// Compare the dump's PNG bytes against its golden: the shared byte-exact
/// check, with the differing pixels in the failure message.
fn assert_png_golden(bytes: &[u8], name: &str) {
    assert_golden(bytes, &golden(name), png_diff);
}

/// The differing pixels, for a failure message: the count and their
/// bounding box, or a size mismatch.
fn png_diff(want: &[u8], actual: &[u8]) -> String {
    let (ww, wh, wbuf) = decode_png(want);
    let (aw, ah, abuf) = decode_png(actual);
    if (ww, wh) != (aw, ah) {
        return format!("the frame sizes differ: golden {ww}x{wh}, actual {aw}x{ah}\n");
    }
    let mut n = 0u32;
    let (mut min_x, mut min_y) = (ww, wh);
    let (mut max_x, mut max_y) = (0u32, 0u32);
    for y in 0..wh {
        for x in 0..ww {
            let i = (y * ww + x) as usize * 4;
            if &wbuf[i..i + 4] != &abuf[i..i + 4] {
                n += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if n == 0 {
        "the PNG bytes differ, but the pixels match (the encoder)\n".to_string()
    } else {
        format!("{n} differing pixels, in the box ({min_x}, {min_y})-({max_x}, {max_y})\n")
    }
}

/// The counter: the readout's color follows the value's sign, and the
/// button labels carry their semantic colors.
fn assert_counter(width: u32, height: u32, buf: &[u8], value: i32) {
    assert_background(width, height, buf);
    assert_centered(width, height, buf);

    let neutral = count_near(buf, width, height, NEUTRAL, 24);
    let positive = count_near(buf, width, height, POSITIVE, 24);
    let negative = count_near(buf, width, height, NEGATIVE, 24);

    assert!(positive > 0, "the + label should be positive (green)");
    assert!(negative > 0, "the - label should be negative (red)");
    if value < 0 {
        assert!(
            neutral == 0,
            "a negative readout must not be neutral: {neutral} px"
        );
    } else {
        assert!(neutral > 0, "a non-negative readout should be neutral");
    }
}

/// The counter's three states, each against its golden.
fn assert_counter_frame(path: &Path) {
    let (w, h, buf, bytes) = dump(path, &[], None);
    assert_counter(w, h, &buf, 0);
    let fresh_negative = count_near(&buf, w, h, NEGATIVE, 24);
    assert_png_golden(&bytes, "counter-0.png");

    let (w, h, buf, bytes) = dump(path, &[2, 2, 4], None); // + + - = 1
    assert_counter(w, h, &buf, 1);
    assert_png_golden(&bytes, "counter-1.png");

    let (w, h, buf, bytes) = dump(path, &[4, 4, 4], None); // - - - = -3
    assert_counter(w, h, &buf, -3);
    // The readout joined the - label in red: the red pixels grow.
    let minus3_negative = count_near(&buf, w, h, NEGATIVE, 24);
    assert!(
        minus3_negative > fresh_negative,
        "the negative readout should add red pixels: {fresh_negative} -> {minus3_negative}"
    );
    assert_png_golden(&bytes, "counter--3.png");
}

/// The stopwatch's two states, each against its golden. The running state
/// runs on a fake clock that advances, so the readout is a fixed 100 ms.
fn assert_stopwatch_frame(path: &Path) {
    // Paused: the readout is neutral, the button reads start (positive).
    let (w, h, buf, bytes) = dump(path, &[], None);
    assert_background(w, h, &buf);
    assert_centered(w, h, &buf);
    let fresh = (
        count_near(&buf, w, h, NEUTRAL, 24),
        count_near(&buf, w, h, POSITIVE, 24),
        count_near(&buf, w, h, ACCENT, 24),
    );
    assert!(fresh.0 > 0, "a paused readout should be neutral");
    assert!(fresh.1 > 0, "the start button should be positive");
    assert_eq!(fresh.2, 0, "a paused readout must not be accent");
    assert_png_golden(&bytes, "stopwatch-paused.png");

    // Started: the readout takes the accent color, the button reads pause
    // (warning). The fake clock advances 50 ms per call, so the readout is
    // the fixed 100 ms.
    let (w, h, buf, bytes) = dump(path, &[3], Some(FAKE_NOW));
    assert_background(w, h, &buf);
    assert_centered(w, h, &buf);
    let (neutral, positive, accent, warning) = (
        count_near(&buf, w, h, NEUTRAL, 24),
        count_near(&buf, w, h, POSITIVE, 24),
        count_near(&buf, w, h, ACCENT, 24),
        count_near(&buf, w, h, WARNING, 24),
    );
    assert!(accent > 0, "a running readout should be accent");
    assert!(warning > 0, "the pause button should be warning");
    assert!(
        neutral < fresh.0,
        "the running readout should leave neutral: {} -> {neutral}",
        fresh.0
    );
    assert_eq!(positive, 0, "the pause button must not be positive");
    assert_png_golden(&bytes, "stopwatch-running.png");
}

/// The todo's frame: the chrome, the header's accent, and the rows'
/// semantic colors.
fn assert_todo(width: u32, height: u32, buf: &[u8]) {
    assert_background(width, height, buf);

    let accent = count_near(buf, width, height, ACCENT, 24);
    let neutral = count_near(buf, width, height, NEUTRAL, 24);
    let positive = count_near(buf, width, height, POSITIVE, 24);
    let negative = count_near(buf, width, height, NEGATIVE, 24);
    let warning = count_near(buf, width, height, WARNING, 24);

    assert!(accent > 0, "the todos header should be accent");
    assert!(neutral > 0, "the active labels should be neutral");
    assert!(
        positive > 0,
        "the done checkbox and the add button should be positive"
    );
    assert!(
        negative > 0,
        "the delete and clear buttons should be negative"
    );
    assert!(warning > 0, "the done label should be warning");
}

/// The todo's frames: the chrome, the header's accent, and the rows'
/// semantic colors, each against its golden. The added state is the add
/// button with an empty field: the next preset label, newest first, the
/// count up one.
fn assert_todo_frame(path: &Path) {
    let (w, h, buf, bytes) = dump(path, &[], None);
    assert_todo(w, h, &buf);
    assert_png_golden(&bytes, "todo.png");

    let (w, h, buf, bytes) = dump(path, &[9], None);
    assert_todo(w, h, &buf);
    assert_png_golden(&bytes, "todo-added.png");
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

#[test]
fn todo_dump() {
    let _guard = RUST_BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    build_component("demo-todo");
    assert_todo_frame(&component_path("demo-todo"));
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
    assert_todo_frame(&component_path_as("demo-todo"));
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
    assert_todo_frame(&component_path_wat("demo-todo"));
}
