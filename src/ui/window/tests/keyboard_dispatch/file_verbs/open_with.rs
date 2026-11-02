// SPDX-License-Identifier: MIT

use super::*;

fn fill_text_and_picture(fixture: &KeyboardFixture) {
    let browser = fixture.view.browser();
    move_to_named(fixture, &browser, "a.txt");
    plain(fixture, Key::space);
    move_to_named(fixture, &browser, "picture.png");
    plain(fixture, Key::space);
    move_to_named(fixture, &browser, "b.txt");
}

fn lists_in(sections: &Sections, title: &str, app: &str) -> bool {
    sections
        .iter()
        .any(|(heading, apps)| heading == title && apps.contains(&app.to_owned()))
}

/// Only handlers shared by the whole fill are recommended; the rest are listed
/// under Other Applications.
fn assert_shared_handlers_recommended(sections: &Sections) {
    assert_eq!(sections[0].0, "Recommended Applications");
    let recommended = &sections[0].1;
    assert_eq!(
        (
            recommended.contains(&"Shared Viewer".to_owned()),
            recommended.contains(&"Text Only".to_owned()),
            lists_in(sections, "Other Applications", "Text Only"),
        ),
        (true, false, true),
        "(Shared Viewer recommended, Text Only recommended, Text Only among the others): \
         {sections:?}"
    );
}

#[test]
fn tenxer_open_with_offers_shared_handlers_for_the_fill() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::open_with::tenxer_open_with_offers_shared_handlers_for_the_fill",
        || {
            let output = glib::user_data_dir().join("received");
            // GIO indexes applications on first use.
            recorder_app(
                "strata-shared",
                "Shared Viewer",
                "text/plain;image/png;",
                &output,
            );
            recorder_app("strata-text", "Text Only", "text/plain;", &output);
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            std::fs::write(directory.join("picture.png"), b"\x89PNG\r\n\x1a\n").expect("png");
            enable_tenxer(&fixture);
            refresh(&fixture, "picture.png");

            fill_text_and_picture(&fixture);
            shifted(&fixture, Key::O);
            assert!(
                !modal_visible(&fixture.overlay),
                "the lookup does not block"
            );
            wait_until(|| modal_visible(&fixture.overlay));
            assert_shared_handlers_recommended(&chooser_sections(&fixture.overlay));
            close_dialog(&fixture);
            assert!(!output.exists(), "cancelling launches nothing");

            focus_files(&fixture);
            shifted(&fixture, Key::O);
            wait_until(|| modal_visible(&fixture.overlay));
            assert!(click_class(&fixture.overlay, "action-dialog-confirm"));
            wait_until(|| received(&output).len() == 2);
            let mut opened = received(&output);
            opened.sort();
            assert_eq!(
                opened,
                [directory.join("a.txt"), directory.join("picture.png")]
            );
        },
    );
}

/// Starts an Open With lookup on a.txt, interrupts it with `interrupt`, and
/// checks that no chooser appears.
fn interrupted_lookup_shows_nothing(fixture: &KeyboardFixture, interrupt: impl FnOnce()) {
    move_to_named(fixture, &fixture.view.browser(), "a.txt");
    shifted(fixture, Key::O);
    interrupt();
    pump(300);
    assert!(!modal_visible(&fixture.overlay));
}

#[test]
fn tenxer_open_with_drops_a_lookup_overtaken_by_a_newer_interaction() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::open_with::tenxer_open_with_drops_a_lookup_overtaken_by_a_newer_interaction",
        || {
            recorder_app(
                "strata-text",
                "Text Only",
                "text/plain;",
                &glib::user_data_dir().join("out"),
            );
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            std::fs::create_dir(directory.join("folder")).expect("folder");
            std::os::unix::fs::symlink(directory.join("missing"), directory.join("link"))
                .expect("dangling link");
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            refresh(&fixture, "link");

            interrupted_lookup_shows_nothing(&fixture, || plain(&fixture, Key::j));
            interrupted_lookup_shows_nothing(&fixture, || browser.select(0, 1));
            interrupted_lookup_shows_nothing(&fixture, || {
                browser.navigate(Location::local(directory.join("folder")));
            });
            browser.navigate(Location::local(&directory));
            wait_loaded(&browser, 0);
            focus_files(&fixture);
            interrupted_lookup_shows_nothing(&fixture, || {
                PreferenceManager::shared().set_tenxer_mode(false);
            });
            PreferenceManager::shared().set_tenxer_mode(true);
            pump(50);
            interrupted_lookup_shows_nothing(&fixture, || {
                fixture.sidebar.state.focus_active_place();
            });

            move_to_named(&fixture, &browser, "link");
            shifted(&fixture, Key::O);
            wait_until(|| !fixture.shortcuts.feedback_text().is_empty());
            assert_eq!(
                feedback(&fixture),
                "Broken symbolic links cannot be opened with an application"
            );
            assert!(!modal_visible(&fixture.overlay));

            open_empty_folder(&fixture);
            shifted(&fixture, Key::O);
            assert_eq!(feedback(&fixture), "Nothing to open");
        },
    );
}

fn recorded_apps(content_type: &str) -> Vec<String> {
    gtk::glib::MainContext::default()
        .block_on(crate::ui::recent_apps::read())
        .expect("recent-app history")
        .recent_ids(&[content_type.to_owned()])
}

#[test]
fn default_activation_records_application_history_for_files_and_folders() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::open_with::default_activation_records_application_history_for_files_and_folders",
        || {
            let output_alpha = gtk::glib::user_data_dir().join("received-alpha");
            let output_dirs = gtk::glib::user_data_dir().join("received-dirs");
            recorder_app("strata-alpha", "Alpha Editor", "text/plain;", &output_alpha);
            recorder_app(
                "strata-dirs",
                "Dir Handler",
                "inode/directory;",
                &output_dirs,
            );
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();
            enable_tenxer(&fixture);

            launch_recorder("strata-dirs", &[(&directory, "inode/directory")]);
            wait_until(|| received(&output_dirs) == [directory.clone()]);
            crate::storage::writer::settle();
            assert_eq!(recorded_apps("inode/directory"), ["strata-dirs.desktop"]);
            assert!(recorded_apps("text/plain").is_empty());

            make_default_text_editor("strata-alpha");
            move_to_named(&fixture, &browser, "a.txt");
            plain(&fixture, Key::Return);
            wait_until(|| received(&output_alpha) == [directory.join("a.txt")]);
            wait_until(|| recorded_apps("text/plain") == ["strata-alpha.desktop"]);
        },
    );
}

fn visible_choosers(overlay: &gtk::Overlay) -> usize {
    widget_children(overlay.upcast_ref())
        .filter(|widget| {
            widget.is_visible() && widget_with_class(widget, "open-with-dialog").is_some()
        })
        .count()
}

#[test]
fn activation_fallback_opens_one_chooser_after_the_history_read() {
    let no_applications = tempfile::tempdir().expect("data dirs without applications");
    crate::test_support::gtk_test_with_env(
        "ui::window::tests::keyboard_dispatch::file_verbs::open_with::activation_fallback_opens_one_chooser_after_the_history_read",
        [("XDG_DATA_DIRS", no_applications.path())],
        || {
            let fixture = KeyboardFixture::new();
            let browser = fixture.view.browser();
            let choosers = || visible_choosers(&fixture.overlay);

            move_to_named(&fixture, &browser, "a.txt");
            let writer = crate::storage::writer::hold();
            plain(&fixture, Key::Return);
            plain(&fixture, Key::Return);
            pump(200);
            assert_eq!(choosers(), 0, "the chooser waits for the history read");
            drop(writer);
            wait_until(|| choosers() > 0);
            crate::storage::writer::settle();
            pump(200);
            assert_eq!(
                choosers(),
                1,
                "a repeated activation stacks no second chooser"
            );
        },
    );
}
