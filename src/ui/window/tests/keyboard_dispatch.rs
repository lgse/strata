// SPDX-License-Identifier: MIT

use gtk::gdk::{Key, ModifierType};

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
                shortcuts,
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
            for key in [
                Key::h,
                Key::j,
                Key::k,
                Key::l,
                Key::s,
                Key::S,
                Key::y,
                Key::space,
            ] {
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
            select_named(&fixture, "folder");
            fixture.press(Key::p, ModifierType::empty());
            assert_eq!(pins.get(), 0, "p must not pin while 10xer is on");
            fixture.view.browser().select(0, 0);
            focus_files(&fixture);
            let control = ModifierType::CONTROL_MASK;
            for key in [Key::d, Key::f, Key::b, Key::r, Key::backslash] {
                fixture.press(key, control);
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
            if let Some(popover) =
                widget_with_class(fixture.window.upcast_ref(), "shortcut-popover")
                    .and_then(|widget| widget.downcast::<gtk::Popover>().ok())
            {
                popover.popdown();
            }
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
