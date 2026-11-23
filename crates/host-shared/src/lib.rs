//! The shared core of the wasmtime hosts: contract-level code only.
//!
//! - `bindgen!` for the `waui:ui/app` world
//! - [`build_tree`]: the rebuild of the component's flat node list into a
//!   widget tree
//! - [`Retained`]: the retained node list the host refreshes (content-only
//!   diffs applied in place, structural ones re-queried from `view`)
//!
//! The flat-list invariants these depend on (depth-first pre-order, parents
//! precede children) are documented by the WIT itself, so their reference
//! implementation lives here rather than in every host. What stays per-host
//! is the renderer — the mapping of this tree onto a toolkit — plus
//! host-local concerns (the clock and refresh-request impls, the TUI's
//! focus ring). A host that cannot express
//! part of the contract (host-cli, plain ASCII) draws less of it.
//!
//! The web host does not use this crate: it is JavaScript, and
//! `web/app.js` re-implements the same rebuild. The WIT package is the
//! cross-language artifact; this crate is the Rust hosts' shared
//! implementation of it.

wasmtime::component::bindgen!({
    world: "app",
    path: "../../wit",
});

// `Node` and `Mutation` resolve to the aliases the macro emits at this
// crate's root.
use waui::ui::widget::{
    Align, Background, Border, Container, Cursor, Direction, Overflow, Radius, Shadow, Stretch,
    Style,
};

// --------------------------------------------------------------------------
// The registry manifest (the desktop's entries).
// --------------------------------------------------------------------------

/// A registry entry: the name (`open-entry`'s key), the component's path,
/// and the `data` blob (the host-defined metadata the component parses).
#[derive(Clone, Debug)]
pub struct EntrySpec {
    pub name: String,
    pub path: String,
    pub data: String,
}

/// Parse a registry manifest (JSON text): its entries' names, paths, and
/// `data` blobs. The manifest is the single source of truth for the
/// desktop's registry; the `data` object it carries is serialized to the
/// blob string the component parses. `source` names the text in errors (the
/// file path natively, the page's fetch in the browser).
pub fn parse_manifest(source: &str, text: &str) -> Result<Vec<EntrySpec>, String> {
    #[derive(serde::Deserialize)]
    struct Manifest {
        entries: Vec<ManifestEntry>,
    }
    #[derive(serde::Deserialize)]
    struct ManifestEntry {
        name: String,
        path: String,
        data: serde_json::Value,
    }

    let manifest: Manifest =
        serde_json::from_str(text).map_err(|e| format!("bad manifest {source}: {e}"))?;
    Ok(manifest
        .entries
        .into_iter()
        .map(|e| EntrySpec {
            name: e.name,
            path: e.path,
            data: e.data.to_string(),
        })
        .collect())
}

/// Read a registry manifest (a JSON file): its entries' names, paths, and
/// `data` blobs.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_manifest(path: &str) -> Result<Vec<EntrySpec>, String> {
    use std::fs;

    let text = fs::read_to_string(path).map_err(|e| format!("failed to read {path}: {e}"))?;
    parse_manifest(path, &text)
}

/// Decode one base64 blob into bytes. `kind` and `path` name the blob in
/// errors (the registry's icons, its components). Wasm-only: the browser has
/// no filesystem, so the page hands file bytes over base64-encoded.
#[cfg(target_arch = "wasm32")]
pub fn decode_b64(kind: &str, path: &str, b64: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;

    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("bad {kind} {path}: {e}"))
}

/// Decode the page's image assets: a JSON object mapping an asset path to its
/// base64-encoded bytes. The browser has no filesystem, so the page fetches
/// the registry's icons and hands them over; the `run`/`dump` exports take
/// this text and decode it into the image state's asset map. Wasm-only: the
/// native host resolves an asset's `src` against the working directory.
#[cfg(target_arch = "wasm32")]
pub fn decode_assets(json: &str) -> Result<std::collections::BTreeMap<String, Vec<u8>>, String> {
    let obj: std::collections::BTreeMap<String, String> =
        serde_json::from_str(json).map_err(|e| format!("bad assets: {e}"))?;
    let mut assets = std::collections::BTreeMap::new();
    for (path, b64) in obj {
        assets.insert(path.clone(), decode_b64("assets", &path, &b64)?);
    }
    Ok(assets)
}

/// Decode the page's entry components: a JSON object mapping a registry
/// entry's `path` to its base64-encoded precompiled artifact. The browser has
/// no filesystem, so the page fetches the registry's artifacts and hands
/// them over; the `run`/`dump` exports take this text and decode it into the
/// host's component map, which `open-entry` loads from. Wasm-only: the
/// native host reads an entry's component from the filesystem.
#[cfg(target_arch = "wasm32")]
pub fn decode_components(
    json: &str,
) -> Result<std::collections::BTreeMap<String, Vec<u8>>, String> {
    let obj: std::collections::BTreeMap<String, String> =
        serde_json::from_str(json).map_err(|e| format!("bad components: {e}"))?;
    let mut components = std::collections::BTreeMap::new();
    for (path, b64) in obj {
        components.insert(path.clone(), decode_b64("components", &path, &b64)?);
    }
    Ok(components)
}

// --------------------------------------------------------------------------
// The browser host's engine config, shared with the precompile tool.
// --------------------------------------------------------------------------

/// The engine config for running waui components on the Pulley interpreter
/// (the browser host) and for precompiling them natively for pulley32 (the
/// build tool). The two must agree: the browser deserializes the artifact
/// the tool writes, and Wasmtime rejects an artifact whose serialized target
/// and tunables do not match the deserializing engine.
///
/// The explicit `pulley32` target skips Wasmtime's platform-dependent
/// refinement, so both sides start from the same base defaults. The
/// no-virtual-memory refinements are applied explicitly on both sides (an
/// explicit target skips the automatic refinement): the base defaults (a
/// 10 MB reservation, copy-on-write) fail at runtime on wasm32, which
/// allocates memories from the heap.
///
/// `concurrency_support` is off on both sides: the browser build lacks the
/// `component-model-async` cargo feature, so it computes the same value by
/// construction, and the artifact records it. The build tool additionally
/// disables `wasm_threads` (the `threads` feature is on by default in its
/// full native build, and the artifact records it); the browser has it off
/// by construction, so the tool sets it, not this shared config.
pub fn pulley_config() -> wasmtime::Config {
    let mut config = wasmtime::Config::default();
    config.target("pulley32").expect("pulley32 target");
    config.concurrency_support(false);
    config.memory_reservation(0);
    config.memory_reservation_for_growth(1 << 20);
    config.memory_init_cow(false);
    config.gc_heap_reservation(0);
    config.gc_heap_reservation_for_growth(1 << 20);
    config
}

// --------------------------------------------------------------------------
// The embedder half of wasmtime's custom-platform contract (wasm32).
// --------------------------------------------------------------------------
//
// On wasm32, wasmtime's runtime falls into its "custom platform" branch and
// declares `wasmtime_tls_{get,set}` as imports, expecting the embedder to
// link definitions. Wasmtime needs two thread-local slots (trap-handler
// state and component-model-async state). A web page is single-threaded, so
// a static array backs them — the same approach as wasmtime's min-platform
// example. lld refuses undefined symbols without import attributes, so these
// must be defined or the cdylib fails to link.

#[cfg(target_arch = "wasm32")]
mod tls {
    use core::sync::atomic::{AtomicPtr, Ordering};

    static SLOTS: [AtomicPtr<u8>; 2] = [const { AtomicPtr::new(core::ptr::null_mut()) }; 2];

    #[no_mangle]
    extern "C" fn wasmtime_tls_get(slot: usize) -> *mut u8 {
        SLOTS[slot].load(Ordering::Relaxed)
    }

    #[no_mangle]
    extern "C" fn wasmtime_tls_set(slot: usize, val: *mut u8) {
        SLOTS[slot].store(val, Ordering::Relaxed);
    }
}

/// A container's chrome and geometry, as the component sent it (see the
/// `container` record in `wit/widget.wit`). A host that cannot express a
/// token renders without it.
#[derive(Debug, Clone, Copy)]
pub struct ContainerStyle {
    /// `true` places the container at `(x, y)` relative to its parent.
    pub positioned: bool,
    pub x: u32,
    pub y: u32,
    /// The explicit size; `0` sizes to content.
    pub w: u32,
    pub h: u32,
    /// The stacking order; `0` = none.
    pub z: u32,
    /// The space between children; `0` = the host's default.
    pub gap: u32,
    /// The space around the children; `0` = the host's default.
    pub padding: u32,
    pub stretch: Stretch,
    pub overflow: Overflow,
    pub background: Background,
    pub border: Border,
    pub radius: Radius,
    pub shadow: Shadow,
    pub cursor: Cursor,
}

impl ContainerStyle {
    /// The token defaults a component's tree carries when it sets none of
    /// them (the `component-ui` builder's defaults).
    pub const fn default() -> Self {
        Self {
            positioned: false,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            z: 0,
            gap: 0,
            padding: 0,
            stretch: Stretch::None,
            overflow: Overflow::Clip,
            background: Background::None,
            border: Border::None,
            radius: Radius::None,
            shadow: Shadow::None,
            cursor: Cursor::Default,
        }
    }

    fn from_node(c: &Container) -> Self {
        Self {
            positioned: c.positioned != 0,
            x: c.x,
            y: c.y,
            w: c.w,
            h: c.h,
            z: c.z,
            gap: c.gap,
            padding: c.padding,
            stretch: c.stretch,
            overflow: c.overflow,
            background: c.background,
            border: c.border,
            radius: c.radius,
            shadow: c.shadow,
            cursor: c.cursor,
        }
    }
}

/// A rebuilt widget tree.
#[derive(Debug, Clone)]
pub enum Tree {
    /// (content, the component's semantic style)
    Text(String, Style),
    /// (widget id — `None` for a display-only button a component did not
    /// give an id, label, style)
    Button(Option<u32>, String, Style),
    /// (widget id — `None` for an input a component did not give an id, the
    /// input's content, the field's width in characters — `0` is the host's
    /// default, style)
    Input(Option<u32>, String, u32, Style),
    /// (widget id, the `src` path/asset name the host resolves, the width,
    /// the height — `0` is the host's default)
    Image(Option<u32>, String, u32, u32),
    /// (widget id — `None` for a container a component did not give an id,
    /// the cross-axis alignment, the chrome and geometry, the children)
    Row(Option<u32>, Align, ContainerStyle, Vec<Tree>),
    /// (widget id — `None` for a container a component did not give an id,
    /// the cross-axis alignment, the chrome and geometry, the children)
    Column(Option<u32>, Align, ContainerStyle, Vec<Tree>),
}

/// The node's id, if it has one (see the WIT identity note: a node the
/// component never addresses or diffs may omit it).
fn node_id(node: &Node) -> Option<u32> {
    match node {
        Node::Text(t) => t.id,
        Node::Button(b) => b.id,
        Node::Input(i) => i.id,
        Node::Image(i) => i.id,
        Node::Container(c) => c.id,
    }
}

fn node_parent(node: &Node) -> Option<u32> {
    match node {
        Node::Text(t) => t.parent,
        Node::Button(b) => b.parent,
        Node::Input(i) => i.parent,
        Node::Image(i) => i.parent,
        Node::Container(c) => c.parent,
    }
}

fn to_tree(node: &Node) -> Tree {
    match node {
        Node::Text(t) => Tree::Text(t.content.clone(), t.style),
        Node::Button(b) => Tree::Button(b.id, b.label.clone(), b.style),
        Node::Input(i) => Tree::Input(i.id, i.content.clone(), i.width, i.style),
        Node::Image(i) => Tree::Image(i.id, i.src.clone(), i.width, i.height),
        Node::Container(c) => {
            let style = ContainerStyle::from_node(c);
            if c.direction == Direction::Row {
                Tree::Row(c.id, c.align, style, Vec::new())
            } else {
                Tree::Column(c.id, c.align, style, Vec::new())
            }
        }
    }
}

/// Rebuild the tree from the flat node list exported by `view()`.
///
/// The list is depth-first pre-ordered (see `wit/`), so children of each
/// node can be linked in list order.
pub fn build_tree(nodes: &[Node]) -> Tree {
    use std::collections::HashMap;

    let mut trees: Vec<Option<Tree>> = Vec::with_capacity(nodes.len());
    let mut children_of: HashMap<u32, Vec<usize>> = HashMap::new();

    for (i, node) in nodes.iter().enumerate() {
        trees.push(Some(to_tree(node)));
        if let Some(parent) = node_parent(node) {
            children_of.entry(parent).or_default().push(i);
        }
    }

    // Visit nodes in *reverse* pre-order: by the time a container is
    // visited, its children (which all come after it) have already been
    // fully linked in place, so they can be moved into it wholesale.
    for i in (0..trees.len()).rev() {
        let node = &nodes[i];
        // Children name their parent by id; an id-less node (a leaf, or a
        // malformed list) has none to look up.
        let kids = node_id(node)
            .and_then(|id| children_of.get(&id))
            .cloned()
            .unwrap_or_default();
        if kids.is_empty() {
            continue;
        }
        // Pre-order guarantees a parent precedes its children, so every
        // child index is after `i`; split the slice to borrow container and
        // children disjointly.
        let first = *kids.first().unwrap();
        if first <= i {
            // Not depth-first pre-ordered; skip rather than panic.
            continue;
        }
        let (head, tail) = trees.split_at_mut(first);
        match head[i].as_mut() {
            Some(Tree::Row(_, _, _, children) | Tree::Column(_, _, _, children)) => {
                for k in kids {
                    if let Some(child) = tail[k - first].take() {
                        children.push(child);
                    }
                }
            }
            _ => {}
        }
    }

    // The root is the first node with no parent. If a component exports
    // several, wrap them in a column; if none, render an empty column.
    let mut roots: Vec<Tree> = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        if node_parent(node).is_none() {
            if let Some(root) = trees[i].take() {
                roots.push(root);
            }
        }
    }
    match roots.len() {
        0 => Tree::Column(None, Align::Center, ContainerStyle::default(), Vec::new()),
        1 => roots.pop().unwrap(),
        _ => Tree::Column(None, Align::Center, ContainerStyle::default(), roots),
    }
}

/// The host's retained copy of the component's tree: a full snapshot from
/// `view` at startup, then content-only diffs applied by node id, and a
/// re-queried snapshot after any structural change.
///
/// The split follows the WIT identity note: a diff cannot carry id-less
/// nodes, so a structural change (which may add or move one — a window's
/// title-bar label) forces a `view` re-query, while a content-only diff
/// carries everything it changes and is applied in place.
#[derive(Debug, Clone)]
pub struct Retained {
    /// The full tree, in canonical pre-order.
    nodes: Vec<Node>,
}

impl Retained {
    /// Retain a full snapshot (the result of `view`).
    pub fn from_view(nodes: Vec<Node>) -> Self {
        Self { nodes }
    }

    /// The retained tree, in canonical pre-order (for `build_tree`).
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// Whether `mutations` is content-only: every mutation is a `changed`
    /// for an id'd leaf that keeps its position (a restyle, no structure).
    ///
    /// Such a diff carries everything it changes, so a host may apply it.
    /// A structural diff may not — it cannot carry id-less nodes (the WIT
    /// identity note) — so the host re-queries `view` instead.
    pub fn is_content_only(&self, mutations: &[Mutation]) -> bool {
        mutations.iter().all(|m| match m {
            Mutation::Changed(c) => {
                let node = &c.node;
                let Some(id) = node_id(node) else {
                    return false;
                };
                if matches!(node, Node::Container(_)) {
                    return false;
                }
                let Some(prev) = self.nodes.iter().find(|n| node_id(n) == Some(id)) else {
                    return false; // a new node: structure
                };
                node_parent(node) == node_parent(prev)
            }
            Mutation::Removed(_) => false,
        })
    }

    /// Apply a content-only diff (see [`is_content_only`]): each changed
    /// node replaces its retained copy in place.
    pub fn apply(&mut self, mutations: &[Mutation]) {
        for mutation in mutations {
            let Mutation::Changed(c) = mutation else {
                continue; // content-only: no removals
            };
            let node = &c.node;
            // A node without an id is unaddressable (see the WIT identity
            // note): a well-behaved component does not report one, and a
            // host may ignore it.
            let Some(id) = node_id(node) else {
                continue;
            };
            if let Some(i) = self.nodes.iter().position(|n| node_id(n) == Some(id)) {
                self.nodes[i] = node.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! The rebuild is pure data structure work — testable without a
    //! component in sight.

    use super::*;
    use waui::ui::widget::{
        Button, Changed, Color, Container, Cursor, Direction, Overflow, Radius, Shadow, Size,
        Stretch, Style, Text,
    };

    fn style(color: Color, size: Size) -> Style {
        Style { color, size }
    }

    fn changed(node: Node, index: u32) -> Mutation {
        Mutation::Changed(Changed { node, index })
    }

    fn cont(id: u32, parent: Option<u32>) -> Node {
        Node::Container(Container {
            id: Some(id),
            parent,
            ..container_defaults()
        })
    }

    /// The token fields a test's `Container` literal leaves at their
    /// defaults (the generated record has no `Default` impl).
    fn container_defaults() -> Container {
        Container {
            id: None,
            parent: None,
            direction: Direction::Column,
            align: Align::Center,
            positioned: 0,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            z: 0,
            gap: 0,
            padding: 0,
            stretch: Stretch::None,
            overflow: Overflow::Clip,
            background: Background::None,
            border: Border::None,
            radius: Radius::None,
            shadow: Shadow::None,
            cursor: Cursor::Default,
        }
    }

    fn text(id: u32, parent: Option<u32>, content: &str) -> Node {
        Node::Text(Text {
            id: Some(id),
            parent,
            content: content.to_string(),
            style: style(Color::Neutral, Size::Large),
        })
    }

    fn button(id: u32, parent: Option<u32>, label: &str) -> Node {
        Node::Button(Button {
            id: Some(id),
            parent,
            label: label.to_string(),
            style: style(Color::Positive, Size::Medium),
        })
    }

    #[test]
    fn content_only_is_a_changed_leaf_in_place() {
        // The counter's tick: the readout text changes, same parent.
        let r = Retained::from_view(vec![
            cont(1, None),
            text(3, Some(1), "0"),
            button(4, Some(1), "+"),
        ]);
        assert!(r.is_content_only(&[changed(text(3, Some(1), "1"), 0)]));
    }

    #[test]
    fn content_only_rejects_structure() {
        let r = Retained::from_view(vec![cont(1, None), text(3, Some(1), "0")]);
        // A container change (a restyle cannot update one in place).
        assert!(!r.is_content_only(&[changed(cont(1, None), 0)]));
        // A node the host has never retained (an addition).
        assert!(!r.is_content_only(&[changed(text(9, Some(1), "x"), 0)]));
        // A re-parent.
        let two = Retained::from_view(vec![cont(1, None), cont(2, None), text(3, Some(1), "0")]);
        assert!(!two.is_content_only(&[changed(text(3, Some(2), "0"), 0)]));
        // A removal.
        assert!(!r.is_content_only(&[Mutation::Removed(3)]));
        // An id-less node (unaddressable: the WIT identity note).
        assert!(!r.is_content_only(&[changed(
            Node::Text(Text {
                id: None,
                parent: Some(1),
                content: "x".to_string(),
                style: style(Color::Neutral, Size::Large),
            }),
            0
        )]));
    }

    #[test]
    fn apply_replaces_the_changed_leaf_in_place() {
        let mut r = Retained::from_view(vec![
            cont(1, None),
            text(3, Some(1), "0"),
            button(4, Some(1), "+"),
        ]);
        r.apply(&[changed(text(3, Some(1), "5"), 0)]);
        assert_eq!(r.nodes.len(), 3);
        match &r.nodes[1] {
            Node::Text(t) => assert_eq!(t.content, "5"),
            other => panic!("the readout should stay text: {other:?}"),
        }
    }

    #[test]
    fn rebuild_carries_a_container_align() {
        // The todo's shape: a centered root with a start-align wrapper.
        let tree = build_tree(&[
            cont(1, None),
            Node::Container(Container {
                id: Some(3),
                parent: Some(1),
                direction: Direction::Column,
                align: Align::Start,
                ..container_defaults()
            }),
            text(4, Some(3), "a"),
        ]);
        match &tree {
            Tree::Column(_, Align::Center, _, children) => match &children[0] {
                Tree::Column(_, Align::Start, _, kids) => assert_eq!(kids.len(), 1),
                _ => panic!("the wrapper should keep its start align"),
            },
            _ => panic!("expected the centered root column"),
        }
    }

    #[test]
    fn rebuild_carries_a_container_id() {
        // A host that addresses containers (its tiles) needs the id; an
        // id-less container carries None, and the synthetic roots (the
        // multi-root wrapper) carry None too.
        let tree = build_tree(&[
            cont(1, None),
            Node::Container(Container {
                id: Some(2),
                parent: Some(1),
                direction: Direction::Row,
                align: Align::Center,
                ..container_defaults()
            }),
            Node::Container(Container {
                id: None,
                parent: None,
                direction: Direction::Column,
                align: Align::Center,
                ..container_defaults()
            }),
        ]);
        match &tree {
            Tree::Column(None, _, _, roots) if roots.len() == 2 => {
                match &roots[0] {
                    Tree::Column(Some(1), _, _, children) => match &children[0] {
                        Tree::Row(Some(2), _, _, _) => {}
                        other => panic!("expected the id'd row: {other:?}"),
                    },
                    other => panic!("root 1 should keep its id: {other:?}"),
                }
                match &roots[1] {
                    Tree::Column(None, _, _, _) => {}
                    other => panic!("the id-less root should carry None: {other:?}"),
                }
            }
            other => panic!("expected the two roots in a wrapper: {other:?}"),
        }
    }
}
