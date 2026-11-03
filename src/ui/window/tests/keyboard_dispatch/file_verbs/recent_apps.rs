// SPDX-License-Identifier: MIT

use super::*;

const RECENT: &str = "Recently Used";
const RECOMMENDED: &str = "Recommended Applications";

fn data_file(name: &str) -> PathBuf {
    gtk::glib::user_data_dir().join(name)
}

/// Installs Alpha Editor and Beta Editor for text; returns their recorder outputs.
fn install_editors() -> (PathBuf, PathBuf) {
    let output_alpha = data_file("received-alpha");
    let output_beta = data_file("received-beta");
    recorder_app("strata-alpha", "Alpha Editor", "text/plain;", &output_alpha);
    recorder_app("strata-beta", "Beta Editor", "text/plain;", &output_beta);
    (output_alpha, output_beta)
}

fn assert_section(sections: &Sections, index: usize, title: &str, apps: &[&str]) {
    assert_eq!(sections[index].0, title, "{sections:?}");
    assert_eq!(sections[index].1, apps, "{sections:?}");
}

fn assert_no_recently_used(sections: &Sections) {
    assert!(
        !sections.iter().any(|(title, _)| title == RECENT),
        "{sections:?}"
    );
}

fn assert_listed_only_in_recently_used(sections: &Sections, app: &str) {
    assert!(
        sections
            .iter()
            .skip(1)
            .all(|(_, apps)| !apps.contains(&app.to_owned())),
        "{sections:?}"
    );
}

fn launch_with_text(id: &str, file: &Path) {
    launch_recorder(id, &[(file, "text/plain")]);
}

/// The chooser opens only once the history read the held writer blocks completes.
fn chooser_waits_for_the_history_read(fixture: &KeyboardFixture) {
    let writer = crate::storage::writer::hold();
    fixture.view.browser().clear_active_selection();
    move_to_named(fixture, &fixture.view.browser(), "b.txt");
    plain(fixture, Key::space);
    shifted(fixture, Key::O);
    pump(200);
    assert!(
        !modal_visible(&fixture.overlay),
        "the chooser waits for the history read"
    );
    drop(writer);
    wait_until(|| modal_visible(&fixture.overlay));
}

#[test]
fn open_with_shows_recently_used_first_and_excludes_it_from_recommended() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::recent_apps::open_with_shows_recently_used_first_and_excludes_it_from_recommended",
        || {
            let (output_alpha, output_beta) = install_editors();
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();
            enable_tenxer(&fixture);

            open_single_with_dialog(&fixture, &browser, "a.txt");
            assert_no_recently_used(&chooser_sections(&fixture.overlay));
            close_dialog(&fixture);
            assert!(
                !output_alpha.exists() && !output_beta.exists(),
                "cancelling launches nothing"
            );

            launch_with_text("strata-beta", &directory.join("a.txt"));
            wait_until(|| received(&output_beta) == [directory.join("a.txt")]);
            chooser_waits_for_the_history_read(&fixture);
            let sections = chooser_sections(&fixture.overlay);
            assert_section(&sections, 0, RECENT, &["Beta Editor"]);
            assert_listed_only_in_recently_used(&sections, "Beta Editor");

            assert!(click_class(&fixture.overlay, "action-dialog-confirm"));
            wait_until(|| received(&output_beta) == [directory.join("b.txt")]);
            wait_until(|| !modal_visible(&fixture.overlay));

            launch_with_text("strata-alpha", &directory.join("a.txt"));
            open_single_with_dialog(&fixture, &browser, "a.txt");
            let sections = chooser_sections(&fixture.overlay);
            assert_eq!(
                sections[0].1,
                ["Alpha Editor", "Beta Editor"],
                "{sections:?}"
            );
            close_dialog(&fixture);
        },
    );
}

fn write_picture(directory: &Path) {
    std::fs::write(directory.join("picture.png"), b"\x89PNG\r\n\x1a\n").expect("png");
}

#[test]
fn open_with_keeps_independent_histories_per_file_type() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::recent_apps::open_with_keeps_independent_histories_per_file_type",
        || {
            recorder_app(
                "strata-alpha",
                "Alpha Editor",
                "text/plain;",
                &data_file("received-alpha"),
            );
            recorder_app(
                "strata-gamma",
                "Gamma Viewer",
                "image/png;",
                &data_file("received-gamma"),
            );
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            write_picture(&directory);
            let browser = fixture.view.browser();
            enable_tenxer(&fixture);
            refresh(&fixture, "picture.png");

            launch_with_text("strata-alpha", &directory.join("a.txt"));
            launch_recorder(
                "strata-gamma",
                &[(&directory.join("picture.png"), "image/png")],
            );

            for (name, app) in [("b.txt", "Alpha Editor"), ("picture.png", "Gamma Viewer")] {
                open_single_with_dialog(&fixture, &browser, name);
                assert_section(&chooser_sections(&fixture.overlay), 0, RECENT, &[app]);
                close_dialog(&fixture);
            }
        },
    );
}

#[test]
fn open_with_recently_used_intersects_multi_type_selections() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::recent_apps::open_with_recently_used_intersects_multi_type_selections",
        || {
            let output = data_file("received");
            recorder_app(
                "strata-shared",
                "Shared Viewer",
                "text/plain;image/png;",
                &output,
            );
            recorder_app("strata-text", "Text Only", "text/plain;", &output);
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            write_picture(&directory);
            let browser = fixture.view.browser();
            enable_tenxer(&fixture);
            refresh(&fixture, "picture.png");

            launch_recorder(
                "strata-shared",
                &[
                    (&directory.join("a.txt"), "text/plain"),
                    (&directory.join("picture.png"), "image/png"),
                ],
            );
            launch_with_text("strata-text", &directory.join("a.txt"));

            move_to_named(&fixture, &browser, "a.txt");
            plain(&fixture, Key::space);
            move_to_named(&fixture, &browser, "picture.png");
            plain(&fixture, Key::space);
            shifted(&fixture, Key::O);
            wait_until(|| modal_visible(&fixture.overlay));
            assert_section(
                &chooser_sections(&fixture.overlay),
                0,
                RECENT,
                &["Shared Viewer"],
            );
            close_dialog(&fixture);

            open_single_with_dialog(&fixture, &browser, "b.txt");
            let sections = chooser_sections(&fixture.overlay);
            assert_eq!(
                sections[0].1,
                ["Text Only", "Shared Viewer"],
                "{sections:?}"
            );
            close_dialog(&fixture);
        },
    );
}

#[test]
fn open_with_recently_used_surfaces_launched_apps_without_declared_handler() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::recent_apps::open_with_recently_used_surfaces_launched_apps_without_declared_handler",
        || {
            let output = data_file("received-oddball");
            recorder_app("strata-oddball", "Oddball", "image/png;", &output);
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();
            enable_tenxer(&fixture);

            let app = installed_app("image/png", "strata-oddball.desktop");
            crate::ui::open_with::launch(
                &app,
                std::slice::from_ref(&gtk::gio::File::for_path(directory.join("a.txt"))),
                std::slice::from_ref(&"text/plain".to_string()),
                None::<&gtk::gio::AppLaunchContext>,
            )
            .expect("fake launch records history");
            wait_until(|| received(&output) == [directory.join("a.txt")]);

            open_single_with_dialog(&fixture, &browser, "b.txt");
            let sections = chooser_sections(&fixture.overlay);
            assert_section(&sections, 0, RECENT, &["Oddball"]);
            assert_listed_only_in_recently_used(&sections, "Oddball");
            close_dialog(&fixture);
        },
    );
}

#[test]
fn open_with_recently_used_caps_displayed_entries_at_five() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::recent_apps::open_with_recently_used_caps_displayed_entries_at_five",
        || {
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();
            enable_tenxer(&fixture);
            for index in 0..6 {
                let output = data_file(&format!("received-{index}"));
                recorder_app(
                    &format!("strata-cap{index}"),
                    &format!("Cap {index}"),
                    "text/plain;",
                    &output,
                );
                launch_with_text(&format!("strata-cap{index}"), &directory.join("a.txt"));
            }

            open_single_with_dialog(&fixture, &browser, "b.txt");
            assert_section(
                &chooser_sections(&fixture.overlay),
                0,
                RECENT,
                &["Cap 5", "Cap 4", "Cap 3", "Cap 2", "Cap 1"],
            );
            close_dialog(&fixture);
        },
    );
}

#[test]
fn open_with_hides_recently_used_for_uninstalled_applications() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::recent_apps::open_with_hides_recently_used_for_uninstalled_applications",
        || {
            recorder_app(
                "strata-gone",
                "Gone Editor",
                "text/plain;",
                &data_file("received"),
            );
            recorder_app(
                "strata-kept",
                "Kept Editor",
                "text/plain;",
                &data_file("received-kept"),
            );
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();
            enable_tenxer(&fixture);

            launch_with_text("strata-gone", &directory.join("a.txt"));
            open_single_with_dialog(&fixture, &browser, "b.txt");
            assert_eq!(
                chooser_sections(&fixture.overlay)[0].0,
                RECENT,
                "history recorded"
            );
            close_dialog(&fixture);

            std::fs::remove_file(data_file("applications/strata-gone.desktop"))
                .expect("uninstall fake application");
            wait_until(|| gio_unix::DesktopAppInfo::new("strata-gone.desktop").is_none());
            open_single_with_dialog(&fixture, &browser, "b.txt");
            assert_no_recently_used(&chooser_sections(&fixture.overlay));
            close_dialog(&fixture);
        },
    );
}

#[test]
fn open_with_keeps_the_default_first_when_recently_used_is_present() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::recent_apps::open_with_keeps_the_default_first_when_recently_used_is_present",
        || {
            install_editors();
            make_default_text_editor("strata-beta");
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();
            enable_tenxer(&fixture);

            launch_with_text("strata-alpha", &directory.join("a.txt"));
            open_single_with_dialog(&fixture, &browser, "b.txt");
            let sections = chooser_sections(&fixture.overlay);
            assert_section(&sections, 0, RECENT, &["Alpha Editor"]);
            assert_eq!(sections[1].0, RECOMMENDED, "{sections:?}");
            assert_eq!(
                sections[1].1.first().map(String::as_str),
                Some("Beta Editor"),
                "{sections:?}"
            );
            close_dialog(&fixture);
        },
    );
}

fn occurrences(sections: &Sections, app: &str) -> usize {
    sections
        .iter()
        .flat_map(|(_, apps)| apps)
        .filter(|name| *name == app)
        .count()
}

#[test]
fn open_with_deduplicates_the_default_application_inside_recently_used() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::recent_apps::open_with_deduplicates_the_default_application_inside_recently_used",
        || {
            install_editors();
            make_default_text_editor("strata-beta");
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();
            enable_tenxer(&fixture);

            open_single_with_dialog(&fixture, &browser, "a.txt");
            let baseline = chooser_sections(&fixture.overlay);
            assert_eq!(baseline[0].0, RECOMMENDED, "{baseline:?}");
            assert_eq!(
                baseline[0].1.first().map(String::as_str),
                Some("Beta Editor"),
                "{baseline:?}"
            );
            close_dialog(&fixture);

            launch_with_text("strata-beta", &directory.join("a.txt"));
            open_single_with_dialog(&fixture, &browser, "b.txt");
            let sections = chooser_sections(&fixture.overlay);
            assert_section(&sections, 0, RECENT, &["Beta Editor"]);
            let remaining: Vec<&str> = baseline[0]
                .1
                .iter()
                .map(String::as_str)
                .filter(|name| *name != "Beta Editor")
                .collect();
            assert_section(&sections, 1, RECOMMENDED, &remaining);
            assert_eq!(occurrences(&sections, "Beta Editor"), 1, "{sections:?}");
            close_dialog(&fixture);
        },
    );
}

fn search_applications(fixture: &KeyboardFixture, query: &str) {
    widget_with_class(fixture.overlay.upcast_ref(), "open-with-search")
        .expect("application search")
        .downcast_ref::<gtk::SearchEntry>()
        .expect("search entry")
        .set_text(query);
}

#[test]
fn open_with_search_hides_recently_used_without_match_and_confirms_selection() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::recent_apps::open_with_search_hides_recently_used_without_match_and_confirms_selection",
        || {
            let (_, output_beta) = install_editors();
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();
            enable_tenxer(&fixture);

            launch_with_text("strata-alpha", &directory.join("a.txt"));
            open_single_with_dialog(&fixture, &browser, "b.txt");
            assert_eq!(
                chooser_sections(&fixture.overlay)[0].0,
                RECENT,
                "history recorded"
            );

            search_applications(&fixture, "beta editor");
            pump(200);
            let visible = visible_chooser_sections(&fixture.overlay);
            assert!(
                visible.iter().all(|(title, _)| title != RECENT),
                "{visible:?}"
            );
            assert!(
                visible.iter().any(|(title, apps)| title == RECOMMENDED
                    && apps.contains(&"Beta Editor".to_owned())),
                "{visible:?}"
            );
            assert!(click_class(&fixture.overlay, "action-dialog-confirm"));
            wait_until(|| received(&output_beta) == [directory.join("b.txt")]);
            wait_until(|| !modal_visible(&fixture.overlay));
        },
    );
}

fn history_path() -> PathBuf {
    data_file("strata/recent-apps.toml")
}

#[test]
fn open_with_survives_malformed_history() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::recent_apps::open_with_survives_malformed_history",
        || {
            recorder_app(
                "strata-alpha",
                "Alpha Editor",
                "text/plain;",
                &data_file("received"),
            );
            std::fs::create_dir_all(history_path().parent().expect("history parent"))
                .expect("history directory");
            std::fs::write(history_path(), "not = [valid toml").expect("malformed history");
            let fixture = KeyboardFixture::new();
            let browser = fixture.view.browser();
            enable_tenxer(&fixture);

            open_single_with_dialog(&fixture, &browser, "a.txt");
            let sections = chooser_sections(&fixture.overlay);
            assert_no_recently_used(&sections);
            assert_eq!(sections[0].0, RECOMMENDED, "{sections:?}");
            close_dialog(&fixture);
        },
    );
}
