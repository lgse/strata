// SPDX-License-Identifier: MIT

use std::fmt;

/// One question from a mount operation.
#[derive(Clone, Debug, PartialEq)]
pub enum MountRequest {
    /// `ask-password`: sign-in, a key passphrase or a LUKS passphrase.
    Password(PasswordRequest),
    /// `ask-question`, which also carries an SSH host key to trust.
    Question(QuestionRequest),
    /// `show-processes`: what keeps a volume busy as it unmounts.
    Processes(ProcessesRequest),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PasswordRequest {
    /// The backend's message, shown above the fields.
    pub message: String,
    /// The username field's text when the prompt opens.
    pub default_user: String,
    /// The domain field's text when the prompt opens.
    pub default_domain: String,
    /// `NEED_USERNAME`.
    pub needs_user: bool,
    /// `NEED_DOMAIN`.
    pub needs_domain: bool,
    /// `NEED_PASSWORD`.
    pub needs_password: bool,
    /// `ANONYMOUS_SUPPORTED`: the "Connect as" choice.
    pub anonymous_allowed: bool,
    /// `SAVING_SUPPORTED`: the "Password storage" choice.
    pub saving_allowed: bool,
    pub secret: Secret,
    pub error: Option<String>,
}

/// What a password prompt's secret is called.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Secret {
    #[default]
    Password,
    /// A key's or an encrypted volume's passphrase: "Passphrase required", "Passphrase".
    Passphrase,
}

impl Secret {
    pub fn guess(message: &str) -> Secret {
        if message.to_lowercase().contains("passphrase") {
            Secret::Passphrase
        } else {
            Secret::Password
        }
    }
}

/// A field of the password prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PromptField {
    User,
    Domain,
    /// The password or passphrase.
    Secret,
}

impl PasswordRequest {
    /// The fields the prompt shows, top to bottom: what a retry message names.
    pub fn shown_fields(&self) -> Vec<PromptField> {
        [
            (self.needs_user, PromptField::User),
            (self.needs_domain, PromptField::Domain),
            (self.needs_password, PromptField::Secret),
        ]
        .into_iter()
        .filter_map(|(shown, f)| shown.then_some(f))
        .collect()
    }
}

/// A choice the backend offers, in its order. Answers name it by that index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub label: String,
    pub rejects: bool,
}

impl Choice {
    /// A choice marked by [`looks_like_rejection`], Strata's English heuristic.
    pub fn new(label: impl Into<String>) -> Choice {
        let label = label.into();
        let rejects = looks_like_rejection(&label);
        Choice { label, rejects }
    }
}

pub fn looks_like_rejection(label: &str) -> bool {
    let label = label.trim().to_lowercase();
    [
        "cancel",
        "reject",
        "deny",
        "disconnect",
        "abort",
        "do not",
        "don't",
        "no",
    ]
    .iter()
    .any(|word| label == *word || label.starts_with(&format!("{word} ")))
}

/// `ask-question`: the backend's message and its choices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuestionRequest {
    pub message: String,
    pub choices: Vec<Choice>,
    pub host_key: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessesRequest {
    /// The backend's message: its first line is the title, the rest the body.
    pub message: String,
    pub processes: Vec<BlockingProcess>,
    pub choices: Vec<Choice>,
}

/// A process keeping a volume busy. GIO gives only the pid; the host finds its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockingProcess {
    pub pid: u32,
    /// The application's or the command's name.
    pub name: String,
    /// Its command line, when the host found one.
    pub command: Option<String>,
}

/// A prompt's answer.
#[derive(Clone, PartialEq, Eq)]
pub enum MountResponse {
    /// Connect: clear `anonymous`, set the credentials, then reply `Handled`.
    Password(Credentials),
    /// Connect as Anonymous: set `anonymous`, then reply `Handled`.
    Anonymous,
    /// A choice, by its index in the request: set `choice`, then reply `Handled`.
    Choice(usize),
    /// Cancel, Escape, the close button or the backdrop: reply `Aborted`.
    Abort,
}

impl fmt::Debug for MountResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MountResponse::Password(c) => f.debug_tuple("Password").field(c).finish(),
            MountResponse::Anonymous => f.write_str("Anonymous"),
            MountResponse::Choice(i) => f.debug_tuple("Choice").field(i).finish(),
            MountResponse::Abort => f.write_str("Abort"),
        }
    }
}

/// What the password prompt submits. A field is `Some` only when the request asked for it.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub user: Option<String>,
    pub domain: Option<String>,
    pub password: Option<String>,
    /// `password-save`: [`Remember::Never`] unless the request allowed saving.
    pub remember: Remember,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("user", &self.user)
            .field("domain", &self.domain)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("remember", &self.remember)
            .finish()
    }
}

/// How long the password is kept (`GPasswordSave`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Remember {
    /// "Don't remember".
    #[default]
    Never,
    /// "Until logout".
    ForSession,
    /// "Forever".
    Permanently,
}

impl Remember {
    pub(crate) const ALL: [Remember; 3] =
        [Remember::Never, Remember::ForSession, Remember::Permanently];

    pub(crate) fn index(self) -> usize {
        match self {
            Remember::Never => 0,
            Remember::ForSession => 1,
            Remember::Permanently => 2,
        }
    }
}
