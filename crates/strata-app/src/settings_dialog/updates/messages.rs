// SPDX-License-Identifier: MIT

//! Status wording, channel rules, and anchor parsing for the Updates page.

use strata_core::i18n::{join_sentences, tr};
use strata_desktop::services::{BuildKind, Channel, ManagedInstall, UpdateMethod, Version};

pub(super) const LATEST_RELEASES_URL: &str = "https://github.com/lgse/strata/releases/latest";

pub(super) fn effective_update_channel(selected: Channel, method: UpdateMethod) -> Channel {
    match method {
        UpdateMethod::InPlace | UpdateMethod::Aur => selected,
        UpdateMethod::Omarchy | UpdateMethod::Pacman => Channel::Stable,
    }
}

pub(super) fn offer_still_eligible(channel: Channel, kind: BuildKind) -> bool {
    match channel {
        Channel::Stable => kind == BuildKind::Stable,
        Channel::Preview => kind != BuildKind::Nightly,
        Channel::Nightly => true,
    }
}

pub(super) fn installed_version_status(
    version: &Version,
    kind: BuildKind,
    method: UpdateMethod,
    managed: Option<&ManagedInstall>,
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
    match method {
        UpdateMethod::InPlace => version,
        UpdateMethod::Aur => match managed {
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

pub(super) fn auto_update_description(method: UpdateMethod) -> &'static str {
    match method {
        UpdateMethod::InPlace => "Look for a new release on GitHub when Strata starts.",
        UpdateMethod::Aur => "Check the AUR for a newer packaged release when Strata starts.",
        UpdateMethod::Omarchy => {
            "Check the Omarchy package repository for a newer release when Strata starts."
        }
        UpdateMethod::Pacman => {
            "Check the configured package repositories for a newer release when Strata starts."
        }
    }
}

pub(super) fn channel_description(channel: Channel) -> &'static str {
    match channel {
        Channel::Stable => "Tested releases recommended for everyday use.",
        Channel::Preview => "Try upcoming releases with alpha, beta, and release-candidate builds.",
        Channel::Nightly => "Everything in Preview, plus daily development builds. May break.",
    }
}

fn channel_label(channel: Channel) -> String {
    tr(match channel {
        Channel::Stable => "Stable",
        Channel::Preview => "release_channel.preview",
        Channel::Nightly => "Nightly",
    })
}

fn managed_channel_label(managed: &ManagedInstall) -> Option<String> {
    let id = managed.channel()?;
    Some(
        managed
            .tracked_channel()
            .map_or_else(|| id.to_owned(), channel_label),
    )
}

pub(super) fn managed_channel_description(managed: &ManagedInstall) -> String {
    let tracked = match managed_channel_label(managed) {
        Some(channel) => rust_i18n::t!(
            "This install tracks the %{channel} release channel.",
            channel = channel
        )
        .into_owned(),
        None => tr("The installed package decides the release channel."),
    };
    match managed.alternate_instruction() {
        Some(alternate) => join_sentences(&tracked, &alternate),
        None => tracked,
    }
}

pub(super) fn managed_install_summary(managed: &ManagedInstall) -> String {
    let mut lines = vec![managed.ownership_summary()];
    if let Some(channel) = managed_channel_label(managed) {
        lines.push(
            rust_i18n::t!(
                "Tracking the %{channel} release channel.",
                channel = channel
            )
            .into_owned(),
        );
    }
    lines.push(managed.update_instruction());
    lines.extend(managed.alternate_instruction());
    lines.join("\n")
}

pub(super) fn available_status_localized(
    url: &str,
    version: &str,
    method: UpdateMethod,
    returns_to_stable: bool,
) -> String {
    if returns_to_stable && matches!(method, UpdateMethod::InPlace | UpdateMethod::Aur) {
        return rust_i18n::t!(
            "Stable channel target: <a href=\"%{value1}\">v%{value2}</a>",
            value1 = url,
            value2 = version
        )
        .into_owned();
    }
    match method {
        UpdateMethod::InPlace | UpdateMethod::Aur => rust_i18n::t!(
            "Update available: <a href=\"%{value1}\">v%{value2}</a>",
            value1 = url,
            value2 = version
        ),
        UpdateMethod::Omarchy => rust_i18n::t!(
            "Update available: <a href=\"%{value1}\">v%{value2}</a> · Run “omarchy update” to install",
            value1 = url,
            value2 = version
        ),
        UpdateMethod::Pacman => rust_i18n::t!(
            "Update available: <a href=\"%{value1}\">v%{value2}</a> · Install through a full system update",
            value1 = url,
            value2 = version
        ),
    }
    .into_owned()
}

pub(super) fn failed_status_localized(message: &str) -> String {
    rust_i18n::t!(
        "Couldn't check for updates: %{value1} · <a href=\"https://github.com/lgse/strata/releases/latest\">View releases on GitHub</a>",
        value1 = message
    )
    .into_owned()
}

/// Splits a localized string containing `<a href="url">label</a>` around that exact anchor.
pub(super) fn split_anchor<'a>(text: &'a str, url: &str) -> Option<(&'a str, &'a str, &'a str)> {
    let open = format!("<a href=\"{url}\">");
    let (prefix, rest) = text.rsplit_once(&open)?;
    let (link_label, suffix) = rest.split_once("</a>")?;
    Some((prefix, link_label, suffix))
}
