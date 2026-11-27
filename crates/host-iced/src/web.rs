//! The browser iced host: the same renderer and panels as the native
//! binary (the crate's library), drawn to a canvas through iced's Web
//! runtime (winit on the DOM, the WebGPU backend live). The component is
//! precompiled natively for pulley32 (the `host-precompile` tool) and
//! deserialized here to run on the Pulley interpreter — Cranelift JIT is
//! unavailable in wasm (no executable memory), so the browser never
//! compiles: it only interprets.
//!
//! The page owns the body: iced's Web runtime appends its canvas to it
//! (the canvas fills its parent, so the page sizes the body). `run`
//! deserializes the component, builds the host state, and starts the
//! application; iced's event loop then runs on the page's animation
//! frames. `dump` is the headless byte-parity surface: one frame through
//! the software `tiny-skia` renderer, independent of the live canvas, so
//! the Node selftest can compare it against the native `host-iced --dump`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use wasm_bindgen::prelude::*;
use wasmtime::component::*;
use wasmtime::Engine;

use host_shared::{pulley_config, EntrySpec};

use crate::{
    apply_pending, canvas_size, load_symbol_font, new_state, render_png, subscription, theme,
    update, view, Injected, State,
};

// A web page is single-threaded, so a thread-local is a plain global. It
// sidesteps the `Send`/`Sync` bounds a `static OnceLock<Mutex<_>>` would
// require of the wasmtime `Store`/`App` — and iced's boot takes no
// parameters, so the pre-built state waits here for it.
thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    /// The live host's component map, as a handle the page's
    /// `components-add` export writes through (the state — and the map's
    /// other arc — move into iced's event loop at boot, so the export
    /// cannot reach it there).
    static COMPONENTS: RefCell<Option<Arc<Mutex<std::collections::BTreeMap<String, Vec<u8>>>>>> =
        const { RefCell::new(None) };
}

/// iced's boot takes no parameters; the pre-built state (the component the
/// page's `run` deserialized) is handed to it here.
fn boot() -> State {
    STATE
        .with(|s| s.borrow_mut().take())
        .expect("the host state was not built; call run first")
}

/// Apply the page's registry (the desktop): the manifest's entries, the icon
/// bytes, and the entry components' bytes. All are `None` for the
/// non-desktop demos, which carry no registry, no images, and no entries to
/// open.
fn apply_registry(
    manifest: Option<String>,
    assets: Option<String>,
    components: Option<String>,
) -> Result<
    (
        Vec<EntrySpec>,
        BTreeMap<String, Vec<u8>>,
        Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    ),
    JsValue,
> {
    let entries = match manifest {
        Some(text) => host_shared::parse_manifest("the page's manifest", &text)
            .map_err(|e| JsValue::from_str(&e))?,
        None => Vec::new(),
    };
    let assets = match assets {
        Some(json) => host_shared::decode_assets(&json).map_err(|e| JsValue::from_str(&e))?,
        None => BTreeMap::new(),
    };
    let components = match components {
        Some(json) => host_shared::decode_components(&json).map_err(|e| JsValue::from_str(&e))?,
        None => BTreeMap::new(),
    };
    Ok((entries, assets, Arc::new(Mutex::new(components))))
}

/// Start the host: deserialize the precompiled component, build the state,
/// and run the application. The event loop runs on the page's animation
/// frames and the canvas is appended to the body, so this returns once the
/// loop is spawned, not when the application exits. `bytes` is the
/// artifact the `host-precompile` tool wrote. `manifest` (the desktop's
/// registry JSON), `assets` (the icons, base64), and `components` (the
/// entry artifacts, base64, keyed by the manifest's `path`) are `None` for
/// the non-desktop demos; the desktop passes `null` for `components` and
/// adds them with [`components_add`] as its background fetches resolve, so
/// the grid's first frame never waits for them.
#[wasm_bindgen]
pub fn run(
    bytes: &[u8],
    manifest: Option<String>,
    assets: Option<String>,
    components: Option<String>,
) -> Result<(), JsValue> {
    let engine =
        Engine::new(&pulley_config()).map_err(|e| JsValue::from_str(&format!("engine: {e}")))?;
    let component = unsafe { Component::deserialize(&engine, bytes) }
        .map_err(|e| JsValue::from_str(&format!("deserialize: {e}")))?;
    let (entries, assets, components) = apply_registry(manifest, assets, components)?;
    let has_registry = !entries.is_empty();
    let state = new_state(
        &engine,
        &component,
        entries,
        Vec::new(),
        None,
        assets,
        components.clone(),
    );
    // The page's `components-add` handle on the live host's component map
    // (the state's arc moves into iced's event loop; this is the clone).
    COMPONENTS.with(|c| *c.borrow_mut() = Some(components));
    STATE.with(|s| *s.borrow_mut() = Some(state));

    // The browser loads no system fonts, so the bundled Fira Sans is the only
    // text font and lacks the todo's ○, ✓, and ✕. Load the symbol font before
    // the first frame shapes text; the guard dedupes if a `dump` follows.
    load_symbol_font();

    iced::application(boot, update, view)
        .theme(theme::theme())
        .style(|_, _| theme::style())
        .title("waui")
        .window_size(canvas_size(has_registry))
        .subscription(subscription)
        .run()
        .map_err(|e| JsValue::from_str(&format!("run: {e}")))
}

/// The headless byte-parity surface: a fresh host (so the fake-clock call
/// sequence matches the native `--dump`), the component deserialized from
/// the precompiled artifact, the clicks and double-clicks applied, and one
/// frame rendered through the software `tiny-skia` renderer to PNG bytes.
/// `now_base` and `now_step` fake the host clock (the n-th `now` call
/// returns `now_base + n * now_step`); `clicks` and `double_clicks` are
/// widget ids, in order (a double-clicked tile opens its entry);
/// `components` is the entry artifacts' base64 map, for a desktop that
/// opens one.
#[wasm_bindgen]
pub fn dump(
    bytes: &[u8],
    now_base: Option<u64>,
    now_step: Option<u64>,
    clicks: &[u32],
    double_clicks: &[u32],
    manifest: Option<String>,
    assets: Option<String>,
    components: Option<String>,
) -> Result<Vec<u8>, JsValue> {
    let engine =
        Engine::new(&pulley_config()).map_err(|e| JsValue::from_str(&format!("engine: {e}")))?;
    let component = unsafe { Component::deserialize(&engine, bytes) }
        .map_err(|e| JsValue::from_str(&format!("deserialize: {e}")))?;

    let fake_now = match (now_base, now_step) {
        (Some(base), Some(step)) => Some((base, step)),
        _ => None,
    };
    let (entries, assets, components) = apply_registry(manifest, assets, components)?;
    let injected = clicks
        .iter()
        .map(|&id| Injected::Click(id))
        .chain(double_clicks.iter().map(|&id| Injected::DoubleClick(id)))
        .collect();
    let mut state = new_state(
        &engine, &component, entries, injected, fake_now, assets, components,
    );
    apply_pending(&mut state);

    let (_, _, png) = render_png(&state);
    Ok(png)
}

/// One entry component's bytes, from the page's background fetch: `path` is
/// the manifest's `path` (what `open-entry` looks up), `b64` the
/// base64-encoded precompiled artifact. The live host's component map grows
/// as the fetches resolve; a double-click that lands before its fetch does
/// fails as a missing file does natively.
#[wasm_bindgen]
pub fn components_add(path: &str, b64: &str) -> Result<(), JsValue> {
    let bytes = host_shared::decode_b64("components", path, b64)?;
    let map = COMPONENTS
        .with(|c| c.borrow().clone())
        .ok_or_else(|| JsValue::from_str("components-add: run first"))?;
    map.lock().unwrap().insert(path.to_string(), bytes);
    Ok(())
}
