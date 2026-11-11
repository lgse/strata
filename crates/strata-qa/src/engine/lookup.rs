// SPDX-License-Identifier: MIT

//! Walks over a backend's nodes, and the lookups that go by role and name.

use crate::Backend;

/// A node and everything below it that the application has not hidden, parents first.
pub fn presented<B: Backend + ?Sized>(backend: &B, root: &B::Node) -> Vec<B::Node> {
    let mut found = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(node) = stack.pop() {
        if !backend.visible(&node) {
            continue;
        }
        stack.extend(backend.children(&node).into_iter().rev());
        found.push(node);
    }
    found
}

/// A node and everything below it, hidden or not, parents first.
pub fn descendants<B: Backend + ?Sized>(backend: &B, root: &B::Node) -> Vec<B::Node> {
    let mut found = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(node) = stack.pop() {
        stack.extend(backend.children(&node).into_iter().rev());
        found.push(node);
    }
    found
}

/// Whether the node is drawn.
pub fn rendered<B: Backend + ?Sized>(backend: &B, node: &B::Node) -> bool {
    backend.frame(node).is_some()
}

/// Whether `ancestor` is somewhere above `node`.
pub(crate) fn is_ancestor<B: Backend + ?Sized>(
    backend: &B,
    ancestor: &B::Node,
    node: &B::Node,
) -> bool {
    std::iter::successors(backend.parent(node), |above| backend.parent(above))
        .any(|above| above == *ancestor)
}

pub(crate) fn rendered_with_role<'a, B: Backend + ?Sized>(
    backend: &'a B,
    presented: &'a [B::Node],
    role: &'a str,
) -> impl Iterator<Item = &'a B::Node> {
    presented
        .iter()
        .filter(move |node| backend.role(node) == role && rendered(backend, node))
}

pub(crate) fn open_dialog<B: Backend + ?Sized>(
    backend: &B,
    presented: &[B::Node],
) -> Option<B::Node> {
    ["dialog", "alert"]
        .into_iter()
        .find_map(|role| rendered_with_role(backend, presented, role).next().cloned())
}

/// The list or table of search results, and the role its rows have.
pub(crate) fn search_results<B: Backend + ?Sized>(
    backend: &B,
    presented: &[B::Node],
    label: &str,
) -> Option<(B::Node, &'static str)> {
    [("list", "list item"), ("table", "table cell")]
        .into_iter()
        .find_map(|(role, row_role)| {
            presented
                .iter()
                .find(|node| {
                    backend.role(node) == role
                        && rendered(backend, node)
                        && backend.name(node) == label
                })
                .map(|results| (results.clone(), row_role))
        })
}
