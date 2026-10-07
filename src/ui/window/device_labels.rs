// SPDX-License-Identifier: MIT

use crate::ui::preferences::PreferenceManager;
use gtk::prelude::*;
use std::rc::Rc;

pub(super) fn bind_row_label(
    row: &gtk::Button,
    preferences: &Rc<PreferenceManager>,
    id: &str,
    system_name: &str,
) {
    let id = id.to_owned();
    let system_name = system_name.to_owned();
    preferences.bind_preference(
        row,
        move |manager| manager.device_label(&id),
        move |widget, label| {
            let row = widget.downcast_ref::<gtk::Button>().expect("device row");
            let label_widget = row
                .child()
                .and_then(|content| content.last_child())
                .and_downcast::<gtk::Label>()
                .expect("device row label");
            let display_name = label.as_deref().unwrap_or(&system_name);
            label_widget.set_text(display_name);
            row.update_property(&[gtk::accessible::Property::Label(display_name)]);
        },
    );
}

#[cfg(test)]
mod tests;
