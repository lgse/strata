// SPDX-License-Identifier: GPL-3.0-or-later

use crate::adapters::gio_file_for_location;
use crate::model::{FileEntry, Location};
use crate::ui::browser::paths::is_trash_location;
use crate::ui::modal::show_error_dialog;
use gtk::gio;
use gtk::prelude::*;
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

pub(in crate::ui) fn open_location(location: &Location, parent: &impl IsA<gtk::Widget>) {
    let file = gio_file_for_location(location);
    let uri = file.uri();
    if let Err(error) = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>) {
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
        show_error_dialog(parent, "Unable to open file", &error.to_string());
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
