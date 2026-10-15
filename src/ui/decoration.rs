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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowControl {
    Minimize,
    Maximize,
    Close,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ControlLayout {
    pub(crate) start: Vec<WindowControl>,
    pub(crate) end: Vec<WindowControl>,
}

pub(crate) fn control_layout(layout: &str) -> ControlLayout {
    let (start, end) = layout.split_once(':').unwrap_or((layout, ""));
    let mut seen = Vec::new();
    let mut parse = |side: &str| {
        side.split(',')
            .filter_map(|name| match name {
                "minimize" => Some(WindowControl::Minimize),
                "maximize" => Some(WindowControl::Maximize),
                "close" => Some(WindowControl::Close),
                _ => None,
            })
            .filter(|control| {
                let new = !seen.contains(control);
                if new {
                    seen.push(*control);
                }
                new
            })
            .collect::<Vec<_>>()
    };
    let start = parse(start);
    let listed = parse(end);
    let mut end: Vec<_> = [
        WindowControl::Minimize,
        WindowControl::Maximize,
        WindowControl::Close,
    ]
    .into_iter()
    .filter(|control| !seen.contains(control))
    .collect();
    end.extend(listed);
    ControlLayout { start, end }
}

pub(crate) fn current_control_layout(widget: &impl IsA<gtk::Widget>) -> ControlLayout {
    settings_control_layout(&widget.as_ref().settings())
}

fn settings_control_layout(settings: &gtk::Settings) -> ControlLayout {
    control_layout(&settings.gtk_decoration_layout().unwrap_or_default())
}

pub(crate) fn follow_control_layout(
    window: &impl IsA<gtk::Window>,
    apply: impl Fn(&ControlLayout) + 'static,
) {
    let settings = window.as_ref().settings();
    apply(&settings_control_layout(&settings));
    let handler = settings.connect_gtk_decoration_layout_notify(move |settings| {
        apply(&settings_control_layout(settings));
    });
    let handler = std::cell::Cell::new(Some(handler));
    window.as_ref().connect_unrealize(move |_| {
        if let Some(handler) = handler.take() {
            settings.disconnect(handler);
        }
    });
}

pub(crate) fn place_controls<'a>(
    host: &gtk::Box,
    mut after: Option<gtk::Widget>,
    controls: impl IntoIterator<Item = &'a gtk::Button>,
) {
    for control in controls {
        if control.parent().as_ref() == Some(host.upcast_ref()) {
            host.reorder_child_after(control, after.as_ref());
        } else {
            if let Some(parent) = control.parent().and_downcast::<gtk::Box>() {
                parent.remove(control);
            }
            host.insert_child_after(control, after.as_ref());
        }
        after = Some(control.clone().upcast());
    }
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
