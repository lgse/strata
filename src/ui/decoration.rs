// SPDX-License-Identifier: MIT

use gtk::prelude::*;

/// Strata's header stays inside the content so overlays and the modal blur
/// cover it, but GTK adds its own default title bar on compositors that leave
/// decorations to the client unless a titlebar is registered. A hidden
/// placeholder claims that slot; GTK's client-side decoration then supplies
/// only the invisible resize edges, and `style.css` draws no window chrome.
pub(crate) fn claim_titlebar(window: &impl IsA<gtk::Window>) {
    let placeholder = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    placeholder.set_visible(false);
    window.set_titlebar(Some(&placeholder));
}

/// GTK only builds its default title bar at realize time for windows it does
/// not already consider client-decorated.
#[cfg(test)]
pub(crate) fn assert_default_decoration_absent(window: &impl IsA<gtk::Window>) {
    let window = window.as_ref();
    assert!(window.is_realized(), "window must be realized");
    assert!(
        window.has_css_class("csd") || window.has_css_class("solid-csd"),
        "GTK must treat the window as client-decorated"
    );
    let titlebar = window.titlebar().expect("registered titlebar");
    assert!(
        !titlebar.is_visible(),
        "the placeholder titlebar takes no space"
    );
    let mut child = window.first_child();
    while let Some(widget) = child {
        assert!(
            !widget.has_css_class("default-decoration"),
            "GTK added its default title bar"
        );
        child = widget.next_sibling();
    }
}

#[cfg(test)]
mod tests;
