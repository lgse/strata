// SPDX-License-Identifier: MIT

//! Running one in-place install at a time and driving its progress on the GTK
//! main loop.

use std::{cell::Cell, rc::Rc, sync::mpsc::TryRecvError, time::Duration};

use gtk::glib;

use crate::services::{self, InstallCancel, InstallRequest, UpdateInstall};

/// Shared "an install is running" guard across the update row and update
/// dialog, the two places [`services::install_update`] is called. Without one
/// process-wide guard, separate windows could replace the executable at the
/// same time. See [`start_install`].
pub(crate) type InstallGuard = Rc<Cell<bool>>;

thread_local! {
    static INSTALL_GUARD: InstallGuard = Rc::new(Cell::new(false));
}

/// The one [`InstallGuard`] for this process.
///
/// Every install writes the *same* target -- the running executable -- so
/// the guard has to span every window, not just the controls within one. A
/// per-window guard would let installs started in separate windows replace
/// that executable concurrently, leaving the last writer as the installed
/// build.
///
/// A `thread_local` `Rc` (rather than a `Mutex`) is the whole story here
/// because every window is built on the single GTK main thread, from
/// `connect_activate`; this mirrors [`PreferenceManager::shared`](crate::ui::preferences::PreferenceManager::shared). It is
/// deliberately never released: it is one `bool`, and the guard's
/// correctness should not depend on some window or in-flight install
/// happening to still hold a strong reference.
pub(crate) fn install_guard() -> InstallGuard {
    INSTALL_GUARD.with(|guard| guard.clone())
}

pub(super) type InstallLauncher =
    Rc<dyn Fn(InstallRequest, InstallCancel) -> std::sync::mpsc::Receiver<UpdateInstall>>;

pub(super) fn default_install_launcher() -> InstallLauncher {
    Rc::new(services::install_update)
}

/// The three non-terminal states [`drive_install`] reports through
/// [`InstallCallbacks::on_progress`]. Keeping this separate from
/// [`UpdateInstall`] means callers never need to (incorrectly) handle
/// `Installed`/`Failed` in that closure -- those terminal states are always
/// reported through the other callbacks instead.
pub(super) enum InstallProgress {
    Downloading { downloaded: u64, total: Option<u64> },
    Verifying,
    Installing,
    Finalizing,
}

pub(super) const FINALIZING_STATUS: &str = "Finalizing update…";

/// How one install flow renders the driver's events. `on_failed` receives
/// `Some(message)` for an explicit [`UpdateInstall::Failed`] or `None` when
/// the receiver disconnected without ever reporting one, since callers render
/// those two cases differently.
pub(super) struct InstallCallbacks {
    pub(super) on_progress: Box<dyn Fn(InstallProgress)>,
    pub(super) on_installed: Box<dyn Fn()>,
    pub(super) on_cancelled: Box<dyn Fn()>,
    pub(super) on_failed: Box<dyn Fn(Option<String>)>,
}

/// Drives an install `receiver` on the GTK main loop until it reports a
/// terminal outcome, then stops.
///
/// This is the shared shape behind `update_check_row`'s and
/// `show_update_dialog`'s install flows: poll `receiver` every 100ms, forward
/// non-terminal updates to `on_progress`, and invoke exactly one of
/// `on_installed`/`on_cancelled`/`on_failed` once a terminal state is reached.
/// Deliberately does *not* format status text itself because the two call
/// sites use different wording.
fn drive_install(receiver: std::sync::mpsc::Receiver<UpdateInstall>, callbacks: InstallCallbacks) {
    glib::timeout_add_local(Duration::from_millis(100), move || {
        loop {
            match receiver.try_recv() {
                Ok(UpdateInstall::Downloading { downloaded, total }) => {
                    (callbacks.on_progress)(InstallProgress::Downloading { downloaded, total });
                }
                Ok(UpdateInstall::Verifying) => (callbacks.on_progress)(InstallProgress::Verifying),
                Ok(UpdateInstall::Installing) => {
                    (callbacks.on_progress)(InstallProgress::Installing);
                }
                Ok(UpdateInstall::Finalizing) => {
                    (callbacks.on_progress)(InstallProgress::Finalizing);
                }
                Ok(UpdateInstall::Installed) => {
                    (callbacks.on_installed)();
                    return glib::ControlFlow::Break;
                }
                Ok(UpdateInstall::Cancelled) => {
                    (callbacks.on_cancelled)();
                    return glib::ControlFlow::Break;
                }
                Ok(UpdateInstall::Failed(message)) => {
                    (callbacks.on_failed)(Some(message));
                    return glib::ControlFlow::Break;
                }
                Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(TryRecvError::Disconnected) => {
                    (callbacks.on_failed)(None);
                    return glib::ControlFlow::Break;
                }
            }
        }
    });
}

/// The shared guard prevents concurrent replacements from different windows.
/// Guard rejection leaves the request unused and must keep the caller retryable.
pub(super) fn start_install(
    guard: &InstallGuard,
    request: InstallRequest,
    launcher: &InstallLauncher,
    callbacks: InstallCallbacks,
) -> Result<InstallCancel, InstallRequest> {
    if guard.replace(true) {
        return Err(request);
    }
    let cancel = InstallCancel::new();
    let receiver = launcher(request, cancel.clone());
    drive_install(receiver, releasing_guard(guard, callbacks));
    Ok(cancel)
}

/// Wraps the terminal callbacks so each releases `guard` before reporting.
fn releasing_guard(guard: &InstallGuard, callbacks: InstallCallbacks) -> InstallCallbacks {
    let InstallCallbacks {
        on_progress,
        on_installed,
        on_cancelled,
        on_failed,
    } = callbacks;
    let guard_for_installed = guard.clone();
    let guard_for_cancelled = guard.clone();
    let guard_for_failed = guard.clone();
    InstallCallbacks {
        on_progress,
        on_installed: Box::new(move || {
            guard_for_installed.set(false);
            on_installed();
        }),
        on_cancelled: Box::new(move || {
            guard_for_cancelled.set(false);
            on_cancelled();
        }),
        on_failed: Box::new(move |message| {
            guard_for_failed.set(false);
            on_failed(message);
        }),
    }
}

/// The update row's compact progress rendering, distinct from the update
/// dialog's dialog-specific wording.
pub(super) fn apply_install_progress(
    status: &gtk::Label,
    progress: &gtk::ProgressBar,
    event: InstallProgress,
) {
    match event {
        InstallProgress::Downloading { downloaded, total } => {
            apply_download_progress(status, progress, downloaded, total);
        }
        InstallProgress::Verifying => status.set_text(&crate::i18n::tr("Verifying update…")),
        InstallProgress::Installing => {
            progress.set_fraction(1.0);
            status.set_text(&crate::i18n::tr("Installing update…"));
        }
        InstallProgress::Finalizing => {
            progress.set_fraction(1.0);
            status.set_text(&crate::i18n::tr(FINALIZING_STATUS));
        }
    }
}

fn apply_download_progress(
    status: &gtk::Label,
    progress: &gtk::ProgressBar,
    downloaded: u64,
    total: Option<u64>,
) {
    if let Some(total) = total.filter(|total| *total > 0) {
        let fraction = (downloaded as f64 / total as f64).clamp(0.0, 1.0);
        progress.set_fraction(fraction);
        status.set_text(&rust_i18n::t!(
            "Downloading update… %{value1}%",
            value1 = format!("{:.0}", fraction * 100.0)
        ));
    } else {
        progress.pulse();
        status.set_text(&rust_i18n::t!(
            "Downloading update… %{value1} MB",
            value1 = crate::i18n::decimal(downloaded as f64 / 1_048_576.0, 1)
        ));
    }
}
