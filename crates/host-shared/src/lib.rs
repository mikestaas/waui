//! The shared core of the wasmtime hosts: contract-level code only.
//!
//! - `bindgen!` for the `waui:ui/app` world
//! - [`build_tree`]: the rebuild of the component's flat node list into a
//!   widget tree
//! - [`Retained`]: the retained node list the diff protocol applies against
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
use waui::ui::widget::{Align, Direction, Style};

/// A rebuilt widget tree.
#[derive(Debug, Clone)]
pub enum Tree {
    /// (content, the component's semantic style)
    Text(String, Style),
    /// (widget id — `None` for a display-only button a component did not
    /// give an id, label, style)
    Button(Option<u32>, String, Style),
    /// (widget id — `None` for an input a component did not give an id, the
    /// input's content, style)
    Input(Option<u32>, String, Style),
    /// (the cross-axis alignment, the children)
    Row(Align, Vec<Tree>),
    /// (the cross-axis alignment, the children)
    Column(Align, Vec<Tree>),
}

/// The node's id, if it has one (see the WIT identity note: a node the
/// component never addresses or diffs may omit it).
fn node_id(node: &Node) -> Option<u32> {
    match node {
        Node::Text(t) => t.id,
        Node::Button(b) => b.id,
        Node::Input(i) => i.id,
        Node::Container(c) => c.id,
    }
}

fn node_parent(node: &Node) -> Option<u32> {
    match node {
        Node::Text(t) => t.parent,
        Node::Button(b) => b.parent,
        Node::Input(i) => i.parent,
        Node::Container(c) => c.parent,
    }
}

fn to_tree(node: &Node) -> Tree {
    match node {
        Node::Text(t) => Tree::Text(t.content.clone(), t.style),
        Node::Button(b) => Tree::Button(b.id, b.label.clone(), b.style),
        Node::Input(i) => Tree::Input(i.id, i.content.clone(), i.style),
        Node::Container(c) => {
            if c.direction == Direction::Row {
                Tree::Row(c.align, Vec::new())
            } else {
                Tree::Column(c.align, Vec::new())
            }
        }
    }
}

/// Rebuild the tree from the flat node list exported by `view()`.
///
/// The list is depth-first pre-ordered: a parent always precedes its
/// children and siblings appear in visual order, so children of each node
/// can be linked in list order.
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
            Some(Tree::Row(_, children) | Tree::Column(_, children)) => {
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
        0 => Tree::Column(Align::Center, Vec::new()),
        1 => roots.pop().unwrap(),
        _ => Tree::Column(Align::Center, roots),
    }
}

/// The host's retained copy of the component's tree, as the diff protocol
/// maintains it: a full snapshot from `view` at startup, then `changes`
/// diffs applied by node id.
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

    /// Apply a `changes` diff. Returns whether anything was retained.
    pub fn apply(&mut self, mutations: &[Mutation]) -> bool {
        if mutations.is_empty() {
            return false;
        }
        for mutation in mutations {
            match mutation {
                Mutation::Changed(c) => {
                    let node = &c.node;
                    // A node without an id is unaddressable (see the WIT
                    // identity note): the diff cannot name it, so skip it.
                    if node_id(node).is_none() {
                        continue;
                    }
                    match self.nodes.iter().position(|n| node_id(n) == node_id(node)) {
                        Some(i) => {
                            if node_parent(&self.nodes[i]) == node_parent(node) {
                                // Same parent: the node keeps its position
                                // (the index is ignored).
                                self.nodes[i] = node.clone();
                            } else {
                                // Re-parented: move it to `index` among the
                                // new parent's children.
                                self.nodes.remove(i);
                                self.insert(node.clone(), c.index);
                            }
                        }
                        // A node the host has never seen (added after
                        // bootstrap): insert at `index` among the parent's
                        // children.
                        None => self.insert(node.clone(), c.index),
                    }
                }
                Mutation::Removed(id) => self.remove_subtree(*id),
            }
        }
        true
    }

    /// Insert a node the host does not have at `index` among its parent's
    /// children (0 is the front; an index past the end appends), or at the
    /// end of the list if it is a new root. The walk skips the node's own
    /// descendants still in the list, so a re-added container lands after
    /// them.
    fn insert(&mut self, node: Node, index: u32) {
        let id = node_id(&node);
        // A node with no parent, or whose parent the host does not retain,
        // cannot be placed among a parent's children: it is a new root.
        let Some(parent) = node_parent(&node) else {
            self.nodes.push(node);
            return;
        };
        let Some(p) = self.nodes.iter().position(|n| node_id(n) == Some(parent)) else {
            self.nodes.push(node);
            return;
        };
        let mut i = p + 1;
        let mut siblings = 0;
        while i < self.nodes.len() {
            let n = &self.nodes[i];
            if self.descendant_of(n, id) {
                i += 1; // the node's own descendant
                continue;
            }
            if !self.descendant_of(n, Some(parent)) {
                break; // past the parent's subtree
            }
            if node_parent(n) == Some(parent) {
                if siblings >= index as usize {
                    break; // before the `index`-th sibling
                }
                siblings += 1;
                let child_id = node_id(n);
                i += 1;
                while i < self.nodes.len() && self.descendant_of(&self.nodes[i], child_id) {
                    i += 1;
                }
            } else {
                i += 1; // a deeper descendant: advance
            }
        }
        self.nodes.insert(i, node);
    }

    /// Remove a node and, if it is a container, its whole subtree.
    fn remove_subtree(&mut self, id: u32) {
        let Some(p) = self.nodes.iter().position(|n| node_id(n) == Some(id)) else {
            return;
        };
        let mut end = p + 1;
        while end < self.nodes.len() && self.descendant_of(&self.nodes[end], Some(id)) {
            end += 1;
        }
        self.nodes.drain(p..end);
    }

    /// Whether `node` is in the subtree rooted at `ancestor`.
    fn descendant_of(&self, node: &Node, ancestor: Option<u32>) -> bool {
        let Some(ancestor) = ancestor else {
            // Nothing can be a descendant of an id-less node.
            return false;
        };
        let mut current = node_parent(node);
        while let Some(parent) = current {
            if parent == ancestor {
                return true;
            }
            current = self
                .nodes
                .iter()
                .find(|n| node_id(n) == Some(parent))
                .and_then(node_parent);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    //! The diff machinery is pure data structure work — testable without
    //! a component in sight.

    use super::*;
    use waui::ui::widget::{Button, Changed, Color, Container, Direction, Size, Style, Text};

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
            direction: Direction::Column,
            align: Align::Center,
        })
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

    /// The retained list's ids, in order — the layout contract (id-less
    /// nodes, of which these tests have none, are skipped).
    fn ids(r: &Retained) -> Vec<u32> {
        r.nodes.iter().filter_map(node_id).collect()
    }

    #[test]
    fn apply_empty_is_a_noop() {
        let mut r = Retained::from_view(vec![cont(1, None), text(3, Some(1), "0")]);
        assert!(!r.apply(&[]));
        assert_eq!(ids(&r), vec![1, 3]);
    }

    #[test]
    fn apply_skips_an_unaddressable_change() {
        // A `changed` node without an id cannot be named (see the WIT
        // identity note): the host ignores it rather than guess.
        let mut r = Retained::from_view(vec![cont(1, None), text(3, Some(1), "0")]);
        assert!(r.apply(&[changed(
            Node::Text(Text {
                id: None,
                parent: Some(1),
                content: "intruder".to_string(),
                style: style(Color::Neutral, Size::Large),
            }),
            0
        )]));
        assert_eq!(ids(&r), vec![1, 3]);
        match &r.nodes[1] {
            Node::Text(t) => assert_eq!(t.content, "0"),
            _ => panic!("expected the original text node"),
        }
    }

    #[test]
    fn apply_replaces_in_place() {
        let mut r = Retained::from_view(vec![
            cont(1, None),
            text(3, Some(1), "0"),
            button(4, Some(1), "-"),
        ]);
        assert!(r.apply(&[changed(text(3, Some(1), "-1"), 0)]));
        assert_eq!(ids(&r), vec![1, 3, 4]); // position kept
        match &r.nodes[1] {
            Node::Text(t) => assert_eq!(t.content, "-1"),
            _ => panic!("node 3 should still be the text readout"),
        }
    }

    #[test]
    fn apply_prunes_the_whole_subtree() {
        let mut r = Retained::from_view(vec![
            cont(1, None),
            cont(2, Some(1)),
            text(3, Some(2), "a"),
            text(4, Some(2), "b"),
            button(5, Some(1), "x"),
        ]);
        assert!(r.apply(&[Mutation::Removed(2)]));
        assert_eq!(ids(&r), vec![1, 5]);
    }

    #[test]
    fn apply_reparents_to_the_canonical_position() {
        // Two roots; the readout moves from under the first to under the
        // second.
        let mut r = Retained::from_view(vec![
            cont(1, None),
            text(3, Some(1), "a"),
            cont(2, None),
            text(4, Some(2), "b"),
        ]);
        assert!(r.apply(&[changed(text(3, Some(2), "a"), 0)]));
        // At index 0 of its new parent's children, before its new
        // siblings: still pre-ordered.
        assert_eq!(ids(&r), vec![1, 2, 3, 4]);
    }

    #[test]
    fn apply_inserts_a_new_node_at_index_zero() {
        let mut r = Retained::from_view(vec![
            cont(1, None),
            text(3, Some(1), "a"),
            cont(2, Some(1)),
            text(4, Some(2), "b"),
        ]);
        assert!(r.apply(&[changed(text(7, Some(1), "c"), 0)]));
        // Index 0 is the front: the leaf lands before the existing
        // children, and the walk steps over the container's subtree.
        assert_eq!(ids(&r), vec![1, 7, 3, 2, 4]);
    }

    #[test]
    fn apply_inserts_children_in_the_emitted_order() {
        // A batch of children emitted in pre-order must land in the
        // component's order, not reversed at the front of the parent.
        let mut r = Retained::from_view(vec![
            cont(1, None),
            text(3, Some(1), "a"),
            button(4, Some(1), "b"),
        ]);
        assert!(r.apply(&[
            changed(cont(5, Some(1)), 0),
            changed(text(6, Some(5), "x"), 0),
            changed(text(7, Some(5), "y"), 1),
            changed(button(8, Some(5), "z"), 2),
        ]));
        assert_eq!(ids(&r), vec![1, 5, 6, 7, 8, 3, 4]);
    }

    #[test]
    fn apply_inserts_at_a_middle_index_and_appends() {
        let mut r = Retained::from_view(vec![
            cont(1, None),
            text(3, Some(1), "a"),
            button(4, Some(1), "b"),
        ]);
        assert!(r.apply(&[
            changed(text(7, Some(1), "c"), 1),
            changed(text(8, Some(1), "d"), 3),
        ]));
        // 7 before the second child; 8 past the end (appended).
        assert_eq!(ids(&r), vec![1, 3, 7, 4, 8]);
    }

    #[test]
    fn a_mutated_tree_still_builds() {
        // End-to-end over the machinery: mutate, then rebuild — the
        // rebuilt tree must reflect the diff.
        let mut r = Retained::from_view(vec![
            cont(1, None),
            text(3, Some(1), "0"),
            button(2, Some(1), "+"),
            button(4, Some(1), "-"),
        ]);
        assert!(r.apply(&[changed(text(3, Some(1), "-1"), 0)]));
        let tree = build_tree(r.nodes());
        if let Tree::Column(_, children) = &tree {
            // List order: the readout first, then the two buttons.
            assert_eq!(children.len(), 3);
            assert!(matches!(&children[0], Tree::Text(c, _) if c.as_str() == "-1"));
        } else {
            panic!("expected a column of three children");
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
            }),
            text(4, Some(3), "a"),
        ]);
        match &tree {
            Tree::Column(Align::Center, children) => match &children[0] {
                Tree::Column(Align::Start, kids) => assert_eq!(kids.len(), 1),
                _ => panic!("the wrapper should keep its start align"),
            },
            _ => panic!("expected the centered root column"),
        }
    }
}
