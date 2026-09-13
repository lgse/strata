// SPDX-License-Identifier: MIT

mod media_keys;

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
        Self::with_provider(Rc::new(super::type_to_search::TextPreview))
    }

    fn with_provider(provider: Rc<dyn crate::services::PreviewProvider>) -> Self {
        ThemeManager::seed_saved_preferences_for_test();
        let preferences = ThemeManager::shared();
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
        let top_bar = TopBarNavigation::new(&header, &sidebar.widget, &toggle);
        let preview = PreviewDrawer::new(provider, false);
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

fn text_view_in(widget: &gtk::Widget) -> Option<gtk::TextView> {
    if let Some(view) = widget.downcast_ref::<gtk::TextView>() {
        return Some(view.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(view) = text_view_in(&widget) {
            return Some(view);
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
fn ctrl_a_during_rename_selects_only_unicode_entry_text_in_every_view() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::ctrl_a_during_rename_selects_only_unicode_entry_text_in_every_view",
        || {
            let fixture = KeyboardFixture::new();
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                fixture.view.set_view_mode(mode);
                fixture.view.browser().select(0, 0);
                fixture.view.browser().focus_active();
                wait_until(|| {
                    fixture.view.item_view_has_focus()
                        && rendered_name(&fixture.view.widget(), "a.txt")
                });
                assert!(fixture.press(Key::F2, ModifierType::empty()), "{mode:?}");
                assert!(fixture.view.rename_is_active(), "{mode:?}");
                let field = fixture.view.active_rename_field().expect("rename field");
                field.set_text("résumé-💾.txt");
                field.set_position(-1);

                assert!(
                    fixture.press(Key::a, ModifierType::CONTROL_MASK),
                    "{mode:?}"
                );
                assert_eq!(
                    field.selection_bounds(),
                    Some((0, field.text().chars().count() as i32)),
                    "{mode:?}"
                );
                assert_eq!(fixture.selected(), [0], "{mode:?}");

                assert!(
                    fixture.press(Key::Escape, ModifierType::empty()),
                    "{mode:?}"
                );
                assert!(!fixture.view.rename_is_active(), "{mode:?}");
                assert_eq!(fixture.selected(), [0], "{mode:?}");
            }
        },
    );
}

#[test]
fn clipboard_and_delete_shortcuts_proceed_inside_preview_text() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::clipboard_and_delete_shortcuts_proceed_inside_preview_text",
        || {
            let fixture = KeyboardFixture::new();
            assert!(fixture.press(Key::space, ModifierType::empty()));
            wait_until(|| {
                fixture.preview.is_open() && text_view_in(&fixture.preview.widget()).is_some()
            });
            let text = text_view_in(&fixture.preview.widget()).expect("preview text");
            text.grab_focus();
            wait_until(|| text.has_focus());

            for key in [Key::a, Key::c, Key::d, Key::v, Key::x] {
                assert!(
                    !fixture.press(key, ModifierType::CONTROL_MASK),
                    "{key:?} should reach the text view"
                );
            }
            assert!(!fixture.press(Key::Delete, ModifierType::empty()));
            assert!(!fixture.press(Key::Delete, ModifierType::SHIFT_MASK));
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

#[test]
fn shift_after_escape_starts_on_the_focused_entry() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::shift_after_escape_starts_on_the_focused_entry",
        || {
            let fixture = KeyboardFixture::new();
            for (mode, next) in [
                (BrowserMode::Columns, Key::Down),
                (BrowserMode::List, Key::Down),
                (BrowserMode::Icons, Key::Right),
            ] {
                fixture.view.set_view_mode(mode);
                fixture.view.browser().select(0, 0);
                fixture.view.browser().focus_active();
                wait_until(|| fixture.view.item_view_has_focus() && fixture.selected() == [0]);

                assert!(fixture.press(Key::Escape, ModifierType::empty()));
                assert!(
                    fixture.selected().is_empty(),
                    "{mode:?}: Escape must clear filled selection"
                );

                assert!(
                    fixture.press(next, ModifierType::SHIFT_MASK),
                    "{mode:?}: first Shift after Escape must start on the cursor"
                );
                assert_eq!(fixture.selected(), [0], "{mode:?}");
                assert_eq!(
                    fixture.view.browser().selection_anchor_position(0),
                    Some(0),
                    "{mode:?}"
                );

                if mode == BrowserMode::Columns {
                    assert!(fixture.press(next, ModifierType::SHIFT_MASK));
                    assert_eq!(fixture.selected(), [0, 1], "{mode:?}");

                    assert!(fixture.press(Key::Escape, ModifierType::empty()));
                    assert!(fixture.selected().is_empty(), "{mode:?}");
                    assert!(fixture.press(next, ModifierType::SHIFT_MASK));
                    assert_eq!(
                        fixture.selected(),
                        [1],
                        "{mode:?}: leftover range anchor must not expand after Escape"
                    );
                } else {
                    assert!(
                        !fixture.press(next, ModifierType::SHIFT_MASK),
                        "{mode:?}: further Shift arrows stay native once a range exists"
                    );
                }
            }
        },
    );
}

#[test]
fn arrow_scope_preference_keeps_up_in_the_file_list() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::arrow_scope_preference_keeps_up_in_the_file_list",
        || {
            let fixture = KeyboardFixture::new();
            fixture.view.set_view_mode(BrowserMode::List);
            fixture.view.browser().select(0, 0);
            fixture.view.browser().focus_active();
            wait_until(|| fixture.view.item_view_has_focus() && fixture.selected() == [0]);

            // Seeded preference is on: Up from the first row stays in the file list.
            assert!(!fixture.press(Key::Up, ModifierType::empty()));
            assert!(fixture.view.item_view_has_focus());
            assert!(!fixture.view.header_actions_have_focus());

            // Toggle off: Up from the first row escapes into the pane header.
            ThemeManager::shared().set_arrow_navigation_scoped(false);
            assert!(fixture.press(Key::Up, ModifierType::empty()));
            assert!(fixture.view.header_actions_have_focus());
        },
    );
}
