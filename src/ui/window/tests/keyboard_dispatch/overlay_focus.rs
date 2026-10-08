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
            keys.emit_by_name::<bool>(
                "key-pressed",
                &[&Key::Escape, &0u32, &ModifierType::empty()],
            )
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
                    failures.push(format!(
                        "{mode:?}: focus is on {}",
                        fixture.describe_focus()
                    ));
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

#[derive(Clone, Copy, Debug)]
enum DialogOverSettings {
    ActionEditor,
    /// An error chained on a delete confirmation whose delete failed.
    FailedDelete,
    /// A delete confirmation whose delete re-rendered the row that opened it.
    RerenderedDelete,
}

const FOCUS_ACTION_MANIFEST: &str = "schema_version = 1\nid = \"focus-demo\"\n\
    name = \"Focus demo\"\nmenu = \"top\"\n\n[when]\nextensions = [\"txt\"]\n\n\
    [run]\nruntime = \"command\"\nprogram = \"/bin/sh\"\nargs = [\"-c\", \"true\"]\n";

fn descendant(
    widget: &gtk::Widget,
    matches: &impl Fn(&gtk::Widget) -> bool,
) -> Option<gtk::Widget> {
    if matches(widget) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = descendant(&current, matches) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

fn top_dialog(fixture: &ComposedFolder) -> Option<gtk::Widget> {
    let mut child = fixture.content.overlay().last_child();
    while let Some(widget) = child {
        if widget.is_visible()
            && widget.has_css_class("app-modal-layer")
            && !widget.has_css_class("settings-backdrop")
            && !widget.has_css_class("dismissing")
        {
            return Some(widget);
        }
        child = widget.prev_sibling();
    }
    None
}

/// Closes `layer` the way a key press does: GTK hides focus rings on the release
/// of the key whose press disabled the focused control.
fn close_by_key(fixture: &ComposedFolder, layer: &gtk::Widget, close: impl FnOnce()) {
    fixture.window.set_focus_visible(true);
    close();
    fixture.window.set_focus_visible(false);
    assert!(settles(|| layer.parent().is_none()), "the dialog closes");
}

#[test]
fn a_dialog_closed_over_settings_returns_focus_to_its_opener() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::overlay_focus::a_dialog_closed_over_settings_returns_focus_to_its_opener",
        || {
            let fixture = ComposedFolder::open();
            let action_directory = crate::storage::config_directory()
                .join("actions")
                .join("focus-demo");
            let mut failures = Vec::new();
            for dialog in [
                DialogOverSettings::ActionEditor,
                DialogOverSettings::FailedDelete,
                DialogOverSettings::RerenderedDelete,
            ] {
                fixture.focus_cursor_row(BrowserMode::List);
                assert!(press_phase(
                    &fixture.window,
                    gtk::PropagationPhase::Bubble,
                    Key::comma,
                    ModifierType::CONTROL_MASK,
                ));
                let settings = fixture.layer("settings-backdrop").expect("Settings layer");
                assert!(settles(|| settings.is_visible() && settings.is_mapped()));
                descendant(&settings, &|widget| {
                    widget.is::<gtk::Button>() && widget.widget_name() == "Actions"
                })
                .and_downcast::<gtk::Button>()
                .expect("Settings page")
                .emit_clicked();
                let opener = match dialog {
                    DialogOverSettings::ActionEditor => {
                        assert!(settles(|| widget_with_class(
                            &settings,
                            "settings-actions-create-button"
                        )
                        .is_some_and(|button| button.is_mapped())));
                        let new_action =
                            widget_with_class(&settings, "settings-actions-create-button")
                                .and_downcast::<gtk::Button>()
                                .expect("New action");
                        assert!(new_action.grab_focus());
                        new_action.emit_clicked();
                        Some(new_action.upcast::<gtk::Widget>())
                    }
                    DialogOverSettings::FailedDelete | DialogOverSettings::RerenderedDelete => {
                        std::fs::create_dir_all(&action_directory).expect("action folder");
                        std::fs::write(action_directory.join("action.toml"), FOCUS_ACTION_MANIFEST)
                            .expect("action manifest");
                        crate::ui::actions::shared().reload();
                        let delete = || {
                            let row = descendant(&settings, &|widget| {
                                widget.has_css_class("settings-action-row") && widget.is_mapped()
                            })?;
                            descendant(&row, &|widget| {
                                widget.has_css_class("settings-action-icon-button")
                                    && widget.has_css_class("danger")
                            })
                        };
                        assert!(settles(|| delete().is_some()), "the action row");
                        let delete = delete()
                            .and_downcast::<gtk::Button>()
                            .expect("Delete action");
                        if matches!(dialog, DialogOverSettings::FailedDelete) {
                            std::fs::remove_dir_all(&action_directory)
                                .expect("remove the action behind the row");
                        }
                        assert!(delete.grab_focus());
                        delete.emit_clicked();
                        assert!(
                            settles(|| top_dialog(&fixture).is_some()),
                            "{dialog:?} asks"
                        );
                        let confirmation = top_dialog(&fixture).expect("confirmation");
                        let confirm = descendant(&confirmation, &|widget| {
                            widget
                                .downcast_ref::<gtk::Button>()
                                .is_some_and(|button| button.label().as_deref() == Some("Delete"))
                        })
                        .and_downcast::<gtk::Button>()
                        .expect("Delete confirmation");
                        close_by_key(&fixture, &confirmation, || confirm.emit_clicked());
                        matches!(dialog, DialogOverSettings::FailedDelete)
                            .then(|| delete.upcast::<gtk::Widget>())
                    }
                };
                if !matches!(dialog, DialogOverSettings::RerenderedDelete) {
                    assert!(
                        settles(|| top_dialog(&fixture).is_some()),
                        "{dialog:?} opens"
                    );
                    let layer = top_dialog(&fixture).expect("dialog layer");
                    pump(50);
                    close_by_key(&fixture, &layer, || press_escape_on(&layer));
                }
                let restored = || match &opener {
                    Some(opener) => opener.has_focus(),
                    None => gtk::prelude::RootExt::focus(&fixture.window)
                        .is_some_and(|focus| focus != settings && focus.is_ancestor(&settings)),
                };
                if !settles(restored) || !fixture.window.gets_focus_visible() {
                    failures.push(format!(
                        "{dialog:?}: focus is on {}, focus ring shown: {}",
                        fixture.describe_focus(),
                        fixture.window.gets_focus_visible()
                    ));
                }
                press_escape_on(&settings);
                assert!(settles(|| !settings.is_visible()));
            }
            assert!(
                failures.is_empty(),
                "a dialog closed over Settings must return focus to its opener:\n{}",
                failures.join("\n")
            );
        },
    );
}
