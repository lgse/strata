// SPDX-License-Identifier: MIT

use std::cell::Cell;

use gtk::prelude::*;

thread_local! {
    pub(super) static SETUP_RUNNING: Cell<bool> = const { Cell::new(false) };
}

/// Replaces a Settings integration row in builds whose package ships the integration.
pub(super) fn package_provided_row(title: &str, note: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
    row.add_css_class("settings-option");
    let title = gtk::Label::new(Some(&crate::i18n::tr(title)));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let note = gtk::Label::new(Some(&crate::i18n::tr(note)));
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.add_css_class("settings-option-description");
    row.append(&title);
    row.append(&note);
    row
}

pub(crate) fn set_search_available(widget: &impl IsA<gtk::Widget>, available: bool) {
    if available {
        widget.remove_css_class("settings-search-unavailable");
    } else {
        widget.add_css_class("settings-search-unavailable");
    }
    widget.set_visible(available);
}
