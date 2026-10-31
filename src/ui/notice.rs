// SPDX-License-Identifier: MIT
use gtk::{glib, prelude::*};
use std::time::Duration;

const NOTICE_CLASS: &str = "overlay-notice";

/// Shows a brief non-interactive notice in the window's progress dock, or in
/// `overlay` when it is not in a window, replacing any notice still visible.
pub(super) fn show_notice(
    overlay: &gtk::Overlay,
    heading: Option<&str>,
    text: &str,
    duration: Duration,
) {
    show(overlay, true, true, heading, text, duration);
}

pub(super) fn add_notice(
    overlay: &gtk::Overlay,
    heading: Option<&str>,
    text: &str,
    duration: Duration,
) {
    show(overlay, true, false, heading, text, duration);
}

/// A notice without the success check, for reporting a value as it changes.
pub(super) fn show_status(overlay: &gtk::Overlay, text: &str, duration: Duration) {
    show(overlay, false, true, None, text, duration);
}

fn show(
    overlay: &gtk::Overlay,
    check: bool,
    replace: bool,
    heading: Option<&str>,
    text: &str,
    duration: Duration,
) {
    let overlay = &super::modal::window_overlay(overlay).unwrap_or_else(|| overlay.clone());
    let (dock, list) = super::progress_dock::dock(overlay);
    if replace {
        let mut child = list.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if widget.has_css_class(NOTICE_CLASS) {
                list.remove(&widget);
            }
        }
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class(NOTICE_CLASS);
    row.set_can_target(false);
    if check {
        let icon = crate::assets::primary_icon(crate::assets::icons::CHECK, 16);
        icon.set_valign(gtk::Align::Start);
        icon.set_can_target(false);
        row.append(&icon);
    } else {
        row.add_css_class("status");
    }
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    if let Some(heading) = heading {
        let heading = gtk::Label::new(Some(heading));
        heading.add_css_class("overlay-notice-heading");
        heading.set_xalign(0.0);
        column.append(&heading);
    }
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_max_width_chars(60);
    label.set_lines(3);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    column.append(&label);
    column.set_can_target(false);
    row.append(&column);
    list.append(&row);
    let weak_overlay = overlay.downgrade();
    let (weak_dock, weak_list, weak_row) = (dock.downgrade(), list.downgrade(), row.downgrade());
    glib::timeout_add_local_once(duration, move || {
        let (Some(overlay), Some(dock), Some(list), Some(row)) = (
            weak_overlay.upgrade(),
            weak_dock.upgrade(),
            weak_list.upgrade(),
            weak_row.upgrade(),
        ) else {
            return;
        };
        if row.parent().as_ref() == Some(list.upcast_ref()) {
            list.remove(&row);
        }
        if list.first_child().is_none() && dock.parent().is_some() {
            overlay.remove_overlay(&dock);
        }
    });
}
