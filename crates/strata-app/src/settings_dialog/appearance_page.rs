// SPDX-License-Identifier: MIT

//! The Appearance settings page (`src/ui/settings/theme.rs`).

use std::rc::Rc;

use rune::{Color, Cursor, El, ScrollSpec, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant};
use strata_components::controls::{
    number_field::{NumberField, NumberFieldState},
    settings_search::SettingsSearch,
};
use strata_core::i18n::tr;
use strata_desktop::settings::{
    CornerRoundness, FileIconColors, InterfaceRenderer, OmarchyVariant, Preferences, TextSize,
    normalized_font_family,
};
use strata_kit::{
    fonts::FAMILY,
    icons::IconName,
    look::Look,
    theme::{Colors, Theme},
};

use super::controls::{
    ChoiceOption, ChoiceSpec, PageCtx, RowSpec, SearchField, bounded_scroll_with_bar, card_surface,
    choice_menu, first_section_heading, group_card, number_field, option_row, panel_border,
    search_field, section_heading, segmented_toggles, settings_bar_look, switch_row,
};
use crate::{App, buttons, shell};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ThemeFilter {
    #[default]
    All,
    Light,
    Dark,
}

pub(super) fn text_size_field() -> NumberField<App> {
    number_field(
        "settings-text-size",
        "Text size in pixels",
        (
            |app: &App| &app.settings_dialog.text_size,
            |app: &mut App| &mut app.settings_dialog.text_size,
        ),
        |app, value, tx| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clamped 8..=48"
            )]
            let px = value.round() as u32;
            shell::saved(app).change(|s| s.text_size = TextSize::new(px));
            shell::follow_look_tx(app, tx);
        },
    )
}

pub(super) fn roundness_field() -> NumberField<App> {
    number_field(
        "settings-corner-roundness",
        "Corner roundness in percent",
        (
            |app: &App| &app.settings_dialog.roundness,
            |app: &mut App| &mut app.settings_dialog.roundness,
        ),
        |app, value, tx| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clamped 0..=200"
            )]
            let pct = value.round() as u32;
            shell::saved(app).change(|s| s.corner_roundness = CornerRoundness::new(pct));
            shell::follow_look_tx(app, tx);
        },
    )
}

pub(super) fn build(app: &App, search: &SettingsSearch<App>, menu_layer: &mut Vec<El>) -> Vec<El> {
    let ctx = PageCtx { app, search };
    let prefs = shell::preferences(app);
    let blocks: Vec<Option<El>> = vec![
        first_section_heading(ctx, "theme-source", "THEME"),
        theme_source_section(ctx, menu_layer),
        theme_library_section(ctx),
        section_heading(ctx, "rendering", "RENDERING"),
        rendering_section(ctx, menu_layer),
        section_heading(ctx, "text-group", "TEXT"),
        text_section(ctx, menu_layer),
        section_heading(ctx, "shape", "SHAPE"),
        shape_section(ctx),
        section_heading(ctx, "file-icons", "FILE ICONS"),
        file_icons_section(ctx, menu_layer),
        section_heading(ctx, "effects", "EFFECTS"),
        look_switch_card(
            ctx,
            RowSpec::new(
                "glow",
                "Element glow",
                "Show accent glow around dialogs, menus, and other elements.",
            ),
            prefs.element_glow,
            |s, on| s.element_glow = on,
        ),
        section_heading(ctx, "transparency", "TRANSPARENCY"),
        super::transparency_section::build(ctx),
        section_heading(ctx, "motion-group", "MOTION"),
        look_switch_card(
            ctx,
            RowSpec::new(
                "motion",
                "Reduce motion",
                "Disable nonessential interface animations.",
            ),
            prefs.reduce_motion,
            |s, on| s.reduce_motion = on,
        ),
    ];
    blocks.into_iter().flatten().collect()
}

fn theme_source_section(ctx: PageCtx<'_>, menu_layer: &mut Vec<El>) -> Option<El> {
    let app = ctx.app;
    let prefs = shell::preferences(app);
    let themes = shell::themes(app);
    let system_avail = themes.system_available();
    let omarchy_avail = themes.omarchy_available();
    let follows_system = system_avail && prefs.mode == "system";
    let follows_omarchy = omarchy_avail && prefs.mode == "omarchy";

    let mut rows = Vec::new();
    if system_avail {
        rows.push(follow_system_row(ctx, follows_system));
    }
    if omarchy_avail {
        rows.extend(follow_omarchy_rows(ctx, follows_omarchy, menu_layer));
    }
    rows.push(current_theme_row(ctx, (follows_system, follows_omarchy)));
    group_card(&app.look, rows)
}

fn follow_system_row(ctx: PageCtx<'_>, follows_system: bool) -> Option<El> {
    switch_row(
        ctx,
        RowSpec::new(
            "system-light-dark-mode",
            "Follow system light/dark mode",
            "Use the light or dark version of the chosen theme to match your desktop's light/dark mode.",
        ),
        follows_system,
        |app, on, tx| {
            let mode = if on { "system" } else { "theme" };
            shell::saved(app).change(|s| s.mode = mode.to_owned());
            shell::follow_look_tx(app, tx);
        },
    )
}

fn follow_omarchy_rows(
    ctx: PageCtx<'_>,
    follows_omarchy: bool,
    menu_layer: &mut Vec<El>,
) -> [Option<El>; 2] {
    let toggle = switch_row(
        ctx,
        RowSpec::new(
            "omarchy",
            "Follow Omarchy",
            "Use the active Omarchy Quattro theme and switch when the system theme changes.",
        ),
        follows_omarchy,
        |app, on, tx| {
            let mode = if on { "omarchy" } else { "theme" };
            shell::saved(app).change(|s| s.mode = mode.to_owned());
            shell::follow_look_tx(app, tx);
        },
    );
    [
        toggle,
        omarchy_variant_row(ctx, follows_omarchy, menu_layer),
    ]
}

fn omarchy_variant_row(
    ctx: PageCtx<'_>,
    follows_omarchy: bool,
    menu_layer: &mut Vec<El>,
) -> Option<El> {
    let app = ctx.app;
    let slug = shell::themes(app).active_omarchy_slug();
    let current = shell::preferences(app).omarchy_variant(slug);
    let choices = [
        ("Lighter", OmarchyVariant::Lighter),
        ("Darker", OmarchyVariant::Darker),
        ("Contrast", OmarchyVariant::Contrast),
    ];
    let current_label = choices
        .iter()
        .find_map(|&(l, v)| (v == current).then(|| tr(l)))
        .unwrap_or_else(|| tr("Darker"));
    let menu = choice_menu(
        app,
        ChoiceSpec {
            id: "omarchy-variant",
            title: "Omarchy variant",
            current: current_label,
            disabled: !follows_omarchy,
            options: choices
                .into_iter()
                .map(|(label, variant)| {
                    ChoiceOption::new(tr(label), current == variant, move |app, tx| {
                        apply_omarchy_variant(app, variant, tx);
                    })
                })
                .collect(),
        },
        menu_layer,
    );
    option_row(
        ctx,
        RowSpec::new(
            "omarchy-variant",
            "Omarchy variant",
            "Lighter or darker file columns, or stronger contrast. Keeps following your system theme.",
        )
        .indented()
        .disabled(!follows_omarchy),
        menu,
    )
}

fn apply_omarchy_variant(app: &mut App, variant: OmarchyVariant, tx: &mut Tx) {
    let Some(active) = shell::themes(app).active_omarchy_slug().map(str::to_owned) else {
        return;
    };
    let mut changed = false;
    shell::saved(app).mutate(|s| {
        changed = s.set_omarchy_variant(&active, variant);
        changed
    });
    if changed {
        shell::saved(app).commit();
    }
    shell::follow_look_tx(app, tx);
}

fn current_theme_row(
    ctx: PageCtx<'_>,
    (follows_system, follows_omarchy): (bool, bool),
) -> Option<El> {
    let app = ctx.app;
    let resolved = shell::themes(app).resolved_theme(shell::preferences(app));
    let desc = if follows_omarchy {
        "Managed by Omarchy. Turn off Follow Omarchy to pick a theme manually."
    } else if follows_system {
        "Following the system light/dark mode. Choose a theme family from the library below."
    } else {
        "Choose a theme from the library below."
    };
    let preview = row()
        .gap(app.look.space.md)
        .items_center()
        .child(theme_swatch(&app.look, &resolved.colors))
        .child(
            text(resolved.name)
                .font(app.look.kit.ty.ui)
                .color(app.look.kit.c.text_base),
        );
    option_row(
        ctx,
        RowSpec::new("current-theme", "Current theme", desc),
        preview,
    )
}

fn theme_library_section(ctx: PageCtx<'_>) -> Option<El> {
    let app = ctx.app;
    if !ctx.search.shows_row(app, "themes") {
        return None;
    }
    let prefs = shell::preferences(app);
    let themes = shell::themes(app);
    let follows_omarchy = themes.omarchy_available() && prefs.mode == "omarchy";
    let effective_id = themes.resolved_theme(prefs).id;
    let query = app.settings_dialog.theme_query.text.trim().to_lowercase();
    let filter = app.settings_dialog.theme_filter;

    let look = &app.look;
    let divider = look
        .kit
        .alpha(look.kit.c.std_border, look.kit.c.std_border.a * 0.45);
    let cards: Vec<El> = themes
        .library()
        .themes()
        .iter()
        .filter(|theme| theme_matches(theme, filter, &query))
        .enumerate()
        .map(|(idx, theme)| {
            let selected = !follows_omarchy && theme.id == effective_id;
            let card = theme_card(app, theme, (selected, follows_omarchy));
            if idx > 0 {
                col()
                    .w_full()
                    .border_sides([1.0, 0.0, 0.0, 0.0], divider)
                    .child(card)
            } else {
                card
            }
        })
        .collect();
    let scroller = theme_catalog_scroller(app, cards);
    let footer = text(tr("Custom themes live in ~/.config/strata/themes"))
        .font(look.kit.ty.small)
        .color(look.kit.c.text_muted)
        .px(4.0);
    let mut section = col()
        .w_full()
        .pt(24.0)
        .gap(10.0)
        .child(library_toolbar(app))
        .child(scroller)
        .child(footer);
    if follows_omarchy {
        section = section.inert().layer_opacity(0.45);
    }
    ctx.search.row(app, look, "themes", section)
}

fn theme_catalog_scroller(app: &App, cards: Vec<El>) -> El {
    let look = &app.look;
    let theme_bar = &app.settings_dialog.theme_bar;
    let inner = col().w_full().min_w(0.0).pb(look.space.xs).children(cards);
    let viewport = col()
        .key("settings-theme-catalog-scroll")
        .w_full()
        .max_h(336.0)
        .shrink()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .on_scroll_offset(theme_bar.watch())
        .child(inner);
    let radius = look.round(10.0);
    bounded_scroll_with_bar(
        viewport,
        theme_bar,
        (
            "settings-theme-catalog-scrollbar",
            "settings-theme-catalog-scroll",
        ),
        settings_bar_look(look, 3.0, 6.0),
    )
    .radius(radius)
    .clip_soft(radius)
    .fill(card_surface(look))
    .border(1.0, panel_border(look))
}

fn library_toolbar(app: &App) -> El {
    let look = &app.look;
    let search_box = search_field(
        app,
        SearchField {
            key: "settings-theme-search",
            clear_key: "settings-theme-search-clear",
            label: tr("Search themes"),
            edit: |app| &app.settings_dialog.theme_query,
            edit_mut: |app| &mut app.settings_dialog.theme_query,
            width: Some(196.0),
        },
    );
    let current = app.settings_dialog.theme_filter;
    let filters = segmented_toggles(
        look,
        "settings-theme-filter",
        None,
        vec![
            (
                tr("All"),
                current == ThemeFilter::All,
                Rc::new(|app, _| app.settings_dialog.theme_filter = ThemeFilter::All),
            ),
            (
                tr("Light"),
                current == ThemeFilter::Light,
                Rc::new(|app, _| app.settings_dialog.theme_filter = ThemeFilter::Light),
            ),
            (
                tr("Dark"),
                current == ThemeFilter::Dark,
                Rc::new(|app, _| app.settings_dialog.theme_filter = ThemeFilter::Dark),
            ),
        ],
    );
    row()
        .w_full()
        .gap(look.space.md)
        .items_center()
        .child(
            text(tr("THEME LIBRARY"))
                .font(look.ty.heading)
                .color(look.kit.c.text_muted)
                .grow(1.0),
        )
        .child(search_box)
        .child(filters)
}

fn theme_matches(theme: &Theme, filter: ThemeFilter, query: &str) -> bool {
    let light = theme.is_light();
    let mode_ok = match filter {
        ThemeFilter::All => true,
        ThemeFilter::Light => light,
        ThemeFilter::Dark => !light,
    };
    let name_ok = query.is_empty() || theme.name.to_lowercase().contains(query);
    mode_ok && name_ok
}

fn theme_card(app: &App, theme: &Theme, (selected, disabled): (bool, bool)) -> El {
    let look = &app.look;
    let kit = &look.kit;
    let c = &kit.c;
    let id = theme.id.clone();
    let name = theme.name.clone();
    let swatch = theme_swatch(look, &theme.colors);
    let kind_text = tr(if theme.is_light() { "LIGHT" } else { "DARK" });
    let check = look.icon(IconName::Check);
    let fill = if selected {
        kit.alpha(c.bg_accent, 0.10)
    } else {
        Color::TRANSPARENT
    };
    let mut right = row().gap(12.0).items_center().child(
        text(kind_text)
            .font(kit.ty.small)
            .color(c.text_muted)
            .a11y_hidden(),
    );
    if selected {
        right = right.child(icon(check, 14.0).color(c.icon_accent).a11y_hidden());
    }
    let mut card = row()
        .role("button")
        .label(name.clone())
        .checked(selected)
        .w_full()
        .px(18.0)
        .py(14.0)
        .gap(16.0)
        .items_center()
        .fill(fill)
        .focus_ring(2.0, -2.0, c.border_focus)
        .focusable(!disabled)
        .disabled(disabled)
        .child(swatch)
        .child(
            text(name)
                .font(kit.ty.ui_medium)
                .color(c.text_base)
                .grow(1.0)
                .a11y_hidden(),
        )
        .child(right);
    if !disabled {
        card = card
            .cursor(Cursor::Pointer)
            .hover(kit.alpha(c.bg_accent, 0.10))
            .press(c.pressed)
            .on_press_tx(move |app: &mut App, tx: &mut Tx| select_theme_id(app, &id, tx));
    }
    buttons::on_return(format!("settings-theme-card-{}", theme.id), card)
}

fn select_theme_id(app: &mut App, id: &str, tx: &mut Tx) {
    let keep_system =
        shell::themes(app).system_available() && shell::preferences(app).mode == "system";
    let mode = if keep_system { "system" } else { "theme" };
    let chosen = id.to_owned();
    shell::saved(app).change(|s| {
        s.mode = mode.to_owned();
        s.theme = chosen;
    });
    shell::follow_look_tx(app, tx);
}

fn theme_swatch(look: &Look, colors: &Colors) -> El {
    let pill_r = look.round(999.0).min(5.0);
    let pills = [
        (colors.accent, 14.0),
        (colors.danger, 14.0),
        (colors.highlight, 14.0),
        (colors.dim_text, 6.0),
    ]
    .into_iter()
    .map(|(c, w)| el().size(w, 10.0).radius(pill_r).fill(c.color()));
    row()
        .size(80.0, 28.0)
        .px(8.0)
        .gap(4.0)
        .items_center()
        .radius(look.round(6.0))
        .fill(colors.background.color())
        .border(1.0, colors.border.color())
        .a11y_hidden()
        .children(pills)
}

fn rendering_section(ctx: PageCtx<'_>, menu_layer: &mut Vec<El>) -> Option<El> {
    let app = ctx.app;
    let prefs = shell::preferences(app);
    let choices = [
        ("GTK default", InterfaceRenderer::System),
        ("Cairo", InterfaceRenderer::Cairo),
    ];
    let current = choices
        .iter()
        .find_map(|&(l, r)| (r == prefs.interface_renderer).then(|| tr(l)))
        .unwrap_or_else(|| tr("GTK default"));
    let menu = choice_menu(
        app,
        ChoiceSpec {
            id: "interface-renderer",
            title: "Interface renderer",
            current,
            disabled: false,
            options: choices
                .into_iter()
                .map(|(label, renderer)| {
                    ChoiceOption::new(
                        tr(label),
                        prefs.interface_renderer == renderer,
                        move |app, _| shell::saved(app).change(|s| s.interface_renderer = renderer),
                    )
                })
                .collect(),
        },
        menu_layer,
    );
    let mut controls = col().gap(app.look.space.xs).items_end().child(menu);
    if shell::interface_renderer_restart_required(app) {
        let restart = Button::new(tr("Restart now"))
            .variant(ButtonVariant::Submit)
            .on_press_tx(super::restart_process)
            .build(&app.look.kit)
            .label(tr("Restart to apply interface renderer"));
        controls = controls.child(buttons::on_return("settings-renderer-restart", restart));
    }
    group_card(
        &app.look,
        vec![option_row(
            ctx,
            RowSpec::new(
                "renderer",
                "Interface renderer",
                "Cairo avoids text artifacts on some displays. Restart to apply changes; GSK_RENDERER overrides this choice.",
            ),
            controls,
        )],
    )
}

fn text_section(ctx: PageCtx<'_>, menu_layer: &mut Vec<El>) -> Option<El> {
    let app = ctx.app;
    let font_menu = interface_font_menu(app, menu_layer);
    group_card(
        &app.look,
        vec![
            option_row(
                ctx,
                RowSpec::new(
                    "interface-font",
                    "Interface font",
                    "The typeface for menus, file lists, and dialogs. File previews keep a monospace font.",
                ),
                font_menu,
            ),
            option_row(
                ctx,
                RowSpec::new(
                    "text",
                    "Text size",
                    "Logical pixels, 8–48. Display scaling applies on top.",
                ),
                text_size_field().build(app, &app.look),
            ),
        ],
    )
}

fn interface_font_menu(app: &App, menu_layer: &mut Vec<El>) -> El {
    let current_font = shell::preferences(app).interface_font.as_deref();
    let current_label = current_font.map_or_else(|| tr("Default"), str::to_owned);
    let mut families: Vec<Option<String>> = vec![None, Some(FAMILY.to_owned())];
    for candidate in [
        "Inter",
        "Cantarell",
        "Noto Sans",
        "DejaVu Sans",
        "Liberation Sans",
    ] {
        families.push(Some(candidate.to_owned()));
    }
    if let Some(custom) = current_font
        && !families.iter().any(|f| f.as_deref() == Some(custom))
    {
        families.insert(1, Some(custom.to_owned()));
    }
    choice_menu(
        app,
        ChoiceSpec {
            id: "interface-font",
            title: "Interface font",
            current: current_label,
            disabled: false,
            options: families
                .into_iter()
                .map(|family| {
                    let label = family.clone().unwrap_or_else(|| tr("Default"));
                    let selected = current_font == family.as_deref();
                    ChoiceOption::new(label, selected, move |app, tx| {
                        let next = normalized_font_family(family.clone());
                        shell::saved(app).change(|s| s.interface_font = next);
                        shell::follow_look_tx(app, tx);
                    })
                })
                .collect(),
        },
        menu_layer,
    )
}

fn shape_section(ctx: PageCtx<'_>) -> Option<El> {
    let app = ctx.app;
    group_card(
        &app.look,
        vec![option_row(
            ctx,
            RowSpec::new(
                "roundness",
                "Corner roundness",
                "Percent of the default corner radius, 0–200. 0 squares every corner.",
            ),
            roundness_field().build(app, &app.look),
        )],
    )
}

fn file_icons_section(ctx: PageCtx<'_>, menu_layer: &mut Vec<El>) -> Option<El> {
    let app = ctx.app;
    let prefs = shell::preferences(app);
    let menu = file_icon_colors_menu(app, menu_layer);
    group_card(
        &app.look,
        vec![
            option_row(
                ctx,
                RowSpec::new(
                    "file-icon-colors",
                    "File icon colors",
                    "Brand colors use each file type's own palette; theme colors use the accent.",
                ),
                menu,
            ),
            switch_row(
                ctx,
                RowSpec::new(
                    "show-thumbnails",
                    "Show thumbnails",
                    "Show thumbnails instead of file-type icons. The preview panel stays available.",
                ),
                prefs.show_thumbnails,
                |app, on, _| {
                    shell::saved(app).change(|s| s.show_thumbnails = on);
                    let _ = app.window.thumbnails_mut().set_enabled(on);
                },
            ),
            switch_row(
                ctx,
                RowSpec::new(
                    "stl-thumbnails",
                    "STL model thumbnails",
                    "Render local STL models as file thumbnails, one at a time. Requires Show thumbnails.",
                ),
                prefs.stl_thumbnails,
                |app, on, _| {
                    shell::saved(app).change(|s| s.stl_thumbnails = on);
                    shell::refresh_worn(app);
                },
            ),
        ],
    )
}

fn file_icon_colors_menu(app: &App, menu_layer: &mut Vec<El>) -> El {
    let prefs = shell::preferences(app);
    let choices = [
        ("Brand colors", FileIconColors::Brand),
        ("Theme colors", FileIconColors::Theme),
    ];
    let current = choices
        .iter()
        .find_map(|&(l, c)| (c == prefs.file_icon_colors).then(|| tr(l)))
        .unwrap_or_else(|| tr("Brand colors"));
    choice_menu(
        app,
        ChoiceSpec {
            id: "file-icon-colors",
            title: "File icon colors",
            current,
            disabled: false,
            options: choices
                .into_iter()
                .map(|(label, colors)| {
                    ChoiceOption::new(
                        tr(label),
                        prefs.file_icon_colors == colors,
                        move |app, tx| {
                            shell::saved(app).change(|s| s.file_icon_colors = colors);
                            shell::follow_look_tx(app, tx);
                        },
                    )
                })
                .collect(),
        },
        menu_layer,
    )
}

fn look_switch_card(
    ctx: PageCtx<'_>,
    spec: RowSpec<'_>,
    checked: bool,
    write: fn(&mut Preferences, bool),
) -> Option<El> {
    group_card(
        &ctx.app.look,
        vec![switch_row(ctx, spec, checked, move |app, on, tx| {
            shell::saved(app).change(|s| write(s, on));
            shell::follow_look_tx(app, tx);
        })],
    )
}

pub(super) fn sync_appearance_fields(
    text_size: &mut NumberFieldState,
    roundness: &mut NumberFieldState,
    prefs: &Preferences,
) {
    if !text_size.edit().focused {
        text_size.set_value(f64::from(prefs.text_size.root_font_px()));
    }
    if !roundness.edit().focused {
        roundness.set_value(f64::from(prefs.corner_roundness.percent()));
    }
}
