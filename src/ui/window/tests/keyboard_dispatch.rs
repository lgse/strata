// SPDX-License-Identifier: MIT

mod media_keys;
mod scroll_zoom;

use gtk::gdk::{Key, ModifierType};

use super::super::*;
use crate::services::{
    ArchiveFileEntry, LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewProvider,
    PreviewRequest, archive_preview_tree,
};
use crate::ui::{
    preview::PreviewDrawer, shortcut_footer::ShortcutFooter, top_bar_navigation::TopBarNavigation,
};

struct ArchivePreview;

impl PreviewProvider for ArchivePreview {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        let tree = archive_preview_tree(vec![
            ArchiveFileEntry {
                name: "docs/readme.md".to_owned(),
                directory: false,
                size: 1,
            },
            ArchiveFileEntry {
                name: "top.txt".to_owned(),
                directory: false,
                size: 2,
            },
        ]);
        glib::idle_add_local_once(move || {
            emit(PreviewEvent::Ready(Preview {
                request_id: request.id,
                entry: request.entry,
                content_type: "application/zip".into(),
                content: PreviewContent::Archive { tree },
            }))
        });
        LoadHandle::new(|| {})
    }
}

struct ProtectedArchivePreview;

impl PreviewProvider for ProtectedArchivePreview {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        glib::idle_add_local_once(move || {
            emit(PreviewEvent::NeedsPassword {
                request_id: request.id,
                entry: request.entry,
            });
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
        Self::with_provider(Rc::new(super::type_to_search::TextPreview))
    }

    fn with_provider(provider: Rc<dyn crate::services::PreviewProvider>) -> Self {
        PreferenceManager::seed_saved_preferences_for_test();
        let preferences = PreferenceManager::shared();
        // Keyboard focus-return scenarios need a place to focus; the saved fixture hides all places.
        preferences.set_sidebar_show_home(true);
        // The exhaustive fixture enables Omastrata, which replaces this default map.
        preferences.set_omastrata_mode(false);
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

    fn with_archive() -> Self {
        let fixture = Self::with_provider(Rc::new(ArchivePreview));
        std::fs::write(fixture._directory.path().join("archive.zip"), b"fixture")
            .expect("archive fixture");
        fixture.view.refresh();
        wait_until(|| {
            fixture
                .view
                .browser()
                .column_snapshot(0)
                .is_some_and(|column| !column.loading && column.count == 4)
        });
        fixture.view.browser().select(0, 1);
        fixture.view.browser().focus_active();
        wait_until(|| {
            fixture.view.item_view_has_focus()
                && rendered_name(&fixture.view.widget(), "archive.zip")
        });
        fixture
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

fn press_on(widget: &gtk::Widget, key: Key, modifiers: ModifierType) -> bool {
    let controllers = widget.observe_controllers();
    let controller = (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .next()
        .expect("key controller");
    controller.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers])
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
fn space_opens_folders_without_toggling_preview_in_every_mode() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::space_opens_folders_without_toggling_preview_in_every_mode",
        || {
            let fixture = KeyboardFixture::new();
            std::fs::create_dir(fixture._directory.path().join("folder")).expect("fixture folder");
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                fixture.view.set_view_mode(mode);
                fixture
                    .view
                    .browser()
                    .navigate(Location::local(fixture._directory.path()));
                wait_until(|| {
                    fixture
                        .view
                        .browser()
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading && column.count == 4)
                });
                let position = (0..4)
                    .find(|&position| {
                        fixture
                            .view
                            .browser()
                            .entry_at(0, position)
                            .is_some_and(|entry| {
                                entry.location
                                    == Location::local(fixture._directory.path().join("folder"))
                            })
                    })
                    .expect("folder entry");
                fixture.view.browser().select(0, position);
                fixture.view.browser().focus_active();
                wait_until(|| fixture.view.item_view_has_focus());
                assert!(fixture.press(Key::space, ModifierType::empty()), "{mode:?}");
                wait_until(|| {
                    fixture.view.browser().active_location()
                        == Some(Location::local(fixture._directory.path().join("folder")))
                });
                assert!(!fixture.preview.is_open(), "{mode:?}");
            }
        },
    );
}

#[test]
fn space_opens_filtered_folders_without_toggling_preview() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::space_opens_filtered_folders_without_toggling_preview",
        || {
            let fixture = KeyboardFixture::new();
            let folder = fixture._directory.path().join("folder");
            std::fs::create_dir(&folder).expect("fixture folder");
            for mode in [BrowserMode::Icons, BrowserMode::List] {
                fixture.view.set_view_mode(mode);
                fixture
                    .view
                    .browser()
                    .navigate(Location::local(fixture._directory.path()));
                wait_until(|| {
                    fixture
                        .view
                        .browser()
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading && column.count == 4)
                });
                assert!(fixture.view.show_filter_with_query("folder"));
                let field = gtk::prelude::RootExt::focus(&fixture.window)
                    .expect("filter focus")
                    .ancestor(gtk::Entry::static_type())
                    .and_downcast::<gtk::Entry>()
                    .expect("filter entry");
                wait_until(|| {
                    press_on(field.upcast_ref(), Key::Down, ModifierType::empty());
                    fixture.view.selected_search_result().is_some()
                });
                assert_eq!(
                    fixture
                        .view
                        .selected_search_result()
                        .map(|entry| entry.location),
                    Some(Location::local(&folder))
                );
                assert!(fixture.press(Key::space, ModifierType::empty()), "{mode:?}");
                wait_until(|| {
                    fixture.view.browser().active_location() == Some(Location::local(&folder))
                });
                assert!(!fixture.preview.is_open(), "{mode:?}");
            }
        },
    );
}

#[test]
fn escape_closes_archive_preview_with_password_focus() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::escape_closes_archive_preview_with_password_focus",
        || {
            let fixture = KeyboardFixture::with_provider(Rc::new(ProtectedArchivePreview));
            assert!(fixture.press(Key::space, ModifierType::empty()));
            wait_until(|| {
                let focus = gtk::prelude::RootExt::focus(&fixture.window);
                fixture.preview.password_has_focus(focus.as_ref())
            });
            let selected = fixture.selected();
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until(|| !fixture.preview.is_open());
            assert_eq!(fixture.selected(), selected);
            wait_until(|| fixture.view.item_view_has_focus());
            assert!(
                !fixture
                    .preview
                    .password_has_focus(gtk::prelude::RootExt::focus(&fixture.window).as_ref())
            );
        },
    );
}

#[test]
fn archive_preview_keys_navigate_the_tree_without_moving_the_listing() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::archive_preview_keys_navigate_the_tree_without_moving_the_listing",
        || {
            let fixture = KeyboardFixture::with_archive();
            assert!(fixture.press(Key::space, ModifierType::empty()));
            wait_until(|| {
                widget_with_class(&fixture.preview.widget(), "preview-archive").is_some()
            });

            for key in [
                Key::Down,
                Key::j,
                Key::Up,
                Key::k,
                Key::Right,
                Key::l,
                Key::Left,
                Key::h,
            ] {
                assert!(
                    fixture.press(key, ModifierType::empty()),
                    "{key:?} should stay inside the archive"
                );
            }
            assert_eq!(fixture.selected(), [1]);

            assert!(fixture.press(Key::Down, ModifierType::empty()));
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            assert!(
                fixture._directory.path().join("archive.zip").exists(),
                "archive member activation must not extract"
            );
            assert_eq!(fixture.selected(), [1]);

            assert!(fixture.press(Key::space, ModifierType::empty()));
            wait_until(|| !fixture.preview.is_open());
            // This fixture lacks the split binding that restores listing focus on close.
            fixture.view.browser().focus_active();
            assert!(fixture.press(Key::space, ModifierType::empty()));
            wait_until(|| fixture.preview.is_open());
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until(|| !fixture.preview.is_open());
            assert_eq!(fixture.selected(), [1]);
        },
    );
}

#[test]
fn archive_keys_route_when_the_preview_list_has_focus() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::archive_keys_route_when_the_preview_list_has_focus",
        || {
            let fixture = KeyboardFixture::with_archive();
            assert!(fixture.press(Key::space, ModifierType::empty()));
            wait_until(|| {
                widget_with_class(&fixture.preview.widget(), "preview-archive").is_some()
            });
            let list = widget_with_class(&fixture.preview.widget(), "preview-archive-list")
                .expect("archive list");
            wait_until(|| list.is_mapped());
            assert!(list.grab_focus());
            assert!(!fixture.view.item_view_has_focus());
            assert!(fixture.press(Key::Down, ModifierType::empty()));
            assert_eq!(fixture.selected(), [1]);
            assert!(fixture.press(Key::Up, ModifierType::empty()));
            assert_eq!(fixture.selected(), [1]);
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until(|| !fixture.preview.is_open());
            assert_eq!(fixture.selected(), [1]);
            wait_until(|| fixture.view.item_view_has_focus());
        },
    );
}

#[test]
fn space_opening_archive_focuses_the_tree_first_entry() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::space_opening_archive_focuses_the_tree_first_entry",
        || {
            let fixture = KeyboardFixture::with_archive();
            assert!(fixture.press(Key::space, ModifierType::empty()));
            wait_until(|| {
                widget_with_class(&fixture.preview.widget(), "preview-archive").is_some()
            });
            let list = widget_with_class(&fixture.preview.widget(), "preview-archive-list")
                .expect("archive list");
            wait_until(|| list.is_mapped());
            let focused = gtk::prelude::RootExt::focus(&fixture.window).expect("window focus");
            assert!(
                focused == list || focused.is_ancestor(&list),
                "archive tree must own keyboard focus, got {focused:?}"
            );
            assert_eq!(fixture.selected(), [1]);
            assert!(fixture.press(Key::Down, ModifierType::empty()));
            assert_eq!(fixture.selected(), [1]);
            assert!(fixture.press(Key::Up, ModifierType::empty()));
            assert_eq!(fixture.selected(), [1]);
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until(|| !fixture.preview.is_open());
            assert_eq!(fixture.selected(), [1]);
        },
    );
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
fn view_shortcuts_work_from_the_pane_filter() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::view_shortcuts_work_from_the_pane_filter",
        || {
            let fixture = KeyboardFixture::new();
            for (key, mode) in [
                (Key::_2, BrowserMode::Icons),
                (Key::_3, BrowserMode::List),
                (Key::_1, BrowserMode::Columns),
            ] {
                assert!(fixture.view.show_filter_with_query("a"));
                wait_until(|| fixture.view.filter_has_focus());
                assert!(fixture.press(key, ModifierType::CONTROL_MASK));
                assert_eq!(fixture.view.view_mode(), mode);
            }
        },
    );
}

#[test]
fn inline_editing_and_location_edit_own_search_shortcuts() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::inline_editing_and_location_edit_own_search_shortcuts",
        || {
            let fixture = KeyboardFixture::new();
            let searches = Rc::new(Cell::new(0));
            let observed = searches.clone();
            let action = gio::SimpleAction::new("search", None);
            action.connect_activate(move |_, _| observed.set(observed.get() + 1));
            fixture.window.add_action(&action);
            let jumps = Rc::new(Cell::new(0));
            let observed = jumps.clone();
            let action = gio::SimpleAction::new("jump-folder", None);
            action.connect_activate(move |_, _| observed.set(observed.get() + 1));
            fixture.window.add_action(&action);
            assert!(fixture.press(Key::F2, ModifierType::empty()));
            assert!(fixture.view.rename_is_active());
            assert!(!fixture.press(Key::f, ModifierType::CONTROL_MASK));
            assert!(!fixture.view.filter_has_focus());
            assert!(!fixture.press(Key::k, ModifierType::CONTROL_MASK));
            assert!(!fixture.press(
                Key::k,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK,
            ));
            assert_eq!(searches.get(), 0);
            assert_eq!(jumps.get(), 0);
            assert!(!fixture.press(Key::_2, ModifierType::CONTROL_MASK));
            assert_eq!(fixture.view.view_mode(), BrowserMode::Columns);
            assert!(fixture.view.rename_is_active());
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            assert!(!fixture.view.rename_is_active());
            assert_eq!(fixture.selected(), [0]);
            wait_until(|| {
                !gtk::prelude::RootExt::focus(&fixture.window)
                    .is_some_and(|focused| focused.is::<gtk::Entry>() || focused.is::<gtk::Text>())
            });

            assert!(fixture.press(Key::l, ModifierType::CONTROL_MASK));
            wait_until(|| fixture.view.location_has_focus());
            assert!(!fixture.press(Key::k, ModifierType::CONTROL_MASK));
            assert!(!fixture.press(
                Key::k,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK,
            ));
            assert_eq!(searches.get(), 0);
            assert_eq!(jumps.get(), 0);
            assert!(!fixture.press(Key::_2, ModifierType::CONTROL_MASK));
            assert_eq!(fixture.view.view_mode(), BrowserMode::Columns);
            assert!(fixture.view.location_has_focus());
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            assert!(!fixture.view.location_has_focus());

            assert!(fixture.press(Key::k, ModifierType::CONTROL_MASK));
            assert_eq!(searches.get(), 1);
            assert!(fixture.press(
                Key::k,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK,
            ));
            assert_eq!(jumps.get(), 1);
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
fn delete_trashes_a_selected_filter_result() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::delete_trashes_a_selected_filter_result",
        || {
            let fixture = KeyboardFixture::new();
            fixture
                .view
                .set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            assert!(fixture.view.show_filter_with_query("b.txt"));
            let entry = widget_with_class(&fixture.view.widget(), "column-filter-entry")
                .expect("filter entry");
            wait_until(|| {
                press_on(&entry, Key::Down, ModifierType::empty());
                fixture.view.selected_search_result().is_some()
            });
            fixture.view.browser().focus_active();
            wait_until(|| !fixture.view.filter_has_focus());

            assert!(fixture.press(Key::Delete, ModifierType::empty()));
            wait_until(|| !fixture._directory.path().join("b.txt").exists());
            assert!(fixture._directory.path().join("a.txt").exists());
            assert!(fixture._directory.path().join("c.txt").exists());
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
            let fixtures = [KeyboardFixture::new(), KeyboardFixture::new()];
            let preferences = PreferenceManager::shared();
            assert!(preferences.arrow_navigation_scoped());
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                for scoped in [true, false, true] {
                    preferences.set_arrow_navigation_scoped(scoped);
                    for fixture in &fixtures {
                        fixture.view.set_view_mode(mode);
                        fixture.window.present();
                        fixture.view.browser().select(0, 0);
                        fixture.view.browser().focus_active();
                        wait_until(|| {
                            fixture.view.item_view_has_focus() && fixture.selected() == [0]
                        });

                        fixture.press(Key::Up, ModifierType::empty());
                        assert_eq!(fixture.view.item_view_has_focus(), scoped, "{mode:?}");
                        assert_eq!(
                            fixture.view.header_actions_have_focus(),
                            !scoped,
                            "{mode:?}"
                        );

                        fixture.view.browser().focus_active();
                        wait_until(|| fixture.view.item_view_has_focus());
                        fixture.press(Key::Left, ModifierType::empty());
                        assert_eq!(fixture.view.item_view_has_focus(), scoped, "{mode:?}");
                    }
                }
            }
        },
    );
}

#[test]
fn omastrata_keeps_keyboard_navigation_inside_the_file_panes() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::omastrata_keeps_keyboard_navigation_inside_the_file_panes",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_arrow_navigation_scoped(false);
            preferences.set_omastrata_mode(true);
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

            preferences.set_omastrata_mode(false);
            fixture.view.set_view_mode(BrowserMode::List);
            fixture.view.browser().select(0, 0);
            focus_files(&fixture);
            fixture.press(Key::Up, ModifierType::empty());
            wait_until(|| fixture.view.header_actions_have_focus());
        },
    );
}

#[test]
fn right_from_the_sidebar_returns_to_the_files_after_the_header() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::right_from_the_sidebar_returns_to_the_files_after_the_header",
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_arrow_navigation_scoped(false);
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                fixture.view.set_view_mode(mode);
                fixture.view.browser().select(0, 0);
                fixture.view.browser().focus_active();
                wait_until(|| fixture.view.item_view_has_focus());

                fixture.press(Key::Up, ModifierType::empty());
                wait_until(|| fixture.view.header_actions_have_focus());

                fixture.press(Key::Left, ModifierType::empty());
                wait_until(|| {
                    gtk::prelude::RootExt::focus(&fixture.window)
                        .is_some_and(|focus| focus.is_ancestor(&fixture.sidebar.widget))
                });

                assert!(fixture.press(Key::Right, ModifierType::empty()), "{mode:?}");
                assert!(
                    fixture.view.item_view_has_focus(),
                    "{mode:?}: Right from the sidebar must re-enter the file view"
                );
            }
        },
    );
}

#[test]
fn omastrata_file_list_skips_conflicting_defaults_and_keeps_bound_shortcuts() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::omastrata_file_list_skips_conflicting_defaults_and_keeps_bound_shortcuts",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_type_to_search(true);
            preferences.set_arrow_navigation_scoped(true);
            preferences.set_omastrata_mode(true);
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
                assert!(
                    preferences.omastrata_mode(),
                    "{key:?} must not leave the mode"
                );
            }
            select_named(&fixture, "folder");
            fixture.press(Key::p, ModifierType::empty());
            assert_eq!(pins.get(), 0, "p must not pin while Omastrata is on");
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
            assert!(preferences.omastrata_mode());

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
                    preferences.omastrata_mode(),
                    "{modifier:?}+q must not leave the mode"
                );
                assert!(fixture.window.is_visible());
                fixture.press(Key::Q, modifier | ModifierType::SHIFT_MASK);
                assert!(
                    fixture.window.is_visible(),
                    "{modifier:?}+Shift+Q must not close the window"
                );
                assert!(preferences.omastrata_mode());
            }
            assert_eq!(
                directory_names(fixture._directory.path()),
                names_before_quit
            );
            assert!(fixture.press(Key::q, ModifierType::empty()));
            assert!(!preferences.omastrata_mode());
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
            preferences.set_omastrata_mode(true);
            let other = gtk::Window::new();
            other.present();
            assert!(fixture.press(Key::Q, ModifierType::SHIFT_MASK));
            assert!(!fixture.window.is_visible());
            assert!(other.is_visible());
            assert!(preferences.omastrata_mode());
        },
    );
}

#[test]
fn appearance_menu_hides_space_preview_while_omastrata_is_on() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::appearance_menu_hides_space_preview_while_omastrata_is_on",
        || {
            let preferences = PreferenceManager::shared();
            preferences.set_omastrata_mode(false);
            let view = browser_for_window();
            let preview = PreviewDrawer::new(Rc::new(super::type_to_search::TextPreview), false);
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
            preferences.set_omastrata_mode(true);
            assert_eq!(preview_shortcut(&toggle), "");
            assert_eq!(
                toggle.tooltip_text().as_deref(),
                Some("Toggle preview panel while browsing")
            );
            preferences.set_omastrata_mode(false);
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
            preferences.set_omastrata_mode(true);
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
            preferences.set_omastrata_mode(false);
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
fn omastrata_entries_menus_and_reference_keep_their_keys() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::omastrata_entries_menus_and_reference_keep_their_keys",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_omastrata_mode(true);
            let names = directory_names(fixture._directory.path());
            focus_files(&fixture);
            assert!(fixture.press(Key::F2, ModifierType::empty()));
            let field = fixture.view.active_rename_field().expect("rename field");
            field.set_text("kept.txt");
            field.set_position(-1);
            for key in [Key::q, Key::d, Key::p, Key::a] {
                assert!(!fixture.press(key, ModifierType::empty()), "{key:?}");
                assert_eq!(field.text(), "kept.txt");
                assert!(preferences.omastrata_mode());
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
                assert!(preferences.omastrata_mode());
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
            assert!(!preferences.omastrata_mode());
            assert!(fixture.view.location_has_focus());
            assert!(fixture.press(
                Key::m,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK
            ));
            assert!(preferences.omastrata_mode());
            assert!(fixture.press(Key::Escape, ModifierType::empty()));

            focus_files(&fixture);
            assert!(fixture.press(Key::F1, ModifierType::empty()));
            wait_until(|| {
                widget_with_class(fixture.window.upcast_ref(), "shortcut-popover")
                    .is_some_and(|popover| popover.is_visible())
            });
            fixture.press(Key::q, ModifierType::empty());
            assert!(preferences.omastrata_mode());
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
            assert!(fixture.press(Key::asciitilde, ModifierType::empty()));
            wait_until(|| {
                widget_with_class(fixture.window.upcast_ref(), "shortcut-popover")
                    .is_some_and(|popover| popover.is_visible())
            });
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
            assert!(preferences.omastrata_mode());
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
            assert!(preferences.omastrata_mode());
            assert!(modal.is_visible());
            let capture = fixture.press(Key::comma, ModifierType::CONTROL_MASK);
            assert!(preferences.omastrata_mode());
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
            assert!(preferences.omastrata_mode());
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
