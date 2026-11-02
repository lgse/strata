// SPDX-License-Identifier: MIT

use super::CustomizedItem;
use crate::ui::preferences::PreferenceManager;
use gtk::prelude::*;
use std::rc::Rc;

/// The bundled icon grid plus the emoji picker of the Customize dialog.
pub(super) struct IconPicker {
    pub(super) grid: gtk::FlowBox,
    pub(super) choices: IconChoices,
    pub(super) emoji_chooser: gtk::EmojiChooser,
}

/// The selectable widgets; handlers hold these but never the chooser popover.
#[derive(Clone)]
pub(super) struct IconChoices {
    buttons: Rc<Vec<(&'static str, gtk::Button)>>,
    pub(super) emoji_button: gtk::Button,
}

impl IconPicker {
    pub(super) fn build(initial_icon: Option<&str>) -> Self {
        let grid = build_icon_grid();
        let buttons = Rc::new(build_icon_buttons(&grid, initial_icon));
        let selected_emoji = initial_icon.and_then(crate::assets::icons::custom_emoji);
        let emoji_button = build_emoji_button(selected_emoji);
        let emoji_chooser = gtk::EmojiChooser::new();
        emoji_chooser.add_css_class("customize-emoji-chooser");
        emoji_chooser.set_parent(&emoji_button);
        let chooser_for_button = emoji_chooser.clone();
        emoji_button.connect_clicked(move |_| chooser_for_button.popup());
        Self {
            grid,
            choices: IconChoices {
                buttons,
                emoji_button,
            },
            emoji_chooser,
        }
    }

    pub(super) fn connect_icon_chosen(&self, item: &Rc<CustomizedItem>) {
        for (icon_name, button) in self.choices.buttons.iter() {
            let selected_name = *icon_name;
            let choices = self.choices.clone();
            let item = item.clone();
            button.connect_clicked(move |_| {
                choices.show_icon_active(Some(selected_name));
                PreferenceManager::shared().set_custom_icon(&item.path, Some(selected_name));
                item.note_customized();
            });
        }

        let choices = self.choices.clone();
        let item = item.clone();
        self.emoji_chooser
            .connect_emoji_picked(move |chooser, emoji| {
                let preference = format!("emoji:{emoji}");
                choices.mark_buttons_active(None);
                choices
                    .emoji_button
                    .set_label(&rust_i18n::t!("Emoji  %{emoji}", emoji = emoji));
                PreferenceManager::shared().set_custom_icon(&item.path, Some(&preference));
                item.note_customized();
                chooser.popdown();
            });
    }
}

impl IconChoices {
    pub(super) fn show_icon_active(&self, selected_name: Option<&'static str>) {
        self.mark_buttons_active(selected_name);
        self.emoji_button
            .set_label(&crate::i18n::tr("Choose Emoji…"));
    }

    fn mark_buttons_active(&self, selected_name: Option<&'static str>) {
        for (name, button) in self.buttons.iter() {
            if Some(*name) == selected_name {
                button.add_css_class("active");
            } else {
                button.remove_css_class("active");
            }
        }
    }
}

fn build_icon_grid() -> gtk::FlowBox {
    let icon_grid = gtk::FlowBox::new();
    icon_grid.add_css_class("customize-icon-grid");
    icon_grid.set_selection_mode(gtk::SelectionMode::None);
    icon_grid.set_homogeneous(true);
    icon_grid.set_min_children_per_line(4);
    icon_grid.set_max_children_per_line(8);
    icon_grid.set_row_spacing(6);
    icon_grid.set_column_spacing(6);
    icon_grid
}

fn build_icon_buttons(
    icon_grid: &gtk::FlowBox,
    initial_icon: Option<&str>,
) -> Vec<(&'static str, gtk::Button)> {
    crate::assets::icons::CUSTOMIZATION_CHOICES
        .iter()
        .map(|&(icon_name, label)| {
            let button = gtk::Button::new();
            button.add_css_class("customize-icon-button");
            let label = crate::i18n::tr(label);
            button.set_tooltip_text(Some(&label));
            crate::ui::accessibility::set_label(&button, &label);
            button.set_child(Some(&crate::assets::primary_icon(icon_name, 20)));
            if initial_icon == Some(icon_name) {
                button.add_css_class("active");
            }
            icon_grid.append(&button);
            (icon_name, button)
        })
        .collect()
}

fn build_emoji_button(selected_emoji: Option<&str>) -> gtk::Button {
    let emoji_button = gtk::Button::with_label(&selected_emoji.map_or_else(
        || crate::i18n::tr("Choose Emoji…"),
        |emoji| rust_i18n::t!("Emoji  %{emoji}", emoji = emoji).into_owned(),
    ));
    emoji_button.add_css_class("customize-emoji-button");
    emoji_button.update_property(&[gtk::accessible::Property::Description(&crate::i18n::tr(
        "Choose any emoji for this item",
    ))]);
    emoji_button
}
