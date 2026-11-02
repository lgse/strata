// SPDX-License-Identifier: MIT

//! The editor's icon picker: a row of common icons and a popover with the rest.

use std::{cell::RefCell, rc::Rc};

use gtk::{glib, prelude::*};

use super::label_control;
use crate::ui::actions::{ACTION_ICON_CHOICES, action_icon, is_known_action_icon};

const QUICK_ICONS: [&str; 8] = [
    "play",
    "terminal",
    "file-code",
    "scissors",
    "copy",
    "file-archive",
    "image",
    "refresh",
];

pub(super) fn icon_chooser(selected: &Rc<RefCell<Option<String>>>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let (more, grid, popover) = more_icons_button();
    let mut first: Option<gtk::ToggleButton> = None;
    let current = selected.borrow().clone();
    let current = current
        .as_deref()
        .filter(|slug| is_known_action_icon(slug))
        .unwrap_or("play");
    let mut remaining = 0;
    for slug in QUICK_ICONS.iter().copied().chain(
        ACTION_ICON_CHOICES
            .iter()
            .map(|(slug, _)| *slug)
            .filter(|slug| !QUICK_ICONS.contains(slug)),
    ) {
        let button = icon_choice(slug);
        if let Some(first) = first.as_ref() {
            button.set_group(Some(first));
        } else {
            first = Some(button.clone());
        }
        button.set_active(current == slug);
        let is_more = !QUICK_ICONS.contains(&slug);
        if is_more {
            grid.attach(&button, remaining % 4, remaining / 4, 1, 1);
            remaining += 1;
        } else {
            row.append(&button);
        }
        let choice = IconChoice {
            slug,
            is_more,
            selected: selected.clone(),
            more: more.downgrade(),
            popover: popover.downgrade(),
        };
        button.connect_toggled(move |button| {
            if button.is_active() {
                choice.choose();
            }
        });
    }
    if !QUICK_ICONS.contains(&current) {
        more.set_child(Some(&picker_icon(action_icon(Some(current)))));
        more.add_css_class("selected");
        more.set_tooltip_text(Some(&action_icon_name(current)));
    }
    row.append(&more);
    row
}

fn more_icons_button() -> (gtk::MenuButton, gtk::Grid, gtk::Popover) {
    let more = gtk::MenuButton::new();
    more.set_child(Some(&picker_icon(crate::assets::icons::PLUS)));
    more.add_css_class("settings-action-more-icons");
    more.set_tooltip_text(Some(&crate::i18n::tr("More icons")));
    label_control(&more, "More icons", "Choose another bundled Lucide icon");
    let grid = gtk::Grid::builder()
        .row_spacing(6)
        .column_spacing(6)
        .build();
    let popover = gtk::Popover::builder()
        .has_arrow(false)
        .child(&grid)
        .build();
    popover.add_css_class("column-popover");
    popover.add_css_class("settings-action-icon-popover");
    more.set_popover(Some(&popover));
    (more, grid, popover)
}

fn icon_choice(slug: &'static str) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::new();
    button.add_css_class("settings-action-icon-choice");
    button.set_child(Some(&picker_icon(action_icon(Some(slug)))));
    let icon_name = action_icon_name(slug);
    button.set_tooltip_text(Some(&icon_name));
    crate::ui::accessibility::set_own_label(
        &button,
        &rust_i18n::t!("%{icon} icon", icon = icon_name),
    );
    button
}

/// One icon button and the "more icons" button that shows a popover choice.
struct IconChoice {
    slug: &'static str,
    is_more: bool,
    selected: Rc<RefCell<Option<String>>>,
    more: glib::WeakRef<gtk::MenuButton>,
    popover: glib::WeakRef<gtk::Popover>,
}

impl IconChoice {
    fn choose(&self) {
        self.selected.replace(Some(self.slug.to_owned()));
        if let Some(more) = self.more.upgrade() {
            self.show_on(&more);
        }
        if let Some(popover) = self.popover.upgrade() {
            popover.popdown();
        }
    }

    /// The "more icons" button shows a popover choice in place of its plus.
    fn show_on(&self, more: &gtk::MenuButton) {
        let slug = self.slug;
        more.set_child(Some(&picker_icon(if self.is_more {
            action_icon(Some(slug))
        } else {
            crate::assets::icons::PLUS
        })));
        more.set_tooltip_text(Some(&if self.is_more {
            action_icon_name(slug)
        } else {
            crate::i18n::tr("More icons")
        }));
        if self.is_more {
            more.add_css_class("selected");
        } else {
            more.remove_css_class("selected");
        }
    }
}

fn picker_icon(name: &str) -> gtk::Image {
    let icon = crate::assets::primary_icon(name, 18);
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    icon
}

/// Semantic keys; a missing entry falls back to the slug instead of the key.
pub(super) fn action_icon_name(slug: &str) -> String {
    let key = format!("action_icon.{slug}");
    let name = crate::i18n::tr(&key);
    if name == key { slug.to_owned() } else { name }
}
