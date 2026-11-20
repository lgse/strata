// SPDX-License-Identifier: MIT

//! Handing package-managed installs to their package manager: the AUR helper
//! or Omarchy's updater in a terminal, or the package's AUR page.

use std::process::Command;

use gtk::gio;

use super::super::terminal;
use crate::services::{InstallSource, UpdateMethod};

pub(super) fn aur_update_action_label() -> &'static str {
    match InstallSource::detect().managed() {
        Some(managed) if managed.aur_update_target().is_some() => "Open AUR Update",
        Some(managed) if managed.package().is_some() => "View on AUR",
        _ => "Close",
    }
}

fn aur_update_command(terminal: &terminal::Terminal, helper: &str, package: &str) -> Command {
    terminal.exec_command(&[helper, "-Syu", package])
}

pub(super) async fn launch_managed_update(method: UpdateMethod) -> Result<String, String> {
    if method == UpdateMethod::Aur {
        return launch_aur_update().await.map_err(|error| {
            rust_i18n::t!("Couldn't open AUR update: %{error}", error = error).into_owned()
        });
    }
    launch_omarchy_update()
        .await
        .map(|()| crate::i18n::tr("Omarchy Update opened in your terminal."))
        .map_err(|error| {
            rust_i18n::t!("Couldn't open Omarchy Update: %{error}", error = error).into_owned()
        })
}

pub(super) async fn launch_aur_update() -> Result<String, String> {
    let managed = InstallSource::detect()
        .managed()
        .ok_or_else(|| crate::i18n::tr("missing package metadata"))?;
    if let Some((helper, package)) = managed.aur_update_target() {
        return terminal::spawn_reaped(move || {
            let terminal =
                terminal::Terminal::resolve().ok_or_else(terminal::no_terminal_message)?;
            aur_update_command(&terminal, helper, package)
                .spawn()
                .map_err(|error| terminal.launch_failure(&error))
        })
        .await
        .map(|()| crate::i18n::tr("AUR update opened in your terminal."));
    }
    let package = managed
        .package()
        .ok_or_else(|| crate::i18n::tr("missing AUR package name"))?;
    let uri = format!("https://aur.archlinux.org/packages/{package}");
    let context = gtk::gdk::Display::default()
        .map(|display| crate::ui::launch_context::for_display(&display));
    gio::AppInfo::launch_default_for_uri(&uri, context.as_ref())
        .map(|()| crate::i18n::tr("AUR package page opened."))
        .map_err(|error| error.to_string())
}

fn omarchy_update_command(terminal: &terminal::Terminal) -> Command {
    terminal.exec_command(&["omarchy", "update"])
}

async fn launch_omarchy_update() -> Result<(), String> {
    terminal::spawn_reaped(|| {
        if let Some(mut command) = terminal::omarchy_presentation_command(
            std::env::var_os("PATH").as_deref(),
            "omarchy-update",
        ) {
            match command.spawn() {
                Ok(child) => return Ok(child),
                Err(error) => tracing::warn!(
                    %error,
                    "Omarchy presentation launcher failed to start; falling back to a plain terminal"
                ),
            }
        }
        let terminal = terminal::Terminal::resolve().ok_or_else(terminal::no_terminal_message)?;
        omarchy_update_command(&terminal)
            .spawn()
            .map_err(|error| terminal.launch_failure(&error))
    })
    .await
}
