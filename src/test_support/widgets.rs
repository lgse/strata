// SPDX-License-Identifier: MIT

use gtk::gdk::{Key, ModifierType};
use gtk::prelude::*;

/// The rendered pane-header Back button; a view-mode switch leaves the old
/// header in the tree until it rebuilds.
pub(crate) fn mapped_back_button(widget: &gtk::Widget) -> Option<gtk::Widget> {
    if !widget.is_mapped() {
        return None;
    }
    if widget.has_css_class("list-navigation") {
        return widget
            .first_child()
            .filter(|back| back.is_mapped() && back.width() > 0);
    }
    std::iter::successors(widget.first_child(), gtk::Widget::next_sibling)
        .find_map(|child| mapped_back_button(&child))
}

/// Presses a key on `widget`'s own key controllers, as GTK does once the
/// window's capture-phase controller declines it.
pub(crate) fn press_on_widget(widget: &gtk::Widget, key: Key, modifiers: ModifierType) {
    let controllers = widget.observe_controllers();
    for index in 0..controllers.n_items() {
        if let Some(keys) = controllers
            .item(index)
            .and_downcast::<gtk::EventControllerKey>()
        {
            keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers]);
        }
    }
}
