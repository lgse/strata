// SPDX-License-Identifier: MIT

mod color_bar;
mod custom_color;
mod icon_picker;

use crate::model::Location;
use crate::services::ItemIdentity;
use crate::ui::controls::{ModalLayout, focus_button, modal_layout};
use crate::ui::keys::{self, bindings::ESCAPE_ANY};
use crate::ui::modal::{ModalHost, dismiss_modal_layer, modal_layer, remember_modal_focus};
use crate::ui::preferences::PreferenceManager;
use color_bar::{ColorBarItem, FolderColorBar, build_folder_color_bar};
use gtk::glib;
use gtk::prelude::*;
use icon_picker::{IconChoices, IconPicker};
use std::path::PathBuf;
use std::rc::Rc;

/// Item customizations are keyed by native path; remote locations carry none.
pub(super) fn relocate_item_customizations(from: &Location, to: &Location, merged: bool) {
    if let Some(from) = from.native_path() {
        PreferenceManager::shared().relocate_item_customizations(from, to.native_path(), merged);
    }
}

pub(super) fn forget_item_customizations(
    location: &Location,
    trash_identity: Option<ItemIdentity>,
) {
    if let Some(path) = location.native_path() {
        PreferenceManager::shared().forget_item_customizations(path, trash_identity);
    }
}

pub(super) fn restore_item_customizations(location: &Location, identity: Option<ItemIdentity>) {
    if let (Some(path), Some(identity)) = (location.native_path(), identity) {
        PreferenceManager::shared().restore_item_customizations(path, identity);
    }
}

fn popdown_ancestor_popover(parent: &impl IsA<gtk::Widget>) {
    if let Some(popover) = parent
        .ancestor(gtk::Popover::static_type())
        .and_downcast::<gtk::Popover>()
    {
        popover.popdown();
    }
}

/// The item being customized plus the dialog widgets every edit refreshes.
struct CustomizedItem {
    path: PathBuf,
    fallback_icon: &'static str,
    preview: gtk::Image,
    clear: gtk::Button,
}

impl CustomizedItem {
    fn refresh_preview(&self) {
        crate::ui::thumbnail::show_customized_icon_image(
            &self.preview,
            &self.path,
            self.fallback_icon,
            56,
        );
    }

    fn note_customized(&self) {
        self.refresh_preview();
        self.clear.set_sensitive(true);
    }
}

pub(in crate::ui) fn show_customize_modal(
    parent: &impl IsA<gtk::Widget>,
    path: PathBuf,
    is_directory: bool,
    fallback_icon: &'static str,
) {
    let Some(host) = ModalHost::blurred_for(parent) else {
        return;
    };
    popdown_ancestor_popover(parent);

    let item_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let layout = build_customize_layout(is_directory, &item_name);

    let preference_manager = PreferenceManager::shared();
    let initial_color = preference_manager.folder_color(&path);
    let initial_icon = preference_manager.custom_icon(&path);

    let preview_icon = append_preview(&layout.body, &path, &item_name, fallback_icon);
    let clear = build_clear_button(&layout, initial_color.is_some() || initial_icon.is_some());
    let item = Rc::new(CustomizedItem {
        path,
        fallback_icon,
        preview: preview_icon,
        clear: clear.clone(),
    });

    let color_bar = append_color_section(&layout.body, &item, initial_color, is_directory);
    let icon_picker = append_icon_section(&layout.body, &item, initial_icon.as_deref());
    connect_clear(&clear, &item, color_bar, icon_picker.choices.clone());

    present_customize_layer(layout, host, &icon_picker.emoji_chooser);
}

fn build_customize_layout(is_directory: bool, item_name: &str) -> ModalLayout {
    let item_kind = if is_directory {
        "Customize Folder"
    } else {
        "Customize File"
    };
    let layout = modal_layout(
        crate::assets::icons::PALETTE,
        &crate::i18n::tr(item_kind),
        item_name,
        &crate::i18n::tr("Done"),
    );
    layout.content.add_css_class("customize-dialog");
    layout
        .subtitle
        .set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    layout.subtitle.set_max_width_chars(36);
    layout.cancel.set_visible(false);
    layout
}

fn append_preview(
    body: &gtk::Box,
    path: &PathBuf,
    item_name: &str,
    fallback_icon: &'static str,
) -> gtk::Image {
    let preview = gtk::Box::new(gtk::Orientation::Vertical, 7);
    preview.add_css_class("customize-preview");
    preview.set_halign(gtk::Align::Center);
    let preview_icon = gtk::Image::new();
    preview_icon.add_css_class("customize-preview-icon");
    crate::ui::thumbnail::show_customized_icon_image(&preview_icon, path, fallback_icon, 56);
    let preview_name = gtk::Label::new(Some(item_name));
    preview_name.add_css_class("customize-preview-name");
    preview_name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    preview_name.set_max_width_chars(30);
    preview.append(&preview_icon);
    preview.append(&preview_name);
    body.append(&preview);
    preview_icon
}

fn build_clear_button(layout: &ModalLayout, customized: bool) -> gtk::Button {
    let clear = gtk::Button::with_label(&crate::i18n::tr("Clear"));
    clear.add_css_class("action-dialog-cancel");
    clear.set_sensitive(customized);
    layout
        .actions
        .insert_child_after(&clear, Some(&layout.cancel));
    clear
}

fn build_section(label: &str) -> gtk::Box {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 8);
    section.add_css_class("customize-section");
    let label = gtk::Label::new(Some(&crate::i18n::tr(label)));
    label.add_css_class("customize-section-label");
    label.set_xalign(0.0);
    section.append(&label);
    section
}

fn append_color_section(
    body: &gtk::Box,
    item: &Rc<CustomizedItem>,
    initial_color: Option<crate::model::FolderColorValue>,
    is_directory: bool,
) -> FolderColorBar {
    let color_section = build_section("COLOR");
    let color_item = item.clone();
    let bar_item = ColorBarItem {
        preview_icon: item.fallback_icon,
        label: if is_directory { "folder" } else { "file" },
    };
    let color_bar = build_folder_color_bar(initial_color, bar_item, move |selected_color| {
        PreferenceManager::shared().set_folder_color(&color_item.path, selected_color);
        color_item.note_customized();
    });
    color_section.append(&color_bar.container);
    body.append(&color_section);
    color_bar
}

fn append_icon_section(
    body: &gtk::Box,
    item: &Rc<CustomizedItem>,
    initial_icon: Option<&str>,
) -> IconPicker {
    let section = build_section("ICON");
    section.add_css_class("separated");
    let picker = IconPicker::build(initial_icon);
    picker.connect_icon_chosen(item);
    section.append(&picker.grid);
    section.append(&picker.choices.emoji_button);
    body.append(&section);
    picker
}

fn connect_clear(
    clear: &gtk::Button,
    item: &Rc<CustomizedItem>,
    color_bar: FolderColorBar,
    icon_choices: IconChoices,
) {
    let item = item.clone();
    clear.connect_clicked(move |button| {
        PreferenceManager::shared().clear_item_customization(&item.path);
        color_bar.selection.update_active(None);
        icon_choices.show_icon_active(None);
        item.refresh_preview();
        button.set_sensitive(false);
    });
}

fn present_customize_layer(
    layout: ModalLayout,
    host: ModalHost,
    emoji_chooser: &gtk::EmojiChooser,
) {
    let ModalHost {
        overlay: window_overlay,
        blurred_root,
    } = host;
    let content = layout.content;
    let confirm = layout.confirm;
    let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
    remember_modal_focus(&layer, &window_overlay);
    window_overlay.add_overlay(&layer);

    let dismiss = {
        let layer = layer.clone();
        let overlay = window_overlay.clone();
        let root = blurred_root.clone();
        move || dismiss_modal_layer(&layer, &overlay, root.as_ref())
    };
    let done_dismiss = dismiss.clone();
    confirm.connect_clicked(move |_| done_dismiss());
    layout.close.connect_clicked(move |_| dismiss());

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let escape_layer = layer.clone();
    let escape_overlay = window_overlay;
    let escape_root = blurred_root;
    let escape_emoji = emoji_chooser.downgrade();
    keys::on_press(&keys, move |_, press| {
        if !press.is(ESCAPE_ANY) {
            return glib::Propagation::Proceed;
        }
        // The capture phase sees the emoji popover's Escape before the popover does.
        if let Some(emoji) = escape_emoji.upgrade().filter(|emoji| emoji.is_visible()) {
            emoji.popdown();
        } else {
            dismiss_modal_layer(&escape_layer, &escape_overlay, escape_root.as_ref());
        }
        glib::Propagation::Stop
    });
    layer.add_controller(keys);
    focus_button(&confirm);
}
