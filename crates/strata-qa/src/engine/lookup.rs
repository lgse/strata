// SPDX-License-Identifier: MIT

//! Walks over a backend's nodes, and the lookups that go by role and name.

use crate::{Backend, LookupError};

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

/// The rows of the search results, or of the whole of `root` when no results are shown.
pub fn result_rows<B: Backend + ?Sized>(backend: &B, root: &B::Node) -> Vec<B::Node> {
    let everything = presented(backend, root);
    let (container, row_role) =
        search_results(backend, &everything, &backend.labels().search_results)
            .unwrap_or_else(|| (root.clone(), "list item"));
    presented(backend, &container)
        .into_iter()
        .filter(|row| backend.role(row) == row_role && rendered(backend, row))
        .collect()
}

/// Browser windows are titled "<folder> — Strata", so "Strata" matches every one.
fn titled<B: Backend + ?Sized>(backend: &B, window: &B::Node, title: &str) -> bool {
    backend.title(window).is_some_and(|actual| {
        actual == title
            || actual
                .strip_suffix(title)
                .is_some_and(|folder| folder.ends_with(" — "))
    })
}

/// The drawn windows with the title.
pub fn windows_titled<B: Backend + ?Sized>(backend: &B, title: &str) -> Vec<B::Node> {
    backend
        .toplevels()
        .into_iter()
        .filter(|window| titled(backend, window, title) && rendered(backend, window))
        .collect()
}

pub(crate) fn other_window<B: Backend + ?Sized>(
    backend: &B,
    main: &B::Node,
    title: &str,
) -> Option<B::Node> {
    windows_titled(backend, title)
        .into_iter()
        .find(|window| window != main)
}

pub(crate) fn find<B: Backend + ?Sized>(
    backend: &B,
    root: &B::Node,
    role: &str,
    name: &str,
) -> Result<B::Node, LookupError> {
    first_rendered(backend, root, |node| {
        backend.role(node) == role && backend.name(node) == name
    })
    .ok_or_else(|| missing(format!("no {role} named {name:?} on screen")))
}

pub(crate) fn first_rendered<B: Backend + ?Sized>(
    backend: &B,
    root: &B::Node,
    matches: impl Fn(&B::Node) -> bool,
) -> Option<B::Node> {
    presented(backend, root)
        .into_iter()
        .find(|node| matches(node) && rendered(backend, node))
}

/// The first presented node with the role and name, drawn or not.
pub fn find_presented<B: Backend + ?Sized>(
    backend: &B,
    root: &B::Node,
    role: &str,
    name: &str,
) -> Option<B::Node> {
    presented(backend, root)
        .into_iter()
        .find(|node| backend.role(node) == role && backend.name(node) == name)
}

pub(crate) fn search_result<B: Backend + ?Sized>(
    backend: &B,
    root: &B::Node,
    path: &str,
) -> Option<B::Node> {
    let everything = presented(backend, root);
    let (container, row_role) =
        search_results(backend, &everything, &backend.labels().search_results)
            .unwrap_or_else(|| (root.clone(), "list item"));
    let file_name = std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    presented(backend, &container).into_iter().find(|row| {
        backend.role(row) == row_role
            && rendered(backend, row)
            && backend.name(row) == file_name
            && presented(backend, row).iter().any(|label| {
                backend.role(label) == "label"
                    && rendered(backend, label)
                    && backend.name(label).ends_with(path)
            })
    })
}

pub(crate) fn missing(message: impl Into<String>) -> LookupError {
    LookupError::Missing(message.into())
}
