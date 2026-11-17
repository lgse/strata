// SPDX-License-Identifier: MIT

//! Reset all's confirmation: GTK's `confirm_reset_all` (`src/ui/settings/keyboard.rs`), a
//! message over the Settings pages that names the shown mode and restores its shipped keys.
//! It is drawn inside Settings, as the save notice is (`notice.rs`), so Settings stays in the
//! accessibility tree beneath it as GTK's overlay leaves it.

use rune::{El, Key, KeyEvent, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant};
use strata_components::dialog::{
    FOCUS_RING_ALPHA, FOCUS_RING_WIDTH, ICON_BOX, ICON_BOX_RADIUS, MESSAGE_WIDTH, PANEL_RADIUS,
    keep_words_whole,
};
use strata_core::i18n::tr;
use strata_desktop::keys::Map;
use strata_kit::{icons::IconName, look::Look};

use super::RESET_ALL_KEY;
use crate::{App, buttons, shell};

const CONFIRM_KEY: &str = "settings-keymap-reset-all-confirm";
const CANCEL_KEY: &str = "settings-keymap-reset-all-cancel";
const CLOSE_KEY: &str = "settings-keymap-reset-all-close";
const STOPS: [&str; 3] = [CANCEL_KEY, CONFIRM_KEY, CLOSE_KEY];

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
    let look = &app.look;
    let kit = &look.kit;
    let space = look.space;
    let title = tr("Reset all keybinds?");
    let mode = mode_name(map);
    let body = col().w_full().px(space.xl).pt(space.lg).child(
        text(keep_words_whole(&tr(
            "Restore the shipped keys of every action in the shown mode.",
        )))
        .font(kit.ty.ui)
        .color(kit.c.text_base)
        .wrap(),
    );
    let close = buttons::on_return(
        CLOSE_KEY,
        IconButton::new(look.icon(IconName::Close))
            .variant(IconButtonVariant::Ghost)
            .label(tr("Close dialog"))
            .on_press_tx(dismiss)
            .build(kit),
    )
    .pin(None, Some(space.lg), Some(space.lg), None);
    let card = col()
        .key("settings-keymap-reset-all-dialog")
        .role("dialog")
        .label(title.clone())
        .description(mode.clone())
        .w_full()
        .max_w(MESSAGE_WIDTH)
        .radius(look.round(PANEL_RADIUS))
        .fill(kit.c.layer_01)
        .border(1.0, kit.c.border_base)
        .on_press(|_: &mut App| {})
        .on_key(on_key)
        .child(header(look, &title, &mode))
        .child(body)
        .child(footer(look))
        .child(close);
    Some(
        col()
            .key("settings-keymap-reset-all-layer")
            .role("none")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .fill(kit.c.scrim)
            .padding(space.lg)
            .center()
            .on_background_press_tx(dismiss)
            .child(card),
    )
}

fn header(look: &Look, title: &str, mode: &str) -> El {
    let c = &look.kit.c;
    let space = look.space;
    let badge = el()
        .size(ICON_BOX, ICON_BOX)
        .radius(look.round(ICON_BOX_RADIUS))
        .fill(c.danger.bg)
        .border(1.0, c.danger.border)
        .center()
        .a11y_hidden()
        .child(icon(look.icon(IconName::Refresh), look.icon_size.lg).color(c.danger.icon));
    let heading = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(space.xxs)
        .child(
            text(keep_words_whole(title))
                .font(look.ty.title)
                .color(c.text_base)
                .wrap()
                .a11y_hidden(),
        )
        .child(
            text(mode.to_owned())
                .font(look.kit.ty.small)
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

fn footer(look: &Look) -> El {
    let kit = &look.kit;
    let ring = kit.c.danger.fg.alpha(FOCUS_RING_ALPHA);
    let cancel = Button::new(tr("Cancel"))
        .variant(ButtonVariant::Neutral)
        .on_press_tx(dismiss)
        .build(kit);
    let reset = Button::new(tr("Reset"))
        .variant(ButtonVariant::Danger)
        .on_press_tx(confirm)
        .build(kit)
        .focus_ring(FOCUS_RING_WIDTH, 0.0, ring);
    row()
        .w_full()
        .justify_end()
        .gap(look.space.sm)
        .padding(look.space.lg)
        .px(look.space.xl)
        .child(buttons::on_return(CANCEL_KEY, cancel))
        .child(buttons::on_return(CONFIRM_KEY, reset))
}

fn has_command_modifiers(mods: rune::Mods) -> bool {
    mods.ctrl || mods.alt || mods.cmd || mods.logo
}

/// Escape closes it, and Tab cycles its three buttons.
fn on_key(app: &mut App, event: &KeyEvent, tx: &mut Tx) -> bool {
    let mods = event.mods;
    if has_command_modifiers(mods) {
        return false;
    }
    match event.key {
        Key::Escape => {
            dismiss(app, tx);
            true
        }
        Key::Tab => {
            let at = tx
                .focused()
                .and_then(|focused| STOPS.iter().position(|stop| *stop == focused));
            let next = match (at, mods.shift) {
                (None, _) => 0,
                (Some(i), false) => (i + 1) % STOPS.len(),
                (Some(i), true) => (i + STOPS.len() - 1) % STOPS.len(),
            };
            tx.focus(STOPS[next], true);
            true
        }
        _ => false,
    }
}
