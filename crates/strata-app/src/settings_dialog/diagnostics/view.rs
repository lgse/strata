// SPDX-License-Identifier: MIT

//! Rune view for Settings -> Diagnostics, matching `src/ui/settings/diagnostics.rs` and `src/style.css`.

use rune::{Color, Cursor, El, Tx, col, el, icon, row, text};
use rune_kit::{IconButton, IconButtonVariant, Spinner};
use strata_components::{controls::settings_search::SettingsSearch, dialog::keep_words_whole};
use strata_core::i18n::tr;
use strata_kit::{icons::IconName, look::Look};

use super::{
    DiagnosticsState,
    checks::{CheckId, GROUPS, InstallHint, Status, install_hint},
    copy_command, copy_report, format, run_install_command, start,
};
use crate::{
    App, buttons,
    settings_dialog::controls::{card_surface, control_border, group_card, panel_border},
};

const DESCRIPTION: &str = "Checks the tools and services that previews, locations, devices and desktop integration rely on. Sample files are rendered in the preview sandbox and GitHub is contacted to test the connection. Nothing is changed or installed.";

pub(in crate::settings_dialog) fn build(app: &App, search: &SettingsSearch<App>) -> Vec<El> {
    if !search.shows_row(app, "diagnostics") {
        return Vec::new();
    }
    let state = &app.settings_dialog.diagnostics;
    let mut items = Vec::new();
    if let Some(header) = header_card(app, search, state) {
        items.push(header);
    }
    if !state.has_run {
        return items;
    }
    items.push(summary_banner(&app.look, state));
    items.push(group_heading(&app.look, "SYSTEM"));
    items.push(system_card(&app.look, &state.environment));
    for (group, ids) in GROUPS {
        items.push(group_heading(&app.look, format::heading(group)));
        let rows = ids
            .iter()
            .map(|id| Some(check_row(&app.look, state, *id)))
            .collect();
        if let Some(card) = group_card(&app.look, rows) {
            items.push(card);
        }
    }
    items
}

fn header_card(app: &App, search: &SettingsSearch<App>, state: &DiagnosticsState) -> Option<El> {
    let look = &app.look;
    let kit = &look.kit;
    let c = &kit.c;
    let description = tr(DESCRIPTION);
    let copy_label = tr(if state.copied_until.is_some() {
        "Copied"
    } else {
        "Copy report"
    });
    let actions = row()
        .w_full()
        .pt(10.0)
        .gap(8.0)
        .justify_end()
        .items_center()
        .child(action_button(
            look,
            ActionButtonSpec {
                key: "settings-diagnostics-copy",
                label: copy_label,
                primary: false,
                disabled: !state.can_copy(),
                on_press: copy_report,
            },
        ))
        .child(action_button(
            look,
            ActionButtonSpec {
                key: "settings-diagnostics-run",
                label: tr("Run"),
                primary: true,
                disabled: state.running,
                on_press: start,
            },
        ));
    let content = col()
        .w_full()
        .min_w(0.0)
        .px(18.0)
        .py(16.0)
        .gap(3.0)
        .child(search.title(app, look, &tr("Run diagnostics"), kit.ty.ui_medium))
        .child(
            text(keep_words_whole(&description))
                .label(description)
                .font(kit.ty.small)
                .color(c.text_muted)
                .wrap(),
        )
        .child(actions);
    let wrapped = search.row(app, look, "diagnostics", content);
    group_card(look, vec![wrapped])
}

struct ActionButtonSpec {
    key: &'static str,
    label: String,
    primary: bool,
    disabled: bool,
    on_press: fn(&mut App, &mut Tx),
}

fn action_button(look: &Look, spec: ActionButtonSpec) -> El {
    let kit = &look.kit;
    let (fill, border, ink, hover, press, ring) = button_palette(look, spec.primary, spec.disabled);
    let mut btn = row()
        .role("button")
        .label(spec.label.clone())
        .min_h(26.0)
        .px(11.0)
        .py(4.0)
        .center()
        .radius(look.round(5.0))
        .fill(fill)
        .border(1.0, border)
        .focusable(!spec.disabled)
        .disabled(spec.disabled)
        .child(text(spec.label).font(kit.ty.ui).color(ink).a11y_hidden());
    if spec.disabled {
        btn = btn.key(spec.key).layer_opacity(0.7);
        return btn;
    }
    let on_press = spec.on_press;
    let ring_width = if spec.primary { 3.0 } else { 2.0 };
    let ring_offset = if spec.primary { 0.0 } else { 1.0 };
    btn = btn
        .cursor(Cursor::Pointer)
        .hover(hover)
        .press(press)
        .focus_ring(ring_width, ring_offset, ring)
        .on_press_tx(move |app: &mut App, tx: &mut Tx| on_press(app, tx));
    buttons::on_return(spec.key, btn)
}

fn button_palette(
    look: &Look,
    primary: bool,
    disabled: bool,
) -> (Color, Color, Color, Color, Color, Color) {
    let kit = &look.kit;
    let c = &kit.c;
    if disabled {
        return (
            c.surface_disabled,
            c.border_disabled,
            c.text_muted,
            c.surface_disabled,
            c.surface_disabled,
            c.border_focus,
        );
    }
    if primary {
        (
            c.info.bg,
            c.border_active,
            c.text_accent,
            kit.alpha(c.bg_accent, 0.18),
            kit.alpha(c.bg_accent, 0.28),
            kit.alpha(c.text_accent, 0.28),
        )
    } else {
        (
            Color::TRANSPARENT,
            c.border_weak,
            c.text_base,
            c.hover,
            c.pressed,
            kit.alpha(c.border_focus, 0.50),
        )
    }
}

fn summary_banner(look: &Look, state: &DiagnosticsState) -> El {
    let summary = if state.running {
        tr("Running checks…")
    } else {
        format::summary_text(&state.results)
    };
    text(summary)
        .key("settings-diagnostics-summary")
        .font(look.kit.ty.ui)
        .color(look.kit.c.text_muted)
        .px(4.0)
        .pt(18.0)
        .wrap()
}

fn group_heading(look: &Look, heading: &str) -> El {
    text(tr(heading))
        .font(look.ty.heading)
        .color(look.kit.c.text_muted)
        .px(4.0)
        .pt(24.0)
        .pb(10.0)
}

fn system_card(look: &Look, environment: &[(String, String)]) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let mut card = col()
        .key("settings-diagnostics-system")
        .w_full()
        .min_w(0.0)
        .py(15.0)
        .radius(look.round(10.0))
        .fill(card_surface(look))
        .border(1.0, panel_border(look));
    for (label, value) in environment {
        card = card.child(
            row()
                .w_full()
                .min_w(0.0)
                .px(18.0)
                .py(3.0)
                .gap(look.space.xl)
                .items_center()
                .child(
                    text(label.clone())
                        .font(kit.ty.ui_medium)
                        .color(c.text_base)
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    text(value.clone())
                        .font(kit.ty.mono)
                        .color(c.text_muted)
                        .selectable(true),
                ),
        );
    }
    card
}

fn check_row(look: &Look, state: &DiagnosticsState, id: CheckId) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let status = state.results.get(&id).map(|check| check.status);
    let mut copy = col().grow(1.0).min_w(0.0).gap(3.0).child(
        text(tr(format::title(id)))
            .font(kit.ty.ui_medium)
            .color(c.text_base)
            .wrap(),
    );
    copy = append_check_body(look, state, id, copy);
    row()
        .key(format!("settings-diagnostics-row-{}", id.key()))
        .w_full()
        .min_w(0.0)
        .padding(18.0)
        .gap(14.0)
        .items_start()
        .child(status_badge(look, status))
        .child(copy)
}

fn status_badge(look: &Look, status: Option<Status>) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let inner = match status {
        None => el()
            .role("progressbar")
            .label(tr("Checking…"))
            .child(Spinner::new().size(16.0).build(kit).a11y_hidden()),
        Some(status) => {
            let (glyph, tint) = match status {
                Status::Passed => (IconName::Success, c.icon_accent),
                Status::Warning => (IconName::Warning, c.icon_base),
                Status::Problem => (IconName::CircleX, c.danger.icon),
            };
            el().role("image")
                .label(tr(format::status_label(status)))
                .child(icon(look.icon(glyph), 18.0).color(tint).a11y_hidden())
        }
    };
    el().size(18.0, 18.0).pt(1.0).center().child(inner)
}

fn append_check_body(look: &Look, state: &DiagnosticsState, id: CheckId, mut copy: El) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let Some(check) = state.results.get(&id) else {
        return copy.child(
            text(tr("Checking…"))
                .font(kit.ty.small)
                .color(c.text_muted)
                .wrap(),
        );
    };
    let msg = format::message(check);
    let msg_color = if check.status == Status::Problem {
        c.danger.fg
    } else {
        c.text_muted
    };
    copy = copy.child(
        text(keep_words_whole(&msg))
            .label(msg)
            .font(kit.ty.small)
            .color(msg_color)
            .selectable(true)
            .wrap(),
    );
    if let Some(detail) = format::detail(check) {
        copy = copy.child(
            text(detail)
                .font(kit.ty.mono)
                .color(c.text_muted)
                .selectable(true)
                .wrap(),
        );
    }
    if check.status != Status::Passed
        && let Some(hint) = install_hint(check.packages, state.manager)
    {
        copy = copy.child(fix_block(look, state, id, hint));
    }
    copy
}

fn fix_block(look: &Look, state: &DiagnosticsState, id: CheckId, hint: InstallHint) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let mut box_el = col().w_full().min_w(0.0).pt(4.0).gap(6.0);
    match hint {
        InstallHint::Command(command) => {
            box_el = box_el
                .child(
                    text(tr("Install with:"))
                        .font(kit.ty.small)
                        .color(c.text_muted)
                        .wrap(),
                )
                .child(install_command_box(look, state, id, command));
            if let Some(status) = state.terminal_status.get(&id) {
                let (message, color) = match status {
                    Ok(msg) => (msg.clone(), c.text_muted),
                    Err(err) => (err.clone(), c.danger.fg),
                };
                box_el = box_el.child(text(message).font(kit.ty.small).color(color).wrap());
            }
        }
        InstallHint::Package(package) => {
            let text_val = rust_i18n::t!(
                "Install %{package} or your distribution's equivalent.",
                package = package
            )
            .into_owned();
            box_el = box_el.child(text(text_val).font(kit.ty.small).color(c.text_muted).wrap());
        }
    }
    box_el
}

fn install_command_box(look: &Look, state: &DiagnosticsState, id: CheckId, command: String) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let accent_kit = look.accent_icon_kit();
    let copied = state.copied_command.is_some_and(|(who, _)| who == id);
    let cmd_for_run = command.clone();
    let run_label = tr("Run in terminal");
    let run_btn = IconButton::new(look.icon(IconName::Play))
        .variant(IconButtonVariant::Ghost)
        .label(run_label.clone())
        .on_press_tx(move |app: &mut App, _: &mut Tx| {
            run_install_command(app, id, &cmd_for_run);
        })
        .build(&accent_kit)
        .help(run_label);
    let cmd_for_copy = command.clone();
    let copy_icon = if copied {
        IconName::Check
    } else {
        IconName::Copy
    };
    let copy_label = tr(if copied {
        "Install command copied"
    } else {
        "Copy install command"
    });
    let copy_btn = IconButton::new(look.icon(copy_icon))
        .variant(IconButtonVariant::Ghost)
        .label(copy_label.clone())
        .on_press_tx(move |app: &mut App, tx: &mut Tx| {
            copy_command(app, id, &cmd_for_copy, tx);
        })
        .build(&accent_kit)
        .help(copy_label);
    row()
        .key(format!("settings-diagnostics-fix-{}", id.key()))
        .w_full()
        .min_w(0.0)
        .px(10.0)
        .py(6.0)
        .gap(6.0)
        .items_center()
        .radius(look.round(6.0))
        .fill(look.fill.muted)
        .border(1.0, control_border(look))
        .child(
            text(command)
                .font(kit.ty.mono)
                .color(c.text_base)
                .selectable(true)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
        .child(buttons::on_return(
            format!("settings-diagnostics-run-cmd-{}", id.key()),
            run_btn,
        ))
        .child(buttons::on_return(
            format!("settings-diagnostics-copy-cmd-{}", id.key()),
            copy_btn,
        ))
}
