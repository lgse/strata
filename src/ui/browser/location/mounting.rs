// SPDX-License-Identifier: MIT

use super::devices::{DeviceKeys, DeviceMatch};
use super::dialogs::{
    MountCredentials, MountDialogHandlers, MountPromptDetails, apply_mount_credentials,
    dismiss_authentication_prompt, show_authentication_dialog, show_trust_question_dialog,
};
use super::mount_errors::{
    MountStrategy, log_mount_finished, log_mount_started, mount_error_is_authentication_failure,
    mount_failure_message, mount_result_is_ok, trust_question_result,
};
use super::{ConnectOutcome, PendingConnect};
use crate::adapters::gio_file_for_location;
use crate::model::Location;
use crate::ui::browser::{BrowserView, ViewState};
use crate::ui::modal::show_error_dialog;
use crate::ui::window::gio_volume_is_encrypted;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

pub(super) enum MountTarget {
    Location(Location, MountStrategy),
    Volume(gio::Volume),
    Drive(gio::Drive),
}

pub(super) struct MountRequest {
    pub(super) target: MountTarget,
    pub(super) credentials: Option<MountCredentials>,
    pub(super) cancellable: Option<gio::Cancellable>,
}

/// How a mount ended, with the credentials last tried and what the backend asked for.
pub(super) struct MountOutcome {
    pub(super) result: Result<(), glib::Error>,
    pub(super) attempted: Option<MountCredentials>,
    pub(super) details: Option<MountPromptDetails>,
}

/// The unlock progress a device mount shows while it waits.
#[derive(Clone)]
struct UnlockChrome {
    state: Weak<ViewState>,
    name: Option<String>,
    keys: DeviceKeys,
    encrypted: bool,
}

impl UnlockChrome {
    fn for_target(state: &Rc<ViewState>, target: &MountTarget) -> Self {
        let (name, keys, encrypted) = match target {
            MountTarget::Volume(volume) => (
                Some(volume.name().to_string()),
                DeviceMatch::from_volume(volume).keys,
                gio_volume_is_encrypted(volume),
            ),
            MountTarget::Drive(drive) => (
                Some(drive.name().to_string()),
                DeviceMatch::from_drive(drive).keys,
                true,
            ),
            MountTarget::Location(_, _) => (None, DeviceKeys::new([], []), false),
        };
        Self {
            state: Rc::downgrade(state),
            name,
            keys,
            encrypted,
        }
    }

    fn dismiss(&self) {
        if let Some(state) = self.state.upgrade() {
            state.dismiss_unlock_progress(&self.keys);
        }
    }

    /// An encrypted device shows its unlocking progress again once credentials are submitted.
    fn resume(&self) {
        if let (Some(state), Some(name)) = (self.state.upgrade(), self.name.as_ref())
            && self.encrypted
        {
            state.present_unlock_progress(&self.keys, name);
        }
    }
}

/// The Strata prompts a mount operation shows instead of GTK's own.
#[derive(Clone)]
struct MountPrompts {
    overlay: gtk::Overlay,
    active_prompt: Rc<RefCell<Option<gtk::Box>>>,
    active_question: Rc<RefCell<Option<gtk::Box>>>,
    question_declined: Rc<Cell<bool>>,
    details: Rc<RefCell<Option<MountPromptDetails>>>,
    attempted: Rc<RefCell<Option<MountCredentials>>>,
}

fn replace_prompt(overlay: &gtk::Overlay, slot: &RefCell<Option<gtk::Box>>) {
    if let Some(previous) = slot.borrow_mut().take() {
        dismiss_authentication_prompt(overlay, &previous);
    }
}

fn question_arguments(values: &[glib::Value]) -> Option<(String, Vec<String>)> {
    let message = values.get(1).and_then(|value| value.get::<String>().ok())?;
    let choices = values
        .get(2)
        .and_then(|value| value.get::<glib::StrV>().ok())?;
    if choices.is_empty() {
        return None;
    }
    Some((message, choices.iter().map(ToString::to_string).collect()))
}

impl MountPrompts {
    fn new(overlay: &gtk::Overlay, credentials: Option<&MountCredentials>) -> Self {
        Self {
            overlay: overlay.clone(),
            active_prompt: Rc::new(RefCell::new(None)),
            active_question: Rc::new(RefCell::new(None)),
            question_declined: Rc::new(Cell::new(false)),
            details: Rc::new(RefCell::new(None)),
            attempted: Rc::new(RefCell::new(credentials.cloned())),
        }
    }

    /// SFTP host-key choices are rendered by Strata without answering automatically.
    fn connect_trust_questions(&self, operation: &gtk::MountOperation) {
        let prompts = self.clone();
        let question_operation = operation.clone();
        operation.connect_local("ask-question", false, move |values| {
            let (message, choices) = question_arguments(values)?;
            question_operation.stop_signal_emission_by_name("ask-question");
            replace_prompt(&prompts.overlay, &prompts.active_question);
            let prompt = show_trust_question_dialog(
                &prompts.overlay,
                question_operation.upcast_ref(),
                &message,
                &choices,
                prompts.question_declined.clone(),
            );
            prompts.active_question.replace(prompt);
            None
        });
    }

    fn record_attempt(&self, chrome: &UnlockChrome) -> Rc<dyn Fn(MountCredentials)> {
        let attempted = self.attempted.clone();
        let chrome = chrome.clone();
        Rc::new(move |credentials| {
            attempted.replace(Some(credentials));
            chrome.resume();
        })
    }

    fn connect_password_requests(
        &self,
        operation: &gtk::MountOperation,
        chrome: UnlockChrome,
        supplied: Option<MountCredentials>,
    ) {
        let prompts = self.clone();
        let already_prompted = Cell::new(supplied.is_some());
        let supplied = RefCell::new(supplied);
        operation.connect_ask_password(
            move |operation, message, default_user, default_domain, flags| {
                // Suppress GtkMountOperation's native password dialog: we reply
                // ourselves so it cannot race the Strata authentication prompt.
                operation.stop_signal_emission_by_name("ask-password");
                prompts.details.replace(Some(MountPromptDetails {
                    message: message.to_owned(),
                    default_user: default_user.to_owned(),
                    default_domain: default_domain.to_owned(),
                    flags,
                }));
                if let Some(credentials) = supplied.borrow_mut().take() {
                    apply_mount_credentials(operation.upcast_ref(), &credentials);
                    operation.reply(gio::MountOperationResult::Handled);
                    return;
                }
                chrome.dismiss();
                replace_prompt(&prompts.overlay, &prompts.active_prompt);
                let retry = already_prompted.replace(true);
                let prompt = show_authentication_dialog(
                    &prompts.overlay,
                    Some(operation.upcast_ref()),
                    message,
                    (default_user, default_domain),
                    flags,
                    retry,
                    MountDialogHandlers {
                        submitted: Some(prompts.record_attempt(&chrome)),
                        cancelled: None,
                    },
                );
                prompts.active_prompt.replace(prompt);
            },
        );
    }

    fn finish(&self, result: Result<(), glib::Error>) -> MountOutcome {
        replace_prompt(&self.overlay, &self.active_prompt);
        replace_prompt(&self.overlay, &self.active_question);
        MountOutcome {
            result,
            attempted: self.attempted.borrow().clone(),
            details: self.details.borrow().clone(),
        }
    }
}

async fn mount_location_target(
    location: &Location,
    strategy: MountStrategy,
    operation: &gtk::MountOperation,
) -> Result<(), glib::Error> {
    let file = gio_file_for_location(location);
    match strategy {
        MountStrategy::EnclosingVolume => {
            file.mount_enclosing_volume_future(gio::MountMountFlags::NONE, Some(operation))
                .await
        }
        MountStrategy::Mountable => file
            .mount_mountable_future(gio::MountMountFlags::NONE, Some(operation))
            .await
            .map(|_| ()),
    }
}

async fn run_mount(
    target: MountTarget,
    operation: &gtk::MountOperation,
    cancellable: Option<gio::Cancellable>,
) -> Result<(), glib::Error> {
    let mount = async {
        match target {
            MountTarget::Volume(volume) => {
                volume
                    .mount_future(gio::MountMountFlags::NONE, Some(operation))
                    .await
            }
            MountTarget::Drive(drive) => {
                drive
                    .start_future(gio::DriveStartFlags::NONE, Some(operation))
                    .await
            }
            MountTarget::Location(location, strategy) => {
                mount_location_target(&location, strategy, operation).await
            }
        }
    };
    match cancellable {
        Some(cancellable) => gio::CancellableFuture::new(mount, cancellable)
            .await
            .unwrap_or_else(|gio::Cancelled| {
                Err(glib::Error::new(
                    gio::IOErrorEnum::Cancelled,
                    "Mount cancelled",
                ))
            }),
        None => mount.await,
    }
}

/// Credentials last tried and what the backend asked for, carried into a retry prompt.
type PromptHistory = (Option<MountCredentials>, Option<MountPromptDetails>);

impl ViewState {
    pub(in crate::ui::browser) fn mount_then_navigate_with_credentials(
        self: &Rc<Self>,
        location: Location,
        strategy: MountStrategy,
        credentials: Option<MountCredentials>,
    ) {
        let connect = self.pending_connect.borrow().clone();
        let cancellable = connect.as_ref().map(|connect| connect.cancellable.clone());
        self.mount_target(
            MountRequest {
                target: MountTarget::Location(location.clone(), strategy),
                credentials,
                cancellable,
            },
            move |state, outcome| {
                state.finish_navigation_mount(connect.as_ref(), (&location, strategy), outcome);
            },
        );
    }

    fn finish_navigation_mount(
        self: &Rc<Self>,
        connect: Option<&Rc<PendingConnect>>,
        (location, strategy): (&Location, MountStrategy),
        outcome: MountOutcome,
    ) {
        if connect.is_some_and(|connect| !self.is_pending_connect(connect)) {
            return;
        }
        if mount_result_is_ok(&outcome.result) {
            if let Some(connect) = connect {
                self.settle_connect(connect, ConnectOutcome::Opened(location.clone()));
            }
            if location.is_administrator() {
                self.open_as_administrator(location.clone());
            } else {
                self.browser.navigate(location.clone());
            }
            self.location_stack.set_visible_child_name("breadcrumbs");
            self.browser.focus_active();
            return;
        }
        let MountOutcome {
            result,
            attempted,
            details,
        } = outcome;
        let Err(error) = result else {
            return;
        };
        if mount_error_is_authentication_failure(location, &error) {
            self.prompt_to_retry_navigation((location.clone(), strategy), (attempted, details));
        } else {
            self.report_navigation_mount_failure(connect, location, &error);
        }
    }

    fn report_navigation_mount_failure(
        &self,
        connect: Option<&Rc<PendingConnect>>,
        location: &Location,
        error: &glib::Error,
    ) {
        self.abandon_deferred_reveal();
        self.location_stack.set_visible_child_name("breadcrumbs");
        self.restore_location_text();
        let message = mount_failure_message(location, error);
        let reported = connect.is_some_and(|connect| {
            self.settle_connect(connect, ConnectOutcome::Failed(message.clone()))
        });
        if !reported && let Some(message) = message {
            show_error_dialog(
                &self.overlay,
                &crate::i18n::tr("Unable to connect"),
                &message,
            );
        }
    }

    pub(super) fn mount_then_descend(
        self: &Rc<Self>,
        parent_depth: usize,
        location: Location,
        strategy: MountStrategy,
    ) {
        self.mount_then_descend_with_credentials(parent_depth, (location, strategy), None);
    }

    fn mount_then_descend_with_credentials(
        self: &Rc<Self>,
        parent_depth: usize,
        (location, strategy): (Location, MountStrategy),
        credentials: Option<MountCredentials>,
    ) {
        self.mount_target(
            MountRequest {
                target: MountTarget::Location(location.clone(), strategy),
                credentials,
                cancellable: None,
            },
            move |state, outcome| {
                let MountOutcome {
                    result,
                    attempted,
                    details,
                } = outcome;
                if mount_result_is_ok(&result) {
                    state.browser.descend(parent_depth, location.clone());
                    return;
                }
                let Err(error) = result else {
                    return;
                };
                if mount_error_is_authentication_failure(&location, &error) {
                    state.prompt_to_retry_descend(
                        parent_depth,
                        (location.clone(), strategy),
                        (attempted, details),
                    );
                } else if let Some(message) = mount_failure_message(&location, &error) {
                    show_error_dialog(
                        &state.overlay,
                        &crate::i18n::tr("Unable to connect"),
                        &message,
                    );
                }
            },
        );
    }

    fn prompt_to_retry_navigation(
        self: &Rc<Self>,
        (location, strategy): (Location, MountStrategy),
        (previous_credentials, prompt_details): PromptHistory,
    ) {
        let weak = Rc::downgrade(self);
        let cancel_weak = weak.clone();
        let details = prompt_details.unwrap_or_else(|| MountPromptDetails::fallback(&location));
        self.show_mount_retry_prompt(
            previous_credentials,
            details,
            MountDialogHandlers {
                submitted: Some(Rc::new(move |credentials| {
                    if let Some(state) = weak.upgrade() {
                        state.mount_then_navigate_with_credentials(
                            location.clone(),
                            strategy,
                            Some(credentials),
                        );
                    }
                })),
                cancelled: Some(Rc::new(move || {
                    if let Some(state) = cancel_weak.upgrade() {
                        state.abandon_navigation_retry();
                    }
                })),
            },
        );
    }

    fn abandon_navigation_retry(&self) {
        self.abandon_deferred_reveal();
        self.location_stack.set_visible_child_name("breadcrumbs");
        self.restore_location_text();
        if !self.fail_pending_connect(None) {
            self.browser.focus_active();
        }
    }

    fn prompt_to_retry_descend(
        self: &Rc<Self>,
        parent_depth: usize,
        (location, strategy): (Location, MountStrategy),
        (previous_credentials, prompt_details): PromptHistory,
    ) {
        let weak = Rc::downgrade(self);
        let details = prompt_details.unwrap_or_else(|| MountPromptDetails::fallback(&location));
        self.show_mount_retry_prompt(
            previous_credentials,
            details,
            MountDialogHandlers {
                submitted: Some(Rc::new(move |credentials| {
                    if let Some(state) = weak.upgrade() {
                        state.mount_then_descend_with_credentials(
                            parent_depth,
                            (location.clone(), strategy),
                            Some(credentials),
                        );
                    }
                })),
                cancelled: Some(Rc::new(|| {})),
            },
        );
    }

    pub(super) fn show_unlock_retry_prompt(
        self: &Rc<Self>,
        keys: DeviceKeys,
        previous_credentials: Option<MountCredentials>,
        details: MountPromptDetails,
        retry: impl Fn(MountCredentials) + 'static,
    ) {
        let weak = Rc::downgrade(self);
        self.show_mount_retry_prompt(
            previous_credentials,
            details,
            MountDialogHandlers {
                submitted: Some(Rc::new(retry)),
                cancelled: Some(Rc::new(move || {
                    if let Some(state) = weak.upgrade() {
                        state.finish_unlock_slot(&keys);
                    }
                })),
            },
        );
    }

    fn show_mount_retry_prompt(
        &self,
        previous_credentials: Option<MountCredentials>,
        details: MountPromptDetails,
        handlers: MountDialogHandlers,
    ) {
        let authentication_failed = previous_credentials.is_some();
        let defaults = previous_credentials.unwrap_or_else(|| prompt_defaults(&details));
        let _prompt = show_authentication_dialog(
            &self.overlay,
            None,
            &details.message,
            (&defaults.username, &defaults.domain),
            details.flags,
            authentication_failed,
            handlers,
        );
    }

    pub(super) fn mount_target(
        self: &Rc<Self>,
        request: MountRequest,
        on_result: impl Fn(&Rc<Self>, MountOutcome) + 'static,
    ) {
        let Some(window) = self.overlay.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let MountRequest {
            target,
            credentials,
            cancellable,
        } = request;
        let log_location = match &target {
            MountTarget::Location(location, strategy) => {
                log_mount_started(location, *strategy);
                Some(location.clone())
            }
            _ => None,
        };
        let chrome = UnlockChrome::for_target(self, &target);
        if let Some(name) = chrome.name.as_deref() {
            self.schedule_device_mount_chrome(&chrome.keys, name, chrome.encrypted);
        }
        let activity = BrowserView {
            state: self.clone(),
        }
        .begin_global_activity(crate::i18n::tr("Connecting…"));
        // Keep GTK's native question handler for other schemes and devices.
        let sftp_question = matches!(
            &target,
            MountTarget::Location(location, _) if location.backend_name() == "sftp"
        );
        let operation = gtk::MountOperation::new(Some(&window));
        let prompts = MountPrompts::new(&self.overlay, credentials.as_ref());
        if sftp_question {
            prompts.connect_trust_questions(&operation);
        }
        prompts.connect_password_requests(&operation, chrome, credentials);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let _activity = activity;
            let result = run_mount(target, &operation, cancellable).await;
            let result = trust_question_result(result, prompts.question_declined.get());
            if let Some(location) = log_location.as_ref() {
                log_mount_finished(location, &result);
            }
            let outcome = prompts.finish(result);
            if let Some(state) = weak.upgrade() {
                on_result(&state, outcome);
            }
        });
    }
}

fn prompt_defaults(details: &MountPromptDetails) -> MountCredentials {
    let mut defaults = MountCredentials::default_for_prompt(details.flags);
    if !details.default_user.is_empty() {
        defaults.username.clone_from(&details.default_user);
    }
    if !details.default_domain.is_empty() {
        defaults.domain.clone_from(&details.default_domain);
    }
    defaults
}
