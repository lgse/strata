// SPDX-License-Identifier: MIT

//! Main Settings -> Actions page view, localized action summaries, problems card, and
//! delete / notice dialogs, matching `src/ui/settings/actions.rs`.

use std::rc::Rc;

use rune::{Color, Cursor, El, Tx, col, icon, row, text};
use strata_components::{
    controls::{settings_search::SettingsSearch, switch::Switch},
    dialog::{Focus, MESSAGE_WIDTH, StrataDialog, Tone},
};
use strata_core::{
    i18n::{list, tr},
    model::{ExecutionMode, InputKind},
};
use strata_fs::services::{ActionHandle, ActionLoadFailure, error_detail};
use strata_kit::{icons::IconName, look::Look};

use super::{
    ask_delete, confirm_delete, dismiss_delete, dismiss_notice, duplicate_action, set_enabled,
    start_edit, start_export, start_import, start_new,
};
use crate::{
    App, buttons,
    custom_actions::action_icon_glyph,
    settings_dialog::controls::{PageCtx, first_section_heading, group_card},
};

pub(super) const NEW_ACTION_KEY: &str = "settings-actions-new";
pub(super) const IMPORT_ACTION_KEY: &str = "settings-actions-import";

pub(in crate::settings_dialog) fn build(app: &App, search: &SettingsSearch<App>) -> Vec<El> {
    let ctx = PageCtx { app, search };
    let show_list = search.shows_row(app, "custom-actions");
    let show_new = search.shows_row(app, "new-action") || show_list;
    let show_import = search.shows_row(app, "import-action") || show_list;
    if !show_list && !show_new && !show_import {
        return Vec::new();
    }
    let look = &app.look;
    let mut items = Vec::new();
    if let Some(heading) = first_section_heading(ctx, "actions-group", "CUSTOM ACTIONS") {
        items.push(heading);
    }
    items.push(
        text(tr(
            "Add your own scripts to the file and folder context menus. Actions run with your permissions, so only enable scripts you trust.",
        ))
        .font(look.kit.ty.small)
        .color(look.kit.c.text_muted)
        .px(4.0)
        .wrap(),
    );
    items.push(toolbar(look, show_new, show_import));
    if show_list {
        let catalog = app.settings_dialog.actions.catalog(app);
        items.extend(installed_card(look, catalog.actions(), catalog.failures()));
        if !catalog.failures().is_empty() {
            items.extend(problems_section(look, catalog.failures()));
        }
    }
    items
}

fn toolbar(look: &Look, show_new: bool, show_import: bool) -> El {
    let mut bar = row()
        .w_full()
        .pt(12.0)
        .pb(12.0)
        .gap(8.0)
        .justify_end()
        .items_center();
    if show_new {
        bar = bar.child(toolbar_button(
            look,
            NEW_ACTION_KEY,
            tr("New action…"),
            start_new,
        ));
    }
    if show_import {
        bar = bar.child(toolbar_button(
            look,
            IMPORT_ACTION_KEY,
            tr("Import…"),
            start_import,
        ));
    }
    bar
}

fn toolbar_button(
    look: &Look,
    key: &'static str,
    label: String,
    on_press: fn(&mut App, &mut Tx),
) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let btn = row()
        .role("button")
        .label(label.clone())
        .min_h(28.0)
        .px(11.0)
        .py(4.0)
        .center()
        .radius(look.round(5.0))
        .fill(Color::TRANSPARENT)
        .border(1.0, kit.alpha(c.std_border, c.std_border.a * 0.54))
        .hover(kit.alpha(c.bg_accent, 0.15))
        .press(kit.alpha(c.bg_accent, 0.22))
        .focusable(true)
        .focus_ring(2.0, 1.0, kit.alpha(c.border_focus, 0.50))
        .cursor(Cursor::Pointer)
        .child(text(label).font(kit.ty.ui).color(c.text_base).a11y_hidden())
        .on_press_tx(on_press);
    buttons::on_return(key, btn)
}

fn installed_card(
    look: &Look,
    actions: &[Rc<ActionHandle>],
    failures: &[ActionLoadFailure],
) -> Option<El> {
    let mut rows = Vec::new();
    if actions.is_empty() && failures.is_empty() {
        rows.push(Some(
            text(tr(
                "No custom actions yet. Create one to add it to the context menu.",
            ))
            .key("settings-actions-empty")
            .font(look.kit.ty.small)
            .color(look.kit.c.text_muted)
            .padding(18.0)
            .wrap(),
        ));
    } else {
        for action in actions {
            rows.push(Some(action_row(look, action)));
        }
    }
    group_card(look, rows)
}

fn action_row(look: &Look, action: &Rc<ActionHandle>) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let id = action.id().to_owned();
    let name = action.name().to_owned();
    let enabled = action.definition.enabled;
    let toggle_id = id.clone();
    let enabled_switch = Switch::new(enabled)
        .label(tr("Enabled"))
        .on_toggle_tx(move |app: &mut App, _: &mut Tx| {
            set_enabled(app, toggle_id.clone(), !enabled);
        })
        .build(look)
        .key(format!("settings-action-enabled-{id}"));
    let copy = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(
            text(name.clone())
                .font(kit.ty.ui_medium)
                .color(c.text_base)
                .truncate(),
        )
        .child(
            text(summary(action))
                .font(kit.ty.small)
                .color(c.text_muted)
                .wrap(),
        );
    row()
        .key(format!("settings-action-row-{id}"))
        .w_full()
        .min_w(0.0)
        .min_h(64.0)
        .px(18.0)
        .py(14.0)
        .gap(12.0)
        .items_center()
        .child(
            icon(
                action_icon_glyph(look, action.definition.icon.as_deref()),
                20.0,
            )
            .color(c.icon_accent)
            .a11y_hidden(),
        )
        .child(copy)
        .child(enabled_switch)
        .child(action_row_buttons(look, &id, &name))
}

fn action_row_buttons(look: &Look, id: &str, name: &str) -> El {
    let specs = [
        ("edit", "Edit", IconName::Rename, false, RowActionKind::Edit),
        (
            "duplicate",
            "Duplicate",
            IconName::Copy,
            false,
            RowActionKind::Duplicate,
        ),
        (
            "export",
            "Export…",
            IconName::Downloads,
            false,
            RowActionKind::Export,
        ),
        (
            "delete",
            "Delete",
            IconName::Trash,
            true,
            RowActionKind::Delete,
        ),
    ];
    let buttons: Vec<El> = specs
        .into_iter()
        .map(|(slug, label, glyph, danger, kind)| {
            row_icon_button(
                look,
                RowButtonSpec {
                    key: format!("settings-action-{slug}-{id}"),
                    label: tr(label),
                    glyph,
                    danger,
                    action_id: id.to_owned(),
                    action_name: name.to_owned(),
                    kind,
                },
            )
        })
        .collect();
    row().gap(6.0).items_center().children(buttons)
}

#[derive(Clone, Copy)]
enum RowActionKind {
    Edit,
    Duplicate,
    Export,
    Delete,
}

struct RowButtonSpec {
    key: String,
    label: String,
    glyph: IconName,
    danger: bool,
    action_id: String,
    action_name: String,
    kind: RowActionKind,
}

fn row_icon_button(look: &Look, spec: RowButtonSpec) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let (border, ink, hover, press, ring) = if spec.danger {
        (
            kit.alpha(c.danger.fg, 0.40),
            c.danger.icon,
            kit.alpha(c.danger.fg, 0.14),
            kit.alpha(c.danger.fg, 0.22),
            c.danger.fg,
        )
    } else {
        (
            kit.alpha(c.std_border, c.std_border.a * 0.54),
            c.icon_accent,
            kit.alpha(c.bg_accent, 0.15),
            kit.alpha(c.bg_accent, 0.22),
            kit.alpha(c.border_focus, 0.50),
        )
    };
    let id = spec.action_id;
    let name = spec.action_name;
    let kind = spec.kind;
    let btn = row()
        .role("button")
        .label(spec.label.clone())
        .help(spec.label)
        .size(28.0, 28.0)
        .center()
        .radius(look.round(5.0))
        .fill(Color::TRANSPARENT)
        .border(1.0, border)
        .hover(hover)
        .press(press)
        .focusable(true)
        .focus_ring(2.0, 1.0, ring)
        .cursor(Cursor::Pointer)
        .child(icon(look.icon(spec.glyph), 16.0).color(ink).a11y_hidden())
        .on_press_tx(move |app: &mut App, tx: &mut Tx| match kind {
            RowActionKind::Edit => start_edit(app, id.clone()),
            RowActionKind::Duplicate => duplicate_action(app, id.clone(), name.clone()),
            RowActionKind::Export => start_export(app, id.clone()),
            RowActionKind::Delete => ask_delete(app, id.clone(), name.clone(), tx),
        });
    buttons::on_return(spec.key, btn)
}

fn problems_section(look: &Look, failures: &[ActionLoadFailure]) -> Vec<El> {
    let mut items = vec![
        text(tr("PROBLEMS"))
            .font(look.ty.heading)
            .color(look.kit.c.text_muted)
            .px(4.0)
            .pt(24.0)
            .pb(6.0),
        text(tr(
            "These files could not be loaded. Fix them in the actions folder, or remove them.",
        ))
        .font(look.kit.ty.small)
        .color(look.kit.c.text_muted)
        .px(4.0)
        .pb(10.0)
        .wrap(),
    ];
    let rows = failures
        .iter()
        .map(|failure| Some(problem_row(look, failure)))
        .collect();
    if let Some(card) = group_card(look, rows) {
        items.push(card);
    }
    items
}

fn problem_row(look: &Look, failure: &ActionLoadFailure) -> El {
    let line = rust_i18n::t!(
        "%{name}: %{error}",
        name = failure.directory,
        error = failure.error.user_message()
    )
    .into_owned();
    row()
        .key(format!("settings-action-problem-{}", failure.directory))
        .w_full()
        .px(18.0)
        .py(14.0)
        .child(
            text(line)
                .font(look.kit.ty.ui_medium)
                .color(look.kit.c.danger.fg)
                .wrap(),
        )
}

pub(super) fn build_delete_dialog(app: &App, layer: &mut Vec<El>) {
    let Some(confirm) = app.settings_dialog.actions.delete_confirm.as_ref() else {
        return;
    };
    StrataDialog::new(
        format!("settings-action-delete-confirm-{}", confirm.generation),
        tr("Delete this action?"),
        dismiss_delete,
    )
    .open(true)
    .stacked(true)
    .icon(IconName::Trash)
    .tone(Tone::Danger)
    .subtitle(confirm.name.clone())
    .message(tr(
        "The action folder and its script are removed from your actions directory.",
    ))
    .secondary(tr("Cancel"), dismiss_delete)
    .primary(tr("Delete"), confirm_delete)
    .initial_focus(Focus::Primary)
    .close_label(tr("Close dialog"))
    .width(MESSAGE_WIDTH)
    .build(&app.look, layer);
}

pub(super) fn build_notice_dialog(app: &App, layer: &mut Vec<El>) {
    let Some(notice) = app.settings_dialog.actions.notice.as_ref() else {
        return;
    };
    StrataDialog::new(
        format!("settings-action-notice-{}", notice.generation),
        notice.title.clone(),
        dismiss_notice,
    )
    .open(true)
    .stacked(true)
    .icon(IconName::Check)
    .tone(Tone::Accent)
    .subtitle(notice.detail.clone())
    .expose_subtitle(true)
    .primary(tr("Close"), dismiss_notice)
    .initial_focus(Focus::Primary)
    .close_label(tr("Close dialog"))
    .width(MESSAGE_WIDTH)
    .build(&app.look, layer);
}

pub(super) fn summary(action: &ActionHandle) -> String {
    let mut parts = vec![action.definition.run.runtime.localized_label()];
    parts.push(match action.definition.run.mode {
        ExecutionMode::PerItem => tr("per item"),
        ExecutionMode::WholeSelection => tr("whole selection"),
    });
    parts.push(match action.definition.when.kinds.as_slice() {
        [InputKind::File] => tr("files"),
        [InputKind::Folder] => tr("folders"),
        _ => tr("files and folders"),
    });
    if !action.definition.when.extensions.is_empty() {
        parts.push(list(action.definition.when.extensions.iter().cloned()));
    }
    if !action.definition.enabled {
        parts.push(tr("disabled"));
    }
    if let Some(reason) = action.unavailable_reason() {
        parts.push(error_detail(reason.to_owned()));
    }
    parts.join(" · ")
}
