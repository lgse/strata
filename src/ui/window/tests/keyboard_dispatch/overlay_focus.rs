// SPDX-License-Identifier: MIT

use super::*;

const MODES: [BrowserMode; 3] = [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons];

struct ComposedFolder {
    window: gtk::ApplicationWindow,
    content: super::super::super::composition::WindowContent,
    _directory: tempfile::TempDir,
}

impl ComposedFolder {
    fn open() -> Self {
        PreferenceManager::seed_saved_preferences_for_test();
        PreferenceManager::shared().set_tenxer_mode(false);
        let (window, content) = composed_window();
        let directory = tempfile::tempdir().expect("overlay folder");
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(directory.path().join(name), b"overlay").expect("fixture file");
        }
        content
            .browser
            .navigate_location(Location::local(directory.path()));
        wait_until(|| {
            content
                .browser
                .browser()
                .column_snapshot(0)
                .is_some_and(|column| !column.loading)
        });
        Self {
            window,
            content,
            _directory: directory,
        }
    }

    fn focus_cursor_row(&self, mode: BrowserMode) {
        self.content.browser.set_view_mode(mode);
        let browser = self.content.browser.browser();
        browser.select(0, 1);
        browser.focus_active();
        wait_until(|| self.content.browser.item_view_has_focus());
        // Deferred load and rebuild focus work must not land after the overlay opens.
        pump(300);
        assert!(self.cursor_row_has_focus(), "{mode:?} cursor row focused");
    }

    fn cursor_row_has_focus(&self) -> bool {
        self.content.browser.item_view_has_focus()
            && focused_index(&self.content.browser.browser()) == 1
    }

    fn describe_focus(&self) -> String {
        match gtk::prelude::RootExt::focus(&self.window) {
            None => "nothing".to_owned(),
            Some(focused) => format!(
                "{} {:?} (mapped: {})",
                focused.type_().name(),
                focused.css_classes(),
                focused.is_mapped()
            ),
        }
    }

    fn layer(&self, class: &str) -> Option<gtk::Widget> {
        let mut child = self.content.overlay().first_child();
        while let Some(widget) = child {
            if widget.has_css_class(class) {
                return Some(widget);
            }
            child = widget.next_sibling();
        }
        None
    }
}

impl Drop for ComposedFolder {
    fn drop(&mut self) {
        self.window.destroy();
    }
}

fn settles(condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        if Instant::now() >= deadline {
            return false;
        }
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
    true
}

fn press_escape_on(layer: &gtk::Widget) {
    let controllers = layer.observe_controllers();
    let handled = (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .any(|keys| {
            keys.emit_by_name::<bool>("key-pressed", &[&Key::Escape, &0u32, &ModifierType::empty()])
        });
    assert!(handled, "the overlay handles Escape");
}

#[derive(Clone, Copy, Debug)]
enum SettingsOpener {
    Shortcut,
    Gear,
}

#[derive(Clone, Copy, Debug)]
enum SettingsClose {
    Escape,
    CloseButton,
}

#[test]
fn closing_settings_returns_focus_to_the_file_list_cursor() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::overlay_focus::closing_settings_returns_focus_to_the_file_list_cursor",
        || {
            let fixture = ComposedFolder::open();
            let mut failures = Vec::new();
            for mode in MODES {
                for (opener, close) in [
                    (SettingsOpener::Shortcut, SettingsClose::Escape),
                    (SettingsOpener::Gear, SettingsClose::CloseButton),
                ] {
                    fixture.focus_cursor_row(mode);
                    match opener {
                        SettingsOpener::Shortcut => {
                            assert!(press_phase(
                                &fixture.window,
                                gtk::PropagationPhase::Bubble,
                                Key::comma,
                                ModifierType::CONTROL_MASK,
                            ));
                        }
                        SettingsOpener::Gear => {
                            // A pointer click focuses the gear before it opens Settings.
                            fixture.content.settings_button().grab_focus();
                            fixture.content.settings_button().emit_clicked();
                        }
                    }
                    let layer = fixture.layer("settings-backdrop").expect("Settings layer");
                    assert!(settles(|| layer.is_visible() && layer.is_mapped()));
                    pump(50);
                    match close {
                        SettingsClose::Escape => press_escape_on(&layer),
                        SettingsClose::CloseButton => {
                            widget_with_class(&layer, "settings-close")
                                .and_downcast::<gtk::Button>()
                                .expect("Close settings")
                                .emit_clicked();
                        }
                    }
                    assert!(
                        settles(|| !layer.is_visible()),
                        "{mode:?} {opener:?} {close:?}: Settings did not close"
                    );
                    if !settles(|| fixture.cursor_row_has_focus()) {
                        failures.push(format!(
                            "{mode:?} opened by {opener:?}, closed by {close:?}: focus is on {}",
                            fixture.describe_focus()
                        ));
                    }
                }
            }
            assert!(
                failures.is_empty(),
                "closing Settings must return focus to the file list cursor row:\n{}",
                failures.join("\n")
            );
        },
    );
}

#[derive(Clone, Copy, Debug)]
enum Palette {
    GlobalSearch,
    FolderJump,
}

#[derive(Clone, Copy, Debug)]
enum PaletteClose {
    Escape,
    Toggle,
}

#[test]
fn dismissing_search_palettes_returns_focus_to_the_file_list_cursor() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::overlay_focus::dismissing_search_palettes_returns_focus_to_the_file_list_cursor",
        || {
            let fixture = ComposedFolder::open();
            let mut failures = Vec::new();
            for mode in MODES {
                for (palette, close) in [
                    (Palette::GlobalSearch, PaletteClose::Escape),
                    (Palette::GlobalSearch, PaletteClose::Toggle),
                    (Palette::FolderJump, PaletteClose::Escape),
                ] {
                    fixture.focus_cursor_row(mode);
                    let action = match palette {
                        Palette::GlobalSearch => "search",
                        Palette::FolderJump => "jump-folder",
                    };
                    gtk::gio::prelude::ActionGroupExt::activate_action(
                        &fixture.window,
                        action,
                        None,
                    );
                    let layer = fixture.layer("search-backdrop").expect("search layer");
                    assert!(settles(|| layer.is_visible() && layer.is_mapped()));
                    pump(50);
                    match close {
                        PaletteClose::Escape => press_escape_on(&layer),
                        PaletteClose::Toggle => {
                            gtk::gio::prelude::ActionGroupExt::activate_action(
                                &fixture.window,
                                "search",
                                None,
                            );
                        }
                    }
                    assert!(
                        settles(|| !layer.is_visible()),
                        "{mode:?} {palette:?} {close:?}: the palette did not close"
                    );
                    if !settles(|| fixture.cursor_row_has_focus()) {
                        failures.push(format!(
                            "{mode:?} {palette:?} closed by {close:?}: focus is on {}",
                            fixture.describe_focus()
                        ));
                    }
                }
            }
            assert!(
                failures.is_empty(),
                "dismissing a search palette must return focus to the file list cursor row:\n{}",
                failures.join("\n")
            );
        },
    );
}

#[test]
fn closing_settings_returns_focus_to_a_focused_filter_field() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::overlay_focus::closing_settings_returns_focus_to_a_focused_filter_field",
        || {
            let fixture = ComposedFolder::open();
            let browser = &fixture.content.browser;
            let mut failures = Vec::new();
            for mode in MODES {
                fixture.focus_cursor_row(mode);
                assert!(browser.show_filter(), "{mode:?} shows the filter");
                assert!(settles(|| browser.filter_has_focus()));
                assert!(press_phase(
                    &fixture.window,
                    gtk::PropagationPhase::Bubble,
                    Key::comma,
                    ModifierType::CONTROL_MASK,
                ));
                let layer = fixture.layer("settings-backdrop").expect("Settings layer");
                assert!(settles(|| layer.is_visible() && layer.is_mapped()));
                press_escape_on(&layer);
                assert!(settles(|| !layer.is_visible()));
                if !settles(|| browser.filter_has_focus()) {
                    failures.push(format!("{mode:?}: focus is on {}", fixture.describe_focus()));
                }
                browser.dismiss_focused_filter();
            }
            assert!(
                failures.is_empty(),
                "closing Settings must give a focused filter field its focus back:\n{}",
                failures.join("\n")
            );
        },
    );
}
