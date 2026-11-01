// SPDX-License-Identifier: MIT

use gtk::prelude::*;

pub(super) const PADDING: i32 = 16;
pub(super) const GAP: i32 = 12;
pub(super) const CONTROLS_GAP: i32 = 4;
pub(super) const PANEL_MIN: i32 = 200;

pub(super) fn label(class: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class(class);
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label
}

pub(super) struct DetailsHeader {
    pub(super) root: gtk::Box,
    pub(super) eyebrow: gtk::Label,
    pub(super) title: gtk::Label,
}

impl DetailsHeader {
    pub(super) fn new(volume: Option<&gtk::Widget>) -> Self {
        let eyebrow = label("preview-details-eyebrow");
        let title = label("preview-details-title");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 2);
        if let Some(volume) = volume {
            eyebrow.set_hexpand(true);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.append(&eyebrow);
            row.append(volume);
            root.append(&row);
        } else {
            root.append(&eyebrow);
        }
        root.append(&title);
        Self {
            root,
            eyebrow,
            title,
        }
    }
}

/// `tooltip` names `binding`, when given, with `%{keys}`.
pub(super) fn navigation_button(
    icon: &str,
    name: &str,
    tooltip: &'static str,
    binding: Option<crate::ui::keys::Command>,
) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("preview-media-button");
    button.set_child(Some(&crate::assets::primary_icon(icon, 18)));
    match binding {
        Some(binding) => crate::ui::shortcut_reference::follow_tooltip(
            &button,
            tooltip,
            crate::ui::keys::Keys::One(binding.shown()),
        ),
        None => button.set_tooltip_text(Some(&crate::i18n::tr(tooltip))),
    }
    crate::ui::accessibility::set_label(&button, &crate::i18n::tr(name));
    button
}

pub(super) fn navigation_row(
    previous: &gtk::Button,
    center: &impl IsA<gtk::Widget>,
    next: &gtk::Button,
    spacing: i32,
) -> gtk::CenterBox {
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, spacing);
    buttons.set_valign(gtk::Align::Center);
    buttons.append(previous);
    buttons.append(center);
    buttons.append(next);
    let row = gtk::CenterBox::new();
    row.set_center_widget(Some(&buttons));
    row
}

pub(super) fn badge_row() -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("preview-details-badges");
    row
}

pub(super) fn badge(text: &str) -> gtk::Label {
    let badge = gtk::Label::new(Some(text));
    badge.add_css_class("preview-details-badge");
    badge
}
