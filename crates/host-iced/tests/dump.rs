//! Headless check of the iced host's render: run the host binary with
//! `--dump` (one frame through iced's software `tiny-skia` renderer,
//! written as a PNG) and assert on the pixels — the theme's background,
//! the button chrome, the semantic colors, and the centered layout.
//!
//! The render is headless (no window, display, or GPU), so it runs in CI.
//! Text renders with the bundled Fira Sans, so the pixels are stable on a
//! machine with no system fonts.
//!
//! Each scenario runs against both producer languages: the Rust component
//! and its AssemblyScript one. The counter's content is deterministic, so
//! the two producers must render byte-identical PNGs; the stopwatch's
//! readout is wall-clock time, so only its colors are asserted.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use waui_test::{
    as_toolchain_available, build_component, build_component_as, component_path, component_path_as,
    AS_BUILD_LOCK, RUST_BUILD_LOCK,
};

// The dump file: unique per call, so parallel tests do not collide.
static DUMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Run the host binary in `--dump` mode and read the PNG back as RGBA
/// pixels. `clicks` are injected before the frame, like a pointer would
/// drive them.
fn dump(path: &Path, clicks: &[u32]) -> (u32, u32, Vec<u8>) {
    let bin = env!("CARGO_BIN_EXE_host-iced");
    let file = std::env::temp_dir().join(format!(
        "waui-host-iced-dump-{}-{}.png",
        std::process::id(),
        DUMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let mut cmd = Command::new(bin);
    cmd.arg("--dump").arg(&file);
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

    // Decode the PNG back into raw RGBA pixels (one row-major frame).
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

/// The flat, non-antialiased regions: the theme's background and the
/// button's fill. The fill's centroid must sit on the canvas's center
/// line — the column is centered, and a button is sized to its label.
fn assert_chrome(width: u32, height: u32, buf: &[u8]) {
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

    // The button fill: present, and centered on the canvas.
    let fill = count_near(buf, width, height, BUTTON_FILL, 8);
    assert!(
        fill > 100,
        "the button fill is missing or too small: {fill} px"
    );
    let cx = centroid_x(buf, width, height, BUTTON_FILL, 8).expect("no button fill pixels");
    assert!(
        (cx - width as f32 / 2.0).abs() <= 10.0,
        "the buttons are not centered: centroid x {cx}"
    );
}

/// The counter: the readout's color follows the value's sign, and the
/// button labels carry their semantic colors.
fn assert_counter(width: u32, height: u32, buf: &[u8], value: i32) {
    assert_chrome(width, height, buf);

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

#[test]
fn counter_dump() {
    let _guard = RUST_BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    build_component("demo-counter");
    let path = component_path("demo-counter");

    let (w, h, buf) = dump(&path, &[]);
    assert_counter(w, h, &buf, 0);
    let fresh_negative = count_near(&buf, w, h, NEGATIVE, 24);

    let (w, h, buf) = dump(&path, &[2, 2, 4]); // + + - = 1
    assert_counter(w, h, &buf, 1);

    let (w, h, buf) = dump(&path, &[4, 4, 4]); // - - - = -3
    assert_counter(w, h, &buf, -3);
    // The readout joined the - label in red: the red pixels grow.
    let minus3_negative = count_near(&buf, w, h, NEGATIVE, 24);
    assert!(
        minus3_negative > fresh_negative,
        "the negative readout should add red pixels: {fresh_negative} -> {minus3_negative}"
    );
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
    let path = component_path_as("demo-counter");

    let rust = {
        let _g = RUST_BUILD_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        build_component("demo-counter");
        let (w, h, buf) = dump(&component_path("demo-counter"), &[]);
        assert_counter(w, h, &buf, 0);
        buf
    };
    let (w, h, buf) = dump(&path, &[]);
    assert_counter(w, h, &buf, 0);
    assert_eq!(
        rust, buf,
        "the Rust and AssemblyScript counters must render identically"
    );
}

#[test]
fn stopwatch_dump() {
    let _guard = RUST_BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    build_component("demo-stopwatch");
    let path = component_path("demo-stopwatch");

    // Paused: the readout is neutral, the button reads start (positive).
    let (w, h, buf) = dump(&path, &[]);
    assert_chrome(w, h, &buf);
    let fresh = (
        count_near(&buf, w, h, NEUTRAL, 24),
        count_near(&buf, w, h, POSITIVE, 24),
        count_near(&buf, w, h, ACCENT, 24),
    );
    assert!(fresh.0 > 0, "a paused readout should be neutral");
    assert!(fresh.1 > 0, "the start button should be positive");
    assert_eq!(fresh.2, 0, "a paused readout must not be accent");

    // Started: the readout takes the accent color, the button reads pause
    // (warning). The value is wall-clock time, so only its colors are
    // asserted.
    let (w, h, buf) = dump(&path, &[3]);
    assert_chrome(w, h, &buf);
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
    let path = component_path_as("demo-stopwatch");

    let (w, h, buf) = dump(&path, &[]);
    assert_chrome(w, h, &buf);
    assert!(
        count_near(&buf, w, h, NEUTRAL, 24) > 0,
        "a paused readout should be neutral"
    );
    assert!(
        count_near(&buf, w, h, POSITIVE, 24) > 0,
        "the start button should be positive"
    );

    let (w, h, buf) = dump(&path, &[3]);
    assert_chrome(w, h, &buf);
    assert!(
        count_near(&buf, w, h, ACCENT, 24) > 0,
        "a running readout should be accent"
    );
    assert!(
        count_near(&buf, w, h, WARNING, 24) > 0,
        "the pause button should be warning"
    );
}
