// SPDX-License-Identifier: MIT

use super::*;

use std::time::{Duration, Instant};

struct TabFixture {
    window: gtk::Window,
    outer: gtk::Overlay,
    overlay: gtk::Overlay,
    launcher: SettingsLauncher,
    shortcuts: crate::ui::shortcut_footer::ShortcutFooter,
}

fn tab_fixture() -> TabFixture {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let blurred_root = BlurBin::new(&content);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&blurred_root));
    let outer = gtk::Overlay::new();
    outer.add_css_class("tab-window");
    outer.set_child(Some(&overlay));
    let window = gtk::Window::builder().child(&outer).build();
    let shortcuts = crate::ui::shortcut_footer::ShortcutFooter::new(
        crate::ui::browser_modes::BrowserMode::Columns,
    );
    let view = crate::ui::browser::BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        crate::ui::browser::PeekBehavior::default(),
    );
    let launcher = SettingsLauncher {
        layer: RefCell::new(None),
        focus_restore: RefCell::new(None),
        target: crate::ui::browser::PaneTarget::new(&view).downgrade(),
        button: gtk::Button::new(),
        blurred_root,
        overlay: overlay.clone(),
        preferences: PreferenceManager::shared(),
        notice: Rc::new(|_| {}),
        guard: settings::install_guard(),
        cancel_chords: {
            let shortcuts = shortcuts.clone();
            Rc::new(move || shortcuts.cancel_chord())
        },
    };
    TabFixture {
        window,
        outer,
        overlay,
        launcher,
        shortcuts,
    }
}

fn run_until(condition: impl Fn() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn only_the_first_tab_shown_prepares_a_settings_layer_that_stays_closed() {
    crate::test_support::gtk_test(
        "ui::window::composition::settings::tests::only_the_first_tab_shown_prepares_a_settings_layer_that_stays_closed",
        || {
            crate::ui::prepare_portal_ui();
            let first = tab_fixture();
            let second = tab_fixture();
            let first_launcher = Rc::new(first.launcher);
            let second_launcher = Rc::new(second.launcher);
            prepare_after_first_paint(&first_launcher);
            prepare_after_first_paint(&second_launcher);

            first.window.present();
            let focus = gtk::prelude::RootExt::focus(&first.window);
            let prepared_child = || {
                first_launcher
                    .layer
                    .borrow()
                    .as_ref()
                    .and_then(|layer| layer.widget.first_child())
            };
            run_until(
                || prepared_child().is_some_and(|child| child.width() > 0),
                "the layer's layout to be computed before it opens",
            );
            let layer = first_launcher
                .layer
                .borrow()
                .as_ref()
                .map(|layer| layer.widget.clone())
                .expect("prepared layer");
            assert!(!layer.is_visible());
            assert!(!layer.is_mapped());
            assert!(super::super::super::visible_modal_layer(&first.window).is_none());
            assert_eq!(gtk::prelude::RootExt::focus(&first.window), focus);

            second.window.present();
            run_until(|| second.window.is_mapped(), "the second tab to be shown");
            let settled = Instant::now() + Duration::from_millis(200);
            run_until(|| Instant::now() >= settled, "later frames");
            assert!(second_launcher.layer.borrow().is_none());

            first_launcher.show();
            assert!(layer.is_mapped());
            assert_eq!(
                first_launcher
                    .layer
                    .borrow()
                    .as_ref()
                    .map(|layer| layer.widget.clone()),
                Some(layer.clone()),
                "opening should reuse the prepared layer"
            );
            first.window.destroy();
            second.window.destroy();
        },
    );
}

#[test]
fn settings_launcher_refuses_to_open_over_another_modal() {
    crate::test_support::gtk_test(
        "ui::window::composition::settings::tests::settings_launcher_refuses_to_open_over_another_modal",
        || {
            crate::ui::prepare_portal_ui();
            let TabFixture {
                window,
                outer,
                overlay,
                launcher,
                shortcuts,
            } = tab_fixture();
            let action = crate::ui::modal::modal_layer(
                &gtk::Button::with_label("Action"),
                &overlay,
                Some(launcher.blurred_root.clone()),
                None,
            );
            outer.add_overlay(&action);
            window.present();
            action.grab_focus();
            let focus = gtk::prelude::RootExt::focus(&window);

            launcher.show();
            assert!(launcher.layer.borrow().is_none());
            assert!(!launcher.button.has_css_class("active"));
            assert_eq!(gtk::prelude::RootExt::focus(&window), focus);

            outer.remove_overlay(&action);
            shortcuts.arm_chord(crate::ui::tenxer_mode::Chord::Go);
            launcher.show();
            assert_eq!(shortcuts.armed_chord(), None);
            let settings = launcher
                .layer
                .borrow()
                .as_ref()
                .map(|layer| layer.widget.clone())
                .expect("Settings layer");
            assert!(settings.is_visible());
            assert!(launcher.button.has_css_class("active"));
            settings.set_visible(false);
            launcher.button.remove_css_class("active");
            outer.add_overlay(&action);
            action.grab_focus();
            let focus = gtk::prelude::RootExt::focus(&window);

            launcher.show();
            assert!(!settings.is_visible());
            assert!(!launcher.button.has_css_class("active"));
            assert_eq!(gtk::prelude::RootExt::focus(&window), focus);
            assert_eq!(outer.last_child(), Some(action.clone().upcast()));

            action.set_visible(false);
            launcher.show();
            assert!(
                settings.is_visible(),
                "hidden modals must not block Settings"
            );
            assert_eq!(
                launcher.layer.borrow().as_ref().map(|layer| &layer.widget),
                Some(&settings)
            );
            window.destroy();
        },
    );
}
