// SPDX-License-Identifier: GPL-3.0-or-later

use crate::adapters::gio_file_for_location;
use crate::model::{FileEntry, Location};
use crate::ui::browser::paths::is_trash_location;
use crate::ui::controls::{ModalTone, message_dialog_description, message_dialog_layout};
use crate::ui::modal::{ModalHost, dismiss_modal_layer, modal_layer, show_error_dialog};
use gtk::gio;
use gtk::prelude::*;
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

pub(in crate::ui) fn open_location(location: &Location, parent: &impl IsA<gtk::Widget>) {
    let file = gio_file_for_location(location);
    let uri = file.uri();
    if gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>).is_err() {
        // No registered handler: an executable file is meant to be run, not
        // opened. Offer that, matching Nautilus, instead of a dead end.
        let path = location.native_path();
        if path.is_some_and(|path| path.is_file() && is_executable(path)) {
            confirm_run_program(location, parent);
            return;
        }
        tracing::warn!(
            backend = %location.backend_name(),
            "unable to open file"
        );
        tracing::debug!(
            location = %location.diagnostic_path(),
            "file open location"
        );
        show_error_dialog(
            parent,
            "Unable to open file",
            "No application is registered for this file",
        );
    }
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.permissions().mode() & 0o111 != 0)
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
    let cancel = layout.cancel;
    let run = layout.confirm;

    let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
    window_overlay.add_overlay(&layer);
    let cancel_layer = layer.clone();
    let cancel_overlay = window_overlay.clone();
    let cancel_root = blurred_root.clone();
    cancel.connect_clicked(move |_| {
        dismiss_modal_layer(&cancel_layer, &cancel_overlay, cancel_root.as_ref());
    });
    let run_layer = layer.clone();
    let run_overlay = window_overlay;
    let run_root = blurred_root;
    let run_location = location.clone();
    run.connect_clicked(move |_| {
        dismiss_modal_layer(&run_layer, &run_overlay, run_root.as_ref());
        launch_program(&run_location);
    });
    run.grab_focus();
}

fn launch_program(location: &Location) {
    let Some(path) = location.native_path().map(Path::to_path_buf) else {
        return;
    };
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let program = path.clone();
    // The child outlives the GTK callback; spawn must not block the main loop
    // if the program's startup stalls, so the spawn itself runs off-thread.
    let result = std::thread::spawn(move || {
        Command::new(&program)
            .current_dir(&parent)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    })
    .join();
    match result {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => tracing::warn!(%error, "unable to run program"),
        Err(_) => tracing::warn!("run thread panicked"),
    }
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
    let result = Command::new("xdg-terminal-exec")
        .arg(terminal_directory_argument(&path))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Err(error) = result {
        tracing::warn!(%error, "unable to launch terminal");
        show_error_dialog(parent, "Unable to open terminal", &error.to_string());
    }
}

#[cfg(test)]
mod tests;
