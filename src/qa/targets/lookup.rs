// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::{super::state, LookupError, accessible_name, missing, role_name};

pub(crate) fn result_rows(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let everything = presented(root);
    let (container, row_role) =
        search_results(&everything).unwrap_or_else(|| (root.clone(), "list item"));
    presented(&container)
        .into_iter()
        .filter(|row| role_name(row) == row_role && state::rendered_bounds(row).is_some())
        .collect()
}

/// Browser windows are titled "<folder> — Strata", so "Strata" matches every one.
fn titled(window: &gtk::Window, title: &str) -> bool {
    window.title().is_some_and(|actual| {
        actual == title
            || actual
                .strip_suffix(title)
                .is_some_and(|folder| folder.ends_with(" — "))
    })
}

pub(crate) fn windows_titled(title: &str) -> Vec<gtk::Window> {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|window| window.downcast::<gtk::Window>().ok())
        .filter(|window| {
            titled(window, title) && state::rendered_bounds(window.upcast_ref()).is_some()
        })
        .collect()
}

pub(crate) fn window_titled(title: &str, but: Option<&gtk::Window>) -> Option<gtk::Window> {
    windows_titled(title)
        .into_iter()
        .find(|window| Some(window) != but)
}

pub(super) fn toplevel(title: &str) -> Option<gtk::Widget> {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|window| window.downcast::<gtk::Window>().ok())
        .find(|window| {
            titled(window, title) && state::rendered_bounds(window.upcast_ref()).is_some()
        })
        .map(Cast::upcast)
}

pub(super) fn other_window(main: &gtk::Window, title: &str) -> Option<gtk::Widget> {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|window| window.downcast::<gtk::Window>().ok())
        .find(|window| {
            window != main
                && titled(window, title)
                && state::rendered_bounds(window.upcast_ref()).is_some()
        })
        .map(Cast::upcast)
}

pub(super) fn find(root: &gtk::Widget, role: &str, name: &str) -> Result<gtk::Widget, LookupError> {
    first_rendered(root, |widget| {
        role_name(widget) == role && accessible_name(widget) == name
    })
    .ok_or_else(|| missing(format!("no {role} named {name:?} on screen")))
}

pub(super) fn first_rendered(
    root: &gtk::Widget,
    matches: impl Fn(&gtk::Widget) -> bool,
) -> Option<gtk::Widget> {
    presented(root)
        .into_iter()
        .find(|widget| matches(widget) && state::rendered_bounds(widget).is_some())
}

pub(crate) fn find_presented(root: &gtk::Widget, role: &str, name: &str) -> Option<gtk::Widget> {
    presented(root)
        .into_iter()
        .find(|widget| role_name(widget) == role && accessible_name(widget) == name)
}

pub(crate) fn open_dialog(presented: &[gtk::Widget]) -> Option<gtk::Widget> {
    ["dialog", "alert"].into_iter().find_map(|role| {
        presented
            .iter()
            .find(|widget| role_name(widget) == role && state::rendered_bounds(widget).is_some())
            .cloned()
    })
}

pub(crate) fn search_results(presented: &[gtk::Widget]) -> Option<(gtk::Widget, &'static str)> {
    [("list", "list item"), ("table", "table cell")]
        .into_iter()
        .find_map(|(role, row_role)| {
            presented
                .iter()
                .find(|widget| {
                    role_name(widget) == role
                        && state::rendered_bounds(widget).is_some()
                        && accessible_name(widget)
                            == crate::i18n::tr(crate::ui::SEARCH_RESULTS_LABEL)
                })
                .map(|results| (results.clone(), row_role))
        })
}

pub(super) fn search_result(root: &gtk::Widget, path: &str) -> Option<gtk::Widget> {
    let everything = presented(root);
    let (container, row_role) =
        search_results(&everything).unwrap_or_else(|| (root.clone(), "list item"));
    let file_name = std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    presented(&container).into_iter().find(|row| {
        role_name(row) == row_role
            && state::rendered_bounds(row).is_some()
            && accessible_name(row) == file_name
            && presented(row).iter().any(|label| {
                role_name(label) == "label"
                    && state::rendered_bounds(label).is_some()
                    && accessible_name(label).ends_with(path)
            })
    })
}

pub(crate) fn presented(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(widget) = stack.pop() {
        if !widget.is_visible() {
            continue;
        }
        let mut children = Vec::new();
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            children.push(current);
        }
        stack.extend(children.into_iter().rev());
        found.push(widget);
    }
    found
}
