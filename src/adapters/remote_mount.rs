// SPDX-License-Identifier: MIT

//! Drives a GIO mount operation for remote locations. Every backend prompt is
//! routed to a [`MountPrompter`] that must answer it explicitly; unanswered
//! prompts are aborted, so trust decisions are never accepted by default.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gio::prelude::*;
use gtk::{gio, glib};

use super::gio_file_for_location;
use crate::{
    model::Location,
    services::remote::{
        MountQuestion, MountResolution, RemoteErrorContext, resolve_mount_result,
        scheme_is_supported,
    },
};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MountStrategy {
    /// The location itself is accessible but sits on an unmounted volume.
    EnclosingVolume,
    /// The location is itself the mountable target (an SMB share, a
    /// "Connect to Server" bookmark, ...).
    Mountable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MountCredentials {
    pub anonymous: bool,
    pub username: String,
    pub domain: String,
    pub password: String,
    pub save: gio::PasswordSave,
}

impl MountCredentials {
    #[cfg(test)]
    pub(crate) fn password(username: &str, password: &str) -> Self {
        Self {
            anonymous: false,
            username: username.to_owned(),
            domain: String::new(),
            password: password.to_owned(),
            save: gio::PasswordSave::Never,
        }
    }

    pub(crate) fn apply_to(&self, operation: &gio::MountOperation) {
        operation.set_anonymous(self.anonymous);
        if self.anonymous {
            return;
        }
        if !self.username.is_empty() {
            operation.set_username(Some(&self.username));
        }
        if !self.domain.is_empty() {
            operation.set_domain(Some(&self.domain));
        }
        operation.set_password(Some(&self.password));
        operation.set_password_save(self.save);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PasswordRequest {
    pub message: String,
    pub default_user: String,
    pub default_domain: String,
    pub flags: gio::AskPasswordFlags,
    /// A previous answer in this operation was rejected.
    pub retry: bool,
}

impl PasswordRequest {
    #[cfg(test)]
    pub(crate) fn is_passphrase(&self) -> bool {
        requests_passphrase(&self.message)
    }
}

/// A key or volume passphrase rather than an account password.
pub(crate) fn requests_passphrase(message: &str) -> bool {
    message.to_lowercase().contains("passphrase")
}

pub(crate) trait MountPrompter {
    fn ask_password(&self, request: PasswordRequest, reply: PasswordReply);
    fn ask_question(&self, question: MountQuestion, reply: QuestionReply);
}

#[derive(Debug, Eq, PartialEq)]
enum PasswordStep {
    UseSupplied(MountCredentials),
    Prompt { retry: bool },
}

#[derive(Default)]
struct PromptSession {
    supplied: Option<MountCredentials>,
    prompted: bool,
    attempted: Option<MountCredentials>,
    last_request: Option<PasswordRequest>,
    declined: bool,
    question_open: bool,
}

impl PromptSession {
    fn new(supplied: Option<MountCredentials>) -> Self {
        Self {
            supplied,
            ..Self::default()
        }
    }

    /// Supplied credentials answer only the first prompt; any later prompt
    /// means they were rejected and asks the user again.
    fn next_password_step(&mut self) -> PasswordStep {
        if let Some(credentials) = self.supplied.take() {
            self.prompted = true;
            self.attempted = Some(credentials.clone());
            return PasswordStep::UseSupplied(credentials);
        }
        PasswordStep::Prompt {
            retry: std::mem::replace(&mut self.prompted, true),
        }
    }
}

/// Answers one `ask-password` emission. Dropping it unanswered aborts.
pub(crate) struct PasswordReply {
    operation: gio::MountOperation,
    session: Rc<RefCell<PromptSession>>,
    answered: Cell<bool>,
}

impl PasswordReply {
    pub(crate) fn submit(self, credentials: MountCredentials) {
        self.answered.set(true);
        credentials.apply_to(&self.operation);
        self.session.borrow_mut().attempted = Some(credentials);
        self.operation.reply(gio::MountOperationResult::Handled);
    }

    pub(crate) fn cancel(self) {
        self.answered.set(true);
        self.session.borrow_mut().declined = true;
        self.operation.reply(gio::MountOperationResult::Aborted);
    }
}

impl Drop for PasswordReply {
    fn drop(&mut self) {
        if !self.answered.get() {
            self.session.borrow_mut().declined = true;
            self.operation.reply(gio::MountOperationResult::Aborted);
        }
    }
}

/// Answers one `ask-question` or `show-processes` emission. Dropping it
/// unanswered aborts.
pub(crate) struct QuestionReply {
    operation: gio::MountOperation,
    session: Rc<RefCell<PromptSession>>,
    question: MountQuestion,
    answered: Cell<bool>,
}

impl QuestionReply {
    pub(crate) fn choose(self, index: usize) {
        self.answered.set(true);
        let mut session = self.session.borrow_mut();
        session.question_open = false;
        // Declining a trust or busy question is a cancellation, even when
        // the backend reports it as a failure.
        if self.question.is_risky_choice(0) && index != 0 {
            session.declined = true;
        }
        drop(session);
        self.operation.set_choice(index as i32);
        self.operation.reply(gio::MountOperationResult::Handled);
    }

    pub(crate) fn cancel(self) {
        self.answered.set(true);
        let mut session = self.session.borrow_mut();
        session.question_open = false;
        session.declined = true;
        drop(session);
        self.operation.reply(gio::MountOperationResult::Aborted);
    }
}

impl Drop for QuestionReply {
    fn drop(&mut self) {
        if !self.answered.get() {
            let mut session = self.session.borrow_mut();
            session.question_open = false;
            session.declined = true;
            drop(session);
            self.operation.reply(gio::MountOperationResult::Aborted);
        }
    }
}

/// A mount operation whose prompts are answered by a [`MountPrompter`].
pub(crate) struct MountSession {
    operation: gio::MountOperation,
    session: Rc<RefCell<PromptSession>>,
    scheme: RefCell<Option<String>>,
}

impl MountSession {
    pub(crate) fn new(
        operation: gio::MountOperation,
        supplied: Option<MountCredentials>,
        prompter: Rc<dyn MountPrompter>,
    ) -> Self {
        let session = Rc::new(RefCell::new(PromptSession::new(supplied)));
        connect_prompts(&operation, &session, prompter);
        Self {
            operation,
            session,
            scheme: RefCell::new(None),
        }
    }

    pub(crate) fn operation(&self) -> &gio::MountOperation {
        &self.operation
    }

    pub(crate) fn attempted_credentials(&self) -> Option<MountCredentials> {
        self.session.borrow().attempted.clone()
    }

    pub(crate) fn last_password_request(&self) -> Option<PasswordRequest> {
        self.session.borrow().last_request.clone()
    }

    pub(crate) fn declined(&self) -> bool {
        self.session.borrow().declined
    }

    pub(crate) fn resolve(
        &self,
        result: &Result<(), glib::Error>,
        context: RemoteErrorContext,
    ) -> MountResolution {
        let backend_available = self
            .scheme
            .borrow()
            .as_deref()
            .is_none_or(scheme_is_supported);
        resolve_mount_result(result, self.declined(), context, backend_available)
    }

    pub(crate) async fn mount(
        &self,
        location: &Location,
        strategy: MountStrategy,
    ) -> Result<(), glib::Error> {
        self.scheme.replace(
            location
                .uri_value()
                .and_then(glib::Uri::parse_scheme)
                .map(|scheme| scheme.to_string()),
        );
        let file = gio_file_for_location(location);
        match strategy {
            MountStrategy::EnclosingVolume => {
                file.mount_enclosing_volume_future(
                    gio::MountMountFlags::NONE,
                    Some(&self.operation),
                )
                .await
            }
            MountStrategy::Mountable => file
                .mount_mountable_future(gio::MountMountFlags::NONE, Some(&self.operation))
                .await
                .map(|_| ()),
        }
    }

    pub(crate) async fn unmount(&self, mount: &gio::Mount) -> Result<(), glib::Error> {
        mount
            .unmount_with_operation_future(gio::MountUnmountFlags::NONE, Some(&self.operation))
            .await
    }
}

fn connect_prompts(
    operation: &gio::MountOperation,
    session: &Rc<RefCell<PromptSession>>,
    prompter: Rc<dyn MountPrompter>,
) {
    let password_session = session.clone();
    let password_prompter = prompter.clone();
    operation.connect_ask_password(
        move |operation, message, default_user, default_domain, flags| {
            // Stop GTK's or GIO's default handler so only one reply is sent.
            operation.stop_signal_emission_by_name("ask-password");
            let step = password_session.borrow_mut().next_password_step();
            let retry = match step {
                PasswordStep::UseSupplied(credentials) => {
                    credentials.apply_to(operation);
                    operation.reply(gio::MountOperationResult::Handled);
                    return;
                }
                PasswordStep::Prompt { retry } => retry,
            };
            let request = PasswordRequest {
                message: message.to_owned(),
                default_user: default_user.to_owned(),
                default_domain: default_domain.to_owned(),
                flags,
                retry,
            };
            password_session.borrow_mut().last_request = Some(request.clone());
            password_prompter.ask_password(
                request,
                PasswordReply {
                    operation: operation.clone(),
                    session: password_session.clone(),
                    answered: Cell::new(false),
                },
            );
        },
    );

    // gio-rs doesn't generate bindings for the string-array signals.
    let question_session = session.clone();
    let question_prompter = prompter.clone();
    operation.connect_local("ask-question", false, move |values| {
        let (operation, message, choices) = question_arguments(values, 2)?;
        operation.stop_signal_emission_by_name("ask-question");
        let question = MountQuestion::classify(&message, &choices);
        ask_question(&operation, &question_session, &question_prompter, question);
        None
    });

    let processes_session = session.clone();
    operation.connect_local("show-processes", false, move |values| {
        let (operation, message, choices) = question_arguments(values, 3)?;
        operation.stop_signal_emission_by_name("show-processes");
        // GIO repeats this signal as the process list changes; keep the open prompt.
        if processes_session.borrow().question_open {
            return None;
        }
        let question = MountQuestion::busy(&message, &choices);
        ask_question(&operation, &processes_session, &prompter, question);
        None
    });
}

fn question_arguments(
    values: &[glib::Value],
    choices_index: usize,
) -> Option<(gio::MountOperation, String, Vec<String>)> {
    let operation = values.first()?.get::<gio::MountOperation>().ok()?;
    let message = values
        .get(1)?
        .get::<Option<String>>()
        .ok()?
        .unwrap_or_default();
    let choices = values
        .get(choices_index)?
        .get::<Vec<String>>()
        .unwrap_or_default();
    Some((operation, message, choices))
}

fn ask_question(
    operation: &gio::MountOperation,
    session: &Rc<RefCell<PromptSession>>,
    prompter: &Rc<dyn MountPrompter>,
    question: MountQuestion,
) {
    session.borrow_mut().question_open = true;
    prompter.ask_question(
        question.clone(),
        QuestionReply {
            operation: operation.clone(),
            session: session.clone(),
            question,
            answered: Cell::new(false),
        },
    );
}
