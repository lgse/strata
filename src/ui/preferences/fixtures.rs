// SPDX-License-Identifier: MIT

use super::*;

/// Deliberately exhaustive: adding a stored preference requires extending this
/// fixture, and the setter-coverage test fails until the new setter is exercised.
pub(in crate::ui) fn non_default_preferences() -> Preferences {
    Preferences {
        mode: "theme".into(),
        theme: "nord".into(),
        folder_peeking: false,
        single_click_previews: false,
        render_documents_by_default: false,
        hardware_accelerated_video_previews: Some(false),
        video_preview_backend: "vulkan".into(),
        search_open_files_directly: true,
        type_to_search: false,
        arrow_navigation_scoped: true,
        filter_include_subfolders: false,
        show_keybinding_hints: false,
        reduce_motion: true,
        element_glow: false,
        browser_mode: "list".into(),
        browser_density: "airy".into(),
        group_by_type: true,
        columns_file_clicks: 1,
        columns_folder_clicks: 2,
        icons_file_clicks: 1,
        icons_folder_clicks: 1,
        list_file_clicks: 1,
        list_folder_clicks: 1,
        sidebar_order: vec![
            "videos".into(),
            "pictures".into(),
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
        sidebar_show_pictures: false,
        sidebar_show_videos: false,
        show_hidden: true,
        text_size: TextSize::new(24),
        folders_first: false,
        sort_key: "size".into(),
        sort_direction: "descending".into(),
        check_for_updates: false,
        preview_muted: true,
        preview_volume: 0.35,
        preview_text_wrap: true,
        preview_autoplay: true,
        auto_refresh_interval: 600,
        thumbnail_workers: 6,
        icons_thumbnail_size: 128,
        cross_volume_drop_strategy: CrossVolumeDropStrategy::Move.as_str().into(),
        open_folder_after_drop: true,
        release_channel: "nightly".into(),
        default_directory: Some("/fixture/default".into()),
        folder_colors: HashMap::from([("/fixture/folder".into(), "red".into())]),
        custom_icons: HashMap::from([(
            "/fixture/folder".into(),
            crate::assets::icons::HOME.into(),
        )]),
    }
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
    let state = super::super::theme::omarchy_state_dir();
    fs::create_dir_all(state.join("theme")).expect("isolated Omarchy theme directory");
    fs::write(state.join("theme.name"), "fixture").expect("isolated Omarchy name");
    fs::write(
        state.join("theme/colors.toml"),
        "background = '#112233'\nforeground = '#ddeeff'\naccent = '#445566'\n",
    )
    .expect("isolated Omarchy colors");
}

impl PreferenceManager {
    pub(in crate::ui) fn seed_saved_preferences_for_test() {
        seed_saved_preferences_for_test();
    }

    pub(in crate::ui) fn seed_omarchy_for_test() {
        seed_omarchy_for_test();
    }
}
