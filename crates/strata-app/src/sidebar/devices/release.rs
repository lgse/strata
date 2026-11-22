// SPDX-License-Identifier: MIT

//! Unified device release lifecycle (eject, unmount, lock), network share disconnect,
//! process-wide pending-release tracking for stand-in ghost rows, multi-tab/multi-window
//! leave-before-release, and the 350 ms progress / safe-to-remove overlay
//! (`src/ui/window/device_release.rs` and its submodules).

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    future::Future,
    io,
    path::PathBuf,
    rc::{Rc, Weak},
    time::Duration,
};

use gio::prelude::*;
use rune::Tx;
use strata_core::{i18n::tr, model::Location};
use strata_fs::{
    adapters::{
        ActiveOperation, flush_filesystem, mount_can_unplug, mount_watch, volume_can_unplug,
    },
    app::Browser,
    services::{gio_device_error_message, io_error_message},
};
use strata_kit::icons::IconName;

use super::{Labels, MediaRelease, display_name, leaves, listing::unix_device};
use crate::{
    App,
    error_dialog::{self, Error},
    mount_prompts::{
        self, ReleaseOverlaySpec,
        operation::{BoundOperation, OperationOptions},
    },
    shell,
    slots::{Later, WeakLater},
};

const RELEASE_PROGRESS_DELAY: Duration = Duration::from_millis(350);
const RELEASE_BODY: &str =
    "Writing data to the drive. Do not unplug it. You can hide this and keep working.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReleaseKind {
    Unmount,
    Eject,
    Lock,
}

impl ReleaseKind {
    pub(super) fn title(self) -> String {
        tr(match self {
            Self::Unmount => "Unmounting",
            Self::Eject => "Ejecting",
            Self::Lock => "Locking volume",
        })
    }

    pub(super) fn icon(self) -> IconName {
        match self {
            Self::Lock => IconName::Lock,
            Self::Unmount | Self::Eject => IconName::Eject,
        }
    }

    pub(super) fn error_title(self) -> &'static str {
        match self {
            Self::Unmount => "Unable to unmount device",
            Self::Eject => "Unable to eject device",
            Self::Lock => "Unable to lock device",
        }
    }

    pub(super) fn safe_to_remove(self) -> bool {
        matches!(self, Self::Unmount | Self::Eject)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct DeviceIds {
    pub(super) unix_device: Option<String>,
    pub(super) uuid: Option<String>,
    pub(super) mount_root: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReleaseKey {
    pub(super) unix_device: Option<String>,
    pub(super) uuid: Option<String>,
    pub(super) mount_root: Option<PathBuf>,
    pub(super) display_name: String,
}

impl ReleaseKey {
    pub(super) fn ids(&self) -> DeviceIds {
        DeviceIds {
            unix_device: self.unix_device.clone(),
            uuid: self.uuid.clone(),
            mount_root: self.mount_root.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReleaseDecision {
    pub(super) dialog_title: Option<String>,
    pub(super) dialog_message: Option<String>,
    pub(super) safe_to_remove: bool,
}

pub(super) struct ReleaseTarget {
    pub(super) key: ReleaseKey,
    pub(super) kind: ReleaseKind,
    pub(super) unplug: bool,
    pub(super) sync_root: Option<PathBuf>,
    pub(super) away_root: Option<Location>,
}

struct PendingRelease {
    key: ReleaseKey,
    kind: ReleaseKind,
    overlay_id: u64,
    overlay_shown: bool,
    dismissed: bool,
    progress_message: Option<String>,
    unplug: bool,
    present_source: Option<glib::SourceId>,
}

struct WatchEntry {
    window_id: u64,
    window_later: WeakLater<App>,
    browser: Weak<Browser>,
}

/// RAII guard for an in-flight network share disconnect.
struct DisconnectGuard(String);

impl DisconnectGuard {
    fn acquire(id: String) -> Option<Self> {
        DISCONNECTING
            .with(|set| set.borrow_mut().insert(id.clone()))
            .then_some(Self(id))
    }
}

impl Drop for DisconnectGuard {
    fn drop(&mut self) {
        DISCONNECTING.with(|set| set.borrow_mut().remove(&self.0));
    }
}

thread_local! {
    static RELEASES: RefCell<Vec<PendingRelease>> = const { RefCell::new(Vec::new()) };
    static DISCONNECTING: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static WATCHES: RefCell<Vec<(u64, Weak<WatchEntry>)>> = const { RefCell::new(Vec::new()) };
    static NEXT_WATCH: Cell<u64> = const { Cell::new(1) };
}

/// Held by each started sidebar so releases can leave every tab and window and refresh them.
pub(super) struct SidebarWatch {
    id: u64,
    _entry: Rc<WatchEntry>,
}

impl Drop for SidebarWatch {
    fn drop(&mut self) {
        WATCHES.with(|watches| watches.borrow_mut().retain(|(id, _)| *id != self.id));
    }
}

pub(super) fn register_sidebar(app: &App) -> SidebarWatch {
    let id = NEXT_WATCH.with(|next| {
        let id = next.get();
        next.set(id.saturating_add(1));
        id
    });
    let entry = Rc::new(WatchEntry {
        window_id: shell::window_id(app),
        window_later: shell::window_later(app).downgrade(),
        browser: Rc::downgrade(&app.browser),
    });
    WATCHES.with(|watches| watches.borrow_mut().push((id, Rc::downgrade(&entry))));
    SidebarWatch { id, _entry: entry }
}

/// Queues a device sidebar refresh on every live window in the process.
pub(super) fn refresh_all_sidebars() {
    let mut seen_windows = HashSet::new();
    let laters: Vec<Later<App>> = WATCHES.with(|watches| {
        watches
            .borrow()
            .iter()
            .filter_map(|(_, entry)| entry.upgrade())
            .filter(|entry| seen_windows.insert(entry.window_id))
            .filter_map(|entry| entry.window_later.upgrade())
            .collect()
    });
    for later in laters {
        later.then(|app: &mut App, _: &mut Tx| {
            super::refresh(app);
            true
        });
    }
}

pub(super) fn release_kind(action: MediaRelease) -> ReleaseKind {
    match action {
        MediaRelease::UnmountMount => ReleaseKind::Unmount,
        MediaRelease::EjectVolume | MediaRelease::EjectMount => ReleaseKind::Eject,
    }
}

pub(super) fn release_unplug(
    action: MediaRelease,
    mount: Option<&gio::Mount>,
    volume: Option<&gio::Volume>,
) -> bool {
    match action {
        MediaRelease::EjectVolume | MediaRelease::EjectMount => true,
        MediaRelease::UnmountMount => {
            mount.is_some_and(mount_can_unplug) || volume.is_some_and(volume_can_unplug)
        }
    }
}

pub(super) fn device_matches(key: &ReleaseKey, device: &DeviceIds) -> bool {
    key.unix_device
        .as_ref()
        .is_some_and(|id| device.unix_device.as_ref() == Some(id))
        || key
            .uuid
            .as_ref()
            .is_some_and(|id| device.uuid.as_ref() == Some(id))
        || key
            .mount_root
            .as_ref()
            .is_some_and(|root| device.mount_root.as_ref() == Some(root))
}

fn keys_match(left: &ReleaseKey, right: &ReleaseKey) -> bool {
    device_matches(left, &right.ids())
}

pub(super) fn is_pending(device: &DeviceIds) -> bool {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .any(|entry| device_matches(&entry.key, device))
    })
}

pub(super) fn unmatched_pending(live: &[DeviceIds]) -> Vec<ReleaseKey> {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .filter(|entry| !live.iter().any(|device| device_matches(&entry.key, device)))
            .map(|entry| entry.key.clone())
            .collect()
    })
}

pub(super) fn ids_for_volume(volume: &gio::Volume) -> DeviceIds {
    let mount = volume.get_mount();
    DeviceIds {
        unix_device: unix_device(volume)
            .map(String::from)
            .or_else(|| volume.drive().and_then(|drive| drive_unix_device(&drive))),
        uuid: volume.uuid().map(String::from),
        mount_root: mount.as_ref().and_then(|mount| mount.root().path()),
    }
}

pub(super) fn key_for_volume(
    volume: &gio::Volume,
    label_id: Option<&str>,
    labels: &Labels,
) -> ReleaseKey {
    let ids = ids_for_volume(volume);
    ReleaseKey {
        unix_device: ids.unix_device,
        uuid: ids.uuid,
        mount_root: ids.mount_root,
        display_name: display_name(label_id, &volume.name(), labels),
    }
}

pub(super) fn ids_for_mount(mount: &gio::Mount) -> DeviceIds {
    let volume = mount.volume();
    DeviceIds {
        unix_device: mount
            .drive()
            .and_then(|drive| drive_unix_device(&drive))
            .or_else(|| volume.as_ref().and_then(|v| ids_for_volume(v).unix_device)),
        uuid: volume
            .as_ref()
            .and_then(|v| v.uuid())
            .map(String::from)
            .or_else(|| {
                mount
                    .drive()
                    .and_then(|drive| drive.identifier(gio::VOLUME_IDENTIFIER_KIND_UUID.as_str()))
                    .map(String::from)
            }),
        mount_root: mount.root().path(),
    }
}

pub(super) fn key_for_mount(
    mount: &gio::Mount,
    label_id: Option<&str>,
    labels: &Labels,
) -> ReleaseKey {
    let ids = ids_for_mount(mount);
    ReleaseKey {
        unix_device: ids.unix_device,
        uuid: ids.uuid,
        mount_root: ids.mount_root,
        display_name: display_name(label_id, &mount.name(), labels),
    }
}

pub(super) fn ids_for_drive(drive: &gio::Drive) -> DeviceIds {
    DeviceIds {
        unix_device: drive_unix_device(drive),
        uuid: drive
            .identifier(gio::VOLUME_IDENTIFIER_KIND_UUID.as_str())
            .map(String::from),
        mount_root: None,
    }
}

pub(super) fn key_for_drive(drive: &gio::Drive) -> ReleaseKey {
    let ids = ids_for_drive(drive);
    ReleaseKey {
        unix_device: ids.unix_device,
        uuid: ids.uuid,
        mount_root: None,
        display_name: drive.name().to_string(),
    }
}

fn drive_unix_device(drive: &gio::Drive) -> Option<String> {
    drive
        .identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str())
        .map(String::from)
}

pub(super) fn body_for_progress(message: Option<&str>) -> String {
    match message.map(str::trim).filter(|line| !line.is_empty()) {
        Some(message) => {
            rust_i18n::t!("%{message}\nDo not unplug it.", message = message).into_owned()
        }
        None => tr(RELEASE_BODY),
    }
}

pub(super) fn decide_flush(kind: ReleaseKind, error: &io::Error) -> ReleaseDecision {
    ReleaseDecision {
        dialog_title: Some(kind.error_title().to_owned()),
        dialog_message: Some(io_error_message(error)),
        safe_to_remove: false,
    }
}

pub(super) fn decide_gio(
    kind: ReleaseKind,
    progress: Option<&str>,
    result: Result<(), glib::Error>,
    unplug: bool,
) -> ReleaseDecision {
    match result {
        Ok(()) => ReleaseDecision {
            dialog_title: None,
            dialog_message: None,
            safe_to_remove: kind.safe_to_remove() && unplug,
        },
        Err(error) if release_error_is_quiet(&error) => ReleaseDecision {
            dialog_title: None,
            dialog_message: None,
            safe_to_remove: false,
        },
        Err(error) => {
            let mut message = gio_device_error_message(&error);
            if let Some(progress) = progress.map(str::trim).filter(|line| !line.is_empty()) {
                message.push('\n');
                message.push_str(progress);
            }
            ReleaseDecision {
                dialog_title: Some(kind.error_title().to_owned()),
                dialog_message: Some(message),
                safe_to_remove: false,
            }
        }
    }
}

pub(super) fn release_error_is_quiet(error: &glib::Error) -> bool {
    error.matches(gio::IOErrorEnum::Cancelled)
        || error.matches(gio::IOErrorEnum::FailedHandled)
        || error.message().to_ascii_lowercase().contains("aborted")
}

/// Starts a release unless one matching `target.key` is already in flight.
pub(super) fn begin_release<F, Fut>(
    app: &mut App,
    target: ReleaseTarget,
    tx: Option<&mut Tx>,
    run_gio: F,
) -> bool
where
    F: FnOnce(gio::MountOperation) -> Fut + 'static,
    Fut: Future<Output = Result<(), glib::Error>> + 'static,
{
    begin_release_with(app, target, (tx, Box::new(|| {})), run_gio)
}

/// Starts a release with a settled callback invoked after cleanup.
pub(super) fn begin_release_with<F, Fut>(
    app: &mut App,
    target: ReleaseTarget,
    (tx, on_settled): (Option<&mut Tx>, Box<dyn FnOnce()>),
    run_gio: F,
) -> bool
where
    F: FnOnce(gio::MountOperation) -> Fut + 'static,
    Fut: Future<Output = Result<(), glib::Error>> + 'static,
{
    let overlay_id = mount_prompts::next_id();
    if !register_release(&target, overlay_id) {
        return false;
    }
    let way_back = target
        .away_root
        .as_ref()
        .map_or_else(WayBack::default, |root| {
            leave_device_across_all(app, root, tx)
        });
    super::redress(app);
    refresh_all_sidebars();

    let window_later = shell::window_later(app).downgrade();
    schedule_overlay(&window_later, target.key.clone(), overlay_id);

    glib::MainContext::default().spawn_local(async move {
        let _reads = release_reads(target.sync_root.clone()).await;
        let decision = run_flush_and_gio(&window_later, &target, &way_back, run_gio).await;
        let finished = finish_release(&target.key);
        on_settled();
        refresh_all_sidebars();
        if let Some(later) = window_later.upgrade() {
            later.then(move |app: &mut App, _: &mut Tx| {
                settle_release(app, finished, decision);
                true
            });
        }
    });
    true
}

fn register_release(target: &ReleaseTarget, overlay_id: u64) -> bool {
    RELEASES.with(|releases| {
        let mut releases = releases.borrow_mut();
        if releases
            .iter()
            .any(|entry| keys_match(&entry.key, &target.key))
        {
            return false;
        }
        releases.push(PendingRelease {
            key: target.key.clone(),
            kind: target.kind,
            overlay_id,
            overlay_shown: false,
            dismissed: false,
            progress_message: None,
            unplug: target.unplug,
            present_source: None,
        });
        true
    })
}

fn schedule_overlay(window_later: &WeakLater<App>, key: ReleaseKey, overlay_id: u64) {
    let later = window_later.clone();
    let present_key = key.clone();
    let source = glib::timeout_add_local_once(RELEASE_PROGRESS_DELAY, move || {
        present_overlay(&later, &present_key, overlay_id);
    });
    RELEASES.with(|releases| {
        if let Some(entry) = releases
            .borrow_mut()
            .iter_mut()
            .find(|entry| keys_match(&entry.key, &key))
        {
            if let Some(previous) = entry.present_source.replace(source) {
                previous.remove();
            }
        } else {
            source.remove();
        }
    });
}

fn present_overlay(window_later: &WeakLater<App>, key: &ReleaseKey, overlay_id: u64) {
    let Some(later) = window_later.upgrade() else {
        return;
    };
    let spec = RELEASES.with(|releases| {
        let mut releases = releases.borrow_mut();
        let entry = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))?;
        entry.present_source = None;
        if entry.dismissed || entry.overlay_shown {
            return None;
        }
        entry.overlay_shown = true;
        Some(ReleaseOverlaySpec {
            id: overlay_id,
            title: entry.kind.title(),
            subtitle: entry.key.display_name.clone(),
            body: body_for_progress(entry.progress_message.as_deref()),
            icon: entry.kind.icon(),
        })
    });
    let Some(spec) = spec else {
        return;
    };
    let hide_key = key.clone();
    later.then(move |app: &mut App, _: &mut Tx| {
        mount_prompts::show_release_overlay(app, spec, move || mark_dismissed(&hide_key));
        true
    });
}

fn mark_dismissed(key: &ReleaseKey) {
    RELEASES.with(|releases| {
        if let Some(entry) = releases
            .borrow_mut()
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))
        {
            entry.dismissed = true;
            entry.overlay_shown = false;
            if let Some(source) = entry.present_source.take() {
                source.remove();
            }
        }
    });
}

fn note_unmount_progress(window_later: &WeakLater<App>, key: &ReleaseKey, message: &str) {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return;
    }
    let update = RELEASES.with(|releases| {
        let mut releases = releases.borrow_mut();
        let entry = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))?;
        entry.progress_message = Some(trimmed.to_owned());
        entry
            .overlay_shown
            .then(|| (entry.overlay_id, body_for_progress(Some(trimmed))))
    });
    if let Some((overlay_id, body)) = update
        && let Some(later) = window_later.upgrade()
    {
        later.then(move |app: &mut App, _: &mut Tx| {
            mount_prompts::update_release_overlay_body(app, overlay_id, body);
            true
        });
    }
}

/// Cancels Strata's own reads under `sync_root` so flush and unmount do not hit `EBUSY`.
async fn release_reads(sync_root: Option<PathBuf>) -> ActiveOperation {
    let operation = ActiveOperation::begin([]);
    if let Some(root) = sync_root {
        operation
            .release_lexically(vec![root], &gio::Cancellable::new())
            .await;
    }
    operation
}

async fn run_flush_and_gio<F, Fut>(
    window_later: &WeakLater<App>,
    target: &ReleaseTarget,
    way_back: &WayBack,
    run_gio: F,
) -> ReleaseDecision
where
    F: FnOnce(gio::MountOperation) -> Fut,
    Fut: Future<Output = Result<(), glib::Error>>,
{
    if let Err(error) = flush_mount(target.sync_root.clone()).await {
        restore_if_local_mounted(target.sync_root.as_deref(), way_back).await;
        return decide_flush(target.kind, &error);
    }
    let progress_later = window_later.clone();
    let progress_key = target.key.clone();
    let bound = BoundOperation::new(
        window_later.clone(),
        OperationOptions {
            on_unmount_progress: Some(Rc::new(move |message| {
                note_unmount_progress(&progress_later, &progress_key, message);
            })),
            ..OperationOptions::default()
        },
    );
    let result = run_gio(bound.operation().clone()).await;
    let outcome = bound.finish(result);
    if outcome.result.is_err() {
        restore_if_local_mounted(target.sync_root.as_deref(), way_back).await;
    }
    let (progress, unplug) = RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .find(|entry| keys_match(&entry.key, &target.key))
            .map(|entry| (entry.progress_message.clone(), entry.unplug))
            .unwrap_or((None, target.unplug))
    });
    decide_gio(target.kind, progress.as_deref(), outcome.result, unplug)
}

struct FinishedRelease {
    overlay_id: u64,
    overlay_shown: bool,
    dismissed: bool,
    display_name: String,
}

fn finish_release(key: &ReleaseKey) -> Option<FinishedRelease> {
    RELEASES.with(|releases| {
        let mut releases = releases.borrow_mut();
        let index = releases
            .iter()
            .position(|entry| keys_match(&entry.key, key))?;
        let mut entry = releases.remove(index);
        if let Some(source) = entry.present_source.take() {
            source.remove();
        }
        Some(FinishedRelease {
            overlay_id: entry.overlay_id,
            overlay_shown: entry.overlay_shown,
            dismissed: entry.dismissed,
            display_name: entry.key.display_name,
        })
    })
}

fn settle_release(app: &mut App, finished: Option<FinishedRelease>, decision: ReleaseDecision) {
    if let Some(finished) = finished {
        if decision.safe_to_remove && finished.overlay_shown && !finished.dismissed {
            mount_prompts::show_release_safe_to_remove(
                app,
                finished.overlay_id,
                &finished.display_name,
            );
        } else {
            mount_prompts::dismiss_release_overlay(app, finished.overlay_id);
        }
    }
    super::refresh(app);
    if let (Some(title), Some(message)) = (decision.dialog_title, decision.dialog_message) {
        error_dialog::show(app, Error::new(tr(&title), message));
    }
}

async fn flush_mount(root: Option<PathBuf>) -> Result<(), io::Error> {
    let Some(root) = root else {
        return Ok(());
    };
    gio::spawn_blocking(move || flush_filesystem(&root))
        .await
        .unwrap_or_else(|_| Err(io::Error::other("filesystem sync panicked")))
}

async fn restore_if_local_mounted(sync_root: Option<&std::path::Path>, way_back: &WayBack) {
    let Some(root) = sync_root else {
        return;
    };
    if way_back.left.is_empty() || !mount_watch::fresh().await.is_mount_point(root) {
        return;
    }
    way_back.restore();
}

/// Where each browser a release moved off the device was before.
#[derive(Default)]
pub(super) struct WayBack {
    left: Vec<LeftBrowser>,
}

struct LeftBrowser {
    browser: Weak<Browser>,
    previous: Location,
    left_at: Option<Location>,
}

impl WayBack {
    /// Returns each browser to where it was, unless the user navigated it since.
    pub(super) fn restore(&self) {
        for left in &self.left {
            if let Some(browser) = left.browser.upgrade()
                && browser.active_location() == left.left_at
            {
                browser.navigate(left.previous.clone());
            }
        }
    }
}

/// Moves every tab and window currently inside `root` to Home before flushing or unmounting.
pub(super) fn leave_device_across_all(
    app: &mut App,
    root: &Location,
    mut tx: Option<&mut Tx>,
) -> WayBack {
    let mut browsers = collect_all_browsers(app);
    let home = Location::local(crate::launch::home_directory());
    let active_browser = Rc::clone(&app.browser);
    let mut left = Vec::new();
    for browser in browsers.drain(..) {
        let Some(previous) = browser.active_location() else {
            continue;
        };
        if !leaves(&previous, root) {
            continue;
        }
        if Rc::ptr_eq(&browser, &active_browser) {
            shell::publish(app);
            app.browser.navigate(home.clone());
            if let Some(tx) = tx.as_deref_mut() {
                shell::settle(app, tx);
            }
        } else {
            browser.navigate(home.clone());
        }
        let left_at = browser.active_location();
        if left_at.as_ref() != Some(&previous) {
            left.push(LeftBrowser {
                browser: Rc::downgrade(&browser),
                previous,
                left_at,
            });
        }
    }
    WayBack { left }
}

fn collect_all_browsers(app: &App) -> Vec<Rc<Browser>> {
    let mut browsers: Vec<Rc<Browser>> = Vec::new();
    let mut push = |browser: Rc<Browser>| {
        if !browsers.iter().any(|known| Rc::ptr_eq(known, &browser)) {
            browsers.push(browser);
        }
    };
    push(Rc::clone(&app.browser));
    for &key in shell::tabs::tabs(app) {
        if let Some(browser) = shell::tabs::tab_browser(app, key) {
            push(Rc::clone(browser));
        }
    }
    WATCHES.with(|watches| {
        for entry in watches.borrow().iter().filter_map(|(_, e)| e.upgrade()) {
            if let Some(browser) = entry.browser.upgrade() {
                push(browser);
            }
        }
    });
    browsers
}

/// The share to disconnect from the sidebar menu.
pub(super) struct ShareDisconnect {
    pub(super) id: String,
    pub(super) mount: gio::Mount,
    pub(super) root: Location,
}

/// Disconnects a GVfs network share (`src/ui/window/device_release/disconnect.rs`).
pub(super) fn disconnect_share(app: &mut App, share: ShareDisconnect, tx: &mut Tx) {
    let ShareDisconnect { id, mount, root } = share;
    let Some(guard) = DisconnectGuard::acquire(id) else {
        return;
    };
    let way_back = leave_device_across_all(app, &root, Some(tx));
    let window_later = shell::window_later(app).downgrade();
    glib::MainContext::default().spawn_local(async move {
        let bound = BoundOperation::new(window_later.clone(), OperationOptions::default());
        let gio_root = mount.root();
        let result = mount
            .unmount_with_operation_future(gio::MountUnmountFlags::NONE, Some(bound.operation()))
            .await;
        let outcome = bound.finish(result);
        drop(guard);
        if outcome.result.is_err() && share_still_mounted(&gio_root) {
            way_back.restore();
        }
        let error_message = match outcome.result {
            Err(error) if !release_error_is_quiet(&error) => Some(error.to_string()),
            _ => None,
        };
        refresh_all_sidebars();
        if let Some(later) = window_later.upgrade() {
            later.then(move |app: &mut App, _: &mut Tx| {
                super::refresh(app);
                if let Some(message) = error_message {
                    error_dialog::show(app, Error::new(tr("Unable to disconnect"), message));
                }
                true
            });
        }
    });
}

fn share_still_mounted(root: &gio::File) -> bool {
    gio::VolumeMonitor::get()
        .mounts()
        .iter()
        .any(|mount| mount.root().equal(root))
}

#[cfg(test)]
pub(super) fn push_pending_for_test(key: ReleaseKey, kind: ReleaseKind) {
    RELEASES.with(|releases| {
        releases.borrow_mut().push(PendingRelease {
            key,
            kind,
            overlay_id: mount_prompts::next_id(),
            overlay_shown: false,
            dismissed: false,
            progress_message: None,
            unplug: false,
            present_source: None,
        });
    });
}

#[cfg(test)]
pub(super) fn clear_pending_for_test() {
    RELEASES.with(|releases| {
        for mut entry in releases.borrow_mut().drain(..) {
            if let Some(source) = entry.present_source.take() {
                source.remove();
            }
        }
    });
}
