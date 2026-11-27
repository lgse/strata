// SPDX-License-Identifier: MIT

use rune::{El, Tx};
use rune_kit::ButtonVariant;
use strata_core::i18n::tr;
use strata_desktop::keys::Map;
use strata_kit::icons::IconName;

use super::{
    super::message::{MessageButton, MessageSpec, inline_message},
    RESET_ALL_KEY,
};
use crate::{App, shell};

const CONFIRM_KEY: &str = "settings-keymap-reset-all-confirm";

pub(super) fn ask(app: &mut App, tx: &mut Tx) {
    let state = &mut app.settings_dialog.keyboard;
    state.end_capture();
    state.confirm = Some(state.shown);
    tx.focus_when_shown(CONFIRM_KEY, tx.focus_visible() || tx.by_keys());
}

/// Closes the confirmation, giving Reset all the keyboard back.
pub(in crate::settings_dialog) fn dismiss(app: &mut App, tx: &mut Tx) {
    if app.settings_dialog.keyboard.confirm.take().is_some() {
        tx.focus(RESET_ALL_KEY, true);
    }
}

fn confirm(app: &mut App, tx: &mut Tx) {
    if let Some(map) = app.settings_dialog.keyboard.confirm {
        shell::saved(app).change(|s| s.keymap.reset_map(map));
    }
    dismiss(app, tx);
}

fn mode_name(map: Map) -> String {
    tr(match map {
        Map::Default => "Default",
        Map::Tenxer => "10xer mode",
    })
}

/// The confirmation while it is open.
pub(in crate::settings_dialog) fn overlay(app: &App) -> Option<El> {
    let map = app.settings_dialog.keyboard.confirm?;
    let button = |key, label: &str, variant, on_press| MessageButton {
        key,
        label: tr(label),
        variant,
        on_press,
        autofocus: false,
    };
    Some(inline_message(
        &app.look,
        MessageSpec {
            key: "settings-keymap-reset-all",
            icon: IconName::Refresh,
            title: tr("Reset all keybinds?"),
            subtitle: mode_name(map),
            body: tr("Restore the shipped keys of every action in the shown mode."),
            buttons: vec![
                button(
                    "settings-keymap-reset-all-cancel",
                    "Cancel",
                    ButtonVariant::Neutral,
                    dismiss,
                ),
                button(CONFIRM_KEY, "Reset", ButtonVariant::Danger, confirm),
            ],
            close_key: "settings-keymap-reset-all-close",
            dismiss,
            enter_dismisses: false,
        },
    ))
}
