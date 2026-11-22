// SPDX-License-Identifier: MIT

//! Safe GIO [`gio::MountOperation`] binding onto C26's mount prompts
//! (`src/ui/browser/location/mounting.rs` and `dialogs.rs`).

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gio::prelude::*;
use rune::Tx;
use strata_components::mount_prompts::{
    Choice, MountRequest, MountResponse, PasswordRequest, QuestionRequest, Remember, Secret,
};
use strata_core::i18n::{list, tr};

use super::{PromptTicket, next_id, open_prompt, withdraw_prompt};
use crate::{App, slots::Later};

/// Credentials submitted from a mount authentication prompt or carried into a retry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MountCredentials {
    pub(crate) anonymous: bool,
    pub(crate) username: String,
    pub(crate) domain: String,
    pub(crate) password: String,
    pub(crate) save: gio::PasswordSave,
}

impl MountCredentials {
    pub(crate) fn default_for_prompt(flags: gio::AskPasswordFlags) -> Self {
        Self {
            anonymous: false,
            username: glib::user_name().to_string_lossy().into_owned(),
            domain: if flags.contains(gio::AskPasswordFlags::NEED_DOMAIN) {
                "WORKGROUP".to_owned()
            } else {
                String::new()
            },
            password: String::new(),
            save: gio::PasswordSave::Never,
        }
    }

    pub(crate) fn from_response(response: &MountResponse) -> Option<Self> {
        match response {
            MountResponse::Anonymous => Some(Self {
                anonymous: true,
                username: String::new(),
                domain: String::new(),
                password: String::new(),
                save: gio::PasswordSave::Never,
            }),
            MountResponse::Password(credentials) => Some(Self {
                anonymous: false,
                username: credentials.user.clone().unwrap_or_default(),
                domain: credentials.domain.clone().unwrap_or_default(),
                password: credentials.password.clone().unwrap_or_default(),
                save: remember_to_gio(credentials.remember),
            }),
            MountResponse::Choice(_) | MountResponse::Abort => None,
        }
    }
}

/// What GIO last asked for in `ask-password`, preserved so authentication failures can reopen
/// the prompt with the same flags and previous username/domain.
#[derive(Clone, Debug)]
pub(crate) struct MountPromptDetails {
    pub(crate) message: String,
    pub(crate) default_user: String,
    pub(crate) default_domain: String,
    pub(crate) flags: gio::AskPasswordFlags,
    pub(crate) encrypted: bool,
}

impl MountPromptDetails {
    pub(crate) fn password_request(
        &self,
        retry: bool,
        previous: Option<&MountCredentials>,
    ) -> PasswordRequest {
        let secret = if self.encrypted || Secret::guess(&self.message) == Secret::Passphrase {
            Secret::Passphrase
        } else {
            Secret::Password
        };
        let (default_user, default_domain) = match previous {
            Some(prev) => (prev.username.clone(), prev.domain.clone()),
            None => (self.default_user.clone(), self.default_domain.clone()),
        };
        let has = |flag| self.flags.contains(flag);
        PasswordRequest {
            message: self.message.clone(),
            default_user,
            default_domain,
            needs_user: has(gio::AskPasswordFlags::NEED_USERNAME),
            needs_domain: has(gio::AskPasswordFlags::NEED_DOMAIN),
            needs_password: has(gio::AskPasswordFlags::NEED_PASSWORD),
            anonymous_allowed: has(gio::AskPasswordFlags::ANONYMOUS_SUPPORTED),
            saving_allowed: has(gio::AskPasswordFlags::SAVING_SUPPORTED),
            secret,
            error: retry.then(|| authentication_retry_message(self.flags, &self.message, secret)),
        }
    }
}

/// How a mount operation ended, with the credentials last tried and what the backend asked for.
pub(crate) struct MountOutcome {
    pub(crate) result: Result<(), glib::Error>,
    pub(crate) attempted: Option<MountCredentials>,
    pub(crate) details: Option<MountPromptDetails>,
}

/// Progress callback receiving GIO's `show-unmount-progress` text.
pub(crate) type ProgressHandler = Rc<dyn Fn(&str)>;

/// Pending prompt tickets owned by one [`BoundOperation`], each paired with its withdrawal flag.
type PendingPrompts = Rc<RefCell<Vec<(u64, Rc<Cell<bool>>)>>>;

/// Options for binding a [`gio::MountOperation`] to the window's mount prompts.
#[derive(Default)]
pub(crate) struct OperationOptions {
    pub(crate) encrypted: bool,
    pub(crate) supplied: Option<MountCredentials>,
    pub(crate) on_prompt_open: Option<Rc<dyn Fn()>>,
    pub(crate) on_submitted: Option<Rc<dyn Fn()>>,
    pub(crate) on_unmount_progress: Option<ProgressHandler>,
}

/// A bound [`gio::MountOperation`] whose signals are routed through [`super::MountPromptsPart`].
pub(crate) struct BoundOperation {
    operation: gio::MountOperation,
    later: Later<App>,
    pending: PendingPrompts,
    attempted: Rc<RefCell<Option<MountCredentials>>>,
    details: Rc<RefCell<Option<MountPromptDetails>>>,
}

impl BoundOperation {
    pub(crate) fn new(later: Later<App>, options: OperationOptions) -> Self {
        let operation = gio::MountOperation::new();
        let pending: PendingPrompts = Rc::default();
        let attempted = Rc::new(RefCell::new(options.supplied.clone()));
        let details = Rc::new(RefCell::new(None));

        connect_ask_password(
            &operation,
            later.clone(),
            PasswordState {
                encrypted: options.encrypted,
                supplied: RefCell::new(options.supplied),
                already_prompted: Cell::new(attempted.borrow().is_some()),
                pending: Rc::clone(&pending),
                attempted: Rc::clone(&attempted),
                details: Rc::clone(&details),
                on_prompt_open: options.on_prompt_open,
                on_submitted: options.on_submitted,
            },
        );
        connect_ask_question(&operation, later.clone(), Rc::clone(&pending));
        if let Some(on_progress) = options.on_unmount_progress {
            connect_unmount_progress(&operation, on_progress);
        }
        connect_aborted(&operation, later.clone(), Rc::clone(&pending));

        Self {
            operation,
            later,
            pending,
            attempted,
            details,
        }
    }

    pub(crate) fn operation(&self) -> &gio::MountOperation {
        &self.operation
    }

    pub(crate) fn finish(self, result: Result<(), glib::Error>) -> MountOutcome {
        withdraw_all(&self.later, &self.pending);
        MountOutcome {
            result,
            attempted: self.attempted.borrow().clone(),
            details: self.details.borrow().clone(),
        }
    }
}

struct PasswordState {
    encrypted: bool,
    supplied: RefCell<Option<MountCredentials>>,
    already_prompted: Cell<bool>,
    pending: PendingPrompts,
    attempted: Rc<RefCell<Option<MountCredentials>>>,
    details: Rc<RefCell<Option<MountPromptDetails>>>,
    on_prompt_open: Option<Rc<dyn Fn()>>,
    on_submitted: Option<Rc<dyn Fn()>>,
}

fn connect_ask_password(operation: &gio::MountOperation, later: Later<App>, state: PasswordState) {
    operation.connect_ask_password(move |op, message, default_user, default_domain, flags| {
        op.stop_signal_emission_by_name("ask-password");
        let info = MountPromptDetails {
            message: message.to_owned(),
            default_user: default_user.to_owned(),
            default_domain: default_domain.to_owned(),
            flags,
            encrypted: state.encrypted,
        };
        state.details.replace(Some(info.clone()));
        if let Some(credentials) = state.supplied.borrow_mut().take() {
            apply_credentials(op, &credentials);
            op.reply(gio::MountOperationResult::Handled);
            return;
        }
        if let Some(on_open) = state.on_prompt_open.as_ref() {
            on_open();
        }
        let retry = state.already_prompted.replace(true);
        let request = info.password_request(retry, None);
        let op = op.clone();
        let attempted = Rc::clone(&state.attempted);
        let on_submitted = state.on_submitted.clone();
        enqueue_prompt(
            &later,
            &state.pending,
            MountRequest::Password(request),
            move |response| {
                if let Some(credentials) = MountCredentials::from_response(&response) {
                    attempted.replace(Some(credentials.clone()));
                    if let Some(on_submitted) = on_submitted.as_ref() {
                        on_submitted();
                    }
                    apply_credentials(&op, &credentials);
                    op.reply(gio::MountOperationResult::Handled);
                } else {
                    op.reply(gio::MountOperationResult::Aborted);
                }
            },
        );
    });
}

fn connect_ask_question(
    operation: &gio::MountOperation,
    later: Later<App>,
    pending: PendingPrompts,
) {
    operation.connect_local("ask-question", false, move |values| {
        let op = values.first()?.get::<gio::MountOperation>().ok()?;
        let message = values.get(1)?.get::<String>().ok()?;
        let choices = values.get(2)?.get::<glib::StrV>().ok()?;
        op.stop_signal_emission_by_name("ask-question");
        if choices.is_empty() {
            op.reply(gio::MountOperationResult::Unhandled);
            return None;
        }
        let request = QuestionRequest {
            message,
            choices: choices
                .iter()
                .map(|choice| Choice::new(choice.as_str()))
                .collect(),
            host_key: false,
        };
        enqueue_prompt(
            &later,
            &pending,
            MountRequest::Question(request),
            move |response| match response {
                MountResponse::Choice(index) => {
                    op.set_choice(i32::try_from(index).unwrap_or(i32::MAX));
                    op.reply(gio::MountOperationResult::Handled);
                }
                _ => op.reply(gio::MountOperationResult::Aborted),
            },
        );
        None
    });
}

fn connect_unmount_progress(operation: &gio::MountOperation, on_progress: ProgressHandler) {
    operation.connect_show_unmount_progress(move |op, message, _time_left, _bytes_left| {
        op.stop_signal_emission_by_name("show-unmount-progress");
        on_progress(message);
    });
}

fn connect_aborted(operation: &gio::MountOperation, later: Later<App>, pending: PendingPrompts) {
    operation.connect_aborted(move |_| withdraw_all(&later, &pending));
}

fn enqueue_prompt(
    later: &Later<App>,
    pending: &PendingPrompts,
    request: MountRequest,
    reply: impl FnOnce(MountResponse) + 'static,
) {
    let id = next_id();
    let withdrawn = Rc::new(Cell::new(false));
    pending.borrow_mut().push((id, Rc::clone(&withdrawn)));
    let ticket = PromptTicket::new(id, withdrawn, reply);
    later.then(move |app: &mut App, _: &mut Tx| {
        open_prompt(app, request, ticket);
        true
    });
}

fn withdraw_all(later: &Later<App>, pending: &PendingPrompts) {
    let ids: Vec<u64> = pending
        .borrow_mut()
        .drain(..)
        .map(|(id, withdrawn)| {
            withdrawn.set(true);
            id
        })
        .collect();
    if ids.is_empty() {
        return;
    }
    later.then(move |app: &mut App, _: &mut Tx| {
        for id in ids {
            withdraw_prompt(app, id);
        }
        true
    });
}

/// Opens a standalone retry prompt (after GIO reported an authentication failure), returning the
/// chosen credentials to `on_submit` or calling `on_cancel`.
pub(crate) fn show_retry_prompt(
    app: &mut App,
    details: &MountPromptDetails,
    previous: Option<&MountCredentials>,
    on_submit: impl FnOnce(MountCredentials) + 'static,
    on_cancel: impl FnOnce() + 'static,
) {
    let retry = previous.is_some();
    let defaults = previous
        .cloned()
        .unwrap_or_else(|| MountCredentials::default_for_prompt(details.flags));
    let request = details.password_request(retry, Some(&defaults));
    let id = next_id();
    let withdrawn = Rc::new(Cell::new(false));
    let ticket = PromptTicket::new(id, withdrawn, move |response| {
        if let Some(credentials) = MountCredentials::from_response(&response) {
            on_submit(credentials);
        } else {
            on_cancel();
        }
    });
    open_prompt(app, MountRequest::Password(request), ticket);
}

/// Applies `credentials` onto a [`gio::MountOperation`] before replying `Handled`.
pub(crate) fn apply_credentials(operation: &gio::MountOperation, credentials: &MountCredentials) {
    operation.set_anonymous(credentials.anonymous);
    if credentials.anonymous {
        return;
    }
    operation.set_username(Some(&credentials.username));
    if !credentials.domain.is_empty() {
        operation.set_domain(Some(&credentials.domain));
    }
    operation.set_password(Some(&credentials.password));
    operation.set_password_save(credentials.save);
}

fn remember_to_gio(remember: Remember) -> gio::PasswordSave {
    match remember {
        Remember::Never => gio::PasswordSave::Never,
        Remember::ForSession => gio::PasswordSave::ForSession,
        Remember::Permanently => gio::PasswordSave::Permanently,
    }
}

/// Localized retry text matching GTK's `authentication_retry_message`.
pub(crate) fn authentication_retry_message(
    flags: gio::AskPasswordFlags,
    _message: &str,
    secret: Secret,
) -> String {
    let mut fields = Vec::new();
    if flags.contains(gio::AskPasswordFlags::NEED_USERNAME) {
        fields.push("username");
    }
    if flags.contains(gio::AskPasswordFlags::NEED_DOMAIN) {
        fields.push("domain");
    }
    if flags.contains(gio::AskPasswordFlags::NEED_PASSWORD) {
        fields.push(match secret {
            Secret::Passphrase => "passphrase",
            Secret::Password => "password",
        });
    }
    if fields.is_empty() {
        tr("That attempt wasn’t accepted. Try again.")
    } else {
        rust_i18n::t!(
            "Those credentials weren’t accepted. Check %{value1} and try again.",
            value1 = list(fields.into_iter().map(tr))
        )
        .into_owned()
    }
}
