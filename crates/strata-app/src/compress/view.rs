// SPDX-License-Identifier: MIT

//! The Compress dialog's view and its stacked password-validation error dialog
//! (`src/ui/browser/archive.rs`), built on C15's [`StrataDialog`].

use rune::{El, TextEdit, Tx, col, row, text, text_input};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant};
use strata_components::dialog::{Focus, MESSAGE_WIDTH, StrataDialog, Tone};
use strata_core::{i18n::tr, model::ArchiveFormat};
use strata_kit::{icons::IconName, look::Look};

use super::{Form, Notice};
use crate::{App, shell, slots::Frame};

/// The archive formats in GTK's order (`segmented_control(&["ZIP", "7Z", "TAR.GZ", "TAR"], 0)`).
const FORMATS: [(&str, &str, ArchiveFormat); 4] = [
    ("zip", "ZIP", ArchiveFormat::Zip),
    ("7z", "7Z", ArchiveFormat::SevenZ),
    ("tar-gz", "TAR.GZ", ArchiveFormat::TarGz),
    ("tar", "TAR", ArchiveFormat::Tar),
];

pub(super) fn name_key(generation: u64) -> String {
    format!("compress-{generation}-name")
}

pub(super) fn password_key(generation: u64) -> String {
    format!("compress-{generation}-password")
}

fn confirm_key(generation: u64) -> String {
    format!("compress-{generation}-confirm")
}

pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let compress = &app.compress;
    if let Some(form) = &compress.form {
        let mut dialog = StrataDialog::new(
            format!("compress-{}", form.generation),
            form.title.clone(),
            super::cancel,
        )
        .open(compress.open)
        .subtitle(form.subtitle.clone())
        .icon(IconName::Archive)
        .blocking(form.is_dirty())
        .children(body(&app.look, form))
        .secondary(tr("Cancel"), super::cancel)
        .primary(tr("Compress"), super::submit)
        .initial_focus(Focus::First)
        .close_label(tr("Close dialog"));
        // `Form::new` focuses the name field as the dialog opens, so tell Rune's modal stack to
        // restore focus to the listing view on close (`remember_modal_focus`).
        if let Some(view) = shell::view_focus(app) {
            dialog = dialog.return_focus(view);
        }
        dialog.build(&app.look, frame.layer);
    }
    if let Some(notice) = &compress.notice {
        build_notice(&app.look, notice, compress.notice_open, frame);
    }
    None
}

fn build_notice(look: &Look, notice: &Notice, open: bool, frame: &mut Frame<'_, App>) {
    StrataDialog::new(
        format!("compress-notice-{}", notice.generation),
        notice.title.clone(),
        super::close_notice,
    )
    .open(open)
    .stacked(true)
    .icon(IconName::Close)
    .tone(Tone::Danger)
    .subtitle(tr("The operation could not be completed"))
    .message(notice.message.clone())
    .primary(tr("Close"), super::close_notice)
    .initial_focus(Focus::Primary)
    .close_label(tr("Close dialog"))
    .width(MESSAGE_WIDTH)
    .build(look, frame.layer);
}

fn body(look: &Look, form: &Form) -> Vec<El> {
    let mut sections = vec![
        labeled_group(look, &tr("Archive name"), name_field(look, form)),
        labeled_group(look, &tr("archive.format"), format_choices(look, form)),
    ];
    if form.format.supports_password() {
        sections.push(protection_section(look, form));
    }
    sections
}

fn labeled_group(look: &Look, label: &str, control: El) -> El {
    col()
        .w_full()
        .gap(look.space.xs)
        .child(
            text(label.to_owned())
                .font(look.kit.ty.ui_medium)
                .color(look.kit.c.text_base),
        )
        .child(control)
}

fn name_field(look: &Look, form: &Form) -> El {
    let kit = &look.kit;
    let key = name_key(form.generation);
    let invalid = form.name_error.is_some();
    let ink = if invalid && kit.invalid_text {
        kit.c.danger.fg
    } else {
        kit.c.text_base
    };
    let mut input = text_input(&form.name, kit.ty.ui)
        .single_line()
        .key(key.clone())
        .label(tr("Archive name"))
        .color(ink)
        .caret(ink, kit.selection())
        .spellcheck(false)
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .invalid(invalid)
        .on_edit(|app: &mut App| {
            let form = app.compress.form.as_mut().expect("compress form");
            &mut form.name
        })
        .on_input_change(|app: &mut App, _: &mut Tx| super::name_changed(app))
        .on_secondary_press(|_: &mut App, _, _| {});
    if let Some(error) = form.name_error {
        input = input.description(tr(error));
    }
    let inner = field_row(look, format!("{key}-frame")).child(input);
    field_frame(look, inner, form.name.focused, invalid)
}

fn format_choices(look: &Look, form: &Form) -> El {
    let generation = form.generation;
    let buttons = FORMATS.map(|(slug, label, format)| {
        let active = form.format == format;
        segment_button(
            look,
            format!("compress-{generation}-format-{slug}"),
            tr(label),
            active,
        )
        .on_press(move |app: &mut App| super::choose_format(app, format))
    });
    segmented_bar(look, format!("compress-{generation}-format"), buttons)
}

fn protection_section(look: &Look, form: &Form) -> El {
    let generation = form.generation;
    let choices = [
        segment_button(
            look,
            format!("compress-{generation}-protection-none"),
            tr("No password"),
            !form.password_protected,
        )
        .on_press(|app: &mut App| super::set_password_protected(app, false)),
        segment_button(
            look,
            format!("compress-{generation}-protection-password"),
            tr("Password protected"),
            form.password_protected,
        )
        .on_press(|app: &mut App| super::set_password_protected(app, true)),
    ];
    let bar = segmented_bar(look, format!("compress-{generation}-protection"), choices);
    let mut group = labeled_group(look, &tr("Protection"), bar);
    if form.password_protected {
        group = group.child(password_fields(look, form)).gap(look.space.sm);
    }
    group
}

fn password_fields(look: &Look, form: &Form) -> El {
    let generation = form.generation;
    let password = secret_field(
        look,
        &form.password,
        SecretSpec {
            key: password_key(generation),
            label: tr("Password"),
            revealed: form.password_revealed,
            on_edit: |app| &mut app.compress.form.as_mut().expect("compress form").password,
            on_toggle: |app| {
                if let Some(form) = &mut app.compress.form {
                    form.password_revealed = !form.password_revealed;
                }
            },
        },
    );
    let confirm = secret_field(
        look,
        &form.confirm,
        SecretSpec {
            key: confirm_key(generation),
            label: tr("Confirm password"),
            revealed: form.confirm_revealed,
            on_edit: |app| &mut app.compress.form.as_mut().expect("compress form").confirm,
            on_toggle: |app| {
                if let Some(form) = &mut app.compress.form {
                    form.confirm_revealed = !form.confirm_revealed;
                }
            },
        },
    );
    col()
        .w_full()
        .gap(look.space.sm)
        .child(labeled_group(look, &tr("Password"), password))
        .child(labeled_group(look, &tr("Confirm password"), confirm))
}

struct SecretSpec {
    key: String,
    label: String,
    revealed: bool,
    on_edit: fn(&mut App) -> &mut TextEdit,
    on_toggle: fn(&mut App),
}

fn secret_field(look: &Look, edit: &TextEdit, spec: SecretSpec) -> El {
    let kit = &look.kit;
    let ink = kit.c.text_base;
    let input = text_input(edit, kit.ty.ui)
        .single_line()
        .key(spec.key.clone())
        .label(spec.label)
        .color(ink)
        .caret(ink, kit.selection())
        .spellcheck(false)
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .revealed(spec.revealed)
        .on_edit(spec.on_edit)
        .on_secondary_press(|_: &mut App, _, _| {});
    let peek_icon = if spec.revealed {
        IconName::EyeOff
    } else {
        IconName::Eye
    };
    let peek_label = if spec.revealed {
        tr("Hide password")
    } else {
        tr("Show password")
    };
    let peek = IconButton::new(look.icon(peek_icon))
        .variant(IconButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .label(peek_label)
        .on_press(spec.on_toggle)
        .build(kit)
        .focusable(false)
        .key(format!("{}-reveal", spec.key));
    let inner = field_row(look, format!("{}-frame", spec.key))
        .child(input)
        .child(peek);
    field_frame(look, inner, edit.focused, false)
}

fn field_row(look: &Look, key: String) -> El {
    row()
        .key(key)
        .w_full()
        .h(look.frame.control)
        .px(look.space.sm)
        .gap(look.space.xs)
        .items_center()
        .min_w(0.0)
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

/// Builds a single-choice button in a segmented bar (`src/ui/controls.rs:segmented_control`),
/// keeping AT-SPI role `"toggle button"` (`El::checked`) as GTK's `ToggleButton`s do.
fn segment_button(look: &Look, key: String, label: String, active: bool) -> SegmentButton<'_> {
    SegmentButton {
        look,
        key,
        label,
        active,
    }
}

struct SegmentButton<'a> {
    look: &'a Look,
    key: String,
    label: String,
    active: bool,
}

impl SegmentButton<'_> {
    fn on_press(self, press: impl Fn(&mut App) + 'static) -> El {
        let variant = if self.active {
            ButtonVariant::Neutral
        } else {
            ButtonVariant::Ghost
        };
        Button::new(self.label)
            .variant(variant)
            .control_size(ControlSize::Small)
            .on_press(press)
            .build(&self.look.kit)
            .key(self.key)
            .grow(1.0)
            .checked(self.active)
    }
}

fn segmented_bar<const N: usize>(look: &Look, key: String, buttons: [El; N]) -> El {
    let kit = &look.kit;
    row()
        .key(key)
        .w_full()
        .gap(look.space.xxs)
        .px(look.space.xxs)
        .py(look.space.xxs)
        .radius(kit.radius.md)
        .fill(kit.c.layer_01)
        .children(buttons)
}
