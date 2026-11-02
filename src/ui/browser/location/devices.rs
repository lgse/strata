// SPDX-License-Identifier: MIT

use crate::model::Location;
use crate::ui::window::crypto_password_uuid_for_volume;
use futures_channel::oneshot;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ForeignVolumeWaitOutcome {
    Mounted,
    StillLocked,
    Gone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VolumeSuccessorKind {
    Mounted,
    Locked,
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ForeignVolumeWaitFollowUp {
    Navigate,
    StartOwnedMount,
    Quiet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UnlockViewFollowUp {
    Reload,
    Navigate,
    None,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui::browser) struct DeviceKeys {
    volume_tokens: Vec<String>,
    drive_tokens: Vec<String>,
    pub(super) volume_object: Option<glib::Object>,
    pub(super) drive_object: Option<glib::Object>,
}

impl DeviceKeys {
    pub(super) fn new(
        volume: impl IntoIterator<Item = Option<String>>,
        drive: impl IntoIterator<Item = Option<String>>,
    ) -> Self {
        Self {
            volume_tokens: collect_device_tokens(volume),
            drive_tokens: collect_device_tokens(drive),
            volume_object: None,
            drive_object: None,
        }
    }

    pub(super) fn has_volume_identity(&self) -> bool {
        !self.volume_tokens.is_empty() || self.volume_object.is_some()
    }

    pub(super) fn is_empty(&self) -> bool {
        !self.has_volume_identity() && self.drive_tokens.is_empty() && self.drive_object.is_none()
    }
}

fn collect_device_tokens(parts: impl IntoIterator<Item = Option<String>>) -> Vec<String> {
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

pub(super) fn same_volume(left: &DeviceKeys, right: &DeviceKeys) -> bool {
    tokens_overlap(&left.volume_tokens, &right.volume_tokens)
        || left
            .volume_object
            .as_ref()
            .is_some_and(|object| Some(object) == right.volume_object.as_ref())
}

pub(super) fn same_drive(left: &DeviceKeys, right: &DeviceKeys) -> bool {
    tokens_overlap(&left.drive_tokens, &right.drive_tokens)
        || left
            .drive_object
            .as_ref()
            .is_some_and(|object| Some(object) == right.drive_object.as_ref())
}

pub(super) fn unlock_target_matches(left: &DeviceKeys, right: &DeviceKeys) -> bool {
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

pub(super) fn identity_matches_volume(waited: &DeviceKeys, candidate: &DeviceKeys) -> bool {
    same_volume(waited, candidate)
        || (!waited.has_volume_identity()
            && same_drive(waited, candidate)
            && !unix_device_is_sibling_partition(waited, candidate))
}

fn gio_identifier(value: Option<glib::GString>) -> Option<String> {
    value.map(|value| value.to_string())
}

pub(super) fn device_keys(volume: Option<&gio::Volume>, drive: Option<&gio::Drive>) -> DeviceKeys {
    let mut keys = DeviceKeys::new(
        [
            volume.and_then(|volume| {
                gio_identifier(volume.identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str()))
            }),
            volume.and_then(|volume| gio_identifier(volume.uuid())),
            volume.and_then(crypto_password_uuid_for_volume),
        ],
        [
            drive.and_then(|drive| {
                gio_identifier(drive.identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str()))
            }),
            drive.and_then(|drive| {
                gio_identifier(drive.identifier(gio::VOLUME_IDENTIFIER_KIND_UUID.as_str()))
            }),
        ],
    );
    keys.volume_object = volume.map(|volume| volume.clone().upcast());
    keys.drive_object = drive.map(|drive| drive.clone().upcast());
    keys
}

#[derive(Clone)]
pub(super) struct DeviceMatch {
    pub(super) keys: DeviceKeys,
}

impl DeviceMatch {
    pub(super) fn from_volume(volume: &gio::Volume) -> Self {
        let drive = volume.drive();
        Self {
            keys: device_keys(Some(volume), drive.as_ref()),
        }
    }

    pub(super) fn from_drive(drive: &gio::Drive) -> Self {
        Self {
            keys: device_keys(None, Some(drive)),
        }
    }

    pub(super) fn is_absent(&self) -> bool {
        self.keys.is_empty()
    }

    pub(super) fn matches_mount(&self, mount: &gio::Mount) -> bool {
        identity_matches_volume(
            &self.keys,
            &device_keys(mount.volume().as_ref(), mount.drive().as_ref()),
        )
    }

    pub(super) fn matches_volume(&self, volume: &gio::Volume) -> bool {
        identity_matches_volume(
            &self.keys,
            &device_keys(Some(volume), volume.drive().as_ref()),
        )
    }

    pub(super) fn matches_password_drive(&self, drive: &gio::Drive) -> bool {
        drive.start_stop_type() == gio::DriveStartStopType::Password
            && same_drive(&self.keys, &device_keys(None, Some(drive)))
    }
}

pub(super) fn successor_kind(waited: &DeviceMatch) -> VolumeSuccessorKind {
    if waited.is_absent() {
        return VolumeSuccessorKind::Absent;
    }
    let monitor = gio::VolumeMonitor::get();
    let volumes = monitor.volumes();
    if monitor
        .mounts()
        .iter()
        .any(|mount| waited.matches_mount(mount))
    {
        return VolumeSuccessorKind::Mounted;
    }
    if volumes.iter().any(|volume| waited.matches_volume(volume)) {
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

pub(super) fn successor_mount_location(waited: &DeviceMatch) -> Option<Location> {
    let monitor = gio::VolumeMonitor::get();
    monitor
        .mounts()
        .into_iter()
        .find(|mount| waited.matches_mount(mount))
        .and_then(|mount| crate::adapters::location_for_file(&mount.root()))
}

pub(super) fn successor_volume(waited: &DeviceMatch) -> Option<gio::Volume> {
    gio::VolumeMonitor::get()
        .volumes()
        .into_iter()
        .find(|volume| waited.matches_volume(volume))
}

pub(super) fn successor_password_drive(waited: &DeviceMatch) -> Option<gio::Drive> {
    gio::VolumeMonitor::get()
        .connected_drives()
        .into_iter()
        .find(|drive| waited.matches_password_drive(drive))
}

pub(super) const FOREIGN_VOLUME_MOUNT_WAIT: Duration = Duration::from_secs(8);

pub(super) async fn wait_for_mount_change(
    signal: oneshot::Receiver<()>,
    mounted: impl Fn() -> bool,
    timeout: Duration,
) {
    futures_lite::future::race(
        async {
            let _ = signal.await;
        },
        async {
            let deadline = Instant::now() + timeout;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() || mounted() {
                    break;
                }
                glib::timeout_future(remaining.min(Duration::from_millis(200))).await;
            }
        },
    )
    .await;
}

/// A one-shot completion that the first of several signal handlers fires.
fn completion_signal() -> (Rc<dyn Fn()>, oneshot::Receiver<()>) {
    let (tx, rx) = oneshot::channel();
    let tx = RefCell::new(Some(tx));
    let complete: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(tx) = tx.borrow_mut().take() {
            let _ = tx.send(());
        }
    });
    (complete, rx)
}

fn wait_outcome(mounted: bool, removed: bool) -> ForeignVolumeWaitOutcome {
    if mounted {
        ForeignVolumeWaitOutcome::Mounted
    } else if removed {
        ForeignVolumeWaitOutcome::Gone
    } else {
        ForeignVolumeWaitOutcome::StillLocked
    }
}

pub(super) async fn wait_for_foreign_volume_mount(
    volume: &gio::Volume,
) -> ForeignVolumeWaitOutcome {
    if volume.get_mount().is_some() {
        return ForeignVolumeWaitOutcome::Mounted;
    }
    let (complete, rx) = completion_signal();
    let removed = Rc::new(Cell::new(false));
    let changed_complete = complete.clone();
    let changed_volume = volume.clone();
    let changed_id = volume.connect_changed(move |_| {
        if changed_volume.get_mount().is_some() {
            changed_complete();
        }
    });
    let removed_flag = removed.clone();
    let removed_id = volume.connect_removed(move |_| {
        removed_flag.set(true);
        complete();
    });
    wait_for_mount_change(
        rx,
        || volume.get_mount().is_some(),
        FOREIGN_VOLUME_MOUNT_WAIT,
    )
    .await;
    volume.disconnect(changed_id);
    volume.disconnect(removed_id);
    wait_outcome(volume.get_mount().is_some(), removed.get())
}

pub(super) async fn wait_for_foreign_drive_start(
    drive: &gio::Drive,
    waited: &DeviceMatch,
) -> ForeignVolumeWaitOutcome {
    let mounted = || successor_kind(waited) == VolumeSuccessorKind::Mounted;
    if mounted() {
        return ForeignVolumeWaitOutcome::Mounted;
    }
    let (complete, rx) = completion_signal();
    let removed = Rc::new(Cell::new(false));
    let changed_complete = complete.clone();
    let changed_waited = waited.clone();
    let changed_id = drive.connect_changed(move |_| {
        if successor_kind(&changed_waited) == VolumeSuccessorKind::Mounted {
            changed_complete();
        }
    });
    let removed_flag = removed.clone();
    let disconnected_id = drive.connect_disconnected(move |_| {
        removed_flag.set(true);
        complete();
    });
    wait_for_mount_change(rx, mounted, FOREIGN_VOLUME_MOUNT_WAIT).await;
    drive.disconnect(changed_id);
    drive.disconnect(disconnected_id);
    wait_outcome(mounted(), removed.get())
}

/// What replaced a volume another process was mounting, by how the wait ended.
pub(super) fn foreign_wait_successor(
    outcome: ForeignVolumeWaitOutcome,
    waited: &DeviceMatch,
) -> VolumeSuccessorKind {
    match outcome {
        ForeignVolumeWaitOutcome::Mounted => VolumeSuccessorKind::Mounted,
        ForeignVolumeWaitOutcome::StillLocked => VolumeSuccessorKind::Locked,
        ForeignVolumeWaitOutcome::Gone => successor_kind(waited),
    }
}
