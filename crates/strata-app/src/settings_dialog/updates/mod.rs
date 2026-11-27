// SPDX-License-Identifier: MIT

//! Live update checks, channel/auto-update wiring, and in-place install flow.

mod install;
mod messages;
#[cfg(test)]
mod tests;
mod view;

use std::{
    sync::{
        OnceLock,
        mpsc::{Receiver, TryRecvError},
    },
    time::Duration,
};

use rune::Tx;
use strata_core::i18n::tr;
use strata_desktop::{
    build_info::{build_kind, installed_version},
    services::{
        BuildKind, Channel, InstallCancel, InstallRequest, InstallSource, ManagedInstall,
        ReleaseMetadata, UpdateCheck, UpdateInstall, UpdateMethod, check_for_updates,
        update_method,
    },
};
use strata_fs::services::UNEXPECTED_SYSTEM_ERROR;

pub(in crate::settings_dialog) use view::build;

use crate::{App, settings_dialog::close_settings, shell};
use install::{
    InstallLock, InstallProgressState, aur_update_action_label, launch_managed_update,
    restart_after_install, spawn_install,
};
use messages::{effective_update_channel, offer_still_eligible};

const POLL_INTERVAL: Duration = Duration::from_millis(50);

static UPDATE_METHOD_CACHE: OnceLock<UpdateMethod> = OnceLock::new();

#[cfg(test)]
type CheckHook = std::rc::Rc<
    dyn Fn(Channel, strata_desktop::services::Version, UpdateMethod, bool) -> Receiver<UpdateCheck>,
>;
#[cfg(test)]
type InstallHook = std::rc::Rc<dyn Fn(InstallRequest, InstallCancel) -> Receiver<UpdateInstall>>;
#[cfg(test)]
type RestartHook = std::rc::Rc<dyn Fn()>;
#[cfg(test)]
type OpenUriHook = std::rc::Rc<dyn Fn(&str)>;
#[cfg(test)]
type ManagedLaunchHook = std::rc::Rc<dyn Fn(UpdateMethod) -> Result<String, String>>;

enum UpdatePhase {
    Idle,
    Checking {
        receiver: Receiver<UpdateCheck>,
        channel: Channel,
        method: UpdateMethod,
    },
    UpToDate,
    Available {
        release: Box<ReleaseMetadata>,
        install: InstallRequest,
        method: UpdateMethod,
        returns_to_stable: bool,
        managed_status: Option<String>,
        install_busy: bool,
    },
    Installing {
        receiver: Receiver<UpdateInstall>,
        cancel: InstallCancel,
        returns_to_stable: bool,
        progress: InstallProgressState,
        cancelling: bool,
        finalizing: bool,
        _lock: InstallLock,
    },
    Installed {
        returns_to_stable: bool,
    },
    InstallCancelled,
    InstallFailed {
        message: Option<String>,
    },
    CheckFailed {
        message: String,
    },
}

pub(super) struct UpdatesState {
    phase: UpdatePhase,
    generation: u64,
    method: Option<UpdateMethod>,
    detecting_method: bool,
    #[cfg(test)]
    pub(super) check_hook: Option<CheckHook>,
    #[cfg(test)]
    pub(super) install_hook: Option<InstallHook>,
    #[cfg(test)]
    pub(super) restart_hook: Option<RestartHook>,
    #[cfg(test)]
    pub(super) open_uri_hook: Option<OpenUriHook>,
    #[cfg(test)]
    pub(super) managed_launch_hook: Option<ManagedLaunchHook>,
    #[cfg(test)]
    pub(super) managed_override: Option<Option<ManagedInstall>>,
}

impl UpdatesState {
    pub(super) fn new() -> Self {
        Self {
            phase: UpdatePhase::Idle,
            generation: 0,
            method: UPDATE_METHOD_CACHE.get().copied(),
            detecting_method: false,
            #[cfg(test)]
            check_hook: None,
            #[cfg(test)]
            install_hook: None,
            #[cfg(test)]
            restart_hook: None,
            #[cfg(test)]
            open_uri_hook: None,
            #[cfg(test)]
            managed_launch_hook: None,
            #[cfg(test)]
            managed_override: None,
        }
    }

    pub(super) fn effective_method(&self) -> UpdateMethod {
        self.method.unwrap_or(UpdateMethod::InPlace)
    }

    pub(super) fn managed(&self) -> Option<&ManagedInstall> {
        #[cfg(test)]
        if let Some(override_val) = &self.managed_override {
            return override_val.as_ref();
        }
        InstallSource::detect().managed()
    }

    pub(super) fn is_installing(&self) -> bool {
        matches!(self.phase, UpdatePhase::Installing { .. })
    }

    fn is_install_underway(&self) -> bool {
        matches!(
            self.phase,
            UpdatePhase::Installing { .. } | UpdatePhase::Installed { .. }
        )
    }

    fn is_polling(&self) -> bool {
        matches!(
            self.phase,
            UpdatePhase::Checking { .. } | UpdatePhase::Installing { .. }
        )
    }

    pub(super) fn closed(&mut self) {
        if matches!(self.phase, UpdatePhase::Checking { .. }) {
            self.generation = self.generation.saturating_add(1);
            self.phase = UpdatePhase::Idle;
        }
    }
}

pub(super) fn ensure_method_detected(app: &mut App) {
    let state = &mut app.settings_dialog.updates;
    if state.method.is_some() || state.detecting_method {
        return;
    }
    if let Some(cached) = UPDATE_METHOD_CACHE.get().copied() {
        state.method = Some(cached);
        return;
    }
    state.detecting_method = true;
    let weak = app.settings_dialog.later.downgrade();
    glib::MainContext::default().spawn_local(async move {
        let detected = gio::spawn_blocking(detect_update_method).await;
        let method = detected.unwrap_or(UpdateMethod::InPlace);
        let Some(later) = weak.upgrade() else {
            return;
        };
        later.then(move |app, _| {
            let state = &mut app.settings_dialog.updates;
            state.detecting_method = false;
            if state.method.is_none() {
                state.method = Some(method);
            }
            true
        });
    });
}

fn detect_update_method() -> UpdateMethod {
    *UPDATE_METHOD_CACHE.get_or_init(|| {
        if let Some(managed) = InstallSource::detect().managed() {
            managed.find_installed_aur_helper();
        }
        update_method()
    })
}

pub(super) fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let state = &mut app.settings_dialog.updates;
    let changed = match &mut state.phase {
        UpdatePhase::Checking { .. } => poll_check(state),
        UpdatePhase::Installing { .. } => poll_install(state),
        _ => false,
    };
    if state.is_polling() {
        tx.wake_after(POLL_INTERVAL);
    }
    changed
}

fn poll_check(state: &mut UpdatesState) -> bool {
    let UpdatePhase::Checking {
        receiver,
        channel,
        method,
    } = &state.phase
    else {
        return false;
    };
    let (channel, method) = (*channel, *method);
    let result = match receiver.try_recv() {
        Ok(result) => result,
        Err(TryRecvError::Empty) => return false,
        Err(TryRecvError::Disconnected) => UpdateCheck::Failed(tr(UNEXPECTED_SYSTEM_ERROR)),
    };
    state.phase = match result {
        UpdateCheck::UpToDate => UpdatePhase::UpToDate,
        UpdateCheck::Available { release, install } => {
            let returns_to_stable = channel == Channel::Stable
                && build_kind() != BuildKind::Stable
                && release.kind == BuildKind::Stable;
            UpdatePhase::Available {
                release: Box::new(release),
                install,
                method,
                returns_to_stable,
                managed_status: None,
                install_busy: false,
            }
        }
        UpdateCheck::Failed(message) => UpdatePhase::CheckFailed { message },
    };
    true
}

fn poll_install(state: &mut UpdatesState) -> bool {
    let mut changed = false;
    loop {
        let next = {
            let UpdatePhase::Installing { receiver, .. } = &state.phase else {
                return changed;
            };
            receiver.try_recv()
        };
        match next {
            Ok(event) => {
                changed = true;
                if apply_install_event(state, event) {
                    return true;
                }
            }
            Err(TryRecvError::Empty) => return changed,
            Err(TryRecvError::Disconnected) => {
                state.phase = UpdatePhase::InstallFailed { message: None };
                return true;
            }
        }
    }
}

/// Returns `true` if `event` transitioned `state` into a terminal phase.
fn apply_install_event(state: &mut UpdatesState, event: UpdateInstall) -> bool {
    let UpdatePhase::Installing {
        returns_to_stable,
        progress,
        finalizing,
        ..
    } = &mut state.phase
    else {
        return true;
    };
    let returns_to_stable = *returns_to_stable;
    match event {
        UpdateInstall::Downloading { downloaded, total } => {
            progress.apply_downloading(downloaded, total);
            false
        }
        UpdateInstall::Verifying => {
            progress.status = tr("Verifying update…");
            false
        }
        UpdateInstall::Installing => {
            progress.fraction = Some(1.0);
            progress.status = tr("Installing update…");
            false
        }
        UpdateInstall::Finalizing => {
            progress.fraction = Some(1.0);
            progress.status = tr(install::FINALIZING_STATUS);
            *finalizing = true;
            false
        }
        UpdateInstall::Installed => {
            state.phase = UpdatePhase::Installed { returns_to_stable };
            true
        }
        UpdateInstall::Cancelled => {
            state.phase = UpdatePhase::InstallCancelled;
            true
        }
        UpdateInstall::Failed(message) => {
            state.phase = UpdatePhase::InstallFailed {
                message: Some(message),
            };
            true
        }
    }
}

pub(super) fn start_check(app: &mut App, force: bool, tx: &mut Tx) {
    let selected = shell::preferences(app).release_channel();
    let method = app.settings_dialog.updates.effective_method();
    let channel = effective_update_channel(selected, method);
    let receiver = spawn_check(&app.settings_dialog.updates, channel, method, force);
    let state = &mut app.settings_dialog.updates;
    state.generation = state.generation.saturating_add(1);
    state.phase = UpdatePhase::Checking {
        receiver,
        channel,
        method,
    };
    tx.wake_after(POLL_INTERVAL);
}

fn spawn_check(
    state: &UpdatesState,
    channel: Channel,
    method: UpdateMethod,
    force: bool,
) -> Receiver<UpdateCheck> {
    let version = installed_version();
    #[cfg(test)]
    if let Some(hook) = &state.check_hook {
        return hook(channel, version, method, force);
    }
    let _ = state;
    check_for_updates(channel, version, method, force)
}

pub(super) fn on_action_press(app: &mut App, tx: &mut Tx) {
    match &mut app.settings_dialog.updates.phase {
        UpdatePhase::Checking { .. } => {}
        UpdatePhase::Installed { .. } => restart_after_install(app, tx),
        UpdatePhase::Installing {
            cancel,
            cancelling,
            finalizing,
            ..
        } => {
            if *cancelling || *finalizing {
                return;
            }
            if cancel.cancel() {
                *cancelling = true;
            } else {
                *finalizing = true;
            }
        }
        UpdatePhase::Available { .. } => handle_available_action(app, tx),
        UpdatePhase::Idle
        | UpdatePhase::UpToDate
        | UpdatePhase::InstallCancelled
        | UpdatePhase::InstallFailed { .. }
        | UpdatePhase::CheckFailed { .. } => start_check(app, true, tx),
    }
}

fn handle_available_action(app: &mut App, tx: &mut Tx) {
    let (method, release_kind, returns_to_stable, request) = {
        let UpdatePhase::Available {
            release,
            install,
            method,
            returns_to_stable,
            ..
        } = &app.settings_dialog.updates.phase
        else {
            return;
        };
        (*method, release.kind, *returns_to_stable, install.clone())
    };

    match method {
        UpdateMethod::Pacman => start_check(app, true, tx),
        UpdateMethod::Aur => {
            if aur_update_action_label(app.settings_dialog.updates.managed()) == "Close" {
                close_settings(app);
            } else {
                launch_managed_update(app, UpdateMethod::Aur);
            }
        }
        UpdateMethod::Omarchy => launch_managed_update(app, UpdateMethod::Omarchy),
        UpdateMethod::InPlace => {
            let channel = shell::preferences(app).release_channel();
            if !offer_still_eligible(channel, release_kind) {
                start_check(app, true, tx);
                return;
            }
            start_install(app, request, returns_to_stable, tx);
        }
    }
}

fn start_install(app: &mut App, request: InstallRequest, returns_to_stable: bool, tx: &mut Tx) {
    let Some(lock) = InstallLock::try_acquire() else {
        if let UpdatePhase::Available { install_busy, .. } = &mut app.settings_dialog.updates.phase
        {
            *install_busy = true;
        }
        return;
    };
    let cancel = InstallCancel::new();
    let receiver = spawn_install(&app.settings_dialog.updates, request, cancel.clone());
    let state = &mut app.settings_dialog.updates;
    state.generation = state.generation.saturating_add(1);
    state.phase = UpdatePhase::Installing {
        receiver,
        cancel,
        returns_to_stable,
        progress: InstallProgressState::starting(returns_to_stable),
        cancelling: false,
        finalizing: false,
        _lock: lock,
    };
    tx.wake_after(POLL_INTERVAL);
}

pub(super) fn on_auto_updates_toggled(app: &mut App, enabled: bool, tx: &mut Tx) {
    if app.settings_dialog.updates.is_install_underway() {
        return;
    }
    if enabled {
        start_check(app, false, tx);
    } else {
        let state = &mut app.settings_dialog.updates;
        state.generation = state.generation.saturating_add(1);
        state.phase = UpdatePhase::Idle;
    }
}

pub(super) fn on_channel_changed(app: &mut App, tx: &mut Tx) {
    if app.settings_dialog.updates.is_install_underway() {
        return;
    }
    start_check(app, false, tx);
}

pub(super) fn open_uri(app: &App, uri: &str) -> Result<(), String> {
    #[cfg(test)]
    if let Some(hook) = &app.settings_dialog.updates.open_uri_hook {
        hook(uri);
        return Ok(());
    }
    let _ = app;
    gio::AppInfo::launch_default_for_uri(uri, gio::AppLaunchContext::NONE)
        .map_err(|error| error.to_string())
}
