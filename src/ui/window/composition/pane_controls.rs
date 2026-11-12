// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::prelude::*;

use super::{PaneCommand, WindowContent};
use crate::{
    assets,
    ui::{accessibility, keys},
};

pub(super) fn toggle() -> gtk::ToggleButton {
    let label = crate::i18n::tr("Show or hide the extra pane");
    let button = gtk::ToggleButton::new();
    button.set_child(Some(&assets::chrome_icon(assets::icons::COLUMNS)));
    button.add_css_class("header-action");
    button.set_cursor_from_name(Some("pointer"));
    button.set_tooltip_text(Some(&label));
    accessibility::set_label(&button, &label);
    accessibility::set_description(
        &button,
        Some(&keys::label(&keys::Keys::One(
            keys::bindings::EXTRA_PANE.shown(),
        ))),
    );
    button
}

pub(super) fn bind(content: &Rc<WindowContent>) {
    let weak = Rc::downgrade(content);
    content.header.extra_pane.connect_clicked(move |_| {
        if let Some(content) = weak.upgrade() {
            content.run_pane_command(PaneCommand::Toggle);
        }
    });
}
