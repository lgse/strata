// SPDX-License-Identifier: MIT

use super::lookup::{presented, rendered};
use crate::{Backend, FieldRef, LookupError, Memory};

/// An editable text field: a `text` node that reports field content and takes edits.
pub(crate) fn is_field<B: Backend + ?Sized>(backend: &B, node: &B::Node) -> bool {
    backend.role(node) == "text" && backend.states(node).editable && backend.field(node).is_some()
}

/// How many editable fields the window draws.
pub fn editable_fields<B: Backend + ?Sized>(backend: &B, window: &B::Node) -> usize {
    presented(backend, window)
        .iter()
        .filter(|node| is_field(backend, node) && rendered(backend, node))
        .count()
}

pub(crate) fn editable_texts<B: Backend + ?Sized>(backend: &B, window: &B::Node) -> Vec<String> {
    presented(backend, window)
        .iter()
        .filter(|node| is_field(backend, node) && rendered(backend, node))
        .filter_map(|node| backend.text(node))
        .collect()
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

/// The remembered field, the drawn one with the name, or the focused one.
pub(crate) fn field<B: Backend>(
    backend: &B,
    window: &B::Node,
    memory: &Memory<B>,
    field: &FieldRef,
) -> Result<Option<B::Node>, LookupError> {
    if let Some(remembered) = &field.of {
        return memory
            .get(remembered)
            .map(|kept| backend.recall(kept))
            .ok_or_else(|| {
                LookupError::Broken(format!("no field was remembered as {remembered:?}"))
            });
    }
    let everything = presented(backend, window);
    Ok(match &field.name {
        Some(name) => everything.into_iter().find(|node| {
            is_field(backend, node) && rendered(backend, node) && backend.name(node) == *name
        }),
        None => focused_field(backend, &everything),
    })
}
