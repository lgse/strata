// SPDX-License-Identifier: MIT

//! Running a release: flush, leave the device, call GIO, then settle.

#[cfg(test)]
use std::cell::{Cell, RefCell};
use std::{io, path::PathBuf, rc::Rc, time::Duration};

use crate::{
    app::Browser,
    model::Location,
    ui::{
        browser::{BrowserView, show_error_dialog},
        modal::dismiss_modal_layer,
    },
};

use super::{
    super::device_actions::navigate_home_if_within,
    PendingRelease, ReleaseDecision, ReleaseKey, ReleaseKind, ReleasePhase, enter_releasing,
    inspect_release, keys_match,
    overlay::{ReleaseOverlay, mount_operation, schedule_present, show_safe_to_remove},
    refresh_sidebars, release_is_pending, remove_release, sidebar_browsers, update_release,
    with_releases,
};

pub(in crate::ui::window) fn decide_flush(
    kind: ReleaseKind,
    flush: Result<(), impl ToString>,
) -> ReleaseDecision {
    match flush {
        Ok(()) => ReleaseDecision {
            call_gio: true,
            clear_pending: false,
            dialog_title: None,
            dialog_message: None,
            safe_to_remove: false,
        },
        Err(error) => ReleaseDecision {
            call_gio: false,
            clear_pending: true,
            dialog_title: Some(kind.error_title().to_owned()),
            dialog_message: Some(error.to_string()),
            safe_to_remove: false,
        },
    }
}

pub(in crate::ui::window) fn decide_gio(
    kind: ReleaseKind,
    progress: Option<&str>,
    result: Result<(), glib::Error>,
    unplug: bool,
) -> ReleaseDecision {
    match result {
        Ok(()) => ReleaseDecision {
            call_gio: false,
            clear_pending: true,
            dialog_title: None,
            dialog_message: None,
            safe_to_remove: kind.safe_to_remove() && unplug,
        },
        Err(error) if release_error_is_quiet(&error) => ReleaseDecision {
            call_gio: false,
            clear_pending: true,
            dialog_title: None,
            dialog_message: None,
            safe_to_remove: false,
        },
        Err(error) => {
            let mut message = crate::services::gio_device_error_message(&error);
            if let Some(progress) = progress.map(str::trim).filter(|line| !line.is_empty()) {
                message.push('\n');
                message.push_str(progress);
            }
            ReleaseDecision {
                call_gio: false,
                clear_pending: true,
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

pub(super) fn apply_decision(key: &ReleaseKey, decision: ReleaseDecision) {
    if let Some(settling) = take_for_decision(key, decision.safe_to_remove) {
        settling.settle_overlay(decision.safe_to_remove);
        if decision.clear_pending {
            report_failure(key, settling.parent.as_ref(), &decision);
        }
    }
    if decision.clear_pending {
        remove_release(key);
        refresh_sidebars();
    }
}

/// What a decided release still shows, taken out of the registry.
struct Settling {
    dismissed: bool,
    parent: Option<gtk::Widget>,
    overlay: Option<ReleaseOverlay>,
    display_name: String,
}

impl Settling {
    /// A safe release turns its overlay into the unplug notice unless the user
    /// hid it; any other outcome closes the overlay.
    fn settle_overlay(&self, safe_to_remove: bool) {
        let Some(overlay) = self.overlay.as_ref() else {
            return;
        };
        if !safe_to_remove {
            dismiss_modal_layer(
                &overlay.layer,
                &overlay.host_overlay,
                overlay.blurred_root.as_ref(),
            );
        } else if !self.dismissed {
            show_safe_to_remove(overlay, &self.display_name);
        }
    }
}

fn take_for_decision(key: &ReleaseKey, safe_to_remove: bool) -> Option<Settling> {
    with_releases(|releases| {
        let entry = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))?;
        if safe_to_remove {
            entry.phase = ReleasePhase::FinishedOk;
        }
        Some(Settling {
            dismissed: entry.dismissed,
            parent: entry.parent.clone(),
            overlay: entry.overlay.take(),
            display_name: entry.key.display_name.clone(),
        })
    })
}

fn report_failure(key: &ReleaseKey, parent: Option<&gtk::Widget>, decision: &ReleaseDecision) {
    let (Some(parent), Some(title), Some(message)) = (
        parent,
        decision.dialog_title.as_deref(),
        decision.dialog_message.as_deref(),
    ) else {
        return;
    };
    update_release(key, |entry| {
        entry.error_count = entry.error_count.saturating_add(1)
    });
    show_error_dialog(parent, &crate::i18n::tr(title), message);
}
/// The device a release frees, how, and which folder it flushes and leaves.
pub(in crate::ui::window) struct ReleaseTarget {
    pub(in crate::ui::window) key: ReleaseKey,
    pub(in crate::ui::window) kind: ReleaseKind,
    pub(in crate::ui::window) unplug: bool,
    pub(in crate::ui::window) sync_root: Option<PathBuf>,
    pub(in crate::ui::window) away_root: Option<gio::File>,
}

/// A release and the window it reports progress through.
pub(in crate::ui::window) struct ReleaseRequest<'a> {
    pub(in crate::ui::window) parent: &'a gtk::Widget,
    pub(in crate::ui::window) browser: Option<Rc<Browser>>,
    pub(in crate::ui::window) view: Option<&'a BrowserView>,
    pub(in crate::ui::window) target: ReleaseTarget,
}

pub(in crate::ui::window) fn begin_release<F, Fut>(
    request: ReleaseRequest<'_>,
    on_settled: impl FnOnce() + 'static,
    run_gio: F,
) -> bool
where
    F: FnOnce(gtk::MountOperation) -> Fut + 'static,
    Fut: Future<Output = Result<(), glib::Error>> + 'static,
{
    let ReleaseRequest {
        parent,
        browser,
        view,
        target,
    } = request;
    if release_is_pending(&target.key) {
        return false;
    }
    let activity =
        view.map(|view| view.begin_global_activity(crate::i18n::tr("Writing to device…")));
    if !register_release(&target, parent, activity) {
        return false;
    }
    let run = ReleaseRun {
        parent: parent.clone(),
        browser,
        target,
    };
    glib::timeout_add_local_once(Duration::ZERO, move || {
        refresh_sidebars();
        schedule_present(run.parent.clone(), run.target.key.clone());
        glib::MainContext::default().spawn_local(async move {
            // Leave before flushing to drop directory monitors.
            let way_back = run.leave_device();
            #[cfg(test)]
            note_release_before_flush(run.browser.as_ref());
            let _reads = release_reads(run.target.sync_root.clone()).await;
            let decision = run.flush_and_release(&way_back, run_gio).await;
            apply_decision(&run.target.key, decision);
            on_settled();
        });
    });
    true
}

fn register_release(
    target: &ReleaseTarget,
    parent: &gtk::Widget,
    activity: Option<crate::ui::browser::GlobalActivity>,
) -> bool {
    with_releases(|releases| {
        if releases
            .iter()
            .any(|entry| keys_match(&entry.key, &target.key))
        {
            return false;
        }
        releases.push(PendingRelease {
            key: target.key.clone(),
            kind: target.kind,
            phase: ReleasePhase::Flushing,
            dismissed: false,
            progress_message: None,
            error_count: 0,
            unplug: target.unplug,
            parent: Some(parent.clone()),
            overlay: None,
            present_source: None,
            activity,
        });
        true
    })
}

/// Ends Strata's own reads under the mount so the flush and unmount can proceed.
async fn release_reads(sync_root: Option<PathBuf>) -> crate::adapters::ActiveOperation {
    let operation = crate::adapters::ActiveOperation::begin([]);
    if let Some(root) = sync_root {
        operation
            .release_lexically(vec![root], &gio::Cancellable::new())
            .await;
    }
    operation
}

/// A started release, owned by its task until it settles.
struct ReleaseRun {
    parent: gtk::Widget,
    browser: Option<Rc<Browser>>,
    target: ReleaseTarget,
}

/// Where each browser a release moved off the device was before.
#[derive(Default)]
pub(super) struct WayBack {
    left: Vec<LeftBrowser>,
}

impl WayBack {
    /// Returns each browser to where it was, unless the user has navigated it since.
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

pub(super) fn leave_device_at(root: &gio::File, browser: Option<&Rc<Browser>>) -> WayBack {
    let mut browsers = sidebar_browsers();
    if let Some(browser) = browser
        && !browsers.iter().any(|known| Rc::ptr_eq(known, browser))
    {
        browsers.insert(0, Rc::clone(browser));
    }
    let left = browsers
        .into_iter()
        .filter_map(|browser| {
            let previous = browser.active_location()?;
            navigate_home_if_within(&browser, root);
            let left_at = browser.active_location();
            (left_at.as_ref() != Some(&previous)).then(|| LeftBrowser {
                browser: Rc::downgrade(&browser),
                previous,
                left_at,
            })
        })
        .collect();
    WayBack { left }
}

struct LeftBrowser {
    browser: std::rc::Weak<Browser>,
    previous: Location,
    left_at: Option<Location>,
}

impl ReleaseRun {
    fn leave_device(&self) -> WayBack {
        self.target
            .away_root
            .as_ref()
            .map_or_else(WayBack::default, |root| {
                leave_device_at(root, self.browser.as_ref())
            })
    }

    async fn flush_and_release<F, Fut>(&self, way_back: &WayBack, run_gio: F) -> ReleaseDecision
    where
        F: FnOnce(gtk::MountOperation) -> Fut,
        Fut: Future<Output = Result<(), glib::Error>>,
    {
        let kind = self.target.kind;
        let key = &self.target.key;
        if let Err(error) = flush_mount(self.target.sync_root.clone()).await {
            self.restore_if_mounted(way_back).await;
            return decide_flush(kind, Err(crate::services::io_error_message(&error)));
        }
        enter_releasing(key);
        let result = run_gio(mount_operation(&self.parent, key)).await;
        if result.is_err() {
            self.restore_if_mounted(way_back).await;
        }
        let (progress, unplug) =
            inspect_release(key, |entry| (entry.progress_message.clone(), entry.unplug))
                .unwrap_or((None, false));
        decide_gio(kind, progress.as_deref(), result, unplug)
    }

    /// A failed release that left the device mounted returns each browser to
    /// where it was, unless the user has navigated it since.
    async fn restore_if_mounted(&self, way_back: &WayBack) {
        let Some(root) = self.target.sync_root.as_deref() else {
            return;
        };
        if way_back.left.is_empty()
            || !crate::adapters::mount_watch::fresh()
                .await
                .is_mount_point(root)
        {
            return;
        }
        way_back.restore();
    }
}
pub(super) fn blocking_join_message(error: Box<dyn std::any::Any + Send>) -> String {
    error
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            error
                .downcast_ref::<&str>()
                .map(|message| (*message).to_owned())
        })
        .unwrap_or_else(|| "filesystem sync panicked".to_owned())
}

pub(super) async fn flush_mount(root: Option<PathBuf>) -> Result<(), io::Error> {
    let Some(root) = root else {
        return Ok(());
    };
    // Even opening a stalled mount must not block the UI thread.
    match gio::spawn_blocking(move || crate::adapters::flush_filesystem(&root)).await {
        Ok(result) => result,
        Err(error) => Err(io::Error::other(blocking_join_message(error))),
    }
}

#[cfg(test)]
thread_local! {
    static LOCATION_AT_FLUSH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static SYNCS_BEFORE_FLUSH: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
pub(super) fn note_release_before_flush(browser: Option<&Rc<Browser>>) {
    LOCATION_AT_FLUSH.with(|slot| {
        *slot.borrow_mut() = browser
            .and_then(|browser| browser.active_location())
            .and_then(|location| location.native_path().map(std::path::Path::to_path_buf));
    });
    SYNCS_BEFORE_FLUSH.with(|slot| slot.set(crate::adapters::sync_probe_len()));
}

#[cfg(test)]
pub(in crate::ui::window) fn location_at_flush_for_test() -> Option<PathBuf> {
    LOCATION_AT_FLUSH.with(|slot| slot.borrow().clone())
}

#[cfg(test)]
pub(in crate::ui::window) fn syncs_before_flush_for_test() -> usize {
    SYNCS_BEFORE_FLUSH.with(|slot| slot.get())
}
