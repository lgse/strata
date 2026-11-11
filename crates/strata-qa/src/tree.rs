// SPDX-License-Identifier: MIT

//! Walks over a backend's nodes.

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
