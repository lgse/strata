// SPDX-License-Identifier: MIT

use super::*;
use std::path::PathBuf;

/// Deliberately exhaustive: adding a stored preference requires extending this
/// fixture, and the setter-coverage test fails until the new setter is exercised.
pub(in crate::ui) fn non_default_preferences() -> Preferences {
    Preferences {
        mode: "theme".into(),
        theme: "nord".into(),
        legacy_omarchy_variant: None,
        omarchy_variants: HashMap::from([("fixture".into(), OmarchyVariant::Lighter)]),
        folder_peeking: true,
        single_click_previews: false,
        columns_mirror_selection: false,
        columns_show_sizes: false,
        render_documents_by_default: false,
        hardware_accelerated_video_previews: Some(false),
        video_preview_backend: "vulkan".into(),
        search_open_files_directly: true,
        search_exclusions: vec![".venv".into(), "/fixture/custom_excluded".into()],
        search_time_limit: 300,
        search_entry_limit: 1_000_000,
        type_to_search: false,
        arrow_navigation_scoped: true,
        tenxer_mode: true,
        filter_include_subfolders: false,
        show_keybinding_hints: false,
        window_show_close: false,
        window_show_minimize: true,
        window_show_maximize: true,
        reduce_motion: true,
        element_glow: false,
        file_icon_colors: FileIconColors::Theme,
        browser_mode: "list".into(),
        browser_density: "airy".into(),
        browser_density_custom_px: 11,
        frame_density: "custom".into(),
        frame_density_custom_px: 6,
        group_by_type: true,
        columns_file_clicks: 1,
        columns_folder_clicks: 2,
        icons_file_clicks: 1,
        icons_folder_clicks: 1,
        list_file_clicks: 1,
        list_folder_clicks: 1,
        sidebar_order: vec![
            "recent".into(),
            "network".into(),
            "trash".into(),
            "home".into(),
            "videos".into(),
            "pictures".into(),
            "music".into(),
            "downloads".into(),
            "documents".into(),
            "desktop".into(),
        ],
        sidebar_show_home: false,
        sidebar_show_trash: false,
        sidebar_show_network: false,
        sidebar_show_recent: false,
        sidebar_show_desktop: false,
        sidebar_show_documents: false,
        sidebar_show_downloads: false,
        sidebar_show_music: false,
        sidebar_show_pictures: false,
        sidebar_show_videos: false,
        sidebar_expanded: false,
        sidebar_compact: true,
        show_hidden: true,
        text_size: TextSize::new(24),
        corner_roundness: CornerRoundness::new(40),
        interface_font: Some("Fixture Sans".into()),
        interface_renderer: InterfaceRenderer::Cairo,
        language: crate::i18n::Language::English,
        folders_first: false,
        sort_key: "size".into(),
        sort_direction: "descending".into(),
        check_for_updates: false,
        preview_muted: true,
        preview_volume: 0.35,
        preview_text_wrap: true,
        unit_system: UnitSystem::Imperial,
        preview_autoplay: true,
        watch_network_folders: false,
        thumbnail_workers: 6,
        show_thumbnails: false,
        stl_thumbnails: true,
        icons_thumbnail_size: 128,
        chooser_column_width: Some(420),
        browser_column_width: Some(380),
        cross_volume_drop_strategy: CrossVolumeDropStrategy::Move.as_str().into(),
        open_folder_after_drop: true,
        date_format: "iso".into(),
        release_channel: "nightly".into(),
        default_directory: Some("/fixture/default".into()),
        restore_tabs: false,
        remember_folder_views: false,
        folder_colors: HashMap::from([("/fixture/folder".into(), "red".into())]),
        custom_icons: HashMap::from([(
            "/fixture/folder".into(),
            crate::assets::icons::HOME.into(),
        )]),
        device_labels: HashMap::from([("volume:fixture-kingston".into(), "Research drive".into())]),
        send_to_recent_destinations: HashMap::from([
            (
                "volume:fixture-kingston".into(),
                vec![PathBuf::from("Academia/2026"), PathBuf::from("Teaching")],
            ),
            (
                "volume:fixture-sandisk".into(),
                vec![PathBuf::from("Backup")],
            ),
        ]),
        browser_list_columns: Some(ListColumns {
            name: None,
            mode: 110,
            size: 80,
            kind: 95,
            modified: 140,
        }),
        chooser_list_columns: Some(ListColumns {
            name: Some(280),
            mode: 100,
            size: 72,
            kind: 90,
            modified: 130,
        }),
        list_column_order: vec![
            ListColumnId::Name,
            ListColumnId::Size,
            ListColumnId::Type,
            ListColumnId::Modified,
            ListColumnId::Mode,
        ],
        list_hidden_columns: vec![ListColumnId::Mode],
        keymap: KeymapOverrides {
            default: [("settings", &["<Control>comma", "<Control><Alt>F12"][..])]
                .into_iter()
                .map(keymap_entry)
                .collect(),
            tenxer: [
                (
                    "focus_sidebar",
                    &["<Control><Shift>b", "<Control><Alt>F11"][..],
                ),
                ("future.command", &["x"][..]),
            ]
            .into_iter()
            .map(keymap_entry)
            .collect(),
        },
    }
}

pub(in crate::ui) fn keymap_entry((id, accelerators): (&str, &[&str])) -> (String, toml::Value) {
    (
        id.to_owned(),
        toml::Value::Array(
            accelerators
                .iter()
                .map(|accelerator| toml::Value::String((*accelerator).to_owned()))
                .collect(),
        ),
    )
}

pub(in crate::ui) fn seed_saved_preferences_for_test() {
    let path = settings_path();
    fs::create_dir_all(path.parent().expect("settings parent"))
        .expect("isolated preferences directory");
    fs::write(
        path,
        toml::to_string(&non_default_preferences()).expect("serialize complete fixture"),
    )
    .expect("persist complete fixture");
}

pub(in crate::ui) fn seed_omarchy_for_test() {
    seed_omarchy_colors_for_test(
        "background = '#112233'\ndark_background = '#0a1520'\nforeground = '#ddeeff'\naccent = '#445566'\n",
    );
}

pub(in crate::ui) fn seed_omarchy_colors_for_test(colors: &str) {
    let state = super::super::theme::omarchy_state_dir();
    fs::create_dir_all(state.join("theme")).expect("isolated Omarchy theme directory");
    fs::write(state.join("theme.name"), "fixture").expect("isolated Omarchy name");
    fs::write(state.join("theme/colors.toml"), colors).expect("isolated Omarchy colors");
}

impl PreferenceManager {
    pub(in crate::ui) fn seed_saved_preferences_for_test() {
        seed_saved_preferences_for_test();
    }

    /// Saves the exhaustive fixture outside 10xer mode, with only these keymap
    /// overrides: command ids and their accelerators.
    pub(in crate::ui) fn seed_saved_keymap_for_test(
        default: &[(&str, &[&str])],
        tenxer: &[(&str, &[&str])],
    ) {
        let preferences = Preferences {
            tenxer_mode: false,
            keymap: KeymapOverrides {
                default: default.iter().copied().map(keymap_entry).collect(),
                tenxer: tenxer.iter().copied().map(keymap_entry).collect(),
            },
            ..non_default_preferences()
        };
        let path = settings_path();
        fs::create_dir_all(path.parent().expect("settings parent"))
            .expect("isolated preferences directory");
        fs::write(
            path,
            toml::to_string(&preferences).expect("serialize keymap fixture"),
        )
        .expect("persist keymap fixture");
    }
}
