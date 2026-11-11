//! The prompts' words, in the interface language. English by default: Strata's.

use super::request::{PasswordRequest, PromptField, Secret};

/// What the prompts say. Strata fills it with its translations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountLabels {
    pub sign_in_title: String,
    pub passphrase_title: String,
    /// The password prompt's subtitle, heard as the dialog's description.
    pub sign_in_subtitle: String,
    pub connect: String,
    pub cancel: String,
    pub username: String,
    pub domain: String,
    pub password: String,
    pub passphrase: String,
    /// The anonymous choice's name and its two options.
    pub connect_as: String,
    pub registered: String,
    pub anonymous: String,
    /// The storage choice's name and its options, in [`Remember`](super::Remember)'s order.
    pub storage: String,
    pub never: String,
    pub session: String,
    pub forever: String,
    /// The secret field's show and hide toggle.
    pub show_password: String,
    pub hide_password: String,
    pub show_passphrase: String,
    pub hide_passphrase: String,
    /// The host-key prompt's title, subtitle and warning.
    pub host_key_title: String,
    pub host_key_subtitle: String,
    pub host_key_warning: String,
    /// The process list's name, the pid's prefix and each row's button.
    pub processes: String,
    pub pid: String,
    pub end_process: String,
    /// The close button's name.
    pub close: String,
}

impl Default for MountLabels {
    fn default() -> Self {
        let s = |t: &str| t.to_owned();
        MountLabels {
            sign_in_title: s("Authentication required"),
            passphrase_title: s("Passphrase required"),
            sign_in_subtitle: s("Authenticate to access this volume or location"),
            connect: s("Connect"),
            cancel: s("Cancel"),
            username: s("Username"),
            domain: s("Domain"),
            password: s("Password"),
            passphrase: s("Passphrase"),
            connect_as: s("Connect as"),
            registered: s("Registered user"),
            anonymous: s("Anonymous"),
            storage: s("Password storage"),
            never: s("Don't remember"),
            session: s("Until logout"),
            forever: s("Forever"),
            show_password: s("Show password"),
            hide_password: s("Hide password"),
            show_passphrase: s("Show passphrase"),
            hide_passphrase: s("Hide passphrase"),
            host_key_title: s("Verify server identity"),
            host_key_subtitle: s(
                "Only continue after verifying the SSH host key through a trusted source.",
            ),
            host_key_warning: s(
                "If the server did not provide a fingerprint, verify it outside Strata before \
                 continuing.",
            ),
            processes: s("Programs using the volume"),
            pid: s("PID"),
            end_process: s("End process"),
            close: s("Close dialog"),
        }
    }
}

impl MountLabels {
    /// The secret field's label.
    pub fn secret(&self, secret: Secret) -> &str {
        match secret {
            Secret::Password => &self.password,
            Secret::Passphrase => &self.passphrase,
        }
    }

    /// Strata's English retry message (`authentication_retry_message`): "Those credentials
    /// weren’t accepted. Check username, domain and password and try again.", naming only the
    /// fields `request` shows. A translating host words its own.
    pub fn retry_message(request: &PasswordRequest) -> String {
        let names: Vec<&str> = request
            .shown_fields()
            .into_iter()
            .map(|f| match (f, request.secret) {
                (PromptField::User, _) => "username",
                (PromptField::Domain, _) => "domain",
                (PromptField::Secret, Secret::Password) => "password",
                (PromptField::Secret, Secret::Passphrase) => "passphrase",
            })
            .collect();
        let list = match names.as_slice() {
            [] => return "That attempt wasn’t accepted. Try again.".to_owned(),
            [one] => (*one).to_owned(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        };
        format!("Those credentials weren’t accepted. Check {list} and try again.")
    }
}
