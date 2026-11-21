// SPDX-License-Identifier: MIT

//! DEVICES: the volumes and mounts GIO's volume monitor reports, on C8's device rows, as the
//! GTK window's sidebar lists them (`append_devices` in `src/ui/window/sidebar_devices.rs`).
//!
//! - **The rows** follow plug and unplug: the monitor's signals read it again ([`Watch`]). Each
//!   drive's rows go together ([`order`]). A row is named by the label the user gave the
//!   device, else by the system, and described by where it is mounted, else by its device.
//!   No device, no heading: C8 draws DEVICES only over rows.
//! - **A click** opens a mounted device's root; an unmounted volume is mounted first, then
//!   opened, and a failed mount shows "Unable to mount volume" with GIO's text, leaving the
//!   browser where it was (`mount_device_volume` in `src/ui/browser/location/volumes.rs`).
//! - **Eject or Unmount**, by what the device allows ([`release_action`], GTK's
//!   `volume_release_action`). The browser leaves a device it is in for Home first, the
//!   device's files are flushed, then GIO ejects or unmounts it, while the row shows a spinner
//!   (`begin_release` in `src/ui/window/device_release/run.rs`). A failure shows its error
//!   and, while the device is still mounted, returns the browser where it was.
//!
//! Not here yet, each in A3's notes (`docs/rune/parity/packages/A3.md`): the device rows'
//! menus, encrypted volumes and their lock buttons, password drives, Disconnect for network
//! shares, the stand-in of a device that went while its release ran, the safe-to-remove
//! notice, the probe for a volume another holder claimed, and leaving the device in the other
//! tabs and windows.

use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    io,
    path::PathBuf,
    rc::Rc,
};

use gio::prelude::*;
use rune::Tx;
use strata_components::sidebar::{Device, DeviceGroup, ItemId, Release};
use strata_core::{i18n::tr, model::Location};
use strata_fs::adapters::{flush_filesystem, location_for_file};
use strata_fs::services::{gio_device_error_message, io_error_message};
use strata_kit::icons::IconName;

use super::places::{self, Navigation};
use crate::{
    App,
    error_dialog::{self, Error},
    shell,
    slots::Later,
};

mod listing;
mod order;

#[cfg(test)]
mod tests;

use listing::{Seen, Source};

/// The labels the user gave devices, by `label_identity`.
type Labels = HashMap<String, String>;

/// How a device is safely removed: Eject wherever its drive or mount reports it, else Unmount
/// (GTK's `MediaRelease`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MediaRelease {
    EjectVolume,
    EjectMount,
    UnmountMount,
}

impl MediaRelease {
    fn shown(self) -> Release {
        match self {
            Self::EjectVolume | Self::EjectMount => Release::Eject,
            Self::UnmountMount => Release::Unmount,
        }
    }

    fn error_title(self) -> &'static str {
        match self {
            Self::EjectVolume | Self::EjectMount => "Unable to eject device",
            Self::UnmountMount => "Unable to unmount device",
        }
    }
}

/// GTK's `volume_release_action`: none for fixed storage that can do neither.
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
    /// The devices whose release is running.
    releasing: HashSet<String>,
    /// The volumes being mounted from a click.
    mounting: HashSet<String>,
    watch: Option<Watch>,
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

    /// Draws the rows again from what was last read, with the labels the settings give.
    pub(super) fn dress(&mut self, labels: &Labels) {
        self.groups = self
            .seen
            .iter()
            .map(|group| {
                DeviceGroup::new(
                    group
                        .iter()
                        .map(|seen| device(seen, labels, self.releasing.contains(&seen.id)))
                        .collect(),
                )
            })
            .collect();
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
    if let Some(release) = seen.release {
        row = row.release(release.shown());
    }
    row
}

/// The label the user gave the device, when it is one the settings may hold, else the
/// system's name (`device_label` in `src/ui/preferences/file_settings.rs`).
fn display_name(label_id: Option<&str>, name: &str, labels: &Labels) -> String {
    label_id
        .and_then(|id| labels.get(id))
        .filter(|label| is_shown_label(label))
        .cloned()
        .unwrap_or_else(|| name.to_owned())
}

fn is_shown_label(label: &str) -> bool {
    !label.trim().is_empty() && label.chars().count() <= 255 && !label.chars().any(char::is_control)
}

/// Where a mounted row opens: a volume's root at once, as GTK's `navigate_to_gio_file`; a
/// mount's checked first, as its place row.
fn place_row(seen: &Seen) -> Option<places::Row> {
    let navigation = match seen.source {
        Source::Volume(_) => Navigation::Direct,
        Source::Mount(_) => Navigation::Validate,
    };
    Some(places::Row {
        item: ItemId::Device(seen.id.clone()),
        location: seen.root.clone()?,
        navigation,
    })
}

/// The sidebar starts: the rows as the monitor shows them, and the watch that reads it again.
pub(super) fn start(app: &mut App) {
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
    app.sidebar.devices.seen = if cfg!(test) {
        Vec::new()
    } else {
        listing::read(local_only)
    };
    redress(app);
}

/// Draws the rows again, and marks the active location's row again.
pub(super) fn redress(app: &mut App) {
    let labels = shell::preferences(app).device_labels.clone();
    app.sidebar.devices.dress(&labels);
    app.sidebar.follow(&app.browser);
}

/// The monitor's signals, held while the sidebar is: each reads it again once, after what is
/// queued (`connect_device_changes` in `src/ui/window/sidebar.rs`).
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

/// A click on an unmounted volume: GIO mounts it, then its root opens. The row is marked
/// before, as GTK's `select_sidebar_row`; a second click while it mounts asks nothing more.
pub(super) fn mount(app: &mut App, id: &str) {
    let devices = &mut app.sidebar.devices;
    let Some(Source::Volume(volume)) = devices.find(id).map(|seen| seen.source.clone()) else {
        return;
    };
    if !devices.mounting.insert(id.to_owned()) {
        return;
    }
    app.sidebar.current = Some(ItemId::Device(id.to_owned()));
    let later = app.sidebar.later.clone();
    let id = id.to_owned();
    glib::MainContext::default().spawn_local(async move {
        let result = volume
            .mount_future(gio::MountMountFlags::NONE, None::<&gio::MountOperation>)
            .await;
        let done = Mounted { id, volume, result };
        later.then(move |app: &mut App, tx: &mut Tx| {
            mounted(app, done, tx);
            true
        });
    });
}

/// How a mount from a click ended.
struct Mounted {
    id: String,
    volume: gio::Volume,
    result: Result<(), glib::Error>,
}

fn mounted(app: &mut App, done: Mounted, tx: &mut Tx) {
    let Mounted { id, volume, result } = done;
    app.sidebar.devices.mounting.remove(&id);
    refresh(app);
    match result {
        Ok(()) => {
            let root = volume
                .get_mount()
                .and_then(|mount| location_for_file(&mount.root()));
            if let Some(root) = root {
                app.sidebar.current = Some(ItemId::Device(id));
                shell::publish(app);
                app.browser.navigate(root);
                shell::settle(app, tx);
            }
        }
        Err(error) => {
            if !is_cancelled(&error) {
                let message = gio_device_error_message(&error);
                error_dialog::show(app, Error::new(tr("Unable to mount volume"), message));
            }
        }
    }
}

/// A dismissed prompt, which GTK shows nothing for (`mount_error_is_cancelled`).
fn is_cancelled(error: &glib::Error) -> bool {
    error.matches(gio::IOErrorEnum::Cancelled) || error.matches(gio::IOErrorEnum::FailedHandled)
}

/// Why a release failed: its flush, or GIO.
enum Failure {
    Flush(io::Error),
    Gio(glib::Error),
}

/// Eject or Unmount on a device's row.
pub(super) fn release(app: &mut App, id: &str, tx: &mut Tx) {
    let Some(seen) = app.sidebar.devices.find(id).cloned() else {
        return;
    };
    let Some(action) = seen.release else {
        return;
    };
    if !app.sidebar.devices.releasing.insert(id.to_owned()) {
        return;
    }
    // Leave first, so the folder's monitors let the device go.
    let left = seen.root.as_ref().and_then(|root| leave(app, root, tx));
    redress(app);
    let later = app.sidebar.later.clone();
    let id = id.to_owned();
    let sync_root = seen
        .root
        .as_ref()
        .and_then(Location::native_path)
        .map(ToOwned::to_owned);
    glib::MainContext::default().spawn_local(async move {
        let result = run_release(action, seen.source, sync_root).await;
        let done = Released {
            id,
            action,
            result,
            left,
        };
        later.then(move |app: &mut App, tx: &mut Tx| {
            released(app, done, tx);
            true
        });
    });
}

/// Where the browser was on a device it left for Home.
struct Left {
    before: Location,
    home: Location,
}

/// The browser goes Home from inside `root`; what it left, to come back to if the release
/// fails (GTK's `navigate_home_if_within` and `WayBack`).
fn leave(app: &mut App, root: &Location, tx: &mut Tx) -> Option<Left> {
    let before = app.browser.active_location()?;
    if !leaves(&before, root) {
        return None;
    }
    let home = Location::local(crate::launch::home_directory());
    shell::publish(app);
    app.browser.navigate(home.clone());
    shell::settle(app, tx);
    Some(Left { before, home })
}

/// Whether a browser at `active` is on the device mounted at `root`.
pub(super) fn leaves(active: &Location, root: &Location) -> bool {
    active == root || active.is_within(root)
}

async fn run_release(
    action: MediaRelease,
    source: Source,
    sync_root: Option<PathBuf>,
) -> Result<(), Failure> {
    if let Some(root) = sync_root {
        // Even opening a stalled mount must not block the interface thread.
        gio::spawn_blocking(move || flush_filesystem(&root))
            .await
            .unwrap_or_else(|_| Err(io::Error::other("the flush stopped")))
            .map_err(Failure::Flush)?;
    }
    let flags = gio::MountUnmountFlags::NONE;
    let operation = None::<&gio::MountOperation>;
    let mount = match &source {
        Source::Volume(volume) if action == MediaRelease::EjectVolume => {
            return volume
                .eject_with_operation_future(flags, operation)
                .await
                .map_err(Failure::Gio);
        }
        Source::Volume(volume) => volume.get_mount(),
        Source::Mount(mount) => Some(mount.clone()),
    };
    let Some(mount) = mount else {
        let error = glib::Error::new(gio::IOErrorEnum::Failed, &tr("The volume is not mounted."));
        return Err(Failure::Gio(error));
    };
    let result = match action {
        MediaRelease::UnmountMount => mount.unmount_with_operation_future(flags, operation).await,
        MediaRelease::EjectVolume | MediaRelease::EjectMount => {
            mount.eject_with_operation_future(flags, operation).await
        }
    };
    result.map_err(Failure::Gio)
}

/// How a release ended.
struct Released {
    id: String,
    action: MediaRelease,
    result: Result<(), Failure>,
    left: Option<Left>,
}

fn released(app: &mut App, done: Released, tx: &mut Tx) {
    let Released {
        id,
        action,
        result,
        left,
    } = done;
    app.sidebar.devices.releasing.remove(&id);
    refresh(app);
    let message = match result {
        Ok(()) => return,
        Err(Failure::Gio(error)) if is_cancelled(&error) => None,
        Err(Failure::Gio(error)) => Some(gio_device_error_message(&error)),
        Err(Failure::Flush(error)) => Some(io_error_message(&error)),
    };
    if let Some(before) = way_back(app, &id, left) {
        shell::publish(app);
        app.browser.navigate(before);
        shell::settle(app, tx);
    }
    if let Some(message) = message {
        error_dialog::show(app, Error::new(tr(action.error_title()), message));
    }
}

/// Where a failed release returns the browser: where it was, while the device is still
/// mounted and the browser still at the Home it was sent to.
fn way_back(app: &App, id: &str, left: Option<Left>) -> Option<Location> {
    let Left { before, home } = left?;
    let mounted = app
        .sidebar
        .devices
        .find(id)
        .is_some_and(|seen| seen.root.is_some());
    let at_home = app.browser.active_location().as_ref() == Some(&home);
    (mounted && at_home).then_some(before)
}
