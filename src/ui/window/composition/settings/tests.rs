// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn settings_launcher_refuses_to_open_over_another_modal() {
    crate::test_support::gtk_test(
        "ui::window::composition::settings::tests::settings_launcher_refuses_to_open_over_another_modal",
        || {
            crate::ui::prepare_portal_ui();
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let blurred_root = BlurBin::new(&content);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&blurred_root));
            let window = gtk::Window::builder().child(&overlay).build();
            let button = gtk::Button::new();
            let launcher = SettingsLauncher {
                layer: RefCell::new(None),
                button: button.downgrade(),
                blurred_root: blurred_root.downgrade(),
                overlay: overlay.downgrade(),
                preferences: PreferenceManager::shared(),
                notice: Rc::new(|_| {}),
                guard: settings::install_guard(),
                capture_focus: Rc::new(|| Rc::new(|| {})),
                restore_focus: Rc::default(),
            };
            let action = crate::ui::modal::modal_layer(
                &gtk::Button::with_label("Action"),
                &overlay,
                Some(blurred_root.clone()),
                None,
            );
            overlay.add_overlay(&action);
            window.present();
            action.grab_focus();
            let focus = gtk::prelude::RootExt::focus(&window);

            launcher.show();
            assert!(launcher.layer.borrow().is_none());
            assert!(!button.has_css_class("active"));
            assert_eq!(gtk::prelude::RootExt::focus(&window), focus);

            overlay.remove_overlay(&action);
            launcher.show();
            let settings = launcher.layer().expect("Settings layer");
            assert!(settings.is_visible());
            assert!(button.has_css_class("active"));
            settings.set_visible(false);
            button.remove_css_class("active");
            overlay.add_overlay(&action);
            action.grab_focus();
            let focus = gtk::prelude::RootExt::focus(&window);

            launcher.show();
            assert!(!settings.is_visible());
            assert!(!button.has_css_class("active"));
            assert_eq!(gtk::prelude::RootExt::focus(&window), focus);
            assert_eq!(overlay.last_child(), Some(action.clone().upcast()));

            action.set_visible(false);
            launcher.show();
            assert!(
                settings.is_visible(),
                "hidden modals must not block Settings"
            );
            assert_eq!(launcher.layer(), Some(settings));
            window.destroy();
        },
    );
}
