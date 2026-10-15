//! Rebuild a widget tree from the flat, depth-first pre-ordered list of
//! nodes exported by the component (the `waui:ui/widget` contract).
//!
//! This module duplicates a tiny bit of logic per host on purpose: the WIT
//! package is the shared artifact, and each host keeps its own rebuild.

use crate::waui::ui::widget::{Direction, Node};

/// A rebuilt widget tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Tree {
    Text(String),
    /// (widget id, label)
    Button(u32, String),
    Row(Vec<Tree>),
    Column(Vec<Tree>),
}

fn node_id(node: &Node) -> u32 {
    match node {
        Node::Text(t) => t.id,
        Node::Button(b) => b.id,
        Node::Container(c) => c.id,
    }
}

fn node_parent(node: &Node) -> Option<u32> {
    match node {
        Node::Text(t) => t.parent,
        Node::Button(b) => b.parent,
        Node::Container(c) => c.parent,
    }
}

fn to_tree(node: &Node) -> Tree {
    match node {
        Node::Text(t) => Tree::Text(t.content.clone()),
        Node::Button(b) => Tree::Button(b.id, b.label.clone()),
        Node::Container(c) => {
            if c.direction == Direction::Row {
                Tree::Row(Vec::new())
            } else {
                Tree::Column(Vec::new())
            }
        }
    }
}

/// Rebuild the tree from the flat node list.
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
        let kids =
            children_of.get(&node_id(node)).cloned().unwrap_or_default();
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
            Some(Tree::Row(children) | Tree::Column(children)) => {
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
        0 => Tree::Column(Vec::new()),
        1 => roots.pop().unwrap(),
        _ => Tree::Column(roots),
    }
}
