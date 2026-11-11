//! What a mount operation asks, and what the prompts answer: plain data, mapped one to one onto
//! `GMountOperation`'s signals and properties by the binding (in Strata).

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

/// `ask-password`: its message and defaults, and the flags as switches
/// (`GAskPasswordFlags`). A field shows only when its flag is set.
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
    /// What the secret field asks for: a password, or a passphrase (a key's or a LUKS
    /// volume's), which renames the title and the field.
    pub secret: Secret,
    /// Why the last attempt failed, shown and announced over the fields: the host decides that
    /// a request is a retry (Strata: a second `ask-password` in one operation) and words it,
    /// for example with [`MountLabels::retry_message`](super::MountLabels::retry_message).
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
    /// Strata's rule: a passphrase when the backend's message says "passphrase", as GVfs's SSH
    /// key and udisks2 LUKS prompts do. A host that knows the target is an encrypted volume
    /// sets [`Secret::Passphrase`] itself.
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
    /// Declines: Cancel, Cancel Login, Disconnect. A rejecting choice takes focus when the
    /// prompt opens, and the first one is the secondary button. The host marks it: GIO doesn't
    /// say, and labels come translated.
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

/// Strata's English heuristic for a declining choice (`is_rejection_choice`): the label is, or
/// starts with, one of "cancel", "reject", "deny", "disconnect", "abort", "do not", "don't",
/// "no", ignoring case.
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
    /// The backend's message: its first line is the title, the rest the body, as GTK shows it.
    /// A host-key question shows it whole under its own title. It is shown as given: taking
    /// credentials out of a URI in it (Strata's `sanitize_failure_message`) is the host's.
    pub message: String,
    pub choices: Vec<Choice>,
    /// An SSH host key to trust (Strata: a question while mounting an `sftp://` location):
    /// "Verify server identity", with a warning, in the danger tone.
    pub host_key: bool,
}

/// `show-processes`: the processes keeping a volume busy, and the choices. The backend sends
/// it again as the list changes; the prompt then updates in place.
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
    /// The password or passphrase. `Debug` never prints it. From the answer on it is the host's,
    /// a plain `String` that isn't zeroized on drop: the prompt's own secure field has wiped its
    /// copy, but this one lives as long as the host keeps it.
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
