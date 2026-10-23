//! The component-side widget tree: a nested [`Tree`] that authors write and
//! [`lower`], which projects it into the contract's flat, pre-ordered node
//! list — the mirror of `host-shared`'s `build_tree` on the other side of
//! the ABI.
//!
//! The WIT contract has no recursive types, so the tree crosses the ABI as
//! a flat list; hand-assigning parent ids and pre-order is what authors
//! would otherwise do.
//!
//! This crate is dependency-free: its token enums ([`Color`], [`Size`],
//! [`Direction`]) are plain Rust mirrors of the WIT contract's, and
//! [`lower`] is generic over the output type. Each `wit-bindgen generate!`
//! is crate-local, so a component passes a projection that builds *its own*
//! generated `Node` — written once per component.
//!
//! **Identity.** A node's `id` (optional, mirroring the WIT) is its handle:
//! what `on-click` targets and what the `changes` diff channel keys on. A
//! node that is never clicked, diffed, or removed may omit its id. But a
//! node must *have* one if it has children (its children name it in their
//! `parent` field) — asserted in debug builds.
//!
//! **Diff.** The `changes` channel is owned by the layer, not the component:
//! the last export this component made is retained ([`note_view`] records a
//! full snapshot, [`diff`] records a diff), and a node is reported `changed`
//! when its record — content, style, direction, or parent — differs from it,
//! or when the host has never seen its id (bootstrap: before the first
//! export, every node is `changed`); an id that was exported and is no
//! longer in the tree is `removed`. A demo's handlers therefore mutate state
//! and nothing else, and `changes` is one [`diff`] call.

use std::sync::Mutex;

/// A semantic color, mirroring `waui:ui/widget`'s `color` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Neutral,
    Accent,
    Positive,
    Negative,
    Warning,
}

/// A semantic size, mirroring `waui:ui/widget`'s `size` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Small,
    Medium,
    Large,
}

/// Per-node styling: a color and a size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub color: Color,
    pub size: Size,
}

/// A container's orientation, mirroring `waui:ui/widget`'s `direction`
/// enum (same case order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Row,
    Column,
}

impl Style {
    pub fn new(color: Color, size: Size) -> Self {
        Self { color, size }
    }
}

/// A piece of text.
#[derive(Debug, Clone)]
pub struct Text {
    /// This node's id (see the crate docs, *Identity*).
    pub id: Option<u32>,
    pub content: String,
    pub style: Style,
}

/// A button.
#[derive(Debug, Clone)]
pub struct Button {
    /// This node's id (see the crate docs, *Identity*).
    pub id: Option<u32>,
    pub label: String,
    pub style: Style,
}

/// A horizontal or vertical container.
#[derive(Debug, Clone)]
pub struct Container {
    /// This node's id (see the crate docs, *Identity*).
    pub id: Option<u32>,
    pub direction: Direction,
    pub children: Vec<Tree>,
}

/// A nested widget tree — what a component *author* writes.
///
/// Nodes are immutable: state-dependent values are re-derived into a fresh
/// tree on every export, and the diff channel detects what changed by
/// comparing (see the crate docs, *Diff*).
#[derive(Debug, Clone)]
pub enum Tree {
    Text(Text),
    Button(Button),
    Container(Container),
}

impl Tree {
    /// A text node (id-less until [`id`](Self::id) is called).
    pub fn text(content: impl Into<String>, style: Style) -> Self {
        Self::Text(Text {
            id: None,
            content: content.into(),
            style,
        })
    }

    /// A button (id-less until [`id`](Self::id) is called).
    pub fn button(label: impl Into<String>, style: Style) -> Self {
        Self::Button(Button {
            id: None,
            label: label.into(),
            style,
        })
    }

    /// A horizontal container (id-less until [`id`](Self::id) is called;
    /// required if it has children).
    pub fn row(children: Vec<Tree>) -> Self {
        Self::Container(Container {
            id: None,
            direction: Direction::Row,
            children,
        })
    }

    /// A vertical container (id-less until [`id`](Self::id) is called;
    /// required if it has children).
    pub fn column(children: Vec<Tree>) -> Self {
        Self::Container(Container {
            id: None,
            direction: Direction::Column,
            children,
        })
    }

    /// Give this node its id.
    pub fn id(mut self, id: u32) -> Self {
        self.set_id(id);
        self
    }

    /// Replace this node's style.
    pub fn style(mut self, style: Style) -> Self {
        self.set_style(style);
        self
    }

    /// Set this node's id (in place).
    pub fn set_id(&mut self, id: u32) {
        match self {
            Self::Text(t) => t.id = Some(id),
            Self::Button(b) => b.id = Some(id),
            Self::Container(c) => c.id = Some(id),
        }
    }

    /// Replace this node's style (in place). Containers have none.
    pub fn set_style(&mut self, style: Style) {
        match self {
            Self::Text(t) => t.style = style,
            Self::Button(b) => b.style = style,
            Self::Container(_) => {}
        }
    }

    /// This node's id, whichever variant (`None` if id-less). The bare name
    /// `id` is taken by the consuming builder.
    pub fn node_id(&self) -> Option<u32> {
        match self {
            Self::Text(t) => t.id,
            Self::Button(b) => b.id,
            Self::Container(c) => c.id,
        }
    }
}

/// A flat node — a mirror of the WIT contract's `node` variant, with the
/// ids and parent links the lowering computes. Useful for tests and for
/// components that don't run `wit-bindgen`; components targeting the `app`
/// world typically project onto their generated types instead (see
/// [`lower`]).
///
/// Also what the diff channel compares: a node's record is exactly its flat
/// form (a container carries no child data, so a container is "changed"
/// only when its own id, parent, or direction is).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flat {
    Text {
        id: Option<u32>,
        parent: Option<u32>,
        content: String,
        style: Style,
    },
    Button {
        id: Option<u32>,
        parent: Option<u32>,
        label: String,
        style: Style,
    },
    Container {
        id: Option<u32>,
        parent: Option<u32>,
        direction: Direction,
    },
}

impl Flat {
    /// This node's id, whichever variant (`None` if id-less).
    pub fn id(&self) -> Option<u32> {
        match self {
            Flat::Text { id, .. } | Flat::Button { id, .. } => *id,
            Flat::Container { id, .. } => *id,
        }
    }
}

/// One entry of the diff channel: what [`diff`] decides to report.
///
/// `Changed` is a node (with the parent the lowering computed for it) whose
/// record differs from the last export — or one the host has never seen.
/// `Removed` is an id the host retains that is no longer in the tree.
pub enum DiffNode<'a> {
    Changed(&'a Tree, Option<u32>),
    Removed(u32),
}

/// What this component last exported: the tree flattened, one [`Flat`] per
/// node in depth-first pre-order, or `None` before the first export.
///
/// The diff channel's state lives here rather than in the component, so a
/// demo's handlers mutate state and nothing else. The retained list is one
/// tree's worth of buffers, replaced on every export.
static LAST_EXPORTED: Mutex<Option<Vec<Flat>>> = Mutex::new(None);

/// Lower a nested tree into the contract's flat, pre-ordered node list.
///
/// `project` is called once per node, in depth-first pre-order, with the
/// node itself and the `parent` id the lowering computed for it, and returns
/// the caller's node type (typically the component's `wit-bindgen`-generated
/// `Node`). See the crate docs for why the projection is the caller's.
pub fn lower<N, F>(tree: &Tree, mut project: F) -> Vec<N>
where
    F: FnMut(&Tree, Option<u32>) -> N,
{
    let mut out = Vec::new();
    lower_into(tree, None, &mut project, &mut out);
    out
}

/// Lower several root trees (a contract that exports multiple roots).
pub fn lower_all<N, F>(trees: &[Tree], mut project: F) -> Vec<N>
where
    F: FnMut(&Tree, Option<u32>) -> N,
{
    let mut out = Vec::new();
    for tree in trees {
        lower_into(tree, None, &mut project, &mut out);
    }
    out
}

fn lower_into<N, F>(tree: &Tree, parent: Option<u32>, project: &mut F, out: &mut Vec<N>)
where
    F: FnMut(&Tree, Option<u32>) -> N,
{
    out.push(project(tree, parent));
    if let Tree::Container(c) = tree {
        debug_assert!(
            c.id.is_some() || c.children.is_empty(),
            "a container with children must have an id: its children name \
             it in their `parent` field, and an id-less parent link is lost"
        );
        for child in &c.children {
            lower_into(child, c.id, project, out);
        }
    }
}

// ------------------------------------------------------------------ the diff

/// One node with the parent the lowering computed for it — the diff's unit
/// of comparison.
fn flat(node: &Tree, parent: Option<u32>) -> Flat {
    match node {
        Tree::Text(t) => Flat::Text {
            id: t.id,
            parent,
            content: t.content.clone(),
            style: t.style,
        },
        Tree::Button(b) => Flat::Button {
            id: b.id,
            parent,
            label: b.label.clone(),
            style: b.style,
        },
        Tree::Container(c) => Flat::Container {
            id: c.id,
            parent,
            direction: c.direction,
        },
    }
}

/// A tree as a flat list (depth-first pre-order), for the diff channel.
pub fn flat_list(tree: &Tree) -> Vec<Flat> {
    lower(tree, flat)
}

/// The diff channel for a tree against an explicit reference — the
/// component-facing [`diff`] holds that reference in [`LAST_EXPORTED`].
///
/// A node with an id is reported [`DiffNode::Changed`] when its record —
/// content, style, direction, or parent — differs from the reference, or
/// when the reference is `None` (bootstrap: the host has seen nothing, so
/// every node is new to it). Ids in the reference that are no longer in the
/// tree are reported [`DiffNode::Removed`], after the `changed` nodes, in
/// the reference's pre-order. Id-less nodes are unaddressable (the WIT's
/// identity rule) and are never reported. A sibling re-order under an
/// unchanged parent is not expressible in place — a `changed` node is
/// replaced where it stands; the WIT's answer is to remove the node and
/// re-add it.
///
/// Returns the mutation list and the tree flattened, ready to be the next
/// reference.
pub fn diff_with<M, F>(tree: &Tree, last: Option<&Vec<Flat>>, mut f: F) -> (Vec<M>, Vec<Flat>)
where
    F: FnMut(DiffNode<'_>) -> M,
{
    let new = flat_list(tree);
    let mut out = Vec::new();
    diff_node_against(tree, None, last, &mut f, &mut out);
    if let Some(last) = last {
        for ref_node in last {
            let Some(id) = ref_node.id() else { continue };
            let still_present = new.iter().any(|n| n.id() == Some(id));
            if !still_present {
                out.push(f(DiffNode::Removed(id)));
            }
        }
    }
    (out, new)
}

/// The new tree, in pre-order: report a node whose record differs from the
/// reference (or which the reference predates entirely).
fn diff_node_against<'a, M, F>(
    node: &'a Tree,
    parent: Option<u32>,
    last: Option<&Vec<Flat>>,
    f: &mut F,
    out: &mut Vec<M>,
) where
    F: FnMut(DiffNode<'a>) -> M,
{
    if let Some(id) = node.node_id() {
        let changed = match last {
            None => true, // bootstrap
            Some(last) => !last
                .iter()
                .any(|ref_node| ref_node.id() == Some(id) && ref_node == &flat(node, parent)),
        };
        if changed {
            out.push(f(DiffNode::Changed(node, parent)));
        }
    }
    if let Tree::Container(c) = node {
        for child in &c.children {
            diff_node_against(child, node.node_id(), last, f, out);
        }
    }
}

/// The diff channel: what `tree` reports to a host that retains this
/// component's last export, as the caller's mutation type behind `f` (see
/// [`diff_with`] for what is reported and in what order).
///
/// The tree just diffed becomes the reference for the next call; a full
/// snapshot leaves nothing pending, so the demo's `view` calls
/// [`note_view`] for the same reason.
pub fn diff<M, F>(tree: &Tree, f: F) -> Vec<M>
where
    F: FnMut(DiffNode<'_>) -> M,
{
    let last = LAST_EXPORTED.lock().unwrap().take();
    let (out, new) = diff_with(tree, last.as_ref(), f);
    *LAST_EXPORTED.lock().unwrap() = Some(new);
    out
}

/// Record a full snapshot as the reference for the next [`diff`]. A host
/// that just received `view()` retains the whole tree, so a diff after it
/// sees nothing pending; the demo's `view` calls this.
pub fn note_view(tree: &Tree) {
    *LAST_EXPORTED.lock().unwrap() = Some(flat_list(tree));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(c: Color, s: Size) -> Style {
        Style::new(c, s)
    }

    fn to_flat(tree: &Tree, parent: Option<u32>) -> Flat {
        match tree {
            Tree::Text(t) => Flat::Text {
                id: t.id,
                parent,
                content: t.content.clone(),
                style: t.style,
            },
            Tree::Button(b) => Flat::Button {
                id: b.id,
                parent,
                label: b.label.clone(),
                style: b.style,
            },
            Tree::Container(c) => Flat::Container {
                id: c.id,
                parent,
                direction: c.direction,
            },
        }
    }

    fn ids(flat: &[Flat]) -> Vec<(Option<u32>, Option<u32>)> {
        flat.iter()
            .map(|f| match f {
                Flat::Text { id, parent, .. }
                | Flat::Button { id, parent, .. } => (*id, *parent),
                Flat::Container { id, parent, .. } => (*id, *parent),
            })
            .collect()
    }

    #[test]
    fn lowers_in_preorder_with_parent_links() {
        let tree = Tree::column(vec![
            Tree::button("+", style(Color::Positive, Size::Medium)).id(2),
            Tree::text("0", style(Color::Neutral, Size::Large)).id(3),
            Tree::button("-", style(Color::Negative, Size::Medium)).id(4),
        ])
        .id(1);
        let flat = lower(&tree, to_flat);
        // Depth-first pre-order: the parent first, then its children in
        // visual order; every child names its parent.
        assert_eq!(ids(&flat), vec![(Some(1), None), (Some(2), Some(1)), (Some(3), Some(1)), (Some(4), Some(1))]);
        assert_eq!(flat.len(), 4);
        match &flat[1] {
            Flat::Button { label, .. } => assert_eq!(label, "+"),
            _ => panic!("expected the first child to be the + button"),
        }
    }

    #[test]
    fn nested_containers_link_their_parents() {
        let tree = Tree::column(vec![
            Tree::text("a", style(Color::Neutral, Size::Small)).id(3),
            Tree::row(vec![Tree::button("b", style(Color::Accent, Size::Small)).id(5)])
                .id(4),
        ])
        .id(1);
        assert_eq!(
            ids(&lower(&tree, to_flat)),
            vec![(Some(1), None), (Some(3), Some(1)), (Some(4), Some(1)), (Some(5), Some(4))]
        );
    }

    #[test]
    fn multiple_roots_are_all_lowered() {
        let trees = vec![
            Tree::text("one", style(Color::Neutral, Size::Small)).id(1),
            Tree::column(vec![Tree::text("two", style(Color::Neutral, Size::Small)).id(3)]).id(2),
        ];
        assert_eq!(
            ids(&lower_all(&trees, to_flat)),
            vec![(Some(1), None), (Some(2), None), (Some(3), Some(2))]
        );
    }

    #[test]
    fn builders_default_to_id_less() {
        // A leaf may omit its id entirely (it is unaddressable — see the
        // crate docs).
        let flat = lower_all(
            &[
                Tree::text("x", style(Color::Neutral, Size::Small)),
                Tree::button("y", style(Color::Accent, Size::Medium)),
            ],
            to_flat,
        );
        assert_eq!(ids(&flat), vec![(None, None), (None, None)]);
    }

    #[test]
    fn setters_roundtrip() {
        let tree = Tree::text("x", style(Color::Neutral, Size::Small)).id(9);
        if let Tree::Text(t) = &tree {
            assert_eq!(t.id, Some(9));
        } else {
            panic!("expected text");
        }
        let mut tree = Tree::button("y", style(Color::Neutral, Size::Small));
        tree.set_id(10);
        tree.set_style(style(Color::Warning, Size::Large));
        if let Tree::Button(b) = &tree {
            assert_eq!(b.id, Some(10));
            assert_eq!(b.style, style(Color::Warning, Size::Large));
        } else {
            panic!("expected a button");
        }
    }

    // The diff tests drive `diff_with` with an explicit reference rather
    // than the component-facing `diff`: tests run in parallel in one process
    // and share `LAST_EXPORTED`, but the core is pure.

    #[derive(Debug, PartialEq, Eq)]
    enum M {
        Changed(u32, Option<u32>), // id, parent
        Removed(u32),
    }

    fn proj(d: DiffNode<'_>) -> M {
        match d {
            DiffNode::Changed(t, p) => M::Changed(t.node_id().expect("a reported node has an id"), p),
            DiffNode::Removed(i) => M::Removed(i),
        }
    }

    /// The counter's shape: a column (1) with two buttons (2, 4) and a
    /// readout (3).
    fn counter(readout: &str, color: Color) -> Tree {
        Tree::column(vec![
            Tree::button("+", style(Color::Positive, Size::Medium)).id(2),
            Tree::text(readout, style(color, Size::Large)).id(3),
            Tree::button("-", style(Color::Negative, Size::Medium)).id(4),
        ])
        .id(1)
    }

    #[test]
    fn diff_bootstraps_with_every_node() {
        let (out, _) = diff_with(&counter("0", Color::Neutral), None, proj);
        assert_eq!(
            out,
            vec![
                M::Changed(1, None),
                M::Changed(2, Some(1)),
                M::Changed(3, Some(1)),
                M::Changed(4, Some(1)),
            ]
        );
    }

    #[test]
    fn diff_is_empty_when_unchanged() {
        let tree = counter("0", Color::Neutral);
        let last = flat_list(&tree);
        // Rebuild the same tree from "state" the way the demos do.
        let again = counter("0", Color::Neutral);
        let (out, _) = diff_with(&again, Some(&last), proj);
        assert_eq!(out, Vec::<M>::new());
    }

    #[test]
    fn diff_reports_the_changed_readout_alone() {
        let last = flat_list(&counter("0", Color::Neutral));
        let (out, _) = diff_with(&counter("1", Color::Neutral), Some(&last), proj);
        assert_eq!(out, vec![M::Changed(3, Some(1))]);
    }

    #[test]
    fn diff_reports_a_style_only_change() {
        let last = flat_list(&counter("0", Color::Neutral));
        // Crossing zero changes the readout's color with its content.
        let (out, _) = diff_with(&counter("-1", Color::Negative), Some(&last), proj);
        assert_eq!(out, vec![M::Changed(3, Some(1))]);
    }

    #[test]
    fn diff_reports_removed_nodes() {
        let last = flat_list(&counter("0", Color::Neutral));
        // A tree without the readout: the rest is unchanged, the readout is
        // gone.
        let slim = Tree::column(vec![
            Tree::button("+", style(Color::Positive, Size::Medium)).id(2),
            Tree::button("-", style(Color::Negative, Size::Medium)).id(4),
        ])
        .id(1);
        let (out, _) = diff_with(&slim, Some(&last), proj);
        assert_eq!(out, vec![M::Removed(3)]);
    }

    #[test]
    fn diff_reports_additions_and_reparenting() {
        // The reference: a text (5) directly under the box (1).
        let last = flat_list(
            &Tree::column(vec![Tree::text("a", style(Color::Neutral, Size::Small)).id(5)]).id(1),
        );
        // The new tree: a new row (6) under the box, the text re-parented
        // under it.
        let new = Tree::column(vec![Tree::row(vec![
            Tree::text("a", style(Color::Neutral, Size::Small)).id(5),
        ])
        .id(6)])
        .id(1);
        let (out, _) = diff_with(&new, Some(&last), proj);
        assert_eq!(out, vec![M::Changed(6, Some(1)), M::Changed(5, Some(6))]);
    }

    #[test]
    fn diff_never_reports_idless_nodes() {
        let last = flat_list(&Tree::column(vec![
            Tree::text("old", style(Color::Neutral, Size::Small)), // id-less
            Tree::text("keep", style(Color::Neutral, Size::Small)).id(2),
        ])
        .id(1));
        let new = Tree::column(vec![
            Tree::text("new", style(Color::Neutral, Size::Small)), // id-less, changed
            Tree::text("keep", style(Color::Neutral, Size::Small)).id(2),
        ])
        .id(1);
        let (out, _) = diff_with(&new, Some(&last), proj);
        assert_eq!(out, Vec::<M>::new());
    }
}
