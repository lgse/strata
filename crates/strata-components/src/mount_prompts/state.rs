//! The prompt the host keeps in its state: the request it shows, the host's ticket for it, and
//! the password form.

use rune::TextEdit;

use super::request::{
    Credentials, MountRequest, MountResponse, PasswordRequest, ProcessesRequest, PromptField,
    QuestionRequest, Remember,
};

/// One mount prompt at a time, with `T` the host's ticket for the request it shows: Strata's
/// is the GLib bridge's `Request`, answered in [`MountPrompts`](super::MountPrompts)'s
/// `on_answer`. Queueing a second operation's prompt is the host's.
pub struct MountPromptState<T> {
    active: Option<Active<T>>,
    /// Errors shown so far: each one is a new live node, so a second refusal is heard too.
    attempt: u64,
    /// What a field's edit handler reaches once its prompt has gone (an edit landing in the
    /// frame the prompt closed): secure, and wiped after each edit ([`Self::edited`]), so
    /// nothing typed into it is kept readable.
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

/// The password prompt's fields and choices. Its edits are public for the prompt's accessors;
/// the secret field's `TextEdit` is secure, and wipes its text when the prompt closes.
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
        // Strata's `focus_first`: an empty username, else the secret; else the dialog starts on
        // Connect (`MountPrompts`).
        if request.needs_user && request.default_user.is_empty() {
            form.user.focused = true;
        } else if request.needs_password {
            form.secret.focused = true;
        }
        form
    }

    /// Something is typed (a default username counts, as in Strata): the backdrop then doesn't
    /// dismiss the prompt.
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
    /// Shows `request`, answered through `ticket`. Returns the ticket of the prompt it
    /// replaced, which the host answers (Strata: `respond(MountResponse::Abort)`).
    ///
    /// A process list over a process list updates the prompt in place, focus and all: the
    /// backend sends `show-processes` again as programs close their files.
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

    /// Closes the prompt without answering, and returns its ticket: the request was withdrawn
    /// (GIO's `aborted`, or the operation sent a newer one).
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

    /// The password form's edit for `field`, or the spare once the prompt has gone. The field's
    /// change handler then calls [`edited`](Self::edited), which wipes what the edit put there.
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

    /// After a field's edit (Rune's `on_input_change`): wipes the spare, so an edit that
    /// reached it once its prompt had gone isn't kept.
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

    /// Closes the password prompt with what its form holds: Anonymous, or the credentials the
    /// request asked for. The secret's edit wipes as the prompt drops.
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
