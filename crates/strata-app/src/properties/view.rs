// SPDX-License-Identifier: MIT

//! The Properties dialog's view: single-item / folder Properties, multi-item Selection Properties,
//! and the stacked permission-failure notice, built on C15's [`StrataDialog`].

use rune::{El, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, Checkbox, ControlSize, Spinner};
use strata_components::dialog::{Focus, MESSAGE_WIDTH, StrataDialog, Tone, keep_words_whole};
use strata_core::i18n::tr;
use strata_kit::{icons::IconName, look::Look};

use super::{
    Dialog, Notice, Selection, Single,
    actions::{self, PinAction},
    close_notice, dismiss, media,
    permissions::{self, BitSpec, GROUPS, GroupSpec},
};
use crate::{App, clipboard::paths::is_trash, raw_details, sidebar::PinState, slots::Frame};

const LABEL_WIDTH: f32 = 128.0;
const BIT_WIDTH: f32 = 28.0;

pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let state = &app.properties;
    match &state.dialog {
        Some(Dialog::Single(single)) => build_single(app, single, state.open, frame),
        Some(Dialog::Selection(selection)) => {
            build_selection(&app.look, selection, state.open, frame);
        }
        None => {}
    }
    if let Some(notice) = &state.notice {
        build_notice(&app.look, notice, state.notice_open, frame);
    }
    None
}

fn build_single(app: &App, single: &Single, open: bool, frame: &mut Frame<'_, App>) {
    let look = &app.look;
    let key = format!("properties-{}", single.generation);
    let pin_state = app.sidebar.pin_state(&single.location);
    let mut dialog = StrataDialog::new(&key, single.name.clone(), dismiss)
        .open(open)
        .subtitle(single.subtitle.clone())
        .expose_subtitle(true)
        .icon(single.icon)
        .default_button(false)
        .initial_focus(Focus::Close)
        .close_label(tr("Close dialog"))
        .child(general_section(look, &key, single, pin_state));
    if let Some(media) = media_section(look, &key, &single.media) {
        dialog = dialog.child(media);
    }
    dialog = dialog
        .child(permissions_section(look, &key, single))
        .action_tx("open", tr("Open"), actions::open_clicked);
    if !is_trash(&single.location) {
        dialog = dialog
            .action_tx("rename", tr("Rename"), actions::rename_clicked)
            .action_enabled("rename", single.entry.is_some());
    }
    if let Some(pin) = actions::pin_action_for(&single.location, single.is_directory, pin_state) {
        let handler: fn(&mut App) = match pin {
            PinAction::Pin => actions::pin_clicked,
            PinAction::Unpin => actions::unpin_clicked,
        };
        dialog = dialog.action("pin", tr(pin.label()), handler);
    }
    let copy_label = tr(if single.copied { "Copied" } else { "Copy path" });
    dialog
        .action("copy-path", copy_label, actions::copy_path_clicked)
        .build(look, frame.layer);
}

fn build_selection(look: &Look, selection: &Selection, open: bool, frame: &mut Frame<'_, App>) {
    let key = format!("properties-selection-{}", selection.generation);
    let size_suffix = selection.spinning.then(|| {
        spinner_el(
            look,
            format!("{key}-spinner"),
            tr("Calculating selection size"),
        )
    });
    let warning = selection
        .warning
        .as_deref()
        .map(|msg| warning_el(look, format!("{key}-warning"), msg));
    let details = col()
        .key(format!("{key}-details"))
        .w_full()
        .gap(look.space.xs)
        .child(
            DetailRow::new(&tr("SIZE"), &selection.size)
                .maybe_suffix(size_suffix)
                .build(look),
        )
        .child(
            DetailRow::new(&tr("CONTAINS"), &selection.contains)
                .maybe_suffix(warning)
                .build(look),
        );
    StrataDialog::new(&key, selection.title.clone(), dismiss)
        .open(open)
        .subtitle(tr("Selection"))
        .expose_subtitle(true)
        .icon(IconName::Info)
        .default_button(false)
        .initial_focus(Focus::Close)
        .close_label(tr("Close dialog"))
        .child(details)
        .build(look, frame.layer);
}

fn build_notice(look: &Look, notice: &Notice, open: bool, frame: &mut Frame<'_, App>) {
    StrataDialog::new(
        format!("properties-notice-{}", notice.generation),
        notice.title.clone(),
        close_notice,
    )
    .open(open)
    .stacked(true)
    .icon(IconName::Close)
    .tone(Tone::Danger)
    .subtitle(tr("The operation could not be completed"))
    .message(notice.message.clone())
    .primary(tr("Close"), close_notice)
    .initial_focus(Focus::Primary)
    .close_label(tr("Close dialog"))
    .width(MESSAGE_WIDTH)
    .build(look, frame.layer);
}

fn media_section(look: &Look, key: &str, section: &media::Section) -> Option<El> {
    let body = match section {
        media::Section::None => return None,
        media::Section::Media(rows) => {
            let mut list = col()
                .key(format!("{key}-media-rows"))
                .w_full()
                .gap(look.space.xs);
            for (heading, value) in rows {
                list = list.child(DetailRow::new(&tr(heading), value).build(look));
            }
            list
        }
        media::Section::Raw(rows) => {
            raw_details::section_el(look, &format!("{key}-raw-rows"), rows, LABEL_WIDTH)
        }
    };
    Some(
        col()
            .key(format!("{key}-media"))
            .w_full()
            .pt(look.space.xs)
            .border_sides([1.0, 0.0, 0.0, 0.0], look.kit.c.border_muted)
            .child(body),
    )
}

fn general_section(look: &Look, key: &str, single: &Single, pin_state: PinState) -> El {
    let compact = actions::compact_display_path(&single.location);
    let full = single.location.display_path();
    let size_suffix = single.spinning.then(|| {
        spinner_el(
            look,
            format!("{key}-spinner"),
            tr("Calculating folder size"),
        )
    });
    let mut section = col()
        .key(format!("{key}-general"))
        .w_full()
        .gap(look.space.xs)
        .child(
            DetailRow::new(&tr("LOCATION"), &compact)
                .description(&full)
                .build(look),
        )
        .child(
            DetailRow::new(&tr("SIZE"), &single.size)
                .maybe_suffix(size_suffix)
                .build(look),
        );
    if let Some(contains) = &single.contains {
        let warning = single
            .warning
            .as_deref()
            .map(|msg| warning_el(look, format!("{key}-warning"), msg));
        section = section.child(
            DetailRow::new(&tr("CONTAINS"), contains)
                .maybe_suffix(warning)
                .build(look),
        );
    }
    let pinned = tr(if pin_state == PinState::Pinned {
        "Yes"
    } else {
        "No"
    });
    section
        .child(DetailRow::new(&tr("MODIFIED"), &single.modified).build(look))
        .child(DetailRow::new(&tr("OPENS WITH"), &single.opens_with).build(look))
        .child(DetailRow::new(&tr("HIDDEN"), &single.hidden).build(look))
        .child(DetailRow::new(&tr("properties.pinned"), &pinned).build(look))
}

struct DetailRow<'a> {
    heading: &'a str,
    value: &'a str,
    description: Option<&'a str>,
    suffix: Option<El>,
}

impl<'a> DetailRow<'a> {
    fn new(heading: &'a str, value: &'a str) -> Self {
        Self {
            heading,
            value,
            description: None,
            suffix: None,
        }
    }

    fn description(mut self, description: &'a str) -> Self {
        self.description = Some(description);
        self
    }

    fn maybe_suffix(mut self, suffix: Option<El>) -> Self {
        self.suffix = suffix;
        self
    }

    fn build(self, look: &Look) -> El {
        let kit = &look.kit;
        let mut val = text(keep_words_whole(self.value))
            .label(self.value.to_owned())
            .font(kit.ty.ui)
            .color(kit.c.text_base)
            .grow(1.0)
            .min_w(0.0)
            .wrap();
        if let Some(desc) = self.description {
            val = val.description(desc.to_owned());
        }
        let mut line = row()
            .w_full()
            .gap(look.space.md)
            .center_y()
            .child(
                text(keep_words_whole(self.heading))
                    .label(self.heading.to_owned())
                    .font(look.ty.heading)
                    .color(kit.c.text_muted)
                    .w(LABEL_WIDTH),
            )
            .child(val);
        if let Some(end) = self.suffix {
            line = line.child(end);
        }
        line
    }
}

fn spinner_el(look: &Look, key: String, label: String) -> El {
    el().key(key).role("progressbar").label(label).child(
        Spinner::new()
            .size(look.icon_size.sm)
            .build(&look.kit)
            .a11y_hidden(),
    )
}

fn warning_el(look: &Look, key: String, message: &str) -> El {
    let size = look.icon_size.md;
    el().key(key)
        .role("image")
        .size(size, size)
        .focusable(true)
        .label(message.to_owned())
        .center()
        .child(
            icon(look.icon(IconName::Warning), size)
                .color(look.kit.c.warning.icon)
                .a11y_hidden(),
        )
}

fn permissions_section(look: &Look, key: &str, single: &Single) -> El {
    let kit = &look.kit;
    let editor = &single.permissions;
    let mode_text = editor.mode_text();
    let header = row()
        .w_full()
        .gap(look.space.sm)
        .center_y()
        .child(
            text(keep_words_whole(&tr("PERMISSIONS")))
                .label(tr("PERMISSIONS"))
                .font(look.ty.heading)
                .color(kit.c.text_muted)
                .grow(1.0),
        )
        .child(
            text(mode_text.clone())
                .label(mode_text)
                .font(kit.ty.mono)
                .color(kit.c.text_muted),
        );
    let mut section = col()
        .key(format!("{key}-permissions"))
        .w_full()
        .gap(look.space.xs)
        .pt(look.space.xs)
        .border_sides([1.0, 0.0, 0.0, 0.0], kit.c.border_muted)
        .child(header);
    for group in &GROUPS {
        section = section.child(permission_group_row(look, key, editor, group));
    }
    if !single.is_directory {
        let executable = editor.mode.is_some_and(|mode| mode & 0o111 != 0);
        section = section.child(
            Checkbox::new(executable)
                .label(tr("Allow executing file as a program (+x)"))
                .disabled(!editor.sensitive())
                .on_toggle(permissions::toggle_executable)
                .build(kit)
                .key(format!("{key}-executable")),
        );
    }
    section
}

fn permission_group_row(
    look: &Look,
    key: &str,
    editor: &permissions::Editor,
    group: &GroupSpec,
) -> El {
    let kit = &look.kit;
    let title = tr(group.title);
    let identity = editor.identity(group.id).to_owned();
    let mut line = row()
        .key(format!("{key}-perm-{}", group.id))
        .w_full()
        .gap(look.space.xs)
        .center_y()
        .child(
            text(keep_words_whole(&title))
                .label(title)
                .font(kit.ty.ui)
                .color(kit.c.text_base)
                .w(72.0),
        )
        .child(
            text(identity.clone())
                .label(identity)
                .font(kit.ty.small)
                .color(kit.c.text_muted)
                .grow(1.0)
                .min_w(0.0),
        );
    for bit in &group.bits {
        line = line.child(permission_bit_button(look, key, editor, bit));
    }
    line
}

fn permission_bit_button(
    look: &Look,
    key: &str,
    editor: &permissions::Editor,
    bit: &BitSpec,
) -> El {
    let kit = &look.kit;
    let enabled = editor.mode.is_some_and(|mode| mode & bit.mask != 0);
    let sensitive = editor.sensitive();
    let glyph = if enabled { bit.glyph } else { "—" };
    let variant = if enabled {
        ButtonVariant::Neutral
    } else {
        ButtonVariant::Ghost
    };
    let ink = if enabled {
        kit.c.text_accent
    } else {
        kit.c.text_muted
    };
    let mono = kit.ty.mono;
    let mask = bit.mask;
    Button::new(tr(bit.label))
        .variant(variant)
        .control_size(ControlSize::Small)
        .disabled(!sensitive)
        .label_with(move |_, _| text(glyph).font(mono).color(ink).a11y_hidden())
        .on_press(move |app: &mut App| permissions::toggle_bit(app, mask))
        .build(kit)
        .key(format!("{key}-bit-{}", bit.id))
        .w(BIT_WIDTH)
        .checked(enabled)
        .description(tr(bit.description))
}
