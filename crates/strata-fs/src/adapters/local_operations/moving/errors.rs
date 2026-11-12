// SPDX-License-Identifier: MIT

use super::*;

/// Standalone reason for a placement failure, for a `…: %{error}` slot.
pub(in crate::adapters::local_operations) fn placement_reason(error: PlaceError) -> String {
    match error {
        PlaceError::Exists => crate::i18n::tr("An item with that name already exists"),
        PlaceError::InvalidName => crate::i18n::tr(INVALID_NAME_REASON),
        PlaceError::IntoItself => crate::i18n::tr("Can\u{2019}t put a folder inside itself"),
        PlaceError::Changed => {
            crate::i18n::tr("The target changed while the operation was in progress")
        }
        PlaceError::Unconfirmed => crate::i18n::tr(UNCONFIRMED_REASON),
        PlaceError::CrossDevice => os_error_text(rustix::io::Errno::XDEV),
        PlaceError::Unsupported => crate::i18n::tr(UNSUPPORTED_REASON),
        PlaceError::Io(error) => os_error_text(error),
    }
}

pub(in crate::adapters::local_operations) fn placement_error_kind(
    error: PlaceError,
) -> gio::IOErrorEnum {
    match error {
        PlaceError::Exists => gio::IOErrorEnum::Exists,
        PlaceError::InvalidName => gio::IOErrorEnum::InvalidFilename,
        _ => gio::IOErrorEnum::Failed,
    }
}

/// A placement failure as a complete message about `name`, using `template`
/// (with `%{name}` and `%{error}`) unless the failure has its own sentence.
pub(in crate::adapters::local_operations) fn placement_failure(
    error: PlaceError,
    name: &str,
    template: fn(&str, &str) -> String,
) -> glib::Error {
    let message = match error {
        PlaceError::IntoItself => placement_reason(error),
        PlaceError::Changed => rust_i18n::t!(
            "%{name} changed while the operation was in progress",
            name = name
        )
        .into_owned(),
        error => template(
            name,
            &crate::services::error_detail(placement_reason(error)),
        ),
    };
    io_error(message)
}

pub(in crate::adapters::local_operations) fn could_not_move(name: &str, error: &str) -> String {
    rust_i18n::t!(
        "Could not move %{name}: %{error}",
        name = name,
        error = error
    )
    .into_owned()
}

pub(in crate::adapters::local_operations) fn rename_failure_error(
    failure: RenameFailure,
    source: &Path,
    target: &Path,
) -> glib::Error {
    let name = |path: &Path| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    match failure {
        RenameFailure::Place(PlaceError::Exists) => glib::Error::new(
            gio::IOErrorEnum::Exists,
            &rust_i18n::t!("“%{name}” already exists", name = name(target)),
        ),
        RenameFailure::Place(error) => placement_error(error),
        RenameFailure::Stranded { temporary, error } => io_error(rust_i18n::t!(
            "Could not rename “%{name}”; it was left as “%{temporary}”: %{error}",
            name = name(source),
            temporary = temporary.to_string_lossy(),
            error = crate::services::error_detail(placement_reason(error))
        )),
    }
}

pub(in crate::adapters::local_operations) fn still_open_rename(
    from: Entry<'_>,
    error: PlaceError,
) -> Option<String> {
    match error {
        PlaceError::Io(errno) => holders::refused_rename(fs(), from, errno),
        _ => None,
    }
}
