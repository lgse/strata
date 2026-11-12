// SPDX-License-Identifier: MIT

use super::*;

pub(in crate::adapters::local_operations) fn cannot_replace_safely(name: &str) -> String {
    rust_i18n::t!(
        "“%{name}” can’t be replaced safely at this location. Delete or rename it first, then try again.",
        name = name
    )
    .into_owned()
}

pub(in crate::adapters::local_operations) fn original_kept_aside(
    name: &str,
    kept: &OsStr,
    error: &str,
) -> String {
    rust_i18n::t!(
        "Could not replace “%{name}”; the original was kept as “%{kept}”: %{error}",
        name = name,
        kept = kept.to_string_lossy(),
        error = error
    )
    .into_owned()
}

pub(in crate::adapters::local_operations) fn original_not_removed(
    name: &str,
    kept: &OsStr,
    error: &glib::Error,
) -> String {
    rust_i18n::t!(
        "“%{name}” was replaced, but its original could not be removed from “%{kept}”: %{error}",
        name = name,
        kept = kept.to_string_lossy(),
        error = crate::services::gio_error_detail(error)
    )
    .into_owned()
}

pub(in crate::adapters::local_operations) fn could_not_replace(_name: &str, error: &str) -> String {
    rust_i18n::t!("Could not safely replace the item: %{error}", error = error).into_owned()
}

pub(in crate::adapters::local_operations) fn original_in_trash(error: &str) -> glib::Error {
    io_error(rust_i18n::t!(
        "Could not finish placing the replacement item; the original is in Trash: %{error}",
        error = error
    ))
}

pub(in crate::adapters::local_operations) fn displacement_without_trash(
    parent: &OwnedFd,
    directory: bool,
    name: &str,
) -> Result<Displacement, glib::Error> {
    displacement_or_refusal(
        placement::displacement_without_trash(fs(), parent.as_fd(), directory),
        name,
    )
}

pub(in crate::adapters::local_operations) fn displacement_or_refusal(
    displacement: Result<Option<Displacement>, PlaceError>,
    name: &str,
) -> Result<Displacement, glib::Error> {
    match displacement {
        Ok(Some(displacement)) => Ok(displacement),
        Ok(None) => Err(io_error(cannot_replace_safely(name))),
        Err(error) => Err(placement_failure(error, name, could_not_replace)),
    }
}

pub(in crate::adapters::local_operations) fn set_aside_failure(
    failure: RenameFailure,
    name: &str,
) -> glib::Error {
    match failure {
        RenameFailure::Place(PlaceError::Unsupported) => io_error(cannot_replace_safely(name)),
        RenameFailure::Place(PlaceError::Changed) => {
            translated_io_error("The target changed while the operation was in progress")
        }
        RenameFailure::Place(error) => placement_failure(error, name, could_not_replace),
        RenameFailure::Stranded { temporary, error } => io_error(original_kept_aside(
            name,
            &temporary,
            &crate::services::error_detail(placement_reason(error)),
        )),
    }
}
