// SPDX-License-Identifier: MIT

//! Save-failure and unreadable-at-startup notice dialog (`unsaved-changes-one-notice`).
//!
//! When Settings is open, the notice renders as an overlay layer inside `settings-dialog` while
//! marking Settings' main content `.inert(true)`: GTK's dialogs are `GtkOverlay` children that
//! keep the Settings switches in the accessibility tree underneath (`unsaved-changes-one-notice`
//! line 12), whereas a second `El::modal` in Rune prunes outer modals from `accessibility_tree`.
//! When Settings is closed, the notice renders as a standalone `StrataDialog`.

use rune::{El, Key, KeyEvent, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant};
use strata_components::dialog::{
    FOCUS_RING_ALPHA, FOCUS_RING_WIDTH, Focus, ICON_BOX, ICON_BOX_RADIUS, MESSAGE_WIDTH,
    PANEL_RADIUS, StrataDialog, Tone, keep_words_whole,
};
use strata_core::i18n::tr;
use strata_kit::{icons::IconName, look::Look};

use crate::{App, buttons, settings::PendingSaveNotice, shell, slots::Frame};

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
    let look = &app.look;
    let kit = &look.kit;
    let space = look.space;
    let header = inline_header(look, notice);
    let body = col()
        .w_full()
        .px(space.xl)
        .pt(space.lg)
        .child(paragraph(look, &notice.message));
    let footer = inline_footer(look);
    let close = buttons::on_return(
        CORNER_KEY,
        IconButton::new(look.icon(IconName::Close))
            .variant(IconButtonVariant::Ghost)
            .label(tr("Close dialog"))
            .on_press_tx(dismiss_tx)
            .build(kit),
    )
    .pin(None, Some(space.lg), Some(space.lg), None);
    let card = col()
        .key("settings-save-notice-dialog")
        .role("dialog")
        .label(notice.title.clone())
        .description(notice.summary.clone())
        .w_full()
        .max_w(MESSAGE_WIDTH)
        .radius(look.round(PANEL_RADIUS))
        .fill(kit.c.layer_01)
        .border(1.0, kit.c.border_base)
        .on_press(|_: &mut App| {})
        .on_key(handle_inline_key)
        .child(header)
        .child(body)
        .child(footer)
        .child(close);
    Some(
        col()
            .key("settings-save-notice-layer")
            .role("none")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .fill(kit.c.scrim)
            .padding(space.lg)
            .center()
            .on_background_press_tx(dismiss_tx)
            .child(card),
    )
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

fn inline_header(look: &Look, notice: &PendingSaveNotice) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let space = look.space;
    let badge = el()
        .size(ICON_BOX, ICON_BOX)
        .radius(look.round(ICON_BOX_RADIUS))
        .fill(c.danger.bg)
        .border(1.0, c.danger.border)
        .center()
        .a11y_hidden()
        .child(icon(look.icon(IconName::Close), look.icon_size.lg).color(c.danger.icon));
    let heading = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(space.xxs)
        .child(
            text(keep_words_whole(&notice.title))
                .font(look.ty.title)
                .color(c.text_base)
                .wrap()
                .a11y_hidden(),
        )
        .child(
            text(keep_words_whole(&notice.summary))
                .label(notice.summary.clone())
                .font(kit.ty.small)
                .color(c.text_muted)
                .wrap(),
        );
    row()
        .w_full()
        .gap(space.lg)
        .items_center()
        .padding(space.lg)
        .px(space.xl)
        .pr(space.xl + look.frame.control + space.md)
        .border_sides([0.0, 0.0, 1.0, 0.0], c.border_muted)
        .child(badge)
        .child(heading)
}

fn inline_footer(look: &Look) -> El {
    let kit = &look.kit;
    let ring = kit.c.danger.fg.alpha(FOCUS_RING_ALPHA);
    let close = Button::new(tr("Close"))
        .variant(ButtonVariant::Danger)
        .on_press_tx(dismiss_tx)
        .build(kit)
        .focus_ring(FOCUS_RING_WIDTH, 0.0, ring)
        .autofocus();
    row()
        .w_full()
        .justify_end()
        .padding(look.space.lg)
        .px(look.space.xl)
        .child(buttons::on_return(PRIMARY_KEY, close))
}

fn paragraph(look: &Look, message: &str) -> El {
    text(keep_words_whole(message))
        .label(message.to_owned())
        .font(look.kit.ty.ui)
        .color(look.kit.c.text_base)
        .wrap()
}

fn has_command_modifiers(m: rune::Mods) -> bool {
    m.ctrl || m.alt || m.cmd || m.logo
}

fn cycle_inline_focus(tx: &mut Tx) {
    let next = if tx.focused() == Some(PRIMARY_KEY) {
        CORNER_KEY
    } else {
        PRIMARY_KEY
    };
    tx.focus(next, true);
}

fn handle_inline_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if has_command_modifiers(ev.mods) {
        return false;
    }
    match ev.key {
        Key::Escape | Key::Enter => {
            dismiss_tx(app, tx);
            true
        }
        Key::Tab => {
            cycle_inline_focus(tx);
            true
        }
        _ => false,
    }
}
