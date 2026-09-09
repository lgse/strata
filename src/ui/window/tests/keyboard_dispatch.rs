// SPDX-License-Identifier: MIT

use gtk::gdk::{Key, ModifierType};

use super::super::*;
use crate::ui::{
    preview::PreviewDrawer, shortcut_footer::ShortcutFooter, top_bar_navigation::TopBarNavigation,
};

struct KeyboardFixture {
    window: gtk::ApplicationWindow,
    overlay: gtk::Overlay,
    view: BrowserView,
    sidebar: SidebarView,
    preview: PreviewDrawer,
    keys: gtk::EventControllerKey,
    _directory: tempfile::TempDir,
}

impl KeyboardFixture {
    fn new() -> Self {
        ThemeManager::seed_saved_preferences_for_test();
        let preferences = ThemeManager::shared();
        let directory = tempfile::tempdir().expect("fixture");
        for name in ["a.txt", "b.txt"] {
            std::fs::write(directory.path().join(name), b"preview").expect("fixture file");
        }
        let view = browser_for_window();
        view.set_view_mode(BrowserMode::Columns);
        let sidebar = build_sidebar(view.clone(), preferences.clone(), true);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let toggle = gtk::ToggleButton::builder().active(true).build();
        header.append(&toggle);
        let top_bar = TopBarNavigation::new(&header, &sidebar.widget, &toggle);
        let preview = PreviewDrawer::new(Rc::new(super::type_to_search::TextPreview), false);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.append(&sidebar.widget);
        row.append(&view.widget());
        row.append(&preview.widget());
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header);
        content.append(&row);
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
                shortcuts: ShortcutFooter::new(BrowserMode::Columns),
            },
        );
        let keys = window
            .observe_controllers()
            .item(0)
            .and_downcast::<gtk::EventControllerKey>()
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

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "keyboard fixture did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn modal_ownership_precedes_window_shortcuts() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::modal_ownership_precedes_window_shortcuts",
        || {
            let fixture = KeyboardFixture::new();
            let modal = gtk::Box::new(gtk::Orientation::Vertical, 0);
            modal.set_focusable(true);
            modal.add_css_class("app-modal-layer");
            fixture.overlay.add_overlay(&modal);
            assert!(fixture.press(Key::_2, ModifierType::CONTROL_MASK));
            assert_eq!(fixture.view.view_mode(), BrowserMode::Columns);
            assert_eq!(
                gtk::prelude::RootExt::focus(&fixture.window),
                Some(modal.upcast())
            );
            assert!(!fixture.press(Key::_2, ModifierType::CONTROL_MASK));
            assert_eq!(fixture.view.view_mode(), BrowserMode::Columns);
        },
    );
}

#[test]
fn inline_editing_owns_filter_keys_but_not_global_search() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::inline_editing_owns_filter_keys_but_not_global_search",
        || {
            let fixture = KeyboardFixture::new();
            let searches = Rc::new(Cell::new(0));
            let observed = searches.clone();
            let action = gio::SimpleAction::new("search", None);
            action.connect_activate(move |_, _| observed.set(observed.get() + 1));
            fixture.window.add_action(&action);
            assert!(fixture.press(Key::F2, ModifierType::empty()));
            assert!(fixture.view.rename_is_active());
            assert!(!fixture.press(Key::f, ModifierType::CONTROL_MASK));
            assert!(!fixture.view.filter_has_focus());
            assert!(fixture.press(Key::k, ModifierType::CONTROL_MASK));
            assert_eq!(searches.get(), 1);
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            assert!(!fixture.view.rename_is_active());
            assert_eq!(fixture.selected(), [0]);
        },
    );
}

#[test]
fn filter_clipboard_proceeds_and_escape_dismisses_one_surface_at_a_time() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::filter_clipboard_proceeds_and_escape_dismisses_one_surface_at_a_time",
        || {
            let fixture = KeyboardFixture::new();
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                fixture.view.set_view_mode(mode);
                fixture.view.browser().select(0, 0);
                fixture.view.browser().focus_active();
                assert!(fixture.press(Key::space, ModifierType::empty()));
                assert!(fixture.preview.is_open());
                assert!(fixture.press(Key::f, ModifierType::CONTROL_MASK));
                assert!(fixture.view.filter_has_focus());
                for key in [Key::a, Key::c, Key::d, Key::v, Key::x] {
                    assert!(
                        !fixture.press(key, ModifierType::CONTROL_MASK),
                        "{mode:?}: {key:?}"
                    );
                }
                assert!(fixture.press(Key::Escape, ModifierType::empty()));
                assert!(!fixture.view.filter_has_focus());
                assert!(fixture.preview.is_open());
                assert_eq!(fixture.selected(), [0]);
                assert!(fixture.press(Key::Escape, ModifierType::empty()));
                assert!(!fixture.preview.is_open());
                assert_eq!(fixture.selected(), [0]);
                assert!(fixture.press(Key::Escape, ModifierType::empty()));
                assert!(fixture.selected().is_empty());
            }
        },
    );
}

#[test]
fn single_pane_arrows_preserve_native_propagation_and_sidebar_focus_return() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::single_pane_arrows_preserve_native_propagation_and_sidebar_focus_return",
        || {
            let fixture = KeyboardFixture::new();
            for mode in [BrowserMode::Icons, BrowserMode::List] {
                fixture.view.set_view_mode(mode);
                fixture.view.browser().focus_active();
                wait_until(|| fixture.view.item_view_has_focus());
                assert!(!fixture.press(Key::Down, ModifierType::empty()));
                assert!(fixture.press(
                    Key::b,
                    ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK
                ));
                wait_until(|| {
                    gtk::prelude::RootExt::focus(&fixture.window)
                        .is_some_and(|focus| focus.is_ancestor(&fixture.sidebar.widget))
                });
                assert!(fixture.press(Key::Right, ModifierType::empty()));
                assert!(fixture.view.item_view_has_focus());
            }
        },
    );
}
