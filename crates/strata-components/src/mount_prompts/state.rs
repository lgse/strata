// SPDX-License-Identifier: MIT

use rune::TextEdit;

use super::request::{
    Credentials, MountRequest, MountResponse, PasswordRequest, ProcessesRequest, PromptField,
    QuestionRequest, Remember,
};

pub struct MountPromptState<T> {
    active: Option<Active<T>>,
    /// Errors shown so far: each one is a new live node, so a second refusal is heard too.
    attempt: u64,
    spare: TextEdit,
}

struct Active<T> {
    ticket: T,
    prompt: Prompt,
}

pub(crate) enum Prompt {
    Password(Box<PasswordPrompt>),
    Question(QuestionRequest),
    Processes(ProcessesRequest),
}

pub(crate) struct PasswordPrompt {
    pub request: PasswordRequest,
    pub form: PasswordForm,
}

#[derive(Debug)]
pub struct PasswordForm {
    pub user: TextEdit,
    pub domain: TextEdit,
    pub secret: TextEdit,
    /// "Connect as" Anonymous.
    pub anonymous: bool,
    pub remember: Remember,
    /// The secret shows as typed.
    pub revealed: bool,
}

impl PasswordForm {
    fn new(request: &PasswordRequest) -> PasswordForm {
        let mut form = PasswordForm {
            user: TextEdit::new(request.default_user.as_str()),
            domain: TextEdit::new(request.default_domain.as_str()),
            secret: TextEdit::new("").secure(),
            anonymous: false,
            remember: Remember::Never,
            revealed: false,
        };
        if request.needs_user && request.default_user.is_empty() {
            form.user.focused = true;
        } else if request.needs_password {
            form.secret.focused = true;
        }
        form
    }

    pub fn typed(&self) -> bool {
        [&self.user, &self.domain, &self.secret]
            .iter()
            .any(|e| !e.text.is_empty())
    }
}

impl<T> Default for MountPromptState<T> {
    fn default() -> Self {
        MountPromptState {
            active: None,
            attempt: 0,
            spare: TextEdit::new("").secure(),
        }
    }
}

impl<T> MountPromptState<T> {
    pub fn open(&mut self, request: MountRequest, ticket: T) -> Option<T> {
        let prompt = match request {
            MountRequest::Password(r) => {
                if r.error.is_some() {
                    self.attempt += 1;
                }
                Prompt::Password(Box::new(PasswordPrompt {
                    form: PasswordForm::new(&r),
                    request: r,
                }))
            }
            MountRequest::Question(q) => Prompt::Question(q),
            MountRequest::Processes(p) => Prompt::Processes(p),
        };
        let old = self.active.replace(Active { ticket, prompt });
        old.map(|a| a.ticket)
    }

    pub fn withdraw(&mut self) -> Option<T> {
        self.active.take().map(|a| a.ticket)
    }

    pub fn is_open(&self) -> bool {
        self.active.is_some()
    }

    /// The ticket of the request shown: Strata checks it for withdrawal each poll.
    pub fn ticket(&self) -> Option<&T> {
        self.active.as_ref().map(|a| &a.ticket)
    }

    /// The password prompt's form, while one shows.
    pub fn password_form(&self) -> Option<&PasswordForm> {
        match &self.active.as_ref()?.prompt {
            Prompt::Password(p) => Some(&p.form),
            _ => None,
        }
    }

    pub fn password_form_mut(&mut self) -> Option<&mut PasswordForm> {
        match &mut self.active.as_mut()?.prompt {
            Prompt::Password(p) => Some(&mut p.form),
            _ => None,
        }
    }

    pub(crate) fn edit_mut(&mut self, field: PromptField) -> &mut TextEdit {
        let Some(Active {
            prompt: Prompt::Password(p),
            ..
        }) = &mut self.active
        else {
            // Wiped before as well, for an edit path that runs no change handler.
            self.spare.clear();
            return &mut self.spare;
        };
        match field {
            PromptField::User => &mut p.form.user,
            PromptField::Domain => &mut p.form.domain,
            PromptField::Secret => &mut p.form.secret,
        }
    }

    pub(crate) fn edited(&mut self) {
        if !self.spare.text.is_empty() {
            self.spare.clear();
        }
    }

    #[cfg(test)]
    pub(crate) fn spare(&self) -> &TextEdit {
        &self.spare
    }

    pub(crate) fn prompt(&self) -> Option<&Prompt> {
        self.active.as_ref().map(|a| &a.prompt)
    }

    pub(crate) fn attempt(&self) -> u64 {
        self.attempt
    }

    /// Closes the prompt with `response`, for the host's `on_answer`.
    pub(crate) fn answer(&mut self, response: MountResponse) -> Option<(T, MountResponse)> {
        self.active.take().map(|a| (a.ticket, response))
    }

    pub(crate) fn submit(&mut self) -> Option<(T, MountResponse)> {
        let response = {
            let Prompt::Password(p) = &self.active.as_ref()?.prompt else {
                return None;
            };
            submission(&p.request, &p.form)
        };
        self.answer(response)
    }
}

/// What a password form submits.
pub(crate) fn submission(request: &PasswordRequest, form: &PasswordForm) -> MountResponse {
    if request.anonymous_allowed && form.anonymous {
        return MountResponse::Anonymous;
    }
    let take = |asked: bool, e: &TextEdit| asked.then(|| e.text.clone());
    MountResponse::Password(Credentials {
        user: take(request.needs_user, &form.user),
        domain: take(request.needs_domain, &form.domain),
        password: take(request.needs_password, &form.secret),
        remember: if request.saving_allowed {
            form.remember
        } else {
            Remember::Never
        },
    })
}
