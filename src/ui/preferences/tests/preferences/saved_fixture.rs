// SPDX-License-Identifier: MIT

use super::*;

fn assert_saved_behaviour(manager: &PreferenceManager) {
    assert_eq!(
        [
            manager.folder_peeking(),
            manager.single_click_previews(),
            manager.columns_mirror_selection(),
            manager.columns_show_sizes(),
            manager.hardware_accelerated_video_previews(),
        ],
        [true, false, false, false, false],
        "(folder peeking, single-click previews, Columns mirror selection, Columns sizes, \
         hardware video previews)"
    );
    assert_eq!(
        (
            manager.video_preview_backend(),
            manager.media_preview_backend()
        ),
        (MediaPreviewBackend::Vulkan, MediaPreviewBackend::Software),
        "(video, media) preview backends"
    );
}

fn assert_saved_search(manager: &PreferenceManager) {
    assert!(manager.search_open_files_directly());
    assert_eq!(
        manager.search_exclusions(),
        vec![".venv", "/fixture/custom_excluded"]
    );
    assert_eq!(
        (manager.search_time_limit(), manager.search_entry_limit()),
        (300, 1_000_000),
        "(time, entry) limits"
    );
    assert_eq!(
        manager.search_limits(),
        SearchLimits {
            time_budget: Some(Duration::from_secs(300)),
            max_entries: Some(1_000_000),
        }
    );
}

fn assert_saved_keyboard_and_window(manager: &PreferenceManager) {
    assert_eq!(
        [
            manager.type_to_search(),
            manager.arrow_navigation_scoped(),
            manager.tenxer_mode(),
            manager.filter_include_subfolders(),
            manager.show_keybinding_hints(),
        ],
        [false, true, true, false, false],
        "(type to search, scoped arrows, tenxer mode, filter subfolders, keybinding hints)"
    );
    assert_eq!(
        [
            manager.window_show_close(),
            manager.window_show_minimize(),
            manager.window_show_maximize(),
            manager.reduce_motion(),
            manager.element_glow(),
        ],
        [false, true, true, true, false],
        "(close, minimize, maximize, reduce motion, element glow)"
    );
}

#[expect(
    deprecated,
    reason = "GTK has no replacement API for resolving named CSS colors"
)]
fn named_color(widget: &impl IsA<gtk::Widget>, name: &str) -> Option<gtk::gdk::RGBA> {
    widget.style_context().lookup_color(name)
}

/// Element glow follows its setting live in every window.
fn assert_glow_follows_the_setting(manager: &PreferenceManager, windows: &[gtk::Window]) {
    for enabled in [false, true, false] {
        manager.set_element_glow(enabled);
        for window in windows {
            let surface = gtk::Box::new(gtk::Orientation::Vertical, 0);
            window.set_child(Some(&surface));
            let glow = named_color(&surface, "strata_glow").expect("glow color");
            let accent = named_color(&surface, "strata_accent").expect("accent color");
            if enabled {
                assert_eq!(glow, accent);
            } else {
                assert_eq!(glow.alpha(), 0.0);
                assert!(accent.alpha() > 0.0);
            }
        }
    }
}

/// Theme colors follow the selected theme in every window without overriding the
/// user's own GTK colors.
fn assert_theme_colors_beside_user_css(themes: &ThemeManager, windows: &[gtk::Window]) {
    let display = gtk::gdk::Display::default().expect("test display");
    let user_css = gtk::CssProvider::new();
    user_css
        .load_from_string("@define-color theme_bg #ff00ff; @define-color theme_accent #00ff00;");
    gtk::style_context_add_provider_for_display(
        &display,
        &user_css,
        gtk::STYLE_PROVIDER_PRIORITY_USER,
    );
    for theme in ["nord", "azure-glow", "nord"] {
        themes.select_theme(theme);
        let tokens = themes.current_tokens().expect("active theme");
        let expected_colors = [
            ("strata_bg", tokens.background.as_str()),
            ("strata_accent", tokens.accent.as_str()),
            ("theme_bg", "#ff00ff"),
            ("theme_accent", "#00ff00"),
        ];
        for window in windows {
            for (name, expected) in expected_colors {
                assert_eq!(
                    named_color(window, name).expect("named color"),
                    gtk::gdk::RGBA::parse(expected).expect("token color"),
                    "{theme}: {name}",
                );
            }
        }
    }
    gtk::style_context_remove_provider_for_display(&display, &user_css);
}

fn assert_saved_browsing(manager: &PreferenceManager) {
    assert!(manager.compact_full_names());
    assert!(!crate::ui::motion::animations_enabled());
    assert_eq!(
        (
            manager.browser_mode(),
            manager.browser_density(),
            manager.frame_density(),
            manager.group_by_type(),
        ),
        (
            BrowserMode::List,
            BrowserDensity::Airy,
            FrameDensity::Custom(6),
            true
        ),
        "(mode, density, frame density, group by type)"
    );
    // Compact reads the Icons click settings.
    for mode in [
        BrowserMode::Columns,
        BrowserMode::Icons,
        BrowserMode::List,
        BrowserMode::Compact,
    ] {
        let activation = manager.click_activation(mode);
        assert_eq!(activation.files, ClickCount::One);
        assert_eq!(
            activation.folders,
            if mode == BrowserMode::Columns {
                ClickCount::Two
            } else {
                ClickCount::One
            }
        );
    }
}

fn assert_saved_sidebar(manager: &PreferenceManager) {
    assert_eq!(
        manager.sidebar_order(),
        non_default_preferences().sidebar_order
    );
    assert_eq!(
        [
            manager.sidebar_show_home(),
            manager.sidebar_show_trash(),
            manager.sidebar_show_network(),
            manager.sidebar_show_recent(),
            manager.sidebar_show_desktop(),
            manager.sidebar_show_documents(),
            manager.sidebar_show_downloads(),
            manager.sidebar_show_music(),
            manager.sidebar_show_pictures(),
            manager.sidebar_show_videos(),
        ],
        [false; 10],
        "shown places (home, trash, network, recent, desktop, documents, downloads, \
         music, pictures, videos)"
    );
    assert_eq!(
        [
            manager.sidebar_expanded(),
            manager.sidebar_pinned_collapsed(),
            manager.sidebar_devices_collapsed(),
            manager.sidebar_compact(),
        ],
        [false, true, true, true],
        "(expanded, pinned collapsed, devices collapsed, compact)"
    );
    assert_eq!(
        manager.sidebar_places_visibility(),
        [
            false, false, false, false, false, false, false, false, false, false
        ]
    );
}

fn assert_saved_views_and_updates(manager: &PreferenceManager) {
    assert_eq!(manager.text_size(), TextSize::new(24));
    assert_eq!(
        manager.sort_preferences(),
        ViewPreferences {
            name_comparison: crate::model::NameComparison {
                natural: false,
                case_sensitive: true
            },
            show_hidden: true,
            folders_first: false,
            sort_key: SortKey::Size,
            sort_direction: SortDirection::Descending,
        }
    );
    assert_eq!(
        (
            manager.remember_folder_views(),
            manager.checks_for_updates(),
            manager.release_channel(),
        ),
        (false, false, Channel::Nightly),
        "(remember folder views, update checks, release channel)"
    );
}

fn assert_saved_previews(manager: &PreferenceManager) {
    assert_eq!(
        (
            manager.preview_muted(),
            manager.preview_volume(),
            manager.preview_text_wrap(),
            manager.unit_system(),
            manager.preview_autoplay(),
            manager.preview_video_start(),
        ),
        (true, 0.35, false, UnitSystem::Imperial, true, 20),
        "(muted, volume, text wrap, units, autoplay, video start)"
    );
    assert_eq!(
        (
            manager.watch_network_folders(),
            manager.thumbnail_workers(),
            manager.show_thumbnails(),
            manager.stl_thumbnails(),
        ),
        (false, 6, false, true),
        "(watch network folders, thumbnail workers, thumbnails, STL thumbnails)"
    );
}

fn assert_saved_list_and_drops(manager: &PreferenceManager) {
    assert_eq!(
        manager.list_column_order(),
        vec![
            ListColumnId::Name,
            ListColumnId::Size,
            ListColumnId::Type,
            ListColumnId::Modified,
            ListColumnId::Mode,
        ]
    );
    assert_eq!(manager.list_hidden_columns(), vec![ListColumnId::Mode]);
    assert_eq!(
        manager.cross_volume_drop_strategy(),
        CrossVolumeDropStrategy::Move
    );
    assert_eq!(manager.date_format(), crate::util::DateFormat::Iso8601);
}

fn assert_saved_devices_and_folders(manager: &PreferenceManager) {
    assert_eq!(
        manager.device_label("volume:fixture-kingston").as_deref(),
        Some("Research drive")
    );
    assert_eq!(
        manager.send_to_recent_destinations("volume:fixture-kingston"),
        [PathBuf::from("Academia/2026"), PathBuf::from("Teaching")]
    );
    assert_eq!(
        manager.send_to_recent_destinations("volume:fixture-sandisk"),
        [PathBuf::from("Backup")]
    );
    assert_eq!(
        manager.default_directory(),
        Some(std::path::PathBuf::from("/fixture/default"))
    );
    assert_eq!(
        (
            manager.folder_color(Path::new("/fixture/folder")),
            manager.custom_icon(Path::new("/fixture/folder"))
        ),
        (
            FolderColorValue::parse("red"),
            Some(crate::assets::icons::HOME.to_owned())
        ),
        "/fixture/folder (color, icon)"
    );
}

fn assert_saved_keymap(manager: &PreferenceManager) {
    let keymap = manager.keymap();
    assert_eq!(
        keymap.bindings(Map::Default, bindings::SETTINGS),
        [
            Binding::ctrl(gtk::gdk::Key::comma),
            Binding::ctrl_alt(gtk::gdk::Key::F12)
        ]
    );
    assert_eq!(
        keymap.bindings(Map::Tenxer, bindings::SIDEBAR_FOCUS),
        [
            Binding::ctrl_shift(gtk::gdk::Key::b),
            Binding::ctrl_alt(gtk::gdk::Key::F11)
        ]
    );
    assert_eq!(
        keymap.bindings(Map::Tenxer, bindings::SETTINGS),
        bindings::SETTINGS.defaults(Map::Tenxer)
    );
}

#[test]
fn every_saved_preference_loads_before_any_settings_page_exists() {
    gtk_test(
        "ui::preferences::tests::preferences::saved_fixture::every_saved_preference_loads_before_any_settings_page_exists",
        || {
            seed_saved_preferences_for_test();
            let themes = ThemeManager::shared();
            let manager = PreferenceManager::shared();
            assert_eq!(*manager.preferences.borrow(), non_default_preferences());
            assert!(!themes.follows_omarchy());
            assert_eq!(themes.selected_id(), "nord");
            assert_saved_behaviour(&manager);
            assert_saved_search(&manager);
            assert_saved_keyboard_and_window(&manager);
            let windows = [gtk::Window::new(), gtk::Window::new()];
            assert_glow_follows_the_setting(&manager, &windows);
            assert_theme_colors_beside_user_css(&themes, &windows);
            for window in windows {
                window.close();
            }
            assert_saved_browsing(&manager);
            assert_saved_sidebar(&manager);
            assert_saved_views_and_updates(&manager);
            assert_saved_previews(&manager);
            assert_saved_list_and_drops(&manager);
            assert_saved_devices_and_folders(&manager);
            assert_saved_keymap(&manager);
        },
    );
}
