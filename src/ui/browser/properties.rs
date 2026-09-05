// SPDX-License-Identifier: GPL-3.0-or-later

use crate::adapters::gio_file_for_location;
use crate::adapters::trash::summarize_trash;
use crate::model::{FileEntry, Location};
use crate::ui::browser::clipboard::copy_path_text;
use crate::ui::browser::desktop::open_location;
use crate::ui::browser::entry::{entry_icon, format_file_size, item_count_label};
use crate::ui::browser::paths::{compact_display_path, is_trash_location, is_trash_root};
use crate::ui::browser::{PinStatus, ViewState};
use crate::ui::controls::{form_check_button, modal_layout};
use crate::ui::modal::{ModalHost, dismiss_modal_layer, modal_layer, show_error_dialog};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::Cell;
use std::rc::Rc;

fn properties_row(parent: &gtk::Box, label: &str, value: &str) -> gtk::Label {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("properties-row");
    let label = gtk::Label::new(Some(label));
    label.add_css_class("properties-row-label");
    label.set_xalign(0.0);
    let value = gtk::Label::new(Some(value));
    value.add_css_class("properties-row-value");
    value.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    value.set_max_width_chars(48);
    value.set_hexpand(true);
    value.set_xalign(0.0);
    row.append(&label);
    row.append(&value);
    parent.append(&row);
    value
}

#[derive(Clone)]
struct PermissionRow {
    identity: gtk::Label,
    bits: [gtk::Button; 3],
}

#[derive(Clone)]
struct PermissionEditor {
    mode: Rc<Cell<Option<u32>>>,
    changing: Rc<Cell<bool>>,
    syncing: Rc<Cell<bool>>,
    mode_label: gtk::Label,
    rows: [PermissionRow; 3],
    executable: gtk::CheckButton,
}

fn permission_row(parent: &gtk::Box, label: &str) -> PermissionRow {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("properties-permission-row");
    let title = gtk::Label::new(Some(label));
    title.add_css_class("properties-permission-title");
    title.set_xalign(0.0);
    let identity = gtk::Label::new(Some("—"));
    identity.add_css_class("properties-permission-identity");
    identity.set_xalign(0.0);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let read = gtk::Button::with_label("—");
    let write = gtk::Button::with_label("—");
    let execute = gtk::Button::with_label("—");
    for permission in [&read, &write, &execute] {
        permission.add_css_class("properties-permission-bit");
        permission.set_sensitive(false);
    }
    row.append(&title);
    row.append(&identity);
    row.append(&spacer);
    row.append(&read);
    row.append(&write);
    row.append(&execute);
    parent.append(&row);
    PermissionRow {
        identity,
        bits: [read, write, execute],
    }
}

fn set_permission_row(row: &PermissionRow, mode: u32, shift: u32) {
    let value = (mode >> shift) & 0o7;
    row.bits[0].set_label(if value & 0o4 != 0 { "r" } else { "—" });
    row.bits[1].set_label(if value & 0o2 != 0 { "w" } else { "—" });
    row.bits[2].set_label(if value & 0o1 != 0 { "x" } else { "—" });
    for (index, permission) in row.bits.iter().enumerate() {
        let enabled = value & [0o4, 0o2, 0o1][index] != 0;
        if enabled {
            permission.add_css_class("enabled");
        } else {
            permission.remove_css_class("enabled");
        }
    }
}

fn set_permission_editor_sensitive(editor: &PermissionEditor, sensitive: bool) {
    for row in &editor.rows {
        for button in &row.bits {
            button.set_sensitive(sensitive);
        }
    }
    editor.executable.set_sensitive(sensitive);
}

fn update_permission_editor(editor: &PermissionEditor, mode: u32) {
    editor.mode_label.set_text(&format_permissions(mode));
    set_permission_row(&editor.rows[0], mode, 6);
    set_permission_row(&editor.rows[1], mode, 3);
    set_permission_row(&editor.rows[2], mode, 0);
    editor.syncing.set(true);
    editor.executable.set_active(mode & 0o111 != 0);
    editor.syncing.set(false);
}

fn request_permission_change(
    file: gio::File,
    requested_mode: u32,
    editor: PermissionEditor,
    parent: gtk::Widget,
) {
    if editor.changing.replace(true) {
        return;
    }
    let previous_mode = editor.mode.replace(Some(requested_mode));
    update_permission_editor(&editor, requested_mode);
    set_permission_editor_sensitive(&editor, false);
    let attributes = gio::FileInfo::new();
    attributes.set_attribute_uint32("unix::mode", requested_mode & 0o7777);
    glib::MainContext::default().spawn_local(async move {
        match file
            .set_attributes_future(
                &attributes,
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
            )
            .await
        {
            Ok(_) => {}
            Err(error) => {
                if let Some(mode) = previous_mode {
                    editor.mode.set(Some(mode));
                    update_permission_editor(&editor, mode);
                }
                tracing::warn!(%error, "unable to change file permissions");
                show_error_dialog(&parent, "Unable to change permissions", &error.to_string());
            }
        }
        editor.changing.set(false);
        set_permission_editor_sensitive(&editor, true);
    });
}

fn toggled_permission(mode: u32, mask: u32) -> u32 {
    mode ^ mask
}

fn with_execute_permissions(mode: u32, executable: bool) -> u32 {
    if executable {
        mode | 0o111
    } else {
        mode & !0o111
    }
}

pub fn format_permissions(mode: u32) -> String {
    let kind = if mode & 0o170000 == 0o040000 {
        'd'
    } else {
        '-'
    };
    let mut symbolic = String::with_capacity(10);
    symbolic.push(kind);
    for (mask, character) in [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ] {
        symbolic.push(if mode & mask != 0 { character } else { '-' });
    }
    format!("{symbolic}  {:03o}", mode & 0o777)
}

fn properties_action(icon: &str, label: &str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&crate::assets::primary_icon(icon, 14));
    content.append(&gtk::Label::new(Some(label)));
    let button = gtk::Button::builder().child(&content).build();
    button.add_css_class("properties-action");
    button
}

impl ViewState {
    pub(super) fn show_folder_properties(self: &Rc<Self>, location: &Location) {
        self.show_properties(location.clone(), None);
    }

    pub(super) fn show_entry_properties(self: &Rc<Self>, entry: FileEntry) {
        self.show_properties(entry.location.clone(), Some(entry));
    }

    fn show_properties(self: &Rc<Self>, location: Location, entry: Option<FileEntry>) {
        let Some(ModalHost {
            overlay: window_overlay,
            blurred_root,
        }) = ModalHost::blurred_for(&self.overlay)
        else {
            return;
        };
        let is_directory = entry.as_ref().is_none_or(FileEntry::is_directory);
        let name = entry
            .as_ref()
            .map(|entry| entry.display_name.clone())
            .unwrap_or_else(|| location.display_name());
        let icon_name = entry
            .as_ref()
            .map(entry_icon)
            .unwrap_or(crate::assets::icons::FOLDER);

        let layout = modal_layout(
            icon_name,
            &name,
            if is_directory { "Folder" } else { "File" },
            "Close",
        );
        if let Some(path) = location.native_path() {
            crate::ui::thumbnail::show_customized_icon(&layout.icon, path, icon_name, 21);
        }
        layout.content.add_css_class("properties-content");
        layout.title.set_max_width_chars(44);
        layout.title.set_wrap(true);
        layout.title.set_lines(2);
        layout.title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        layout
            .title
            .set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        layout.cancel.set_visible(false);
        layout.confirm.set_visible(false);
        let kind = layout.subtitle.clone();
        let close = layout.close.clone();

        let details = gtk::Box::new(gtk::Orientation::Vertical, 0);
        details.add_css_class("properties-details");
        let location_value = properties_row(&details, "LOCATION", &compact_display_path(&location));
        location_value.set_tooltip_text(Some(&location.display_path()));
        let trash_root = is_trash_root(&location);
        let initial_size = if trash_root {
            "Calculating…".to_owned()
        } else {
            entry
                .as_ref()
                .and_then(|entry| match entry.size {
                    crate::model::MetadataValue::Known(size) => Some(format_file_size(size)),
                    crate::model::MetadataValue::Unknown
                    | crate::model::MetadataValue::Unavailable => None,
                })
                .unwrap_or_else(|| "—".to_owned())
        };
        let size = properties_row(&details, "SIZE", &initial_size);
        let modified = properties_row(&details, "MODIFIED", "—");
        crate::util::set_modified_date(&modified, entry.as_ref(), "—");
        let opens_with = properties_row(&details, "OPENS WITH", "—");
        let hidden = properties_row(
            &details,
            "HIDDEN",
            if name.starts_with('.') { "Yes" } else { "No" },
        );
        let pin_status = self
            .pin_status_handler
            .borrow()
            .as_ref()
            .map_or(PinStatus::Unavailable, |handler| handler(&location));
        let _pinned = properties_row(
            &details,
            "PINNED",
            if pin_status == PinStatus::Pinned {
                "Yes"
            } else {
                "No"
            },
        );
        layout.body.append(&details);

        let permissions = gtk::Box::new(gtk::Orientation::Vertical, 8);
        permissions.add_css_class("properties-permissions");
        let permissions_header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let permissions_title = gtk::Label::new(Some("PERMISSIONS"));
        permissions_title.add_css_class("properties-section-title");
        permissions_title.set_xalign(0.0);
        permissions_title.set_hexpand(true);
        let permissions_mode = gtk::Label::new(Some("—"));
        permissions_mode.add_css_class("properties-mode");
        permissions_header.append(&permissions_title);
        permissions_header.append(&permissions_mode);
        permissions.append(&permissions_header);
        let owner = permission_row(&permissions, "Owner");
        let group = permission_row(&permissions, "Group");
        let others = permission_row(&permissions, "Others");
        let executable = form_check_button("Allow executing file as a program (+x)");
        executable.add_css_class("properties-executable");
        executable.set_sensitive(false);
        executable.set_visible(!is_directory);
        permissions.append(&executable);
        layout.body.append(&permissions);

        layout.actions.add_css_class("properties-actions");
        let open = properties_action(crate::assets::icons::EXTERNAL_LINK, "Open");
        let rename = properties_action(crate::assets::icons::PENCIL, "Rename");
        rename.set_sensitive(
            entry.is_some() && (self.interactive || self.browser.selected_entries().len() == 1),
        );
        open.set_visible(self.interactive);
        let pin = properties_action(crate::assets::icons::PIN, "Pin");
        pin.set_visible(self.interactive);
        let pin_handler = self.pin_handler.borrow().clone();
        pin.set_sensitive(
            is_directory
                && !is_trash_location(&location)
                && pin_handler.is_some()
                && pin_status == PinStatus::Available,
        );
        let copy_path = properties_action(crate::assets::icons::COPY, "Copy path");
        layout.actions.prepend(&copy_path);
        layout.actions.prepend(&pin);
        layout.actions.prepend(&rename);
        layout.actions.prepend(&open);
        let content = layout.content;

        let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
        window_overlay.add_overlay(&layer);

        let permission_editor = PermissionEditor {
            mode: Rc::new(Cell::new(None)),
            changing: Rc::new(Cell::new(false)),
            syncing: Rc::new(Cell::new(false)),
            mode_label: permissions_mode.clone(),
            rows: [owner.clone(), group.clone(), others.clone()],
            executable: executable.clone(),
        };
        for (row, masks, subject) in [
            (&owner, [0o400, 0o200, 0o100], "owner"),
            (&group, [0o040, 0o020, 0o010], "group"),
            (&others, [0o004, 0o002, 0o001], "others"),
        ] {
            for ((button, mask), permission) in
                row.bits.iter().zip(masks).zip(["read", "write", "execute"])
            {
                button.set_tooltip_text(Some(&format!("Toggle {subject} {permission} permission")));
                let edited_file = gio_file_for_location(&location);
                let editor = permission_editor.clone();
                let parent = layer.clone();
                button.connect_clicked(move |_| {
                    let Some(mode) = editor.mode.get() else {
                        return;
                    };
                    request_permission_change(
                        edited_file.clone(),
                        toggled_permission(mode, mask),
                        editor.clone(),
                        parent.clone().upcast(),
                    );
                });
            }
        }
        let executable_file = gio_file_for_location(&location);
        let executable_editor = permission_editor.clone();
        let executable_parent = layer.clone();
        executable.connect_toggled(move |button| {
            if executable_editor.syncing.get() {
                return;
            }
            let Some(mode) = executable_editor.mode.get() else {
                return;
            };
            request_permission_change(
                executable_file.clone(),
                with_execute_permissions(mode, button.is_active()),
                executable_editor.clone(),
                executable_parent.clone().upcast(),
            );
        });
        let closing_layer = layer.clone();
        let closing_overlay = window_overlay.clone();
        let closing_root = blurred_root.clone();
        close.connect_clicked(move |_| {
            dismiss_modal_layer(&closing_layer, &closing_overlay, closing_root.as_ref());
        });
        let opening_layer = layer.clone();
        let opening_overlay = window_overlay.clone();
        let opening_root = blurred_root.clone();
        let opening_location = location.clone();
        open.connect_clicked(move |_| {
            open_location(&opening_location, &opening_layer);
            dismiss_modal_layer(&opening_layer, &opening_overlay, opening_root.as_ref());
        });
        let renamed_layer = layer.clone();
        let renamed_overlay = window_overlay.clone();
        let renamed_root = blurred_root.clone();
        let weak = Rc::downgrade(self);
        rename.connect_clicked(move |_| {
            dismiss_modal_layer(&renamed_layer, &renamed_overlay, renamed_root.as_ref());
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(state) = weak.upgrade() {
                    state.begin_rename();
                }
            });
        });
        let pinning_layer = layer.clone();
        let pinning_overlay = window_overlay.clone();
        let pinning_root = blurred_root.clone();
        let pinning_location = location.clone();
        let pinning_name = name.clone();
        pin.connect_clicked(move |_| {
            if let Some(handler) = pin_handler.as_ref() {
                handler(pinning_location.clone(), pinning_name.clone());
            }
            dismiss_modal_layer(&pinning_layer, &pinning_overlay, pinning_root.as_ref());
        });
        let copied_location = location.clone();
        copy_path.connect_clicked(move |button| {
            if let Some(display) = gtk::gdk::Display::default() {
                display
                    .clipboard()
                    .set_text(&copy_path_text(&copied_location, true));
                button.set_label("Copied");
            }
        });
        let escape = gtk::EventControllerKey::new();
        let escaped_layer = layer.clone();
        let escaped_overlay = window_overlay.clone();
        let escaped_root = blurred_root.clone();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                dismiss_modal_layer(&escaped_layer, &escaped_overlay, escaped_root.as_ref());
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);
        layer.grab_focus();

        if trash_root {
            let weak_size = size.downgrade();
            glib::MainContext::default().spawn_local(async move {
                let summary = summarize_trash(&gio::File::for_uri("trash:///")).await;
                let Some(size) = weak_size.upgrade() else {
                    return;
                };
                match summary {
                    Ok(summary) => {
                        let prefix = if summary.truncated { "≥ " } else { "" };
                        size.set_text(&format!("{prefix}{}", format_file_size(summary.total_size)));
                        size.set_tooltip_text(Some(&format!(
                            "{prefix}{}",
                            item_count_label(summary.item_count)
                        )));
                    }
                    Err(_) => size.set_text("Unavailable"),
                }
            });
        }

        let file = gio_file_for_location(&location);
        glib::MainContext::default().spawn_local(async move {
            let Ok(info) = file
                .query_info_future(
                    "standard::content-type,standard::is-hidden,standard::size,time::modified,unix::mode,owner::user,owner::group",
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    glib::Priority::DEFAULT,
                )
                .await
            else {
                return;
            };
            if !is_directory {
                size.set_text(&format_file_size(info.size().max(0) as u64));
            }
            if let Some(time) = info.modification_date_time() {
                modified.set_text(
                    &time
                        .format("%Y-%m-%d %H:%M")
                        .map(|value| value.to_string())
                        .unwrap_or_else(|_| "—".to_owned()),
                );
            }
            hidden.set_text(if info.is_hidden() { "Yes" } else { "No" });
            if let Some(content_type) = info.content_type() {
                kind.set_text(&gio::content_type_get_description(&content_type));
                if let Some(app) = gio::AppInfo::default_for_type(&content_type, false) {
                    opens_with.set_text(&app.display_name());
                }
            }
            let mode = info.attribute_uint32("unix::mode");
            if mode != 0 {
                permission_editor.mode.set(Some(mode));
                update_permission_editor(&permission_editor, mode);
                set_permission_editor_sensitive(&permission_editor, true);
            }
            owner
                .identity
                .set_text(info.attribute_string("owner::user").as_deref().unwrap_or("—"));
            group
                .identity
                .set_text(info.attribute_string("owner::group").as_deref().unwrap_or("—"));
        });
    }
}

#[cfg(test)]
mod tests;
