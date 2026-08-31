// SPDX-License-Identifier: GPL-3.0-or-later

use gtk::prelude::*;

/// Builds a single-selection group with the same compact treatment as a segmented control.
pub(super) fn segmented_control(
    labels: &[&str],
    selected: usize,
) -> (gtk::Box, Vec<gtk::ToggleButton>) {
    let control = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    control.add_css_class("segmented-control");

    let mut buttons = Vec::with_capacity(labels.len());
    for (index, label) in labels.iter().enumerate() {
        let button = gtk::ToggleButton::with_label(label);
        button.add_css_class("segmented-control-option");
        button.set_hexpand(true);
        if index == 0 {
            button.add_css_class("first");
        } else {
            button.add_css_class("not-first");
        }
        if index + 1 == labels.len() {
            button.add_css_class("last");
        }
        if let Some(first) = buttons.first() {
            button.set_group(Some(first));
        }
        button.set_active(index == selected);
        control.append(&button);
        buttons.push(button);
    }

    (control, buttons)
}
