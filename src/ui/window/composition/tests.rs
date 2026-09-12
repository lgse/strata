// SPDX-License-Identifier: MIT

use gtk::{
    gdk::{Key, ModifierType},
    glib,
};

use super::*;
use crate::{
    services::{BuildKind, ReleaseMetadata, UpdateMethod},
    test_support::gtk_test,
    ui::browser_modes::BrowserMode,
};

struct Fixture {
    window: gtk::ApplicationWindow,
    content: WindowContent,
    preferences: Rc<ThemeManager>,
    notice: UpdateNoticeHandler,
}

impl Fixture {
    fn new() -> Self {
        let preferences = ThemeManager::shared();
        let window = gtk::ApplicationWindow::builder()
            .application(&application())
            .default_width(1200)
            .default_height(760)
            .build();
        let content = WindowContent::new(&window, &preferences);
        let notice = content.bind(&window, &preferences);
        window.present();
        Self {
            window,
            content,
            preferences,
            notice,
        }
    }

    fn layer(&self, class: &str) -> Option<gtk::Widget> {
        let mut child = self.content.overlay.first_child();
        while let Some(widget) = child {
            if widget.has_css_class(class) {
                return Some(widget);
            }
            child = widget.next_sibling();
        }
        None
    }

    fn close(self) {
        self.content.connect_cleanup(&self.window);
        self.window.destroy();
    }
}

fn application() -> gtk::Application {
    if let Some(application) = gio::Application::default().and_downcast::<gtk::Application>() {
        return application;
    }
    let application = gtk::Application::new(None::<&str>, gio::ApplicationFlags::NON_UNIQUE);
    application
        .register(None::<&gio::Cancellable>)
        .expect("test application registration");
    application
}

#[test]
fn composition_initializes_live_preferences_before_settings_in_two_windows() {
    gtk_test(
        "ui::window::composition::tests::composition_initializes_live_preferences_before_settings_in_two_windows",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let first = Fixture::new();
            let second = Fixture::new();
            for fixture in [&first, &second] {
                assert!(fixture.layer("settings-backdrop").is_none());
                fixture
                    .content
                    .browser
                    .assert_saved_preferences(&fixture.preferences);
                assert_eq!(
                    fixture.content.footer.shortcuts.widget().is_visible(),
                    fixture.preferences.show_keybinding_hints()
                );
            }
            for enabled in [true, false, true] {
                first.preferences.set_show_keybinding_hints(enabled);
                first.preferences.set_single_click_previews(enabled);
                for mode in [BrowserMode::Icons, BrowserMode::List, BrowserMode::Columns] {
                    first.preferences.set_browser_mode(mode);
                    for fixture in [&first, &second] {
                        fixture
                            .content
                            .browser
                            .assert_saved_preferences(&fixture.preferences);
                        assert_eq!(fixture.content.browser.view_mode(), mode);
                        assert_eq!(
                            fixture.content.footer.shortcuts.widget().is_visible(),
                            enabled
                        );
                        assert!(fixture.layer("settings-backdrop").is_none());
                    }
                }
            }
            first.close();
            second.close();
        },
    );
}

#[test]
fn search_button_and_action_share_one_dialog_and_dismissal_state() {
    gtk_test(
        "ui::window::composition::tests::search_button_and_action_share_one_dialog_and_dismissal_state",
        || {
            let fixture = Fixture::new();
            let layer = fixture
                .layer("search-backdrop")
                .expect("search layer installed");
            let button = &fixture.content.header.search;
            let action = fixture
                .window
                .lookup_action("search")
                .expect("search action");
            assert!(!layer.is_visible());
            button.emit_clicked();
            assert!(layer.is_visible());
            assert!(button.has_css_class("active"));
            action.activate(None);
            wait_until_hidden(&layer);
            assert!(!button.has_css_class("active"));
            action.activate(None);
            assert!(layer.is_visible());
            button.emit_clicked();
            wait_until_hidden(&layer);
            assert!(!button.has_css_class("active"));
            assert_eq!(fixture.layer("search-backdrop"), Some(layer));
            fixture.close();
        },
    );
}

fn wait_until_hidden(layer: &gtk::Widget) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while layer.is_visible() {
        assert!(
            std::time::Instant::now() < deadline,
            "modal dismissal completes"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn window_actions_keep_default_accelerators() {
    gtk_test(
        "ui::window::composition::tests::window_actions_keep_default_accelerators",
        || {
            let fixture = Fixture::new();
            let application = fixture.window.application().expect("fixture application");
            for (name, expected) in super::super::DEFAULT_ACCELS {
                let installed = application.accels_for_action(name);
                assert_eq!(installed.len(), expected.len());
                for (actual, &expected) in installed.iter().zip(*expected) {
                    assert_eq!(
                        gtk::accelerator_parse(actual).expect("installed accelerator"),
                        gtk::accelerator_parse(expected).expect("default accelerator")
                    );
                }
            }
            fixture.close();
        },
    );
}

#[test]
fn settings_button_and_shortcut_reuse_the_lazy_layer_without_saving() {
    gtk_test(
        "ui::window::composition::tests::settings_button_and_shortcut_reuse_the_lazy_layer_without_saving",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let fixture = Fixture::new();
            let path = glib::user_config_dir().join("strata/settings.toml");
            let saved = std::fs::read(&path).expect("saved preferences");
            assert!(fixture.layer("settings-backdrop").is_none());
            let controllers = fixture.window.observe_controllers();
            let shortcut = (0..controllers.n_items())
                .filter_map(|index| {
                    controllers
                        .item(index)
                        .and_downcast::<gtk::EventControllerKey>()
                })
                .find(|keys| keys.propagation_phase() == gtk::PropagationPhase::Bubble)
                .expect("bubble-phase Settings shortcut");
            assert!(!shortcut.emit_by_name::<bool>(
                "key-pressed",
                &[&Key::comma, &0u32, &ModifierType::empty()]
            ));
            assert!(fixture.layer("settings-backdrop").is_none());
            assert!(shortcut.emit_by_name::<bool>(
                "key-pressed",
                &[&Key::comma, &0u32, &ModifierType::CONTROL_MASK]
            ));
            let layer = fixture.layer("settings-backdrop").expect("built lazily");
            assert!(layer.is_visible());
            layer.set_visible(false);
            fixture.content.header.settings.emit_clicked();
            assert!(layer.is_visible());
            assert_eq!(fixture.layer("settings-backdrop"), Some(layer));
            assert_eq!(std::fs::read(path).expect("unchanged preferences"), saved);
            fixture.close();
        },
    );
}

#[test]
fn update_notices_clear_in_both_windows_without_opening_settings() {
    gtk_test(
        "ui::window::composition::tests::update_notices_clear_in_both_windows_without_opening_settings",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let first = Fixture::new();
            let second = Fixture::new();
            let release = ReleaseMetadata {
                version: "9.0.0".into(),
                url: "https://example.test/release".into(),
                notes: String::new(),
                note_blocks: Vec::new(),
                kind: BuildKind::Stable,
                tag: "v9.0.0".into(),
                published_at: None,
                commit: None,
            };
            for enabled in [true, false] {
                for fixture in [&first, &second] {
                    (fixture.notice)(Some((
                        release.clone(),
                        "https://example.test/download".into(),
                        UpdateMethod::InPlace,
                    )));
                    assert!(fixture.content.sidebar.update_area.is_visible());
                    assert_eq!(
                        fixture
                            .content
                            .sidebar
                            .update_notice
                            .tooltip_text()
                            .as_deref(),
                        Some("Install Strata v9.0.0")
                    );
                    assert!(
                        !fixture
                            .content
                            .sidebar
                            .update_notice
                            .has_css_class("preview")
                    );
                }
                first.preferences.set_checks_for_updates(enabled);
                for fixture in [&first, &second] {
                    assert!(!fixture.content.sidebar.update_area.is_visible());
                    assert!(fixture.layer("settings-backdrop").is_none());
                }
            }
            first.close();
            second.close();
        },
    );
}

#[test]
fn sidebar_toggle_preserves_split_constraints() {
    gtk_test(
        "ui::window::composition::tests::sidebar_toggle_preserves_split_constraints",
        || {
            let fixture = Fixture::new();
            fixture.preferences.set_reduce_motion(true);
            let root = fixture
                .content
                .blurred_root
                .first_child()
                .expect("window root");
            let preview_split = root
                .first_child()
                .expect("header")
                .next_sibling()
                .expect("preview split")
                .downcast::<gtk::Paned>()
                .expect("preview paned");
            let content = preview_split
                .start_child()
                .expect("sidebar/browser split")
                .downcast::<gtk::Paned>()
                .expect("sidebar/browser paned");
            assert_eq!(content.position(), super::super::SIDEBAR_WIDTH);
            assert!(!content.resizes_start_child());
            fixture.content.header.sidebar_toggle.set_active(false);
            assert_eq!(content.position(), 0);
            assert!(!fixture.content.sidebar.widget.is_visible());
            fixture.content.header.sidebar_toggle.set_active(true);
            assert_eq!(content.position(), super::super::SIDEBAR_WIDTH);
            assert!(fixture.content.sidebar.widget.is_visible());
            content.set_position(1);
            assert_eq!(content.position(), super::super::MIN_SIDEBAR_WIDTH);
            fixture.close();
        },
    );
}

#[test]
fn destroy_handler_releases_browser_observers_and_is_idempotent() {
    gtk_test(
        "ui::window::composition::tests::destroy_handler_releases_browser_observers_and_is_idempotent",
        || {
            let fixture = Fixture::new();
            let retained = Rc::new(());
            let weak = Rc::downgrade(&retained);
            fixture.content.browser.browser().observe(move |_| {
                let _ = &retained;
            });
            assert!(weak.upgrade().is_some());
            fixture.content.connect_cleanup(&fixture.window);
            fixture.window.emit_by_name::<()>("destroy", &[]);
            assert!(weak.upgrade().is_none());
            fixture.window.emit_by_name::<()>("destroy", &[]);
            fixture.window.destroy();
        },
    );
}
