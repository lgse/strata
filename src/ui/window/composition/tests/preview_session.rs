// SPDX-License-Identifier: MIT

use super::*;

fn wait(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "preview session did not settle"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn toggle(widget: &gtk::Widget) -> Option<gtk::ToggleButton> {
    if widget.has_css_class("preview-panel-option") {
        return widget.clone().downcast().ok();
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = toggle(&widget) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn select(browser: &crate::app::Browser, depth: usize, name: &str) {
    let position = (0..browser.column_snapshot(depth).expect("column").count)
        .find(|position| {
            browser
                .entry_at(depth, *position)
                .is_some_and(|entry| entry.display_name == name)
        })
        .expect("entry");
    browser.select(depth, position);
}

#[test]
fn appearance_and_space_share_a_window_local_preview_session_across_unsupported_selections() {
    gtk_test(
        "ui::window::composition::tests::preview_session::appearance_and_space_share_a_window_local_preview_session_across_unsupported_selections",
        || {
            let directory = tempfile::tempdir().expect("session files");
            std::fs::create_dir(directory.path().join("folder")).expect("folder");
            for (path, data) in [
                ("a.txt", "alpha"),
                ("z.zip", "unsupported"),
                ("folder/nested.txt", "nested"),
            ] {
                std::fs::write(directory.path().join(path), data).expect("fixture file");
            }
            let first = Fixture::new();
            let second = Fixture::new();
            first.preferences.set_browser_mode(BrowserMode::Columns);
            first.preferences.set_reduce_motion(true);
            for fixture in [&first, &second] {
                fixture
                    .content
                    .browser
                    .browser()
                    .navigate(crate::model::Location::local(directory.path()));
                wait(|| {
                    fixture
                        .content
                        .browser
                        .browser()
                        .column_snapshot(0)
                        .is_some_and(|c| !c.loading)
                });
            }
            let first_toggle =
                toggle(first.content.header.content.upcast_ref()).expect("appearance toggle");
            let second_toggle =
                toggle(second.content.header.content.upcast_ref()).expect("other window toggle");
            assert!(!first_toggle.is_active());
            assert!(!second_toggle.is_active());
            let path = glib::user_config_dir().join("strata/settings.toml");
            let saved = std::fs::read(&path).expect("saved settings");
            let browser = first.content.browser.browser();
            select(&browser, 0, "z.zip");
            first_toggle.emit_clicked();
            assert!(first.content.preview.is_enabled());
            assert!(first_toggle.is_active());
            assert!(!first.content.preview.is_open());
            assert!(!second_toggle.is_active());

            select(&browser, 0, "a.txt");
            wait(|| first.content.preview.is_open());
            select(&browser, 0, "z.zip");
            assert!(first.content.preview.is_enabled());
            assert!(first_toggle.is_active());
            assert!(!first.content.preview.is_open());
            select(&browser, 0, "folder");
            browser.enter_focused_directory();
            wait(|| browser.column_snapshot(1).is_some_and(|c| !c.loading));
            select(&browser, 1, "nested.txt");
            wait(|| first.content.preview.is_open());
            first.content.preview.toggle(None, None);
            assert!(!first_toggle.is_active());
            browser.focus_active();
            assert!(!first.content.preview.is_open());
            assert_eq!(std::fs::read(path).expect("unchanged settings"), saved);
            assert!(!second.content.preview.is_enabled());
            first.close();
            second.close();
        },
    );
}
