// SPDX-License-Identifier: MIT

//! The update row's status wording for the installed build and a check's result.

use gtk::glib;

use crate::services::{BuildKind, InstallSource, UpdateCheck, UpdateMethod, Version};

pub(super) fn shows_available_release_notes(result: &UpdateCheck) -> bool {
    matches!(result, UpdateCheck::Available { .. })
}

/// The installed build's identity, for the update row's idle status line:
/// just the version for a stable build, or `Version {version} · {label}`
/// when running a prerelease -- so a user on an RC or nightly always sees
/// what they currently have installed, not just a bare version number.
pub(super) fn installed_version_status(
    version: &Version,
    kind: BuildKind,
    update_method: UpdateMethod,
) -> String {
    let version = if kind == BuildKind::Stable {
        rust_i18n::t!("Version %{version}", version = version).into_owned()
    } else {
        rust_i18n::t!(
            "Version %{version} · %{value1}",
            version = version,
            value1 = kind.localized_label()
        )
        .into_owned()
    };
    match update_method {
        UpdateMethod::InPlace => version,
        UpdateMethod::Aur => match InstallSource::detect().managed() {
            Some(managed) => rust_i18n::t!(
                "%{version} · Managed by %{manager}",
                version = version,
                manager = managed.manager()
            ),
            None => rust_i18n::t!(
                "%{version} · Managed by a package manager",
                version = version
            ),
        }
        .into_owned(),
        UpdateMethod::Omarchy => {
            rust_i18n::t!("%{version} · Managed by Omarchy", version = version).into_owned()
        }
        UpdateMethod::Pacman => {
            rust_i18n::t!("%{version} · Managed by pacman", version = version).into_owned()
        }
    }
}

pub(super) fn update_status_markup(
    message: String,
    result: &UpdateCheck,
    source: &InstallSource,
) -> String {
    match (source.managed(), result) {
        (Some(managed), UpdateCheck::Available { .. }) => format!(
            "{message}\n{}",
            glib::markup_escape_text(&managed.update_instruction())
        ),
        _ => message,
    }
}

pub(super) fn update_check_message(result: &UpdateCheck, update_method: UpdateMethod) -> String {
    match result {
        UpdateCheck::UpToDate => {
            rust_i18n::t!("Up to date — version %{value1}", value1 = crate::build_info::installed_version()).into_owned()
        }
        UpdateCheck::Available { release, .. } => {
            let url = glib::markup_escape_text(&release.url);
            let version = glib::markup_escape_text(&release.version);
            match update_method {
                UpdateMethod::InPlace | UpdateMethod::Aur => rust_i18n::t!("Update available: <a href=\"%{value1}\">v%{value2}</a>", value1 = url, value2 = version),
                UpdateMethod::Omarchy => rust_i18n::t!("Update available: <a href=\"%{value1}\">v%{value2}</a> · Run “omarchy update” to install", value1 = url, value2 = version),
                UpdateMethod::Pacman => rust_i18n::t!("Update available: <a href=\"%{value1}\">v%{value2}</a> · Install through a full system update", value1 = url, value2 = version),
            }
            .into_owned()
        }
        UpdateCheck::Failed(message) => rust_i18n::t!("Couldn't check for updates: %{value1} · <a href=\"https://github.com/lgse/strata/releases/latest\">View releases on GitHub</a>", value1 = glib::markup_escape_text(message)).into_owned(),
    }
}
