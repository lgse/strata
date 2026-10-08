// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

use super::file_source::{io_error_reason, system_error_text};

/// A ureq failure reduced to language-neutral data, so it can be cached and
/// localized in whichever language is active when it is shown.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum NetworkError {
    Status {
        code: u16,
    },
    /// An English catalog message.
    Reason {
        message: String,
    },
    /// Library or system text that has no translation.
    Detail {
        text: String,
    },
}

impl NetworkError {
    pub(crate) fn from_ureq(error: &ureq::Error) -> Self {
        let reason = |message: &str| Self::Reason {
            message: message.to_owned(),
        };
        match error {
            ureq::Error::StatusCode(code) => Self::Status { code: *code },
            ureq::Error::Io(error) => Self::from_io(error),
            ureq::Error::Timeout(_) => reason("The operation timed out"),
            ureq::Error::HostNotFound => reason("Could not find the server"),
            ureq::Error::ConnectionFailed => reason("Could not connect to the server"),
            ureq::Error::Tls(_) | ureq::Error::Rustls(_) | ureq::Error::Pem(_) => {
                reason("Could not establish a secure connection to the server")
            }
            ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => {
                reason("The server redirected the request too many times")
            }
            other => Self::Detail {
                text: other.to_string(),
            },
        }
    }

    /// Reading a ureq body yields `io::Error`s that may wrap a `ureq::Error`.
    pub(crate) fn from_io(error: &std::io::Error) -> Self {
        if let Some(inner) = error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<ureq::Error>())
        {
            return Self::from_ureq(inner);
        }
        if let Some(message) = io_error_reason(error) {
            return Self::Reason {
                message: message.to_owned(),
            };
        }
        // std's resolver reports getaddrinfo failures as an uncategorized error.
        if error.raw_os_error().is_none()
            && error
                .to_string()
                .starts_with("failed to lookup address information")
        {
            return Self::Reason {
                message: "Could not find the server".to_owned(),
            };
        }
        Self::Detail {
            text: system_error_text(error),
        }
    }

    /// Standalone localized text.
    pub(crate) fn message(&self) -> String {
        match self {
            Self::Status { code } => {
                rust_i18n::t!("The server returned HTTP %{code}", code = code).into_owned()
            }
            Self::Reason { message } => crate::i18n::tr(message),
            Self::Detail { text } => text.clone(),
        }
    }

    /// [`Self::message`] for the `%{error}` slot after a colon.
    pub(crate) fn detail(&self) -> String {
        super::file_source::error_detail(self.message())
    }
}
