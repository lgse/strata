// SPDX-License-Identifier: MIT

//! Catalog section, category filter chips, responsive card grid, and expandable requirements
//! with copyable/runnable dependency install commands for Settings -> Integrations.

use std::rc::Rc;

use rune::{Color, Cursor, El, col, el, icon, row, text};
use strata_core::i18n::{integer, tr};
use strata_desktop::services::integration_catalog::{Category, RequirementResult, Status};
use strata_kit::{icons::IconName, look::Look};

use super::{
    CatalogRow, IntegrationsState, copy_command, install_entry, run_command, select_category,
    toggle_requirements,
    view::{IconLabelButton, counted_heading, icon_label_button, is_compact, logo_tile, note_text},
};
use crate::{
    App, buttons,
    settings_dialog::{
        controls::{card_surface, panel_border},
        install_command::{CommandBoxSpec, command_box},
    },
};

const MAX_COLUMNS: usize = 3;
const MIN_COLUMN_WIDTH: f32 = 320.0;
const GRID_GAP: f32 = 12.0;

pub(super) fn section(app: &App, state: &IntegrationsState) -> Vec<El> {
    let look = &app.look;
    let mut items = vec![counted_heading(look, "CATALOG", state.rows.len())];
    if let Some(filters) = category_filters(look, state) {
        items.push(filters);
    }
    let shown: Vec<&CatalogRow> = state
        .rows
        .iter()
        .filter(|row| state.category.is_none_or(|cat| row.entry.category == cat))
        .collect();
    if !shown.is_empty() {
        let cols = grid_columns(app, shown.len());
        let compact = is_compact(app);
        let cards: Vec<El> = shown
            .into_iter()
            .map(|row| catalog_card(look, state, row, compact))
            .collect();
        items.push(
            el().key("integrations-catalog-grid")
                .w_full()
                .min_w(0.0)
                .equal_columns(cols)
                .gaps(GRID_GAP, GRID_GAP)
                .items_start()
                .children(cards),
        );
    } else if state.loaded {
        items.push(
            text(tr("No integrations are bundled with this build."))
                .key("integrations-empty")
                .font(look.kit.ty.small)
                .color(look.kit.c.text_muted)
                .px(4.0)
                .wrap(),
        );
    }
    items
}

fn grid_columns(app: &App, shown: usize) -> usize {
    let dialog_w = app
        .window_size()
        .map_or(1250.0, |(w, _)| ((w as f32) - 48.0).min(1250.0));
    let pad_x = if is_compact(app) { 40.0 } else { 96.0 };
    let content_w = (dialog_w - 240.0 - pad_x).max(240.0);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "positive column count clamped to 1..=3"
    )]
    let fit = (((content_w + GRID_GAP) / (MIN_COLUMN_WIDTH + GRID_GAP)).floor() as usize)
        .clamp(1, MAX_COLUMNS);
    fit.min(shown.max(1))
}

fn category_filters(look: &Look, state: &IntegrationsState) -> Option<El> {
    let mut options = vec![(None, state.rows.len())];
    for category in Category::ALL {
        let count = state
            .rows
            .iter()
            .filter(|row| row.entry.category == category)
            .count();
        if count > 0 {
            options.push((Some(category), count));
        }
    }
    if options.len() <= 2 {
        return None;
    }
    let chips: Vec<El> = options
        .into_iter()
        .map(|(category, count)| filter_chip(look, category, count, state.category == category))
        .collect();
    Some(
        row()
            .key("integrations-filters")
            .w_full()
            .pb(12.0)
            .gap(6.0)
            .flex_wrap()
            .children(chips),
    )
}

fn filter_chip(look: &Look, category: Option<Category>, count: usize, active: bool) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let label = tr(category.map_or("All", category_label));
    let count_text = integer(count as u64);
    let accessible =
        rust_i18n::t!("%{label}: %{value}", label = label, value = count_text).into_owned();
    let (fill, border, ink, hover) = if active {
        (
            kit.alpha(c.bg_accent, 0.18),
            c.border_active,
            c.text_accent,
            kit.alpha(c.bg_accent, 0.24),
        )
    } else {
        (Color::TRANSPARENT, panel_border(look), c.text_base, c.hover)
    };
    let chip = row()
        .role("button")
        .label(accessible)
        .checked(active)
        .min_h(28.0)
        .px(11.0)
        .py(4.0)
        .gap(6.0)
        .items_center()
        .radius(look.round(8.0))
        .fill(fill)
        .border(1.0, border)
        .hover(hover)
        .press(c.pressed)
        .focusable(true)
        .focus_ring(2.0, 1.0, kit.alpha(c.border_focus, 0.50))
        .cursor(Cursor::Pointer)
        .child(text(label).font(kit.ty.ui).color(ink).a11y_hidden())
        .child(
            text(count_text)
                .font(kit.ty.small)
                .color(c.text_muted)
                .a11y_hidden(),
        )
        .on_press_tx(move |app: &mut App, _| select_category(app, category));
    buttons::on_return(
        format!(
            "integration-filter-{}",
            category.map_or("all", Category::id)
        ),
        chip,
    )
}

fn catalog_card(
    look: &Look,
    state: &IntegrationsState,
    row_data: &CatalogRow,
    compact: bool,
) -> El {
    let entry = &row_data.entry;
    let kit = &look.kit;
    let c = &kit.c;
    let pad = if compact { 10.0 } else { 18.0 };
    let name = tr(&entry.name);
    let mut copy = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(6.0)
        .child(
            text(name.clone())
                .font(kit.ty.ui_medium)
                .color(c.text_base)
                .wrap(),
        )
        .child(note_text(look, tr(&entry.description), false));
    if row_data.status == Status::NotInstalled
        && let Some(message) = state.row_messages.get(&entry.id)
    {
        copy = copy.child(
            note_text(look, message.clone(), true)
                .key(format!("integration-{}-card-message", entry.id)),
        );
    }
    let trailing = card_trailing(look, state, row_data, &name);
    let body = row()
        .w_full()
        .min_w(0.0)
        .padding(pad)
        .gap(12.0)
        .items_start()
        .child(logo_tile(look, row_data.logo.as_ref(), entry.category))
        .child(copy)
        .child(trailing);
    col()
        .key(format!("integration-{}-card", entry.id))
        .w_full()
        .min_w(0.0)
        .radius(look.round(10.0))
        .fill(card_surface(look))
        .border(1.0, panel_border(look))
        .child(body)
        .child(requirements_section(
            look,
            state,
            (&entry.id, &name),
            compact,
        ))
}

fn card_trailing(look: &Look, state: &IntegrationsState, row_data: &CatalogRow, name: &str) -> El {
    let entry = &row_data.entry;
    if row_data.status == Status::NotInstalled {
        let entry_clone = entry.clone();
        return icon_label_button(
            look,
            IconLabelButton {
                key: format!("integration-{}-install", entry.id),
                icon: IconName::Downloads,
                label: tr("Install"),
                accessible: rust_i18n::t!("Install %{name}", name = name).into_owned(),
                disabled: state.busy.contains(&entry.id),
                on_press: Rc::new(move |app, _| install_entry(app, entry_clone.clone())),
            },
        );
    }
    let c = &look.kit.c;
    row()
        .key(format!("integration-{}-installed", entry.id))
        .gap(6.0)
        .items_center()
        .pt(4.0)
        .child(
            icon(look.icon(IconName::Check), 14.0)
                .color(c.icon_accent)
                .a11y_hidden(),
        )
        .child(
            text(tr("Installed"))
                .font(look.kit.ty.small)
                .color(c.text_accent),
        )
}

fn requirements_section(
    look: &Look,
    state: &IntegrationsState,
    (id, name): (&str, &str),
    compact: bool,
) -> El {
    let results = state.requirements.get(id).map(Vec::as_slice);
    let expanded = state.expanded.contains(id);
    let mut section = col().w_full().min_w(0.0).child(requirements_toggle(
        look,
        (id, name),
        results,
        (expanded, compact),
    ));
    if expanded {
        section = section.child(requirements_details(look, state, (id, compact), results));
    }
    section
}

fn requirements_toggle(
    look: &Look,
    (id, name): (&str, &str),
    results: Option<&[RequirementResult]>,
    (expanded, compact): (bool, bool),
) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let met = results.map(|items| items.iter().all(|item| item.satisfied));
    let title = tr(if expanded {
        "Hide requirements"
    } else {
        "Requirements"
    });
    let chevron = if expanded {
        IconName::ChevronUp
    } else {
        IconName::ChevronDown
    };
    let mut heading = row()
        .w_full()
        .min_w(0.0)
        .px(if compact { 10.0 } else { 18.0 })
        .py(if compact { 10.0 } else { 12.0 })
        .gap(6.0)
        .items_center()
        .child(
            text(title)
                .font(kit.ty.small)
                .color(c.text_muted)
                .a11y_hidden(),
        );
    if let Some(met) = met {
        let (glyph, tint) = if met {
            (IconName::Success, c.icon_accent)
        } else {
            (IconName::CircleX, c.danger.icon)
        };
        heading = heading.child(icon(look.icon(glyph), 14.0).color(tint).a11y_hidden());
    }
    heading = heading.child(el().grow(1.0)).child(
        icon(look.icon(chevron), 14.0)
            .color(kit.alpha(c.icon_accent, 0.65))
            .a11y_hidden(),
    );
    let toggle_id = id.to_owned();
    let mut btn = heading
        .role("button")
        .label(rust_i18n::t!("Requirements for %{name}", name = name).into_owned())
        .expanded(expanded)
        .border_sides([1.0, 0.0, 0.0, 0.0], panel_border(look))
        .fill(Color::TRANSPARENT)
        .hover(kit.alpha(c.bg_accent, 0.08))
        .press(kit.alpha(c.bg_accent, 0.14))
        .focusable(true)
        .focus_ring(2.0, -2.0, kit.alpha(c.border_focus, 0.50))
        .cursor(Cursor::Pointer)
        .on_press_tx(move |app: &mut App, _| toggle_requirements(app, toggle_id.clone()));
    if let Some(met) = met {
        btn = btn.description(tr(if met {
            "All requirements are met."
        } else {
            "Some requirements are missing."
        }));
    }
    buttons::on_return(format!("integration-{id}-requirements"), btn)
}

fn requirements_details(
    look: &Look,
    state: &IntegrationsState,
    (id, compact): (&str, bool),
    results: Option<&[RequirementResult]>,
) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let pad = if compact { 10.0 } else { 18.0 };
    let mut chips = row()
        .w_full()
        .min_w(0.0)
        .gap(6.0)
        .items_center()
        .flex_wrap()
        .child(text(tr("Requires:")).font(kit.ty.small).color(c.text_muted));
    let Some(results) = results else {
        chips = chips.child(text(tr("Checking…")).font(kit.ty.small).color(c.text_muted));
        return col()
            .key(format!("integration-{id}-requirements-details"))
            .w_full()
            .min_w(0.0)
            .padding(pad)
            .gap(10.0)
            .child(chips);
    };
    let mut hints = Vec::new();
    for (idx, result) in results.iter().enumerate() {
        chips = chips.child(requirement_chip(look, id, idx, result));
        if !result.satisfied
            && let Some(command) = result.requirement.install_command(state.manager)
            && !hints.contains(&command)
        {
            hints.push(command);
        }
    }
    let mut container = col()
        .key(format!("integration-{id}-requirements-details"))
        .w_full()
        .min_w(0.0)
        .padding(pad)
        .gap(10.0)
        .child(chips);
    for (idx, command) in hints.into_iter().enumerate() {
        container = container.child(install_hint_block(look, state, (id, idx), command));
    }
    container
}

fn requirement_chip(look: &Look, id: &str, idx: usize, result: &RequirementResult) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let (glyph, icon_color, text_color, status) = if result.satisfied {
        (IconName::Success, c.icon_accent, c.text_muted, "Available")
    } else {
        (IconName::CircleX, c.danger.icon, c.danger.fg, "Missing")
    };
    let name = result.requirement.label();
    let accessible =
        rust_i18n::t!("%{label}: %{value}", label = name, value = tr(status)).into_owned();
    row()
        .key(format!("integration-{id}-req-{idx}"))
        .gap(4.0)
        .items_center()
        .child(icon(look.icon(glyph), 14.0).color(icon_color).a11y_hidden())
        .child(
            text(name.to_owned())
                .label(accessible)
                .font(kit.ty.small)
                .color(text_color),
        )
}

fn install_hint_block(
    look: &Look,
    state: &IntegrationsState,
    (id, idx): (&str, usize),
    command: String,
) -> El {
    let cmd_key = (id.to_owned(), idx);
    let copied = state
        .copied_command
        .as_ref()
        .is_some_and(|(who, _)| who == &cmd_key);
    let run_key = cmd_key.clone();
    let run_cmd = command.clone();
    let copy_key = cmd_key.clone();
    let copy_cmd = command.clone();
    let box_el = command_box(
        look,
        CommandBoxSpec {
            key: format!("integration-{id}-fix-{idx}"),
            run_key: format!("integration-{id}-run-cmd-{idx}"),
            copy_key: format!("integration-{id}-copy-cmd-{idx}"),
            command,
            copied,
            on_run: Rc::new(move |app, _| run_command(app, run_key.clone(), &run_cmd)),
            on_copy: Rc::new(move |app, tx| copy_command(app, copy_key.clone(), &copy_cmd, tx)),
        },
    );
    let mut block = col()
        .w_full()
        .min_w(0.0)
        .gap(6.0)
        .child(note_text(
            look,
            tr("Install missing dependencies with:"),
            false,
        ))
        .child(box_el);
    if let Some(status) = state.terminal_status.get(&cmd_key) {
        let (message, is_error) = match status {
            Ok(msg) => (msg.clone(), false),
            Err(err) => (err.clone(), true),
        };
        block = block.child(
            note_text(look, message, is_error).key(format!("integration-{id}-cmd-status-{idx}")),
        );
    }
    block
}

fn category_label(category: Category) -> &'static str {
    match category {
        Category::Cloud => "Cloud",
        Category::Devices => "Devices",
        Category::VersionControl => "Version control",
        Category::Other => "Other",
    }
}
