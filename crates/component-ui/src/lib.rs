//! The component-side widget tree: a nested [`Tree`] that authors write and
//! [`lower`], which projects it into the contract's flat, pre-ordered node
//! list — the mirror of `host-shared`'s `build_tree` on the other side of
//! the ABI. The flat list is the WIT's (see `wit/`); the nested tree spares
//! authors hand-assigning parent ids and pre-order.
//!
//! This crate has no runtime dependencies: its token enums ([`Color`],
//! [`Size`], [`Direction`], [`Align`]) are plain Rust mirrors of the WIT
//! contract's, and [`lower`] is generic over the output type. Its one
//! dependency is a proc-macro crate, compile-time only: it generates the
//! [`to_node`] projection and never lands in the component's wasm.
//!
//! **Projection.** Each `wit-bindgen generate!` is crate-local, so the
//! generated `Node` and `widget` records differ per component and the
//! projection onto them can't be a shared `fn`. The [`to_node`] macro takes
//! the component's `widget` module (`component_ui::to_node!(wit)`) and
//! generates the `fn to_node` to pass to [`lower`] and the `changes` diff,
//! in terms of the component's own generated types.
//!
//! Trees are usually written with the [`ui`] macro, a JSX-like literal:
//! `ui! { column id = 1, { button label = "+", ... } }` builds the same
//! nested [`Tree`] the builders do. A child that is a list of trees is
//! spliced into the container's children (the JSX array idiom), so a
//! variable number of rows is part of the literal.
//!
//! **Identity.** Per the WIT, a node's `id` (optional) is its handle: what
//! `on-click` targets and what the diff keys on. A node with children must
//! have one (its children name it in their `parent` field) — asserted in
//! debug builds.
//!
//! **Diff.** The `changes` channel is owned by the layer, not the component:
//! the last export this component made is retained ([`note_view`] for a full
//! snapshot, [`diff`] for a diff). A node is reported `changed` when its
//! record — content, style, direction, or parent — differs from it, or when
//! the host has never seen its id; an id no longer in the tree is `removed`.
//! A demo's handlers mutate state only, and `changes` is one [`diff`] call.

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

impl Default for Color {
    fn default() -> Self {
        Color::Neutral
    }
}

/// A semantic size, mirroring `waui:ui/widget`'s `size` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Small,
    Medium,
    Large,
}

impl Default for Size {
    fn default() -> Self {
        Size::Medium
    }
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

/// How a container places its children along the cross axis, mirroring
/// `waui:ui/widget`'s `align` enum (same case order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
}

impl Default for Align {
    fn default() -> Self {
        Align::Center
    }
}

impl Style {
    pub fn new(color: Color, size: Size) -> Self {
        Self { color, size }
    }
}

impl Default for Style {
    fn default() -> Self {
        Self::new(Color::Neutral, Size::Medium)
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

/// A single-line text input.
#[derive(Debug, Clone)]
pub struct Input {
    /// This node's id (see the crate docs, *Identity*).
    pub id: Option<u32>,
    pub content: String,
    pub style: Style,
    /// The field's width in characters; `0` leaves it to the host's default.
    pub width: u32,
}

/// A horizontal or vertical container.
#[derive(Debug, Clone)]
pub struct Container {
    /// This node's id (see the crate docs, *Identity*).
    pub id: Option<u32>,
    pub direction: Direction,
    /// How the children are placed along the cross axis.
    pub align: Align,
    pub children: Vec<Tree>,
}

/// A nested widget tree — what a component *author* writes. Nodes are
/// immutable: state-dependent values are re-derived into a fresh tree per
/// export, and the diff compares trees (see the crate docs, *Diff*).
#[derive(Debug, Clone)]
pub enum Tree {
    Text(Text),
    Button(Button),
    Input(Input),
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

    /// A single-line text input (id-less until [`id`](Self::id) is called;
    /// the width is `0`, the host's default, until [`width`](Self::width)).
    pub fn input(content: impl Into<String>, style: Style) -> Self {
        Self::Input(Input {
            id: None,
            content: content.into(),
            style,
            width: 0,
        })
    }

    /// A horizontal container (id-less until [`id`](Self::id) is called;
    /// required if it has children).
    pub fn row(children: Vec<Tree>) -> Self {
        Self::Container(Container {
            id: None,
            direction: Direction::Row,
            align: Align::Center,
            children,
        })
    }

    /// A vertical container (id-less until [`id`](Self::id) is called;
    /// required if it has children).
    pub fn column(children: Vec<Tree>) -> Self {
        Self::Container(Container {
            id: None,
            direction: Direction::Column,
            align: Align::Center,
            children,
        })
    }

    /// A container with an explicit direction (id-less until
    /// [`id`](Self::id) is called; required if it has children).
    pub fn container(direction: Direction, children: Vec<Tree>) -> Self {
        Self::Container(Container {
            id: None,
            direction,
            align: Align::Center,
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

    /// Replace a container's cross-axis alignment. A no-op on a leaf.
    pub fn align(mut self, align: Align) -> Self {
        self.set_align(align);
        self
    }

    /// Replace an input's width, in characters (`0` = the host's default).
    /// A no-op on other nodes.
    pub fn width(mut self, width: u32) -> Self {
        self.set_width(width);
        self
    }

    /// Set this node's id (in place).
    pub fn set_id(&mut self, id: u32) {
        match self {
            Self::Text(t) => t.id = Some(id),
            Self::Button(b) => b.id = Some(id),
            Self::Input(i) => i.id = Some(id),
            Self::Container(c) => c.id = Some(id),
        }
    }

    /// Replace this node's style (in place). Containers have none.
    pub fn set_style(&mut self, style: Style) {
        match self {
            Self::Text(t) => t.style = style,
            Self::Button(b) => b.style = style,
            Self::Input(i) => i.style = style,
            Self::Container(_) => {}
        }
    }

    /// Replace a container's cross-axis alignment (in place). Leaves have none.
    pub fn set_align(&mut self, align: Align) {
        if let Self::Container(c) = self {
            c.align = align;
        }
    }

    /// Replace an input's width, in characters (in place). Other nodes have
    /// none.
    pub fn set_width(&mut self, width: u32) {
        if let Self::Input(i) = self {
            i.width = width;
        }
    }

    /// This node's id, whichever variant (`None` if id-less). The bare name
    /// `id` is taken by the consuming builder.
    pub fn node_id(&self) -> Option<u32> {
        match self {
            Self::Text(t) => t.id,
            Self::Button(b) => b.id,
            Self::Input(i) => i.id,
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
/// Also what the diff channel compares: a node's record is its flat form
/// (a container carries no child data, so it is `changed` only for its own
/// id, parent, or direction).
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
    Input {
        id: Option<u32>,
        parent: Option<u32>,
        content: String,
        style: Style,
        width: u32,
    },
    Container {
        id: Option<u32>,
        parent: Option<u32>,
        direction: Direction,
        align: Align,
    },
}

impl Flat {
    /// This node's id, whichever variant (`None` if id-less).
    pub fn id(&self) -> Option<u32> {
        match self {
            Flat::Text { id, .. } | Flat::Button { id, .. } | Flat::Input { id, .. } => *id,
            Flat::Container { id, .. } => *id,
        }
    }
}

/// One diff entry: a node to report `changed` (with the lowering's parent
/// and its 0-based index among the parent's children, whose record differs
/// from the last export or which the host has never seen), or an id to
/// report `removed`.
pub enum DiffNode<'a> {
    Changed(&'a Tree, Option<u32>, u32),
    Removed(u32),
}

/// What this component last exported: the tree flattened, one [`Flat`] per
/// node in depth-first pre-order, or `None` before the first export. One
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

// ------------------------------------------------------------------ projection

/// The projection from the ergonomic [`Tree`] to a component's generated
/// `Node` — see the crate docs, *Projection*.
pub use component_ui_macros::to_node;

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
        Tree::Input(i) => Flat::Input {
            id: i.id,
            parent,
            content: i.content.clone(),
            style: i.style,
            width: i.width,
        },
        Tree::Container(c) => Flat::Container {
            id: c.id,
            parent,
            direction: c.direction,
            align: c.align,
        },
    }
}

/// A tree as a flat list (depth-first pre-order), for the diff channel.
pub fn flat_list(tree: &Tree) -> Vec<Flat> {
    lower(tree, flat)
}

/// The diff channel for a tree against an explicit reference; the
/// component-facing [`diff`] holds that reference in [`LAST_EXPORTED`].
///
/// An id'd node is reported [`DiffNode::Changed`] when its record — content,
/// style, direction, or parent — differs from the reference, or when the
/// reference is `None` (bootstrap); the entry carries the node's 0-based
/// index among its parent's children, which the host uses to place a new or
/// re-parented node. Ids in the reference that are no longer in the tree are
/// reported [`DiffNode::Removed`], after the `changed` nodes, in the
/// reference's pre-order. Id-less nodes are unaddressable (the WIT's identity
/// rule) and never reported. A sibling re-order under an unchanged parent is
/// not expressible (a `changed` node is replaced where it stands); the WIT
/// says: remove it and re-add it.
///
/// Returns the mutation list and the tree flattened, ready to be the next
/// reference.
pub fn diff_with<M, F>(tree: &Tree, last: Option<&Vec<Flat>>, mut f: F) -> (Vec<M>, Vec<Flat>)
where
    F: FnMut(DiffNode<'_>) -> M,
{
    let new = flat_list(tree);
    let mut out = Vec::new();
    diff_node_against(tree, None, 0, last, &mut f, &mut out);
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

/// Walk the new tree in pre-order, reporting a node whose record differs
/// from the reference (or every id'd node when there is none). `index` is
/// the node's 0-based position among its parent's children (0 for a root),
/// carried in the `changed` entry for the host to place it.
fn diff_node_against<'a, M, F>(
    node: &'a Tree,
    parent: Option<u32>,
    index: u32,
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
            out.push(f(DiffNode::Changed(node, parent, index)));
        }
    }
    if let Tree::Container(c) = node {
        for (i, child) in c.children.iter().enumerate() {
            diff_node_against(child, node.node_id(), i as u32, last, f, out);
        }
    }
}

/// The component-facing diff channel: what `tree` reports against
/// [`LAST_EXPORTED`], as the caller's mutation type behind `f` (see
/// [`diff_with`]). The tree just diffed becomes the next reference; a full
/// snapshot goes through [`note_view`].
pub fn diff<M, F>(tree: &Tree, f: F) -> Vec<M>
where
    F: FnMut(DiffNode<'_>) -> M,
{
    let last = LAST_EXPORTED.lock().unwrap().take();
    let (out, new) = diff_with(tree, last.as_ref(), f);
    *LAST_EXPORTED.lock().unwrap() = Some(new);
    out
}

/// Record a full snapshot as the reference for the next [`diff`]; the demo's
/// `view` calls this, so a diff after a view sees nothing.
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
            Tree::Input(i) => Flat::Input {
                id: i.id,
                parent,
                content: i.content.clone(),
                style: i.style,
                width: i.width,
            },
            Tree::Container(c) => Flat::Container {
                id: c.id,
                parent,
                direction: c.direction,
                align: c.align,
            },
        }
    }

    fn ids(flat: &[Flat]) -> Vec<(Option<u32>, Option<u32>)> {
        flat.iter()
            .map(|f| match f {
                Flat::Text { id, parent, .. }
                | Flat::Button { id, parent, .. }
                | Flat::Input { id, parent, .. } => (*id, *parent),
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
        assert_eq!(
            ids(&flat),
            vec![
                (Some(1), None),
                (Some(2), Some(1)),
                (Some(3), Some(1)),
                (Some(4), Some(1))
            ]
        );
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
            Tree::row(vec![
                Tree::button("b", style(Color::Accent, Size::Small)).id(5)
            ])
            .id(4),
        ])
        .id(1);
        assert_eq!(
            ids(&lower(&tree, to_flat)),
            vec![
                (Some(1), None),
                (Some(3), Some(1)),
                (Some(4), Some(1)),
                (Some(5), Some(4))
            ]
        );
    }

    #[test]
    fn multiple_roots_are_all_lowered() {
        let trees = vec![
            Tree::text("one", style(Color::Neutral, Size::Small)).id(1),
            Tree::column(vec![
                Tree::text("two", style(Color::Neutral, Size::Small)).id(3)
            ])
            .id(2),
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

    // The `ui!` macro: the shapes the demos write with it.

    #[test]
    fn macro_builds_the_counter_shape() {
        let tree = ui! {
            column id = 1, {
                button label = "+", id = 2, color = Color::Positive, size = Size::Medium,
                text content = "0", id = 3, size = Size::Large,
                button label = "-", id = 4, color = Color::Negative, size = Size::Medium,
            }
        };
        assert_eq!(
            ids(&lower(&tree, to_flat)),
            vec![
                (Some(1), None),
                (Some(2), Some(1)),
                (Some(3), Some(1)),
                (Some(4), Some(1))
            ]
        );
    }

    #[test]
    fn macro_builds_a_container_align() {
        // A stated align is honored; an unstated one defaults to center.
        let tree = ui! {
            column id = 1, align = Align::Start, {
                column id = 2, {
                    text content = "x", id = 3, size = Size::Small,
                }
            }
        };
        let flat = lower(&tree, to_flat);
        match &flat[0] {
            Flat::Container {
                direction, align, ..
            } => {
                assert_eq!(*direction, Direction::Column);
                assert_eq!(*align, Align::Start);
            }
            _ => panic!("expected the outer container"),
        }
        match &flat[1] {
            Flat::Container { align, .. } => assert_eq!(*align, Align::Center),
            _ => panic!("expected the inner container"),
        }
    }

    #[test]
    fn macro_takes_a_single_tree_expression_child() {
        let leaf = Tree::text("x", style(Color::Neutral, Size::Small)).id(2);
        let tree = ui! {
            column id = 1, {
                leaf,
            }
        };
        assert_eq!(
            ids(&lower(&tree, to_flat)),
            vec![(Some(1), None), (Some(2), Some(1))]
        );
    }

    #[test]
    fn macro_splices_a_list_child() {
        let rows: Vec<Tree> = (0..3)
            .map(|i| Tree::text(format!("r{i}"), style(Color::Neutral, Size::Small)).id(10 + i))
            .collect();
        let tree = ui! {
            column id = 1, {
                text content = "head", id = 2, size = Size::Small,
                rows,
                text content = "tail", id = 3, size = Size::Small,
            }
        };
        assert_eq!(
            ids(&lower(&tree, to_flat)),
            vec![
                (Some(1), None),
                (Some(2), Some(1)),
                (Some(10), Some(1)),
                (Some(11), Some(1)),
                (Some(12), Some(1)),
                (Some(3), Some(1))
            ]
        );
    }

    #[test]
    fn macro_builds_an_input() {
        // The input lowers like text: the same record shape, a `content`
        // attribute, and its own tag; `width` is the input's own field.
        let tree = ui! {
            column id = 1, {
                input content = "", id = 9, size = Size::Medium, width = 12,
            }
        };
        let flat = lower(&tree, to_flat);
        assert_eq!(ids(&flat), vec![(Some(1), None), (Some(9), Some(1))]);
        match &flat[1] {
            Flat::Input {
                content,
                style,
                width,
                ..
            } => {
                assert_eq!(content, "");
                assert_eq!(style.size, Size::Medium);
                assert_eq!(*width, 12);
            }
            _ => panic!("expected an input node"),
        }
    }

    #[test]
    fn macro_splices_an_iterator_child() {
        // The todo demo's shape: a column whose children are a `map`, not a
        // collected `Vec`.
        let labels = ["a", "b", "c"];
        let tree = ui! {
            column id = 1, {
                column id = 2, { labels.iter().enumerate().map(|(i, l)| Tree::text(*l, style(Color::Neutral, Size::Small)).id(100 + i as u32)) }
            }
        };
        assert_eq!(
            ids(&lower(&tree, to_flat)),
            vec![
                (Some(1), None),
                (Some(2), Some(1)),
                (Some(100), Some(2)),
                (Some(101), Some(2)),
                (Some(102), Some(2))
            ]
        );
    }

    // The diff tests drive `diff_with` with an explicit reference rather
    // than the component-facing `diff`: tests run in parallel in one process
    // and share `LAST_EXPORTED`, but the core is pure.

    #[derive(Debug, PartialEq, Eq)]
    enum M {
        Changed(u32, Option<u32>, u32), // id, parent, index
        Removed(u32),
    }

    fn proj(d: DiffNode<'_>) -> M {
        match d {
            DiffNode::Changed(t, p, i) => {
                M::Changed(t.node_id().expect("a reported node has an id"), p, i)
            }
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
                M::Changed(1, None, 0),
                M::Changed(2, Some(1), 0),
                M::Changed(3, Some(1), 1),
                M::Changed(4, Some(1), 2),
            ]
        );
    }

    #[test]
    fn diff_is_empty_when_unchanged() {
        let tree = counter("0", Color::Neutral);
        let last = flat_list(&tree);
        // Rebuilt the way the demos do: a fresh tree from state.
        let again = counter("0", Color::Neutral);
        let (out, _) = diff_with(&again, Some(&last), proj);
        assert_eq!(out, Vec::<M>::new());
    }

    #[test]
    fn diff_reports_the_changed_readout_alone() {
        let last = flat_list(&counter("0", Color::Neutral));
        let (out, _) = diff_with(&counter("1", Color::Neutral), Some(&last), proj);
        assert_eq!(out, vec![M::Changed(3, Some(1), 1)]);
    }

    #[test]
    fn diff_reports_a_style_only_change() {
        let last = flat_list(&counter("0", Color::Neutral));
        // Crossing zero changes the readout's color with its content.
        let (out, _) = diff_with(&counter("-1", Color::Negative), Some(&last), proj);
        assert_eq!(out, vec![M::Changed(3, Some(1), 1)]);
    }

    /// A box (1) with one child; the box's align is the only thing that
    /// varies. A container's record is its direction and align, so an align
    /// change is a `changed`.
    fn box_align(align: Align) -> Tree {
        Tree::column(vec![
            Tree::text("x", style(Color::Neutral, Size::Small)).id(2)
        ])
        .id(1)
        .align(align)
    }

    #[test]
    fn diff_reports_a_container_align_change() {
        let last = flat_list(&box_align(Align::Center));
        let (out, _) = diff_with(&box_align(Align::Start), Some(&last), proj);
        assert_eq!(out, vec![M::Changed(1, None, 0)]);
    }

    /// A todo-ish shape: a box (1) with an input (9) and an add button (7).
    /// The input's content is state; the button is static.
    fn with_input(content: &str, width: u32) -> Tree {
        Tree::column(vec![
            Tree::input(content, style(Color::Neutral, Size::Medium))
                .id(9)
                .width(width),
            Tree::button("add", style(Color::Positive, Size::Medium)).id(7),
        ])
        .id(1)
    }

    #[test]
    fn diff_reports_an_input_content_change() {
        // Typing into the input changes only the input node's record.
        let last = flat_list(&with_input("", 12));
        let (out, _) = diff_with(&with_input("buy milk", 12), Some(&last), proj);
        assert_eq!(out, vec![M::Changed(9, Some(1), 0)]);
    }

    #[test]
    fn diff_reports_a_width_only_change() {
        // The width is part of the input's record: a resize with the same
        // content is a `changed`.
        let last = flat_list(&with_input("", 12));
        let (out, _) = diff_with(&with_input("", 24), Some(&last), proj);
        assert_eq!(out, vec![M::Changed(9, Some(1), 0)]);
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
            &Tree::column(vec![
                Tree::text("a", style(Color::Neutral, Size::Small)).id(5)
            ])
            .id(1),
        );
        // The new tree: a new row (6) under the box, the text re-parented
        // under it.
        let new = Tree::column(vec![Tree::row(vec![Tree::text(
            "a",
            style(Color::Neutral, Size::Small),
        )
        .id(5)])
        .id(6)])
        .id(1);
        let (out, _) = diff_with(&new, Some(&last), proj);
        assert_eq!(
            out,
            vec![M::Changed(6, Some(1), 0), M::Changed(5, Some(6), 0)]
        );
    }

    #[test]
    fn diff_never_reports_idless_nodes() {
        let last = flat_list(
            &Tree::column(vec![
                Tree::text("old", style(Color::Neutral, Size::Small)), // id-less
                Tree::text("keep", style(Color::Neutral, Size::Small)).id(2),
            ])
            .id(1),
        );
        let new = Tree::column(vec![
            Tree::text("new", style(Color::Neutral, Size::Small)), // id-less, changed
            Tree::text("keep", style(Color::Neutral, Size::Small)).id(2),
        ])
        .id(1);
        let (out, _) = diff_with(&new, Some(&last), proj);
        assert_eq!(out, Vec::<M>::new());
    }
}

// ------------------------------------------------------------------ the macro

/// One child of a [`ui!`] container: a single node, or a list of nodes
/// spliced into the children (the JSX array idiom: a list child is not a
/// node).
#[derive(Debug)]
pub enum Kid {
    Tree(Tree),
    Trees(Vec<Tree>),
}

impl From<Tree> for Kid {
    fn from(tree: Tree) -> Self {
        Kid::Tree(tree)
    }
}

/// Any iterator of trees (a `Vec<Tree>`, a `map`, ...) is a list child.
/// A `Tree` is not an iterator, so the two `From`s do not overlap.
impl<T: IntoIterator<Item = Tree>> From<T> for Kid {
    fn from(t: T) -> Self {
        Kid::Trees(t.into_iter().collect())
    }
}

/// The children of a [`ui!`] container, normalized to a list: a single
/// node becomes a one-element list, a list of nodes is spliced as is.
/// The macro expansion calls this for every child.
pub fn kid(c: impl Into<Kid>) -> Vec<Tree> {
    match c.into() {
        Kid::Tree(tree) => vec![tree],
        Kid::Trees(trees) => trees,
    }
}

/// A JSX-like widget tree.
///
/// ```
/// # use component_ui::{Color, Size, Tree};
/// let tree = component_ui::ui! {
///     column id = 1, {
///         button label = "+", id = 2, color = Color::Positive, size = Size::Medium,
///         text content = "0", id = 3, size = Size::Large,
///         button label = "-", id = 4, color = Color::Negative, size = Size::Medium,
///     }
/// };
/// assert_eq!(tree.node_id(), Some(1));
/// ```
///
/// Tags: `text` (`content`), `button` (`label`), `input` (`content`),
/// `row`, `column`, `container` (`direction`). Attributes in any order:
/// `id`, `color`, `size`, `style`; an unstated color or size is
/// [`Color::Neutral`] / [`Size::Medium`]. Inputs also take an optional
/// `width` (in characters, default `0` = the host's default); a `width` on
/// a `text` or `button` is a no-op. Containers also take an optional
/// `align` (a [`Align`], default [`Align::Center`]). Containers take a
/// `{ ... }` children block; children are tags or bare expressions,
/// comma-separated.
/// A bare expression whose first token is a tag name (a `Tree` named
/// `text`, `row`, ...) is the expression. A bare expression that is a list
/// of trees (a `Vec<Tree>`, an iterator) is spliced into the children — the
/// JSX array idiom — while one that is a single [`Tree`] is one child.
#[macro_export]
macro_rules! ui {
    ( $( $t:tt )* ) => {
        $crate::__ui_kids!(top () $( $t )* )
    };
}

/// Expand a stream of children (the `ui!` content or a block's interior).
///
/// The ctx is `top` (exactly one item; the result is the bare `Tree`) or
/// `kids` (a list; the result is `Vec<Tree>`). The acc is a parenthesized
/// group holding the expanded items so far, each followed by a comma. Each
/// item is a `Vec<Tree>`: a single node wrapped by [`kid`] (a one-element
/// list) or a list of nodes spliced (see [`kid`]).
#[macro_export]
#[doc(hidden)]
macro_rules! __ui_kids {
    // The stream is exhausted: the acc is the whole list. Each item is
    // followed by its comma; a `,*` repeat parses `:expr` restrictively.
    // A spliced list child extends the list; a single node adds one.
    ( kids ( $( $a:expr , )* ) ) => {
        {
            let mut kids = Vec::new();
            $( kids.extend($a); )*
            kids
        }
    };
    ( top ( ) ) => {
        compile_error!("`ui!` needs a root widget")
    };
    ( top ( $a:expr , ) ) => {
        $a
    };
    ( top ( $a:expr , $b:tt $( $c:tt )* ) ) => {
        compile_error!("`ui!` takes a single root widget")
    };
    // A separator comma with no child after it (a stray or trailing one):
    // skip it and continue.
    ( $ctx:tt $acc:tt , $( $r:tt )* ) => {
        $crate::__ui_kids!($ctx $acc $( $r )* )
    };
    // A child whose second token is an attribute key or a children block:
    // a tagged widget. The first token is spliced as a fragment into
    // `__ui_tag`, which matches it by value against the tag names.
    ( $ctx:tt $acc:tt $t0:tt content $( $r:tt )* ) => {
        $crate::__ui_tag!($ctx $acc $t0 content $( $r )* )
    };
    ( $ctx:tt $acc:tt $t0:tt label $( $r:tt )* ) => {
        $crate::__ui_tag!($ctx $acc $t0 label $( $r )* )
    };
    ( $ctx:tt $acc:tt $t0:tt id $( $r:tt )* ) => {
        $crate::__ui_tag!($ctx $acc $t0 id $( $r )* )
    };
    ( $ctx:tt $acc:tt $t0:tt color $( $r:tt )* ) => {
        $crate::__ui_tag!($ctx $acc $t0 color $( $r )* )
    };
    ( $ctx:tt $acc:tt $t0:tt size $( $r:tt )* ) => {
        $crate::__ui_tag!($ctx $acc $t0 size $( $r )* )
    };
    ( $ctx:tt $acc:tt $t0:tt style $( $r:tt )* ) => {
        $crate::__ui_tag!($ctx $acc $t0 style $( $r )* )
    };
    ( $ctx:tt $acc:tt $t0:tt width $( $r:tt )* ) => {
        $crate::__ui_tag!($ctx $acc $t0 width $( $r )* )
    };
    ( $ctx:tt $acc:tt $t0:tt direction $( $r:tt )* ) => {
        $crate::__ui_tag!($ctx $acc $t0 direction $( $r )* )
    };
    ( $ctx:tt $acc:tt $t0:tt align $( $r:tt )* ) => {
        $crate::__ui_tag!($ctx $acc $t0 align $( $r )* )
    };
    ( $ctx:tt $acc:tt $t0:tt { $( $c:tt )* } $( $r:tt )* ) => {
        $crate::__ui_tag!($ctx $acc $t0 { $( $c )* } $( $r )* )
    };
    // A child whose second token is a comma and third a children block:
    // the tolerated `column, { ... }` form. Containers route to the tag
    // machinery; leaf names (or any non-container) are expression children.
    ( $ctx:tt $acc:tt $t0:tt , { $( $c:tt )* } $( $r:tt )* ) => {
        $crate::__ui_cmb!($ctx $acc $t0 , { $( $c )* } $( $r )* )
    };
    // Anything else is a bare expression child. The first token is the
    // caller's own; a `Tree` named after a tag resolves here, not in the
    // tag machinery.
    ( $ctx:tt $acc:tt $t0:tt $( $r:tt )* ) => {
        $crate::__ui_expr!($ctx $acc () $t0 $( $r )* )
    };
}

/// The comma-block shape: a `column, { ... }` children list.
///
/// Container names match by value and route to the container machinery
/// (the comma is stripped there); anything else is an expression child
/// followed by a block child, so the first token is re-emitted as the
/// caller's own.
#[macro_export]
#[doc(hidden)]
macro_rules! __ui_cmb {
    ( $ctx:tt $acc:tt row , { $( $c:tt )* } $( $r:tt )* ) => {
        $crate::__ui_cont!(row $ctx $acc ( some $crate::Direction::Row) () () , { $( $c )* } $( $r )* )
    };
    ( $ctx:tt $acc:tt column , { $( $c:tt )* } $( $r:tt )* ) => {
        $crate::__ui_cont!(column $ctx $acc ( some $crate::Direction::Column) () () , { $( $c )* } $( $r )* )
    };
    ( $ctx:tt $acc:tt container , { $( $c:tt )* } $( $r:tt )* ) => {
        $crate::__ui_cont!(container $ctx $acc () () () , { $( $c )* } $( $r )* )
    };
    ( $ctx:tt $acc:tt $t0:tt , { $( $c:tt )* } $( $r:tt )* ) => {
        $crate::__ui_expr!($ctx $acc () $t0 , { $( $c )* } $( $r )* )
    };
}

/// Dispatch a tagged child on its first token, matched by value.
///
/// The tag literals in the patterns select the builder; they are only ever
/// used to pick a builder or a `$crate` path, never as a caller expression,
/// so their def-site hygiene is fine. A first token that is not a tag name
/// is a bare expression child.
#[macro_export]
#[doc(hidden)]
macro_rules! __ui_tag {
    ( $ctx:tt $acc:tt text $t1:tt $( $r:tt )* ) => {
        $crate::__ui_leaf!(text $ctx $acc () () () () () () $t1 $( $r )* )
    };
    ( $ctx:tt $acc:tt button $t1:tt $( $r:tt )* ) => {
        $crate::__ui_leaf!(button $ctx $acc () () () () () () $t1 $( $r )* )
    };
    ( $ctx:tt $acc:tt input $t1:tt $( $r:tt )* ) => {
        $crate::__ui_leaf!(input $ctx $acc () () () () () () $t1 $( $r )* )
    };
    ( $ctx:tt $acc:tt row $t1:tt $( $r:tt )* ) => {
        $crate::__ui_cont!(row $ctx $acc ( some $crate::Direction::Row) () () $t1 $( $r )* )
    };
    ( $ctx:tt $acc:tt column $t1:tt $( $r:tt )* ) => {
        $crate::__ui_cont!(column $ctx $acc ( some $crate::Direction::Column) () () $t1 $( $r )* )
    };
    ( $ctx:tt $acc:tt container $t1:tt $( $r:tt )* ) => {
        $crate::__ui_cont!(container $ctx $acc () () () $t1 $( $r )* )
    };
    ( $ctx:tt $acc:tt $t0:tt $t1:tt $( $r:tt )* ) => {
        $crate::__ui_expr!($ctx $acc () $t0 $t1 $( $r )* )
    };
}

/// A slot holding an attribute value: `()` or `( some <expr>)`.
#[macro_export]
#[doc(hidden)]
macro_rules! __ui_opt {
    ( () ) => {
        None
    };
    ( ( some $v:expr ) ) => {
        Some($v)
    };
}

/// Accumulate a bare expression child, then continue the list.
///
/// The acc is a group of the finished items (each followed by its comma, or
/// empty); the cur is a group of the current child's tokens. A single
/// expression carries no top-level comma (delimiters fold into groups), so
/// a top-level comma always ends the child. A finished child is normalized
/// by `__ui_wrap`: in `kids` context it becomes a `kid` call (a single node
/// or a spliced list), at `top` it is the root itself.
#[macro_export]
#[doc(hidden)]
macro_rules! __ui_expr {
    // A top-level comma: the child ends; its tokens join the acc.
    ( $ctx:tt ( $( $a:tt )* ) ( $( $c:tt )* ) , $( $r:tt )* ) => {
        $crate::__ui_kids!($ctx ( $( $a )* $crate::__ui_wrap!($ctx $( $c )*) , ) $( $r )* )
    };
    // End of stream: the last child ends too.
    ( $ctx:tt ( $( $a:tt )* ) ( $( $c:tt )* ) ) => {
        $crate::__ui_kids!($ctx ( $( $a )* $crate::__ui_wrap!($ctx $( $c )*) , ) )
    };
    // The next token continues the child.
    ( $ctx:tt ( $( $a:tt )* ) ( $( $c:tt )* ) $t0:tt $( $r:tt )* ) => {
        $crate::__ui_expr!($ctx ( $( $a )* ) ( $( $c )* $t0 ) $( $r )* )
    };
}

/// Normalize a finished child for the children list: in `kids` context,
/// wrap it in [`kid`] (a single node becomes a one-element list, a list of
/// nodes is spliced); at `top`, the single root is the child itself.
#[macro_export]
#[doc(hidden)]
macro_rules! __ui_wrap {
    ( top $e:expr ) => {
        $e
    };
    ( kids $e:expr ) => {
        $crate::kid($e)
    };
}

/// A leaf node: `text` / `button` / `input`. Slot order: label, id, color,
/// size, style. The label must be present (a `content` / `label` attribute).
#[macro_export]
#[doc(hidden)]
macro_rules! __ui_leaf {
    // Attribute peels: each key consumes `key = <expr>,` and restates its
    // slot. The keys are literal in the pattern, so an unknown key or a
    // `label` on a `text` falls through to the terminals and fails there.
    ( $kind:tt $ctx:tt $acc:tt $l:tt $i:tt $c:tt $s:tt $st:tt $w:tt id = $v:expr , $( $r:tt )* ) => {
        $crate::__ui_leaf!($kind $ctx $acc $l ( some $v) $c $s $st $w $( $r )* )
    };
    ( $kind:tt $ctx:tt $acc:tt $l:tt $i:tt $c:tt $s:tt $st:tt $w:tt color = $v:expr , $( $r:tt )* ) => {
        $crate::__ui_leaf!($kind $ctx $acc $l $i ( some $v) $s $st $w $( $r )* )
    };
    ( $kind:tt $ctx:tt $acc:tt $l:tt $i:tt $c:tt $s:tt $st:tt $w:tt size = $v:expr , $( $r:tt )* ) => {
        $crate::__ui_leaf!($kind $ctx $acc $l $i $c ( some $v) $st $w $( $r )* )
    };
    ( $kind:tt $ctx:tt $acc:tt $l:tt $i:tt $c:tt $s:tt $st:tt $w:tt style = $v:expr , $( $r:tt )* ) => {
        $crate::__ui_leaf!($kind $ctx $acc $l $i $c $s ( some $v) $w $( $r )* )
    };
    ( $kind:tt $ctx:tt $acc:tt $l:tt $i:tt $c:tt $s:tt $st:tt $w:tt width = $v:expr , $( $r:tt )* ) => {
        $crate::__ui_leaf!($kind $ctx $acc $l $i $c $s $st ( some $v) $( $r )* )
    };
    ( text $ctx:tt $acc:tt $l:tt $i:tt $c:tt $s:tt $st:tt $w:tt content = $v:expr , $( $r:tt )* ) => {
        $crate::__ui_leaf!(text $ctx $acc ( some $v) $i $c $s $st $w $( $r )* )
    };
    ( button $ctx:tt $acc:tt $l:tt $i:tt $c:tt $s:tt $st:tt $w:tt label = $v:expr , $( $r:tt )* ) => {
        $crate::__ui_leaf!(button $ctx $acc ( some $v) $i $c $s $st $w $( $r )* )
    };
    ( input $ctx:tt $acc:tt $l:tt $i:tt $c:tt $s:tt $st:tt $w:tt content = $v:expr , $( $r:tt )* ) => {
        $crate::__ui_leaf!(input $ctx $acc ( some $v) $i $c $s $st $w $( $r )* )
    };
    // Terminals: the label slot must be present (bound here, unwrapped);
    // the rest are optional.
    ( $kind:tt top $acc:tt ( some $l:expr ) $i:tt $c:tt $s:tt $st:tt $w:tt ) => {
        $crate::__ui_leafnode!($kind $i $c $s $st $w $l)
    };
    ( $kind:tt top $acc:tt ( some $l:expr ) $i:tt $c:tt $s:tt $st:tt $w:tt , ) => {
        $crate::__ui_leafnode!($kind $i $c $s $st $w $l)
    };
    ( $kind:tt top $acc:tt ( some $l:expr ) $i:tt $c:tt $s:tt $st:tt $w:tt , $x:tt $( $y:tt )* ) => {
        compile_error!("`ui!` takes a single root widget")
    };
    ( $kind:tt kids ( $( $x:tt )* ) ( some $l:expr ) $i:tt $c:tt $s:tt $st:tt $w:tt $( $r:tt )* ) => {
        // The acc holds finished items only (each followed by its comma, or
        // empty); a bare `*` re-emits the trailing comma, a `,*` would
        // double it.
        $crate::__ui_kids!(kids ( $( $x )* $crate::__ui_wrap!(kids $crate::__ui_leafnode!($kind $i $c $s $st $w $l)) , ) $( $r )* )
    };
    // A label-less leaf: whatever follows, the required `content` /
    // `label` attribute never appeared.
    ( $kind:tt $ctx:tt $acc:tt () $i:tt $c:tt $s:tt $st:tt $w:tt $( $r:tt )* ) => {
        compile_error!("`text` and `input` need a `content = ...` attribute; `button` needs a `label = ...` attribute")
    };
}

/// The node a leaf peels into. `$l` is the unwrapped label expression; the
/// remaining slots are still `()` / `( some ...)` groups.
#[macro_export]
#[doc(hidden)]
macro_rules! __ui_leafnode {
    ( $kind:tt $i:tt $c:tt $s:tt $st:tt $w:tt $l:expr ) => {{
        let style = $crate::__ui_opt!($st).unwrap_or_else(|| {
            $crate::Style::new(
                $crate::__ui_opt!($c).unwrap_or_default(),
                $crate::__ui_opt!($s).unwrap_or_default(),
            )
        });
        let mut node = $crate::__ui_leafbuild!($kind, $l, style);
        if let Some(id) = $crate::__ui_opt!($i) {
            node.set_id(id);
        }
        if let Some(width) = $crate::__ui_opt!($w) {
            node.set_width(width);
        }
        node
    }};
}

/// The builder call, once per tag.
#[macro_export]
#[doc(hidden)]
macro_rules! __ui_leafbuild {
    ( text, $l:expr, $style:expr ) => {
        $crate::Tree::text($l, $style)
    };
    ( button, $l:expr, $style:expr ) => {
        $crate::Tree::button($l, $style)
    };
    ( input, $l:expr, $style:expr ) => {
        $crate::Tree::input($l, $style)
    };
}

/// A container: `row` / `column` / `container`. Slot order: direction, id,
/// align. `row` / `column` preset the direction (a `direction` attribute is a
/// compile error on them); `container` requires one. `align` is optional (it
/// defaults to `center`).
#[macro_export]
#[doc(hidden)]
macro_rules! __ui_cont {
    // Attribute peels: each consumes `key = <expr>,` and restates its slot.
    ( $tag:tt $ctx:tt $acc:tt $d:tt $i:tt $al:tt id = $v:expr , $( $r:tt )* ) => {
        $crate::__ui_cont!($tag $ctx $acc $d ( some $v) $al $( $r )* )
    };
    ( $tag:tt $ctx:tt $acc:tt $d:tt $i:tt $al:tt align = $v:expr , $( $r:tt )* ) => {
        $crate::__ui_cont!($tag $ctx $acc $d $i ( some $v) $( $r )* )
    };
    ( container $ctx:tt $acc:tt $d:tt $i:tt $al:tt direction = $v:expr , $( $r:tt )* ) => {
        $crate::__ui_cont!(container $ctx $acc ( some $v) $i $al $( $r )* )
    };
    // A separator comma with no attributes (a `column, { ... }`): strip it
    // and retry; a comma-less block matches the terminals below.
    ( $tag:tt $ctx:tt $acc:tt $d:tt $i:tt $al:tt , { $( $c:tt )* } $( $r:tt )* ) => {
        $crate::__ui_cont!($tag $ctx $acc $d $i $al { $( $c )* } $( $r )* )
    };
    // Terminals: the children block. `row` / `column` keep their preset
    // direction; `container` must have been given one. The children are
    // rewrapped in a group: a `:tt` repetition ends where an `:expr` begins
    // only at a delimiter.
    ( $tag:tt top $acc:tt ( some $d:expr ) $i:tt $al:tt { $( $c:tt )* } ) => {
        $crate::__ui_contnode!($tag $i $al ( $( $c )* ) $d)
    };
    ( $tag:tt top $acc:tt ( some $d:expr ) $i:tt $al:tt { $( $c:tt )* } , ) => {
        $crate::__ui_contnode!($tag $i $al ( $( $c )* ) $d)
    };
    ( $tag:tt top $acc:tt ( some $d:expr ) $i:tt $al:tt { $( $c:tt )* } , $x:tt $( $y:tt )* ) => {
        compile_error!("`ui!` takes a single root widget")
    };
    ( $tag:tt top $acc:tt () $i:tt $al:tt { $( $c:tt )* } $( $r:tt )* ) => {
        compile_error!("a `container` needs a `direction = ...` attribute (or use `row` / `column`)")
    };
    // An empty acc matches with zero repetitions: one arm serves both. Each
    // acc item ends in its comma; the `:expr` binding consumes them.
    ( $tag:tt kids ( $( $a:expr , )* ) ( some $d:expr ) $i:tt $al:tt { $( $c:tt )* } $( $r:tt )* ) => {
        $crate::__ui_kids!(kids ( $( $a , )* $crate::__ui_wrap!(kids $crate::__ui_contnode!($tag $i $al ( $( $c )* ) $d)) , ) $( $r )* )
    };
    ( $tag:tt kids $acc:tt () $i:tt $al:tt { $( $c:tt )* } $( $r:tt )* ) => {
        compile_error!("a `container` needs a `direction = ...` attribute (or use `row` / `column`)")
    };
    // `direction` on a preset container: a compile error, not an attribute.
    ( row $ctx:tt $acc:tt $d:tt $i:tt $al:tt direction = $v:expr , $( $r:tt )* ) => {
        compile_error!("a `row` does not take a `direction` attribute (it is preset); use `container`")
    };
    ( column $ctx:tt $acc:tt $d:tt $i:tt $al:tt direction = $v:expr , $( $r:tt )* ) => {
        compile_error!("a `column` does not take a `direction` attribute (it is preset); use `container`")
    };
}

/// The node a container peels into.
#[macro_export]
#[doc(hidden)]
macro_rules! __ui_contnode {
    ( $tag:tt $i:tt $al:tt ( $( $c:tt )* ) $d:expr ) => {
        {
            let mut node = $crate::Tree::container($d, $crate::__ui_kids!(kids () $( $c )* ));
            if let Some(id) = $crate::__ui_opt!($i) {
                node.set_id(id);
            }
            if let Some(align) = $crate::__ui_opt!($al) {
                node.set_align(align);
            }
            node
        }
    };
}
