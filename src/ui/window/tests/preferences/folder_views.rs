// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn saved_folder_sorts_and_icon_sizes_apply_before_settings_and_follow_changes_across_windows() {
    gtk_test(
        "ui::window::tests::preferences::folder_views::saved_folder_sorts_and_icon_sizes_apply_before_settings_and_follow_changes_across_windows",
        || {
            use std::os::unix::ffi::OsStrExt;

            use crate::model::{FolderSort, Location, SortDirection, SortKey};

            let sorted = tempfile::tempdir().expect("sorted folder");
            let plain = tempfile::tempdir().expect("plain folder");
            let sorted_location = Location::local(sorted.path());
            let plain_location = Location::local(plain.path());
            write_folder_views(&format!(
                "version = 1\n[[folder]]\npath = \"{}\"\nsort = \"size\"\ndirection = \"descending\"\nicons_size = 192\n",
                sorted.path().display()
            ));
            let manager = PreferenceManager::shared();
            let first = OpenWindow::open();
            let second = OpenWindow::open();
            let sorting = |open: &OpenWindow| {
                open.content
                    .browser
                    .browser()
                    .column_preferences(0)
                    .map(|preferences| preferences.sort())
            };
            let show = |open: &OpenWindow, location: &Location| {
                open.content.browser.navigate_location(location.clone());
                wait_until(|| {
                    open.content.browser.browser().location_at(0).as_ref() == Some(location)
                        && !column_loading(open, 0)
                });
            };

            show(&first, &sorted_location);
            show(&second, &plain_location);
            assert_eq!(
                sorting(&first),
                Some((SortKey::Size, SortDirection::Descending))
            );
            assert_eq!(
                sorting(&second),
                Some((SortKey::Name, SortDirection::Ascending))
            );
            show(&second, &sorted_location);
            assert_eq!(
                sorting(&second),
                Some((SortKey::Size, SortDirection::Descending))
            );

            first
                .content
                .browser
                .browser()
                .set_sort(0, SortKey::Type, SortDirection::Ascending);
            wait_until(|| sorting(&second) == Some((SortKey::Type, SortDirection::Ascending)));
            assert_eq!(
                manager.default_sort(),
                (SortKey::Name, SortDirection::Ascending)
            );
            manager.flush_folder_views();
            crate::storage::writer::settle();
            assert!(folder_views_file().contains("sort = \"type\""));

            manager.set_browser_mode(BrowserMode::Icons);
            wait_until(|| icons_size(&first) == Some(192) && icons_size(&second) == Some(192));
            let unremembered = plain
                .path()
                .join(std::ffi::OsStr::from_bytes(b"not-utf-8-\xff"));
            std::fs::create_dir(&unremembered).expect("folder that is not remembered");
            show(&first, &Location::local(&unremembered));
            wait_until(|| icons_size(&first) == Some(manager.icons_thumbnail_size()));
            icons_scale(&first)
                .expect("icons size slider")
                .set_value(224.0);
            icons_scale(&second)
                .expect("icons size slider")
                .set_value(128.0);
            assert_eq!(manager.icons_size_for(Some(&sorted_location)), 128);
            settle();
            assert_eq!(
                icons_size(&first),
                Some(224),
                "another folder's change keeps a size that is not remembered"
            );
            show(&first, &plain_location);
            wait_until(|| icons_size(&first) == Some(manager.icons_thumbnail_size()));

            manager.reset_folder_sort(&sorted_location);
            manager.set_default_icons_size(128);
            assert_eq!(
                manager.resolve_folder_sort(&sorted_location, false),
                FolderSort::Default
            );
            wait_until(|| icons_size(&first) == Some(128));

            manager.set_folder_sort(
                &plain_location,
                SortKey::Modified,
                SortDirection::Descending,
            );
            manager.set_default_sort(SortKey::Modified, SortDirection::Descending);
            manager.set_default_sort(SortKey::Name, SortDirection::Ascending);
            manager.set_default_icons_size(96);
            assert_eq!(
                manager.resolve_folder_sort(&plain_location, false),
                FolderSort::Default,
                "a sort equal to a new default is no longer stored"
            );
            assert_eq!(
                manager.icons_size_for(Some(&sorted_location)),
                96,
                "a size equal to a new default is no longer stored"
            );
            manager.set_browser_mode(BrowserMode::Columns);
            wait_until(|| sorting(&second) == Some((SortKey::Name, SortDirection::Ascending)));

            let size_ascending = (SortKey::Size, SortDirection::Ascending);
            manager.set_folder_sort(&plain_location, size_ascending.0, size_ascending.1);
            manager.set_remember_folder_views(false);
            manager.set_default_sort(size_ascending.0, size_ascending.1);
            manager.set_default_sort(SortKey::Name, SortDirection::Ascending);
            manager.set_remember_folder_views(true);
            assert_eq!(
                manager.resolve_folder_sort(&plain_location, false),
                FolderSort::Saved(
                    size_ascending.0,
                    size_ascending.1,
                    crate::model::NameComparison::default()
                ),
                "a default set while folder settings are off drops nothing"
            );
            manager.set_folder_icons_size(Some(&sorted_location), 160);
            manager.set_remember_folder_views(false);
            manager.set_default_sort(size_ascending.0, size_ascending.1);
            let notified_sizes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let notified = notified_sizes.clone();
            let observed = sorted_location.clone();
            manager.observe(std::rc::Rc::new(move || {
                notified
                    .borrow_mut()
                    .push(PreferenceManager::shared().icons_size_for(Some(&observed)));
            }));
            manager.set_remember_folder_views(true);
            assert_eq!(
                manager.resolve_folder_sort(&plain_location, false),
                FolderSort::Default,
                "turning folder settings back on drops values equal to the default"
            );
            assert_eq!(
                notified_sizes.borrow().first(),
                Some(&160),
                "the first notification already uses the folder's own size"
            );
        },
    );
}

#[test]
fn folder_settings_saved_by_another_process_are_merged_instead_of_overwritten() {
    gtk_test(
        "ui::window::tests::preferences::folder_views::folder_settings_saved_by_another_process_are_merged_instead_of_overwritten",
        || {
            use crate::model::{FolderSort, Location, SortDirection, SortKey};

            let here = tempfile::tempdir().expect("folder sorted here");
            let elsewhere = tempfile::tempdir().expect("folder sorted elsewhere");
            let (here, elsewhere) = (
                Location::local(here.path()),
                Location::local(elsewhere.path()),
            );
            let open = OpenWindow::open();
            open.content.browser.navigate_location(elsewhere.clone());
            wait_until(|| !column_loading(&open, 0));
            let manager = PreferenceManager::shared();
            assert_eq!(
                manager.resolve_folder_sort(&elsewhere, false),
                FolderSort::Default
            );

            let held = crate::storage::writer::hold();
            write_folder_views(&format!(
                "version = 1\n[[folder]]\npath = \"{}\"\nsort = \"size\"\ndirection = \"descending\"\n",
                elsewhere.display_path()
            ));
            manager.notice_saves_elsewhere();
            settle_for(std::time::Duration::from_millis(100));
            assert_eq!(
                manager.resolve_folder_sort(&elsewhere, false),
                FolderSort::Default,
                "another process's save is read on the settings writer"
            );
            drop(held);
            wait_until(|| {
                manager.resolve_folder_sort(&elsewhere, false)
                    == FolderSort::Saved(
                        SortKey::Size,
                        SortDirection::Descending,
                        crate::model::NameComparison::default(),
                    )
            });
            manager.set_folder_sort(&here, SortKey::Type, SortDirection::Ascending);
            manager.flush_folder_views();
            crate::storage::writer::settle();

            let saved = folder_views_file();
            assert!(saved.contains(&here.display_path()), "{saved}");
            assert!(saved.contains(&elsewhere.display_path()), "{saved}");
            assert_eq!(
                manager.resolve_folder_sort(&elsewhere, false),
                FolderSort::Saved(
                    SortKey::Size,
                    SortDirection::Descending,
                    crate::model::NameComparison::default()
                )
            );
            wait_until(|| {
                open.content
                    .browser
                    .browser()
                    .column_preferences(0)
                    .is_some_and(|preferences| preferences.sort_key == SortKey::Size)
            });

            for (sort_here, reread_first, saves) in
                [(SortKey::Modified, true, 1), (SortKey::Size, false, 2)]
            {
                let outside = tempfile::tempdir().expect("folder sorted elsewhere");
                let outside = Location::local(outside.path());
                let held = crate::storage::writer::hold();
                write_folder_views(&format!(
                    "{}[[folder]]\npath = \"{}\"\nsort = \"size\"\ndirection = \"ascending\"\n",
                    folder_views_file(),
                    outside.display_path()
                ));
                if reread_first {
                    manager.notice_saves_elsewhere();
                }
                manager.set_folder_sort(&here, sort_here, SortDirection::Ascending);
                for _ in 0..saves {
                    manager.flush_folder_views();
                }
                drop(held);
                crate::storage::writer::settle();
                assert_eq!(
                    manager.resolve_folder_sort(&outside, false),
                    FolderSort::Saved(
                        SortKey::Size,
                        SortDirection::Ascending,
                        crate::model::NameComparison::default()
                    )
                );
                let saved = folder_views_file();
                assert!(saved.contains(&outside.display_path()), "{saved}");
            }

            manager.forget_folder_views();
            manager.flush_folder_views();
            crate::storage::writer::settle();
            write_folder_views(&format!(
                "version = 1\n[[folder]]\npath = \"{}\"\nsort = \"type\"\ndirection = \"descending\"\n",
                elsewhere.display_path()
            ));
            manager.forget_folder_views();
            manager.flush_folder_views();
            crate::storage::writer::settle();
            assert!(
                !folder_views_file().contains("[[folder]]"),
                "forgetting also clears what another process saved"
            );
            assert_eq!(
                manager.resolve_folder_sort(&elsewhere, false),
                FolderSort::Default
            );

            for unsaveable in ["version = 2\n", "version = 1\n[[folder]\n"] {
                write_folder_views(unsaveable);
                manager.set_folder_sort(&here, SortKey::Size, SortDirection::Ascending);
                manager.flush_folder_views();
                crate::storage::writer::settle();
                assert_eq!(folder_views_file(), unsaveable);
            }
            write_folder_views("version = 1\n");
            manager.flush_folder_views();
            crate::storage::writer::settle();
            assert!(
                folder_views_file().contains(&here.display_path()),
                "the save is retried once the file can be read again"
            );

            let saving_elsewhere =
                std::fs::File::open(crate::storage::state_directory()).expect("state directory");
            saving_elsewhere.lock().expect("another process saving");
            manager.set_folder_sort(&here, SortKey::Type, SortDirection::Descending);
            settle_for(std::time::Duration::from_secs(1));
            assert!(
                !folder_views_file().contains("sort = \"type\""),
                "a save waits until another process has saved"
            );
            drop(saving_elsewhere);
            wait_until(|| folder_views_file().contains("sort = \"type\""));

            let stuck_elsewhere =
                std::fs::File::open(crate::storage::state_directory()).expect("state directory");
            stuck_elsewhere
                .lock()
                .expect("another process stopped mid-save");
            let (release, released) = std::sync::mpsc::channel::<()>();
            // Released anyway after a while, so a save that waits forever fails instead of hanging.
            let holder = std::thread::spawn(move || {
                let _held = stuck_elsewhere;
                let _ = released.recv_timeout(std::time::Duration::from_secs(5));
            });
            manager.set_folder_sort(&here, SortKey::Modified, SortDirection::Ascending);
            let started = std::time::Instant::now();
            manager.flush_folder_views();
            crate::storage::writer::settle();
            assert!(
                started.elapsed() < std::time::Duration::from_secs(4),
                "the exit save stops waiting for a stuck process"
            );
            assert!(folder_views_file().contains("sort = \"modified\""));
            let _ = release.send(());
            holder.join().expect("lock holder");

            let mut settings: toml::Table = std::fs::read_to_string(settings_file())
                .ok()
                .and_then(|contents| toml::from_str(&contents).ok())
                .unwrap_or_default();
            settings.insert("sort_key".into(), "modified".into());
            settings.insert("sort_direction".into(), "descending".into());
            let staged = settings_file().with_extension("toml.new");
            std::fs::create_dir_all(staged.parent().expect("config directory"))
                .expect("config directory");
            std::fs::write(&staged, toml::to_string(&settings).expect("settings"))
                .expect("settings saved elsewhere");
            let held = crate::storage::writer::hold();
            std::fs::rename(&staged, settings_file()).expect("settings replaced");
            manager.notice_saves_elsewhere();
            manager.set_folder_peeking(!manager.folder_peeking());
            settle_for(std::time::Duration::from_millis(100));
            assert_eq!(
                manager.default_sort(),
                (SortKey::Name, SortDirection::Ascending),
                "defaults saved elsewhere are read on the settings writer"
            );
            drop(held);
            wait_until(|| manager.default_sort() == (SortKey::Modified, SortDirection::Descending));
            crate::storage::writer::settle();
            assert!(
                std::fs::read_to_string(settings_file())
                    .expect("settings")
                    .contains("sort_key = \"modified\""),
                "a save that was in flight keeps the defaults saved elsewhere"
            );
            manager.set_folder_sort(&here, SortKey::Name, SortDirection::Ascending);
            assert_eq!(
                manager.resolve_folder_sort(&here, false),
                FolderSort::Saved(
                    SortKey::Name,
                    SortDirection::Ascending,
                    crate::model::NameComparison::default()
                ),
                "a sort equal to a default replaced elsewhere is still stored"
            );
        },
    );
}

#[test]
fn unreadable_folder_settings_use_defaults_and_are_never_overwritten() {
    gtk_test(
        "ui::window::tests::preferences::folder_views::unreadable_folder_settings_use_defaults_and_are_never_overwritten",
        || {
            use crate::model::{Location, SortDirection, SortKey};

            let unreadable = "version = 1\n[[folder]\npath = \"/broken\"\n";
            write_folder_views(unreadable);
            let open = OpenWindow::open();
            let folder = load_folder(&open);
            let browser = open.content.browser.browser();
            assert_eq!(
                browser
                    .column_preferences(0)
                    .map(|preferences| preferences.sort_key),
                Some(SortKey::Name)
            );

            browser.set_sort(0, SortKey::Size, SortDirection::Descending);
            wait_until(|| {
                browser
                    .column_preferences(0)
                    .map(|preferences| preferences.sort_key)
                    == Some(SortKey::Size)
            });
            open.content
                .browser
                .navigate_location(Location::local(folder.path()));
            wait_until(|| !column_loading(&open, 0));

            assert_eq!(
                browser
                    .column_preferences(0)
                    .map(|preferences| preferences.sort_key),
                Some(SortKey::Size),
                "changes still apply until Strata closes"
            );
            PreferenceManager::shared().flush_folder_views();
            crate::storage::writer::settle();
            assert_eq!(folder_views_file(), unreadable);
        },
    );
}

#[test]
fn name_comparison_is_restored_before_settings_and_follows_folders_across_views() {
    gtk_test(
        "ui::window::tests::preferences::folder_views::name_comparison_is_restored_before_settings_and_follows_folders_across_views",
        || {
            use crate::model::{Location, NameComparison};
            let folder = tempfile::tempdir().expect("folder");
            for name in ["a2.md", "a10.md", "Z.md"] {
                std::fs::write(folder.path().join(name), "same size").expect("file");
            }
            let location = Location::local(folder.path());
            write_folder_views(&format!(
                "version = 1\n[[folder]]\npath = \"{}\"\nsort = \"name\"\ndirection = \"ascending\"\nnatural_sort = false\ncase_sensitive_sort = true\n",
                folder.path().display()
            ));
            let manager = PreferenceManager::shared();
            let first = OpenWindow::open();
            let second = OpenWindow::open();
            for open in [&first, &second] {
                open.content.browser.navigate_location(location.clone());
            }
            let names = |open: &OpenWindow| {
                let browser = open.content.browser.browser();
                (0..3)
                    .filter_map(|index| browser.entry_at(0, index))
                    .map(|entry| entry.display_name)
                    .collect::<Vec<_>>()
            };
            wait_until(|| {
                names(&first) == ["Z.md", "a10.md", "a2.md"]
                    && names(&second) == ["Z.md", "a10.md", "a2.md"]
            });
            first
                .content
                .browser
                .browser()
                .set_name_comparison(0, NameComparison::default());
            wait_until(|| {
                names(&first) == ["a2.md", "a10.md", "Z.md"]
                    && names(&second) == ["a2.md", "a10.md", "Z.md"]
            });
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                manager.set_browser_mode(mode);
                wait_until(|| names(&second) == ["a2.md", "a10.md", "Z.md"]);
                assert_eq!(
                    second
                        .content
                        .browser
                        .browser()
                        .column_preferences(0)
                        .expect("preferences")
                        .name_comparison,
                    NameComparison::default()
                );
            }
            manager.flush_folder_views();
            crate::storage::writer::settle();
            let saved = folder_views_file();
            assert!(!saved.contains("natural_sort = false"));
            assert!(!saved.contains("case_sensitive_sort = true"));
        },
    );
}
