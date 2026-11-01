// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn browsing_preferences_stay_saved_but_unused_until_exit() {
    gtk_test(
        "ui::window::tests::preferences::browsing_prefs::browsing_preferences_stay_saved_but_unused_until_exit",
        || {
            let manager = PreferenceManager::shared();
            manager.set_type_to_search(true);
            manager.set_arrow_navigation_scoped(true);
            manager.set_columns_mirror_selection(true);
            let open = OpenWindow::open();
            let directory = load_folder(&open);
            assert!(open.content.browser.columns_mirror_selection_enabled());
            open.content.browser.browser().focus_active();
            wait_until(|| open.content.browser.item_view_has_focus());
            press(
                &open.window,
                gtk::gdk::Key::x,
                gtk::gdk::ModifierType::empty(),
            );
            settle();
            assert!(
                filter_buttons(&open)
                    .iter()
                    .any(|button| button.is_active()),
                "type to search filters before 10xer is enabled"
            );
            for button in filter_buttons(&open) {
                if button.is_active() {
                    button.set_active(false);
                }
            }
            open.content.settings_button().emit_clicked();
            settle();
            for title in [
                "Type to search",
                "Keep arrows in file list",
                "Include subfolders",
                "Mirror columns selection",
            ] {
                let switch = switch_named(open.content.overlay(), title);
                assert!(switch.is_active() && switch.is_sensitive());
                assert_ne!(
                    description_named(open.content.overlay(), title),
                    UNUSED_SUBTITLE
                );
            }
            manager.set_tenxer_mode(true);
            settle();
            assert!(manager.type_to_search());
            assert!(manager.arrow_navigation_scoped());
            assert!(
                open.content.browser.columns_mirror_selection_enabled(),
                "10xer Columns keep saved mirroring"
            );
            assert_ne!(
                description_named(open.content.overlay(), "Mirror columns selection"),
                UNUSED_SUBTITLE
            );
            for title in [
                "Type to search",
                "Keep arrows in file list",
                "Include subfolders",
            ] {
                let switch = switch_named(open.content.overlay(), title);
                assert!(switch.is_active() && switch.is_sensitive());
                assert_eq!(
                    description_named(open.content.overlay(), title),
                    UNUSED_SUBTITLE
                );
            }
            let type_to_search = switch_named(open.content.overlay(), "Type to search");
            type_to_search.set_active(false);
            settle();
            assert!(!manager.type_to_search());
            type_to_search.set_active(true);
            settle();
            assert!(manager.type_to_search());
            close_settings(&open);
            open.content.browser.browser().focus_active();
            wait_until(|| open.content.browser.item_view_has_focus());
            press(
                &open.window,
                gtk::gdk::Key::x,
                gtk::gdk::ModifierType::empty(),
            );
            settle();
            assert!(
                filter_buttons(&open)
                    .iter()
                    .all(|button| !button.is_active()),
                "typing does not filter while 10xer is on"
            );
            manager.set_tenxer_mode(false);
            settle();
            assert!(manager.type_to_search());
            assert!(open.content.browser.columns_mirror_selection_enabled());
            open.content.settings_button().emit_clicked();
            settle();
            for title in [
                "Type to search",
                "Keep arrows in file list",
                "Include subfolders",
                "Mirror columns selection",
            ] {
                assert_ne!(
                    description_named(open.content.overlay(), title),
                    UNUSED_SUBTITLE
                );
            }
            close_settings(&open);
            open.content.browser.set_view_mode(BrowserMode::Columns);
            wait_until(|| !column_loading(&open, 0));
            open.content.browser.browser().focus_active();
            wait_until(|| open.content.browser.item_view_has_focus());
            press(
                &open.window,
                gtk::gdk::Key::x,
                gtk::gdk::ModifierType::empty(),
            );
            settle();
            assert!(
                filter_buttons(&open)
                    .iter()
                    .any(|button| button.is_active()),
                "type to search filters again after leaving 10xer"
            );
            drop(directory);
        },
    );
}

#[test]
fn watching_network_folders_applies_before_settings_and_follows_changes_across_windows() {
    gtk_test(
        "ui::window::tests::preferences::browsing_prefs::watching_network_folders_applies_before_settings_and_follows_changes_across_windows",
        || {
            write_settings("watch_network_folders = false\n");
            let manager = PreferenceManager::shared();
            let first = OpenWindow::open();
            let second = OpenWindow::open();
            let watching = || {
                [&first, &second].map(|open| {
                    open.content
                        .browser
                        .browser()
                        .freshness_input()
                        .watch_network_folders
                })
            };
            let saved = watching();
            let closed_at_startup = settings_closed(&first) && settings_closed(&second);
            first.content.settings_button().emit_clicked();
            settle();
            let page = class_in_shown_pane(first.content.overlay().upcast_ref(), "settings-dialog")
                .into_iter()
                .next()
                .expect("Settings page");
            let switch = switch_named(&page, "Watch network folders");
            let shown_off = !switch.is_active();
            switch.set_active(true);
            let switched_on = (manager.watch_network_folders(), watching());
            close_settings(&first);
            manager.set_watch_network_folders(false);

            assert_eq!(saved, [false, false]);
            assert!(closed_at_startup && shown_off);
            assert_eq!(switched_on, (true, [true, true]));
            assert_eq!(watching(), [false, false]);
        },
    );
}

#[test]
fn show_thumbnails_saved_and_live_preferences_apply_across_windows_and_views() {
    thumbnail_preferences_across_windows("photo.png", false);
}

#[test]
fn stl_thumbnails_saved_and_live_preferences_apply_across_windows_and_views() {
    thumbnail_preferences_across_windows("model.stl", true);
}

fn thumbnail_preferences_across_windows(name: &str, model: bool) {
    let test = if model {
        "ui::window::tests::preferences::browsing_prefs::stl_thumbnails_saved_and_live_preferences_apply_across_windows_and_views"
    } else {
        "ui::window::tests::preferences::browsing_prefs::show_thumbnails_saved_and_live_preferences_apply_across_windows_and_views"
    };
    gtk_test(test, || {
        use crate::ui::thumbnail::{ThumbnailSlot, cached_thumbnail, remember_thumbnail_for_test};

        write_settings("show_thumbnails = false\nstl_thumbnails = true\nbrowser_mode = 'list'\n");
        let directory = tempfile::tempdir().expect("image folder");
        let path = directory.path().join(name);
        let texture: gtk::gdk::Texture = gtk::gdk::MemoryTexture::new(
            1,
            1,
            gtk::gdk::MemoryFormat::R8g8b8a8,
            &gtk::glib::Bytes::from_owned(vec![120, 80, 40, 255]),
            4,
        )
        .upcast();
        texture.save_to_png(&path).expect("cached artwork fixture");
        let mtime = std::fs::metadata(&path)
            .expect("fixture metadata")
            .modified()
            .expect("mtime")
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix time")
            .as_secs() as i64;
        let source = path.clone();
        let png = std::fs::read(&path).expect("PNG");
        std::thread::spawn(move || crate::ui::thumbnail_cache::store(&source, mtime, &png))
            .join()
            .expect("seed thumbnail cache off the UI thread");
        let manager = PreferenceManager::shared();
        let windows = [OpenWindow::open(), OpenWindow::open()];
        for open in &windows {
            open.content
                .browser
                .navigate_location(crate::model::Location::local(directory.path()));
            let entry = loaded_entry(open, &path);
            remember_thumbnail_for_test(&entry, texture.clone());
            assert_eq!(
                cached_thumbnail(&entry),
                Some(texture.clone()),
                "preview cache remains usable"
            );
        }
        let slots = || {
            windows
                .iter()
                .map(|open| {
                    let mut found = Vec::new();
                    walk(&open.content.browser.widget(), &mut |widget| {
                        if widget.is_mapped()
                            && let Some(slot) = widget.downcast_ref::<ThumbnailSlot>()
                            && slot.provider_path().as_deref() == path.to_str()
                        {
                            found.push(slot.clone());
                        }
                    });
                    found
                })
                .collect::<Vec<_>>()
        };
        let shown = |enabled| {
            slots().iter().all(|window_slots| {
                !window_slots.is_empty()
                    && window_slots
                        .iter()
                        .all(|slot| slot.texture().is_some() == enabled)
            })
        };
        wait_until(|| shown(false));
        for mode in [BrowserMode::List, BrowserMode::Columns, BrowserMode::Icons] {
            manager.set_browser_mode(mode);
            wait_until(|| {
                windows
                    .iter()
                    .all(|open| open.content.browser.view_mode() == mode)
            });
            wait_until(|| shown(false));
            manager.set_show_thumbnails(true);
            wait_until(|| shown(true));
            manager.set_stl_thumbnails(false);
            wait_until(|| shown(!model));
            manager.set_stl_thumbnails(true);
            wait_until(|| shown(true));
            manager.set_show_thumbnails(false);
            wait_until(|| shown(false));
        }
        manager.set_stl_thumbnails(false);
        crate::storage::writer::settle();
        let saved = std::fs::read_to_string(settings_file()).expect("saved settings");
        assert!(saved.contains("show_thumbnails = false"));
        assert!(saved.contains("stl_thumbnails = false"));
        for open in windows {
            open.window.close();
        }
    });
}
