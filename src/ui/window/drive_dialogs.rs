// SPDX-License-Identifier: MIT

//! Shared volume/mount properties and removable-drive Format/Rename dialogs.

use std::{cell::Cell, future::Future, rc::Rc};

use gtk::{gio, glib, prelude::*};

use crate::{
    assets,
    services::package_manager::PackageManager,
    ui::{
        controls::{
            FormTextField, ModalLayout, ModalTone, ProgressSummary, form_check_button, form_label,
            modal_layout, progress_summary, properties_action,
        },
        missing_tools::{MissingTool, show_missing_tools},
        modal::{ModalHost, dismiss_modal_layer, modal_layer, remember_modal_focus},
    },
};

use super::drive_ops::{self, FilesystemType};

mod format_progress;

#[cfg(test)]
pub(super) mod tests;

pub(super) fn open_from_sidebar(
    view: &super::BrowserView,
    popover: Option<&gtk::Popover>,
    show: impl FnOnce(),
) {
    if let Some(popover) = popover {
        popover.popdown();
    }
    show();
    let window = view.widget().root().and_downcast::<gtk::Window>();
    if let Some(layer) = window.as_ref().and_then(super::visible_modal_layer) {
        let view = view.downgrade();
        crate::ui::modal::set_modal_focus_restore(
            &layer,
            Rc::new(move || {
                if let Some(view) = view.upgrade() {
                    // Resolve the current pane after formatting or a view rebuild.
                    view.focus_file_view();
                }
            }),
        );
    } else {
        view.focus_file_view();
    }
}

#[derive(Clone)]
pub(super) enum PropertiesTarget {
    Volume(gio::Volume),
    Mount(gio::Mount),
}

impl PropertiesTarget {
    pub(super) fn volume(&self) -> Option<gio::Volume> {
        match self {
            Self::Volume(volume) => Some(volume.clone()),
            Self::Mount(mount) => mount.volume(),
        }
    }

    fn mount(&self) -> Option<gio::Mount> {
        match self {
            Self::Volume(volume) => volume.get_mount(),
            Self::Mount(mount) => Some(mount.clone()),
        }
    }

    fn name(&self) -> String {
        match self {
            Self::Volume(volume) => volume.name().to_string(),
            Self::Mount(mount) => mount.name().to_string(),
        }
    }
}

#[derive(Clone, Copy)]
enum PropertyAction {
    Rename,
    Eject,
    Format,
}

fn property_action_control(
    action: PropertyAction,
    editable: bool,
    can_release: bool,
) -> Option<gtk::Button> {
    if !editable && (!matches!(action, PropertyAction::Eject) || !can_release) {
        return None;
    }
    let (label, icon, tone) = match action {
        PropertyAction::Rename => ("Rename", assets::icons::PENCIL, ModalTone::Accent),
        PropertyAction::Eject => ("Eject", assets::icons::EJECT, ModalTone::Accent),
        PropertyAction::Format => ("Format", assets::icons::SHREDDER, ModalTone::Danger),
    };
    let button = properties_action(icon, label, tone);
    if matches!(action, PropertyAction::Eject) {
        button.set_sensitive(can_release);
    }
    Some(button)
}

struct ModalShell {
    layout: ModalLayout,
    layer: gtk::Box,
    overlay: gtk::Overlay,
    blurred_root: Option<crate::ui::blur::BlurBin>,
}

fn modal_shell(
    parent: &gtk::Widget,
    icon: &str,
    title: &str,
    subtitle: &str,
    confirm_label: &str,
    danger: bool,
) -> Option<ModalShell> {
    let host = ModalHost::blurred_for(parent)?;
    let layout = modal_layout(icon, title, subtitle, "Cancel");
    if danger {
        layout.content.add_css_class("destructive");
    }
    layout.confirm.set_label(confirm_label);
    layout.confirm.add_css_class("suggested-action");
    let layer = modal_layer(
        &layout.content,
        &host.overlay,
        host.blurred_root.clone(),
        Some(Rc::new(|| true)),
    );
    remember_modal_focus(&layer, &host.overlay);
    host.overlay.add_overlay(&layer);
    Some(ModalShell {
        layout,
        layer,
        overlay: host.overlay,
        blurred_root: host.blurred_root,
    })
}

fn field_block(label_text: &str, field: &impl IsA<gtk::Widget>) -> gtk::Box {
    let block = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let label = form_label(label_text);
    label.set_xalign(0.0);
    block.append(&label);
    field.set_hexpand(true);
    block.append(field);
    block
}

fn required_drive_tool(fs: FilesystemType, name: &'static str) -> MissingTool<'static> {
    use PackageManager::{Apt, Dnf, Pacman, Zypper};
    let packages: &[(PackageManager, &str)] = match fs {
        FilesystemType::Fat32 => &[
            (Pacman, "dosfstools"),
            (Apt, "dosfstools"),
            (Dnf, "dosfstools"),
            (Zypper, "dosfstools"),
        ],
        FilesystemType::Ntfs => &[
            (Pacman, "ntfsprogs"),
            (Apt, "ntfs-3g"),
            (Dnf, "ntfsprogs"),
            (Zypper, "ntfs-3g"),
        ],
        FilesystemType::Exfat => &[
            (Pacman, "exfatprogs"),
            (Apt, "exfatprogs"),
            (Dnf, "exfatprogs"),
            (Zypper, "exfatprogs"),
        ],
    };
    MissingTool { name, packages }
}

fn inline_error() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class("form-error");
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_visible(false);
    label
}

fn show_inline_error(label: &gtk::Label, message: &str) {
    label.set_text(message);
    label.set_visible(true);
}

fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn capacity_summary(total: u64, available: u64) -> ProgressSummary {
    let used = total.saturating_sub(available);
    let fraction = used as f64 / total.max(1) as f64;
    let summary = progress_summary("Used");
    summary
        .amount
        .set_text(&format!("{} / {}", human_size(used), human_size(total)));
    summary.percent.set_visible(false);
    summary.progress.set_fraction(fraction);
    summary
}

fn refresh_format_validity(
    available: &[FilesystemType],
    fs_combo: &gtk::DropDown,
    label_entry: &gtk::Entry,
    confirm_btn: &gtk::Button,
) {
    let fs_type = available.get(fs_combo.selected() as usize).copied();
    refresh_format_selection(
        fs_type,
        fs_type.is_some_and(|fs| fs.available()),
        label_entry,
        confirm_btn,
    );
}

fn refresh_format_selection(
    fs_type: Option<FilesystemType>,
    tools_available: bool,
    label_entry: &gtk::Entry,
    confirm_btn: &gtk::Button,
) {
    let Some(fs_type) = fs_type else {
        confirm_btn.set_sensitive(false);
        return;
    };
    let max_length = fs_type.max_label_len() as i32;
    if label_entry.max_length() != max_length {
        label_entry.set_max_length(max_length);
    }
    confirm_btn.set_sensitive(
        tools_available && label_entry.text().chars().count() <= fs_type.max_label_len(),
    );
}

fn wire_entry_submission(entry: &gtk::Entry, confirm: &gtk::Button) {
    let confirm = confirm.downgrade();
    entry.connect_activate(move |_| {
        if let Some(confirm) = confirm.upgrade()
            && confirm.is_sensitive()
        {
            confirm.emit_clicked();
        }
    });
}

fn rename_validation_error(
    label: &str,
    current_name: &str,
    filesystem: FilesystemType,
) -> Option<String> {
    let label = label.trim();
    if label.is_empty() {
        Some("The label cannot be empty.".to_owned())
    } else if label == current_name.trim() {
        Some("Enter a label different from the current one.".to_owned())
    } else if label.chars().count() > filesystem.max_label_len() {
        Some(format!(
            "Labels on this volume hold at most {} characters.",
            filesystem.max_label_len()
        ))
    } else {
        filesystem.label_character_error(label)
    }
}

fn refresh_rename_validity(
    entry: &gtk::Entry,
    current_name: &str,
    filesystem: FilesystemType,
    tools_available: bool,
    confirm: &gtk::Button,
    error: &gtk::Label,
) {
    let text = entry.text();
    let message = rename_validation_error(&text, current_name, filesystem);
    confirm.set_sensitive(tools_available && message.is_none());
    let invalid = message.is_some() && text != current_name;
    if invalid {
        entry.add_css_class("error");
        show_inline_error(error, message.as_deref().expect("invalid label"));
    } else {
        entry.remove_css_class("error");
        error.set_visible(false);
        error.set_text("");
    }
    crate::ui::accessibility::set_description(entry, message.as_deref().filter(|_| invalid));
    entry.update_state(&[gtk::accessible::State::Invalid(if invalid {
        gtk::AccessibleInvalidState::True
    } else {
        gtk::AccessibleInvalidState::False
    })]);
}

/// Wire Cancel and Escape. Each dialog connects its own confirm button.
fn wire_modal_close(shell: &ModalShell) {
    let layer = shell.layer.clone();
    let overlay = shell.overlay.clone();
    let root = shell.blurred_root.clone();
    let close: Rc<dyn Fn()> = Rc::new(move || {
        dismiss_modal_layer(&layer, &overlay, root.as_ref());
    });
    let cancelled = close.clone();
    shell.layout.cancel.connect_clicked(move |_| cancelled());
    let closed = close.clone();
    shell.layout.close.connect_clicked(move |_| closed());
    let escaped = close.clone();
    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            escaped();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    shell.layer.add_controller(escape);
    shell.layout.confirm.grab_focus();
}

/// Properties: name, device, filesystem, mount status, capacity bar.
/// Everything is read synchronously; unavailable data shows as em dash.
pub(super) fn show_drive_properties(
    parent: &gtk::Widget,
    target: &PropertiesTarget,
    on_release: Option<Rc<dyn Fn()>>,
) {
    let name = target.name();
    let volume = target.volume();
    let mount = target.mount();
    let Some(shell) = modal_shell(
        parent,
        assets::icons::INFO,
        "Properties",
        &name,
        "Close",
        false,
    ) else {
        return;
    };

    shell
        .layout
        .content
        .add_css_class("drive-properties-content");
    shell.layout.subtitle.set_max_width_chars(44);
    shell
        .layout
        .subtitle
        .set_ellipsize(gtk::pango::EllipsizeMode::End);

    let grid = gtk::Grid::new();
    grid.set_column_spacing(12);
    grid.set_row_spacing(8);
    grid.set_hexpand(true);
    grid.set_column_homogeneous(false);

    let mut row = 0;
    let mut add_row = |label_text: &str, value_text: &str| {
        let label = form_label(label_text);
        label.set_xalign(0.0);
        let value = gtk::Label::new(Some(value_text));
        value.set_xalign(0.0);
        value.set_hexpand(true);
        value.set_selectable(true);
        value.set_ellipsize(gtk::pango::EllipsizeMode::End);
        value.set_max_width_chars(48);
        grid.attach(&label, 0, row, 1, 1);
        grid.attach(&value, 1, row, 1, 1);
        row += 1;
    };

    add_row("Name", &name);
    let block_device = volume
        .as_ref()
        .and_then(drive_ops::block_device_for_volume)
        .or_else(|| {
            mount
                .as_ref()
                .and_then(|mount| mount.root().path())
                .as_deref()
                .and_then(drive_ops::block_device_for_path)
        });
    add_row(
        "Device",
        &block_device
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "—".to_owned()),
    );

    let filesystem = block_device
        .as_ref()
        .and_then(|device| drive_ops::filesystem_label_for_device(device))
        .or_else(|| {
            mount
                .as_ref()
                .and_then(|mount| {
                    mount
                        .root()
                        .query_filesystem_info("filesystem::type", gio::Cancellable::NONE)
                        .ok()
                })
                .and_then(|info| {
                    info.attribute_string("filesystem::type")
                        .map(|name| name.to_string())
                })
        })
        .unwrap_or_else(|| "Unknown".to_owned());
    add_row("Filesystem", &filesystem);

    let total_bytes = block_device
        .as_ref()
        .and_then(|device| drive_ops::device_size_bytes(device));
    match mount {
        Some(mount) => {
            let location = mount
                .root()
                .path()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "—".to_owned());
            add_row("Mount point", &location);
            add_row("Status", "Mounted");
            let usage = mount
                .root()
                .path()
                .and_then(|root| drive_ops::usage_for_path(&root));
            let total = usage
                .map(|(total, _)| total)
                .filter(|total| *total > 0)
                .or(total_bytes);
            match (total, usage) {
                (Some(total), Some((_, available))) => {
                    let used = total.saturating_sub(available);
                    add_row("Capacity", &human_size(total));
                    add_row("Used", &human_size(used));
                    add_row("Free", &human_size(available));
                    let summary = capacity_summary(total, available);
                    summary.widget.set_margin_top(8);
                    grid.attach(&summary.widget, 0, row, 2, 1);
                }
                (Some(total), None) => {
                    add_row("Capacity", &human_size(total));
                    add_row("Used", "Unavailable");
                }
                (None, _) => add_row("Capacity", "Unavailable"),
            }
        }
        None => {
            add_row("Status", "Not mounted");
            if let Some(total) = total_bytes {
                add_row("Capacity", &human_size(total));
            }
        }
    }

    shell.layout.body.append(&grid);

    shell.layout.confirm.set_visible(false);
    shell.layout.cancel.set_visible(false);
    while let Some(child) = shell.layout.actions.first_child() {
        shell.layout.actions.remove(&child);
    }
    shell.layout.actions.add_css_class("properties-actions");
    shell.layout.actions.set_homogeneous(true);
    let responsive_actions = shell.layout.actions.clone();
    shell.layout.content.add_tick_callback(move |content, _| {
        let compact = content.has_css_class("modal-constrained");
        let orientation = if compact {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        };
        if responsive_actions.orientation() != orientation {
            responsive_actions.set_orientation(orientation);
            responsive_actions.set_homogeneous(!compact);
        }
        glib::ControlFlow::Continue
    });
    wire_modal_close(&shell);
    shell.layout.close.grab_focus();
    let editable = drive_ops::is_eligible(volume.as_ref());
    shell
        .layout
        .actions
        .set_visible(editable || on_release.is_some());
    for action in [
        PropertyAction::Rename,
        PropertyAction::Eject,
        PropertyAction::Format,
    ] {
        let Some(button) = property_action_control(action, editable, on_release.is_some()) else {
            continue;
        };
        let parent = parent.clone();
        let volume = volume.clone();
        let layer = shell.layer.clone();
        let overlay = shell.overlay.clone();
        let root = shell.blurred_root.clone();
        let on_release = on_release.clone();
        button.connect_clicked(move |_| {
            dismiss_modal_layer(&layer, &overlay, root.as_ref());
            match action {
                PropertyAction::Rename => {
                    if let Some(volume) = &volume {
                        show_rename_dialog(&parent, volume);
                    }
                }
                PropertyAction::Format => {
                    if let Some(volume) = &volume {
                        show_format_dialog(&parent, volume);
                    }
                }
                PropertyAction::Eject => {
                    if let Some(on_release) = &on_release {
                        on_release();
                    }
                }
            }
        });
        shell.layout.actions.append(&button);
        if matches!(action, PropertyAction::Rename) || !editable {
            button.grab_focus();
        }
    }
}

pub(super) fn show_rename_dialog(parent: &gtk::Widget, volume: &gio::Volume) {
    let detected_fs = drive_ops::block_device_for_volume(volume)
        .map(|device| drive_ops::filesystem_of_device(&device));
    let fs = detected_fs.unwrap_or(FilesystemType::Fat32);
    if !fs.label_tool_available() {
        show_missing_tools(
            parent,
            "Renaming this volume requires an additional filesystem label tool.",
            &[required_drive_tool(fs, fs.label_cmd())],
        );
        return;
    }
    let name = volume.name().to_string();
    let volume = volume.clone();
    let Some(shell) = modal_shell(
        parent,
        assets::icons::PENCIL,
        "Rename Volume",
        &name,
        "Rename",
        false,
    ) else {
        return;
    };

    shell.layout.content.set_width_request(480);
    shell.layout.subtitle.set_max_width_chars(32);
    shell
        .layout
        .subtitle
        .set_ellipsize(gtk::pango::EllipsizeMode::End);

    let max_len = detected_fs.map(|fs| fs.max_label_len()).unwrap_or(11);
    let filesystem = gtk::Label::new(Some(detected_fs.map(|fs| fs.label()).unwrap_or("Unknown")));
    filesystem.set_xalign(0.0);
    shell
        .layout
        .body
        .append(&field_block("Filesystem", &filesystem));

    let field = FormTextField::with_character_limit(max_len as i32);
    let entry = field.entry;
    entry.set_placeholder_text(Some("Volume label"));
    entry.set_max_width_chars(32);
    entry.set_text(&name);
    entry.select_region(0, -1);
    shell
        .layout
        .body
        .append(&field_block("Label", &field.widget));

    // The label tools need exclusive access, so a mounted volume is
    // unmounted first. Only say so when that actually applies.
    if volume.get_mount().is_some() {
        let hint = gtk::Label::new(Some(
            "This volume is currently mounted. It will be unmounted to apply the new label; click it in the sidebar afterwards to mount it again.",
        ));
        hint.add_css_class("dim-label");
        hint.set_xalign(0.0);
        hint.set_max_width_chars(40);
        hint.set_wrap(true);
        hint.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        shell.layout.body.append(&hint);
    }

    let error = inline_error();
    error.set_max_width_chars(26);
    error.set_hexpand(true);
    shell.layout.actions.prepend(&error);

    let confirm = shell.layout.confirm.clone();
    refresh_rename_validity(
        &entry,
        &name,
        fs,
        fs.label_tool_available(),
        &confirm,
        &error,
    );
    {
        let confirm = confirm.downgrade();
        let current_name = name.clone();
        let error = error.downgrade();
        entry.connect_changed(move |entry| {
            let Some(confirm) = confirm.upgrade() else {
                return;
            };
            let Some(error) = error.upgrade() else {
                return;
            };
            refresh_rename_validity(
                entry,
                &current_name,
                fs,
                fs.label_tool_available(),
                &confirm,
                &error,
            );
        });
    }

    wire_entry_submission(&entry, &confirm);
    let shell_layer = shell.layer.clone();
    let shell_overlay = shell.overlay.clone();
    let shell_root = shell.blurred_root.clone();
    let parent = parent.clone();
    let fired = Rc::new(Cell::new(false));
    let submitted_entry = entry.clone();
    shell.layout.confirm.connect_clicked(move |button| {
        if fired.get() || !button.is_sensitive() {
            return;
        }
        let new_label = submitted_entry.text().to_string();
        if rename_validation_error(&new_label, &name, fs).is_some() {
            refresh_rename_validity(
                &submitted_entry,
                &name,
                fs,
                fs.label_tool_available(),
                button,
                &error,
            );
            return;
        }
        fired.set(true);
        dismiss_modal_layer(&shell_layer, &shell_overlay, shell_root.as_ref());
        let display = name.clone();
        let task_parent = parent.clone();
        let task_volume = volume.clone();
        drive_ops::spawn_drive_task(task_parent.clone(), display, move || {
            let volume = task_volume.clone();
            let new_label = new_label.clone();
            async move { drive_ops::rename_volume(task_parent.clone(), volume, new_label).await }
        });
    });
    wire_modal_close_except_confirm(&shell);
    entry.grab_focus();
}

fn run_format_with_feedback<F, Fut>(parent: &gtk::Widget, display_name: &str, task: F)
where
    F: FnOnce(gtk::Widget) -> Fut + 'static,
    Fut: Future<Output = Result<(), drive_ops::DriveOpError>> + 'static,
{
    let Some(progress) = format_progress::FormatProgress::new(parent, display_name) else {
        return;
    };
    let task_parent = parent
        .root()
        .and_downcast::<gtk::Window>()
        .map(|window| window.upcast::<gtk::Widget>())
        .unwrap_or_else(|| parent.clone());
    let hold = parent
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|window| window.application())
        .map(|application| application.hold());
    glib::MainContext::default().spawn_local(async move {
        let _hold = hold;
        progress.complete(task(task_parent).await);
    });
}

fn format_filesystem_selector(
    available: &[FilesystemType],
    current: Option<&str>,
) -> gtk::DropDown {
    let options: Vec<String> = available
        .iter()
        .map(|fs| {
            if current == Some(fs.label()) {
                format!("{} (current)", fs.label())
            } else {
                fs.label().to_owned()
            }
        })
        .collect();
    let labels: Vec<&str> = options.iter().map(String::as_str).collect();
    let selector = gtk::DropDown::from_strings(&labels);
    selector.add_css_class("form-control");
    selector.set_selected(
        available
            .iter()
            .position(|fs| current == Some(fs.label()))
            .unwrap_or(0) as u32,
    );
    selector
}

/// Format in two steps: configure on step one, then review a summary and
/// press the confirm button a second time.
pub(super) fn show_format_dialog(parent: &gtk::Widget, volume: &gio::Volume) {
    let available = vec![
        FilesystemType::Fat32,
        FilesystemType::Ntfs,
        FilesystemType::Exfat,
    ];
    let missing: Vec<_> = available
        .iter()
        .filter(|fs| !fs.available())
        .map(|fs| required_drive_tool(*fs, fs.format_tool_name()))
        .collect();
    if !missing.is_empty() {
        show_missing_tools(
            parent,
            "Formatting drives requires additional filesystem tools.",
            &missing,
        );
        return;
    }
    let name = volume.name().to_string();
    let block_device = drive_ops::block_device_for_volume(volume);
    let current_filesystem = block_device
        .as_deref()
        .and_then(drive_ops::filesystem_label_for_device);
    let device = block_device
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "unknown device".to_owned());
    let subtitle = format!("{name} ({device})");
    let volume = volume.clone();
    let Some(shell) = modal_shell(
        parent,
        assets::icons::TRIANGLE_ALERT,
        "Format Drive",
        &subtitle,
        "Continue",
        true,
    ) else {
        return;
    };

    shell.layout.content.set_width_request(560);

    // Configuration
    let step1 = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let fs_combo = format_filesystem_selector(&available, current_filesystem.as_deref());
    step1.append(&field_block("Filesystem", &fs_combo));

    let label_field = FormTextField::with_character_limit(
        available[fs_combo.selected() as usize].max_label_len() as i32,
    );
    let label_entry = label_field.entry;
    label_entry.set_placeholder_text(Some("Volume label (optional)"));
    step1.append(&field_block("Label", &label_field.widget));

    let quick_check = form_check_button("Quick format");
    quick_check.set_active(true);
    let check_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    check_row.append(&quick_check);
    step1.append(&check_row);

    if volume.get_mount().is_some() {
        let mount_note = gtk::Label::new(Some(
            "This volume is currently mounted. It will be unmounted to format it; click it in the sidebar afterwards to mount it again.",
        ));
        mount_note.add_css_class("dim-label");
        mount_note.set_xalign(0.0);
        mount_note.set_max_width_chars(48);
        mount_note.set_wrap(true);
        mount_note.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        step1.append(&mount_note);
    }
    shell.layout.body.append(&step1);

    // Explicit confirmation
    let step2 = gtk::Box::new(gtk::Orientation::Vertical, 12);
    step2.set_visible(false);
    let summary = gtk::Label::new(None);
    summary.set_xalign(0.0);
    summary.set_max_width_chars(48);
    summary.set_wrap(true);
    summary.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    step2.append(&summary);
    let warning = gtk::Label::new(Some(
        "This permanently erases ALL DATA on this volume. This cannot be undone.",
    ));
    warning.add_css_class("form-message");
    warning.add_css_class("error");
    warning.set_xalign(0.0);
    warning.set_max_width_chars(48);
    warning.set_wrap(true);
    warning.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    step2.append(&warning);
    shell.layout.body.append(&step2);

    let confirm_btn = shell.layout.confirm.clone();
    {
        let available = available.clone();
        let fs_combo = fs_combo.clone();
        let label_entry = label_entry.clone();
        let confirm_btn = confirm_btn.clone();
        label_entry.clone().connect_changed(move |_| {
            refresh_format_validity(&available, &fs_combo, &label_entry, &confirm_btn);
        });
    }
    {
        let available = available.clone();
        let fs_combo = fs_combo.clone();
        let label_entry = label_entry.clone();
        let confirm_btn = confirm_btn.clone();
        fs_combo
            .clone()
            .connect_notify_local(Some("selected"), move |_, _| {
                refresh_format_validity(&available, &fs_combo, &label_entry, &confirm_btn);
            });
    }
    refresh_format_validity(&available, &fs_combo, &label_entry, &confirm_btn);
    wire_entry_submission(&label_entry, &confirm_btn);

    let shell_layer = shell.layer.clone();
    let shell_overlay = shell.overlay.clone();
    let shell_root = shell.blurred_root.clone();
    let parent = parent.clone();
    let armed = Rc::new(Cell::new(false));
    let fired = Rc::new(Cell::new(false));
    let submitted_label_entry = label_entry.clone();
    let focus_selector = fs_combo.clone();
    shell.layout.confirm.connect_clicked(move |_| {
        if fired.get() {
            return;
        }
        if !armed.get() {
            let Some(fs_type) = available.get(fs_combo.selected() as usize).copied() else {
                return;
            };
            let label = submitted_label_entry.text().to_string();
            if !fs_type.available() || label.chars().count() > fs_type.max_label_len() {
                return;
            }
            let size = drive_ops::block_device_for_volume(&volume)
                .and_then(|device| drive_ops::device_size_bytes(&device))
                .map(human_size)
                .unwrap_or_else(|| "unknown size".to_owned());
            summary.set_text(&format!(
                "Drive: {} ({}, {})\nFilesystem: {}\nLabel: {}\nMode: {}",
                name,
                device,
                size,
                fs_type.label(),
                if label.is_empty() { "(none)" } else { &label },
                if quick_check.is_active() {
                    "Quick format"
                } else {
                    "Full format"
                },
            ));
            step1.set_visible(false);
            step2.set_visible(true);
            confirm_btn.set_label("Format");
            confirm_btn.grab_focus();
            armed.set(true);
            return;
        }
        fired.set(true);
        dismiss_modal_layer(&shell_layer, &shell_overlay, shell_root.as_ref());
        let Some(fs_type) = available.get(fs_combo.selected() as usize).copied() else {
            return;
        };
        let label = submitted_label_entry.text().to_string();
        let quick = quick_check.is_active();
        let task_volume = volume.clone();
        run_format_with_feedback(
            &parent,
            &format!("{name} ({device})"),
            move |task_parent| async move {
                drive_ops::format_volume(task_parent, task_volume, fs_type, label, quick).await
            },
        );
    });
    wire_modal_close_except_confirm(&shell);
    focus_selector.grab_focus();
}

/// Wire Cancel and Escape without touching the confirm button, which each
/// dialog connects itself.
fn wire_modal_close_except_confirm(shell: &ModalShell) {
    let layer = shell.layer.clone();
    let overlay = shell.overlay.clone();
    let root = shell.blurred_root.clone();
    let close: Rc<dyn Fn()> = Rc::new(move || {
        dismiss_modal_layer(&layer, &overlay, root.as_ref());
    });
    let cancelled = close.clone();
    shell.layout.cancel.connect_clicked(move |_| cancelled());
    let closed = close.clone();
    shell.layout.close.connect_clicked(move |_| closed());
    let escaped = close.clone();
    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            escaped();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    shell.layer.add_controller(escape);
    shell.layout.confirm.grab_focus();
}
