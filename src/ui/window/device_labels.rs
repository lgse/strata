// SPDX-License-Identifier: MIT

use crate::ui::preferences::PreferenceManager;
use gtk::prelude::*;
use std::rc::Rc;

pub(super) fn display_name(
    preferences: &PreferenceManager,
    id: Option<&str>,
    system_name: &str,
) -> String {
    id.and_then(|id| preferences.device_label(id))
        .unwrap_or_else(|| system_name.to_owned())
}

pub(super) fn bind_row_label(
    row: &gtk::Button,
    preferences: &Rc<PreferenceManager>,
    id: &str,
    system_name: &str,
) {
    bind_display_name(row, preferences, id, system_name, |widget, display_name| {
        let row = widget.downcast_ref::<gtk::Button>().expect("device row");
        let label_widget = row
            .child()
            .and_then(|content| content.last_child())
            .and_downcast::<gtk::Label>()
            .expect("device row label");
        label_widget.set_text(display_name);
        crate::ui::accessibility::set_own_label(row, display_name);
    });
}

pub(super) fn bind_label_text(
    label: &gtk::Label,
    preferences: &Rc<PreferenceManager>,
    id: &str,
    system_name: &str,
) {
    bind_display_name(
        label,
        preferences,
        id,
        system_name,
        |widget, display_name| {
            widget
                .downcast_ref::<gtk::Label>()
                .expect("device name label")
                .set_text(display_name);
        },
    );
}

fn bind_display_name(
    widget: &impl IsA<gtk::Widget>,
    preferences: &Rc<PreferenceManager>,
    id: &str,
    system_name: &str,
    apply: fn(&gtk::Widget, &str),
) {
    let id = id.to_owned();
    let system_name = system_name.to_owned();
    preferences.bind_preference(
        widget,
        move |manager| manager.device_label(&id),
        move |widget, label| apply(widget, label.as_deref().unwrap_or(&system_name)),
    );
}

#[cfg(test)]
mod tests;
