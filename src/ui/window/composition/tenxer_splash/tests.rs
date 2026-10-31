// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn splash_follows_live_enable_without_taking_focus_or_replaying_startup() {
    crate::test_support::gtk_test(
        "ui::window::composition::tenxer_splash::tests::splash_follows_live_enable_without_taking_focus_or_replaying_startup",
        || {
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(true);
            preferences.set_reduce_motion(true);
            let entry = gtk::Entry::new();
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&entry));
            let window = gtk::ApplicationWindow::builder()
                .child(&overlay)
                .default_width(640)
                .default_height(480)
                .build();
            let splash = install(&window, &overlay, &preferences);
            window.present();
            entry.grab_focus();
            let focus = gtk::prelude::RootExt::focus(&window);
            assert!(
                !splash.is_visible(),
                "saved startup state does not replay the splash"
            );
            preferences.set_tenxer_mode(false);
            preferences.set_tenxer_mode(true);
            assert!(splash.is_visible());
            assert!(splash.paintable().is_some(), "the bundled image decodes");
            assert!(!splash.can_target());
            assert!(!splash.is_focusable());
            assert!(splash.has_css_class("tenxer-splash-static"));
            assert_eq!(gtk::prelude::RootExt::focus(&window), focus);
            preferences.set_tenxer_mode(false);
            assert!(!splash.is_visible(), "turning off cancels the splash");
            preferences.set_tenxer_mode(true);
            let main_loop = glib::MainLoop::new(None, false);
            let stop = main_loop.clone();
            glib::timeout_add_local_once(Duration::from_millis(1600), move || stop.quit());
            main_loop.run();
            assert!(!splash.is_visible(), "the splash dismisses itself");
            assert_eq!(gtk::prelude::RootExt::focus(&window), focus);
            window.destroy();
        },
    );
}
