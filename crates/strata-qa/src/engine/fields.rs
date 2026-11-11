// SPDX-License-Identifier: MIT

use super::lookup::rendered;
use crate::Backend;

/// An editable text field: a `text` node that reports field content and takes edits.
pub(crate) fn is_field<B: Backend + ?Sized>(backend: &B, node: &B::Node) -> bool {
    backend.role(node) == "text" && backend.states(node).editable && backend.field(node).is_some()
}

pub(crate) fn focused_field<B: Backend + ?Sized>(
    backend: &B,
    presented: &[B::Node],
) -> Option<B::Node> {
    presented
        .iter()
        .find(|node| {
            is_field(backend, node) && backend.states(node).focused && rendered(backend, node)
        })
        .cloned()
}
