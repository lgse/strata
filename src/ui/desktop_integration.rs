// SPDX-License-Identifier: MIT

use std::cell::Cell;

use gtk::prelude::*;

thread_local! {
    pub(super) static SETUP_RUNNING: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn set_search_available(widget: &impl IsA<gtk::Widget>, available: bool) {
    if available {
        widget.remove_css_class("settings-search-unavailable");
    } else {
        widget.add_css_class("settings-search-unavailable");
    }
    widget.set_visible(available);
}
