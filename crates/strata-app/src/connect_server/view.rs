// SPDX-License-Identifier: MIT

//! Rendering of the Connect to server dialog (`src/ui/window/network_location.rs`).

use rune::{El, LiveRegion, Tx, col, row, text, text_input};
use rune_kit::Checkbox;
use strata_components::dialog::{StrataDialog, keep_words_whole};
use strata_core::i18n::tr;
use strata_kit::{icons::IconName, look::Look};

use super::{Form, cancel, clear_error, submit, toggle_pin};
use crate::App;

/// Strata's `.action-dialog.wide` width (`strata_components::dialog` docs: 520 pt).
const WIDE_WIDTH: f32 = 520.0;

/// URI example placeholder, never translated (`network_location.rs`).
const ADDRESS_EXAMPLE: &str = "smb://server/share";

pub(super) fn dialog_key(generation: u64) -> String {
    format!("connect-server-{generation}")
}

pub(super) fn address_key(generation: u64) -> String {
    format!("{}-address", dialog_key(generation))
}

pub(super) fn pin_key(generation: u64) -> String {
    format!("{}-pin", dialog_key(generation))
}

#[cfg(test)]
pub(super) fn primary_key(generation: u64) -> String {
    format!("{}-primary", dialog_key(generation))
}

pub(super) fn build_dialog(app: &App, form: &Form, open: bool, layer: &mut Vec<El>) {
    let look = &app.look;
    let key = dialog_key(form.generation);
    StrataDialog::new(key, tr("Connect to server"), cancel)
        .open(open)
        .subtitle(tr("Connect to a share or remote server"))
        .icon(IconName::Network)
        .width(WIDE_WIDTH)
        .blocking(form.blocks_backdrop())
        .loading(form.connecting.then(|| tr("Connecting…")))
        .child(address_section(look, form))
        .child(pin_checkbox(look, form))
        .secondary(tr("Cancel"), cancel)
        .primary_enabled(!form.connecting)
        .primary(tr("Connect"), submit)
        .close_label(tr("Close dialog"))
        .build(look, layer);
}

fn address_section(look: &Look, form: &Form) -> El {
    let kit = &look.kit;
    let label = tr("Server address");
    let field = address_input(look, form, &label);

    let mut group = col()
        .w_full()
        .gap(look.space.xs)
        .child(text(label).font(kit.ty.ui_medium).color(kit.c.text_base))
        .child(field);
    if let Some(message) = &form.error {
        group = group.child(
            text(keep_words_whole(message))
                .key(format!(
                    "{}-error-{}",
                    dialog_key(form.generation),
                    form.attempt
                ))
                .font(kit.ty.small)
                .color(kit.c.danger.fg)
                .wrap()
                .live_region(LiveRegion::Polite),
        );
    }
    group
}

fn address_input(look: &Look, form: &Form, label: &str) -> El {
    let kit = &look.kit;
    let key = address_key(form.generation);
    let disabled = form.connecting;
    let invalid = form.error.is_some() && !disabled;
    let ink = if disabled {
        kit.c.text_muted
    } else if invalid && kit.invalid_text {
        kit.c.danger.fg
    } else {
        kit.c.text_base
    };
    let hint = if invalid {
        kit.c.danger.fg
    } else {
        kit.c.text_faint
    };
    let mut input = text_input(&form.address, kit.ty.ui)
        .single_line()
        .key(key.clone())
        .label(label.to_owned())
        .placeholder(ADDRESS_EXAMPLE, hint)
        .color(ink)
        .caret(ink, kit.selection())
        .spellcheck(false)
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .invalid(invalid);
    if disabled {
        input = input.disabled(true);
    } else {
        input = input
            .on_edit(|app: &mut App| {
                &mut app
                    .connect_server
                    .form
                    .as_mut()
                    .expect("connect-server form")
                    .address
            })
            .on_input_change(|app: &mut App, _: &mut Tx| clear_error(app))
            .on_secondary_press(|_: &mut App, _, _| {});
    }
    if let Some(message) = &form.error {
        input = input.description(message.clone());
    }
    address_frame(look, form, &key, input)
}

fn address_frame(look: &Look, form: &Form, key: &str, input: El) -> El {
    let kit = &look.kit;
    let invalid = form.error.is_some() && !form.connecting;
    let inner = row()
        .key(format!("{key}-frame"))
        .w_full()
        .h(look.frame.control)
        .px(look.space.sm)
        .gap(look.space.xs)
        .items_center()
        .min_w(0.0)
        .child(input)
        .role("textbox")
        .on_secondary_press(|_: &mut App, _, _| {})
        .radius(kit.radius.md)
        .fill(kit.c.bg_base)
        .token_motion(kit.motion.control);
    if invalid {
        inner.outline(1.0, -1.0, kit.c.danger.fg)
    } else if form.address.focused && !form.connecting {
        inner.outline(1.0, -1.0, kit.c.border_focus)
    } else {
        kit.elevate(inner, kit.elevation.input())
    }
}

fn pin_checkbox(look: &Look, form: &Form) -> El {
    Checkbox::new(form.pin)
        .label(tr("Pin to sidebar"))
        .disabled(form.connecting)
        .on_toggle(toggle_pin)
        .build(&look.kit)
        .key(pin_key(form.generation))
}
