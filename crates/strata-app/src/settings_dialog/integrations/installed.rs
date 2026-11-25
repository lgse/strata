// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::{Color, Cursor, El, Pixels, Tx, col, el, icon, row, text};
use strata_components::controls::switch::Switch;
use strata_core::i18n::tr;
use strata_desktop::services::integration_catalog::{Category, Entry, Status};
use strata_kit::{icons::IconName, look::Look};

use super::{
    CatalogRow, IntegrationsState, ManualRow, PendingConfirm, accept_confirm, cancel_confirm,
    request_remove, request_update, set_enabled,
    view::{counted_heading, logo_tile, note_text},
};
use crate::{
    App, buttons,
    settings_dialog::controls::{PickHandler, group_card, panel_border},
};

struct InstalledRowData<'a> {
    id: &'a str,
    name: String,
    origin: &'static str,
    category: Category,
    logo: Option<&'a Pixels>,
    enabled: bool,
    update_entry: Option<(Entry, bool)>,
    remove_question: Option<&'static str>,
}

pub(super) fn section(app: &App, state: &IntegrationsState, compact: bool) -> Option<El> {
    let mut rows = collect_installed_rows(state);
    if rows.is_empty() {
        return None;
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let look = &app.look;
    let count = rows.len();
    let built: Vec<Option<El>> = rows
        .into_iter()
        .map(|(_, data)| Some(build_installed_row(look, state, data, compact)))
        .collect();
    let card = group_card(look, built)?;
    Some(
        col()
            .key("integrations-installed-section")
            .w_full()
            .min_w(0.0)
            .child(counted_heading(look, "INSTALLED", count))
            .child(card),
    )
}

fn collect_installed_rows(state: &IntegrationsState) -> Vec<(String, InstalledRowData<'_>)> {
    let mut items: Vec<(String, InstalledRowData<'_>)> = state
        .rows
        .iter()
        .filter(|row| row.status != Status::NotInstalled)
        .map(catalog_installed_data)
        .collect();
    items.extend(state.manual.iter().map(manual_installed_data));
    items
}

fn catalog_installed_data(row: &CatalogRow) -> (String, InstalledRowData<'_>) {
    let name = tr(&row.entry.name);
    let (update, modified) = match row.status {
        Status::Installed { update, modified } => (update, modified),
        Status::Manual | Status::NotInstalled => (false, false),
    };
    let manual = row.status == Status::Manual;
    let remove_question = if manual {
        Some("This copy was not installed from the catalog. Its folder is deleted.")
    } else if modified {
        Some("The installed copy has local changes that are deleted with it.")
    } else {
        None
    };
    (
        name.to_lowercase(),
        InstalledRowData {
            id: &row.entry.id,
            name,
            origin: if manual {
                "Manually installed"
            } else {
                "Installed via catalog"
            },
            category: row.entry.category,
            logo: row.logo.as_ref(),
            enabled: row.enabled,
            update_entry: update.then(|| (row.entry.clone(), modified)),
            remove_question,
        },
    )
}

fn manual_installed_data(row: &ManualRow) -> (String, InstalledRowData<'_>) {
    let name = row.installation.name.clone();
    (
        name.to_lowercase(),
        InstalledRowData {
            id: &row.installation.id,
            name,
            origin: "Manually installed",
            category: Category::Other,
            logo: row.logo.as_ref(),
            enabled: row.installation.enabled,
            update_entry: None,
            remove_question: Some(
                "This copy was not installed from the catalog. Its folder is deleted.",
            ),
        },
    )
}

fn build_installed_row(
    look: &Look,
    state: &IntegrationsState,
    data: InstalledRowData<'_>,
    compact: bool,
) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let id = data.id;
    let confirm = state.confirmations.get(id);
    let locked = state.busy.contains_key(id) || confirm.is_some();
    let copy = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(
            text(data.name.clone())
                .font(kit.ty.ui_medium)
                .color(c.text_base)
                .wrap(),
        )
        .child(
            text(tr(data.origin))
                .key(format!("integration-{id}-origin"))
                .font(kit.ty.small)
                .color(c.text_muted),
        );
    let actions = row_actions(look, &data, locked);
    let logo = logo_tile(look, data.logo, data.category);
    let heading = installed_row_heading(look, (logo, copy, actions), compact);
    let mut container = col()
        .key(format!("integration-{id}"))
        .w_full()
        .min_w(0.0)
        .px(18.0)
        .py(16.0)
        .gap(6.0)
        .child(heading);
    if let Some(message) = state.row_messages.get(id) {
        container = container
            .child(note_text(look, message.clone(), true).key(format!("integration-{id}-message")));
    }
    if let Some(pending) = confirm {
        container = container.child(confirmation_bar(look, id, pending));
    }
    container
}

fn installed_row_heading(look: &Look, (logo, copy, actions): (El, El, El), compact: bool) -> El {
    if compact {
        return col()
            .w_full()
            .min_w(0.0)
            .gap(look.space.sm)
            .child(
                row()
                    .w_full()
                    .min_w(0.0)
                    .gap(12.0)
                    .items_center()
                    .child(logo)
                    .child(copy),
            )
            .child(row().w_full().min_w(0.0).justify_end().child(actions));
    }
    row()
        .w_full()
        .min_w(0.0)
        .gap(12.0)
        .items_center()
        .child(logo)
        .child(copy)
        .child(actions)
}

fn row_actions(look: &Look, data: &InstalledRowData<'_>, locked: bool) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let id = data.id.to_owned();
    let enabled = data.enabled;
    let state_label = tr(if enabled { "Enabled" } else { "Disabled" });
    let switch = Switch::new(enabled)
        .label(rust_i18n::t!("Enable %{name}", name = data.name).into_owned())
        .disabled(locked)
        .on_toggle_tx(move |app: &mut App, _| set_enabled(app, id.clone(), !enabled))
        .build(look)
        .key(format!("integration-{}-enabled", data.id));
    let toggle_group = row()
        .gap(8.0)
        .items_center()
        .child(text(state_label).font(kit.ty.small).color(c.text_muted))
        .child(switch);
    let separator = el().w(1.0).h(20.0).mx(2.0).fill(panel_border(look));
    let mut bar = row()
        .gap(6.0)
        .items_center()
        .child(toggle_group)
        .child(separator);
    if let Some((entry, modified)) = data.update_entry.clone() {
        let name = data.name.clone();
        bar = bar.child(borderless_icon_button(
            look,
            IconActionSpec {
                key: format!("integration-{}-update", data.id),
                icon: IconName::Refresh,
                tooltip: tr("Update"),
                accessible: rust_i18n::t!("Update %{name}", name = data.name).into_owned(),
                danger: false,
                disabled: locked,
                on_press: Rc::new(move |app, _| {
                    request_update(app, entry.clone(), name.clone(), modified);
                }),
            },
        ));
    }
    let remove_id = data.id.to_owned();
    let remove_name = data.name.clone();
    let question = data.remove_question;
    bar = bar.child(borderless_icon_button(
        look,
        IconActionSpec {
            key: format!("integration-{}-remove", data.id),
            icon: IconName::Trash,
            tooltip: tr("Remove"),
            accessible: rust_i18n::t!("Remove %{name}", name = data.name).into_owned(),
            danger: true,
            disabled: locked,
            on_press: Rc::new(move |app, _| {
                request_remove(app, remove_id.clone(), remove_name.clone(), question);
            }),
        },
    ));
    bar
}

struct IconActionSpec {
    key: String,
    icon: IconName,
    tooltip: String,
    accessible: String,
    danger: bool,
    disabled: bool,
    on_press: PickHandler,
}

fn borderless_icon_button(look: &Look, spec: IconActionSpec) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let (ink, hover, press, ring) = if spec.danger {
        (
            c.danger.icon,
            kit.alpha(c.danger.fg, 0.14),
            kit.alpha(c.danger.fg, 0.22),
            c.danger.fg,
        )
    } else {
        (
            c.icon_base,
            kit.alpha(c.bg_accent, 0.15),
            kit.alpha(c.bg_accent, 0.22),
            kit.alpha(c.border_focus, 0.50),
        )
    };
    let mut btn = row()
        .role("button")
        .label(spec.accessible)
        .help(spec.tooltip)
        .size(28.0, 28.0)
        .center()
        .radius(look.round(5.0))
        .fill(Color::TRANSPARENT)
        .focusable(!spec.disabled)
        .disabled(spec.disabled)
        .child(icon(look.icon(spec.icon), 16.0).color(ink).a11y_hidden());
    if spec.disabled {
        return btn.key(spec.key).layer_opacity(0.45);
    }
    let on_press = spec.on_press;
    btn = btn
        .cursor(Cursor::Pointer)
        .hover(hover)
        .press(press)
        .focus_ring(2.0, 1.0, ring)
        .on_press_tx(move |app: &mut App, tx: &mut Tx| on_press(app, tx));
    buttons::on_return(spec.key, btn)
}

fn confirmation_bar(look: &Look, id: &str, pending: &PendingConfirm) -> El {
    let cancel_id = id.to_owned();
    let confirm_id = id.to_owned();
    let cancel = confirm_button(
        look,
        ConfirmButtonSpec {
            key: format!("integration-{id}-cancel"),
            label: tr("Cancel"),
            accessible: tr("Cancel"),
            primary: false,
            on_press: Rc::new(move |app, _| cancel_confirm(app, &cancel_id)),
        },
    );
    let proceed = confirm_button(
        look,
        ConfirmButtonSpec {
            key: format!("integration-{id}-confirm"),
            label: tr(pending.accept_label),
            accessible: pending.accessible.clone(),
            primary: true,
            on_press: Rc::new(move |app, _| accept_confirm(app, confirm_id.clone())),
        },
    );
    col()
        .key(format!("integration-{id}-confirmation"))
        .w_full()
        .pt(4.0)
        .gap(8.0)
        .child(note_text(look, tr(pending.question), false))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .justify_end()
                .items_center()
                .child(cancel)
                .child(proceed),
        )
}

struct ConfirmButtonSpec {
    key: String,
    label: String,
    accessible: String,
    primary: bool,
    on_press: PickHandler,
}

fn confirm_button(look: &Look, spec: ConfirmButtonSpec) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let (fill, border, ink, hover, press, ring) = if spec.primary {
        (
            c.danger.bg,
            kit.alpha(c.danger.fg, 0.55),
            c.danger.fg,
            kit.alpha(c.danger.fg, 0.18),
            kit.alpha(c.danger.fg, 0.28),
            c.danger.fg,
        )
    } else {
        (
            Color::TRANSPARENT,
            kit.alpha(c.std_border, c.std_border.a * 0.54),
            c.text_base,
            c.hover,
            c.pressed,
            kit.alpha(c.border_focus, 0.50),
        )
    };
    let on_press = spec.on_press;
    let btn = row()
        .role("button")
        .label(spec.accessible)
        .min_h(28.0)
        .px(12.0)
        .py(4.0)
        .center()
        .radius(look.round(5.0))
        .fill(fill)
        .border(1.0, border)
        .hover(hover)
        .press(press)
        .focusable(true)
        .focus_ring(2.0, 1.0, ring)
        .cursor(Cursor::Pointer)
        .child(text(spec.label).font(kit.ty.ui).color(ink).a11y_hidden())
        .on_press_tx(move |app: &mut App, tx: &mut Tx| on_press(app, tx));
    buttons::on_return(spec.key, btn)
}
