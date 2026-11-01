// SPDX-License-Identifier: MIT

//! Registry probes and shortcuts the release tests drive.

use std::{cell::Cell, path::PathBuf, rc::Rc};

use crate::{app::Browser, ui::browser::BrowserView};

use super::{
    PendingRelease, RELEASES, ReleaseDecision, ReleaseKey, ReleaseKind, ReleasePhase,
    inspect_release, keys_match,
    overlay::body_for_progress,
    remove_release,
    run::{ReleaseRequest, ReleaseTarget, apply_decision, begin_release, decide_flush, decide_gio},
    update_release, with_releases,
};

pub(in crate::ui::window) fn reset_releases() {
    RELEASES.with(|releases| releases.borrow_mut().clear());
}

pub(in crate::ui::window) fn begin_pending(key: ReleaseKey, kind: ReleaseKind) -> bool {
    with_releases(|releases| {
        if releases.iter().any(|entry| keys_match(&entry.key, &key)) {
            return false;
        }
        releases.push(PendingRelease {
            key,
            kind,
            phase: ReleasePhase::Flushing,
            dismissed: false,
            progress_message: None,
            error_count: 0,
            unplug: true,
            parent: None,
            overlay: None,
            present_source: None,
            activity: None,
        });
        true
    })
}

pub(in crate::ui::window) fn release_is_dismissed(key: &ReleaseKey) -> bool {
    inspect_release(key, |entry| entry.dismissed).unwrap_or(false)
}

pub(in crate::ui::window) fn release_shows_spinner(key: &ReleaseKey) -> bool {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .any(|entry| keys_match(&entry.key, key) && entry.phase != ReleasePhase::FinishedOk)
    })
}

pub(in crate::ui::window) fn release_phase(key: &ReleaseKey) -> Option<ReleasePhase> {
    inspect_release(key, |entry| entry.phase)
}

pub(in crate::ui::window) fn release_error_count(key: &ReleaseKey) -> usize {
    inspect_release(key, |entry| entry.error_count).unwrap_or(0)
}

pub(in crate::ui::window) fn release_body_text(key: &ReleaseKey) -> Option<String> {
    inspect_release(key, |entry| {
        body_for_progress(entry.progress_message.as_deref())
    })
}

pub(in crate::ui::window) fn release_overlay_presented(key: &ReleaseKey) -> bool {
    inspect_release(key, |entry| entry.overlay.is_some()).unwrap_or(false)
}

pub(in crate::ui::window) fn clear_release_dismissed(key: &ReleaseKey) {
    update_release(key, |entry| entry.dismissed = false);
}

/// Runs the flush and GIO decisions for a pending release without its I/O.
pub(in crate::ui::window) fn settle_pending_attempt(
    key: &ReleaseKey,
    flush: Result<(), String>,
    gio_calls: &Cell<u32>,
    gio: impl FnOnce() -> Result<(), glib::Error>,
) -> ReleaseDecision {
    let (kind, progress, unplug) = inspect_release(key, |entry| {
        (entry.kind, entry.progress_message.clone(), entry.unplug)
    })
    .unwrap_or((ReleaseKind::Unmount, None, false));
    let mut decision = decide_flush(kind, flush);
    if decision.call_gio {
        gio_calls.set(gio_calls.get().saturating_add(1));
        decision = decide_gio(kind, progress.as_deref(), gio(), unplug);
    }
    if decision.clear_pending {
        remove_release(key);
    }
    decision
}

pub(in crate::ui::window) fn complete_release_success(key: &ReleaseKey) {
    let (kind, unplug) = inspect_release(key, |entry| (entry.kind, entry.unplug))
        .unwrap_or((ReleaseKind::Unmount, false));
    apply_decision(key, decide_gio(kind, None, Ok(()), unplug));
}

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
