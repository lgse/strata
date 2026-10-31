// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn closed_tabs_reopen_in_reverse_order_at_their_previous_positions() {
    gtk_test(
        "ui::window::composition::tabs::tests::reopen::closed_tabs_reopen_in_reverse_order_at_their_previous_positions",
        || {
            let root = tempfile::tempdir().expect("closed tab fixture");
            let locations: Vec<_> = ["alpha", "beta", "gamma"]
                .into_iter()
                .map(|name| {
                    let path = root.path().join(name);
                    std::fs::create_dir(&path).expect("tab directory");
                    Location::local(path)
                })
                .collect();
            let app = application();
            let (window, tabs) = open_in(&app);
            load(&tabs.active_browser(), locations[0].clone());
            tabs.new_tab();
            load(&tabs.active_browser(), locations[1].clone());
            let middle = tabs.active.get();
            tabs.new_tab();
            load(&tabs.active_browser(), locations[2].clone());
            let last = tabs.active.get();
            tabs.close(middle);
            assert_eq!(
                tabs.active.get(),
                last,
                "closing an inactive tab keeps focus"
            );
            tabs.close(last);
            assert_eq!(tab_locations(&tabs), vec![Some(locations[0].clone())]);

            let (other_window, other) = open_in(&app);
            gio::prelude::ActionGroupExt::activate_action(&other_window, "reopen-tab", None);
            assert_eq!(tab_locations(&other), vec![None], "history is window-local");
            other_window.destroy();

            for location in [&locations[2], &locations[1]] {
                let shortcut = press(
                    gdk::Key::T,
                    gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
                );
                assert!(is_tab_shortcut(&shortcut));
                assert_eq!(tabs.handle_key(&shortcut), glib::Propagation::Stop);
                wait_until(|| {
                    tabs.active_browser().browser().active_location().as_ref() == Some(location)
                });
            }
            assert_eq!(
                tab_locations(&tabs),
                locations.into_iter().map(Some).collect::<Vec<_>>()
            );
            let active = tabs.active.get();
            gio::prelude::ActionGroupExt::activate_action(&window, "reopen-tab", None);
            assert_eq!(
                tabs.tabs.borrow().len(),
                3,
                "empty history does not create a tab"
            );
            assert_eq!(tabs.active.get(), active);
            window.destroy();
        },
    );
}

#[test]
fn modal_input_does_not_consume_closed_tab_history() {
    gtk_test(
        "ui::window::composition::tabs::tests::reopen::modal_input_does_not_consume_closed_tab_history",
        || {
            let root = tempfile::tempdir().expect("closed tab fixture");
            let (window, tabs) = open();
            load(&tabs.active_browser(), Location::local(root.path()));
            tabs.new_tab();
            load(&tabs.active_browser(), Location::local(root.path()));
            tabs.close(tabs.active.get());
            crate::ui::modal::show_error_dialog(&window, "Test modal", "Modal owns input");
            assert!(tabs.blocked());
            assert_eq!(
                tabs.handle_key(&press(
                    gdk::Key::T,
                    gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
                )),
                glib::Propagation::Proceed
            );
            gio::prelude::ActionGroupExt::activate_action(&window, "reopen-tab", None);
            assert_eq!(tabs.tabs.borrow().len(), 1);
            button_with_label(window.upcast_ref(), "Close")
                .expect("modal dismiss button")
                .emit_clicked();
            wait_until(|| !tabs.blocked());
            gio::prelude::ActionGroupExt::activate_action(&window, "reopen-tab", None);
            wait_until(|| {
                tabs.active_browser().browser().active_location()
                    == Some(Location::local(root.path()))
            });
            assert_eq!(
                tabs.tabs.borrow().len(),
                2,
                "blocked attempts preserve history"
            );
            window.destroy();
        },
    );
}

#[test]
fn reopening_a_deleted_directory_keeps_a_retryable_tab() {
    gtk_test(
        "ui::window::composition::tabs::tests::reopen::reopening_a_deleted_directory_keeps_a_retryable_tab",
        || {
            let (root, window, tabs, location) = open_target_fixture();
            tabs.new_tab();
            load(&tabs.active_browser(), location.clone());
            tabs.close(tabs.active.get());
            std::fs::remove_dir(root.path().join("destination")).expect("remove closed directory");
            tabs.reopen_closed_tab();
            let view = tabs.active_browser();
            wait_until(|| button_with_label(view.overlay().upcast_ref(), "Retry").is_some());
            assert_eq!(tabs.tabs.borrow().len(), 2);
            std::fs::create_dir(root.path().join("destination")).expect("restore directory");
            button_with_label(view.overlay().upcast_ref(), "Retry")
                .expect("retry unavailable directory")
                .emit_clicked();
            wait_until(|| view.browser().active_location() == Some(location.clone()));
            window.destroy();
        },
    );
}
