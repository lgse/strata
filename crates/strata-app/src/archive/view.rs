// SPDX-License-Identifier: MIT

//! The Extract password dialog (`show_extract_password_dialog` in `src/ui/browser/archive.rs`),
//! built on C15's [`StrataDialog`].

use rune::{El, LiveRegion, Tx, col, row, text, text_input};
use rune_kit::{ControlSize, IconButton, IconButtonVariant};
use strata_components::dialog::{Focus, StrataDialog, keep_words_whole};
use strata_core::i18n::tr;
use strata_kit::{icons::IconName, look::Look};

use super::PasswordForm;
use crate::{App, shell, slots::Frame};

fn dialog_key(generation: u64) -> String {
    format!("extract-password-{generation}")
}

pub(super) fn password_key(generation: u64) -> String {
    format!("{}-password", dialog_key(generation))
}

pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let archive = &app.archive;
    let form = archive.form.as_ref()?;
    let mut dialog = StrataDialog::new(dialog_key(form.generation), tr("Extract"), super::cancel)
        .open(archive.open)
        .subtitle(form.entry.display_name.clone())
        .icon(IconName::Archive)
        .blocking(!form.password.text.is_empty())
        .child(password_section(&app.look, form))
        .secondary(tr("Cancel"), super::cancel)
        .primary_tx(tr("Extract"), super::submit)
        .initial_focus(Focus::First)
        .close_label(tr("Close dialog"));
    if let Some(view) = shell::view_focus(app) {
        dialog = dialog.return_focus(view);
    }
    dialog.build(&app.look, frame.layer);
    None
}

fn password_section(look: &Look, form: &PasswordForm) -> El {
    let kit = &look.kit;
    let label = tr("Password");
    let mut group = col()
        .w_full()
        .gap(look.space.xs)
        .child(
            text(label.clone())
                .font(kit.ty.ui_medium)
                .color(kit.c.text_base),
        )
        .child(password_field(look, form, label));
    if let Some(err) = form.error {
        let message = tr(err);
        group = group.child(
            text(keep_words_whole(&message))
                .label(message)
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

fn password_field(look: &Look, form: &PasswordForm, label: String) -> El {
    let kit = &look.kit;
    let key = password_key(form.generation);
    let invalid = form.error.is_some();
    let ink = if invalid && kit.invalid_text {
        kit.c.danger.fg
    } else {
        kit.c.text_base
    };
    let mut input = text_input(&form.password, kit.ty.ui)
        .single_line()
        .key(key.clone())
        .label(label)
        .color(ink)
        .caret(ink, kit.selection())
        .spellcheck(false)
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .invalid(invalid)
        .revealed(form.revealed)
        .on_edit(|app: &mut App| {
            &mut app
                .archive
                .form
                .as_mut()
                .expect("extract password form")
                .password
        })
        .on_input_change(|app: &mut App, _: &mut Tx| super::clear_error(app))
        .on_secondary_press(|_: &mut App, _, _| {});
    if let Some(err) = form.error {
        input = input.description(tr(err));
    }
    let inner = row()
        .key(format!("{key}-frame"))
        .w_full()
        .h(look.frame.control)
        .px(look.space.sm)
        .gap(look.space.xs)
        .items_center()
        .min_w(0.0)
        .child(input)
        .child(peek_button(look, &key, form.revealed));
    field_frame(look, inner, form.password.focused, invalid)
}

fn peek_button(look: &Look, key: &str, revealed: bool) -> El {
    let (icon, label) = if revealed {
        (IconName::EyeOff, tr("Hide password"))
    } else {
        (IconName::Eye, tr("Show password"))
    };
    IconButton::new(look.icon(icon))
        .variant(IconButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .label(label)
        .on_press(super::toggle_reveal)
        .build(&look.kit)
        .focusable(false)
        .key(format!("{key}-reveal"))
}

fn field_frame(look: &Look, inner: El, focused: bool, invalid: bool) -> El {
    let kit = &look.kit;
    let framed = inner
        .role("textbox")
        .on_secondary_press(|_: &mut App, _, _| {})
        .radius(kit.radius.md)
        .fill(kit.c.bg_base)
        .token_motion(kit.motion.control);
    if invalid {
        framed.outline(1.0, -1.0, kit.c.danger.fg)
    } else if focused {
        framed.outline(1.0, -1.0, kit.c.border_focus)
    } else {
        kit.elevate(framed, kit.elevation.input())
    }
}
