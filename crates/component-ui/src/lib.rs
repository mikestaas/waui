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

}

/// A flat node — a mirror of the WIT contract's `node` variant, with the
/// ids and parent links the lowering computes. Useful for tests and for
/// components that don't run `wit-bindgen`; components targeting the `app`
/// world typically project onto their generated types instead (see
/// [`lower`]).
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
}
