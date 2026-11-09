// SPDX-License-Identifier: MIT

use super::devices::{
    DeviceMatch, FOREIGN_VOLUME_MOUNT_WAIT, ForeignVolumeWaitFollowUp, ForeignVolumeWaitOutcome,
    UnlockViewFollowUp, VolumeSuccessorKind, foreign_volume_wait_follow_up, foreign_wait_successor,
    successor_kind, successor_mount_location, successor_password_drive, successor_volume,
    unlock_view_follow_up, wait_for_foreign_drive_start, wait_for_foreign_volume_mount,
    wait_for_mount_change,
};
use super::dialogs::{MountCredentials, MountPromptDetails};
use super::mount_errors::{
    device_volume_mount_is_ready, mount_error_is_cancelled, mount_result_is_ok,
    volume_error_is_authentication_failure, volume_error_is_in_flight_mount,
};
use super::mounting::{MountOutcome, MountRequest, MountTarget};
use super::unlock_progress::unlock_progress_dismissed_for;
use crate::model::Location;
use crate::ui::browser::{BrowserView, ViewState};
use crate::ui::modal::show_error_dialog;
use futures_channel::oneshot;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::rc::Rc;

/// A volume to mount or a password-protected drive to start.
#[derive(Clone)]
enum UnlockDevice {
    Volume(gio::Volume),
    Drive(gio::Drive),
}

/// How a device mount attempt came about.
#[derive(Clone, Copy)]
struct DeviceAttempt {
    /// A competing mount was already waited out once.
    already_waited: bool,
    user_asked_to_open: bool,
}

/// One mount attempt on a device: what is mounted, who to wait for, and how it started.
#[derive(Clone)]
struct DeviceMount {
    device: UnlockDevice,
    waited: DeviceMatch,
    attempt: DeviceAttempt,
}

enum DeviceReadiness {
    Ready,
    /// A started drive reports success before its volume appears.
    AwaitVolume,
    Failed,
}

impl UnlockDevice {
    fn waited(&self) -> DeviceMatch {
        match self {
            Self::Volume(volume) => DeviceMatch::from_volume(volume),
            Self::Drive(drive) => DeviceMatch::from_drive(drive),
        }
    }

    fn mount_target(&self) -> MountTarget {
        match self {
            Self::Volume(volume) => MountTarget::Volume(volume.clone()),
            Self::Drive(drive) => MountTarget::Drive(drive.clone()),
        }
    }

    fn volume(&self) -> Option<&gio::Volume> {
        match self {
            Self::Volume(volume) => Some(volume),
            Self::Drive(_) => None,
        }
    }

    fn readiness(&self, result: &Result<(), glib::Error>, waited: &DeviceMatch) -> DeviceReadiness {
        match self {
            Self::Volume(volume) => {
                if device_volume_mount_is_ready(result, volume.get_mount().is_some()) {
                    DeviceReadiness::Ready
                } else {
                    DeviceReadiness::Failed
                }
            }
            Self::Drive(_) => {
                if successor_kind(waited) == VolumeSuccessorKind::Mounted {
                    DeviceReadiness::Ready
                } else if mount_result_is_ok(result) {
                    DeviceReadiness::AwaitVolume
                } else {
                    DeviceReadiness::Failed
                }
            }
        }
    }

    fn log_in_flight(&self, already_waited: bool) {
        match self {
            // A competing automounter may own this job; do not cancel it.
            Self::Volume(volume) => tracing::debug!(
                volume = %volume.name(),
                already_waited,
                "waiting for in-flight volume mount"
            ),
            Self::Drive(drive) => tracing::debug!(
                drive = %drive.name(),
                already_waited,
                "waiting for in-flight drive start"
            ),
        }
    }

    async fn wait_for_foreign_mount(&self, waited: &DeviceMatch) -> ForeignVolumeWaitOutcome {
        match self {
            Self::Volume(volume) => wait_for_foreign_volume_mount(volume).await,
            Self::Drive(drive) => wait_for_foreign_drive_start(drive, waited).await,
        }
    }
}

impl BrowserView {
    pub(crate) fn mount_volume(&self, volume: gio::Volume) {
        self.state.mount_device_volume(volume, false, true);
    }

    pub(crate) fn unlock_volume(&self, volume: gio::Volume) {
        self.state.mount_device_volume(volume, false, true);
    }

    pub(crate) fn start_password_drive(&self, drive: gio::Drive, user_asked_to_open: bool) {
        self.state.start_unlock_device(
            UnlockDevice::Drive(drive),
            None,
            DeviceAttempt {
                already_waited: false,
                user_asked_to_open,
            },
        );
    }
}

impl ViewState {
    fn apply_unlock_view_follow_up(
        &self,
        waited: &DeviceMatch,
        mount_location: &Location,
        user_asked_to_open: bool,
    ) {
        match unlock_view_follow_up(
            self.browser.active_location().as_ref(),
            mount_location,
            user_asked_to_open,
            unlock_progress_dismissed_for(&self.unlock_slots.borrow(), &waited.keys),
        ) {
            UnlockViewFollowUp::Reload => self.browser.reload_active(),
            UnlockViewFollowUp::Navigate => self.browser.navigate(mount_location.clone()),
            UnlockViewFollowUp::None => {}
        }
    }

    fn open_unlocked_identity(
        &self,
        volume: Option<&gio::Volume>,
        waited: &DeviceMatch,
        user_asked_to_open: bool,
    ) {
        let location = volume
            .and_then(|volume| volume.get_mount())
            .and_then(|mount| crate::adapters::location_for_file(&mount.root()))
            .or_else(|| successor_mount_location(waited));
        if let Some(location) = location {
            crate::adapters::invalidate_mount_snapshot();
            self.apply_unlock_view_follow_up(waited, &location, user_asked_to_open);
        }
        self.finish_unlock_slot(&waited.keys);
    }

    fn start_owned_successor(self: &Rc<Self>, waited: &DeviceMatch, user_asked_to_open: bool) {
        let user_asked_to_open = user_asked_to_open
            && !unlock_progress_dismissed_for(&self.unlock_slots.borrow(), &waited.keys);
        self.finish_unlock_slot(&waited.keys);
        if waited.is_absent() {
            return;
        }
        let device = successor_volume(waited)
            .map(UnlockDevice::Volume)
            .or_else(|| successor_password_drive(waited).map(UnlockDevice::Drive));
        if let Some(device) = device {
            self.start_unlock_device(
                device,
                None,
                DeviceAttempt {
                    already_waited: false,
                    user_asked_to_open,
                },
            );
        }
    }

    fn mount_device_volume(
        self: &Rc<Self>,
        volume: gio::Volume,
        already_waited: bool,
        user_asked_to_open: bool,
    ) {
        self.start_unlock_device(
            UnlockDevice::Volume(volume),
            None,
            DeviceAttempt {
                already_waited,
                user_asked_to_open,
            },
        );
    }

    fn start_unlock_device(
        self: &Rc<Self>,
        device: UnlockDevice,
        credentials: Option<MountCredentials>,
        attempt: DeviceAttempt,
    ) {
        let waited = device.waited();
        let first_attempt = credentials.is_none() && !attempt.already_waited;
        if first_attempt && !self.begin_unlock_progress(&waited.keys) {
            return;
        }
        let target = device.mount_target();
        let mount = DeviceMount {
            device,
            waited,
            attempt,
        };
        self.mount_target(
            MountRequest {
                target,
                credentials,
                cancellable: None,
            },
            move |state, outcome| state.finish_device_mount(&mount, outcome),
        );
    }

    fn finish_device_mount(self: &Rc<Self>, mount: &DeviceMount, outcome: MountOutcome) {
        let DeviceMount {
            device,
            waited,
            attempt,
        } = mount;
        let in_flight = outcome
            .result
            .as_ref()
            .err()
            .is_some_and(volume_error_is_in_flight_mount);
        if !in_flight {
            self.dismiss_unlock_progress(&waited.keys);
        }
        match device.readiness(&outcome.result, waited) {
            DeviceReadiness::Ready => {
                self.open_unlocked_identity(device.volume(), waited, attempt.user_asked_to_open);
            }
            DeviceReadiness::AwaitVolume => {
                self.await_started_drive_volume(waited.clone(), attempt.user_asked_to_open);
            }
            DeviceReadiness::Failed => {
                let MountOutcome {
                    result,
                    attempted,
                    details,
                } = outcome;
                if let Err(error) = result {
                    self.handle_device_mount_error(mount, error, (attempted, details));
                }
            }
        }
    }

    fn handle_device_mount_error(
        self: &Rc<Self>,
        mount: &DeviceMount,
        error: glib::Error,
        (attempted, details): (Option<MountCredentials>, Option<MountPromptDetails>),
    ) {
        if volume_error_is_authentication_failure(&error)
            && let Some(details) = details
        {
            self.prompt_device_retry(mount, (attempted, details));
        } else if volume_error_is_in_flight_mount(&error) {
            mount.device.log_in_flight(mount.attempt.already_waited);
            self.await_foreign_mount(mount.clone());
        } else {
            self.finish_unlock_slot(&mount.waited.keys);
            if !mount_error_is_cancelled(&error) {
                show_error_dialog(
                    &self.overlay,
                    &crate::i18n::tr("Unable to mount volume"),
                    &crate::services::gio_device_error_message(&error),
                );
            }
        }
    }

    fn prompt_device_retry(
        self: &Rc<Self>,
        mount: &DeviceMount,
        (attempted, details): (Option<MountCredentials>, MountPromptDetails),
    ) {
        let weak = Rc::downgrade(self);
        let retry_device = mount.device.clone();
        let retry_attempt = DeviceAttempt {
            already_waited: false,
            ..mount.attempt
        };
        self.show_unlock_retry_prompt(
            mount.waited.keys.clone(),
            attempted,
            details,
            move |credentials| {
                if let Some(state) = weak.upgrade() {
                    state.start_unlock_device(
                        retry_device.clone(),
                        Some(credentials),
                        retry_attempt,
                    );
                }
            },
        );
    }

    fn await_foreign_mount(self: &Rc<Self>, mount: DeviceMount) {
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let _activity = BrowserView {
                state: state.clone(),
            }
            .begin_global_activity(crate::i18n::tr("Connecting…"));
            let outcome = mount.device.wait_for_foreign_mount(&mount.waited).await;
            drop(_activity);
            state.follow_foreign_mount(&mount, outcome);
        });
    }

    fn follow_foreign_mount(
        self: &Rc<Self>,
        mount: &DeviceMount,
        outcome: ForeignVolumeWaitOutcome,
    ) {
        let DeviceMount {
            device,
            waited,
            attempt,
        } = mount;
        let successor = foreign_wait_successor(outcome, waited);
        let user_asked_to_open = attempt.user_asked_to_open;
        match foreign_volume_wait_follow_up(outcome, attempt.already_waited, successor) {
            ForeignVolumeWaitFollowUp::Navigate => {
                self.open_unlocked_identity(device.volume(), waited, user_asked_to_open);
            }
            ForeignVolumeWaitFollowUp::StartOwnedMount
                if outcome == ForeignVolumeWaitOutcome::Gone =>
            {
                self.start_owned_successor(waited, user_asked_to_open);
            }
            ForeignVolumeWaitFollowUp::StartOwnedMount => {
                self.start_unlock_device(
                    device.clone(),
                    None,
                    DeviceAttempt {
                        already_waited: true,
                        user_asked_to_open,
                    },
                );
            }
            ForeignVolumeWaitFollowUp::Quiet => {
                self.finish_unlock_slot(&waited.keys);
            }
        }
    }

    fn await_started_drive_volume(self: &Rc<Self>, waited: DeviceMatch, user_asked_to_open: bool) {
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let (_send, receive) = oneshot::channel();
            wait_for_mount_change(
                receive,
                || {
                    successor_volume(&waited).is_some()
                        || successor_mount_location(&waited).is_some()
                },
                FOREIGN_VOLUME_MOUNT_WAIT,
            )
            .await;
            let Some(state) = weak.upgrade() else {
                return;
            };
            if successor_mount_location(&waited).is_some() {
                state.open_unlocked_identity(None, &waited, user_asked_to_open);
            } else if successor_volume(&waited).is_some() {
                state.start_owned_successor(&waited, user_asked_to_open);
            } else {
                state.finish_unlock_slot(&waited.keys);
                show_error_dialog(
                    &state.overlay,
                    &crate::i18n::tr("Unable to mount volume"),
                    &crate::i18n::tr("The device started, but no mountable volume appeared."),
                );
            }
        });
    }
}
