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
    SaveNotice,
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
            let preferences = PreferenceManager::shared();
            preferences.register_save_notice_window(fixture.window.upcast_ref());
            assert!(settles(|| fixture.window.is_active()));
            let settings_path = crate::ui::preferences::config_directory().join("settings.toml");
            let action_directory = crate::storage::config_directory()
                .join("actions")
                .join("focus-demo");
            let mut failures = Vec::new();
            for dialog in [
                DialogOverSettings::ActionEditor,
                DialogOverSettings::SaveNotice,
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
                let page = match dialog {
                    DialogOverSettings::SaveNotice => "general",
                    _ => "actions",
                };
                descendant(&settings, &|widget| {
                    widget.is::<gtk::Button>() && widget.widget_name() == page
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
                    DialogOverSettings::SaveNotice => {
                        let switch = || {
                            descendant(&settings, &|widget| {
                                widget.is::<gtk::Switch>() && widget.is_mapped()
                            })
                        };
                        assert!(settles(|| switch().is_some()), "a General switch");
                        let switch = switch().expect("General switch");
                        assert!(switch.grab_focus());
                        std::fs::remove_file(&settings_path).expect("saved settings");
                        std::fs::create_dir(&settings_path).expect("block the settings file");
                        preferences.set_folder_peeking(!preferences.folder_peeking());
                        Some(switch)
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
                if settings_path.is_dir() {
                    std::fs::remove_dir(&settings_path).expect("repair the settings file");
                }
            }
            assert!(
                failures.is_empty(),
                "a dialog closed over Settings must return focus to its opener:\n{}",
                failures.join("\n")
            );
        },
    );
}

#[derive(Clone, Copy, Debug)]
enum WindowAccelerator {
    Search,
    FolderJump,
    Refresh,
    Terminal,
    ArrowScope,
}

impl WindowAccelerator {
    const ALL: [Self; 5] = [
        Self::Search,
        Self::FolderJump,
        Self::Refresh,
        Self::Terminal,
        Self::ArrowScope,
    ];

    fn action(self) -> &'static str {
        match self {
            Self::Search => "win.search",
            Self::FolderJump => "win.jump-folder",
            Self::Refresh => "win.refresh",
            Self::Terminal => "win.open-terminal",
            Self::ArrowScope => "win.toggle-arrow-scope",
        }
    }

    fn ran(self, before: &AcceleratorEffects, after: &AcceleratorEffects) -> bool {
        match self {
            Self::Search | Self::FolderJump => after.palette_open != before.palette_open,
            Self::Refresh => after.reloads != before.reloads,
            Self::Terminal => after.children != before.children,
            Self::ArrowScope => after.arrows_scoped != before.arrows_scoped,
        }
    }

    /// Running the action again undoes it: the palette closes, the preference flips back.
    fn toggles(self) -> bool {
        matches!(self, Self::Search | Self::FolderJump | Self::ArrowScope)
    }
}

#[derive(Debug, PartialEq)]
struct AcceleratorEffects {
    palette_open: bool,
    reloads: usize,
    arrows_scoped: bool,
    children: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
enum InputOwner {
    Dialog,
    Rename(BrowserMode),
}

#[test]
fn window_accelerator_actions_yield_to_modals_and_inline_edits() {
    const NAME: &str = "ui::window::tests::keyboard_dispatch::overlay_focus::window_accelerator_actions_yield_to_modals_and_inline_edits";
    // `sleep` stands in for the terminal so win.open-terminal shows up as a child process.
    crate::test_support::gtk_test_with_env(NAME, [("TERMINAL", "sleep 5")], || {
        let fixture = ComposedFolder::open();
        let browser = &fixture.content.browser;
        let preferences = PreferenceManager::shared();
        let reloads = Rc::new(Cell::new(0usize));
        let counted = reloads.clone();
        browser.browser().observe(move |event| {
            if matches!(
                event,
                BrowserEvent::ColumnReloaded { .. } | BrowserEvent::ColumnRefreshing { .. }
            ) {
                counted.set(counted.get() + 1);
            }
        });
        let effects = || AcceleratorEffects {
            palette_open: fixture
                .layer("search-backdrop")
                .is_some_and(|layer| layer.is_visible()),
            reloads: reloads.get(),
            arrows_scoped: preferences.arrow_navigation_scoped(),
            children: child_commands(),
        };
        let activate = |accelerator: WindowAccelerator| {
            let action = accelerator.action();
            gtk::prelude::WidgetExt::activate_action(&fixture.window, action, None)
                .unwrap_or_else(|error| panic!("{action}: {error}"));
        };
        let focus_is_in = |widget: &gtk::Widget| {
            gtk::prelude::RootExt::focus(&fixture.window)
                .is_some_and(|focus| &focus == widget || focus.is_ancestor(widget))
        };

        for owner in [
            InputOwner::Dialog,
            InputOwner::Rename(BrowserMode::Columns),
            InputOwner::Rename(BrowserMode::List),
            InputOwner::Rename(BrowserMode::Icons),
        ] {
            let mode = match owner {
                InputOwner::Rename(mode) => mode,
                InputOwner::Dialog => BrowserMode::Columns,
            };
            fixture.focus_cursor_row(mode);
            let owned: gtk::Widget = match owner {
                InputOwner::Dialog => {
                    let dialog = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    dialog.set_focusable(true);
                    dialog.add_css_class("app-modal-layer");
                    fixture.content.overlay().add_overlay(&dialog);
                    dialog.grab_focus();
                    dialog.upcast()
                }
                InputOwner::Rename(_) => {
                    wait_until(|| browser.rename_is_active() || browser.begin_rename());
                    let field = browser.active_rename_field().expect("rename field");
                    field.set_text("kept.txt");
                    field.upcast()
                }
            };
            wait_until(|| focus_is_in(&owned));
            let before = effects();
            for accelerator in WindowAccelerator::ALL {
                activate(accelerator);
                pump(150);
                assert_eq!(effects(), before, "{owner:?}: {accelerator:?} ran");
                assert!(
                    focus_is_in(&owned),
                    "{owner:?}: {accelerator:?} took focus to {}",
                    fixture.describe_focus()
                );
                if let Some(field) = owned.downcast_ref::<gtk::Entry>() {
                    assert!(browser.rename_is_active(), "{owner:?}: {accelerator:?}");
                    assert_eq!(field.text(), "kept.txt", "{owner:?}: {accelerator:?}");
                    assert!(fixture._directory.path().join("b.txt").exists());
                }
            }
            match owner {
                InputOwner::Dialog => fixture.content.overlay().remove_overlay(&owned),
                InputOwner::Rename(_) => assert!(browser.cancel_rename()),
            }
        }

        // A new folder owns the listing from the request until its name field opens.
        fixture.focus_cursor_row(BrowserMode::Columns);
        let before = effects();
        browser.create_new_folder();
        for accelerator in WindowAccelerator::ALL {
            assert!(browser.new_entry_is_active(), "{accelerator:?}");
            activate(accelerator);
            assert!(
                !accelerator.ran(&before, &effects()),
                "{accelerator:?} ran during folder creation"
            );
        }
        wait_until(|| browser.rename_is_active());
        pump(150);
        assert_eq!(
            effects(),
            before,
            "an accelerator ran during folder creation"
        );
        assert!(browser.cancel_rename());

        fixture.focus_cursor_row(BrowserMode::Columns);
        // A dialog still animating out after Escape no longer owns input.
        let closing = gtk::Box::new(gtk::Orientation::Vertical, 0);
        closing.add_css_class("app-modal-layer");
        closing.add_css_class("dismissing");
        fixture.content.overlay().add_overlay(&closing);
        for accelerator in WindowAccelerator::ALL {
            let before = effects();
            activate(accelerator);
            wait_until(|| accelerator.ran(&before, &effects()));
            if accelerator.toggles() {
                activate(accelerator);
                wait_until(|| !accelerator.ran(&before, &effects()));
            }
        }
        fixture.content.overlay().remove_overlay(&closing);
    });
}

#[test]
fn a_rename_to_a_hidden_name_lets_the_next_rename_start() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::overlay_focus::a_rename_to_a_hidden_name_lets_the_next_rename_start",
        || {
            // Directory monitors drop hidden names only when the folder opens with
            // hidden files off, as it does by default.
            PreferenceManager::seed_saved_preferences_for_test();
            let preferences = PreferenceManager::shared();
            let mut sort = preferences.sort_preferences();
            sort.show_hidden = false;
            preferences.set_sort_preferences(sort);
            for mode in [BrowserMode::Columns, BrowserMode::List] {
                let fixture = ComposedFolder::open();
                let browser = &fixture.content.browser;
                assert!(!browser.browser().preferences().show_hidden);
                fixture.focus_cursor_row(mode);
                wait_until(|| browser.rename_is_active() || browser.begin_rename());
                let field = browser.active_rename_field().expect("rename field");
                field.set_text(".hidden-b.txt");
                field.emit_activate();
                let directory = fixture._directory.path();
                wait_until(|| {
                    directory.join(".hidden-b.txt").exists()
                        && !rendered_name(&browser.widget(), "b.txt")
                });

                // The listing never shows the new name; the rename settles anyway.
                fixture.focus_cursor_row(mode);
                wait_until(|| browser.begin_rename());
                let field = browser.active_rename_field().expect("rename field");
                assert_eq!(field.text(), "c.txt", "{mode:?}");
                assert!(browser.cancel_rename());
            }
        },
    );
}
