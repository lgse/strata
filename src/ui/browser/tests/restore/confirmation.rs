// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn cancelling_confirmation_does_not_restore() {
    crate::test_support::gtk_test(
        "ui::browser::tests::restore::confirmation::cancelling_confirmation_does_not_restore",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let destination = fixture.path().join("restored");
            let (entry, info) = trashed_entry(fixture.path(), "safe", &destination);
            let source = entry.thumbnail_path.clone().expect("source");
            let view = view();
            let window = window(&view);
            view.state.request_restore(vec![entry]);
            wait_until(|| button(&window.clone().upcast(), "Restore").is_some());
            button(&window.clone().upcast(), "Cancel")
                .expect("cancel confirmation")
                .emit_clicked();
            assert!(button(&window.clone().upcast(), "Restore").is_none());
            assert!(source.exists());
            assert!(info.exists());
            assert!(!destination.exists());
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

#[test]
fn changing_metadata_after_confirmation_is_presented_refuses_the_move() {
    crate::test_support::gtk_test(
        "ui::browser::tests::restore::confirmation::changing_metadata_after_confirmation_is_presented_refuses_the_move",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let confirmed = fixture.path().join("confirmed");
            let changed = fixture.path().join("changed");
            let (entry, info) = trashed_entry(fixture.path(), "safe", &confirmed);
            let source = entry.thumbnail_path.clone().expect("source");
            let view = view();
            let window = window(&view);
            view.state.request_restore(vec![entry]);
            wait_until(|| button(&window.clone().upcast(), "Restore").is_some());
            fs::write(&info, format!("[Trash Info]\nPath={}\n", changed.display()))
                .expect("change metadata");
            button(&window.clone().upcast(), "Restore")
                .expect("confirm")
                .emit_clicked();
            wait_until(|| {
                find_widget(&window.clone().upcast(), &|label: &gtk::Label| {
                    label
                        .text()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .contains("no longer matches the confirmed destination")
                })
                .is_some()
            });
            assert!(source.exists());
            assert!(info.exists());
            assert!(!confirmed.exists());
            assert!(!changed.exists());
            window.destroy();
            view.browser().clear_observer();
        },
    );
}
