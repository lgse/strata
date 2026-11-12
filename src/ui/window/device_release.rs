// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

use gtk::glib;

mod disconnect;
mod keys;
mod overlay;
#[cfg(test)]
mod probes;
mod run;

pub(super) use disconnect::disconnect_share;
#[cfg(test)]
use disconnect::leave_share_and_unmount;
pub(super) use keys::{
    ids_for_drive, ids_for_mount, ids_for_volume, key_for_drive, key_for_mount, key_for_volume,
};
use overlay::ReleaseOverlay;
pub(super) use overlay::{mount_operation, pending_device_shell};
#[cfg(test)]
pub(super) use overlay::{note_unmount_progress, present_release_overlay};
#[cfg(test)]
pub(super) use probes::start_release;
#[cfg(test)]
pub(super) use probes::{
    begin_pending, clear_release_dismissed, complete_release_success, release_body_text,
    release_error_count, release_is_dismissed, release_overlay_presented, release_phase,
    release_shows_spinner, reset_releases, settle_pending_attempt,
};
pub(super) use run::{ReleaseRequest, ReleaseTarget, begin_release};
#[cfg(test)]
pub(super) use run::{location_at_flush_for_test, syncs_before_flush_for_test};

pub(super) const RELEASE_BODY: &str =
    "Writing data to the drive. Do not unplug it. You can hide this and keep working.";

pub(super) const RELEASE_ROW_TOOLTIP: &str = "Writing data to the drive. Do not unplug it.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReleaseKind {
    Unmount,
    Eject,
    Lock,
}

impl ReleaseKind {
    fn title(self) -> String {
        crate::i18n::tr(match self {
            Self::Unmount => "Unmounting",
            Self::Eject => "Ejecting",
            Self::Lock => "Locking volume",
        })
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Lock => crate::assets::icons::LOCK,
            Self::Unmount | Self::Eject => crate::assets::icons::EJECT,
        }
    }

    pub(super) fn error_title(self) -> &'static str {
        match self {
            Self::Unmount => super::media_release_error_title(super::MediaRelease::UnmountMount),
            Self::Eject => super::media_release_error_title(super::MediaRelease::EjectMount),
            Self::Lock => "Unable to lock device",
        }
    }

    fn safe_to_remove(self) -> bool {
        matches!(self, Self::Unmount | Self::Eject)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReleasePhase {
    Flushing,
    Releasing,
    FinishedOk,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DeviceIds {
    pub unix_device: Option<String>,
    pub uuid: Option<String>,
    pub mount_root: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReleaseKey {
    pub unix_device: Option<String>,
    pub uuid: Option<String>,
    pub mount_root: Option<PathBuf>,
    pub display_name: String,
}

impl ReleaseKey {
    fn ids(&self) -> DeviceIds {
        DeviceIds {
            unix_device: self.unix_device.clone(),
            uuid: self.uuid.clone(),
            mount_root: self.mount_root.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReleaseDecision {
    pub call_gio: bool,
    pub clear_pending: bool,
    pub dialog_title: Option<String>,
    pub dialog_message: Option<String>,
    pub safe_to_remove: bool,
}

struct PendingRelease {
    key: ReleaseKey,
    kind: ReleaseKind,
    phase: ReleasePhase,
    dismissed: bool,
    progress_message: Option<String>,
    error_count: usize,
    unplug: bool,
    parent: Option<gtk::Widget>,
    overlay: Option<ReleaseOverlay>,
    present_source: Option<glib::SourceId>,
    #[expect(
        dead_code,
        reason = "held so Drop clears the header spinner when the release finishes"
    )]
    activity: Option<crate::ui::browser::GlobalActivity>,
}

thread_local! {
    static RELEASES: RefCell<Vec<PendingRelease>> = const { RefCell::new(Vec::new()) };
    static REBUILDERS: RefCell<Vec<(u64, std::rc::Weak<RebuildHook>)>> =
        const { RefCell::new(Vec::new()) };
    static NEXT_WATCH: Cell<u64> = const { Cell::new(1) };
}

struct RebuildHook {
    rebuild: Box<dyn Fn()>,
    browser: Option<std::rc::Weak<crate::app::Browser>>,
}

pub(super) struct SidebarWatch {
    id: u64,
    _hook: Rc<RebuildHook>,
}

impl Drop for SidebarWatch {
    fn drop(&mut self) {
        REBUILDERS.with(|hooks| hooks.borrow_mut().retain(|(id, _)| *id != self.id));
    }
}

/// Rebuilds a sidebar while a release is pending; its `browser` leaves a
/// device that any window or tab releases.
pub(super) fn watch_sidebars(
    browser: Option<&Rc<crate::app::Browser>>,
    rebuild: impl Fn() + 'static,
) -> SidebarWatch {
    let id = NEXT_WATCH.with(|next| {
        let id = next.get();
        next.set(id.saturating_add(1));
        id
    });
    let hook = Rc::new(RebuildHook {
        rebuild: Box::new(rebuild),
        browser: browser.map(Rc::downgrade),
    });
    REBUILDERS.with(|hooks| hooks.borrow_mut().push((id, Rc::downgrade(&hook))));
    SidebarWatch { id, _hook: hook }
}

fn sidebar_browsers() -> Vec<Rc<crate::app::Browser>> {
    REBUILDERS.with(|hooks| {
        let mut browsers: Vec<Rc<crate::app::Browser>> = Vec::new();
        for hook in hooks.borrow().iter().filter_map(|(_, hook)| hook.upgrade()) {
            if let Some(browser) = hook.browser.as_ref().and_then(std::rc::Weak::upgrade)
                && !browsers.iter().any(|known| Rc::ptr_eq(known, &browser))
            {
                browsers.push(browser);
            }
        }
        browsers
    })
}

pub(crate) fn refresh_sidebars() {
    let hooks: Vec<Rc<RebuildHook>> = REBUILDERS.with(|hooks| {
        hooks
            .borrow()
            .iter()
            .filter_map(|(_, hook)| hook.upgrade())
            .collect()
    });
    for hook in &hooks {
        (hook.rebuild)();
    }
    for browser in sidebar_browsers() {
        if browser
            .active_location()
            .as_ref()
            .and_then(|loc| loc.uri_value())
            .is_some_and(|uri| uri.starts_with("computer:"))
        {
            browser.reload_active();
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

fn with_releases<T>(update: impl FnOnce(&mut Vec<PendingRelease>) -> T) -> T {
    RELEASES.with(|releases| update(&mut releases.borrow_mut()))
}

pub(super) fn any_pending() -> bool {
    RELEASES.with(|releases| !releases.borrow().is_empty())
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

pub(super) fn release_is_pending(key: &ReleaseKey) -> bool {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .any(|entry| keys_match(&entry.key, key))
    })
}

/// Reads the pending release that matches `key`, if any.
fn inspect_release<T>(key: &ReleaseKey, read: impl FnOnce(&PendingRelease) -> T) -> Option<T> {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .find(|entry| keys_match(&entry.key, key))
            .map(read)
    })
}

/// Changes the pending release that matches `key`, if any.
fn update_release(key: &ReleaseKey, update: impl FnOnce(&mut PendingRelease)) {
    with_releases(|releases| {
        if let Some(entry) = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))
        {
            update(entry);
        }
    });
}

pub(super) fn enter_releasing(key: &ReleaseKey) {
    update_release(key, |entry| entry.phase = ReleasePhase::Releasing);
}

fn remove_release(key: &ReleaseKey) -> Option<PendingRelease> {
    with_releases(|releases| {
        let index = releases
            .iter()
            .position(|entry| keys_match(&entry.key, key))?;
        let mut entry = releases.remove(index);
        if let Some(source) = entry.present_source.take() {
            source.remove();
        }
        Some(entry)
    })
}

#[cfg(test)]
mod tests;
