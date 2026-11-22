// SPDX-License-Identifier: MIT

//! The General settings page (`src/ui/settings/general.rs`): language, browsing, search,
//! sidebar, and item-opening sections.

use std::rc::Rc;

use rune::{Cursor, El, Tx, col, icon, row, text};
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant};
use strata_components::controls::{
    number_field::{NumberField, NumberFieldState},
    settings_search::SettingsSearch,
};
use strata_core::{
    i18n::{Language, tr},
    model::{SortDirection, SortKey},
};
use strata_desktop::settings::{
    Preferences, SEARCH_ENTRY_LIMIT_CHOICES, SEARCH_TIME_LIMIT_CHOICES, TenxerFx, UnitSystem,
    sort_preferences,
};
use strata_kit::icons::IconName;

use super::{
    controls::{
        ChoiceOption, ChoiceSpec, NamedSegment, PageCtx, RowSpec, choice_menu,
        first_section_heading, group_card, mix_srgb, number_field, option_row, panel_border,
        pref_switch, section_heading, segmented_toggles, segmented_toggles_named, switch_row,
    },
    general_more,
};
use crate::{App, buttons, folder_views, shell, sidebar};

pub(super) fn video_start_field() -> NumberField<App> {
    number_field(
        "settings-video-start",
        "Video preview start",
        (
            |app: &App| &app.settings_dialog.video_start,
            |app: &mut App| &mut app.settings_dialog.video_start,
        ),
        |app, value, _| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clamped 0..=99"
            )]
            let percent = value.round() as u32;
            shell::saved(app).change(|prefs| prefs.preview_video_start = percent);
        },
    )
}

pub(super) fn build(app: &App, search: &SettingsSearch<App>, menu_layer: &mut Vec<El>) -> Vec<El> {
    let ctx = PageCtx { app, search };
    let mut blocks: Vec<Option<El>> = vec![
        first_section_heading(ctx, "language-units", "LANGUAGE & UNITS"),
        language_and_units(ctx, menu_layer),
        section_heading(ctx, "browsing", "BROWSING"),
        browsing_section(ctx, menu_layer),
        section_heading(ctx, "search", "SEARCH & FILTERING"),
        search_section(ctx, menu_layer),
        section_heading(ctx, "sidebar", "SIDEBAR"),
        sidebar_section(ctx),
        section_heading(ctx, "opening", "OPENING ITEMS"),
        opening_section(ctx),
    ];
    blocks.extend(general_more::build(ctx, menu_layer));
    blocks.into_iter().flatten().collect()
}

fn language_and_units(ctx: PageCtx<'_>, menu_layer: &mut Vec<El>) -> Option<El> {
    let app = ctx.app;
    let prefs = shell::preferences(app);
    let lang_col = language_controls(app, prefs.language, menu_layer);
    let units = units_control(app, prefs.unit_system);
    group_card(
        &app.look,
        vec![
            option_row(
                ctx,
                RowSpec::new(
                    "language",
                    "Language",
                    "Auto-detect follows your system language. Restart Strata to apply changes.",
                ),
                lang_col,
            ),
            option_row(
                ctx,
                RowSpec::new(
                    "units",
                    "Units",
                    "For things like distances, heights and volumes.",
                ),
                units,
            ),
        ],
    )
}

fn language_controls(app: &App, current: Language, menu_layer: &mut Vec<El>) -> El {
    let current_lang = Language::CHOICES
        .iter()
        .find_map(|(label, lang)| (*lang == current).then(|| tr(label)))
        .unwrap_or_else(|| tr("Auto-detect"));
    let options = Language::CHOICES
        .iter()
        .map(|&(label, lang)| {
            ChoiceOption::new(tr(label), current == lang, move |app, _| {
                shell::saved(app).change(|saved| saved.language = lang);
            })
        })
        .collect();
    let menu = choice_menu(
        app,
        ChoiceSpec {
            id: "language",
            title: "Language",
            current: current_lang,
            disabled: false,
            options,
        },
        menu_layer,
    );
    let mut lang_col = col().gap(app.look.space.xs).items_end().child(menu);
    if shell::language_restart_required(app) {
        let restart = Button::new(tr("Restart now"))
            .variant(ButtonVariant::Submit)
            .on_press_tx(super::restart_process)
            .build(&app.look.kit);
        lang_col = lang_col.child(buttons::on_return("settings-language-restart", restart));
    }
    lang_col
}

fn units_control(app: &App, unit_system: UnitSystem) -> El {
    segmented_toggles(
        &app.look,
        "settings-units",
        Some(tr("Units")),
        vec![
            (
                tr("Metric"),
                unit_system == UnitSystem::Metric,
                Rc::new(|app, _| shell::saved(app).change(|s| s.unit_system = UnitSystem::Metric)),
            ),
            (
                tr("Imperial"),
                unit_system == UnitSystem::Imperial,
                Rc::new(|app, _| {
                    shell::saved(app).change(|s| s.unit_system = UnitSystem::Imperial);
                }),
            ),
        ],
    )
    .min_w(220.0)
}

type BoolSwitchDef = (
    &'static str,
    &'static str,
    &'static str,
    fn(&Preferences) -> bool,
    fn(&mut Preferences, bool),
);

const TENXER_UNUSED: &str = "Not used in 10xer mode.";
const TENXER_EXPERIMENTAL: &str = "(experimental feature, under active development)";

const BROWSING_SWITCHES: [BoolSwitchDef; 5] = [
    (
        "peeking",
        "Folder peeking",
        "Preview folders on hover in Icons and List views.",
        |s| s.folder_peeking,
        |s, v| s.folder_peeking = v,
    ),
    (
        "previews",
        "Single-click file previews",
        "Show a quick preview when selecting a supported file.",
        |s| s.single_click_previews,
        |s, v| s.single_click_previews = v,
    ),
    (
        "preview-autoplay",
        "Autoplay media previews",
        "Start playing video, audio, and GIF previews as soon as they open.",
        |s| s.preview_autoplay,
        |s, v| s.preview_autoplay = v,
    ),
    (
        "render-documents",
        "Render documents by default",
        "Open Markdown and HTML previews in the rendered view instead of source.",
        |s| s.render_documents_by_default,
        |s, v| s.render_documents_by_default = v,
    ),
    (
        "arrow-scope",
        "Keep arrows in file list",
        "Stop arrow keys from leaving the file list. Use %{sidebar_focus_keys} to focus the sidebar, or use the mouse.",
        |s| s.arrow_navigation_scoped,
        |s, v| s.arrow_navigation_scoped = v,
    ),
];

fn browsing_section(ctx: PageCtx<'_>, menu_layer: &mut Vec<El>) -> Option<El> {
    let app = ctx.app;
    let prefs = shell::preferences(app);
    let simple = |idx: usize| {
        let (k, t, d, read, write) = BROWSING_SWITCHES[idx];
        let desc = if prefs.tenxer_mode && k == "arrow-scope" {
            TENXER_UNUSED
        } else {
            d
        };
        pref_switch(ctx, RowSpec::new(k, t, desc), read(prefs), write)
    };
    let mut rows = vec![
        simple(0),
        simple(1),
        simple(2),
        option_row(
            ctx,
            RowSpec::new(
                "preview-video-start",
                "Video preview start",
                "Percent of a video's length to open it at; 0 starts at the beginning. A video you already previewed resumes where you left it.",
            ),
            video_start_field().build(app, &app.look),
        ),
        simple(3),
        simple(4),
    ];
    rows.extend(columns_and_hints_rows(ctx, menu_layer));
    rows.extend(folder_view_rows(ctx, menu_layer));
    group_card(&app.look, rows)
}

fn columns_and_hints_rows(ctx: PageCtx<'_>, menu_layer: &mut Vec<El>) -> Vec<Option<El>> {
    let prefs = shell::preferences(ctx.app);
    vec![
        switch_row(
            ctx,
            RowSpec::new(
                "columns-sizes",
                "Show sizes in Columns view",
                "Show file sizes and folder item counts when hovering or selecting items in Columns view.",
            ),
            prefs.columns_show_sizes,
            |app, on, _| {
                shell::saved(app).change(|s| s.columns_show_sizes = on);
                app.columns.show_sizes(on);
            },
        ),
        switch_row(
            ctx,
            RowSpec::new(
                "columns-mirror",
                "Mirror columns selection",
                "Show the selected folder's contents in the next pane as you move with the keyboard.",
            ),
            prefs.columns_mirror_selection,
            |app, on, _| {
                shell::saved(app).change(|s| s.columns_mirror_selection = on);
                app.columns
                    .refresh_mirror_preferences(shell::preferences(app));
            },
        ),
        switch_row(
            ctx,
            RowSpec::new(BROWSING_TENXER.0, BROWSING_TENXER.1, BROWSING_TENXER.2)
                .note(prefs.tenxer_mode.then_some(TENXER_EXPERIMENTAL)),
            prefs.tenxer_mode,
            |app, on, tx| {
                shell::saved(app).change(|s| s.tenxer_mode = on);
                shell::live_keys(app).follow(shell::preferences(app));
                if on {
                    crate::tenxer_fx::trigger_current(app, tx);
                } else {
                    crate::tenxer::end_session(app, tx);
                    crate::tenxer_fx::cancel(app);
                }
            },
        ),
        tenxer_fx_row(ctx, menu_layer),
        pref_switch(
            ctx,
            RowSpec::new(
                "hints",
                "Show %{reference_keys} Shortcuts button",
                "Show the shortcuts button in the bottom bar. Item counts and clipboard status remain visible when the button is hidden. %{reference_keys} always opens the full reference.",
            ),
            prefs.show_keybinding_hints,
            |s, v| s.show_keybinding_hints = v,
        ),
    ]
}

fn tenxer_fx_row(ctx: PageCtx<'_>, menu_layer: &mut Vec<El>) -> Option<El> {
    let app = ctx.app;
    let current = shell::tenxer_fx(app);
    let options = TenxerFx::ALL
        .into_iter()
        .map(|effect| {
            ChoiceOption::new(tr(effect.label()), current == effect, move |app, tx| {
                shell::saved(app).set_tenxer_fx(effect);
                if effect.is_enabled() {
                    crate::tenxer_fx::trigger(app, effect, tx);
                } else {
                    crate::tenxer_fx::cancel(app);
                }
            })
        })
        .collect();
    let menu = choice_menu(
        app,
        ChoiceSpec {
            id: "tenxer-fx",
            title: "10x mode effect",
            current: tr(current.label()),
            disabled: false,
            options,
        },
        menu_layer,
    );
    let can_preview = current.is_enabled() && app.look.animations;
    let preview = buttons::on_return(
        "settings-tenxer-fx-preview",
        IconButton::new(app.look.icon(IconName::Play))
            .variant(IconButtonVariant::Ghost)
            .label(tr("Preview effect"))
            .disabled(!can_preview)
            .on_press_tx(move |app: &mut App, tx: &mut Tx| {
                crate::tenxer_fx::trigger(app, current, tx);
            })
            .build(&app.look.accent_icon_kit()),
    );
    let control = row()
        .gap(app.look.space.sm)
        .items_center()
        .child(preview)
        .child(menu);
    option_row(
        ctx,
        RowSpec::new(
            "tenxer-fx",
            "10x mode effect",
            "Visual effect played when entering 10xer mode.",
        )
        .indented(),
        control,
    )
}

const BROWSING_TENXER: (&str, &str, &str) = (
    "tenxer",
    "10xer mode",
    "Modal keyboard navigation with single-key actions and prompt bars (%{tenxer_mode_keys}).",
);

fn folder_view_rows(ctx: PageCtx<'_>, menu_layer: &mut Vec<El>) -> Vec<Option<El>> {
    let app = ctx.app;
    let prefs = shell::preferences(app);
    let key_menu = default_sort_key_menu(app, menu_layer);
    let dir_menu = default_sort_dir_menu(app, menu_layer);
    let clear = Button::new(tr("Clear"))
        .variant(ButtonVariant::Neutral)
        .disabled(!folder_views::has_saved(app))
        .on_press_tx(folder_views::forget_all)
        .build(&app.look.kit)
        .label(tr("Forget all folder settings"));
    vec![
        switch_row(
            ctx,
            RowSpec::new(
                "folder-views",
                "Remember sort and icon size per folder",
                "Each folder keeps the sort and thumbnail size you choose there. Other folders use the defaults.",
            ),
            prefs.remember_folder_views,
            folder_views::set_remember,
        ),
        option_row(
            ctx,
            RowSpec::new(
                "default-sort",
                "Default sort",
                "Folders without their own sort use this order.",
            ),
            row().gap(app.look.space.sm).child(key_menu).child(dir_menu),
        ),
        option_row(
            ctx,
            RowSpec::new(
                "forget-folder-views",
                "Forget all folder settings",
                "Return every folder to the default sort and thumbnail size.",
            ),
            buttons::on_return("settings-forget-folder-views", clear),
        ),
    ]
}

fn default_sort_key_menu(app: &App, menu_layer: &mut Vec<El>) -> El {
    let (current_key, _, _) = sort_preferences(shell::preferences(app)).sort_options();
    let choices = [
        ("Name", SortKey::Name),
        ("Size", SortKey::Size),
        ("Modified", SortKey::Modified),
        ("Type", SortKey::Type),
        ("Extension", SortKey::Extension),
        ("Created", SortKey::Created),
        ("Unsorted", SortKey::Unsorted),
    ];
    let current = choices
        .iter()
        .find_map(|&(l, k)| (k == current_key).then(|| tr(l)))
        .unwrap_or_else(|| tr("Name"));
    choice_menu(
        app,
        ChoiceSpec {
            id: "default-sort-key",
            title: "Default sort",
            current,
            disabled: false,
            options: choices
                .into_iter()
                .map(|(label, key)| {
                    ChoiceOption::new(tr(label), current_key == key, move |app, tx| {
                        let (_, dir, cmp) =
                            sort_preferences(shell::preferences(app)).sort_options();
                        folder_views::set_default_sort(app, (key, dir, cmp), tx);
                    })
                })
                .collect(),
        },
        menu_layer,
    )
}

fn default_sort_dir_menu(app: &App, menu_layer: &mut Vec<El>) -> El {
    let (current_key, current_dir, comparison) =
        sort_preferences(shell::preferences(app)).sort_options();
    let dir_label = if current_dir == SortDirection::Descending {
        tr("Descending")
    } else {
        tr("Ascending")
    };
    let choices = [
        ("Ascending", SortDirection::Ascending),
        ("Descending", SortDirection::Descending),
    ];
    choice_menu(
        app,
        ChoiceSpec {
            id: "default-sort-dir",
            title: "Default sort direction",
            current: dir_label,
            disabled: current_key == SortKey::Unsorted,
            options: choices
                .into_iter()
                .map(|(label, dir)| {
                    ChoiceOption::new(tr(label), current_dir == dir, move |app, tx| {
                        let (key, _, _) = sort_preferences(shell::preferences(app)).sort_options();
                        folder_views::set_default_sort(app, (key, dir, comparison), tx);
                    })
                })
                .collect(),
        },
        menu_layer,
    )
}

struct LimitMenuSpec {
    id: &'static str,
    title: &'static str,
    current_val: u32,
    choices: &'static [(&'static str, u32)],
    write: fn(&mut Preferences, u32),
}

fn search_section(ctx: PageCtx<'_>, menu_layer: &mut Vec<El>) -> Option<El> {
    let app = ctx.app;
    let prefs = shell::preferences(app);
    let type_desc = if prefs.tenxer_mode {
        TENXER_UNUSED
    } else {
        "Start filtering the active pane as soon as you type."
    };
    let sub_desc = if prefs.tenxer_mode {
        TENXER_UNUSED
    } else {
        "Also match items inside nested folders."
    };
    let mut rows = vec![
        pref_switch(
            ctx,
            RowSpec::new("type-search", "Type to search", type_desc),
            prefs.type_to_search,
            |s, v| s.type_to_search = v,
        ),
        pref_switch(
            ctx,
            RowSpec::new("subfolders", "Include subfolders", sub_desc).indented(),
            prefs.filter_include_subfolders,
            |s, v| s.filter_include_subfolders = v,
        ),
        pref_switch(
            ctx,
            RowSpec::new(
                "search-results",
                "Open search results directly",
                "Launch files from search instead of showing the quick preview.",
            ),
            prefs.search_open_files_directly,
            |s, v| s.search_open_files_directly = v,
        ),
    ];
    rows.extend(search_limit_rows(ctx, menu_layer));
    group_card(&app.look, rows)
}

fn search_limit_rows(ctx: PageCtx<'_>, menu_layer: &mut Vec<El>) -> Vec<Option<El>> {
    let app = ctx.app;
    let prefs = shell::preferences(app);
    let time_menu = limit_choice_menu(
        app,
        LimitMenuSpec {
            id: "search-time-limit",
            title: "Search time limit",
            current_val: prefs.search_time_limit,
            choices: &SEARCH_TIME_LIMIT_CHOICES,
            write: |s, v| s.search_time_limit = v,
        },
        menu_layer,
    );
    let entry_menu = limit_choice_menu(
        app,
        LimitMenuSpec {
            id: "search-entry-limit",
            title: "Search entry limit",
            current_val: prefs.search_entry_limit,
            choices: &SEARCH_ENTRY_LIMIT_CHOICES,
            write: |s, v| s.search_entry_limit = v,
        },
        menu_layer,
    );
    let exclusions_summary = if prefs.search_exclusions.is_empty() {
        tr("Default")
    } else {
        prefs.search_exclusions.join(", ")
    };
    let exclusions = text(exclusions_summary)
        .font(app.look.kit.ty.small)
        .color(app.look.kit.c.text_muted);
    vec![
        option_row(
            ctx,
            RowSpec::new(
                "search-time-limit",
                "Search time limit",
                "Stop indexing a search after this long. Slow or network mounts need longer to list every folder.",
            ),
            time_menu,
        ),
        option_row(
            ctx,
            RowSpec::new(
                "search-entry-limit",
                "Search entry limit",
                "Stop indexing a search after this many files and folders. Higher limits use more memory.",
            ),
            entry_menu,
        ),
        option_row(
            ctx,
            RowSpec::new(
                "search-exclusions",
                "Global search exclusions",
                "Exclude folder names or directory paths from global search.",
            ),
            exclusions,
        ),
    ]
}

fn limit_choice_menu(app: &App, spec: LimitMenuSpec, menu_layer: &mut Vec<El>) -> El {
    let current = spec
        .choices
        .iter()
        .find_map(|&(l, v)| (v == spec.current_val).then(|| tr(l)))
        .unwrap_or_else(|| tr(spec.choices[0].0));
    let write = spec.write;
    let options = spec
        .choices
        .iter()
        .map(|&(label, val)| {
            ChoiceOption::new(tr(label), spec.current_val == val, move |app, _| {
                shell::saved(app).change(|prefs| write(prefs, val));
            })
        })
        .collect();
    choice_menu(
        app,
        ChoiceSpec {
            id: spec.id,
            title: spec.title,
            current,
            disabled: false,
            options,
        },
        menu_layer,
    )
}

type PlaceToggle = (
    &'static str,
    &'static str,
    &'static str,
    IconName,
    fn(&Preferences) -> bool,
    fn(&mut Preferences, bool),
);

const SIDEBAR_PLACES: [PlaceToggle; 10] = [
    (
        "Home",
        "Show Home in sidebar",
        "Show the Home folder in the sidebar.",
        IconName::Home,
        |s| s.sidebar_show_home,
        |s, v| s.sidebar_show_home = v,
    ),
    (
        "Trash",
        "Show Trash in sidebar",
        "Show Trash in the sidebar.",
        IconName::Trash,
        |s| s.sidebar_show_trash,
        |s, v| s.sidebar_show_trash = v,
    ),
    (
        "Network",
        "Show Network in sidebar",
        "Show Network in the sidebar.",
        IconName::Network,
        |s| s.sidebar_show_network,
        |s, v| s.sidebar_show_network = v,
    ),
    (
        "Recent",
        "Show Recent in sidebar",
        "Show Recent files in the sidebar.",
        IconName::Recent,
        |s| s.sidebar_show_recent,
        |s, v| s.sidebar_show_recent = v,
    ),
    (
        "Desktop",
        "Show Desktop in sidebar",
        "Show the Desktop folder in the sidebar.",
        IconName::Desktop,
        |s| s.sidebar_show_desktop,
        |s, v| s.sidebar_show_desktop = v,
    ),
    (
        "Documents",
        "Show Documents in sidebar",
        "Show the Documents folder in the sidebar.",
        IconName::Documents,
        |s| s.sidebar_show_documents,
        |s, v| s.sidebar_show_documents = v,
    ),
    (
        "Downloads",
        "Show Downloads in sidebar",
        "Show the Downloads folder in the sidebar.",
        IconName::Downloads,
        |s| s.sidebar_show_downloads,
        |s, v| s.sidebar_show_downloads = v,
    ),
    (
        "Music",
        "Show Music in sidebar",
        "Show the Music folder in the sidebar.",
        IconName::Music,
        |s| s.sidebar_show_music,
        |s, v| s.sidebar_show_music = v,
    ),
    (
        "Pictures",
        "Show Pictures in sidebar",
        "Show the Pictures folder in the sidebar.",
        IconName::Pictures,
        |s| s.sidebar_show_pictures,
        |s, v| s.sidebar_show_pictures = v,
    ),
    (
        "Videos",
        "Show Videos in sidebar",
        "Show the Videos folder in the sidebar.",
        IconName::Videos,
        |s| s.sidebar_show_videos,
        |s, v| s.sidebar_show_videos = v,
    ),
];

fn sidebar_section(ctx: PageCtx<'_>) -> Option<El> {
    let app = ctx.app;
    if !ctx.search.shows_row(app, "sidebar-places") {
        return None;
    }
    let chips: Vec<El> = SIDEBAR_PLACES
        .into_iter()
        .enumerate()
        .map(|(idx, place)| sidebar_place_chip(app, idx, place))
        .collect();
    let kit = &app.look.kit;
    let content = col()
        .w_full()
        .min_w(0.0)
        .px(18.0)
        .py(16.0)
        .gap(app.look.space.md)
        .child(
            col()
                .gap(3.0)
                .child(ctx.search.title(
                    app,
                    &app.look,
                    &tr("Items shown in sidebar"),
                    kit.ty.ui_medium,
                ))
                .child(
                    text(tr("Toggle which locations appear in the sidebar."))
                        .font(kit.ty.small)
                        .color(kit.c.text_muted)
                        .wrap(),
                ),
        )
        .child(
            row()
                .w_full()
                .min_w(0.0)
                .flex_wrap()
                .gap(app.look.space.sm)
                .children(chips),
        );
    let wrapped = ctx.search.row(app, &app.look, "sidebar-places", content);
    group_card(&app.look, vec![wrapped])
}

fn sidebar_place_chip(app: &App, idx: usize, place: PlaceToggle) -> El {
    let (short, full, desc, glyph_name, read, write) = place;
    let on = read(shell::preferences(app));
    let look = &app.look;
    let kit = &look.kit;
    let c = &kit.c;
    let chip_bg = look.transparency.overlay_color(c.bg_base);
    let (fill, border, ink, icon_color) = if on {
        (
            mix_srgb(chip_bg, c.text_base, 0.12),
            kit.alpha(c.text_base, 0.40),
            c.text_base,
            c.icon_accent,
        )
    } else {
        (
            chip_bg,
            panel_border(look),
            c.text_muted,
            kit.alpha(c.icon_accent, 0.65),
        )
    };
    let pill_radius = look.round(999.0).min(14.0);
    let chip = row()
        .role("button")
        .label(tr(full))
        .description(tr(desc))
        .checked(on)
        .min_h(28.0)
        .px(12.0)
        .py(4.0)
        .gap(6.0)
        .items_center()
        .radius(pill_radius)
        .fill(fill)
        .border(1.0, border)
        .hover(look.fill.highlight)
        .press(mix_srgb(c.bg_base, c.text_base, 0.20))
        .focusable(true)
        .focus_ring(2.0, 2.0, c.border_focus)
        .cursor(Cursor::Pointer)
        .child(
            icon(look.icon(glyph_name), 14.0)
                .color(icon_color)
                .a11y_hidden(),
        )
        .child(text(tr(short)).font(kit.ty.ui).color(ink).a11y_hidden())
        .on_press_tx(move |app: &mut App, _: &mut Tx| {
            shell::saved(app).change(|s| write(s, !on));
            sidebar::relist_if_started(app);
        });
    buttons::on_return(format!("settings-sidebar-chip-{idx}"), chip)
}

type ClickSlot = (u8, fn(&mut Preferences, u8));
type OpeningModeRow = (&'static str, &'static str, ClickSlot, ClickSlot);

fn opening_section(ctx: PageCtx<'_>) -> Option<El> {
    let app = ctx.app;
    if !ctx.search.shows_row(app, "opening") {
        return None;
    }
    let prefs = shell::preferences(app);
    let modes: [OpeningModeRow; 3] = [
        (
            "columns",
            "Columns view",
            (prefs.columns_file_clicks, |s, n| s.columns_file_clicks = n),
            (prefs.columns_folder_clicks, |s, n| {
                s.columns_folder_clicks = n
            }),
        ),
        (
            "icons",
            "Icons view",
            (prefs.icons_file_clicks, |s, n| s.icons_file_clicks = n),
            (prefs.icons_folder_clicks, |s, n| s.icons_folder_clicks = n),
        ),
        (
            "list",
            "List view",
            (prefs.list_file_clicks, |s, n| s.list_file_clicks = n),
            (prefs.list_folder_clicks, |s, n| s.list_folder_clicks = n),
        ),
    ];
    let rows: Vec<Option<El>> = modes
        .into_iter()
        .map(|(slug, title, files, folders)| {
            Some(opening_mode_row(app, slug, title, (files, folders)))
        })
        .collect();
    let card = group_card(&app.look, rows)?;
    ctx.search.row(app, &app.look, "opening", card)
}

fn opening_mode_row(
    app: &App,
    slug: &str,
    title: &str,
    (files, folders): (ClickSlot, ClickSlot),
) -> El {
    let view_name = tr(title);
    let files_ctl = click_pair_control(
        app,
        &format!("settings-clicks-{slug}-files"),
        &format!("{view_name} {}", tr("Files")),
        files,
    );
    let folders_ctl = click_pair_control(
        app,
        &format!("settings-clicks-{slug}-folders"),
        &format!("{view_name} {}", tr("Folders")),
        folders,
    );
    row()
        .w_full()
        .px(18.0)
        .py(16.0)
        .gap(app.look.space.xl)
        .items_center()
        .child(
            text(view_name)
                .font(app.look.kit.ty.ui_medium)
                .color(app.look.kit.c.text_base)
                .grow(1.0),
        )
        .child(files_ctl)
        .child(folders_ctl)
}

fn click_pair_control(app: &App, key: &str, a11y_prefix: &str, (current, write): ClickSlot) -> El {
    let choices = [("Single", 1_u8), ("Double", 2_u8)]
        .into_iter()
        .map(|(choice, count)| {
            let active = if count == 1 {
                current == 1
            } else {
                current != 1
            };
            let label = tr(choice);
            let a11y_name = format!("{a11y_prefix} {label}");
            NamedSegment {
                visible: label,
                accessible: a11y_name,
                active,
                on_pick: Rc::new(move |app: &mut App, _: &mut Tx| {
                    shell::saved(app).change(|s| write(s, count));
                }),
            }
        })
        .collect();
    segmented_toggles_named(&app.look, key, None, choices)
}

pub(super) fn sync_general_fields(
    workers: &mut NumberFieldState,
    video_start: &mut NumberFieldState,
    prefs: &Preferences,
) {
    if !workers.edit().focused {
        workers.set_value(prefs.thumbnail_workers as f64);
    }
    if !video_start.edit().focused {
        video_start.set_value(f64::from(prefs.preview_video_start));
    }
}
