// SPDX-License-Identifier: GPL-3.0-or-later

use crate::adapters::gio_file_for_location;
use crate::model::{FileEntry, Location};
use crate::ui::browser::paths::is_trash_location;
use crate::ui::controls::{ModalTone, message_dialog_description, message_dialog_layout};
use crate::ui::modal::{ModalHost, dismiss_modal_layer, modal_layer, show_error_dialog};
use crate::ui::terminal;
use gtk::gio;
use gtk::prelude::*;
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

pub(in crate::ui) fn open_location(location: &Location, parent: &impl IsA<gtk::Widget>) {
    let file = gio_file_for_location(location);
    let uri = file.uri();
    if let Err(error) = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>) {
        if executable_without_handler(location.native_path(), &error) {
            confirm_run_program(location, parent);
            return;
        }
        tracing::warn!(
            backend = %location.backend_name(),
            error_domain = ?error.domain(),
            error_code = error.code(),
            "unable to open file"
        );
        tracing::debug!(
            location = %location.diagnostic_path(),
            "file open location"
        );
        let detail = if error.matches(gio::IOErrorEnum::NotSupported) {
            "No application is registered for this file"
        } else {
            error.message()
        };
        show_error_dialog(parent, "Unable to open file", detail);
    }
}

fn executable_without_handler(path: Option<&Path>, error: &glib::Error) -> bool {
    error.matches(gio::IOErrorEnum::NotSupported) && path.is_some_and(is_regular_executable)
}

fn is_regular_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn confirm_run_program(location: &Location, parent: &impl IsA<gtk::Widget>) {
    let Some(ModalHost {
        overlay: window_overlay,
        blurred_root,
    }) = ModalHost::blurred_for(parent)
    else {
        return;
    };
    let name = location.display_name();
    let layout = message_dialog_layout(
        crate::assets::icons::TERMINAL,
        "Run this program?",
        &name,
        "Run",
        ModalTone::Danger,
    );
    layout.body.append(&message_dialog_description(&format!(
        "\u{201c}{name}\u{201d} is an executable file. Only run programs you trust."
    )));
    let content = layout.content;
    let close = layout.close;
    let cancel = layout.cancel;
    let run = layout.confirm;

    let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
    window_overlay.add_overlay(&layer);
    let weak_cancel = cancel.downgrade();
    gtk::glib::idle_add_local_once(move || {
        if let Some(cancel) = weak_cancel.upgrade() {
            cancel.grab_focus();
        }
    });
    for button in [close, cancel] {
        let dismiss_layer = layer.clone();
        let dismiss_overlay = window_overlay.clone();
        let dismiss_root = blurred_root.clone();
        button.connect_clicked(move |_| {
            dismiss_modal_layer(&dismiss_layer, &dismiss_overlay, dismiss_root.as_ref());
        });
    }
    let run_layer = layer.clone();
    let run_overlay = window_overlay;
    let run_root = blurred_root;
    let run_location = location.clone();
    let error_parent = parent.as_ref().clone();
    run.connect_clicked(move |_| {
        dismiss_modal_layer(&run_layer, &run_overlay, run_root.as_ref());
        if let Err(error) = launch_program(&run_location) {
            tracing::warn!(%error, "unable to run program");
            show_error_dialog(&error_parent, "Unable to run program", &error.to_string());
        }
    });
}

fn launch_program(location: &Location) -> std::io::Result<()> {
    let path = location.native_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "program is not a local file",
        )
    })?;
    let mut child = program_command(path).spawn()?;
    std::thread::spawn(move || {
        if let Err(error) = child.wait() {
            tracing::warn!(%error, "unable to reap program");
        }
    });
    Ok(())
}

fn program_command(path: &Path) -> Command {
    let mut command = Command::new(path);
    command
        .current_dir(path.parent().unwrap_or_else(|| Path::new(".")))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

pub(super) fn can_open_terminal(location: &Location) -> bool {
    location.native_path().is_some() && !is_trash_location(location)
}

pub(super) fn selected_terminal_location(entries: &[FileEntry]) -> Option<Location> {
    let [entry] = entries else {
        return None;
    };
    entry.is_directory().then(|| entry.location.clone())
}

fn terminal_directory_argument(path: &Path) -> OsString {
    let mut argument = OsString::from("--dir=");
    argument.push(path);
    argument
}

pub(in crate::ui) fn launch_terminal(location: &Location, parent: &impl IsA<gtk::Widget>) {
    let Some(path) = location.native_path() else {
        show_error_dialog(
            parent,
            "Unable to open terminal",
            "This location is not a local folder",
        );
        return;
    };
    if is_trash_location(location) {
        show_error_dialog(
            parent,
            "Unable to open terminal",
            "Terminal cannot be opened in Trash",
        );
        return;
    }
    let path = path.to_path_buf();
    tracing::debug!(
        location = %location.diagnostic_path(),
        "opening terminal"
    );
    let result = terminal::command()
        .arg(terminal_directory_argument(&path))
        .spawn();
    if let Err(error) = result {
        tracing::warn!(%error, launcher = terminal::LAUNCHER, "unable to launch terminal");
        show_error_dialog(
            parent,
            "Unable to open terminal",
            &terminal::launch_failure(&error),
        );
    }
}

#[cfg(test)]
mod tests;
