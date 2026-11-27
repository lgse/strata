// SPDX-License-Identifier: MIT

use rune::{El, Key, KeyEvent, LiveRegion, Mods, Tx, col, el, icon, row, text, text_input};
use rune_kit::{
    Button, ButtonVariant, Checkbox, IconButton, IconButtonVariant, Placement, Select,
    SelectAppearance, Spinner,
};
use strata_components::dialog::{Focus, MESSAGE_WIDTH, StrataDialog, Tone};
use strata_core::i18n::{list, tr};
use strata_desktop::keys::{
    self, Keys as KeyLabels, Map,
    bindings::{CHOOSE_FOLDER, ENTER, OPEN, RENAME},
    label_in_map,
};
use strata_kit::{icons::IconName, look::Look};

use super::{
    ChooserPrompt, NAME_KEY, accept,
    download::DownloadProgress,
    image_conversion,
    options::ChoiceControl,
    state::{self, ActiveChooser},
};
use crate::{
    App, buttons,
    location::labels::with_keys,
    portal::{ChooserKind, DestinationCheck},
    shell,
};

const ACCEPT_KEY: &str = "chooser-accept";
const CANCEL_KEY: &str = "chooser-cancel";
const DOWNLOAD_CANCEL_KEY: &str = "chooser-download-cancel";

pub(crate) fn build_footer(app: &App, frame: &mut crate::slots::Frame<'_, App>) -> Option<El> {
    let active = app.chooser.active.as_ref()?;
    let look = &app.look;
    let details = build_details(active, look, frame.layer);
    let actions = build_actions(active, look, shell::preferences(app).tenxer_mode);
    Some(
        col()
            .key("chooser-footer")
            .w_full()
            .fill(look.kit.c.bg_deep)
            .children(details)
            .child(actions),
    )
}

fn build_details(active: &ActiveChooser, look: &Look, layer: &mut Vec<El>) -> Option<El> {
    let name_or_files = build_name_or_files(active, look);
    let options = build_options(active, look, layer);
    if name_or_files.is_none() && options.is_empty() {
        return None;
    }
    let kit = &look.kit;
    let space = look.space;
    let mut details = col()
        .key("chooser-details")
        .w_full()
        .px(space.lg)
        .py(space.sm)
        .gap(space.sm)
        .border_sides([1.0, 0.0, 0.0, 0.0], kit.c.border_base);
    if let Some(first) = name_or_files {
        details = details.child(first);
    }
    if !options.is_empty() {
        details = details.child(
            row()
                .key("chooser-options")
                .w_full()
                .flex_wrap()
                .items_center()
                .gap(space.lg)
                .children(options),
        );
    }
    Some(details)
}

fn build_name_or_files(active: &ActiveChooser, look: &Look) -> Option<El> {
    if active.has_name {
        return Some(labeled_row(
            look,
            &tr("Name"),
            build_name_input(active, look),
        ));
    }
    let ChooserKind::SaveFiles { names } = &active.request.kind else {
        return None;
    };
    let joined = list(names.iter().map(|name| name.to_string_lossy().into_owned()));
    let files = text(joined.clone())
        .key("chooser-save-files")
        .label(joined.clone())
        .description(joined)
        .font(look.kit.ty.ui)
        .color(look.kit.c.text_muted)
        .grow(1.0)
        .shrink()
        .truncate();
    Some(labeled_row(look, &tr("Files"), files))
}

fn build_name_input(active: &ActiveChooser, look: &Look) -> El {
    let kit = &look.kit;
    let invalid = active.name_error.is_some();
    let ink = if invalid && kit.invalid_text {
        kit.c.danger.fg
    } else {
        kit.c.text_base
    };
    let mut input = text_input(&active.name, kit.ty.ui)
        .single_line()
        .key(NAME_KEY)
        .label(tr("Name"))
        .placeholder(active.name_placeholder.clone(), kit.c.text_faint)
        .color(ink)
        .caret(ink, kit.selection())
        .spellcheck(false)
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .invalid(invalid)
        .on_edit(|app: &mut App| &mut app.chooser.active.as_mut().expect("chooser name edit").name)
        .on_input_change(|app: &mut App, _: &mut Tx| state::name_changed(app))
        .on_input_key(on_name_key);
    if let Some(error) = &active.name_error {
        input = input.description(error.clone());
    }
    let inner = row()
        .key("chooser-name-frame")
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .h(look.frame.control)
        .px(look.space.sm)
        .items_center()
        .child(input)
        .role("textbox")
        .radius(kit.radius.md)
        .fill(kit.c.bg_base)
        .token_motion(kit.motion.control);
    if invalid {
        inner.outline(1.0, -1.0, kit.c.danger.fg)
    } else if active.name.focused {
        inner.outline(1.0, -1.0, kit.c.border_focus)
    } else {
        kit.elevate(inner, kit.elevation.input())
    }
}

fn on_name_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let composing = app
        .chooser
        .active
        .as_ref()
        .is_some_and(|active| active.name.is_composing());
    if composing || ev.mods != Mods::default() {
        return false;
    }
    match ev.key {
        Key::Enter => {
            if !tx.key_repeat() {
                accept::accept(app, tx);
            }
            true
        }
        Key::Escape => {
            if !tx.key_repeat() {
                escape_name(app, tx);
            }
            true
        }
        _ => false,
    }
}

fn escape_name(app: &mut App, tx: &mut Tx) {
    if !shell::preferences(app).tenxer_mode {
        accept::cancel(app, tx);
        return;
    }
    if let Some(active) = app.chooser.active.as_mut() {
        active.name.focused = false;
    }
    if let Some(view) = shell::view_focus(app) {
        tx.focus(view, true);
    }
}

fn build_options(active: &ActiveChooser, look: &Look, layer: &mut Vec<El>) -> Vec<El> {
    let mut items = Vec::new();
    if let Some(filter_el) = build_filter_select(active, look, layer) {
        items.push(filter_el);
    }
    for (idx, choice) in active.choices.iter().enumerate() {
        items.push(build_choice_control(idx, choice, look, layer));
    }
    if let Some(read_only) = active.read_only {
        let label = tr("Open files read-only");
        let check = Checkbox::new(read_only)
            .label(label)
            .on_toggle(|app: &mut App| {
                if let Some(active) = app.chooser.active.as_mut()
                    && let Some(flag) = active.read_only.as_mut()
                {
                    *flag = !*flag;
                }
            })
            .build(&look.kit)
            .key("chooser-read-only");
        items.push(check);
    }
    items
}

fn build_filter_select(active: &ActiveChooser, look: &Look, layer: &mut Vec<El>) -> Option<El> {
    if active.filters.is_empty() {
        return None;
    }
    let labels: Vec<String> = active
        .filters
        .iter()
        .map(|f| f.portal.label.clone())
        .collect();
    let current = active
        .filter_selected
        .and_then(|idx| labels.get(idx))
        .map(String::as_str);
    let filter_title = tr("chooser.filter");
    let select = Select::new("chooser-filter", &labels)
        .appearance(SelectAppearance::Field)
        .width(200.0)
        .placement(Placement::TopStart)
        .label(filter_title.clone())
        .current(current)
        .open(active.filter_open)
        .on_toggle(|app: &mut App| {
            if let Some(active) = app.chooser.active.as_mut() {
                let next = !active.filter_open;
                active.dismiss_dropdown();
                active.filter_open = next;
            }
        })
        .on_close(|app: &mut App| {
            if let Some(active) = app.chooser.active.as_mut() {
                active.filter_open = false;
            }
        })
        .on_choose(|app: &mut App, chosen: &str, tx: &mut Tx| {
            super::choose_filter(app, chosen, tx);
        })
        .build(&look.kit, layer);
    Some(labeled_row(look, &filter_title, select))
}

fn build_choice_control(
    idx: usize,
    choice: &ChoiceControl,
    look: &Look,
    layer: &mut Vec<El>,
) -> El {
    match choice {
        ChoiceControl::Boolean {
            id, label, checked, ..
        } => build_choice_check(idx, (id, label, *checked), look),
        ChoiceControl::Select {
            id,
            label,
            options,
            selected,
            open,
        } => {
            let labels: Vec<String> = options.iter().map(|(_, text)| text.clone()).collect();
            let current = labels.get(*selected).map(String::as_str);
            let select = Select::new(format!("chooser-choice-{id}"), &labels)
                .appearance(SelectAppearance::Field)
                .width(180.0)
                .placement(Placement::TopStart)
                .label(label.clone())
                .current(current)
                .open(*open)
                .on_toggle(move |app: &mut App| {
                    if let Some(active) = app.chooser.active.as_mut() {
                        active.toggle_choice_list(idx);
                    }
                })
                .on_close(move |app: &mut App| {
                    if let Some(choice) = choice_at(app, idx) {
                        choice.set_open(false);
                    }
                })
                .on_choose(move |app: &mut App, chosen: &str, _tx: &mut Tx| {
                    if let Some(choice) = choice_at(app, idx) {
                        choice.choose(chosen);
                    }
                })
                .build(&look.kit, layer);
            labeled_row(look, label, select)
        }
    }
}

fn choice_at(app: &mut App, idx: usize) -> Option<&mut ChoiceControl> {
    app.chooser.active.as_mut()?.choice_mut(idx)
}

fn build_choice_check(idx: usize, (id, label, checked): (&str, &str, bool), look: &Look) -> El {
    Checkbox::new(checked)
        .label(label.to_owned())
        .on_toggle(move |app: &mut App| {
            if let Some(choice) = choice_at(app, idx) {
                choice.toggle_checked();
            }
        })
        .build(&look.kit)
        .key(format!("chooser-choice-{id}"))
}

fn labeled_row(look: &Look, title: &str, control: El) -> El {
    row()
        .items_center()
        .gap(look.space.md)
        .child(
            text(title.to_owned())
                .label(title.to_owned())
                .description(title.to_owned())
                .font(look.kit.ty.ui_medium)
                .color(look.kit.c.text_base),
        )
        .child(control)
}

fn build_actions(active: &ActiveChooser, look: &Look, tenxer: bool) -> El {
    let kit = &look.kit;
    let space = look.space;
    let status = build_action_status(active, look, tenxer);
    let cancel = buttons::on_return(
        CANCEL_KEY.to_owned(),
        Button::new(tr("Cancel"))
            .variant(ButtonVariant::Neutral)
            .on_press_tx(|app: &mut App, tx: &mut Tx| {
                accept::cancel(app, tx);
            })
            .build(kit)
            .key(CANCEL_KEY),
    );
    let accept_label = mnemonic_stripped(&active.request.accept_label);
    let mut accept_btn = Button::new(accept_label)
        .variant(ButtonVariant::Submit)
        .disabled(!active.accept_sensitive);
    if active.accept_sensitive {
        accept_btn = accept_btn.on_press_tx(|app: &mut App, tx: &mut Tx| {
            accept::accept(app, tx);
        });
    }
    let mut accept = accept_btn.build(kit).key(ACCEPT_KEY).default_button();
    if matches!(
        &active.request.kind,
        ChooserKind::Open {
            directory: true,
            ..
        }
    ) {
        let desc = with_keys(
            "Select folder (%{keys})",
            Some("Select folder"),
            KeyLabels::One(CHOOSE_FOLDER.shown()),
        );
        accept = accept.description(desc);
    }
    let accept = if active.accept_sensitive {
        buttons::on_return(ACCEPT_KEY.to_owned(), accept)
    } else {
        accept
    };
    row()
        .key("chooser-actions")
        .w_full()
        .px(space.lg)
        .py(space.md)
        .gap(space.md)
        .items_center()
        .border_sides([1.0, 0.0, 0.0, 0.0], kit.c.border_base)
        .child(status)
        .child(cancel)
        .child(accept)
}

fn build_action_status(active: &ActiveChooser, look: &Look, tenxer: bool) -> El {
    let kit = &look.kit;
    if let Some(error) = &active.error {
        return row()
            .key("chooser-status-error")
            .grow(1.0)
            .min_w(0.0)
            .items_center()
            .child(
                text(error.clone())
                    .key("chooser-error")
                    .role("alert")
                    .label(error.clone())
                    .live_region(LiveRegion::Assertive)
                    .font(kit.ty.small)
                    .color(kit.c.danger.fg)
                    .grow(1.0)
                    .shrink()
                    .truncate(),
            );
    }
    let mut normal = row()
        .key("chooser-status-normal")
        .grow(1.0)
        .min_w(0.0)
        .gap(look.space.md)
        .items_center();
    if let Some(progress) = &active.download_progress {
        normal = normal.child(build_download_progress(progress, look));
    }
    if tenxer && let Some(hints) = build_save_hints(&active.request.kind, look) {
        normal = normal.child(hints);
    }
    normal
}

fn build_download_progress(progress: &DownloadProgress, look: &Look) -> El {
    let kit = &look.kit;
    let indicator = match progress.fraction {
        Some(_) => {
            let mut bar = el()
                .key("chooser-download-ring")
                .role("progressbar")
                .label(tr("Download progress"))
                .size(20.0, 20.0)
                .center()
                .child(Spinner::new().size(16.0).build(kit).a11y_hidden());
            if let Some(val) = &progress.value_text {
                bar = bar.a11y_value(val.clone());
            }
            bar
        }
        None => Spinner::new().size(16.0).build(kit),
    };
    let cancel = buttons::on_return(
        DOWNLOAD_CANCEL_KEY.to_owned(),
        IconButton::new(look.icon(IconName::Close))
            .variant(IconButtonVariant::Ghost)
            .label(progress.cancel_label.clone())
            .on_press(|app: &mut App| {
                if let Some(active) = app.chooser.active.as_mut() {
                    active.cancel_download();
                }
            })
            .build(&look.accent_icon_kit())
            .key(DOWNLOAD_CANCEL_KEY),
    );
    let mut item = row()
        .key("chooser-download")
        .items_center()
        .gap(look.space.sm)
        .child(
            icon(look.icon(IconName::Downloads), look.icon_size.sm)
                .color(kit.c.icon_accent)
                .a11y_hidden(),
        )
        .child(
            text(progress.name.clone())
                .font(kit.ty.small_medium)
                .color(kit.c.text_base)
                .truncate(),
        )
        .child(indicator);
    if let Some(status) = &progress.status {
        item = item.child(
            text(status.clone())
                .font(kit.ty.small)
                .color(kit.c.text_muted),
        );
    }
    item.child(cancel)
}

fn build_save_hints(kind: &ChooserKind, look: &Look) -> Option<El> {
    let pairs: &[(keys::Shown, &str)] = match kind {
        ChooserKind::SaveFile { .. } => &[
            (ENTER.shown(), "Save here"),
            (RENAME.alternate(0).shown(), "Edit name"),
            (OPEN.shown(), "Replace file"),
        ],
        ChooserKind::SaveFiles { .. } => &[(ENTER.shown(), "Save here")],
        ChooserKind::Open { .. } => return None,
    };
    let kit = &look.kit;
    let locale = rust_i18n::locale();
    let mut hints = row()
        .key("chooser-save-hints")
        .items_center()
        .gap(look.space.sm);
    for (idx, (shown, action)) in pairs.iter().enumerate() {
        let cap = label_in_map(&locale, &KeyLabels::One(*shown), Map::Tenxer);
        hints = hints
            .child(
                row()
                    .key(format!("chooser-hint-{idx}"))
                    .px(look.space.xs)
                    .radius(look.round(4.0))
                    .fill(kit.c.layer_02)
                    .child(text(cap).font(kit.ty.small).color(kit.c.text_muted)),
            )
            .child(text(tr(action)).font(kit.ty.small).color(kit.c.text_muted));
    }
    Some(hints)
}

fn mnemonic_stripped(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '_' {
            if chars.peek() == Some(&'_') {
                out.push('_');
                chars.next();
            }
        } else {
            out.push(ch);
        }
    }
    out
}

pub(crate) fn build_dialog(app: &App, frame: &mut crate::slots::Frame<'_, App>) -> Option<El> {
    let prompt = app.chooser_dialog.prompt.as_ref()?;
    match prompt {
        ChooserPrompt::Overwrite(destinations) => {
            build_overwrite_dialog(destinations, &app.look, frame.layer);
        }
        ChooserPrompt::ConvertImage(consent) => {
            build_conversion_dialog(consent, &app.look, frame.layer);
        }
    }
    None
}

fn build_overwrite_dialog(destinations: &DestinationCheck, look: &Look, layer: &mut Vec<El>) {
    let multiple = destinations.existing.len() > 1;
    let names = list(
        destinations
            .existing
            .iter()
            .take(3)
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().chars().take(80).collect::<String>()),
    );
    let title = tr(if multiple {
        "Replace existing files?"
    } else {
        "Replace existing file?"
    });
    let message = tr(if multiple {
        "One or more destination files already exist. Continuing may overwrite them."
    } else {
        "The destination file already exists. Continuing may overwrite it."
    });
    let paths = destinations.paths.clone();
    StrataDialog::new("chooser-overwrite", title, |app: &mut App| {
        app.chooser_dialog.close();
    })
    .open(true)
    .width(MESSAGE_WIDTH)
    .icon(IconName::Copy)
    .tone(Tone::Danger)
    .subtitle(names)
    .message(message)
    .close_label(tr("Close dialog"))
    .secondary(tr("Cancel"), |app: &mut App| {
        app.chooser_dialog.close();
    })
    .primary_tx(tr("Replace"), move |app: &mut App, tx: &mut Tx| {
        app.chooser_dialog.close();
        accept::complete_paths(app, paths.clone(), None, tx);
    })
    .initial_focus(Focus::Secondary)
    .build(look, layer);
}

fn build_conversion_dialog(
    consent: &image_conversion::ConversionConsent,
    look: &Look,
    layer: &mut Vec<El>,
) {
    let subtitle = rust_i18n::t!(
        "This image is %{value1}, which the selected filter does not accept. Convert it to PNG?",
        value1 = consent.kind.label()
    )
    .into_owned();
    let owned = consent.clone();
    StrataDialog::new(
        "chooser-convert-png",
        tr("Convert image to PNG?"),
        |app: &mut App| {
            app.chooser_dialog.close();
        },
    )
    .open(true)
    .width(MESSAGE_WIDTH)
    .icon(IconName::Copy)
    .tone(Tone::Accent)
    .subtitle(subtitle)
    .close_label(tr("Close dialog"))
    .secondary(tr("Cancel"), |app: &mut App| {
        app.chooser_dialog.close();
    })
    .primary(tr("Convert to PNG"), move |app: &mut App| {
        app.chooser_dialog.close();
        image_conversion::confirm_conversion(app, owned.clone());
    })
    .initial_focus(Focus::Secondary)
    .build(look, layer);
}
