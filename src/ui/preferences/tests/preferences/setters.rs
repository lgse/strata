// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn all_preference_setters_publish_and_persist_without_duplicate_notifications() {
    gtk_test(
        "ui::preferences::tests::preferences::setters::all_preference_setters_publish_and_persist_without_duplicate_notifications",
        || {
            seed_saved_preferences_for_test();
            seed_omarchy_for_test();
            let manager = PreferenceManager::shared();
            let themes = ThemeManager::shared();
            let anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let observations = Rc::new(RefCell::new(Vec::new()));
            let observed = observations.clone();
            manager.bind_preference(
                &anchor,
                |manager| manager.preferences.borrow().clone(),
                move |_, value| observed.borrow_mut().push(value),
            );
            let preference_setters: &[fn(&PreferenceManager)] = &[
                |m| m.set_folder_peeking(false),
                |m| m.set_single_click_previews(true),
                |m| m.set_columns_mirror_selection(true),
                |m| m.set_columns_show_sizes(true),
                |m| m.set_render_documents_by_default(true),
                |m| m.set_hardware_accelerated_video_previews(true),
                |m| m.set_video_preview_backend(MediaPreviewBackend::VaApi),
                |m| m.set_search_open_files_directly(false),
                |m| m.set_search_exclusions(vec!["changed_exclusion".to_owned()]),
                |m| m.set_search_time_limit(0),
                |m| m.set_search_entry_limit(0),
                |m| m.set_type_to_search(true),
                |m| m.set_arrow_navigation_scoped(false),
                |m| m.set_tenxer_mode(false),
                |m| m.set_filter_include_subfolders(true),
                |m| m.set_show_keybinding_hints(true),
                |m| m.set_window_show_close(true),
                |m| m.set_window_show_minimize(false),
                |m| m.set_window_show_maximize(false),
                |m| m.set_reduce_motion(false),
                |m| m.set_element_glow(true),
                |m| m.set_omarchy_variant(OmarchyVariant::Contrast),
                |m| m.set_file_icon_colors(FileIconColors::Brand),
                |m| m.set_browser_mode(BrowserMode::Icons),
                |m| m.set_browser_density(BrowserDensity::Custom(9)),
                |m| m.set_browser_density(BrowserDensity::Compact),
                |m| m.set_frame_density(FrameDensity::Custom(5)),
                |m| m.set_frame_density(FrameDensity::Airy),
                |m| m.set_frame_density(FrameDensity::Compact),
                |m| m.set_group_by_type(false),
                |m| m.set_compact_full_names(false),
                |m| {
                    m.set_click_activation(
                        BrowserMode::Columns,
                        crate::ui::browser_modes::ClickActivation::default_for(
                            BrowserMode::Columns,
                        ),
                    )
                },
                |m| {
                    m.set_click_activation(
                        BrowserMode::Icons,
                        crate::ui::browser_modes::ClickActivation::default_for(BrowserMode::Icons),
                    )
                },
                |m| {
                    m.set_click_activation(
                        BrowserMode::List,
                        crate::ui::browser_modes::ClickActivation::default_for(BrowserMode::List),
                    )
                },
                |m| m.set_sidebar_order(default_sidebar_order()),
                |m| m.set_sidebar_show_home(true),
                |m| m.set_sidebar_show_trash(true),
                |m| m.set_sidebar_show_network(true),
                |m| m.set_sidebar_show_recent(true),
                |m| m.set_sidebar_show_desktop(true),
                |m| m.set_sidebar_show_documents(true),
                |m| m.set_sidebar_show_downloads(true),
                |m| m.set_sidebar_show_music(true),
                |m| m.set_sidebar_show_pictures(true),
                |m| m.set_sidebar_show_videos(true),
                |m| m.set_sidebar_expanded(true),
                |m| m.set_sidebar_pinned_collapsed(false),
                |m| m.set_sidebar_devices_collapsed(false),
                |m| m.set_sidebar_compact(false),
                |m| m.set_sort_preferences(ViewPreferences::default()),
                |m| m.set_text_size(TextSize::new(11)),
                |m| m.set_corner_roundness(CornerRoundness::new(150)),
                |m| m.set_interface_font(None),
                |m| m.set_interface_renderer(InterfaceRenderer::System),
                |m| m.set_language(crate::i18n::Language::Auto),
                |m| m.set_checks_for_updates(true),
                |m| m.set_release_channel(Channel::Stable),
                |m| m.set_preview_muted(false),
                |m| m.set_preview_volume(0.8),
                |m| m.set_preview_text_wrap(true),
                |m| m.set_unit_system(UnitSystem::Metric),
                |m| m.set_preview_autoplay(false),
                |m| m.set_preview_video_start(0),
                |m| m.set_watch_network_folders(true),
                |m| m.set_thumbnail_workers(3),
                |m| m.set_show_thumbnails(true),
                |m| m.set_stl_thumbnails(false),
                |m| m.set_icons_thumbnail_size(96),
                |m| m.set_chooser_column_width(Some(360)),
                |m| m.set_chooser_list_columns(None),
                |m| m.set_browser_column_width(Some(400)),
                |m| m.set_browser_list_columns(None),
                |m| m.set_list_column_order(default_list_column_order()),
                |m| m.set_list_hidden_columns(Vec::new()),
                |m| m.set_cross_volume_drop_strategy(CrossVolumeDropStrategy::Copy),
                |m| m.set_date_format(crate::util::DateFormat::Long),
                |m| m.set_default_directory(None),
                |m| m.set_restore_tabs(true),
                |m| m.set_remember_folder_views(true),
                |m| m.set_device_label("volume:fixture-kingston", "Photos / 📁"),
                |m| m.set_device_label("volume:fixture-kingston", ""),
                |m| {
                    m.remember_send_to_destination(
                        "volume:fixture-kingston",
                        Path::new("Research"),
                        None,
                    )
                },
                |m| m.set_open_folder_after_drop(false),
                |m| m.set_folder_color(Path::new("/fixture/folder"), None),
                |m| m.set_custom_icon(Path::new("/fixture/folder"), None),
                |m| {
                    m.set_bindings(
                        Map::Default,
                        bindings::RENAME,
                        vec![Binding::ctrl(gtk::gdk::Key::e)],
                    )
                },
                |m| m.reset_binding(Map::Tenxer, bindings::SIDEBAR_FOCUS),
                |m| m.reset_map(Map::Default),
            ];
            let theme_setters: &[fn(&ThemeManager)] = &[
                |m| m.set_follow_omarchy(true),
                |m| m.select_theme("azure-glow"),
            ];
            let mut previous = toml::Table::try_from(&*manager.preferences.borrow())
                .expect("preference inventory");
            let all_keys = previous
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            let mut changed_keys = std::collections::BTreeSet::new();
            let mut published = 1;
            let mut assert_setter = |published: &mut usize| {
                let expected = manager.preferences.borrow().clone();
                let current =
                    toml::Table::try_from(&expected).expect("changed preference inventory");
                for key in previous.keys().chain(current.keys()) {
                    if previous.get(key) != current.get(key) {
                        changed_keys.insert(key.clone());
                    }
                }
                previous = current;
                assert_eq!(
                    observations.borrow().len(),
                    *published,
                    "setter publishes exactly once"
                );
                assert_eq!(observations.borrow().last(), Some(&expected));
                crate::storage::writer::settle();
                assert_eq!(read_preferences().expect("saved preferences"), expected);
            };
            for setter in preference_setters {
                setter(&manager);
                published += 1;
                assert_setter(&mut published);
                setter(&manager);
                assert_eq!(
                    observations.borrow().len(),
                    published,
                    "preference setter ignores redundant values"
                );
            }
            for setter in theme_setters {
                setter(&themes);
                published += 1;
                assert_setter(&mut published);
                setter(&themes);
                assert_eq!(
                    observations.borrow().len(),
                    published,
                    "theme setter ignores redundant values"
                );
            }
            assert_eq!(
                changed_keys, all_keys,
                "every stored field needs setter and notification coverage"
            );
        },
    );
}
