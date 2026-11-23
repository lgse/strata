// SPDX-License-Identifier: MIT

use rune::{El, Key, KeyEvent, Mods, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant};
use strata_components::dialog::{
    FOCUS_RING_ALPHA, FOCUS_RING_WIDTH, ICON_BOX, ICON_BOX_RADIUS, MESSAGE_WIDTH, PANEL_RADIUS,
    keep_words_whole,
};
use strata_core::i18n::tr;
use strata_kit::{icons::IconName, look::Look};

use crate::{App, buttons};

type Handler = fn(&mut App, &mut Tx);

/// One of the message's buttons, in the order they are drawn and cycled.
pub(super) struct MessageButton {
    pub(super) key: &'static str,
    pub(super) label: String,
    pub(super) variant: ButtonVariant,
    pub(super) on_press: Handler,
    /// Whether it takes the keyboard as the message mounts.
    pub(super) autofocus: bool,
}

pub(super) struct MessageSpec {
    pub(super) key: &'static str,
    pub(super) icon: IconName,
    pub(super) title: String,
    pub(super) subtitle: String,
    pub(super) body: String,
    pub(super) buttons: Vec<MessageButton>,
    pub(super) close_key: &'static str,
    pub(super) dismiss: Handler,
    /// Whether a Return no button takes dismisses it too.
    pub(super) enter_dismisses: bool,
}

pub(super) fn inline_message(look: &Look, spec: MessageSpec) -> El {
    let kit = &look.kit;
    let space = look.space;
    let body = col().w_full().px(space.xl).pt(space.lg).child(
        text(keep_words_whole(&spec.body))
            .label(spec.body.clone())
            .font(kit.ty.ui)
            .color(kit.c.text_base)
            .wrap(),
    );
    let close = buttons::on_return(
        spec.close_key,
        IconButton::new(look.icon(IconName::Close))
            .variant(IconButtonVariant::Ghost)
            .label(tr("Close dialog"))
            .on_press_tx(spec.dismiss)
            .build(kit),
    )
    .pin(None, Some(space.lg), Some(space.lg), None);
    let keys = MessageKeys::of(&spec);
    let card = col()
        .key(format!("{}-dialog", spec.key))
        .role("dialog")
        .label(spec.title.clone())
        .description(spec.subtitle.clone())
        .w_full()
        .max_w(MESSAGE_WIDTH)
        .radius(look.round(PANEL_RADIUS))
        .fill(kit.c.layer_01)
        .border(1.0, kit.c.border_base)
        .on_press(|_: &mut App| {})
        .on_key(move |app: &mut App, event: &KeyEvent, tx: &mut Tx| keys.handle(app, event, tx))
        .child(header(look, spec.icon, &spec.title, &spec.subtitle))
        .child(body)
        .child(footer(look, spec.buttons))
        .child(close);
    col()
        .key(format!("{}-layer", spec.key))
        .role("none")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(kit.c.scrim)
        .padding(space.lg)
        .center()
        .on_background_press_tx(spec.dismiss)
        .child(card)
}

fn header(look: &Look, glyph: IconName, title: &str, subtitle: &str) -> El {
    let c = &look.kit.c;
    let space = look.space;
    let badge = el()
        .size(ICON_BOX, ICON_BOX)
        .radius(look.round(ICON_BOX_RADIUS))
        .fill(c.danger.bg)
        .border(1.0, c.danger.border)
        .center()
        .a11y_hidden()
        .child(icon(look.icon(glyph), look.icon_size.lg).color(c.danger.icon));
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
            text(keep_words_whole(subtitle))
                .label(subtitle.to_owned())
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

fn footer(look: &Look, specs: Vec<MessageButton>) -> El {
    let kit = &look.kit;
    let ring = kit.c.danger.fg.alpha(FOCUS_RING_ALPHA);
    let buttons = specs.into_iter().map(|spec| {
        let danger = spec.variant == ButtonVariant::Danger;
        let mut button = Button::new(spec.label)
            .variant(spec.variant)
            .on_press_tx(spec.on_press)
            .build(kit);
        if danger {
            button = button.focus_ring(FOCUS_RING_WIDTH, 0.0, ring);
        }
        if spec.autofocus {
            button = button.autofocus();
        }
        buttons::on_return(spec.key, button)
    });
    row()
        .w_full()
        .justify_end()
        .gap(look.space.sm)
        .padding(look.space.lg)
        .px(look.space.xl)
        .children(buttons.collect::<Vec<_>>())
}

/// What the card's keys need: its Tab stops and how it is dismissed.
struct MessageKeys {
    stops: Vec<&'static str>,
    dismiss: Handler,
    enter_dismisses: bool,
}

impl MessageKeys {
    fn of(spec: &MessageSpec) -> Self {
        let mut stops: Vec<&'static str> = spec.buttons.iter().map(|button| button.key).collect();
        stops.push(spec.close_key);
        Self {
            stops,
            dismiss: spec.dismiss,
            enter_dismisses: spec.enter_dismisses,
        }
    }

    fn handle(&self, app: &mut App, event: &KeyEvent, tx: &mut Tx) -> bool {
        if has_command_modifiers(event.mods) {
            return false;
        }
        match event.key {
            Key::Escape => (self.dismiss)(app, tx),
            Key::Enter if self.enter_dismisses => (self.dismiss)(app, tx),
            Key::Tab => self.cycle(event.mods.shift, tx),
            _ => return false,
        }
        true
    }

    fn cycle(&self, backward: bool, tx: &mut Tx) {
        let count = self.stops.len();
        let at = tx
            .focused()
            .and_then(|focused| self.stops.iter().position(|stop| *stop == focused));
        let next = match (at, backward) {
            (None, _) => 0,
            (Some(i), false) => (i + 1) % count,
            (Some(i), true) => (i + count - 1) % count,
        };
        tx.focus(self.stops[next], true);
    }
}

fn has_command_modifiers(mods: Mods) -> bool {
    mods.ctrl || mods.alt || mods.cmd || mods.logo
}
