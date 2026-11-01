// SPDX-License-Identifier: MIT

//! Registry probes and shortcuts the release tests drive.

use std::cell::Cell;

use super::{
    PendingRelease, RELEASES, ReleaseDecision, ReleaseKey, ReleaseKind, ReleasePhase, keys_match,
    overlay::body_for_progress,
    remove_release,
    run::{apply_decision, decide_flush, decide_gio},
    with_releases,
};

#[cfg(test)]
pub(in crate::ui::window) fn reset_releases() {
    RELEASES.with(|releases| releases.borrow_mut().clear());
}

#[cfg(test)]
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

#[cfg(test)]
pub(in crate::ui::window) fn release_is_dismissed(key: &ReleaseKey) -> bool {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .find(|entry| keys_match(&entry.key, key))
            .is_some_and(|entry| entry.dismissed)
    })
}

#[cfg(test)]
pub(in crate::ui::window) fn release_shows_spinner(key: &ReleaseKey) -> bool {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .any(|entry| keys_match(&entry.key, key) && entry.phase != ReleasePhase::FinishedOk)
    })
}

#[cfg(test)]
pub(in crate::ui::window) fn release_phase(key: &ReleaseKey) -> Option<ReleasePhase> {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .find(|entry| keys_match(&entry.key, key))
            .map(|entry| entry.phase)
    })
}

#[cfg(test)]
pub(in crate::ui::window) fn release_error_count(key: &ReleaseKey) -> usize {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .find(|entry| keys_match(&entry.key, key))
            .map(|entry| entry.error_count)
            .unwrap_or(0)
    })
}

#[cfg(test)]
pub(in crate::ui::window) fn release_body_text(key: &ReleaseKey) -> Option<String> {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .find(|entry| keys_match(&entry.key, key))
            .map(|entry| body_for_progress(entry.progress_message.as_deref()))
    })
}

#[cfg(test)]
pub(in crate::ui::window) fn release_overlay_presented(key: &ReleaseKey) -> bool {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .find(|entry| keys_match(&entry.key, key))
            .is_some_and(|entry| entry.overlay.is_some())
    })
}

#[cfg(test)]
pub(in crate::ui::window) fn clear_release_dismissed(key: &ReleaseKey) {
    with_releases(|releases| {
        if let Some(entry) = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))
        {
            entry.dismissed = false;
        }
    });
}

#[cfg(test)]
pub(in crate::ui::window) fn run_release_attempt(
    kind: ReleaseKind,
    progress: Option<&str>,
    flush: Result<(), String>,
    unplug: bool,
    gio_calls: &Cell<u32>,
    gio: impl FnOnce() -> Result<(), glib::Error>,
) -> ReleaseDecision {
    let decision = decide_flush(kind, flush);
    if !decision.call_gio {
        return decision;
    }
    gio_calls.set(gio_calls.get().saturating_add(1));
    decide_gio(kind, progress, gio(), unplug)
}

#[cfg(test)]
pub(in crate::ui::window) fn settle_pending_attempt(
    key: &ReleaseKey,
    flush: Result<(), String>,
    gio_calls: &Cell<u32>,
    gio: impl FnOnce() -> Result<(), glib::Error>,
) -> ReleaseDecision {
    let (kind, progress, unplug) = RELEASES.with(|releases| {
        let releases = releases.borrow();
        let entry = releases.iter().find(|entry| keys_match(&entry.key, key));
        (
            entry
                .map(|entry| entry.kind)
                .unwrap_or(ReleaseKind::Unmount),
            entry.and_then(|entry| entry.progress_message.clone()),
            entry.is_some_and(|entry| entry.unplug),
        )
    });
    let decision = run_release_attempt(kind, progress.as_deref(), flush, unplug, gio_calls, gio);
    if decision.clear_pending {
        remove_release(key);
    }
    decision
}

#[cfg(test)]
pub(in crate::ui::window) fn complete_release_success(key: &ReleaseKey) {
    let (kind, unplug) = RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .find(|entry| keys_match(&entry.key, key))
            .map(|entry| (entry.kind, entry.unplug))
            .unwrap_or((ReleaseKind::Unmount, false))
    });
    apply_decision(key, decide_gio(kind, None, Ok(()), unplug));
}
