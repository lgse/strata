// SPDX-License-Identifier: MIT

//! Running a release: flush, leave the device, call GIO, then settle.

#[cfg(test)]
use std::cell::{Cell, RefCell};
use std::{
    io,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use crate::{
    app::Browser,
    ui::{
        browser::{BrowserView, show_error_dialog},
        modal::dismiss_modal_layer,
    },
};

use super::{
    super::device_actions::navigate_home_if_within,
    PendingRelease, RELEASES, ReleaseDecision, ReleaseKey, ReleaseKind, ReleasePhase,
    enter_releasing, keys_match,
    overlay::{mount_operation, schedule_present, show_safe_to_remove},
    refresh_sidebars, release_is_pending, remove_release, with_releases,
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
            let mut message = crate::services::gio_error_message(&error);
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
    let taken = with_releases(|releases| {
        let entry = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))?;
        if decision.safe_to_remove {
            entry.phase = ReleasePhase::FinishedOk;
        }
        Some((
            entry.dismissed,
            entry.parent.clone(),
            entry.overlay.take(),
            entry.key.display_name.clone(),
        ))
    });
    if let Some((dismissed, parent, overlay, display_name)) = taken {
        if decision.safe_to_remove {
            if !dismissed && let Some(overlay) = overlay {
                show_safe_to_remove(&overlay, &display_name);
            }
        } else if let Some(overlay) = overlay {
            dismiss_modal_layer(
                &overlay.layer,
                &overlay.host_overlay,
                overlay.blurred_root.as_ref(),
            );
        }
        if decision.clear_pending
            && let (Some(parent), Some(title), Some(message)) = (
                parent.as_ref(),
                decision.dialog_title.as_deref(),
                decision.dialog_message.as_deref(),
            )
        {
            with_releases(|releases| {
                if let Some(entry) = releases
                    .iter_mut()
                    .find(|entry| keys_match(&entry.key, key))
                {
                    entry.error_count = entry.error_count.saturating_add(1);
                }
            });
            show_error_dialog(parent, &crate::i18n::tr(title), message);
        }
    }
    if decision.clear_pending {
        remove_release(key);
        refresh_sidebars();
    }
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

#[cfg(test)]
#[expect(
    clippy::too_many_arguments,
    reason = "keeps the positional form the release tests drive"
)]
pub(in crate::ui::window) fn start_release<F, Fut>(
    parent: &gtk::Widget,
    browser: Option<Rc<Browser>>,
    view: Option<&BrowserView>,
    key: ReleaseKey,
    kind: ReleaseKind,
    unplug: bool,
    sync_root: Option<PathBuf>,
    away_root: Option<gio::File>,
    on_settled: impl FnOnce() + 'static,
    run_gio: F,
) -> bool
where
    F: FnOnce(gtk::MountOperation) -> Fut + 'static,
    Fut: Future<Output = Result<(), glib::Error>> + 'static,
{
    let target = ReleaseTarget {
        key,
        kind,
        unplug,
        sync_root,
        away_root,
    };
    let request = ReleaseRequest {
        parent,
        browser,
        view,
        target,
    };
    begin_release(request, on_settled, run_gio)
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
        target:
            ReleaseTarget {
                key,
                kind,
                unplug,
                sync_root,
                away_root,
            },
    } = request;
    if release_is_pending(&key) {
        return false;
    }
    let activity =
        view.map(|view| view.begin_global_activity(crate::i18n::tr("Writing to device…")));
    let inserted = with_releases(|releases| {
        if releases.iter().any(|entry| keys_match(&entry.key, &key)) {
            return false;
        }
        releases.push(PendingRelease {
            key: key.clone(),
            kind,
            phase: ReleasePhase::Flushing,
            dismissed: false,
            progress_message: None,
            error_count: 0,
            unplug,
            parent: Some(parent.clone()),
            overlay: None,
            present_source: None,
            activity,
        });
        true
    });
    if !inserted {
        return false;
    }
    let parent = parent.clone();
    glib::timeout_add_local_once(Duration::ZERO, move || {
        refresh_sidebars();
        schedule_present(parent.clone(), key.clone());
        glib::MainContext::default().spawn_local(async move {
            // Leave before flushing to drop directory monitors.
            let previous = browser
                .as_ref()
                .and_then(|browser| browser.active_location());
            if let (Some(browser), Some(root)) = (browser.as_ref(), away_root.as_ref()) {
                navigate_home_if_within(browser, root);
            }
            let left_at = browser
                .as_ref()
                .and_then(|browser| browser.active_location());
            #[cfg(test)]
            note_release_before_flush(browser.as_ref());
            let operation = crate::adapters::ActiveOperation::begin([]);
            if let Some(root) = sync_root.clone() {
                operation
                    .release_lexically(vec![root], &gio::Cancellable::new())
                    .await;
            }
            let flush_root = sync_root.clone();
            let flush = flush_mount(sync_root).await;
            let decision = match flush {
                Err(error) => {
                    restore_previous_if_mounted(
                        browser.as_ref(),
                        previous.as_ref(),
                        left_at.as_ref(),
                        flush_root.as_deref(),
                    )
                    .await;
                    decide_flush(kind, Err(crate::services::io_error_message(&error)))
                }
                Ok(()) => {
                    enter_releasing(&key);
                    let operation = mount_operation(&parent, &key);
                    let result = run_gio(operation).await;
                    if result.is_err() {
                        restore_previous_if_mounted(
                            browser.as_ref(),
                            previous.as_ref(),
                            left_at.as_ref(),
                            flush_root.as_deref(),
                        )
                        .await;
                    }
                    let (progress, unplug) = RELEASES.with(|releases| {
                        let releases = releases.borrow();
                        let entry = releases.iter().find(|entry| keys_match(&entry.key, &key));
                        (
                            entry.and_then(|entry| entry.progress_message.clone()),
                            entry.is_some_and(|entry| entry.unplug),
                        )
                    });
                    decide_gio(kind, progress.as_deref(), result, unplug)
                }
            };
            apply_decision(&key, decision);
            on_settled();
        });
    });
    true
}

pub(super) async fn restore_previous_if_mounted(
    browser: Option<&Rc<Browser>>,
    previous: Option<&crate::model::Location>,
    left_at: Option<&crate::model::Location>,
    root: Option<&Path>,
) {
    let Some(root) = root else {
        return;
    };
    if crate::adapters::mount_watch::fresh()
        .await
        .is_mount_point(root)
        && let (Some(browser), Some(previous)) = (browser, previous)
        && browser.active_location().as_ref() == left_at
        && left_at != Some(previous)
    {
        browser.navigate(previous.clone());
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
            .and_then(|location| location.native_path().map(Path::to_path_buf));
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
