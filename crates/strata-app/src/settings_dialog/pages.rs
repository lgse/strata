// SPDX-License-Identifier: MIT

//! Actions, Integrations, Updates, Diagnostics, and About pages.

use rune::{El, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant};
use strata_components::controls::settings_search::SettingsSearch;
use strata_core::i18n::tr;
use strata_desktop::{
    build_info::{AUTHOR, COMMIT, REPOSITORY, build_kind, displayed_version},
    services::{BuildKind, Channel},
};

use super::controls::{
    ChoiceOption, ChoiceSpec, PageCtx, RowSpec, choice_menu, group_card, option_row, pref_switch,
    section_heading,
};
use crate::{App, buttons, shell};

pub(super) fn actions_page(app: &App, search: &SettingsSearch<App>) -> Vec<El> {
    placeholder_card(
        PageCtx { app, search },
        RowSpec::new("custom-actions", "Custom actions", ""),
        "",
    )
}

pub(super) fn integrations_page(app: &App, search: &SettingsSearch<App>) -> Vec<El> {
    placeholder_card(
        PageCtx { app, search },
        RowSpec::new("integrations", "Integrations", "Apps and services"),
        "",
    )
}

pub(super) fn diagnostics_page(app: &App, search: &SettingsSearch<App>) -> Vec<El> {
    placeholder_card(
        PageCtx { app, search },
        RowSpec::new("diagnostics", "Run diagnostics", "Troubleshooting"),
        "",
    )
}

fn placeholder_card(ctx: PageCtx<'_>, spec: RowSpec<'_>, trailing: &str) -> Vec<El> {
    let right = text(trailing.to_owned())
        .font(ctx.app.look.kit.ty.ui)
        .color(ctx.app.look.kit.c.text_muted);
    group_card(&ctx.app.look, vec![option_row(ctx, spec, right)])
        .into_iter()
        .collect()
}

pub(super) fn updates_page(
    app: &App,
    search: &SettingsSearch<App>,
    menu_layer: &mut Vec<El>,
) -> Vec<El> {
    let ctx = PageCtx { app, search };
    let prefs = shell::preferences(app);
    let channel = prefs.release_channel();
    let channel_desc = match channel {
        Channel::Stable => "Tested releases recommended for everyday use.",
        Channel::Preview => "Try upcoming releases with alpha, beta, and release-candidate builds.",
        Channel::Nightly => "Everything in Preview, plus daily development builds. May break.",
    };
    let menu = release_channel_menu(app, channel, !prefs.check_for_updates, menu_layer);
    let version = text(displayed_version())
        .font(app.look.kit.ty.ui)
        .color(app.look.kit.c.text_muted);
    group_card(
        &app.look,
        vec![
            option_row(ctx, RowSpec::new("check", "Check for updates", ""), version),
            pref_switch(
                ctx,
                RowSpec::new(
                    "auto-updates",
                    "Check for updates automatically",
                    "Look for a new release on GitHub when Strata starts.",
                ),
                prefs.check_for_updates,
                |s, on| s.check_for_updates = on,
            ),
            option_row(
                ctx,
                RowSpec::new("channel", "Release channel", channel_desc)
                    .disabled(!prefs.check_for_updates),
                menu,
            ),
        ],
    )
    .into_iter()
    .collect()
}

fn release_channel_menu(
    app: &App,
    channel: Channel,
    disabled: bool,
    menu_layer: &mut Vec<El>,
) -> El {
    let choices = [
        ("Stable", Channel::Stable),
        ("release_channel.preview", Channel::Preview),
        ("Nightly", Channel::Nightly),
    ];
    let current = choices
        .iter()
        .find_map(|&(l, c)| (c == channel).then(|| tr(l)))
        .unwrap_or_else(|| tr("Stable"));
    choice_menu(
        app,
        ChoiceSpec {
            id: "release-channel",
            title: "Release channel",
            current,
            disabled,
            options: choices
                .into_iter()
                .map(|(label, ch)| {
                    ChoiceOption::new(tr(label), channel == ch, move |app, _| {
                        let mut changed = false;
                        shell::saved(app).mutate(|s| {
                            changed = s.set_release_channel(ch);
                            changed
                        });
                        if changed {
                            shell::saved(app).commit();
                        }
                    })
                })
                .collect(),
        },
        menu_layer,
    )
}

pub(super) fn about_page(app: &App, search: &SettingsSearch<App>) -> Vec<El> {
    let ctx = PageCtx { app, search };
    vec![
        about_identity(ctx),
        section_heading(ctx, "about-build", "BUILD"),
        about_build_card(ctx),
        section_heading(ctx, "about-links", "LINKS"),
        about_links_card(ctx),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn about_identity(ctx: PageCtx<'_>) -> Option<El> {
    let app = ctx.app;
    if !ctx.search.shows_row(app, "version") {
        return None;
    }
    let look = &app.look;
    let kit = &look.kit;
    let mut heading = row()
        .gap(look.space.sm)
        .items_center()
        .child(text("Strata").font(look.ty.title).color(kit.c.text_base));
    let kind = build_kind();
    if kind != BuildKind::Stable {
        heading = heading.child(
            text(kind.localized_label())
                .font(kit.ty.small)
                .color(kit.c.text_muted),
        );
    }
    let copy_btn = Button::new(tr("Copy version info"))
        .variant(ButtonVariant::Neutral)
        .on_press_tx(|app: &mut App, _: &mut Tx| {
            app.clipboard.write_text(&version_info_string());
        })
        .build(kit);
    let line = row()
        .w_full()
        .px(18.0)
        .py(18.0)
        .gap(look.space.xl)
        .items_center()
        .child(
            col().grow(1.0).gap(3.0).child(heading).child(
                text(tr("Navigate every layer."))
                    .font(kit.ty.small)
                    .color(kit.c.text_muted),
            ),
        )
        .child(buttons::on_return("settings-about-copy", copy_btn));
    group_card(look, vec![Some(line)])
}

fn about_build_card(ctx: PageCtx<'_>) -> Option<El> {
    let app = ctx.app;
    if !ctx.search.shows_row(app, "version") {
        return None;
    }
    let version = displayed_version();
    let rows = [
        ("Version", version.as_str()),
        ("Commit", COMMIT),
        ("Toolkit", "Rune"),
    ]
    .into_iter()
    .map(|(label, val)| Some(detail_row(app, &tr(label), val)))
    .collect();
    let card = group_card(&app.look, rows)?;
    ctx.search.row(app, &app.look, "version", card)
}

fn about_links_card(ctx: PageCtx<'_>) -> Option<El> {
    let app = ctx.app;
    let links = [
        (
            "website",
            "Website",
            "https://stratafiles.io/".to_owned(),
            "",
        ),
        ("source", "Source code", REPOSITORY.to_owned(), ""),
        (
            "issue",
            "Report an issue",
            format!("{REPOSITORY}/issues/new/choose"),
            "",
        ),
        (
            "license",
            "License",
            format!("{REPOSITORY}/blob/main/LICENSE"),
            "MIT",
        ),
    ];
    let rows = links
        .into_iter()
        .map(|(id, label, uri, detail)| {
            let right = if detail.is_empty() {
                uri
            } else {
                detail.to_owned()
            };
            option_row(
                ctx,
                RowSpec::new(id, label, ""),
                text(right)
                    .font(app.look.kit.ty.small)
                    .color(app.look.kit.c.text_muted),
            )
        })
        .collect();
    group_card(&app.look, rows)
}

fn detail_row(app: &App, label: &str, value: &str) -> El {
    let look = &app.look;
    row()
        .w_full()
        .px(18.0)
        .py(16.0)
        .gap(look.space.xl)
        .items_center()
        .child(
            text(label.to_owned())
                .font(look.kit.ty.ui_medium)
                .color(look.kit.c.text_base)
                .grow(1.0),
        )
        .child(
            text(value.to_owned())
                .font(look.kit.ty.mono)
                .color(look.kit.c.text_muted)
                .selectable(true),
        )
}

fn version_info_string() -> String {
    rust_i18n::t!(
        "Strata %{value1}\n%{value2}\nCommit: %{value3}\nToolkit: %{value4}\nAuthor: %{value5}\nLicense: MIT",
        value1 = displayed_version(),
        value2 = tr("Navigate every layer."),
        value3 = COMMIT,
        value4 = "Rune",
        value5 = AUTHOR
    )
    .into_owned()
}
