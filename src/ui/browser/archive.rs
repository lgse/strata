// SPDX-License-Identifier: GPL-3.0-or-later

use crate::adapters::gio_file_for_location;
use crate::model::{FileEntry, Location};
use crate::services::{ArchiveFormat, TransferConflict, validate_basename};
use crate::ui::browser::ViewState;
use crate::ui::browser::destination::{
    folder_input_path, resolve_destination_path, setup_transfer_search,
};
use crate::ui::browser::entry::{entry_kind_summary, item_count_label};
use crate::ui::browser::inline_edit::update_basename_validation;
use crate::ui::browser::paths::compact_display_path;
use crate::ui::controls::{
    ModalTone, form_entry, form_label, form_password_entry, message_dialog_description,
    message_dialog_layout, modal_layout, segmented_control,
};
use crate::ui::modal::{
    ModalHost, dismiss_modal_layer, modal_layer, show_error_dialog, submit_on_enter,
};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

fn normalized_archive_name(name: &str, format: ArchiveFormat) -> String {
    name.strip_suffix(&format!(".{}", format.extension()))
        .unwrap_or(name)
        .to_owned()
}

fn archive_has_collision(destination: &Location, archive_name: &str) -> bool {
    gio_file_for_location(destination)
        .child(archive_name)
        .query_exists(None::<&gio::Cancellable>)
}

impl ViewState {
    fn build_archive_modal(
        self: &Rc<Self>,
        title: &str,
        subtitle: &str,
        confirm_label: &str,
        block_dismiss: Option<Rc<dyn Fn() -> bool>>,
    ) -> (gtk::Box, gtk::Button, Rc<dyn Fn()>) {
        let Some(ModalHost {
            overlay: window_overlay,
            blurred_root,
        }) = ModalHost::blurred_for(&self.overlay)
        else {
            return (gtk::Box::default(), gtk::Button::default(), Rc::new(|| {}));
        };

        let layout = modal_layout(
            crate::assets::icons::FILE_ARCHIVE,
            title,
            subtitle,
            confirm_label,
        );
        let layer = modal_layer(
            &layout.content,
            &window_overlay,
            blurred_root.clone(),
            block_dismiss,
        );
        window_overlay.add_overlay(&layer);

        let dismiss: Rc<dyn Fn()> = Rc::new({
            let layer = layer.clone();
            let overlay = window_overlay.clone();
            let root = blurred_root.clone();
            move || dismiss_modal_layer(&layer, &overlay, root.as_ref())
        });
        let dismiss_for_cancel = dismiss.clone();
        layout.cancel.connect_clicked(move |_| dismiss_for_cancel());
        let dismiss_for_close = dismiss.clone();
        layout.close.connect_clicked(move |_| dismiss_for_close());
        let escape = gtk::EventControllerKey::new();
        let dismiss_for_escape = dismiss.clone();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                dismiss_for_escape();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);
        (layout.body, layout.confirm, dismiss)
    }

    fn start_compression(
        self: &Rc<Self>,
        entries: Vec<FileEntry>,
        destination: Location,
        archive_name: String,
        format: ArchiveFormat,
        password: Option<String>,
    ) {
        let final_name = format!("{archive_name}.{}", format.extension());
        if !archive_has_collision(&destination, &final_name) {
            self.browser.compress(
                entries,
                destination,
                archive_name,
                TransferConflict::FailIfExists,
                format,
                password,
            );
            return;
        }
        let Some(ModalHost {
            overlay: window_overlay,
            blurred_root,
        }) = ModalHost::blurred_for(&self.overlay)
        else {
            return;
        };
        let layout = message_dialog_layout(
            crate::assets::icons::FILE_ARCHIVE,
            "File already exists",
            &final_name,
            "Replace",
            ModalTone::Danger,
        );
        layout.body.append(&message_dialog_description(&format!(
            "An archive named “{final_name}” already exists in {}. Replacing it will overwrite its contents.",
            compact_display_path(&destination)
        )));
        let content = layout.content;
        let close = layout.close;
        let cancel = layout.cancel;
        let replace = layout.confirm;
        let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
        window_overlay.add_overlay(&layer);

        for button in [&close, &cancel] {
            let dismissed_layer = layer.clone();
            let dismissed_overlay = window_overlay.clone();
            let dismissed_root = blurred_root.clone();
            let browser = self.browser.clone();
            button.connect_clicked(move |_| {
                dismiss_modal_layer(
                    &dismissed_layer,
                    &dismissed_overlay,
                    dismissed_root.as_ref(),
                );
                browser.focus_active();
            });
        }

        let replaced_layer = layer.clone();
        let replaced_overlay = window_overlay.clone();
        let replaced_root = blurred_root.clone();
        let browser = self.browser.clone();
        replace.connect_clicked(move |_| {
            dismiss_modal_layer(&replaced_layer, &replaced_overlay, replaced_root.as_ref());
            browser.compress(
                entries.clone(),
                destination.clone(),
                archive_name.clone(),
                TransferConflict::ReplaceExisting,
                format,
                password.clone(),
            );
        });

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let escaped_layer = layer.clone();
        let escaped_overlay = window_overlay;
        let escaped_root = blurred_root;
        let enter_replace = replace.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                dismiss_modal_layer(&escaped_layer, &escaped_overlay, escaped_root.as_ref());
                glib::Propagation::Stop
            } else if key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter {
                enter_replace.emit_clicked();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(keys);
        replace.grab_focus();
    }

    pub(super) fn show_compress_dialog(self: &Rc<Self>, entries: Vec<FileEntry>) {
        if entries.is_empty() {
            return;
        }
        let destination = entries[0]
            .location
            .parent()
            .or_else(|| self.browser.active_location())
            .unwrap_or_else(|| Location::local(glib::home_dir()));

        let default_name = if entries.len() == 1 {
            entries[0].display_name.clone()
        } else {
            "archive".to_owned()
        };

        let title = format!("Compress {}", item_count_label(entries.len()));
        let subtitle = entry_kind_summary(&entries);

        let name_entry = form_entry();
        name_entry.set_text(&default_name);
        name_entry.connect_changed(|field| {
            update_basename_validation(field);
        });
        let password_entry = form_password_entry();
        password_entry.set_show_peek_icon(true);
        let confirm_entry = form_password_entry();
        confirm_entry.set_show_peek_icon(true);
        let compress_default_name = default_name.clone();
        let dirty_name = name_entry.clone();
        let dirty_password = password_entry.clone();
        let dirty_confirm = confirm_entry.clone();
        let (body, confirm, dismiss) = self.build_archive_modal(
            &title,
            &subtitle,
            "Compress",
            Some(Rc::new(move || {
                dirty_name.text() != compress_default_name
                    || !dirty_password.text().is_empty()
                    || !dirty_confirm.text().is_empty()
            })),
        );

        let name_label = form_label("Archive name");
        body.append(&name_label);
        body.append(&name_entry);

        let format_label = form_label("Format");
        let (format_control, format_options) =
            segmented_control(&["ZIP", "7Z", "TAR.GZ", "TAR"], 0);
        let selected_format = Rc::new(Cell::new(ArchiveFormat::Zip));
        body.append(&format_label);
        body.append(&format_control);

        let protection_label = form_label("Protection");
        let (protection_control, protection_options) =
            segmented_control(&["No password", "Password protected"], 0);
        let no_password = protection_options[0].clone();
        let password_protected = protection_options[1].clone();

        let password_label = form_label("Password");
        let confirm_label = form_label("Confirm password");
        let password_fields = gtk::Box::new(gtk::Orientation::Vertical, 6);
        password_fields.append(&password_label);
        password_fields.append(&password_entry);
        password_fields.append(&confirm_label);
        password_fields.append(&confirm_entry);
        password_fields.set_visible(false);

        let protection_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        protection_box.append(&protection_label);
        protection_box.append(&protection_control);
        protection_box.append(&password_fields);
        body.append(&protection_box);

        let fields_for_protection = password_fields.clone();
        let password_for_focus = password_entry.clone();
        password_protected.connect_toggled(move |option| {
            fields_for_protection.set_visible(option.is_active());
            if option.is_active() {
                password_for_focus.grab_focus();
            }
        });

        for (option, format) in format_options.into_iter().zip([
            ArchiveFormat::Zip,
            ArchiveFormat::SevenZ,
            ArchiveFormat::TarGz,
            ArchiveFormat::Tar,
        ]) {
            let selected_format = selected_format.clone();
            let protection_for_format = protection_box.clone();
            let no_password_for_format = no_password.clone();
            option.connect_toggled(move |option| {
                if !option.is_active() {
                    return;
                }
                selected_format.set(format);
                let supported = format.supports_password();
                protection_for_format.set_visible(supported);
                if !supported {
                    no_password_for_format.set_active(true);
                }
            });
        }

        let state = Rc::downgrade(self);
        let confirm_entries = entries.clone();
        let confirm_destination = destination.clone();
        let name_for_confirm = name_entry.clone();
        let format_for_confirm = selected_format.clone();
        let password_for_confirm = password_entry.clone();
        let confirm_for_confirm = confirm_entry.clone();
        let protected_for_confirm = password_protected.clone();
        let overlay_for_error = self.overlay.clone();
        let dismiss_for_confirm = dismiss.clone();
        confirm.connect_clicked(move |_| {
            let name = name_for_confirm.text().to_string();
            let format = format_for_confirm.get();
            let archive_name = normalized_archive_name(&name, format);
            if let Err(message) = validate_basename(&archive_name) {
                name_for_confirm.add_css_class("error");
                name_for_confirm.set_tooltip_text(Some(message));
                name_for_confirm.grab_focus();
                return;
            }
            let password = if format.supports_password() && protected_for_confirm.is_active() {
                let pw = password_for_confirm.text().to_string();
                if pw.is_empty() {
                    show_error_dialog(
                        &overlay_for_error,
                        "Password required",
                        "Enter a password or choose No password.",
                    );
                    return;
                }
                let confirm_pw = confirm_for_confirm.text().to_string();
                if pw != confirm_pw {
                    show_error_dialog(
                        &overlay_for_error,
                        "Passwords do not match",
                        "Please enter the same password in both fields.",
                    );
                    return;
                }
                Some(pw)
            } else {
                None
            };
            dismiss_for_confirm();
            if let Some(state) = state.upgrade() {
                state.start_compression(
                    confirm_entries.clone(),
                    confirm_destination.clone(),
                    archive_name,
                    format,
                    password,
                );
            }
        });
        submit_on_enter(&body, &confirm);
        name_entry.grab_focus();
    }

    pub(super) fn extract_entry(self: &Rc<Self>, entry: FileEntry) {
        let Some(parent) = entry.location.parent() else {
            show_error_dialog(
                &self.overlay,
                "Cannot extract",
                "This archive has no parent directory.",
            );
            return;
        };
        let format = ArchiveFormat::from_extension(&entry.display_name);
        if format.map(|f| f.supports_password()).unwrap_or(false) {
            self.pending_extract_retry
                .replace(Some((entry.clone(), parent.clone())));
        }
        self.browser.extract(entry, parent, None);
    }

    pub(super) fn show_extract_to_dialog(self: &Rc<Self>, entry: FileEntry) {
        let base = entry
            .location
            .parent()
            .and_then(|p| p.native_path().map(Path::to_path_buf))
            .unwrap_or_else(glib::home_dir);
        let field = form_entry();
        field.set_hexpand(true);
        field.set_placeholder_text(Some("Search for a folder…"));
        field.set_text(&folder_input_path(&base));
        field.set_position(-1);
        let extract_initial_text = folder_input_path(&base);
        let dirty_field = field.clone();
        let (body, confirm, dismiss) = self.build_archive_modal(
            "Extract to",
            &entry.display_name,
            "Extract here",
            Some(Rc::new(move || dirty_field.text() != extract_initial_text)),
        );
        let field_label = form_label("Destination folder");
        body.append(&field_label);
        body.append(&field);

        let suggestions = gtk::Box::new(gtk::Orientation::Vertical, 2);
        suggestions.add_css_class("transfer-suggestions");
        let suggestion_scroll = gtk::ScrolledWindow::builder()
            .child(&suggestions)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .min_content_height(150)
            .max_content_height(220)
            .propagate_natural_height(true)
            .build();
        suggestion_scroll.add_css_class("transfer-suggestion-scroll");
        body.append(&suggestion_scroll);
        let error = gtk::Label::new(None);
        error.add_css_class("form-message");
        error.add_css_class("error");
        error.set_wrap(true);
        error.set_xalign(0.0);
        error.set_visible(false);
        body.append(&error);

        let generation = Rc::new(Cell::new(0_u64));
        let suggestions_box = suggestions.clone();
        let extract_error = error.clone();
        setup_transfer_search(
            &field,
            &suggestions_box,
            &generation,
            base.clone(),
            self.browser.preferences().show_hidden,
            move |field| {
                field.remove_css_class("error");
                extract_error.set_visible(false);
            },
        );

        let extract_state = self.clone();
        let confirm_field = field.clone();
        let confirm_error = error.clone();
        let confirm_base = base.clone();
        let extract_entry = entry.clone();
        let dismiss_for_confirm = dismiss.clone();
        confirm.connect_clicked(move |_| {
            let path =
                resolve_destination_path(&confirm_field.text(), &confirm_base, &glib::home_dir());
            if path.exists() && !path.is_dir() {
                confirm_error.set_text("The destination exists, but it is not a folder.");
                confirm_error.set_visible(true);
                confirm_field.add_css_class("error");
                confirm_field.grab_focus();
                return;
            }
            if !path.exists()
                && let Err(e) = std::fs::create_dir_all(&path)
            {
                confirm_error.set_text(&format!("Could not create folder: {e}"));
                confirm_error.set_visible(true);
                confirm_field.add_css_class("error");
                return;
            }
            let dest = Location::local(path);
            let format = ArchiveFormat::from_extension(&extract_entry.display_name);
            if format.map(|f| f.supports_password()).unwrap_or(false) {
                extract_state
                    .pending_extract_retry
                    .replace(Some((extract_entry.clone(), dest.clone())));
            }
            extract_state.pending_navigate.replace(Some(dest.clone()));
            extract_state
                .browser
                .extract(extract_entry.clone(), dest, None);
            dismiss_for_confirm();
        });

        submit_on_enter(&body, &confirm);
        field.grab_focus();
    }

    pub(super) fn show_extract_password_dialog(
        self: &Rc<Self>,
        entry: FileEntry,
        destination: Location,
    ) {
        let password_entry = form_password_entry();
        password_entry.set_show_peek_icon(true);
        let dirty_password = password_entry.clone();
        let (body, confirm, dismiss) = self.build_archive_modal(
            "Extract",
            &entry.display_name,
            "Extract",
            Some(Rc::new(move || !dirty_password.text().is_empty())),
        );

        let password_label = form_label("Password");
        body.append(&password_label);
        body.append(&password_entry);

        let browser = self.browser.clone();
        let password_for_confirm = password_entry.clone();
        let dismiss_for_confirm = dismiss.clone();
        confirm.connect_clicked(move |_| {
            let pw = password_for_confirm.text().to_string();
            let password = if pw.is_empty() { None } else { Some(pw) };
            dismiss_for_confirm();
            browser.extract(entry.clone(), destination.clone(), password);
        });
        submit_on_enter(&body, &confirm);
        password_entry.grab_focus();
    }
}

#[cfg(test)]
mod tests;
