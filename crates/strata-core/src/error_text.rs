// SPDX-License-Identifier: MIT

//! Localized text for I/O failures.

#[cfg(test)]
mod tests;

/// Localized text for an I/O failure. An unrecognized OS error gets a generic reason and its
/// system text is logged; errors built by Strata or a library keep their own message.
pub fn io_error_message(error: &std::io::Error) -> String {
    if let Some(link) = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<crate::storage::DestinationError>())
    {
        return link.message();
    }
    match io_error_reason(error) {
        Some(reason) => crate::i18n::tr(reason),
        None if error.raw_os_error().is_some() => {
            tracing::debug!(error = %system_error_text(error), "unrecognized I/O error");
            crate::i18n::tr(UNEXPECTED_SYSTEM_ERROR)
        }
        None => system_error_text(error),
    }
}

pub const UNEXPECTED_SYSTEM_ERROR: &str = "An unexpected system error occurred";

/// [`io_error_message`] for the `%{error}` slot after a colon; see [`error_detail`].
pub fn io_error_detail(error: &std::io::Error) -> String {
    error_detail(io_error_message(error))
}

/// The English catalog message for a recognized I/O failure.
pub fn io_error_reason(error: &std::io::Error) -> Option<&'static str> {
    use std::io::ErrorKind;
    // These errnos have no stable `ErrorKind`.
    match error.raw_os_error() {
        Some(libc::ELOOP) => return Some("There are too many levels of symbolic links"),
        Some(libc::EIO) => return Some("A device input/output error occurred"),
        Some(libc::ENODEV | libc::ENXIO) => return Some("The device is not available"),
        _ => {}
    }
    Some(match error.kind() {
        ErrorKind::NotFound => "No such file or folder",
        ErrorKind::PermissionDenied => "Permission denied",
        ErrorKind::NotADirectory => "Not a folder",
        ErrorKind::StorageFull => "There is not enough space on the device",
        ErrorKind::ReadOnlyFilesystem => "The file system is read-only",
        ErrorKind::TimedOut => "The operation timed out",
        ErrorKind::AlreadyExists => "An item with that name already exists",
        ErrorKind::DirectoryNotEmpty => "The folder is not empty",
        ErrorKind::IsADirectory => "The item is a folder",
        ErrorKind::FileTooLarge => "The file is too large for this file system",
        ErrorKind::QuotaExceeded => "The disk quota has been exceeded",
        ErrorKind::ResourceBusy => "The item is in use",
        ErrorKind::InvalidFilename => "The name is too long",
        ErrorKind::UnexpectedEof => "The file is truncated or damaged",
        ErrorKind::ConnectionRefused => "The connection was refused",
        ErrorKind::ConnectionReset => "The connection was reset",
        ErrorKind::ConnectionAborted => "The connection was interrupted",
        ErrorKind::HostUnreachable => "The server cannot be reached",
        ErrorKind::NetworkUnreachable => "The network is unreachable",
        ErrorKind::NetworkDown => "The network is down",
        ErrorKind::NotConnected => "The location is no longer connected",
        ErrorKind::StaleNetworkFileHandle => "The network file handle is no longer valid",
        ErrorKind::ExecutableFileBusy => "The file is running and cannot be changed",
        ErrorKind::CrossesDevices => "The destination is on a different file system",
        _ => return None,
    })
}

/// System text without the errno suffix or a tempfile path, which can name internal staging files.
pub fn system_error_text(error: &std::io::Error) -> String {
    let text = error.to_string();
    let text = match error.get_ref() {
        Some(_) => text
            .split_once(" at path \"")
            .map_or(text.as_str(), |(head, _)| head),
        None => text.as_str(),
    };
    match text.rsplit_once(" (os error ") {
        Some((head, tail)) if tail.ends_with(')') => head.to_owned(),
        _ => text.to_owned(),
    }
}

/// Adapts a standalone, capitalized error reason for a `…: %{error}` slot. French, Spanish,
/// Italian, Portuguese, Hungarian, Russian and Vietnamese continue in lower case after a colon;
/// German capitalizes nouns, and English and CJK keep the text as is.
pub fn error_detail(text: String) -> String {
    error_detail_in(&rust_i18n::locale(), text)
}

fn error_detail_in(locale: &str, text: String) -> String {
    if !matches!(locale, "fr" | "es" | "it" | "pt-BR" | "hu" | "ru" | "vi") {
        return text;
    }
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        // A second capital marks an acronym such as "HTTP".
        (Some(first), second)
            if first.is_uppercase() && !second.is_some_and(char::is_uppercase) =>
        {
            first.to_lowercase().chain(text.chars().skip(1)).collect()
        }
        _ => text,
    }
}
