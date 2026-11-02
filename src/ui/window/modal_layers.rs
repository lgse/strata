// SPDX-License-Identifier: MIT

//! Finds the modal layer that owns input in a window.

use gtk::prelude::*;

type Skip<'a> = &'a dyn Fn(&gtk::Widget) -> bool;

pub(in crate::ui) fn visible_modal_layer(window: &impl IsA<gtk::Window>) -> Option<gtk::Widget> {
    topmost_modal_layer(window, &|_| false)
}

pub(super) fn topmost_modal_layer(
    window: &impl IsA<gtk::Window>,
    skip: Skip<'_>,
) -> Option<gtk::Widget> {
    let root = window.child()?;
    if root.has_css_class("tab-window") {
        visible_layer(&root, skip)
    } else {
        children_last_first(&root).find(|widget| is_unskipped_visible_layer(widget, skip))
    }
}

fn is_modal_layer(widget: &gtk::Widget) -> bool {
    widget.has_css_class("app-modal-layer")
}

fn is_unskipped_visible_layer(widget: &gtk::Widget, skip: Skip<'_>) -> bool {
    widget.is_visible() && is_modal_layer(widget) && !skip(widget)
}

/// Hidden or faded-out subtrees, and the tab strip, never own input.
fn subtree_is_shown(root: &gtk::Widget) -> bool {
    root.is_visible()
        && root.is_child_visible()
        && root.opacity() != 0.0
        && !root.has_css_class("tab-strip")
}

/// A tab's context only contributes its own modal layers, not its content.
fn walks_into(root: &gtk::Widget, child: &gtk::Widget) -> bool {
    !root.has_css_class("tab-context") || is_modal_layer(child)
}

fn children_last_first(root: &gtk::Widget) -> impl Iterator<Item = gtk::Widget> {
    std::iter::successors(root.last_child(), gtk::Widget::prev_sibling)
}

fn visible_layer(root: &gtk::Widget, skip: Skip<'_>) -> Option<gtk::Widget> {
    if !subtree_is_shown(root) {
        return None;
    }
    if is_modal_layer(root) {
        return (!skip(root)).then(|| root.clone());
    }
    children_last_first(root)
        .filter(|widget| walks_into(root, widget))
        .find_map(|widget| visible_layer(&widget, skip))
}

/// True when the topmost visible modal layer is not `own`. An action that opens a
/// modal passes its own layer so it can still toggle that layer closed. A layer that
/// is animating out no longer owns input.
pub(in crate::ui) fn foreign_modal_visible(
    window: &impl IsA<gtk::Window>,
    own: Option<&gtk::Widget>,
) -> bool {
    topmost_modal_layer(window, &crate::ui::modal::is_closing_layer)
        .is_some_and(|layer| own != Some(&layer))
}

pub(in crate::ui) fn install_modal_focus_trap(window: &impl IsA<gtk::Window>) {
    window.connect_focus_widget_notify(|window| {
        let Some(layer) = visible_modal_layer(window) else {
            return;
        };
        let focus_is_inside = gtk::prelude::RootExt::focus(window.as_ref())
            .is_some_and(|focus| focus == layer || focus.is_ancestor(&layer));
        if !focus_is_inside {
            layer.grab_focus();
        }
    });
}
