// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use gio::prelude::*;
use rune::Tx;
use strata_components::sidebar::ItemId;
use strata_core::{i18n::tr, model::Location};
use strata_fs::{
    adapters::{invalidate_mount_snapshot, location_for_file},
    services::gio_device_error_message,
};

use super::{
    crypto::{crypto_password_uuid_for_volume, gio_volume_is_encrypted},
    listing::unix_device,
    release::refresh_all_sidebars,
};
use crate::{
    App,
    error_dialog::{self, Error},
    mount_prompts::{
        self,
        operation::{
            BoundOperation, MountCredentials, MountOutcome, MountPromptDetails, OperationOptions,
            RetryDefaults, show_retry_prompt,
        },
    },
    shell,
    slots::WeakLater,
};

const UNLOCK_PROGRESS_DELAY: Duration = Duration::from_millis(350);
const FOREIGN_VOLUME_MOUNT_WAIT: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ForeignVolumeWaitOutcome {
    Mounted,
    StillLocked,
    Gone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VolumeSuccessorKind {
    Mounted,
    Locked,
    Absent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ForeignVolumeWaitFollowUp {
    Navigate,
    StartOwnedMount,
    Quiet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UnlockViewFollowUp {
    Reload,
    Navigate,
    None,
}

/// Identifies a volume or password drive across GIO object replacement during unlock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DeviceKeys {
    volume_tokens: Vec<String>,
    drive_tokens: Vec<String>,
    volume_object: Option<glib::Object>,
    drive_object: Option<glib::Object>,
}

impl DeviceKeys {
    pub(super) fn new(
        volume: impl IntoIterator<Item = Option<String>>,
        drive: impl IntoIterator<Item = Option<String>>,
    ) -> Self {
        Self {
            volume_tokens: collect_tokens(volume),
            drive_tokens: collect_tokens(drive),
            volume_object: None,
            drive_object: None,
        }
    }

    fn has_volume_identity(&self) -> bool {
        !self.volume_tokens.is_empty() || self.volume_object.is_some()
    }

    fn is_empty(&self) -> bool {
        !self.has_volume_identity() && self.drive_tokens.is_empty() && self.drive_object.is_none()
    }
}

#[derive(Clone)]
struct DeviceMatch {
    keys: DeviceKeys,
}

impl DeviceMatch {
    fn from_volume(volume: &gio::Volume) -> Self {
        let drive = volume.drive();
        Self {
            keys: device_keys(Some(volume), drive.as_ref()),
        }
    }

    fn from_drive(drive: &gio::Drive) -> Self {
        Self {
            keys: device_keys(None, Some(drive)),
        }
    }

    fn is_absent(&self) -> bool {
        self.keys.is_empty()
    }

    fn matches_mount(&self, mount: &gio::Mount) -> bool {
        identity_matches_volume(
            &self.keys,
            &device_keys(mount.volume().as_ref(), mount.drive().as_ref()),
        )
    }

    fn matches_volume(&self, volume: &gio::Volume) -> bool {
        identity_matches_volume(
            &self.keys,
            &device_keys(Some(volume), volume.drive().as_ref()),
        )
    }

    fn matches_password_drive(&self, drive: &gio::Drive) -> bool {
        drive.start_stop_type() == gio::DriveStartStopType::Password
            && same_drive(&self.keys, &device_keys(None, Some(drive)))
    }
}

#[derive(Clone)]
enum UnlockDevice {
    Volume(gio::Volume),
    Drive(gio::Drive),
}

#[derive(Clone, Copy)]
struct DeviceAttempt {
    already_waited: bool,
    user_asked_to_open: bool,
}

struct DeviceMount {
    row_id: String,
    device: UnlockDevice,
    waited: DeviceMatch,
    attempt: DeviceAttempt,
    guard: UnlockSlotGuard,
}

pub(super) struct UnlockSlotGuard {
    keys: Option<DeviceKeys>,
    window_later: WeakLater<App>,
}

impl UnlockSlotGuard {
    pub(super) fn acquire(window_later: WeakLater<App>, keys: DeviceKeys) -> Option<Self> {
        begin_unlock_slot(&keys).then_some(Self {
            keys: Some(keys),
            window_later,
        })
    }

    fn finish(mut self) {
        if let Some(keys) = self.keys.take() {
            finish_unlock_slot(&self.window_later, &keys);
        }
    }
}

impl Drop for UnlockSlotGuard {
    fn drop(&mut self) {
        if let Some(keys) = self.keys.take() {
            finish_unlock_slot(&self.window_later, &keys);
        }
    }
}

enum DeviceReadiness {
    Ready,
    AwaitVolume,
    Failed,
}

struct UnlockSlot {
    keys: DeviceKeys,
    overlay_id: u64,
    pending_timer: Option<glib::SourceId>,
    dismissed: bool,
    in_flight: bool,
}

thread_local! {
    static UNLOCK_SLOTS: RefCell<Vec<UnlockSlot>> = const { RefCell::new(Vec::new()) };
}

impl UnlockDevice {
    fn waited(&self) -> DeviceMatch {
        match self {
            Self::Volume(volume) => DeviceMatch::from_volume(volume),
            Self::Drive(drive) => DeviceMatch::from_drive(drive),
        }
    }

    fn name(&self) -> String {
        match self {
            Self::Volume(volume) => volume.name().to_string(),
            Self::Drive(drive) => drive.name().to_string(),
        }
    }

    fn is_encrypted(&self) -> bool {
        match self {
            Self::Volume(volume) => gio_volume_is_encrypted(volume),
            Self::Drive(_) => true,
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

    async fn run(&self, operation: &gio::MountOperation) -> Result<(), glib::Error> {
        match self {
            Self::Volume(volume) => {
                volume
                    .mount_future(gio::MountMountFlags::NONE, Some(operation))
                    .await
            }
            Self::Drive(drive) => {
                drive
                    .start_future(gio::DriveStartFlags::NONE, Some(operation))
                    .await
            }
        }
    }

    async fn wait_for_foreign_mount(&self, waited: &DeviceMatch) -> ForeignVolumeWaitOutcome {
        match self {
            Self::Volume(volume) => wait_for_foreign_volume_mount(volume).await,
            Self::Drive(drive) => wait_for_foreign_drive_start(drive, waited).await,
        }
    }
}

struct UnlockRequest {
    row_id: String,
    device: UnlockDevice,
    credentials: Option<MountCredentials>,
    attempt: DeviceAttempt,
    guard: Option<UnlockSlotGuard>,
}

/// Mounts or unlocks `volume` and opens its root once ready.
pub(super) fn mount_volume(app: &mut App, row_id: String, volume: gio::Volume) {
    app.sidebar.current = Some(ItemId::Device(row_id.clone()));
    let window_later = shell::window_later(app).downgrade();
    start_unlock_device(
        window_later,
        UnlockRequest {
            row_id,
            device: UnlockDevice::Volume(volume),
            credentials: None,
            attempt: DeviceAttempt {
                already_waited: false,
                user_asked_to_open: true,
            },
            guard: None,
        },
    );
}

/// Starts a password-protected drive and opens its mounted volume once ready.
pub(super) fn start_password_drive(app: &mut App, row_id: String, drive: gio::Drive) {
    app.sidebar.current = Some(ItemId::Device(row_id.clone()));
    let window_later = shell::window_later(app).downgrade();
    start_unlock_device(
        window_later,
        UnlockRequest {
            row_id,
            device: UnlockDevice::Drive(drive),
            credentials: None,
            attempt: DeviceAttempt {
                already_waited: false,
                user_asked_to_open: true,
            },
            guard: None,
        },
    );
}

fn start_unlock_device(window_later: WeakLater<App>, request: UnlockRequest) {
    let UnlockRequest {
        row_id,
        device,
        credentials,
        attempt,
        guard,
    } = request;
    let waited = device.waited();
    let guard = match guard {
        Some(guard) => guard,
        None => match UnlockSlotGuard::acquire(window_later.clone(), waited.keys.clone()) {
            Some(guard) => guard,
            None => return,
        },
    };
    let encrypted = device.is_encrypted();
    let name = device.name();
    if encrypted {
        schedule_unlock_progress(&window_later, &waited.keys, &name);
    }
    let mount = DeviceMount {
        row_id,
        device,
        waited,
        attempt,
        guard,
    };
    spawn_device_mount(window_later, mount, credentials);
}

fn spawn_device_mount(
    window_later: WeakLater<App>,
    mount: DeviceMount,
    credentials: Option<MountCredentials>,
) {
    let encrypted = mount.device.is_encrypted();
    let resume_name = mount.device.name();
    let dismiss_later = window_later.clone();
    let dismiss_keys = mount.waited.keys.clone();
    let resume_later = window_later.clone();
    let resume_keys = mount.waited.keys.clone();
    let bound = BoundOperation::new(
        window_later.clone(),
        OperationOptions {
            encrypted,
            supplied: credentials,
            on_prompt_open: Some(Rc::new(move || {
                dismiss_unlock_progress(&dismiss_later, &dismiss_keys);
            })),
            on_submitted: Some(Rc::new(move || {
                if encrypted {
                    present_unlock_progress(&resume_later, &resume_keys, &resume_name);
                }
            })),
            on_unmount_progress: None,
        },
    );
    glib::MainContext::default().spawn_local(async move {
        let result = mount.device.run(bound.operation()).await;
        let outcome = bound.finish(result);
        finish_device_mount(window_later, mount, outcome);
    });
}

fn finish_device_mount(window_later: WeakLater<App>, mount: DeviceMount, outcome: MountOutcome) {
    let in_flight = outcome
        .result
        .as_ref()
        .err()
        .is_some_and(volume_error_is_in_flight_mount);
    if !in_flight {
        dismiss_unlock_progress(&window_later, &mount.waited.keys);
    }
    match mount.device.readiness(&outcome.result, &mount.waited) {
        DeviceReadiness::Ready => {
            open_unlocked_identity(window_later, mount);
        }
        DeviceReadiness::AwaitVolume => {
            await_started_drive_volume(window_later, mount);
        }
        DeviceReadiness::Failed => {
            if let Err(error) = outcome.result {
                handle_device_mount_error(
                    window_later,
                    mount,
                    error,
                    (outcome.attempted, outcome.details),
                );
            }
        }
    }
}

fn open_unlocked_identity(window_later: WeakLater<App>, mount: DeviceMount) {
    let location = mount
        .device
        .volume()
        .and_then(|volume| volume.get_mount())
        .and_then(|m| location_for_file(&m.root()))
        .or_else(|| successor_mount_location(&mount.waited));
    let dismissed = unlock_progress_dismissed(&mount.waited.keys);
    mount.guard.finish();
    refresh_all_sidebars();
    let user_asked_to_open = mount.attempt.user_asked_to_open;
    let row_id = mount.row_id;
    if let Some(later) = window_later.upgrade() {
        later.then(move |app: &mut App, tx: &mut Tx| {
            super::refresh(app);
            if let Some(location) = location {
                invalidate_mount_snapshot();
                match unlock_view_follow_up(
                    app.browser.active_location().as_ref(),
                    &location,
                    user_asked_to_open,
                    dismissed,
                ) {
                    UnlockViewFollowUp::Reload => {
                        shell::publish(app);
                        app.browser.reload_active();
                        shell::settle(app, tx);
                    }
                    UnlockViewFollowUp::Navigate => {
                        app.sidebar.current = Some(ItemId::Device(row_id));
                        shell::publish(app);
                        app.browser.navigate(location);
                        shell::settle(app, tx);
                    }
                    UnlockViewFollowUp::None => {}
                }
            }
            true
        });
    }
}

fn handle_device_mount_error(
    window_later: WeakLater<App>,
    mount: DeviceMount,
    error: glib::Error,
    (attempted, details): (Option<RetryDefaults>, Option<MountPromptDetails>),
) {
    if volume_error_is_authentication_failure(&error)
        && let Some(details) = details
    {
        prompt_device_retry(window_later, mount, attempted, details);
    } else if volume_error_is_in_flight_mount(&error) {
        await_foreign_mount(window_later, mount);
    } else {
        mount.guard.finish();
        refresh_all_sidebars();
        let message = (!mount_error_is_cancelled(&error)).then(|| gio_device_error_message(&error));
        if let Some(later) = window_later.upgrade() {
            later.then(move |app: &mut App, _: &mut Tx| {
                super::refresh(app);
                if let Some(message) = message {
                    error_dialog::show(app, Error::new(tr("Unable to mount volume"), message));
                }
                true
            });
        }
    }
}

fn prompt_device_retry(
    window_later: WeakLater<App>,
    mount: DeviceMount,
    attempted: Option<RetryDefaults>,
    details: MountPromptDetails,
) {
    let Some(later) = window_later.upgrade() else {
        return;
    };
    let retry_later = window_later.clone();
    let shared_mount = Rc::new(RefCell::new(Some(mount)));
    let submit_mount = Rc::clone(&shared_mount);
    let cancel_mount = Rc::clone(&shared_mount);
    later.then(move |app: &mut App, _: &mut Tx| {
        show_retry_prompt(
            app,
            &details,
            attempted.as_ref(),
            move |credentials| {
                let Some(mount) = submit_mount.borrow_mut().take() else {
                    return;
                };
                start_unlock_device(
                    retry_later,
                    UnlockRequest {
                        row_id: mount.row_id,
                        device: mount.device,
                        credentials: Some(credentials),
                        attempt: DeviceAttempt {
                            already_waited: false,
                            ..mount.attempt
                        },
                        guard: Some(mount.guard),
                    },
                );
            },
            move || {
                let _ = cancel_mount.borrow_mut().take();
            },
        );
        true
    });
}

fn await_foreign_mount(window_later: WeakLater<App>, mount: DeviceMount) {
    glib::MainContext::default().spawn_local(async move {
        let outcome = mount.device.wait_for_foreign_mount(&mount.waited).await;
        let successor = foreign_wait_successor(outcome, &mount.waited);
        match foreign_volume_wait_follow_up(outcome, mount.attempt.already_waited, successor) {
            ForeignVolumeWaitFollowUp::Navigate => open_unlocked_identity(window_later, mount),
            ForeignVolumeWaitFollowUp::StartOwnedMount
                if outcome == ForeignVolumeWaitOutcome::Gone =>
            {
                start_owned_successor(window_later, mount);
            }
            ForeignVolumeWaitFollowUp::StartOwnedMount => start_unlock_device(
                window_later,
                UnlockRequest {
                    row_id: mount.row_id,
                    device: mount.device,
                    credentials: None,
                    attempt: DeviceAttempt {
                        already_waited: true,
                        ..mount.attempt
                    },
                    guard: Some(mount.guard),
                },
            ),
            ForeignVolumeWaitFollowUp::Quiet => {
                mount.guard.finish();
                refresh_all_sidebars();
            }
        }
    });
}

fn start_owned_successor(window_later: WeakLater<App>, mount: DeviceMount) {
    let user_asked_to_open =
        mount.attempt.user_asked_to_open && !unlock_progress_dismissed(&mount.waited.keys);
    mount.guard.finish();
    if mount.waited.is_absent() {
        refresh_all_sidebars();
        return;
    }
    let device = successor_volume(&mount.waited)
        .map(UnlockDevice::Volume)
        .or_else(|| successor_password_drive(&mount.waited).map(UnlockDevice::Drive));
    if let Some(device) = device {
        start_unlock_device(
            window_later,
            UnlockRequest {
                row_id: mount.row_id,
                device,
                credentials: None,
                attempt: DeviceAttempt {
                    already_waited: false,
                    user_asked_to_open,
                },
                guard: None,
            },
        );
    } else {
        refresh_all_sidebars();
    }
}

fn await_started_drive_volume(window_later: WeakLater<App>, mount: DeviceMount) {
    glib::MainContext::default().spawn_local(async move {
        wait_until(
            || {
                successor_volume(&mount.waited).is_some()
                    || successor_mount_location(&mount.waited).is_some()
            },
            FOREIGN_VOLUME_MOUNT_WAIT,
        )
        .await;
        if successor_mount_location(&mount.waited).is_some() {
            open_unlocked_identity(window_later, mount);
        } else if successor_volume(&mount.waited).is_some() {
            start_owned_successor(window_later, mount);
        } else {
            mount.guard.finish();
            refresh_all_sidebars();
            if let Some(later) = window_later.upgrade() {
                later.then(move |app: &mut App, _: &mut Tx| {
                    super::refresh(app);
                    error_dialog::show(
                        app,
                        Error::new(
                            tr("Unable to mount volume"),
                            tr("The device started, but no mountable volume appeared."),
                        ),
                    );
                    true
                });
            }
        }
    });
}

fn begin_unlock_slot(keys: &DeviceKeys) -> bool {
    UNLOCK_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        if let Some(slot) = slots
            .iter_mut()
            .find(|slot| unlock_target_matches(&slot.keys, keys))
        {
            if slot.in_flight {
                return false;
            }
            slot.in_flight = true;
            slot.dismissed = false;
            return true;
        }
        slots.push(UnlockSlot {
            keys: keys.clone(),
            overlay_id: mount_prompts::next_id(),
            pending_timer: None,
            dismissed: false,
            in_flight: true,
        });
        true
    })
}

fn unlock_progress_dismissed(keys: &DeviceKeys) -> bool {
    UNLOCK_SLOTS.with(|slots| {
        slots
            .borrow()
            .iter()
            .find(|slot| unlock_target_matches(&slot.keys, keys))
            .is_some_and(|slot| slot.dismissed)
    })
}

fn schedule_unlock_progress(window_later: &WeakLater<App>, keys: &DeviceKeys, name: &str) {
    dismiss_unlock_progress(window_later, keys);
    if unlock_progress_dismissed(keys) {
        return;
    }
    let later = window_later.clone();
    let timer_keys = keys.clone();
    let name = name.to_owned();
    let source = glib::timeout_add_local_once(UNLOCK_PROGRESS_DELAY, move || {
        UNLOCK_SLOTS.with(|slots| {
            if let Some(slot) = slots
                .borrow_mut()
                .iter_mut()
                .find(|slot| unlock_target_matches(&slot.keys, &timer_keys))
            {
                slot.pending_timer = None;
            }
        });
        present_unlock_progress(&later, &timer_keys, &name);
    });
    let mut source = Some(source);
    UNLOCK_SLOTS.with(|slots| {
        if let Some(slot) = slots
            .borrow_mut()
            .iter_mut()
            .find(|slot| unlock_target_matches(&slot.keys, keys))
        {
            if let Some(previous) = slot.pending_timer.take() {
                previous.remove();
            }
            slot.pending_timer = source.take();
        }
    });
    if let Some(source) = source {
        source.remove();
    }
}

fn present_unlock_progress(window_later: &WeakLater<App>, keys: &DeviceKeys, name: &str) {
    let Some(later) = window_later.upgrade() else {
        return;
    };
    let overlay_id = UNLOCK_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        let slot = slots
            .iter_mut()
            .find(|slot| unlock_target_matches(&slot.keys, keys))?;
        if let Some(timer) = slot.pending_timer.take() {
            timer.remove();
        }
        (!slot.dismissed).then_some(slot.overlay_id)
    });
    let Some(overlay_id) = overlay_id else {
        return;
    };
    let volume_name = name.to_owned();
    let hide_keys = keys.clone();
    later.then(move |app: &mut App, _: &mut Tx| {
        mount_prompts::show_unlock_progress(app, overlay_id, volume_name, move || {
            UNLOCK_SLOTS.with(|slots| {
                if let Some(slot) = slots
                    .borrow_mut()
                    .iter_mut()
                    .find(|slot| unlock_target_matches(&slot.keys, &hide_keys))
                {
                    slot.dismissed = true;
                }
            });
        });
        true
    });
}

fn dismiss_unlock_progress(window_later: &WeakLater<App>, keys: &DeviceKeys) {
    let overlay_id = UNLOCK_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        let slot = slots
            .iter_mut()
            .find(|slot| unlock_target_matches(&slot.keys, keys))?;
        if let Some(timer) = slot.pending_timer.take() {
            timer.remove();
        }
        Some(slot.overlay_id)
    });
    if let Some(overlay_id) = overlay_id
        && let Some(later) = window_later.upgrade()
    {
        later.then(move |app: &mut App, _: &mut Tx| {
            mount_prompts::dismiss_unlock_progress(app, overlay_id);
            true
        });
    }
}

fn finish_unlock_slot(window_later: &WeakLater<App>, keys: &DeviceKeys) {
    dismiss_unlock_progress(window_later, keys);
    UNLOCK_SLOTS.with(|slots| {
        slots
            .borrow_mut()
            .retain(|slot| !unlock_target_matches(&slot.keys, keys));
    });
}

pub(super) fn mount_result_is_ok(result: &Result<(), glib::Error>) -> bool {
    match result {
        Ok(()) => true,
        Err(error) => error.matches(gio::IOErrorEnum::AlreadyMounted),
    }
}

pub(super) fn device_volume_mount_is_ready(
    result: &Result<(), glib::Error>,
    mount_present: bool,
) -> bool {
    mount_present || mount_result_is_ok(result)
}

pub(super) fn mount_error_is_cancelled(error: &glib::Error) -> bool {
    error.matches(gio::IOErrorEnum::Cancelled) || error.matches(gio::IOErrorEnum::FailedHandled)
}

pub(super) fn volume_error_is_authentication_failure(error: &glib::Error) -> bool {
    let message = error.message().to_ascii_lowercase();
    [
        "incorrect passphrase",
        "invalid passphrase",
        "no key available with this passphrase",
        "authentication failed",
    ]
    .iter()
    .any(|reason| message.contains(reason))
}

pub(super) fn volume_error_is_in_flight_mount(error: &glib::Error) -> bool {
    if error.matches(gio::IOErrorEnum::Pending) || error.matches(gio::IOErrorEnum::Busy) {
        return true;
    }
    let message = error.message().to_ascii_lowercase();
    ["already unlocking", "already in progress"]
        .iter()
        .any(|reason| message.contains(reason))
}

pub(super) fn unlock_view_follow_up(
    active: Option<&Location>,
    mount_location: &Location,
    user_asked_to_open: bool,
    progress_dismissed: bool,
) -> UnlockViewFollowUp {
    if active.is_some_and(|active| active == mount_location || active.is_within(mount_location)) {
        UnlockViewFollowUp::Reload
    } else if user_asked_to_open && !progress_dismissed {
        UnlockViewFollowUp::Navigate
    } else {
        UnlockViewFollowUp::None
    }
}

pub(super) fn foreign_volume_wait_follow_up(
    outcome: ForeignVolumeWaitOutcome,
    already_waited: bool,
    successor: VolumeSuccessorKind,
) -> ForeignVolumeWaitFollowUp {
    match outcome {
        ForeignVolumeWaitOutcome::Mounted => ForeignVolumeWaitFollowUp::Navigate,
        ForeignVolumeWaitOutcome::Gone => match successor {
            VolumeSuccessorKind::Mounted => ForeignVolumeWaitFollowUp::Navigate,
            VolumeSuccessorKind::Locked => ForeignVolumeWaitFollowUp::StartOwnedMount,
            VolumeSuccessorKind::Absent => ForeignVolumeWaitFollowUp::Quiet,
        },
        ForeignVolumeWaitOutcome::StillLocked if already_waited => ForeignVolumeWaitFollowUp::Quiet,
        ForeignVolumeWaitOutcome::StillLocked => ForeignVolumeWaitFollowUp::StartOwnedMount,
    }
}

fn foreign_wait_successor(
    outcome: ForeignVolumeWaitOutcome,
    waited: &DeviceMatch,
) -> VolumeSuccessorKind {
    match outcome {
        ForeignVolumeWaitOutcome::Mounted => VolumeSuccessorKind::Mounted,
        ForeignVolumeWaitOutcome::StillLocked => VolumeSuccessorKind::Locked,
        ForeignVolumeWaitOutcome::Gone => successor_kind(waited),
    }
}

fn collect_tokens(parts: impl IntoIterator<Item = Option<String>>) -> Vec<String> {
    parts
        .into_iter()
        .filter_map(|value| {
            let value = value?.trim().to_owned();
            (!value.is_empty()).then_some(value)
        })
        .collect()
}

fn tokens_overlap(left: &[String], right: &[String]) -> bool {
    left.iter().any(|token| right.contains(token))
}

fn same_volume(left: &DeviceKeys, right: &DeviceKeys) -> bool {
    tokens_overlap(&left.volume_tokens, &right.volume_tokens)
        || left
            .volume_object
            .as_ref()
            .is_some_and(|object| Some(object) == right.volume_object.as_ref())
}

fn same_drive(left: &DeviceKeys, right: &DeviceKeys) -> bool {
    tokens_overlap(&left.drive_tokens, &right.drive_tokens)
        || left
            .drive_object
            .as_ref()
            .is_some_and(|object| Some(object) == right.drive_object.as_ref())
}

fn unlock_target_matches(left: &DeviceKeys, right: &DeviceKeys) -> bool {
    if left.has_volume_identity() && right.has_volume_identity() {
        same_volume(left, right)
    } else {
        same_drive(left, right)
    }
}

fn unix_device_is_partition_of(volume_unix: &str, drive_unix: &str) -> bool {
    let Some(rest) = volume_unix.strip_prefix(drive_unix) else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    rest.starts_with(|ch: char| ch.is_ascii_digit())
        || rest
            .strip_prefix('p')
            .is_some_and(|digits| digits.starts_with(|ch: char| ch.is_ascii_digit()))
}

fn unix_device_is_sibling_partition(waited: &DeviceKeys, candidate: &DeviceKeys) -> bool {
    candidate.volume_tokens.iter().any(|volume_unix| {
        waited
            .drive_tokens
            .iter()
            .any(|drive_unix| unix_device_is_partition_of(volume_unix, drive_unix))
    })
}

fn identity_matches_volume(waited: &DeviceKeys, candidate: &DeviceKeys) -> bool {
    same_volume(waited, candidate)
        || (!waited.has_volume_identity()
            && same_drive(waited, candidate)
            && !unix_device_is_sibling_partition(waited, candidate))
}

fn device_keys(volume: Option<&gio::Volume>, drive: Option<&gio::Drive>) -> DeviceKeys {
    let mut keys = DeviceKeys::new(
        [
            volume.and_then(|v| unix_device(v).map(String::from)),
            volume.and_then(|v| v.uuid().map(String::from)),
            volume.and_then(crypto_password_uuid_for_volume),
        ],
        [
            drive.and_then(|d| {
                d.identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str())
                    .map(String::from)
            }),
            drive.and_then(|d| {
                d.identifier(gio::VOLUME_IDENTIFIER_KIND_UUID.as_str())
                    .map(String::from)
            }),
        ],
    );
    keys.volume_object = volume.map(|v| v.clone().upcast());
    keys.drive_object = drive.map(|d| d.clone().upcast());
    keys
}

fn successor_kind(waited: &DeviceMatch) -> VolumeSuccessorKind {
    if waited.is_absent() {
        return VolumeSuccessorKind::Absent;
    }
    let monitor = gio::VolumeMonitor::get();
    if monitor
        .mounts()
        .iter()
        .any(|mount| waited.matches_mount(mount))
    {
        return VolumeSuccessorKind::Mounted;
    }
    if monitor
        .volumes()
        .iter()
        .any(|volume| waited.matches_volume(volume))
    {
        return VolumeSuccessorKind::Locked;
    }
    if monitor
        .connected_drives()
        .iter()
        .any(|drive| waited.matches_password_drive(drive))
    {
        return VolumeSuccessorKind::Locked;
    }
    VolumeSuccessorKind::Absent
}

fn successor_mount_location(waited: &DeviceMatch) -> Option<Location> {
    gio::VolumeMonitor::get()
        .mounts()
        .into_iter()
        .find(|mount| waited.matches_mount(mount))
        .and_then(|mount| location_for_file(&mount.root()))
}

fn successor_volume(waited: &DeviceMatch) -> Option<gio::Volume> {
    gio::VolumeMonitor::get()
        .volumes()
        .into_iter()
        .find(|volume| waited.matches_volume(volume))
}

fn successor_password_drive(waited: &DeviceMatch) -> Option<gio::Drive> {
    gio::VolumeMonitor::get()
        .connected_drives()
        .into_iter()
        .find(|drive| waited.matches_password_drive(drive))
}

async fn wait_until(done: impl Fn() -> bool, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || done() {
            break;
        }
        glib::timeout_future(remaining.min(Duration::from_millis(200))).await;
    }
}

async fn wait_for_foreign_volume_mount(volume: &gio::Volume) -> ForeignVolumeWaitOutcome {
    if volume.get_mount().is_some() {
        return ForeignVolumeWaitOutcome::Mounted;
    }
    let removed = Rc::new(Cell::new(false));
    let removed_flag = Rc::clone(&removed);
    let removed_id = volume.connect_removed(move |_| removed_flag.set(true));
    wait_until(
        || volume.get_mount().is_some() || removed.get(),
        FOREIGN_VOLUME_MOUNT_WAIT,
    )
    .await;
    volume.disconnect(removed_id);
    if volume.get_mount().is_some() {
        ForeignVolumeWaitOutcome::Mounted
    } else if removed.get() {
        ForeignVolumeWaitOutcome::Gone
    } else {
        ForeignVolumeWaitOutcome::StillLocked
    }
}

async fn wait_for_foreign_drive_start(
    drive: &gio::Drive,
    waited: &DeviceMatch,
) -> ForeignVolumeWaitOutcome {
    let mounted = || successor_kind(waited) == VolumeSuccessorKind::Mounted;
    if mounted() {
        return ForeignVolumeWaitOutcome::Mounted;
    }
    let removed = Rc::new(Cell::new(false));
    let removed_flag = Rc::clone(&removed);
    let disconnected_id = drive.connect_disconnected(move |_| removed_flag.set(true));
    wait_until(|| mounted() || removed.get(), FOREIGN_VOLUME_MOUNT_WAIT).await;
    drive.disconnect(disconnected_id);
    if mounted() {
        ForeignVolumeWaitOutcome::Mounted
    } else if removed.get() {
        ForeignVolumeWaitOutcome::Gone
    } else {
        ForeignVolumeWaitOutcome::StillLocked
    }
}
