// SPDX-License-Identifier: MIT

//! In-place update installation, progress tracking, restart, and package-managed launch.

use std::{cell::Cell, sync::mpsc::Receiver};

use rune::Tx;
use strata_core::{
    error_text::error_detail,
    i18n::{decimal, tr},
};
use strata_desktop::services::{
    InstallCancel, InstallRequest, ManagedInstall, UpdateInstall, UpdateMethod, install_update,
};

use super::{UpdatePhase, UpdatesState};
use crate::{
    App,
    settings_dialog::install_command,
    shell,
    terminal::{Emulator, no_terminal_message},
};

pub(super) const FINALIZING_STATUS: &str = "Finalizing update…";

thread_local! {
    static INSTALL_RUNNING: Cell<bool> = const { Cell::new(false) };
}

pub(super) struct InstallLock;

impl InstallLock {
    pub(super) fn try_acquire() -> Option<Self> {
        INSTALL_RUNNING.with(|running| (!running.replace(true)).then_some(Self))
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        INSTALL_RUNNING.with(|running| running.set(false));
    }
}

pub(super) struct InstallProgressState {
    pub(super) fraction: Option<f32>,
    pub(super) status: String,
}

impl InstallProgressState {
    pub(super) fn starting(returns_to_stable: bool) -> Self {
        Self {
            fraction: Some(0.0),
            status: tr(if returns_to_stable {
                "Downloading stable release…"
            } else {
                "Downloading update…"
            }),
        }
    }

    pub(super) fn apply_downloading(&mut self, downloaded: u64, total: Option<u64>) {
        if let Some(total) = total.filter(|total| *total > 0) {
            let ratio = (downloaded as f64 / total as f64).clamp(0.0, 1.0);
            #[expect(clippy::cast_possible_truncation, reason = "clamped 0.0..=1.0")]
            {
                self.fraction = Some(ratio as f32);
            }
            self.status = rust_i18n::t!(
                "Downloading update… %{value1}%",
                value1 = format!("{:.0}", ratio * 100.0)
            )
            .into_owned();
        } else {
            self.fraction = None;
            self.status = rust_i18n::t!(
                "Downloading update… %{value1} MB",
                value1 = decimal(downloaded as f64 / 1_048_576.0, 1)
            )
            .into_owned();
        }
    }
}

pub(super) fn install_failure_message(message: Option<String>) -> String {
    match message {
        Some(message) => rust_i18n::t!(
            "Couldn't install update: %{message}",
            message = error_detail(message)
        )
        .into_owned(),
        None => tr("Couldn't install update"),
    }
}

pub(super) fn spawn_install(
    state: &UpdatesState,
    request: InstallRequest,
    cancel: InstallCancel,
) -> Receiver<UpdateInstall> {
    #[cfg(test)]
    if let Some(hook) = &state.install_hook {
        return hook(request, cancel);
    }
    let _ = state;
    install_update(request, cancel)
}

pub(super) fn aur_update_action_label(managed: Option<&ManagedInstall>) -> &'static str {
    match managed {
        Some(managed) if managed.aur_update_target().is_some() => "Open AUR Update",
        Some(managed) if managed.package().is_some() => "View on AUR",
        _ => "Close",
    }
}

pub(super) fn launch_managed_update(app: &mut App, method: UpdateMethod) {
    let generation = app.settings_dialog.updates.generation;
    #[cfg(test)]
    if let Some(hook) = app.settings_dialog.updates.managed_launch_hook.clone() {
        let text = hook(method).unwrap_or_else(|error| error);
        set_managed_status(app, generation, text);
        return;
    }

    match method {
        UpdateMethod::Aur => launch_aur_update(app, generation),
        UpdateMethod::Omarchy => launch_omarchy_update(app, generation),
        UpdateMethod::InPlace | UpdateMethod::Pacman => {}
    }
}

fn launch_aur_update(app: &mut App, generation: u64) {
    let managed = app.settings_dialog.updates.managed().cloned();
    let Some(managed) = managed else {
        let error = rust_i18n::t!(
            "Couldn't open AUR update: %{error}",
            error = tr("missing package metadata")
        )
        .into_owned();
        set_managed_status(app, generation, error);
        return;
    };
    if let Some((helper, package)) = managed.aur_update_target() {
        let helper = helper.to_owned();
        let package = package.to_owned();
        let weak = app.settings_dialog.later.downgrade();
        install_command::spawn_reaped(
            weak,
            move || {
                let terminal = Emulator::resolve().ok_or_else(no_terminal_message)?;
                terminal
                    .exec_command(&[helper.as_str(), "-Syu", package.as_str()])
                    .spawn()
                    .map_err(|error| terminal.launch_failure(&error))
            },
            move |app, outcome| {
                let text = match outcome {
                    Ok(()) => tr("AUR update opened in your terminal."),
                    Err(error) => {
                        rust_i18n::t!("Couldn't open AUR update: %{error}", error = error)
                            .into_owned()
                    }
                };
                set_managed_status(app, generation, text)
            },
        );
        return;
    }
    let Some(package) = managed.package() else {
        let error = rust_i18n::t!(
            "Couldn't open AUR update: %{error}",
            error = tr("missing AUR package name")
        )
        .into_owned();
        set_managed_status(app, generation, error);
        return;
    };
    let uri = format!("https://aur.archlinux.org/packages/{package}");
    let text = match super::open_uri(app, &uri) {
        Ok(()) => tr("AUR package page opened."),
        Err(error) => {
            rust_i18n::t!("Couldn't open AUR update: %{error}", error = error).into_owned()
        }
    };
    set_managed_status(app, generation, text);
}

fn launch_omarchy_update(app: &mut App, generation: u64) {
    let weak = app.settings_dialog.later.downgrade();
    install_command::spawn_reaped(
        weak,
        || {
            let terminal = Emulator::resolve().ok_or_else(no_terminal_message)?;
            terminal
                .exec_command(&["omarchy", "update"])
                .spawn()
                .map_err(|error| terminal.launch_failure(&error))
        },
        move |app, outcome| {
            let text = match outcome {
                Ok(()) => tr("Omarchy Update opened in your terminal."),
                Err(error) => {
                    rust_i18n::t!("Couldn't open Omarchy Update: %{error}", error = error)
                        .into_owned()
                }
            };
            set_managed_status(app, generation, text)
        },
    );
}

fn set_managed_status(app: &mut App, generation: u64, status: String) -> bool {
    let updates = &mut app.settings_dialog.updates;
    if updates.generation != generation {
        return false;
    }
    if let UpdatePhase::Available { managed_status, .. } = &mut updates.phase {
        *managed_status = Some(status);
        return true;
    }
    false
}

pub(super) fn restart_after_install(app: &mut App, tx: &mut Tx) {
    #[cfg(test)]
    if let Some(hook) = &app.settings_dialog.updates.restart_hook {
        hook();
        return;
    }
    #[cfg(not(test))]
    spawn_installed_relaunch();
    shell::close_window(app, tx);
}

#[cfg(not(test))]
fn spawn_installed_relaunch() {
    use std::{
        os::unix::process::CommandExt,
        process::{Command, Stdio},
    };
    let Ok(exe) =
        strata_desktop::services::installed_executable().or_else(|_| std::env::current_exe())
    else {
        return;
    };
    let pid = std::process::id().to_string();
    let _ = Command::new("/bin/sh")
        .args([
            "-c",
            "while kill -0 \"$1\" 2>/dev/null; do sleep 0.1; done; exec \"$2\"",
            "strata-restart",
        ])
        .arg(pid)
        .arg(exe)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
}
