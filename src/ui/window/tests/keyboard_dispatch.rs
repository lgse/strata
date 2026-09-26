// SPDX-License-Identifier: MIT

use std::cell::RefCell;

use gtk::gdk::{Key, ModifierType};

use crate::app::BrowserEvent;
use crate::model::{SortDirection, SortKey};

use super::super::*;
use crate::services::{
    LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
};
use crate::ui::{
    preview::PreviewDrawer, shortcut_footer::ShortcutFooter, top_bar_navigation::TopBarNavigation,
};

struct TextPreview;

impl PreviewProvider for TextPreview {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        glib::idle_add_local_once(move || {
            emit(PreviewEvent::Ready(Preview {
                request_id: request.id,
                entry: request.entry,
                content_type: "text/plain".into(),
                content: PreviewContent::Text {
                    content: "Space opens quick preview.\n".into(),
                    truncated: false,
                },
            }))
        });
        LoadHandle::new(|| {})
    }
}

struct KeyboardFixture {
    window: gtk::ApplicationWindow,
    overlay: gtk::Overlay,
    view: BrowserView,
    sidebar: SidebarView,
    preview: PreviewDrawer,
    sidebar_toggle: gtk::ToggleButton,
    shortcuts: ShortcutFooter,
    keys: gtk::EventControllerKey,
    _directory: tempfile::TempDir,
}

impl KeyboardFixture {
    fn new() -> Self {
        Self::with_provider(Rc::new(TextPreview))
    }

    fn with_provider(provider: Rc<dyn crate::services::PreviewProvider>) -> Self {
        PreferenceManager::seed_saved_preferences_for_test();
        let preferences = PreferenceManager::shared();
        // Keyboard focus-return scenarios need a place to focus; the saved fixture hides all places.
        preferences.set_sidebar_show_home(true);
        // The exhaustive fixture enables 10xer, which replaces this default map.
        preferences.set_tenxer_mode(false);
        let directory = tempfile::tempdir().expect("fixture");
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(directory.path().join(name), b"preview").expect("fixture file");
        }
        let view = browser_for_window();
        view.set_view_mode(BrowserMode::Columns);
        let sidebar = build_sidebar(view.clone(), preferences.clone(), true);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let toggle = gtk::ToggleButton::builder().active(true).build();
        header.append(&toggle);
        header.append(&view.location_widget());
        let top_bar = TopBarNavigation::new(&header, &sidebar.widget, &toggle);
        let preview = PreviewDrawer::new(provider, false);
        let shortcuts = ShortcutFooter::new(BrowserMode::Columns);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.append(&sidebar.widget);
        row.append(&view.widget());
        row.append(&preview.widget());
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header);
        content.append(&row);
        content.append(shortcuts.widget());
        let overlay = gtk::Overlay::builder().child(&content).build();
        let window = gtk::ApplicationWindow::builder()
            .child(&overlay)
            .default_width(1000)
            .default_height(600)
            .build();
        keyboard::install(
            &window,
            &sidebar,
            keyboard::Bindings {
                view: view.clone(),
                top_bar,
                preview: preview.clone(),
                type_to_search: TypeToSearch {
                    view: view.clone(),
                    preferences,
                },
                shortcuts: shortcuts.clone(),
            },
        );
        let controllers = window.observe_controllers();
        let keys = (0..controllers.n_items())
            .filter_map(|index| {
                controllers
                    .item(index)
                    .and_downcast::<gtk::EventControllerKey>()
            })
            .next()
            .expect("key controller");
        window.present();
        view.browser().navigate(Location::local(directory.path()));
        wait_until(|| {
            view.browser()
                .column_snapshot(0)
                .is_some_and(|column| !column.loading)
        });
        view.browser().select(0, 0);
        view.browser().focus_active();
        wait_until(|| view.item_view_has_focus() && rendered_name(&view.widget(), "a.txt"));
        Self {
            window,
            overlay,
            view,
            sidebar,
            preview,
            sidebar_toggle: toggle,
            shortcuts: shortcuts.clone(),
            keys,
            _directory: directory,
        }
    }

    fn press(&self, key: Key, modifiers: ModifierType) -> bool {
        self.keys
            .emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers])
    }

    fn selected(&self) -> Vec<usize> {
        self.view.browser().selected_positions(0)
    }
}

impl Drop for KeyboardFixture {
    fn drop(&mut self) {
        self.view.browser().clear_observer();
        self.sidebar.disconnect();
        self.window.destroy();
    }
}

fn rendered_name(widget: &gtk::Widget, name: &str) -> bool {
    if !widget.is_mapped()
        || widget.width() <= 0
        || widget
            .downcast_ref::<gtk::Stack>()
            .is_some_and(|stack| stack.is_transition_running())
    {
        return false;
    }
    if widget
        .downcast_ref::<gtk::Label>()
        .is_some_and(|label| label.label() == name)
        || widget
            .downcast_ref::<gtk::Inscription>()
            .is_some_and(|label| label.text().as_deref() == Some(name))
    {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if rendered_name(&widget, name) {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

fn widget_with_class(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if widget.has_css_class(class) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = widget_with_class(&widget, class) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "keyboard fixture did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn tenxer_keeps_keyboard_navigation_inside_the_file_panes() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tenxer_keeps_keyboard_navigation_inside_the_file_panes",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_arrow_navigation_scoped(false);
            preferences.set_tenxer_mode(true);
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                fixture.view.set_view_mode(mode);
                fixture.window.present();
                fixture.view.browser().select(0, 0);
                focus_files(&fixture);

                for (key, modifiers) in [
                    (Key::Up, ModifierType::empty()),
                    (Key::Tab, ModifierType::empty()),
                    (Key::ISO_Left_Tab, ModifierType::empty()),
                    (Key::Tab, ModifierType::SHIFT_MASK),
                    (
                        Key::b,
                        ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK,
                    ),
                ] {
                    focus_files(&fixture);
                    fixture.press(key, modifiers);
                    assert!(
                        fixture.view.item_view_has_focus(),
                        "{mode:?} {key:?} left the file panes"
                    );
                    assert!(
                        !sidebar_has_focus(&fixture),
                        "{mode:?} {key:?} focused the sidebar"
                    );
                }

                let origin = fixture.view.browser().active_location();
                focus_files(&fixture);
                fixture.press(Key::Left, ModifierType::empty());
                assert!(
                    file_panes_have_focus(&fixture),
                    "{mode:?} Left left the file panes"
                );
                assert!(!sidebar_has_focus(&fixture), "{mode:?}");
                if fixture.view.browser().active_location() != origin
                    && let Some(origin) = origin
                {
                    fixture.view.browser().navigate(origin);
                    wait_until(|| {
                        fixture
                            .view
                            .browser()
                            .column_snapshot(0)
                            .is_some_and(|column| !column.loading)
                    });
                }
                fixture.view.browser().select(0, 0);
                focus_files(&fixture);
                if mode == BrowserMode::Columns {
                    assert!(fixture.press(Key::Down, ModifierType::empty()));
                    assert_eq!(fixture.selected(), [1], "{mode:?}");
                    assert!(fixture.view.item_view_has_focus());
                }

                assert!(
                    fixture.sidebar.state.focus_active_place(),
                    "{mode:?} sidebar place"
                );
                wait_until(|| sidebar_has_focus(&fixture));
                fixture.press(Key::j, ModifierType::empty());
                assert!(
                    fixture.view.item_view_has_focus(),
                    "{mode:?} j must return from the sidebar to the files"
                );
                assert!(
                    fixture.sidebar.state.focus_active_place(),
                    "{mode:?} sidebar place"
                );
                wait_until(|| sidebar_has_focus(&fixture));
                fixture.press(Key::Down, ModifierType::empty());
                assert!(
                    fixture.view.item_view_has_focus(),
                    "{mode:?} Down from the sidebar must return to the files"
                );

                assert!(fixture.sidebar_toggle.grab_focus(), "{mode:?}");
                fixture.press(Key::Right, ModifierType::empty());
                assert!(fixture.view.item_view_has_focus(), "{mode:?}");
                assert!(!fixture.sidebar_toggle.has_focus(), "{mode:?}");

                let shortcuts =
                    widget_with_class(fixture.window.upcast_ref(), "shortcut-footer-button")
                        .expect("shortcuts button");
                wait_until(|| shortcuts.is_mapped());
                assert!(shortcuts.grab_focus(), "{mode:?}");
                fixture.press(Key::Tab, ModifierType::empty());
                assert!(
                    fixture.view.item_view_has_focus(),
                    "{mode:?} Tab from the footer must return to the files"
                );
            }

            focus_files(&fixture);
            assert!(fixture.press(Key::l, ModifierType::CONTROL_MASK));
            assert!(fixture.view.location_has_focus());
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            assert!(!fixture.view.location_has_focus());

            preferences.set_tenxer_mode(false);
            fixture.view.set_view_mode(BrowserMode::List);
            fixture.view.browser().select(0, 0);
            focus_files(&fixture);
            fixture.press(Key::Up, ModifierType::empty());
            wait_until(|| fixture.view.header_actions_have_focus());
        },
    );
}

#[test]
fn tenxer_file_list_skips_conflicting_defaults_and_keeps_bound_shortcuts() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tenxer_file_list_skips_conflicting_defaults_and_keeps_bound_shortcuts",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_type_to_search(true);
            preferences.set_arrow_navigation_scoped(true);
            preferences.set_tenxer_mode(true);
            let jumps = Rc::new(Cell::new(0));
            let observed = jumps.clone();
            let action = gio::SimpleAction::new("jump-folder", None);
            action.connect_activate(move |_, _| observed.set(observed.get() + 1));
            fixture.window.add_action(&action);
            let searches = Rc::new(Cell::new(0));
            let observed = searches.clone();
            let action = gio::SimpleAction::new("search", None);
            action.connect_activate(move |_, _| observed.set(observed.get() + 1));
            fixture.window.add_action(&action);
            let pins = Rc::new(Cell::new(0));
            let observed = pins.clone();
            fixture.view.set_pin_handlers(
                Rc::new(move |_location, _name| observed.set(observed.get() + 1)),
                Rc::new(|_location| {}),
                Rc::new(|_| PinStatus::Available),
            );
            std::fs::create_dir(fixture._directory.path().join("folder")).expect("folder");
            fixture.view.refresh();
            wait_until(|| {
                fixture
                    .view
                    .browser()
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
                    && rendered_name(&fixture.view.widget(), "folder")
            });
            focus_files(&fixture);
            let names = directory_names(fixture._directory.path());
            for key in [Key::s, Key::S, Key::y, Key::space] {
                fixture.view.browser().select(0, 0);
                focus_files(&fixture);
                fixture.press(key, ModifierType::empty());
                assert_eq!(
                    fixture.selected(),
                    [0],
                    "{key:?} must not move the selection"
                );
                assert!(!fixture.view.filter_has_focus(), "{key:?} must not search");
                assert!(!fixture.preview.is_open(), "{key:?} must not preview");
                assert!(!fixture.view.rename_is_active(), "{key:?}");
                assert!(preferences.tenxer_mode(), "{key:?} must not leave the mode");
            }
            for key in [Key::j, Key::k] {
                fixture.view.browser().select(0, 0);
                focus_files(&fixture);
                fixture.press(key, ModifierType::empty());
                assert!(!fixture.view.filter_has_focus(), "{key:?} must not search");
                assert!(!fixture.preview.is_open(), "{key:?} must not preview");
                assert!(!fixture.view.rename_is_active(), "{key:?}");
                assert!(preferences.tenxer_mode(), "{key:?} must not leave the mode");
            }
            select_named(&fixture, "folder");
            fixture.press(Key::p, ModifierType::empty());
            assert_eq!(pins.get(), 0, "p must not pin while 10xer is on");
            fixture.view.browser().select(0, 0);
            focus_files(&fixture);
            let control = ModifierType::CONTROL_MASK;
            let names_before_paging = directory_names(fixture._directory.path());
            for key in [Key::d, Key::f, Key::b] {
                fixture.press(key, control);
                assert!(
                    !fixture.view.filter_has_focus(),
                    "{key:?} must not open the filter"
                );
                assert!(
                    fixture.sidebar_toggle.is_active(),
                    "{key:?} must not toggle the sidebar"
                );
            }
            assert_eq!(
                directory_names(fixture._directory.path()),
                names_before_paging,
                "Ctrl+D must not duplicate while 10xer is on"
            );
            fixture.view.browser().select(0, 0);
            focus_files(&fixture);
            for key in [Key::r, Key::backslash] {
                fixture.press(key, control);
                assert_eq!(fixture.selected(), [0], "{key:?}");
                assert!(!fixture.view.rename_is_active(), "{key:?}");
            }
            let children = child_commands();
            fixture.press(Key::t, control);
            pump(200);
            assert_eq!(
                child_commands(),
                children,
                "Ctrl+T must not launch a terminal"
            );
            fixture.press(Key::k, control | ModifierType::SHIFT_MASK);
            pump(400);
            assert_eq!(directory_names(fixture._directory.path()), names);
            assert!(!fixture.view.filter_has_focus());
            assert!(fixture.sidebar_toggle.is_active());
            assert!(!fixture.view.rename_is_active());
            assert_eq!(jumps.get(), 0);
            assert!(
                !modal_visible(&fixture.overlay),
                "Ctrl+T must not open a terminal"
            );
            assert!(preferences.arrow_navigation_scoped());
            assert_eq!(fixture.selected(), [0]);
            assert!(preferences.tenxer_mode());

            let text_size = preferences.text_size().root_font_px();
            assert!(fixture.press(Key::plus, control));
            assert_eq!(preferences.text_size().root_font_px(), text_size + 1);
            assert!(fixture.press(Key::_0, control));
            assert_eq!(preferences.text_size().root_font_px(), 13);
            select_named(&fixture, "a.txt");
            assert!(fixture.press(Key::c, control));
            select_named(&fixture, "folder");
            assert!(fixture.press(Key::v, control));
            wait_until(|| fixture._directory.path().join("folder/a.txt").exists());
            select_named(&fixture, "a.txt");
            assert!(fixture.press(Key::x, control));
            assert!(fixture._directory.path().join("a.txt").exists());
            select_named(&fixture, "b.txt");
            assert!(fixture.press(Key::Delete, ModifierType::SHIFT_MASK));
            wait_until(|| modal_visible(&fixture.overlay));
            assert!(click_class(&fixture.overlay, "action-dialog-close"));
            wait_until(|| !modal_visible(&fixture.overlay));
            assert!(fixture.press(Key::F2, ModifierType::empty()));
            assert!(fixture.view.rename_is_active());
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            assert!(!fixture.view.rename_is_active());
            std::fs::write(fixture._directory.path().join("d.txt"), b"d").expect("refresh file");
            assert!(fixture.press(Key::F5, ModifierType::empty()));
            wait_until(|| rendered_name(&fixture.view.widget(), "d.txt"));
            assert!(fixture.press(Key::l, control));
            wait_until(|| fixture.view.location_has_focus());
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            focus_files(&fixture);
            assert!(fixture.press(Key::k, control));
            assert_eq!(searches.get(), 1);
            assert!(fixture.press(Key::_2, control));
            assert_eq!(fixture.view.view_mode(), BrowserMode::Icons);
            assert!(fixture.press(Key::_3, control));
            assert_eq!(fixture.view.view_mode(), BrowserMode::List);
            assert!(fixture.press(Key::_1, control));
            assert_eq!(fixture.view.view_mode(), BrowserMode::Columns);
            focus_files(&fixture);
            assert!(fixture.press(Key::n, control | ModifierType::SHIFT_MASK));
            assert!(fixture.view.new_entry_is_active());
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            assert!(!fixture.view.new_entry_is_active());
            focus_files(&fixture);
            assert!(fixture.press(Key::Return, ModifierType::ALT_MASK));
            wait_until(|| {
                widget_with_class(fixture.overlay.upcast_ref(), "properties-content").is_some()
            });
            assert!(click_class(&fixture.overlay, "action-dialog-close"));
            wait_until(|| !modal_visible(&fixture.overlay));
            focus_files(&fixture);
            assert!(fixture.press(Key::Menu, ModifierType::empty()));
            wait_until(|| visible_menu(fixture.window.upcast_ref()).is_some());
            visible_menu(fixture.window.upcast_ref())
                .expect("menu")
                .popdown();
            wait_until(|| visible_menu(fixture.window.upcast_ref()).is_none());
            focus_files(&fixture);
            assert!(fixture.press(Key::F10, ModifierType::SHIFT_MASK));
            wait_until(|| visible_menu(fixture.window.upcast_ref()).is_some());
            visible_menu(fixture.window.upcast_ref())
                .expect("menu")
                .popdown();
            focus_files(&fixture);
            let names_before_quit = directory_names(fixture._directory.path());
            for modifier in [
                ModifierType::CONTROL_MASK,
                ModifierType::ALT_MASK,
                ModifierType::SUPER_MASK,
            ] {
                fixture.press(Key::q, modifier);
                assert!(
                    preferences.tenxer_mode(),
                    "{modifier:?}+q must not leave the mode"
                );
                assert!(fixture.window.is_visible());
                fixture.press(Key::Q, modifier | ModifierType::SHIFT_MASK);
                assert!(
                    fixture.window.is_visible(),
                    "{modifier:?}+Shift+Q must not close the window"
                );
                assert!(preferences.tenxer_mode());
            }
            assert_eq!(
                directory_names(fixture._directory.path()),
                names_before_quit
            );
            assert!(fixture.press(Key::q, ModifierType::empty()));
            assert!(!preferences.tenxer_mode());
            assert!(fixture.window.is_visible());
            focus_files(&fixture);
            assert!(fixture.press(Key::s, ModifierType::empty()));
            assert!(fixture.view.filter_has_focus(), "type-to-search returns");
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            focus_files(&fixture);
            assert!(fixture.press(Key::f, control));
            assert!(fixture.view.filter_has_focus(), "Ctrl+F filters again");
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            focus_files(&fixture);
            assert!(fixture.sidebar_toggle.is_active());
            assert!(fixture.press(Key::b, control));
            assert!(!fixture.sidebar_toggle.is_active());
            assert!(fixture.press(Key::r, control));
            assert!(fixture.view.rename_is_active(), "Ctrl+R renames again");
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            focus_files(&fixture);
            let jumps_before = jumps.get();
            assert!(fixture.press(Key::k, control | ModifierType::SHIFT_MASK));
            assert_eq!(jumps.get(), jumps_before + 1);
            preferences.set_type_to_search(false);
            fixture.view.browser().select(0, 0);
            focus_files(&fixture);
            assert!(fixture.press(Key::j, ModifierType::empty()));
            assert_ne!(fixture.selected(), [0], "home row moves again");
            assert!(!fixture.press(Key::backslash, control));
            assert!(preferences.arrow_navigation_scoped());
            preferences.set_tenxer_mode(true);
            let other = gtk::Window::new();
            other.present();
            assert!(fixture.press(Key::Q, ModifierType::SHIFT_MASK));
            assert!(!fixture.window.is_visible());
            assert!(other.is_visible());
            assert!(preferences.tenxer_mode());
        },
    );
}

#[test]
fn appearance_menu_hides_space_preview_while_tenxer_is_on() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::appearance_menu_hides_space_preview_while_tenxer_is_on",
        || {
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(false);
            let view = browser_for_window();
            let preview = PreviewDrawer::new(Rc::new(TextPreview), false);
            let menu = build_appearance_menu(&view, &view.browser(), preferences.clone(), &preview);
            let window = gtk::Window::builder().child(&menu).build();
            window.present();
            menu.popup();
            wait_until(|| menu.popover().is_some_and(|popover| popover.is_visible()));
            let popover = menu.popover().expect("appearance popover");
            let toggle = widget_with_class(popover.upcast_ref(), "preview-panel-option")
                .expect("preview panel option");
            assert_eq!(preview_shortcut(&toggle), "Space");
            assert_eq!(
                toggle.tooltip_text().as_deref(),
                Some("Toggle preview panel while browsing (Space)")
            );
            preferences.set_tenxer_mode(true);
            assert_eq!(preview_shortcut(&toggle), "");
            assert_eq!(
                toggle.tooltip_text().as_deref(),
                Some("Toggle preview panel while browsing")
            );
            preferences.set_tenxer_mode(false);
            assert_eq!(preview_shortcut(&toggle), "Space");
            window.destroy();
        },
    );
}

fn preview_shortcut(toggle: &gtk::Widget) -> String {
    widget_with_class(toggle, "folder-context-shortcut")
        .and_then(|widget| widget.downcast::<gtk::Label>().ok())
        .filter(|label| label.is_visible())
        .map(|label| label.text().to_string())
        .unwrap_or_default()
}

#[test]
fn hidden_shortcut_button_keeps_prompt_chord_and_feedback_usable() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::hidden_shortcut_button_keeps_prompt_chord_and_feedback_usable",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(true);
            preferences.set_show_keybinding_hints(false);
            fixture.shortcuts.bind_preferences(&preferences);
            pump(50);
            let button = widget_with_class(fixture.window.upcast_ref(), "shortcut-footer-button")
                .expect("shortcuts button");
            assert!(!button.is_visible());
            assert!(fixture.shortcuts.tag_visible());
            fixture.shortcuts.show_prompt();
            fixture.shortcuts.arm_chord("g-");
            fixture.shortcuts.show_feedback("Copied");
            assert!(gtk::prelude::WidgetExt::is_visible(
                fixture.shortcuts.prompt()
            ));
            assert!(fixture.shortcuts.prompt().is_sensitive());
            assert_eq!(fixture.shortcuts.chord().text(), "g-");
            assert!(fixture.shortcuts.chord().is_visible());
            fixture.shortcuts.prompt().set_text("keep");
            assert!(fixture.shortcuts.prompt().grab_focus());
            let names = directory_names(fixture._directory.path());
            assert!(!fixture.press(Key::Delete, ModifierType::empty()));
            assert_eq!(fixture.shortcuts.prompt().text(), "keep");
            assert_eq!(directory_names(fixture._directory.path()), names);
            assert!(fixture.press(Key::F1, ModifierType::empty()));
            wait_until(|| {
                widget_with_class(fixture.window.upcast_ref(), "shortcut-popover")
                    .is_some_and(|popover| popover.is_visible())
            });
            assert_eq!(fixture.shortcuts.prompt().text(), "keep");
            assert!(fixture.shortcuts.chord().is_visible());
            assert_eq!(fixture.shortcuts.chord().text(), "g-");
            fixture.press(Key::Escape, ModifierType::empty());
            wait_until(|| {
                widget_with_class(fixture.window.upcast_ref(), "shortcut-popover")
                    .is_none_or(|popover| !popover.is_visible())
            });
            assert_eq!(fixture.shortcuts.prompt().text(), "keep");
            assert!(fixture.shortcuts.prompt().grab_focus());
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            assert!(!gtk::prelude::WidgetExt::is_visible(
                fixture.shortcuts.prompt()
            ));
            assert!(fixture.shortcuts.prompt().text().is_empty());
            fixture.shortcuts.dismiss_feedback();
            assert!(!widget_text_visible(
                fixture.shortcuts.widget().upcast_ref(),
                "Copied"
            ));
            assert!(fixture.shortcuts.chord().is_visible());
            preferences.set_tenxer_mode(false);
            pump(50);
            assert!(!fixture.shortcuts.chord().is_visible());
            assert!(fixture.shortcuts.chord().text().is_empty());
            assert!(!fixture.shortcuts.tag_visible());
        },
    );
}

fn widget_text_visible(widget: &gtk::Widget, text: &str) -> bool {
    if widget
        .downcast_ref::<gtk::Label>()
        .is_some_and(|label| label.is_visible() && label.text() == text)
    {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if widget_text_visible(&current, text) {
            return true;
        }
        child = current.next_sibling();
    }
    false
}

#[test]
fn tenxer_entries_menus_and_reference_keep_their_keys() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tenxer_entries_menus_and_reference_keep_their_keys",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(true);
            let names = directory_names(fixture._directory.path());
            focus_files(&fixture);
            assert!(fixture.press(Key::F2, ModifierType::empty()));
            let field = fixture.view.active_rename_field().expect("rename field");
            field.set_text("kept.txt");
            field.set_position(-1);
            for key in [Key::q, Key::d, Key::p, Key::a] {
                assert!(!fixture.press(key, ModifierType::empty()), "{key:?}");
                assert_eq!(field.text(), "kept.txt");
                assert!(preferences.tenxer_mode());
                assert_eq!(fixture.selected(), [0]);
                assert_eq!(directory_names(fixture._directory.path()), names);
            }
            assert!(fixture.press(Key::a, ModifierType::CONTROL_MASK));
            assert_eq!(
                field.selection_bounds(),
                Some((0, field.text().chars().count() as i32))
            );
            assert_eq!(fixture.selected(), [0]);
            assert!(fixture.press(Key::Escape, ModifierType::empty()));

            focus_files(&fixture);
            assert!(fixture.press(Key::l, ModifierType::CONTROL_MASK));
            wait_until(|| fixture.view.location_has_focus());
            let location = focused_entry(&fixture.window);
            location.set_text("kept-location");
            location.set_position(-1);
            for key in [Key::q, Key::d, Key::p, Key::a] {
                assert!(!fixture.press(key, ModifierType::empty()), "{key:?}");
                assert_eq!(location.text(), "kept-location");
                assert!(preferences.tenxer_mode());
                assert_eq!(fixture.selected(), [0]);
                assert_eq!(directory_names(fixture._directory.path()), names);
                assert!(fixture.view.location_has_focus());
            }
            assert!(fixture.press(Key::a, ModifierType::CONTROL_MASK));
            assert_eq!(
                location.selection_bounds(),
                Some((0, location.text().chars().count() as i32))
            );
            assert!(fixture.press(
                Key::m,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK
            ));
            assert!(!preferences.tenxer_mode());
            assert!(fixture.view.location_has_focus());
            assert!(fixture.press(
                Key::m,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK
            ));
            assert!(preferences.tenxer_mode());
            assert!(fixture.press(Key::Escape, ModifierType::empty()));

            focus_files(&fixture);
            assert!(fixture.press(Key::F1, ModifierType::empty()));
            wait_until(|| {
                widget_with_class(fixture.window.upcast_ref(), "shortcut-popover")
                    .is_some_and(|popover| popover.is_visible())
            });
            fixture.press(Key::q, ModifierType::empty());
            assert!(preferences.tenxer_mode());
            assert_eq!(directory_names(fixture._directory.path()), names);
            assert_eq!(fixture.selected(), [0]);
            fixture.press(Key::Delete, ModifierType::empty());
            assert_eq!(
                directory_names(fixture._directory.path()),
                names,
                "Delete must not remove files while the reference is open"
            );
            assert!(fixture.press(Key::asciitilde, ModifierType::empty()));
            wait_until(|| {
                widget_with_class(fixture.window.upcast_ref(), "shortcut-popover")
                    .is_none_or(|popover| !popover.is_visible())
            });

            focus_files(&fixture);
            assert!(fixture.press(Key::Menu, ModifierType::empty()));
            wait_until(|| visible_menu(fixture.window.upcast_ref()).is_some());
            fixture.press(Key::q, ModifierType::empty());
            fixture.press(Key::d, ModifierType::empty());
            fixture.press(Key::a, ModifierType::empty());
            assert!(preferences.tenxer_mode());
            assert_eq!(directory_names(fixture._directory.path()), names);
            assert_eq!(fixture.selected(), [0]);
            visible_menu(fixture.window.upcast_ref())
                .expect("menu")
                .popdown();

            let modal = gtk::Box::new(gtk::Orientation::Vertical, 0);
            modal.set_focusable(true);
            modal.add_css_class("app-modal-layer");
            fixture.overlay.add_overlay(&modal);
            modal.grab_focus();
            wait_until(|| modal.has_focus());
            fixture.press(
                Key::m,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK,
            );
            assert!(preferences.tenxer_mode());
            assert!(modal.is_visible());
            let capture = fixture.press(Key::comma, ModifierType::CONTROL_MASK);
            assert!(preferences.tenxer_mode());
            assert!(modal.has_focus() || capture);

            let (window, content) = composed_window();
            let directory = tempfile::tempdir().expect("settings folder");
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
            content.browser.browser().focus_active();
            wait_until(|| content.browser.item_view_has_focus());
            let capture = press_phase(
                &window,
                gtk::PropagationPhase::Capture,
                Key::comma,
                ModifierType::CONTROL_MASK,
            );
            assert!(!capture, "Ctrl+, must reach Settings");
            assert!(press_phase(
                &window,
                gtk::PropagationPhase::Bubble,
                Key::comma,
                ModifierType::CONTROL_MASK
            ));
            wait_until(|| settings_layer(&content).is_some());
            let layer = settings_layer(&content).expect("settings");
            assert!(layer.is_visible());
            layer.set_visible(false);
            let blocking = gtk::Box::new(gtk::Orientation::Vertical, 0);
            blocking.set_focusable(true);
            blocking.add_css_class("app-modal-layer");
            content.overlay().add_overlay(&blocking);
            blocking.grab_focus();
            press_phase(
                &window,
                gtk::PropagationPhase::Bubble,
                Key::comma,
                ModifierType::CONTROL_MASK,
            );
            assert!(settings_layer(&content).is_none_or(|layer| !layer.is_visible()));
            press_phase(
                &window,
                gtk::PropagationPhase::Capture,
                Key::m,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK,
            );
            assert!(preferences.tenxer_mode());
            assert!(blocking.is_visible());
            window.destroy();
        },
    );
}

#[test]
fn tenxer_list_and_columns_move_enter_and_traverse_history() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tenxer_list_and_columns_move_enter_and_traverse_history",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(true);
            preferences.set_columns_mirror_selection(true);
            preferences.set_group_by_type(false);
            preferences.set_single_click_previews(true);
            let browser = fixture.view.browser();
            let root = fixture._directory.path().to_path_buf();
            std::fs::create_dir(root.join("nest")).expect("nest");
            std::fs::write(root.join("nest/inside.txt"), b"inside").expect("inside");
            std::fs::create_dir(root.join("empty")).expect("empty");
            std::fs::create_dir(root.join("aaa-dir")).expect("group folder");
            std::fs::write(root.join("a.png"), b"p").expect("png");
            std::fs::write(root.join("m.txt"), b"m").expect("text");
            for index in 0..40 {
                std::fs::write(root.join(format!("n{index:02}.txt")), b"n").expect("page file");
            }
            fixture.view.refresh();
            wait_loaded(&browser, 0);
            browser.set_folders_first(0, false);
            browser.set_sort(0, SortKey::Name, SortDirection::Ascending);
            wait_until(|| {
                source_names(&browser)
                    .first()
                    .is_some_and(|name| name == "a.png")
            });
            let origin = browser.active_location();

            fixture.view.set_view_mode(BrowserMode::List);
            focus_files(&fixture);
            wait_until(|| {
                list_display_names(&fixture.view.widget())
                    .first()
                    .is_some_and(|name| name == "a.png")
            });
            fixture.press(Key::Home, ModifierType::empty());
            assert_eq!(focused_name(&browser), "a.png");
            fixture.press(Key::j, ModifierType::empty());
            assert_eq!(focused_name(&browser), "a.txt");
            fixture.press(Key::k, ModifierType::empty());
            assert_eq!(focused_name(&browser), "a.png");
            fixture.press(Key::Down, ModifierType::empty());
            assert_eq!(focused_name(&browser), "a.txt");
            fixture.press(Key::Up, ModifierType::empty());
            assert_eq!(focused_name(&browser), "a.png");
            assert!(fixture.view.item_view_has_focus());
            assert!(!sidebar_has_focus(&fixture));
            fixture.press(Key::End, ModifierType::empty());
            let last = focused_name(&browser);
            fixture.press(Key::Home, ModifierType::empty());
            fixture.press(Key::G, ModifierType::SHIFT_MASK);
            assert_eq!(focused_name(&browser), last, "G reaches the last item");
            fixture.press(Key::Home, ModifierType::empty());
            assert_eq!(focused_name(&browser), "a.png");

            let start = focused_index(&browser);
            let count = entry_count(&browser);
            fixture.press(Key::d, ModifierType::CONTROL_MASK);
            let half = focused_index(&browser);
            assert!(half > start, "Ctrl+D moves down");
            assert!(half + 1 < count, "half page stays inside the listing");
            fixture.press(Key::Home, ModifierType::empty());
            fixture.press(Key::Page_Down, ModifierType::empty());
            let page_down = focused_index(&browser);
            fixture.press(Key::Home, ModifierType::empty());
            fixture.press(Key::f, ModifierType::CONTROL_MASK);
            let full = focused_index(&browser);
            assert!(full > start && page_down > start);
            assert_eq!(full, page_down, "Ctrl+F and Page Down move one full page");
            if half - start > 1 {
                assert!(full > half, "a full page moves farther than a half page");
            }
            fixture.press(Key::End, ModifierType::empty());
            let end = focused_index(&browser);
            fixture.press(Key::d, ModifierType::CONTROL_MASK);
            fixture.press(Key::Page_Down, ModifierType::empty());
            fixture.press(Key::f, ModifierType::CONTROL_MASK);
            assert_eq!(
                focused_index(&browser),
                end,
                "paging down clamps at the last item"
            );
            fixture.press(Key::u, ModifierType::CONTROL_MASK);
            let half_up = focused_index(&browser);
            assert!(half_up < end, "Ctrl+U moves up");
            fixture.press(Key::End, ModifierType::empty());
            fixture.press(Key::Page_Up, ModifierType::empty());
            let page_up = focused_index(&browser);
            fixture.press(Key::End, ModifierType::empty());
            fixture.press(Key::b, ModifierType::CONTROL_MASK);
            assert_eq!(
                focused_index(&browser),
                page_up,
                "Ctrl+B and Page Up move one full page"
            );
            fixture.press(Key::Home, ModifierType::empty());
            fixture.press(Key::u, ModifierType::CONTROL_MASK);
            fixture.press(Key::Page_Up, ModifierType::empty());
            fixture.press(Key::b, ModifierType::CONTROL_MASK);
            assert_eq!(
                focused_index(&browser),
                start,
                "paging up clamps at the first item"
            );
            assert!(!fixture.view.filter_has_focus());
            assert!(fixture.sidebar_toggle.is_active());
            assert!(!fixture.preview.is_open());

            browser.set_sort(0, SortKey::Name, SortDirection::Descending);
            wait_until(|| {
                source_names(&browser)
                    .first()
                    .is_some_and(|name| name != "a.png")
            });
            focus_files(&fixture);
            fixture.press(Key::Home, ModifierType::empty());
            let descending = source_names(&browser);
            assert_eq!(focused_name(&browser), descending[0]);
            fixture.press(Key::j, ModifierType::empty());
            assert_eq!(focused_name(&browser), descending[1]);
            fixture.press(Key::k, ModifierType::empty());
            assert_eq!(focused_name(&browser), descending[0]);

            browser.set_sort(0, SortKey::Name, SortDirection::Ascending);
            preferences.set_group_by_type(true);
            wait_until(|| {
                list_display_names(&fixture.view.widget())
                    .first()
                    .is_some_and(|name| name == "aaa-dir")
            });
            let displayed = list_display_names(&fixture.view.widget());
            assert_ne!(
                displayed,
                source_names(&browser),
                "grouped rows are not source order"
            );
            focus_files(&fixture);
            fixture.press(Key::Home, ModifierType::empty());
            let mut walked = vec![focused_name(&browser)];
            for _ in 1..displayed.len().min(6) {
                fixture.press(Key::j, ModifierType::empty());
                walked.push(focused_name(&browser));
            }
            assert_eq!(&walked[..], &displayed[..walked.len()]);

            preferences.set_group_by_type(false);
            wait_until(|| {
                list_display_names(&fixture.view.widget())
                    .first()
                    .is_some_and(|name| name == "a.png")
            });
            move_to_named(&fixture, &browser, "nest");
            fixture.press(Key::l, ModifierType::empty());
            wait_loaded(&browser, 0);
            assert!(
                location_ends_with(browser.active_location(), "nest"),
                "l from {} stayed at {:?}",
                focused_name(&browser),
                browser
                    .active_location()
                    .map(|location| location.display_path())
            );
            assert!(!sidebar_has_focus(&fixture));
            fixture.press(Key::H, ModifierType::SHIFT_MASK);
            wait_loaded(&browser, 0);
            assert_eq!(browser.active_location(), origin);
            fixture.press(Key::L, ModifierType::SHIFT_MASK);
            wait_loaded(&browser, 0);
            assert!(location_ends_with(browser.active_location(), "nest"));
            fixture.press(Key::BackSpace, ModifierType::empty());
            wait_loaded(&browser, 0);
            assert_eq!(browser.active_location(), origin);
            move_to_named(&fixture, &browser, "nest");
            fixture.press(Key::Right, ModifierType::empty());
            wait_loaded(&browser, 0);
            fixture.press(Key::Up, ModifierType::ALT_MASK);
            wait_loaded(&browser, 0);
            assert_eq!(browser.active_location(), origin);
            fixture.press(Key::Left, ModifierType::ALT_MASK);
            wait_loaded(&browser, 0);
            assert!(location_ends_with(browser.active_location(), "nest"));
            fixture.press(Key::Right, ModifierType::ALT_MASK);
            wait_loaded(&browser, 0);
            assert_eq!(browser.active_location(), origin);

            move_to_named(&fixture, &browser, "empty");
            fixture.press(Key::o, ModifierType::empty());
            wait_loaded(&browser, 0);
            assert_eq!(entry_count(&browser), 0);
            focus_files(&fixture);
            for key in [Key::j, Key::k, Key::l, Key::Right, Key::i, Key::Return] {
                fixture.press(key, ModifierType::empty());
                assert!(!sidebar_has_focus(&fixture), "{key:?}");
            }
            fixture.press(Key::h, ModifierType::empty());
            wait_loaded(&browser, 0);
            assert_eq!(browser.active_location(), origin);
            assert!(!sidebar_has_focus(&fixture));

            fixture.view.set_view_mode(BrowserMode::Columns);
            wait_loaded(&browser, 0);
            assert!(!fixture.view.columns_mirror_selection_enabled());
            select_named(&fixture, "nest");
            pump(200);
            assert!(
                browser.column_snapshot(1).is_none(),
                "moving onto a directory must not open a child column"
            );
            focus_files(&fixture);
            fixture.press(Key::j, ModifierType::empty());
            pump(200);
            assert!(browser.column_snapshot(1).is_none());
            assert!(!fixture.preview.is_open());
            select_named(&fixture, "a.txt");
            fixture.press(Key::i, ModifierType::empty());
            pump(200);
            assert!(
                browser.column_snapshot(1).is_none(),
                "i on a file opens nothing"
            );
            assert!(!fixture.preview.is_open());
            select_named(&fixture, "nest");
            fixture.press(Key::i, ModifierType::empty());
            wait_loaded(&browser, 1);
            assert!(location_ends_with(browser.location_at(1), "nest"));
            assert_eq!(browser.focused_item().map(|(depth, _, _)| depth), Some(0));
            let child = browser.location_at(1);
            fixture.press(Key::j, ModifierType::empty());
            pump(200);
            assert_eq!(browser.location_at(1), child);
            assert_eq!(browser.focused_item().map(|(depth, _, _)| depth), Some(0));
            assert!(fixture.view.widget().has_css_class("keyboard-navigation"));
            fixture.view.record_pointer_hover((12.0, 24.0), Some(1));
            assert!(
                !fixture.view.widget().has_css_class("keyboard-navigation"),
                "pointer motion claims command destination"
            );
            let before = focused_index(&browser);
            fixture.press(Key::k, ModifierType::empty());
            assert_ne!(focused_index(&browser), before);
            assert_eq!(browser.focused_item().map(|(depth, _, _)| depth), Some(0));
            assert!(fixture.view.widget().has_css_class("keyboard-navigation"));
            fixture.view.record_pointer_hover((12.0, 24.0), Some(1));
            assert!(
                fixture.view.widget().has_css_class("keyboard-navigation"),
                "a parked pointer must not reclaim the destination"
            );
            fixture.press(Key::j, ModifierType::empty());
            assert_eq!(browser.focused_item().map(|(depth, _, _)| depth), Some(0));
            assert_eq!(browser.location_at(1), child);

            select_named(&fixture, "nest");
            fixture.press(Key::l, ModifierType::empty());
            wait_loaded(&browser, 1);
            assert_eq!(browser.focused_item().map(|(depth, _, _)| depth), Some(1));
            assert!(!sidebar_has_focus(&fixture));
            fixture.press(Key::h, ModifierType::empty());
            wait_until(|| browser.column_snapshot(1).is_none());
            assert_eq!(browser.focused_item().map(|(depth, _, _)| depth), Some(0));
            assert!(!sidebar_has_focus(&fixture));

            select_named(&fixture, "empty");
            fixture.press(Key::Right, ModifierType::empty());
            wait_loaded(&browser, 1);
            assert_eq!(
                browser.column_snapshot(1).map(|column| column.count),
                Some(0)
            );
            focus_files(&fixture);
            fixture.press(Key::i, ModifierType::empty());
            pump(100);
            assert!(browser.column_snapshot(2).is_none());
            fixture.press(Key::Left, ModifierType::empty());
            wait_until(|| browser.column_snapshot(1).is_none());
            assert!(!sidebar_has_focus(&fixture));

            select_named(&fixture, "nest");
            fixture.press(Key::Right, ModifierType::empty());
            wait_loaded(&browser, 1);
            fixture.press(Key::H, ModifierType::SHIFT_MASK);
            wait_loaded(&browser, 0);
            assert!(browser.column_snapshot(1).is_none());
            fixture.press(Key::L, ModifierType::empty());
            wait_loaded(&browser, 1);
            assert!(location_ends_with(browser.location_at(1), "nest"));

            preferences.set_tenxer_mode(false);
            assert!(
                fixture.view.columns_mirror_selection_enabled(),
                "turning the mode off restores saved column mirroring"
            );

            browser.navigate(Location::local("/"));
            wait_loaded(&browser, 0);
            preferences.set_tenxer_mode(true);
            focus_files(&fixture);
            let filesystem_root = browser.active_location();
            fixture.press(Key::h, ModifierType::empty());
            fixture.press(Key::Left, ModifierType::empty());
            pump(100);
            assert_eq!(browser.active_location(), filesystem_root);
            assert!(!sidebar_has_focus(&fixture));
            assert!(
                browser
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
            );

            preferences.set_tenxer_mode(false);
            browser.navigate(Location::local(&root));
            wait_loaded(&browser, 0);
            fixture.view.set_view_mode(BrowserMode::List);
            focus_files(&fixture);
            let before_duplicate = directory_names(&root);
            fixture.press(Key::d, ModifierType::CONTROL_MASK);
            wait_until(|| directory_names(&root).len() > before_duplicate.len());
            fixture.press(Key::f, ModifierType::CONTROL_MASK);
            assert!(
                fixture.view.filter_has_focus(),
                "Ctrl+F filters when the mode is off"
            );
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            assert!(fixture.sidebar_toggle.is_active());
            fixture.press(Key::b, ModifierType::CONTROL_MASK);
            assert!(!fixture.sidebar_toggle.is_active());

            preferences.set_tenxer_mode(true);
            focus_files(&fixture);
            move_to_named(&fixture, &browser, "a.txt");
            let opened = Rc::new(RefCell::new(Vec::new()));
            let record = opened.clone();
            browser.observe(move |event| {
                if let BrowserEvent::OpenRequested { location } = event {
                    record.borrow_mut().push(location.clone());
                }
            });
            let stayed = browser.active_location();
            fixture.press(Key::l, ModifierType::empty());
            fixture.press(Key::Right, ModifierType::empty());
            assert!(
                opened.borrow().is_empty(),
                "plain l / Right must not launch a file"
            );
            assert_eq!(browser.active_location(), stayed);
            assert!(!fixture.preview.is_open());
            fixture.press(Key::o, ModifierType::empty());
            assert!(
                opened
                    .borrow()
                    .iter()
                    .any(|location| location_ends_with(Some(location.clone()), "a.txt")),
                "o launches the focused file"
            );
            opened.borrow_mut().clear();
            fixture.press(Key::Return, ModifierType::empty());
            assert!(
                opened
                    .borrow()
                    .iter()
                    .any(|location| location_ends_with(Some(location.clone()), "a.txt")),
                "Enter launches the focused file"
            );
        },
    );
}

fn file_panes_have_focus(fixture: &KeyboardFixture) -> bool {
    let panes = fixture.view.widget();
    gtk::prelude::RootExt::focus(&fixture.window)
        .is_some_and(|focused| focused == panes || focused.is_ancestor(&panes))
}

fn sidebar_has_focus(fixture: &KeyboardFixture) -> bool {
    gtk::prelude::RootExt::focus(&fixture.window).is_some_and(|focused| {
        focused == fixture.sidebar.widget || focused.is_ancestor(&fixture.sidebar.widget)
    })
}

fn focus_files(fixture: &KeyboardFixture) {
    fixture.view.browser().focus_active();
    wait_until(|| fixture.view.item_view_has_focus());
}

fn move_to_named(fixture: &KeyboardFixture, browser: &crate::app::Browser, name: &str) {
    // History restoration ignores selection changes until its frame ticks finish.
    pump(300);
    focus_files(fixture);
    fixture.press(Key::Home, ModifierType::empty());
    let mut steps = 0;
    while focused_name(browser) != name {
        assert!(
            fixture.press(Key::j, ModifierType::empty()),
            "j should move toward {name}"
        );
        steps += 1;
        assert!(
            steps < 80,
            "could not reach {name}, stopped on {}",
            focused_name(browser)
        );
    }
}

fn select_named(fixture: &KeyboardFixture, name: &str) {
    let browser = fixture.view.browser();
    let count = browser.column_snapshot(0).expect("column").count;
    let position = browser
        .with_entries(0, 0..count, |entries| {
            entries.iter().position(|entry| entry.display_name == name)
        })
        .flatten()
        .unwrap_or_else(|| panic!("{name} is listed"));
    browser.select(0, position);
    focus_files(fixture);
}

fn directory_names(path: &std::path::Path) -> Vec<String> {
    let mut names = std::fs::read_dir(path)
        .expect("fixture directory")
        .map(|entry| {
            entry
                .expect("fixture entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn wait_loaded(browser: &crate::app::Browser, depth: usize) {
    wait_until(|| {
        browser
            .column_snapshot(depth)
            .is_some_and(|column| !column.loading)
    });
}

fn entry_count(browser: &crate::app::Browser) -> usize {
    browser
        .column_snapshot(0)
        .map(|column| column.count)
        .unwrap_or(0)
}

fn source_names(browser: &crate::app::Browser) -> Vec<String> {
    let count = entry_count(browser);
    browser
        .with_entries(0, 0..count, |entries| {
            entries
                .iter()
                .map(|entry| entry.display_name.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn focused_name(browser: &crate::app::Browser) -> String {
    browser
        .focused_entry()
        .map(|entry| entry.display_name)
        .unwrap_or_default()
}

fn focused_index(browser: &crate::app::Browser) -> usize {
    browser
        .focused_item()
        .map(|(_, position, _)| position)
        .unwrap_or(0)
}

fn location_ends_with(location: Option<crate::model::Location>, name: &str) -> bool {
    location
        .as_ref()
        .and_then(|location| location.native_path())
        .is_some_and(|path| path.ends_with(name))
}

fn list_display_names(widget: &gtk::Widget) -> Vec<String> {
    if let Some(list) = widget.downcast_ref::<gtk::ListView>()
        && list.is_visible()
        && list.has_css_class("file-list-mode")
        && let Some(model) = list.model()
    {
        return (0..model.n_items())
            .filter_map(|position| {
                let value = model
                    .item(position)?
                    .downcast::<gtk::StringObject>()
                    .ok()?
                    .string();
                Some(
                    value
                        .split_once('\t')
                        .map_or_else(|| value.to_string(), |(_, name)| name.to_owned()),
                )
            })
            .collect();
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        let names = list_display_names(&widget);
        if !names.is_empty() {
            return names;
        }
        child = widget.next_sibling();
    }
    Vec::new()
}

fn pump(millis: u64) {
    let deadline = Instant::now() + Duration::from_millis(millis);
    while Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn modal_visible(overlay: &gtk::Overlay) -> bool {
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        if widget.is_visible() && widget.has_css_class("app-modal-layer") {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

fn click_class(widget: &impl IsA<gtk::Widget>, class: &str) -> bool {
    let widget = widget.as_ref();
    if widget.is_visible()
        && widget.has_css_class(class)
        && let Some(button) = widget.downcast_ref::<gtk::Button>()
    {
        button.emit_clicked();
        return true;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if click_class(&widget, class) {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

fn visible_menu(widget: &gtk::Widget) -> Option<gtk::PopoverMenu> {
    if widget.is_visible()
        && let Some(menu) = widget.downcast_ref::<gtk::PopoverMenu>()
    {
        return Some(menu.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(menu) = visible_menu(&widget) {
            return Some(menu);
        }
        child = widget.next_sibling();
    }
    None
}

fn focused_entry(window: &gtk::ApplicationWindow) -> gtk::Entry {
    let focused = gtk::prelude::RootExt::focus(window).expect("focus");
    focused
        .clone()
        .downcast()
        .ok()
        .or_else(|| {
            focused
                .ancestor(gtk::Entry::static_type())
                .and_then(|entry| entry.downcast().ok())
        })
        .expect("focused entry")
}

fn child_commands() -> Vec<String> {
    let output = std::process::Command::new("ps")
        .args(["--ppid", &std::process::id().to_string(), "-o", "comm="])
        .output()
        .expect("ps lists child processes");
    let mut commands = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|command| !command.is_empty() && *command != "ps")
        .map(str::to_owned)
        .collect::<Vec<_>>();
    commands.sort();
    commands
}

fn composed_window() -> (
    gtk::ApplicationWindow,
    super::super::composition::WindowContent,
) {
    let preferences = PreferenceManager::shared();
    let application = gtk::gio::Application::default()
        .and_downcast::<gtk::Application>()
        .unwrap_or_else(|| {
            let application =
                gtk::Application::new(None::<&str>, gtk::gio::ApplicationFlags::NON_UNIQUE);
            application
                .register(None::<&gtk::gio::Cancellable>)
                .expect("test application registration");
            application
        });
    let window = gtk::ApplicationWindow::builder()
        .application(&application)
        .default_width(1200)
        .default_height(760)
        .build();
    let content = super::super::composition::WindowContent::new(&window, &preferences);
    content.bind(&window, &preferences);
    window.present();
    (window, content)
}

fn press_phase(
    window: &gtk::ApplicationWindow,
    phase: gtk::PropagationPhase,
    key: Key,
    modifiers: ModifierType,
) -> bool {
    let controllers = window.observe_controllers();
    let keys = (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .find(|keys| keys.propagation_phase() == phase)
        .unwrap_or_else(|| panic!("{phase:?} key controller"));
    keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers])
}

fn settings_layer(content: &super::super::composition::WindowContent) -> Option<gtk::Widget> {
    let mut child = content.overlay().first_child();
    while let Some(widget) = child {
        if widget.has_css_class("settings-backdrop") {
            return Some(widget);
        }
        child = widget.next_sibling();
    }
    None
}
