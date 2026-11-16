// SPDX-License-Identifier: MIT

//! Transfers, date & time, performance, window buttons, and startup sections of General settings.

use std::path::Path;

use rune::{El, Tx, row, text};
use rune_kit::{Button, ButtonVariant};
use strata_components::controls::number_field::NumberField;
use strata_core::{
    i18n::tr,
    util::{DateFormat, modified_date_example, set_date_format},
};
use strata_desktop::settings::{
    configured_hardware_acceleration, configured_video_preview_backend,
};
use strata_fs::services::CrossVolumeDropStrategy;
use strata_preview::sandbox::{MediaPreviewBackend, browser, polaris_gpu_available};

use super::controls::{
    ChoiceOption, ChoiceSpec, PageCtx, RowSpec, choice_menu, group_card, number_field, option_row,
    pref_switch, section_heading, switch_row,
};
use crate::{App, buttons, launch, shell};

pub(super) fn workers_field() -> NumberField<App> {
    number_field(
        "settings-thumbnail-workers",
        "Thumbnail workers",
        (
            |app: &App| &app.settings_dialog.workers,
            |app: &mut App| &mut app.settings_dialog.workers,
        ),
        |app, value, _| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clamped 1..=16"
            )]
            let count = value.round() as usize;
            shell::saved(app).change(|prefs| prefs.thumbnail_workers = count);
            browser::set_worker_limit(count);
        },
    )
}

pub(super) fn build(ctx: PageCtx<'_>, menu_layer: &mut Vec<El>) -> Vec<Option<El>> {
    vec![
        section_heading(ctx, "transfers", "FILE TRANSFERS"),
        transfers_section(ctx, menu_layer),
        section_heading(ctx, "date-time", "DATE & TIME"),
        date_time_section(ctx, menu_layer),
        section_heading(ctx, "performance", "PERFORMANCE"),
        performance_section(ctx, menu_layer),
        section_heading(ctx, "window-buttons", "WINDOW BUTTONS"),
        window_buttons_section(ctx),
        section_heading(ctx, "startup", "STARTUP"),
        startup_section(ctx),
    ]
}

fn transfers_section(ctx: PageCtx<'_>, menu_layer: &mut Vec<El>) -> Option<El> {
    let app = ctx.app;
    let prefs = shell::preferences(app);
    let current = CrossVolumeDropStrategy::parse(&prefs.cross_volume_drop_strategy);
    let choices = [
        ("Always copy", CrossVolumeDropStrategy::Copy),
        ("Always move", CrossVolumeDropStrategy::Move),
        ("Always ask", CrossVolumeDropStrategy::Ask),
    ];
    let current_label = choices
        .iter()
        .find_map(|&(l, s)| (s == current).then(|| tr(l)))
        .unwrap_or_else(|| tr("Always ask"));
    let menu = choice_menu(
        app,
        ChoiceSpec {
            id: "cross-volume-drop",
            title: "Drag & drop to another device",
            current: current_label,
            disabled: false,
            options: choices
                .into_iter()
                .map(|(label, strategy)| {
                    ChoiceOption::new(tr(label), current == strategy, move |app, _| {
                        shell::saved(app).change(|s| {
                            s.cross_volume_drop_strategy = strategy.as_str().to_owned();
                        });
                    })
                })
                .collect(),
        },
        menu_layer,
    );
    group_card(
        &app.look,
        vec![
            option_row(
                ctx,
                RowSpec::new(
                    "transfers",
                    "Drag & drop to another device",
                    "What happens when you drop items onto a different drive or share.",
                ),
                menu,
            ),
            pref_switch(
                ctx,
                RowSpec::new(
                    "open-after-drop",
                    "Open folder after dropping files",
                    "Show the destination folder after a successful drag and drop.",
                ),
                prefs.open_folder_after_drop,
                |s, v| s.open_folder_after_drop = v,
            ),
        ],
    )
}

fn date_time_section(ctx: PageCtx<'_>, menu_layer: &mut Vec<El>) -> Option<El> {
    let app = ctx.app;
    let prefs = shell::preferences(app);
    let current = DateFormat::parse(&prefs.date_format);
    let choices = [
        ("Relative", DateFormat::Relative),
        ("ISO 8601", DateFormat::Iso8601),
        ("Long", DateFormat::Long),
    ];
    let current_label = choices
        .iter()
        .find_map(|&(l, f)| (f == current).then(|| tr(l)))
        .unwrap_or_else(|| tr("Relative"));
    let options = choices
        .into_iter()
        .map(|(label, fmt)| {
            ChoiceOption::new(tr(label), current == fmt, move |app, _| {
                set_date_format(fmt);
                shell::saved(app).change(|s| s.date_format = fmt.as_str().to_owned());
            })
            .subtitle(modified_date_example(fmt))
        })
        .collect();
    let menu = choice_menu(
        app,
        ChoiceSpec {
            id: "date-format",
            title: "Modified date format",
            current: current_label,
            disabled: false,
            options,
        },
        menu_layer,
    );
    group_card(
        &app.look,
        vec![option_row(
            ctx,
            RowSpec::new(
                "date-format",
                "Modified date format",
                "How file modified times appear in lists and details.",
            ),
            menu,
        )],
    )
}

fn performance_section(ctx: PageCtx<'_>, menu_layer: &mut Vec<El>) -> Option<El> {
    let app = ctx.app;
    let prefs = shell::preferences(app);
    let mut rows = vec![
        switch_row(
            ctx,
            RowSpec::new(
                "watch-network-folders",
                "Watch network folders",
                "Check open folders on network shares every few seconds for changes made on other computers. When off, those changes show only after a refresh.",
            ),
            prefs.watch_network_folders,
            |app, on, _| {
                shell::saved(app).change(|s| s.watch_network_folders = on);
                shell::sync_freshness(app);
            },
        ),
        option_row(
            ctx,
            RowSpec::new(
                "thumbnail-workers",
                "Thumbnail workers",
                "Parallel thumbnail decoders across all windows. More workers use more CPU and memory.",
            ),
            workers_field().build(app, &app.look),
        ),
    ];
    if ctx.search.shows_row(app, "video") {
        rows.extend(video_preview_rows(ctx, menu_layer));
    }
    group_card(&app.look, rows)
}

fn video_preview_rows(ctx: PageCtx<'_>, menu_layer: &mut Vec<El>) -> Vec<Option<El>> {
    let app = ctx.app;
    let prefs = shell::preferences(app);
    let hw_accel = configured_hardware_acceleration(prefs, polaris_gpu_available());
    let backend_menu = video_backend_menu(app, hw_accel, menu_layer);
    let accel_toggle = switch_row(
        ctx,
        RowSpec::new(
            "video",
            "Hardware-accelerated video previews",
            "Decode video thumbnails on the GPU.",
        ),
        hw_accel,
        |app, on, _| shell::saved(app).change(|s| s.set_hardware_accelerated_video_previews(on)),
    );
    let backend_row = option_row(
        ctx,
        RowSpec::new("video-backend-row", "Decoding backend", "")
            .indented()
            .disabled(!hw_accel),
        backend_menu,
    );
    vec![accel_toggle, backend_row]
}

fn video_backend_menu(app: &App, hw_accel: bool, menu_layer: &mut Vec<El>) -> El {
    let selected = configured_video_preview_backend(shell::preferences(app));
    let backends = [
        ("Automatic", MediaPreviewBackend::Automatic),
        ("VA-API", MediaPreviewBackend::VaApi),
        ("Vulkan", MediaPreviewBackend::Vulkan),
    ];
    let backend_label = match selected {
        MediaPreviewBackend::VaApi => tr("VA-API"),
        MediaPreviewBackend::Vulkan => tr("Vulkan"),
        MediaPreviewBackend::Automatic | MediaPreviewBackend::Software => tr("Automatic"),
    };
    choice_menu(
        app,
        ChoiceSpec {
            id: "video-backend",
            title: "Video preview hardware backend",
            current: backend_label,
            disabled: !hw_accel,
            options: backends
                .into_iter()
                .map(|(label, backend)| {
                    ChoiceOption::new(
                        tr(label),
                        backend_selected(selected, backend),
                        move |app, _| {
                            save_video_backend(app, backend);
                        },
                    )
                })
                .collect(),
        },
        menu_layer,
    )
}

fn backend_selected(selected: MediaPreviewBackend, option: MediaPreviewBackend) -> bool {
    if selected == MediaPreviewBackend::Software {
        option == MediaPreviewBackend::Automatic
    } else {
        selected == option
    }
}

fn save_video_backend(app: &mut App, backend: MediaPreviewBackend) {
    let mut changed = false;
    shell::saved(app).mutate(|s| {
        changed = s.set_video_preview_backend(backend);
        changed
    });
    if changed {
        shell::saved(app).commit();
    }
}

fn window_buttons_section(ctx: PageCtx<'_>) -> Option<El> {
    let prefs = shell::preferences(ctx.app);
    group_card(
        &ctx.app.look,
        vec![
            pref_switch(
                ctx,
                RowSpec::new(
                    "window-minimize",
                    "Show minimize button",
                    "Add a minimize button to the top bar.",
                ),
                prefs.window_show_minimize,
                |s, v| s.window_show_minimize = v,
            ),
            pref_switch(
                ctx,
                RowSpec::new(
                    "window-maximize",
                    "Show maximize button",
                    "Add a maximize and restore button to the top bar.",
                ),
                prefs.window_show_maximize,
                |s, v| s.window_show_maximize = v,
            ),
            pref_switch(
                ctx,
                RowSpec::new(
                    "window-close",
                    "Show close button",
                    "Keep the close button in the top bar. Hide it when the window manager already closes windows.",
                ),
                prefs.window_show_close,
                |s, v| s.window_show_close = v,
            ),
        ],
    )
}

fn startup_section(ctx: PageCtx<'_>) -> Option<El> {
    let app = ctx.app;
    let prefs = shell::preferences(app);
    let default_dir_label = prefs.default_directory.as_deref().map_or_else(
        || tr("Home directory"),
        |dir| abbreviate_home(Path::new(dir)),
    );
    let dir_badge = text(default_dir_label)
        .font(app.look.kit.ty.ui)
        .color(app.look.kit.c.text_base);
    let reset = Button::new(tr("Reset"))
        .variant(ButtonVariant::Neutral)
        .disabled(prefs.default_directory.is_none())
        .on_press_tx(|app: &mut App, _: &mut Tx| {
            shell::saved(app).change(|s| s.default_directory = None);
        })
        .build(&app.look.kit)
        .description(tr("Restore the home directory as default"));
    group_card(
        &app.look,
        vec![
            option_row(
                ctx,
                RowSpec::new(
                    "default-directory",
                    "Default directory",
                    "Open this folder when launching Strata without a target.",
                ),
                row()
                    .gap(app.look.space.sm)
                    .items_center()
                    .child(dir_badge)
                    .child(buttons::on_return("settings-default-dir-reset", reset)),
            ),
            pref_switch(
                ctx,
                RowSpec::new(
                    "restore-tabs",
                    "Restore open tabs",
                    "Reopen your tabs from the last session when launching Strata without a target.",
                ),
                prefs.restore_tabs,
                |s, v| s.restore_tabs = v,
            ),
        ],
    )
}

fn abbreviate_home(path: &Path) -> String {
    let home = launch::home_directory();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}
