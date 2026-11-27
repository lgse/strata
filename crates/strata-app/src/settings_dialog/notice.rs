// SPDX-License-Identifier: MIT

use rune::{El, Tx, text};
use rune_kit::ButtonVariant;
use strata_components::dialog::{Focus, MESSAGE_WIDTH, StrataDialog, Tone, keep_words_whole};
use strata_core::i18n::tr;
use strata_kit::icons::IconName;

use super::message::{MessageButton, MessageSpec, inline_message};
use crate::{App, settings::PendingSaveNotice, shell, slots::Frame};

const PRIMARY_KEY: &str = "settings-save-notice-close";
const CORNER_KEY: &str = "settings-save-notice-x";

#[derive(Default)]
pub(super) struct NoticeState {
    shown: Option<PendingSaveNotice>,
    open: bool,
    opener: Option<String>,
}

impl NoticeState {
    pub(super) fn is_open(&self) -> bool {
        self.open
    }
}

pub(super) fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let Some(pending) = shell::saved(app).take_notice() else {
        return false;
    };
    let opener = tx.focused().map(str::to_owned);
    let notice = &mut app.settings_dialog.notice;
    notice.shown = Some(pending);
    notice.open = true;
    notice.opener = opener;
    if app.settings_dialog.is_open() {
        tx.focus_when_shown(PRIMARY_KEY, tx.focus_visible() || tx.by_keys());
    }
    true
}

pub(super) fn dismiss_tx(app: &mut App, tx: &mut Tx) {
    let opener = {
        let notice = &mut app.settings_dialog.notice;
        if !notice.open {
            return;
        }
        notice.open = false;
        notice.opener.take()
    };
    if app.settings_dialog.is_open()
        && let Some(opener) = opener
    {
        tx.focus(opener, true);
    }
}

fn dismiss(app: &mut App) {
    app.settings_dialog.notice.open = false;
}

pub(super) fn inline_overlay(app: &App) -> Option<El> {
    let state = &app.settings_dialog.notice;
    if !state.open {
        return None;
    }
    let notice = state.shown.as_ref()?;
    Some(inline_message(
        &app.look,
        MessageSpec {
            key: "settings-save-notice",
            icon: IconName::Close,
            title: notice.title.clone(),
            subtitle: notice.summary.clone(),
            body: notice.message.clone(),
            buttons: vec![MessageButton {
                key: PRIMARY_KEY,
                label: tr("Close"),
                variant: ButtonVariant::Danger,
                on_press: dismiss_tx,
                autofocus: true,
            }],
            close_key: CORNER_KEY,
            dismiss: dismiss_tx,
            enter_dismisses: true,
        },
    ))
}

pub(super) fn standalone_dialog(app: &App, frame: &mut Frame<'_, App>) {
    let state = &app.settings_dialog.notice;
    if !state.open || app.settings_dialog.is_open() {
        return;
    }
    let Some(notice) = state.shown.as_ref() else {
        return;
    };
    let look = &app.look;
    let summary = text(keep_words_whole(&notice.summary))
        .label(notice.summary.clone())
        .font(look.kit.ty.small)
        .color(look.kit.c.text_muted)
        .wrap();
    StrataDialog::new("settings-save-notice", notice.title.clone(), dismiss)
        .open(true)
        .icon(IconName::Close)
        .tone(Tone::Danger)
        .subtitle(notice.summary.clone())
        .child(summary)
        .message(notice.message.clone())
        .primary(tr("Close"), dismiss)
        .initial_focus(Focus::Primary)
        .close_label(tr("Close dialog"))
        .width(MESSAGE_WIDTH)
        .build(look, frame.layer);
}
