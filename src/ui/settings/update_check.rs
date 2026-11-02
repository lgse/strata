// SPDX-License-Identifier: MIT

//! The process-wide update check: its schedule, the generation that discards
//! superseded results, and the sidebar notices fed by every completed check.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    sync::{OnceLock, mpsc::TryRecvError},
    time::{Duration, Instant},
};

use gtk::glib;

use super::{super::preferences::PreferenceManager, update_status};
use crate::services::{
    self, BuildKind, Channel, InstallRequest, InstallSource, ReleaseMetadata, UpdateCheck,
    UpdateMethod,
};

type AvailableUpdate = (ReleaseMetadata, InstallRequest, UpdateMethod);
pub(super) type CachedUpdate = Option<AvailableUpdate>;
pub(crate) type UpdateNoticeHandler = Rc<dyn Fn(CachedUpdate)>;
type WeakUpdateNoticeHandler = Weak<dyn Fn(CachedUpdate)>;

thread_local! {
    /// Shared by the due scheduler so every window uses one TTL.
    static LAST_COMPLETED_CHECK: Cell<Option<Instant>> = const { Cell::new(None) };
    static CHECK_IN_FLIGHT: Cell<bool> = const { Cell::new(false) };
    static CHECK_GENERATION: Cell<u64> = const { Cell::new(0) };
    static UPDATE_NOTICE_HANDLERS: RefCell<Vec<WeakUpdateNoticeHandler>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn next_check_generation() -> u64 {
    CHECK_IN_FLIGHT.set(true);
    let next = CHECK_GENERATION.get().saturating_add(1);
    CHECK_GENERATION.set(next);
    next
}

/// Detection spawns a package-manager child, so it resolves asynchronously
/// on first need, never during startup.
static UPDATE_METHOD_CACHE: OnceLock<UpdateMethod> = OnceLock::new();

/// Invokes `callback` on the GTK thread, detecting on a worker thread on a cache miss.
pub(crate) fn resolve_update_method_async(callback: impl FnOnce(UpdateMethod) + 'static) {
    if let Some(method) = UPDATE_METHOD_CACHE.get().copied() {
        callback(method);
        return;
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("strata-update-method".into())
        .spawn(move || {
            let method = *UPDATE_METHOD_CACHE.get_or_init(detect_update_method);
            let _sent = sender.send(method);
        });
    if spawned.is_err() {
        let method = *UPDATE_METHOD_CACHE.get_or_init(services::update_method);
        callback(method);
        return;
    }
    let mut callback = Some(callback);
    glib::timeout_add_local(Duration::from_millis(50), move || {
        match receiver.try_recv() {
            Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
            resolved => {
                let method = match resolved {
                    Ok(method) => method,
                    Err(TryRecvError::Disconnected) => {
                        *UPDATE_METHOD_CACHE.get_or_init(services::update_method)
                    }
                    Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
                };
                if let Some(callback) = callback.take() {
                    callback(method);
                }
                glib::ControlFlow::Break
            }
        }
    });
}

fn detect_update_method() -> UpdateMethod {
    if let Some(managed) = InstallSource::detect().managed() {
        managed.find_installed_aur_helper();
    }
    services::update_method()
}

const UPDATE_DUE_INTERVAL: Duration = Duration::from_secs(24 * 3600);

fn update_check_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|completed| now.duration_since(completed) >= UPDATE_DUE_INTERVAL)
}

fn force_due_update_check(last: Option<Instant>) -> bool {
    last.is_none()
}

pub(crate) fn maybe_run_due_update_check(manager: &Rc<PreferenceManager>) {
    if due_check_blocked(manager) {
        return;
    }
    let last_completed = LAST_COMPLETED_CHECK.get();
    if !update_check_due(last_completed, Instant::now()) {
        return;
    }
    let force = force_due_update_check(last_completed);
    let generation = next_check_generation();
    let channel = manager.release_channel();
    let weak_manager = Rc::downgrade(manager);
    resolve_update_method_async(move |method| {
        if is_superseded(generation) {
            return;
        }
        let receiver = services::check_for_updates(
            channel,
            crate::build_info::installed_version(),
            method,
            force,
        );
        let due = DueCheck {
            manager: weak_manager,
            channel,
            method,
            generation,
        };
        glib::timeout_add_local(Duration::from_millis(100), move || due.poll(&receiver));
    });
}

fn due_check_blocked(manager: &PreferenceManager) -> bool {
    !crate::build_info::updater_enabled() || !manager.checks_for_updates() || CHECK_IN_FLIGHT.get()
}

/// A background check started by the due scheduler, identified by its generation.
struct DueCheck {
    manager: Weak<PreferenceManager>,
    channel: Channel,
    method: UpdateMethod,
    generation: u64,
}

impl DueCheck {
    fn poll(&self, receiver: &std::sync::mpsc::Receiver<UpdateCheck>) -> glib::ControlFlow {
        if is_superseded(self.generation) {
            return glib::ControlFlow::Break;
        }
        let result = match receiver.try_recv() {
            Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => unexpected_failure(),
            Ok(result) => result,
        };
        complete_due_update_check(
            &self.manager,
            self.channel,
            result,
            self.method,
            self.generation,
        );
        glib::ControlFlow::Break
    }
}

fn unexpected_failure() -> UpdateCheck {
    UpdateCheck::Failed(crate::i18n::tr(crate::services::UNEXPECTED_SYSTEM_ERROR))
}

pub(super) fn complete_due_update_check(
    manager: &Weak<PreferenceManager>,
    channel: Channel,
    result: UpdateCheck,
    method: UpdateMethod,
    generation: u64,
) {
    if is_superseded(generation) {
        return;
    }
    CHECK_IN_FLIGHT.set(false);
    if !still_checking_channel(manager, channel) {
        return;
    }
    LAST_COMPLETED_CHECK.set(Some(Instant::now()));
    publish_update_check(effective_update_channel(channel, method), method, result);
}

fn still_checking_channel(manager: &Weak<PreferenceManager>, channel: Channel) -> bool {
    manager
        .upgrade()
        .is_some_and(|manager| manager.checks_for_updates() && manager.release_channel() == channel)
}

/// Records a check that a Settings row ran to completion and publishes its result.
pub(super) fn complete_row_check(channel: Channel, method: UpdateMethod, result: UpdateCheck) {
    CHECK_IN_FLIGHT.set(false);
    LAST_COMPLETED_CHECK.set(Some(Instant::now()));
    publish_update_check(channel, method, result);
}

/// Publishes the failure of a row's check whose worker went away without a result.
pub(super) fn abandon_row_check(channel: Channel, method: UpdateMethod) {
    CHECK_IN_FLIGHT.set(false);
    publish_update_check(channel, method, unexpected_failure());
}

fn publish_update_check(channel: Channel, method: UpdateMethod, result: UpdateCheck) {
    update_status::publish(channel, method, result);
    publish_update_notice(update_status::available());
}

pub(crate) fn register_update_notice(notice: &UpdateNoticeHandler) {
    UPDATE_NOTICE_HANDLERS.with(|handlers| {
        handlers.borrow_mut().push(Rc::downgrade(notice));
    });
    if let Some(result) = update_status::available() {
        notice(Some(result));
    }
}

pub(super) fn publish_update_notice(result: CachedUpdate) {
    let handlers = UPDATE_NOTICE_HANDLERS.with(|handlers| {
        let mut handlers = handlers.borrow_mut();
        handlers.retain(|notice| notice.strong_count() > 0);
        handlers
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>()
    });
    for notice in handlers {
        notice(result.clone());
    }
}

pub(crate) fn clear_cached_update_notice() {
    update_status::clear();
    publish_update_notice(None);
}

/// Whether a check's result -- issued under `result_generation` -- has been
/// superseded by a newer check, whose generation is `current_generation`.
///
/// A toggle mid-check must start a fresh check rather than being silently
/// dropped (see `run_check`'s doc comment), which means an older check's
/// result can still land after a newer one has already started or even
/// finished. Applying that stale result regardless is exactly how a Preview
/// fetch in flight when the user flips back to Stable could still offer an
/// RC to a Stable user: the result carries no channel of its own, so
/// nothing but generation order distinguishes it from a current one.
pub(super) fn is_stale_check(result_generation: u64, current_generation: u64) -> bool {
    result_generation != current_generation
}

/// Whether the check issued under `generation` has been superseded process-wide.
pub(super) fn is_superseded(generation: u64) -> bool {
    is_stale_check(generation, CHECK_GENERATION.get())
}

pub(super) fn effective_update_channel(selected: Channel, update_method: UpdateMethod) -> Channel {
    match update_method {
        UpdateMethod::InPlace | UpdateMethod::Aur => selected,
        UpdateMethod::Omarchy | UpdateMethod::Pacman => Channel::Stable,
    }
}

/// Whether an offer for a `kind` build may still be installed by a user now
/// on `channel`.
///
/// `is_stale_check` only covers a check whose *result* has not landed yet,
/// and only within the one row that started it. This covers the other half:
/// an offer that already landed and is sitting in a window's "Install
/// update" button or an open update dialog. The channel preference is
/// process-wide ([`PreferenceManager::shared`]), so switching back to Stable in
/// one window leaves every other window holding a cached RC offer it would
/// otherwise happily install. Re-testing at the moment of the click is what
/// makes the preference authoritative regardless of how many views cached
/// an offer under the old one.
pub(super) fn offer_still_eligible(channel: Channel, kind: BuildKind) -> bool {
    match channel {
        Channel::Stable => kind == BuildKind::Stable,
        Channel::Preview => kind != BuildKind::Nightly,
        Channel::Nightly => true,
    }
}
