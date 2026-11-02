// SPDX-License-Identifier: MIT

//! The Updates page's preferences: automatic checks and the release channel,
//! and how a package-managed install reports the channel it tracks.

use std::rc::Rc;

use gtk::prelude::*;

use super::{
    super::preferences::PreferenceManager, bind_switch, bindings, control_row, search,
    settings_option,
};
use crate::services::{Channel, ManagedInstall, UpdateMethod};

pub(super) fn automatic_updates_option(
    manager: &Rc<PreferenceManager>,
    method: UpdateMethod,
) -> gtk::Box {
    let (row, toggle) = settings_option(
        "Check for updates automatically",
        match method {
            UpdateMethod::InPlace => "Look for a new release on GitHub when Strata starts.",
            UpdateMethod::Aur => "Check the AUR for a newer packaged release when Strata starts.",
            UpdateMethod::Omarchy => {
                "Check the Omarchy package repository for a newer release when Strata starts."
            }
            UpdateMethod::Pacman => {
                "Check the configured package repositories for a newer release when Strata starts."
            }
        },
        manager.checks_for_updates(),
    );
    bind_switch(
        manager,
        &toggle,
        PreferenceManager::checks_for_updates,
        PreferenceManager::set_checks_for_updates,
    );
    row
}

const RELEASE_CHANNEL_TITLE: &str = "Release channel";
const STABLE_CHANNEL: &str = "Stable";
// The release channel, distinct from the file-preview sense of "Preview".
const PREVIEW_CHANNEL: &str = "release_channel.preview";
const NIGHTLY_CHANNEL: &str = "Nightly";

fn channel_label(channel: Channel) -> String {
    crate::i18n::tr(match channel {
        Channel::Stable => STABLE_CHANNEL,
        Channel::Preview => PREVIEW_CHANNEL,
        Channel::Nightly => NIGHTLY_CHANNEL,
    })
}

/// The manifest's channel id, shown by its localized name when Strata knows it.
fn managed_channel_label(managed: &ManagedInstall) -> Option<String> {
    let id = managed.channel()?;
    Some(
        managed
            .tracked_channel()
            .map_or_else(|| id.to_owned(), channel_label),
    )
}

fn channel_option(
    manager: Rc<PreferenceManager>,
    managed: Option<&ManagedInstall>,
) -> (gtk::Box, gtk::MenuButton) {
    let control = bindings::choice_menu(
        &manager,
        RELEASE_CHANNEL_TITLE,
        &[
            (STABLE_CHANNEL, Channel::Stable),
            (PREVIEW_CHANNEL, Channel::Preview),
            (NIGHTLY_CHANNEL, Channel::Nightly),
        ],
        PreferenceManager::release_channel,
        PreferenceManager::set_release_channel,
    );
    let description = managed
        .map(managed_channel_description)
        .unwrap_or_else(|| channel_description(manager.release_channel()).to_owned());
    let row = control_row(RELEASE_CHANNEL_TITLE, &description, &control);
    if managed.is_none() {
        let label = row
            .first_child()
            .and_downcast::<gtk::Box>()
            .expect("channel row copy")
            .last_child()
            .and_downcast::<gtk::Label>()
            .expect("channel description");
        manager.bind_preference(
            &label,
            PreferenceManager::release_channel,
            |widget, channel| {
                widget
                    .downcast_ref::<gtk::Label>()
                    .expect("channel description binding")
                    .set_text(&crate::i18n::tr(channel_description(channel)));
            },
        );
    }
    (row, control)
}

fn channel_description(channel: Channel) -> &'static str {
    match channel {
        Channel::Stable => "Tested releases recommended for everyday use.",
        Channel::Preview => "Try upcoming releases with alpha, beta, and release-candidate builds.",
        Channel::Nightly => "Everything in Preview, plus daily development builds. May break.",
    }
}

fn managed_channel_description(managed: &ManagedInstall) -> String {
    let tracked = match managed_channel_label(managed) {
        Some(channel) => rust_i18n::t!(
            "This install tracks the %{channel} release channel.",
            channel = channel
        )
        .into_owned(),
        None => crate::i18n::tr("The installed package decides the release channel."),
    };
    match managed.alternate_instruction() {
        Some(alternate) => crate::i18n::join_sentences(&tracked, &alternate),
        None => tracked,
    }
}

pub(super) fn managed_install_row(managed: &ManagedInstall) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
    row.add_css_class("settings-option");
    search::tag(&row, "Check for updates");
    let title = gtk::Label::new(Some(&crate::i18n::tr("Package-managed installation")));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let description = gtk::Label::new(Some(&managed_install_summary(managed)));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.set_selectable(true);
    description.add_css_class("settings-option-description");
    row.append(&title);
    row.append(&description);
    row
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

pub(super) fn append_channel_option(
    preferences: &gtk::Box,
    manager: Rc<PreferenceManager>,
    managed: Option<&ManagedInstall>,
    update_method: UpdateMethod,
) {
    let (channel_row, control) = channel_option(manager.clone(), managed);
    let choosable = managed.is_none();
    let weak_control = control.downgrade();
    manager.bind_preference(
        &channel_row,
        PreferenceManager::checks_for_updates,
        move |row, checks| {
            row.set_sensitive(checks);
            if let Some(control) = weak_control.upgrade() {
                bindings::set_choice_sensitive(&control, checks && choosable);
            }
        },
    );
    search::set_available(
        &channel_row,
        managed.is_some() || !update_method.is_package_managed(),
    );
    preferences.append(&channel_row);
}
