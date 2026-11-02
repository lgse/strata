// SPDX-License-Identifier: MIT

//! Relaunching the installed executable after an update, and the rollback copy
//! the restart waiter falls back to.

use std::{
    cell::Cell,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

use crate::services;

const RESTART_GRACE_SECONDS: u64 = 20;
// Must outlast the restart waiter's recovery window.
const ROLLBACK_RETENTION: Duration = Duration::from_secs(60);

pub(crate) fn schedule_rollback_cleanup() {
    schedule_rollback_cleanup_with(
        || {
            let current_exe = services::installed_executable().ok()?;
            Some(services::rollback_path(current_exe.parent()?))
        },
        ROLLBACK_RETENTION,
        remove_unless_replaced,
    );
}

pub(super) fn schedule_rollback_cleanup_with(
    locate: impl FnOnce() -> Option<PathBuf> + Send + 'static,
    retention: Duration,
    remove: impl FnOnce(&Path, &std::fs::Metadata) + Send + 'static,
) {
    glib::MainContext::default().spawn_local(async move {
        let preserved = gio::spawn_blocking(move || {
            crate::util::ui_thread::assert_off();
            let rollback = locate()?;
            let preserved = std::fs::symlink_metadata(&rollback).ok()?;
            preserved.is_file().then_some((rollback, preserved))
        });
        let Ok(Some((rollback, preserved))) = preserved.await else {
            return;
        };
        glib::timeout_future(retention).await;
        let _removed = gio::spawn_blocking(move || remove(&rollback, &preserved)).await;
    });
}

pub(super) fn remove_unless_replaced(rollback: &Path, preserved: &std::fs::Metadata) {
    use std::os::unix::fs::MetadataExt;
    crate::util::ui_thread::assert_off();
    // A later install owns its own rollback copy, even in this process.
    if !std::fs::symlink_metadata(rollback)
        .is_ok_and(|current| current.dev() == preserved.dev() && current.ino() == preserved.ino())
    {
        return;
    }
    if let Err(error) = std::fs::remove_file(rollback) {
        tracing::warn!(%error, "could not remove the preserved previous version");
    }
}

/// Relaunches the (just-updated) executable and quits the current instance.
pub(super) fn restart_application(button: &gtk::Button) {
    let application = button
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok())
        .and_then(|window| window.application());
    restart(application.as_ref());
}

fn process_start_time(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // comm (field 2) may contain spaces or parentheses; the remaining fields
    // begin after its final closing parenthesis. starttime is field 22.
    stat.rsplit_once(") ")?
        .1
        .split_whitespace()
        .nth(19)
        .map(str::to_owned)
}

pub(super) fn restart_waiter(current_exe: &std::path::Path, parent_pid: u32) -> Option<Command> {
    use std::{os::unix::process::CommandExt, process::Stdio};

    crate::util::ui_thread::assert_off();
    let mut command = crate::trusted_command::command("sh").ok()?;
    let sleep = crate::trusted_command::resolve("sleep").ok()?;
    let start_time = process_start_time(parent_pid).unwrap_or_default();
    let date = crate::trusted_command::resolve("date").ok()?;
    let mv = crate::trusted_command::resolve("mv").ok()?;
    let rollback = current_exe.parent().map(services::rollback_path)?;
    command
        .args([
            "-c",
            "pid=$1; binary=$2; sleeper=$3; born=$4; rollback=$5; clock=$6; mover=$7; grace=$8; while kill -0 \"$pid\" 2>/dev/null && [ -r \"/proc/$pid/stat\" ]; do IFS= read -r stat < \"/proc/$pid/stat\" || break; rest=${stat##*) }; set -- $rest; shift 19; [ \"${1:-}\" = \"$born\" ] || break; \"$sleeper\" 0.1; done; \"$sleeper\" 0.5; started=$(\"$clock\" +%s); \"$binary\" && exit 0; status=$?; [ $(($(\"$clock\" +%s) - started)) -lt \"$grace\" ] || exit \"$status\"; [ -f \"$rollback\" ] || exit \"$status\"; \"$mover\" -f -- \"$rollback\" \"$binary\" && exec \"$binary\"; exit \"$status\"",
            "strata-restart",
        ])
        .arg(parent_pid.to_string())
        .arg(current_exe)
        .arg(sleep)
        .arg(start_time)
        .arg(rollback)
        .arg(date)
        .arg(mv)
        .arg(RESTART_GRACE_SECONDS.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    if std::env::var_os(crate::CAIRO_SELECTED_BY_STRATA).as_deref()
        == Some(std::ffi::OsStr::new("1"))
        && std::env::var_os("GSK_RENDERER").as_deref() == Some(std::ffi::OsStr::new("cairo"))
    {
        // The next process must read the saved renderer, not inherit our default.
        command.env_remove("GSK_RENDERER");
        command.env_remove(crate::CAIRO_SELECTED_BY_STRATA);
    }
    crate::child_environment::restore_command(&mut command);
    Some(command)
}

pub(super) fn restart(application: Option<&gtk::Application>) {
    restart_with(application, start_restart_waiter);
}

fn start_restart_waiter() -> Option<std::process::Child> {
    let current_exe = crate::services::installed_executable().ok()?;
    // Wait for this process to exit completely before relaunching. A fixed
    // delay could overlap the old and new GTK/Wayland clients and rapidly hand
    // keyboard focus through an underlying terminal. Besides re-activating the
    // old GApplication instance, that exposed a Foot/libxkbcommon crash on
    // affected systems. Detach the waiter from inherited terminal streams and
    // put it in its own process group so applying an update cannot disturb the
    // terminal that launched Strata.
    restart_waiter(&current_exe, std::process::id())?
        .spawn()
        .ok()
}

thread_local! {
    static RESTART_IN_FLIGHT: Cell<bool> = const { Cell::new(false) };
}

/// A closed window may no longer have an application; never exit the process in that case.
pub(super) fn restart_with(
    application: Option<&gtk::Application>,
    start: impl FnOnce() -> Option<std::process::Child> + Send + 'static,
) {
    let Some(application) = application.cloned() else {
        return;
    };
    if RESTART_IN_FLIGHT.get() || restart_blocker_shown(&application) {
        return;
    }
    RESTART_IN_FLIGHT.set(true);
    glib::spawn_future_local(async move {
        let started = gio::spawn_blocking(start).await;
        RESTART_IN_FLIGHT.set(false);
        let Ok(Some(mut waiter)) = started else {
            return;
        };
        // Do not leave a waiter behind when an unregistered close handler refuses.
        if !close_windows_for_restart(&application) {
            gio::spawn_blocking(move || {
                let _ = waiter.kill();
                let _ = waiter.wait();
            });
            return;
        }
        application.quit();
    });
}

/// Checks every window before closing any, so a refusal cannot leave some closed.
fn restart_blocker_shown(application: &gtk::Application) -> bool {
    let Some((window, blocker)) = crate::ui::close_guard::application_blocker(application) else {
        return false;
    };
    window.present();
    blocker.show(&window);
    true
}

pub(super) fn close_windows_for_restart(application: &gtk::Application) -> bool {
    if restart_blocker_shown(application) {
        return false;
    }
    for window in application.windows() {
        window.close();
    }
    application.windows().is_empty()
}
