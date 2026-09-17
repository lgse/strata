// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use gtk::{gio, glib, prelude::*};

use crate::{
    assets::icons,
    services::SearchExclusions,
    ui::{
        controls::{form_entry, form_error_label, form_label, modal_layout, set_form_field_error},
        modal::{ModalHost, dismiss_modal_layer, modal_layer},
        theme::ThemeManager,
    },
};

#[cfg(test)]
mod tests;

type RemoveAction = Rc<dyn Fn(&str)>;

pub(super) fn show_search_exclusions_dialog(
    parent: &impl IsA<gtk::Widget>,
    manager: &Rc<ThemeManager>,
) {
    let Some(ModalHost {
        overlay: window_overlay,
        blurred_root,
    }) = ModalHost::blurred_for(parent)
    else {
        return;
    };

    let layout = modal_layout(
        icons::SEARCH,
        "Global search exclusions",
        "Folders and directories excluded from search",
        "Done",
    );
    layout.content.add_css_class("wide");

    let field_label = form_label("Exclude folder name or directory path");
    let field = form_entry();
    field.set_hexpand(true);
    field.set_placeholder_text(Some("Folder name (e.g. .venv) or ~/path…"));

    let browse_btn = gtk::Button::with_label("Browse…");
    browse_btn.add_css_class("action-dialog-cancel");
    browse_btn.set_valign(gtk::Align::Fill);
    browse_btn.set_size_request(84, -1);
    browse_btn.set_tooltip_text(Some("Browse for a folder to exclude"));

    let add_btn = gtk::Button::with_label("Add");
    add_btn.add_css_class("action-dialog-confirm");
    add_btn.set_valign(gtk::Align::Fill);
    add_btn.set_size_request(84, -1);
    add_btn.set_tooltip_text(Some("Add to search exclusions"));

    let buttons_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons_box.set_homogeneous(true);
    buttons_box.append(&browse_btn);
    buttons_box.append(&add_btn);

    let input_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    input_row.append(&field);
    input_row.append(&buttons_box);

    let error_label = form_error_label();

    layout.body.append(&field_label);
    layout.body.append(&input_row);
    layout.body.append(&error_label);

    // Reuse modal suggestion list styles for visual consistency with the Copy to dialog.
    let exclusions_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    exclusions_box.add_css_class("transfer-suggestions");

    let list_scroll = gtk::ScrolledWindow::builder()
        .child(&exclusions_box)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(160)
        .max_content_height(240)
        .propagate_natural_height(true)
        .build();
    list_scroll.add_css_class("transfer-suggestion-scroll");
    layout.body.append(&list_scroll);

    let manager_for_remove = manager.clone();
    let box_for_remove = exclusions_box.clone();
    let on_remove: Rc<RefCell<Option<RemoveAction>>> = Rc::new(RefCell::new(None));
    let on_remove_cell = on_remove.clone();
    *on_remove.borrow_mut() = Some(Rc::new(move |item_to_remove: &str| {
        let is_path = SearchExclusions::is_directory_path(item_to_remove);
        let mut current = manager_for_remove.search_exclusions();
        current.retain(|candidate| {
            if is_path {
                candidate != item_to_remove
            } else {
                !candidate.eq_ignore_ascii_case(item_to_remove)
            }
        });
        manager_for_remove.set_search_exclusions(current);
        if let Some(ref cb) = *on_remove_cell.borrow() {
            render_exclusion_rows(&box_for_remove, &manager_for_remove, cb.clone());
        }
    }));

    if let Some(ref cb) = *on_remove.borrow() {
        render_exclusion_rows(&exclusions_box, manager, cb.clone());
    }

    let field_for_add = field.clone();
    let error_for_add = error_label.clone();
    let manager_for_add = manager.clone();
    let box_for_add = exclusions_box.clone();
    let on_remove_for_add = on_remove.clone();
    let do_add = Rc::new(move || {
        let raw = field_for_add.text().to_string();
        let current = manager_for_add.search_exclusions();
        match validate_exclusion_input(&raw, &current) {
            Ok(trimmed) => {
                set_form_field_error(&field_for_add, &error_for_add, None);
                let mut updated = current;
                updated.push(trimmed);
                manager_for_add.set_search_exclusions(updated);
                field_for_add.set_text("");
                if let Some(ref cb) = *on_remove_for_add.borrow() {
                    render_exclusion_rows(&box_for_add, &manager_for_add, cb.clone());
                }
            }
            Err(error) => {
                set_form_field_error(&field_for_add, &error_for_add, error);
            }
        }
    });

    let do_add_activate = do_add.clone();
    field.connect_activate(move |_| {
        do_add_activate();
    });

    let error_for_change = error_label.clone();
    let field_for_change = field.clone();
    field.connect_changed(move |_| {
        set_form_field_error(&field_for_change, &error_for_change, None);
    });

    let do_add_click = do_add.clone();
    add_btn.connect_clicked(move |_| {
        do_add_click();
    });

    let field_for_browse = field.clone();
    let error_for_browse = error_label.clone();
    browse_btn.connect_clicked(move |button| {
        let Some(window) = button.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let dialog = gtk::FileDialog::builder()
            .title("Select folder to exclude")
            .modal(true)
            .build();
        let field = field_for_browse.clone();
        let error = error_for_browse.clone();
        dialog.select_folder(Some(&window), gio::Cancellable::NONE, move |result| {
            let Ok(file) = result else {
                return;
            };
            if let Some(path) = file.path() {
                let display = super::general::abbreviate_home(&path);
                set_form_field_error(&field, &error, None);
                field.set_text(&display);
                field.set_position(-1);
                field.grab_focus();
            }
        });
    });

    let content = layout.content;
    let close = layout.close;
    let cancel = layout.cancel;
    let confirm = layout.confirm;
    cancel.set_visible(false);

    let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
    window_overlay.add_overlay(&layer);

    let close_layer = layer.clone();
    let close_overlay = window_overlay.clone();
    let close_root = blurred_root.clone();
    close.connect_clicked(move |_| {
        dismiss_modal_layer(&close_layer, &close_overlay, close_root.as_ref());
    });

    let confirm_layer = layer.clone();
    let confirm_overlay = window_overlay.clone();
    let confirm_root = blurred_root.clone();
    confirm.connect_clicked(move |_| {
        dismiss_modal_layer(&confirm_layer, &confirm_overlay, confirm_root.as_ref());
    });

    let esc_confirm = confirm.clone();
    let key_controller = gtk::EventControllerKey::new();
    key_controller.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            esc_confirm.emit_clicked();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(key_controller);

    field.grab_focus();
}

fn render_exclusion_rows(container: &gtk::Box, manager: &ThemeManager, on_remove: RemoveAction) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let exclusions = manager.search_exclusions();
    if exclusions.is_empty() {
        let empty = gtk::Label::new(Some(
            "No custom exclusions added. Common tool caches (.venv, node_modules, target, etc.) are excluded automatically.",
        ));
        empty.add_css_class("transfer-suggestions-empty");
        empty.set_xalign(0.0);
        empty.set_wrap(true);
        container.append(&empty);
        return;
    }
    for item in exclusions {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 9);
        row.add_css_class("transfer-suggestion");
        row.append(&crate::assets::primary_icon(icons::FOLDER, 16));

        let name_label = gtk::Label::new(Some(&item));
        name_label.set_xalign(0.0);
        name_label.set_hexpand(true);
        name_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        row.append(&name_label);

        let is_path = SearchExclusions::is_directory_path(&item);
        let type_label = gtk::Label::new(Some(if is_path { "Directory" } else { "Folder name" }));
        type_label.add_css_class("transfer-suggestion-parent");
        type_label.set_xalign(1.0);
        row.append(&type_label);

        let remove = gtk::Button::new();
        remove.add_css_class("action-dialog-close");
        remove.set_valign(gtk::Align::Center);
        remove.set_tooltip_text(Some("Remove exclusion"));
        remove.set_child(Some(&crate::assets::primary_icon(icons::X, 14)));
        let on_remove_clone = on_remove.clone();
        let item_to_remove = item.clone();
        remove.connect_clicked(move |_| {
            on_remove_clone(&item_to_remove);
        });
        row.append(&remove);

        container.append(&row);
    }
}

pub(super) fn validate_exclusion_input(
    raw: &str,
    current: &[String],
) -> Result<String, Option<&'static str>> {
    let input = raw.trim();
    if input.is_empty() {
        return Err(None);
    }
    if input == "/" || input == "~" {
        return Err(Some("Cannot exclude root or entire home directory."));
    }
    let trimmed = input.trim_end_matches('/').to_string();
    if trimmed.is_empty() || trimmed == "~" {
        return Err(Some("Cannot exclude root or entire home directory."));
    }
    let is_path = SearchExclusions::is_directory_path(&trimmed);
    if is_path && !trimmed.starts_with('/') && !trimmed.starts_with("~/") {
        return Err(Some("Directory paths must start with / or ~/"));
    }
    let is_duplicate = current.iter().any(|existing| {
        if is_path {
            existing == &trimmed
        } else {
            existing.eq_ignore_ascii_case(&trimmed)
        }
    });
    if is_duplicate {
        return Err(Some("This exclusion has already been added."));
    }
    Ok(trimmed)
}
