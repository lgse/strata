// SPDX-License-Identifier: MIT

use std::{cell::Cell, collections::HashMap, rc::Rc};

use gio::prelude::*;
use rune::Tx;
use strata_components::sidebar::{Device, DeviceGroup, ItemId, LockState, Release};
use strata_core::{i18n::tr, model::Location};
use strata_kit::icons::IconName;

use super::places::{self, Navigation};
use crate::{App, shell, slots::Later};

mod crypto;
mod listing;
mod lock;
mod order;
mod probes;
mod release;
mod unlock;

#[cfg(test)]
mod tests;

use listing::{Seen, Source};
use release::{
    ReleaseTarget, key_for_mount, key_for_volume, release_kind, release_unplug, unmatched_pending,
};

/// The labels the user gave devices, by `label_identity`.
type Labels = HashMap<String, String>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MediaRelease {
    EjectVolume,
    EjectMount,
    UnmountMount,
}

impl MediaRelease {
    pub(super) fn shown(self) -> Release {
        match self {
            Self::EjectVolume | Self::EjectMount => Release::Eject,
            Self::UnmountMount => Release::Unmount,
        }
    }
}

/// What a device row's context menu needs in order to build its sections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DeviceMenuInfo {
    pub(super) id: String,
    pub(super) root: Option<Location>,
    pub(super) lock: Option<LockState>,
    pub(super) release: Option<MediaRelease>,
    pub(super) can_mount: bool,
    pub(super) network: bool,
    pub(super) pending: bool,
}

pub(super) fn release_action(
    can_eject_volume: bool,
    mount_can_eject: bool,
    mount_can_unmount: bool,
) -> Option<MediaRelease> {
    if can_eject_volume {
        Some(MediaRelease::EjectVolume)
    } else if mount_can_eject {
        Some(MediaRelease::EjectMount)
    } else if mount_can_unmount {
        Some(MediaRelease::UnmountMount)
    } else {
        None
    }
}

/// What DEVICES shows, and what runs on its devices.
#[derive(Default)]
pub(super) struct Devices {
    /// The monitor's rows, one list per drive, as last read.
    seen: Vec<Vec<Seen>>,
    /// What C8 draws.
    groups: Vec<DeviceGroup>,
    /// The mounted rows, with where each opens.
    rows: Vec<places::Row>,
    watch: Option<Watch>,
    sidebar_watch: Option<release::SidebarWatch>,
}

impl Devices {
    pub(super) fn groups(&self) -> &[DeviceGroup] {
        &self.groups
    }

    pub(super) fn rows(&self) -> &[places::Row] {
        &self.rows
    }

    fn find(&self, id: &str) -> Option<&Seen> {
        self.seen.iter().flatten().find(|seen| seen.id == id)
    }

    pub(super) fn menu_info(&self, id: &str) -> Option<DeviceMenuInfo> {
        let seen = self.find(id)?;
        Some(DeviceMenuInfo {
            id: seen.id.clone(),
            root: seen.root.clone(),
            lock: seen.lock,
            release: seen.release,
            can_mount: seen.can_mount,
            network: seen.network,
            pending: release::is_pending(&seen.ids),
        })
    }

    pub(super) fn dress(&mut self, labels: &Labels) {
        let mut live_ids = Vec::new();
        let mut groups: Vec<DeviceGroup> = self
            .seen
            .iter()
            .map(|group| {
                DeviceGroup::new(
                    group
                        .iter()
                        .map(|seen| {
                            live_ids.push(seen.ids.clone());
                            device(seen, labels, release::is_pending(&seen.ids))
                        })
                        .collect(),
                )
            })
            .collect();
        let ghosts: Vec<Device> = unmatched_pending(&live_ids)
            .into_iter()
            .enumerate()
            .map(|(index, key)| {
                Device::new(
                    format!("releasing-{index}"),
                    key.display_name,
                    IconName::Drive,
                    "",
                )
                .mounted(false)
                .pending(true)
            })
            .collect();
        if !ghosts.is_empty() {
            groups.push(DeviceGroup::new(ghosts).divided(false));
        }
        self.groups = groups;
        self.rows = self.seen.iter().flatten().filter_map(place_row).collect();
    }
}

/// The C8 row of what the monitor shows.
fn device(seen: &Seen, labels: &Labels, releasing: bool) -> Device {
    let icon = if seen.network {
        IconName::Network
    } else {
        IconName::Drive
    };
    let mut row = Device::new(
        seen.id.clone(),
        display_name(seen.label_id.as_deref(), &seen.name, labels),
        icon,
        seen.description.clone(),
    )
    .mounted(seen.root.is_some())
    .pending(releasing);
    if let Some(lock) = seen.lock {
        row = row.lock(lock);
    }
    if let Some(release) = seen.release {
        row = row.release(release.shown());
    }
    row
}

pub(super) fn display_name(label_id: Option<&str>, name: &str, labels: &Labels) -> String {
    label_id
        .and_then(|id| labels.get(id))
        .filter(|label| is_shown_label(label))
        .cloned()
        .unwrap_or_else(|| name.to_owned())
}

fn is_shown_label(label: &str) -> bool {
    !label.trim().is_empty() && label.chars().count() <= 255 && !label.chars().any(char::is_control)
}

fn place_row(seen: &Seen) -> Option<places::Row> {
    let navigation = match seen.source {
        Source::Volume(_) | Source::PasswordDrive(_) => Navigation::Direct,
        Source::Mount(_) => Navigation::Validate,
        #[cfg(test)]
        Source::Synthetic => Navigation::Direct,
    };
    Some(places::Row {
        item: ItemId::Device(seen.id.clone()),
        location: seen.root.clone()?,
        navigation,
    })
}

pub(super) fn start(app: &mut App) {
    app.sidebar.devices.sidebar_watch = Some(release::register_sidebar(app));
    if !cfg!(test) {
        app.sidebar.devices.watch = Some(Watch::new(&app.sidebar.later));
    }
    refresh(app);
}

/// Reads the monitor again. None in this crate's tests: they never list the machine's drives.
pub(super) fn refresh(app: &mut App) {
    let local_only = places::Environment::current()
        .local_only(app.browser.is_chooser_mode())
        .local_only;
    if !cfg!(test) {
        app.sidebar.devices.seen = listing::read(local_only);
    }
    redress(app);
}

/// Draws the rows again, and marks the active location's row again.
pub(super) fn redress(app: &mut App) {
    let labels = shell::preferences(app).device_labels.clone();
    app.sidebar.devices.dress(&labels);
    app.sidebar.follow(&app.browser);
}

struct Watch {
    monitor: gio::VolumeMonitor,
    handlers: Vec<glib::SignalHandlerId>,
}

impl Watch {
    fn new(later: &Later<App>) -> Self {
        let monitor = gio::VolumeMonitor::get();
        let later = later.downgrade();
        let queued = Rc::new(Cell::new(false));
        let heard: Rc<dyn Fn()> = Rc::new(move || {
            if queued.replace(true) {
                return;
            }
            let Some(later) = later.upgrade() else {
                return;
            };
            let queued = Rc::clone(&queued);
            later.then(move |app: &mut App, _: &mut Tx| {
                queued.set(false);
                refresh(app);
                true
            });
        });
        let on = |heard: &Rc<dyn Fn()>| {
            let heard = Rc::clone(heard);
            move || heard()
        };
        let handlers = vec![
            monitor.connect_mount_added({
                let heard = on(&heard);
                move |_, _| heard()
            }),
            monitor.connect_mount_removed({
                let heard = on(&heard);
                move |_, _| heard()
            }),
            monitor.connect_mount_changed({
                let heard = on(&heard);
                move |_, _| heard()
            }),
            monitor.connect_volume_added({
                let heard = on(&heard);
                move |_, _| heard()
            }),
            monitor.connect_volume_removed({
                let heard = on(&heard);
                move |_, _| heard()
            }),
            monitor.connect_volume_changed({
                let heard = on(&heard);
                move |_, _| heard()
            }),
            monitor.connect_drive_connected({
                let heard = on(&heard);
                move |_, _| heard()
            }),
            monitor.connect_drive_disconnected({
                let heard = on(&heard);
                move |_, _| heard()
            }),
            monitor.connect_drive_changed({
                let heard = on(&heard);
                move |_, _| heard()
            }),
        ];
        Self { monitor, handlers }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        for handler in self.handlers.drain(..) {
            self.monitor.disconnect(handler);
        }
    }
}

pub(super) fn mount(app: &mut App, id: &str) {
    let Some(source) = app.sidebar.devices.find(id).map(|seen| seen.source.clone()) else {
        return;
    };
    match source {
        Source::Volume(volume) => unlock::mount_volume(app, id.to_owned(), volume),
        Source::PasswordDrive(drive) => unlock::start_password_drive(app, id.to_owned(), drive),
        Source::Mount(_) => {}
        #[cfg(test)]
        Source::Synthetic => {}
    }
}

/// Unlocks an encrypted volume or password-start drive (from its lock button or context menu).
pub(super) fn unlock(app: &mut App, id: &str) {
    mount(app, id);
}

/// Locks an unlocked encrypted volume or mount (from its lock button or context menu).
pub(super) fn lock(app: &mut App, id: &str) {
    let Some((source, label_id)) = app
        .sidebar
        .devices
        .find(id)
        .map(|seen| (seen.source.clone(), seen.label_id.clone()))
    else {
        return;
    };
    match source {
        Source::Volume(volume) => {
            lock::lock_encrypted_volume(app, id.to_owned(), &volume, label_id.as_deref());
        }
        Source::Mount(mount) => {
            lock::lock_encrypted_mount(app, id.to_owned(), &mount, label_id.as_deref());
        }
        Source::PasswordDrive(_) => {}
        #[cfg(test)]
        Source::Synthetic => {}
    }
}

/// Eject or Unmount on a device's row or context menu.
pub(super) fn release(app: &mut App, id: &str, tx: &mut Tx) {
    let Some(seen) = app.sidebar.devices.find(id).cloned() else {
        return;
    };
    if seen.release.is_none() {
        return;
    }
    match seen.source.clone() {
        Source::PasswordDrive(drive) => lock::eject_password_drive(app, drive, tx),
        Source::Volume(volume) => release_volume(app, volume, &seen, tx),
        Source::Mount(mount) => release_mount(app, mount, &seen, tx),
        #[cfg(test)]
        Source::Synthetic => {}
    }
}

fn release_volume(app: &mut App, volume: gio::Volume, seen: &Seen, tx: &mut Tx) {
    let Some(action) = seen.release else {
        return;
    };
    let mount = volume.get_mount();
    if matches!(
        action,
        MediaRelease::EjectMount | MediaRelease::UnmountMount
    ) && mount.is_none()
    {
        return;
    }
    let labels = shell::preferences(app).device_labels.clone();
    let target = ReleaseTarget {
        key: key_for_volume(&volume, seen.label_id.as_deref(), &labels),
        kind: release_kind(action),
        unplug: release_unplug(action, mount.as_ref(), Some(&volume)),
        sync_root: mount.as_ref().and_then(|m| m.root().path()),
        away_root: seen.root.clone(),
    };
    release::begin_release(app, target, Some(tx), move |operation| async move {
        release_volume_media(action, volume, mount, operation).await
    });
}

fn release_mount(app: &mut App, mount: gio::Mount, seen: &Seen, tx: &mut Tx) {
    let Some(action) = seen.release else {
        return;
    };
    let labels = shell::preferences(app).device_labels.clone();
    let target = ReleaseTarget {
        key: key_for_mount(&mount, seen.label_id.as_deref(), &labels),
        kind: release_kind(action),
        unplug: release_unplug(action, Some(&mount), None),
        sync_root: mount.root().path(),
        away_root: seen.root.clone(),
    };
    release::begin_release(app, target, Some(tx), move |operation| async move {
        release_mount_media(action, mount, operation).await
    });
}

async fn release_volume_media(
    action: MediaRelease,
    volume: gio::Volume,
    mount: Option<gio::Mount>,
    operation: gio::MountOperation,
) -> Result<(), glib::Error> {
    match (action, mount) {
        (MediaRelease::EjectVolume, _) => {
            volume
                .eject_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                .await
        }
        (_, None) => Err(glib::Error::new(
            gio::IOErrorEnum::Failed,
            &tr("The volume is not mounted."),
        )),
        (_, Some(mount)) => release_mount_media(action, mount, operation).await,
    }
}

async fn release_mount_media(
    action: MediaRelease,
    mount: gio::Mount,
    operation: gio::MountOperation,
) -> Result<(), glib::Error> {
    match action {
        MediaRelease::EjectVolume | MediaRelease::EjectMount => {
            mount
                .eject_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                .await
        }
        MediaRelease::UnmountMount => {
            mount
                .unmount_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                .await
        }
    }
}

/// Disconnects a network share from its context menu (`device_release::disconnect_share`).
pub(super) fn disconnect(app: &mut App, id: &str, tx: &mut Tx) {
    let Some(seen) = app.sidebar.devices.find(id).cloned() else {
        return;
    };
    if !seen.network {
        return;
    }
    let (Source::Mount(mount), Some(root)) = (seen.source, seen.root) else {
        return;
    };
    release::disconnect_share(
        app,
        release::ShareDisconnect {
            id: id.to_owned(),
            mount,
            root,
        },
        tx,
    );
}

/// Whether a browser at `active` is on the device mounted at `root`.
pub(super) fn leaves(active: &Location, root: &Location) -> bool {
    active == root || active.is_within(root)
}

#[cfg(test)]
fn seed_for_test(app: &mut App, groups: Vec<Vec<Seen>>) {
    app.sidebar.devices.seen = groups;
    redress(app);
}
