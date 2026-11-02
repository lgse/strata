// SPDX-License-Identifier: MIT

use crate::model::Location;
use crate::services::{backend_unavailable_message, sanitize_failure_message, system_error_text};
use gtk::{gio, glib};

#[derive(Clone, Copy)]
pub(in crate::ui::browser) enum MountStrategy {
    /// The location itself is accessible but sits on an unmounted volume.
    EnclosingVolume,
    /// The location is itself the mountable target (an SMB share, a
    /// "Connect to Server" bookmark, ...).
    Mountable,
}

impl MountStrategy {
    fn name(self) -> &'static str {
        match self {
            Self::EnclosingVolume => "enclosing-volume",
            Self::Mountable => "mountable",
        }
    }
}

pub(super) fn log_mount_started(location: &Location, strategy: MountStrategy) {
    tracing::info!(backend = %location.backend_name(), strategy = strategy.name(), "mount requested");
    tracing::debug!(location = %location.diagnostic_path(), "mount location");
}

pub(super) fn log_mount_finished(location: &Location, result: &Result<(), glib::Error>) {
    match result {
        Ok(()) => tracing::info!(backend = %location.backend_name(), "mount finished"),
        Err(error) if mount_error_is_cancelled(error) => {
            tracing::info!(backend = %location.backend_name(), "mount cancelled")
        }
        Err(error) => tracing::info!(
            backend = %location.backend_name(),
            error_domain = ?error.domain(),
            error_code = error.code(),
            "mount failed"
        ),
    }
}

pub(super) fn trust_question_result(
    result: Result<(), glib::Error>,
    user_declined: bool,
) -> Result<(), glib::Error> {
    if user_declined {
        Err(glib::Error::new(
            gio::IOErrorEnum::Cancelled,
            "Trust question dismissed",
        ))
    } else {
        result
    }
}

pub(super) fn mount_result_is_ok(result: &Result<(), glib::Error>) -> bool {
    match result {
        Ok(()) => true,
        Err(error) => error.matches(gio::IOErrorEnum::AlreadyMounted),
    }
}

pub(super) fn mount_error_is_authentication_failure(
    location: &Location,
    error: &glib::Error,
) -> bool {
    if location.uri_value().is_none() || location.is_administrator() {
        return false;
    }
    if mount_error_is_host_key_rejection(error) {
        return false;
    }
    error.matches(gio::IOErrorEnum::PermissionDenied)
        || crate::services::describes_refused_credentials(error.message())
}

fn mount_error_is_host_key_rejection(error: &glib::Error) -> bool {
    let message = error.message().to_ascii_lowercase();
    [
        "host key",
        "host identification",
        "fingerprint",
        "known_hosts",
    ]
    .iter()
    .any(|reason| message.contains(reason))
}

/// Decides what, if anything, to tell the user about a failed mount attempt.
/// A user-initiated cancel (the GTK credential dialog's Cancel button, or a
/// backend that already reported the failure to the operation itself) should
/// quietly return to the prior state rather than surface an alarming error,
/// per lgse/strata#20's "cancelling authentication returns to the prior
/// committed location" requirement.
pub(super) fn mount_failure_message(location: &Location, error: &glib::Error) -> Option<String> {
    if mount_error_is_cancelled(error) || mount_error_is_unauthorized(location, error) {
        return None;
    }
    if error.matches(gio::IOErrorEnum::NotSupported)
        && let Some(uri) = location.uri_value()
    {
        return Some(backend_unavailable_message(uri));
    }
    if mount_error_is_host_key_rejection(error) {
        return Some(crate::i18n::tr(
            "The remote computer’s host key could not be verified. Confirm its fingerprint with the server administrator before trying again.",
        ));
    }
    if let Some(message) = transport_failure_message(error) {
        return Some(message);
    }
    Some(if location.native_path().is_some() {
        sanitize_failure_message(&crate::services::gio_error_message(error))
    } else {
        crate::services::gio_backend_error_message(error)
    })
}

fn transport_failure_message(error: &glib::Error) -> Option<String> {
    let failure = TransportFailure::from_code(error)
        .or_else(|| TransportFailure::from_gvfs_text(error))?;
    Some(crate::i18n::tr(failure.guidance()))
}

#[derive(Clone, Copy)]
enum TransportFailure {
    HostNotFound,
    Refused,
    TimedOut,
    Unreachable,
    Lost,
}

/// GVfs' SFTP backend reports an SSH session that ends during login, as when the
/// server stops before a password reply arrives, as `G_IO_ERROR_FAILED` with its
/// own translation of this message.
const GVFS_LOST_CONNECTION: &str = "Connection failed";

/// GVfs' SMB backends fail a mount with `G_IO_ERROR_FAILED` and
/// "Failed to mount Windows share: %s" (or "Failed to retrieve share list from
/// server: %s") filled with `g_strerror` of the libsmbclient errno, so the
/// errno survives only as that trailing text. gvfsd usually shares the session
/// locale, so this process's `strerror` matches it; when it doesn't, glibc's
/// English wording covers a daemon left in the C locale.
const GVFS_ERRNO_FAILURES: [(i32, &str, TransportFailure); 4] = [
    (
        libc::ECONNREFUSED,
        "Connection refused",
        TransportFailure::Refused,
    ),
    (
        libc::ETIMEDOUT,
        "Connection timed out",
        TransportFailure::TimedOut,
    ),
    (
        libc::EHOSTUNREACH,
        "No route to host",
        TransportFailure::Unreachable,
    ),
    (
        libc::ENETUNREACH,
        "Network is unreachable",
        TransportFailure::Unreachable,
    ),
];

impl TransportFailure {
    fn from_code(error: &glib::Error) -> Option<Self> {
        use gio::IOErrorEnum;
        Some(match error.kind::<IOErrorEnum>()? {
            IOErrorEnum::HostNotFound => Self::HostNotFound,
            IOErrorEnum::ConnectionRefused => Self::Refused,
            IOErrorEnum::TimedOut => Self::TimedOut,
            IOErrorEnum::HostUnreachable | IOErrorEnum::NetworkUnreachable => Self::Unreachable,
            _ => return None,
        })
    }

    fn from_gvfs_text(error: &glib::Error) -> Option<Self> {
        if !error.matches(gio::IOErrorEnum::Failed) {
            return None;
        }
        let message = error.message();
        let lost = message == GVFS_LOST_CONNECTION
            || message == glib::dgettext(Some("gvfs"), GVFS_LOST_CONNECTION);
        if lost {
            return Some(Self::Lost);
        }
        Self::from_gvfs_errno_text(message)
    }

    fn from_gvfs_errno_text(message: &str) -> Option<Self> {
        let (_, reason) = message.rsplit_once([':', '：'])?;
        let reason = reason.trim();
        GVFS_ERRNO_FAILURES
            .iter()
            .find(|(errno, english, _)| {
                reason == *english
                    || reason == system_error_text(&std::io::Error::from_raw_os_error(*errno))
            })
            .map(|&(_, _, failure)| failure)
    }

    fn guidance(self) -> &'static str {
        match self {
            Self::HostNotFound => {
                "That host couldn’t be found. Check the address and DNS settings."
            }
            Self::Refused => "The host refused the connection. Check the service and port.",
            Self::TimedOut => "The connection timed out. Check the host and firewall.",
            Self::Unreachable => "That host is unreachable. Check your network connection.",
            Self::Lost => {
                "The connection to the host failed. Check that the server is running, then try again."
            }
        }
    }
}

pub(super) fn mount_error_is_cancelled(error: &glib::Error) -> bool {
    error.matches(gio::IOErrorEnum::Cancelled) || error.matches(gio::IOErrorEnum::FailedHandled)
}

/// GVfs reports a dismissed or refused polkit prompt for `admin://` as `PERMISSION_DENIED`.
fn mount_error_is_unauthorized(location: &Location, error: &glib::Error) -> bool {
    location.is_administrator() && error.matches(gio::IOErrorEnum::PermissionDenied)
}

pub(super) fn volume_error_is_authentication_failure(error: &glib::Error) -> bool {
    let message = error.message().to_ascii_lowercase();
    [
        "incorrect passphrase",
        "invalid passphrase",
        "no key available with this passphrase",
        "authentication failed",
    ]
    .iter()
    .any(|reason| message.contains(reason))
}

pub(super) fn device_volume_mount_is_ready(
    result: &Result<(), glib::Error>,
    mount_present: bool,
) -> bool {
    mount_present || mount_result_is_ok(result)
}

pub(super) fn volume_error_is_in_flight_mount(error: &glib::Error) -> bool {
    if error.matches(gio::IOErrorEnum::Pending) || error.matches(gio::IOErrorEnum::Busy) {
        return true;
    }
    let message = error.message().to_ascii_lowercase();
    ["already unlocking", "already in progress"]
        .iter()
        .any(|reason| message.contains(reason))
}
