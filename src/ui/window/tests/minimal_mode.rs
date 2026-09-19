// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::gdk::{Key, ModifierType};
use gtk::glib;

use super::super::*;
use crate::{
    app::BrowserEvent,
    test_support::gtk_test,
    ui::{
        preferences::PreferenceManager, preview::PreviewDrawer, shortcut_footer::ShortcutFooter,
        top_bar_navigation::TopBarNavigation,
    },
};

struct MinimalFixture {
    window: gtk::ApplicationWindow,
    view: BrowserView,
    sidebar: SidebarView,
    preview: PreviewDrawer,
    footer: ShortcutFooter,
    keys: gtk::EventControllerKey,
    _directory: tempfile::TempDir,
}

impl MinimalFixture {
    fn new() -> Self {
        PreferenceManager::seed_saved_preferences_for_test();
        Self::from_seeded()
    }

    fn additional() -> Self {
        Self::from_seeded()
    }

    fn from_seeded() -> Self {
        let preferences = PreferenceManager::shared();
        // The saved fixture enables minimal mode; these scenarios exercise it.
        assert!(preferences.minimal_mode());
        preferences.set_sidebar_show_home(true);
        let directory = tempfile::tempdir().expect("fixture");
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(directory.path().join(name), b"preview").expect("fixture file");
        }
        std::fs::create_dir(directory.path().join("sub")).expect("fixture directory");
        let view = browser_for_window();
        view.set_view_mode(BrowserMode::Columns);
        let sidebar = build_sidebar(view.clone(), preferences.clone(), true);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let toggle = gtk::ToggleButton::builder().active(true).build();
        header.append(&toggle);
        header.append(&view.location_widget());
        let top_bar = TopBarNavigation::new(&header, &sidebar.widget, &toggle);
        let preview = PreviewDrawer::new(Rc::new(super::type_to_search::TextPreview), false);
        let shortcuts = ShortcutFooter::new(BrowserMode::Columns);
        shortcuts.bind_minimal_mode(&preferences);
        let footer = shortcuts.clone();
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
                open_settings: Rc::new(|| {}),
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
        let fixture = Self {
            window,
            view,
            sidebar,
            preview,
            footer,
            keys,
            _directory: directory,
        };
        fixture.select_name("a.txt");
        wait_until(|| fixture.view.item_view_has_focus());
        fixture
    }

    fn press(&self, key: Key, modifiers: ModifierType) -> bool {
        self.keys
            .emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers])
    }

    fn index_of(&self, name: &str) -> usize {
        let count = self
            .view
            .browser()
            .column_snapshot(0)
            .expect("loaded column")
            .count;
        (0..count)
            .find(|index| {
                self.view
                    .browser()
                    .entry_at(0, *index)
                    .is_some_and(|entry| entry.display_name == name)
            })
            .expect("fixture entry")
    }

    fn select_name(&self, name: &str) -> usize {
        let position = self.index_of(name);
        self.view.browser().select(0, position);
        self.view.browser().focus_active();
        position
    }

    fn cursor(&self) -> Option<usize> {
        self.view
            .browser()
            .focused_item()
            .map(|(_, position, _)| position)
    }

    fn cursor_name(&self) -> Option<String> {
        self.view
            .browser()
            .focused_item()
            .map(|(_, _, entry)| entry.display_name)
    }

    fn fill(&self) -> Vec<usize> {
        self.view.browser().selected_positions(0)
    }
}

impl Drop for MinimalFixture {
    fn drop(&mut self) {
        self.view.browser().clear_observer();
        self.sidebar.disconnect();
        self.window.destroy();
    }
}

fn highlighted_name(fixture: &MinimalFixture, name: &str) -> Option<String> {
    highlighted_name_in(fixture.view.widget().upcast_ref(), name)
}

fn txt_find_highlights(fixture: &MinimalFixture) -> bool {
    highlighted_name(fixture, "a.txt").as_deref() == Some("txt")
        && highlighted_name(fixture, "b.txt").as_deref() == Some("txt")
        && highlighted_name(fixture, "c.txt").as_deref() == Some("txt")
        && highlighted_name(fixture, "sub").is_none()
}

fn no_find_highlights(fixture: &MinimalFixture) -> bool {
    highlighted_name(fixture, "a.txt").is_none()
        && highlighted_name(fixture, "b.txt").is_none()
        && highlighted_name(fixture, "c.txt").is_none()
        && highlighted_name(fixture, "sub").is_none()
}

fn no_full_name_search_highlight(fixture: &MinimalFixture, name: &str) -> bool {
    highlighted_name(fixture, name).is_none() && !search_hit_name_accent(fixture, name)
}

fn search_hit_name_accent(fixture: &MinimalFixture, name: &str) -> bool {
    label_has_css_class(fixture.view.widget().upcast_ref(), name, "search-hit-name")
}

fn label_has_css_class(widget: &gtk::Widget, name: &str, class: &str) -> bool {
    use gtk::prelude::*;
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && label.label() == name
        && label.has_css_class(class)
    {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if label_has_css_class(&widget, name, class) {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

fn search_txt_hits_visible(fixture: &MinimalFixture) -> bool {
    fixture.view.selected_search_results().is_some()
        && rendered_name(&fixture.view.widget(), "a.txt")
        && rendered_name(&fixture.view.widget(), "b.txt")
        && rendered_name(&fixture.view.widget(), "c.txt")
        && !rendered_name(&fixture.view.widget(), "sub")
}

fn no_search_hit_full_highlights(fixture: &MinimalFixture) -> bool {
    no_full_name_search_highlight(fixture, "a.txt")
        && no_full_name_search_highlight(fixture, "b.txt")
        && no_full_name_search_highlight(fixture, "c.txt")
}

fn listing_shows_all_fixture_names(fixture: &MinimalFixture) -> bool {
    fixture
        .view
        .browser()
        .column_snapshot(0)
        .is_some_and(|column| column.count == 4)
        && !fixture.view.hidden_filter_active()
}

fn find_highlight_fixture(mode: BrowserMode) -> MinimalFixture {
    let fixture = MinimalFixture::new();
    if mode != BrowserMode::Columns {
        fixture.view.set_view_mode(mode);
        wait_until(|| rendered_name(&fixture.view.widget(), "a.txt"));
        fixture.select_name("a.txt");
        wait_until(|| fixture.view.item_view_has_focus());
    }
    fixture
}

fn highlighted_name_in(widget: &gtk::Widget, name: &str) -> Option<String> {
    use gtk::prelude::*;
    if !widget.is_mapped() || widget.width() <= 0 {
        return None;
    }
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && label.label() == name
        && let Some(slice) = crate::ui::browser::find_highlight::highlighted_slice(
            label.text().as_str(),
            label.attributes().as_ref(),
        )
    {
        return Some(slice);
    }
    if let Some(label) = widget.downcast_ref::<gtk::Inscription>()
        && label.text().as_deref() == Some(name)
        && let Some(slice) =
            crate::ui::browser::find_highlight::highlighted_slice(name, label.attributes().as_ref())
    {
        return Some(slice);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = highlighted_name_in(&widget, name) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
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

fn miller_column_header_focus(fixture: &MinimalFixture) -> bool {
    visible_widget_with_class(fixture.view.widget().upcast_ref(), "active-column").is_some()
}

fn visible_widget_with_class(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if widget.has_css_class(class) && widget.is_visible() {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = visible_widget_with_class(&widget, class) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn widget_with_tooltip(widget: &gtk::Widget, tooltip: &str) -> Option<gtk::Widget> {
    if widget.tooltip_text().as_deref() == Some(tooltip) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = widget_with_tooltip(&widget, tooltip) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn wait_until(condition: impl Fn() -> bool) {
    wait_until_msg(condition, "minimal fixture did not settle");
}

fn wait_until_msg(condition: impl Fn() -> bool, msg: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "{msg}");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn focused_widget(fixture: &MinimalFixture) -> Option<gtk::Widget> {
    gtk::prelude::RootExt::focus(&fixture.window)
}

fn sidebar_has_focus(fixture: &MinimalFixture) -> bool {
    focused_widget(fixture).is_some_and(|focus| {
        focus == fixture.sidebar.widget || focus.is_ancestor(&fixture.sidebar.widget)
    })
}

fn press_on(widget: &gtk::Widget, key: Key, modifiers: ModifierType) {
    let controllers = widget.observe_controllers();
    for index in 0..controllers.n_items() {
        let Some(controller) = controllers
            .item(index)
            .and_downcast::<gtk::EventControllerKey>()
        else {
            continue;
        };
        controller.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers]);
    }
}

fn modal_button(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
        if button.label().as_deref() == Some(label) {
            return Some(button);
        }
        if rendered_name(button.upcast_ref(), label) {
            return Some(button);
        }
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = modal_button(&widget, label) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn dismiss_error_dialog(fixture: &MinimalFixture) {
    if let Some(close) = modal_button(fixture.window.upcast_ref(), "Close") {
        close.emit_clicked();
    }
    if let Some(layer) = visible_widget_with_class(fixture.window.upcast_ref(), "app-modal-layer") {
        layer.set_visible(false);
    }
    pump_mainloop(Duration::from_millis(100));
    fixture.view.browser().focus_active();
}

fn pump_mainloop(for_duration: Duration) {
    let deadline = Instant::now() + for_duration;
    while Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn minimal_j_moves_without_filtering_when_type_to_search_is_on() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_j_moves_without_filtering_when_type_to_search_is_on",
        || {
            let fixture = MinimalFixture::new();
            PreferenceManager::shared().set_type_to_search(true);
            let start = fixture.cursor().expect("cursor on a.txt");
            assert!(fixture.press(Key::j, ModifierType::empty()));
            assert_eq!(fixture.cursor(), Some(start + 1));
            assert_eq!(fixture.fill(), vec![start + 1]);
            assert!(
                !fixture.view.hidden_filter_active(),
                "j must move, not filter"
            );
        },
    );
}

#[test]
fn minimal_ctrl_k_opens_search() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_ctrl_k_opens_search",
        || {
            let fixture = MinimalFixture::new();
            let searches = Rc::new(std::cell::Cell::new(0));
            let observed = searches.clone();
            let action = gtk::gio::SimpleAction::new("search", None);
            action.connect_activate(move |_, _| observed.set(observed.get() + 1));
            fixture.window.add_action(&action);
            assert!(fixture.press(Key::k, ModifierType::CONTROL_MASK));
            assert_eq!(searches.get(), 1);
            assert_eq!(fixture.view.view_mode(), BrowserMode::Columns);
        },
    );
}

#[test]
fn minimal_y_yanks_files_not_a_path() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_y_yanks_files_not_a_path",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::y, ModifierType::empty()));
            let formats = fixture.window.clipboard().formats();
            assert!(
                formats.contains_type(gtk::gdk::FileList::static_type()),
                "yank writes files to the clipboard"
            );
        },
    );
}

#[test]
fn minimal_unyank_keeps_a_foreign_clipboard() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_unyank_keeps_a_foreign_clipboard",
        || {
            if let Ok(text) = std::env::var("STRATA_SEED_CLIPBOARD") {
                let window = gtk::Window::new();
                window.present();
                window
                    .clipboard()
                    .set_content(Some(&gtk::gdk::ContentProvider::for_value(
                        &text.to_value(),
                    )))
                    .expect("seed foreign clipboard");
                glib::MainLoop::new(None, false).run();
                return;
            }
            let fixture = MinimalFixture::new();
            let mut seeder =
                std::process::Command::new(std::env::current_exe().expect("test executable"))
                    .args([
                        "--exact",
                        "ui::window::tests::minimal_mode::minimal_unyank_keeps_a_foreign_clipboard",
                        "--nocapture",
                    ])
                    .env(
                        "STRATA_ISOLATED_GTK_TEST",
                        "ui::window::tests::minimal_mode::minimal_unyank_keeps_a_foreign_clipboard",
                    )
                    .env("STRATA_SEED_CLIPBOARD", "foreign-payload")
                    .env("STRATA_REQUIRE_GTK_TESTS", "1")
                    .stdin(std::process::Stdio::null())
                    .spawn()
                    .expect("clipboard seeder");
            wait_until(|| {
                matches!(
                    glib::MainContext::default()
                        .block_on(fixture.window.clipboard().read_text_future()),
                    Ok(Some(ref text)) if text == "foreign-payload"
                ) && !fixture.window.clipboard().is_local()
            });
            let before = glib::MainContext::default()
                .block_on(fixture.window.clipboard().read_text_future())
                .expect("foreign read")
                .expect("foreign text");
            assert_eq!(before, "foreign-payload");
            assert!(fixture.press(Key::Y, ModifierType::empty()));
            let after = glib::MainContext::default()
                .block_on(fixture.window.clipboard().read_text_future())
                .expect("foreign reread")
                .expect("foreign text survived");
            assert_eq!(after, "foreign-payload");
            assert!(
                !fixture.window.clipboard().is_local(),
                "unyank must not claim a foreign clipboard"
            );
            let _ = seeder.kill();
            let _ = seeder.wait();
        },
    );
}

#[test]
fn minimal_unyank_clears_a_local_yank() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_unyank_clears_a_local_yank",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::y, ModifierType::empty()));
            let formats = fixture.window.clipboard().formats();
            assert!(
                formats.contains_type(gtk::gdk::FileList::static_type()),
                "yank writes files to the clipboard"
            );
            assert!(fixture.press(Key::Y, ModifierType::empty()));
            let formats = fixture.window.clipboard().formats();
            assert!(
                !formats.contains_type(gtk::gdk::FileList::static_type()),
                "local yank is cleared"
            );
        },
    );
}

#[test]
fn minimal_space_toggles_selection_not_preview() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_space_toggles_selection_not_preview",
        || {
            let fixture = MinimalFixture::new();
            fixture.view.browser().select(0, 0);
            fixture.view.browser().focus_active();
            let start = fixture.cursor().expect("cursor on the first row");
            assert_eq!(start, 0);
            assert_eq!(fixture.fill(), vec![start]);
            assert!(fixture.press(Key::space, ModifierType::empty()));
            assert_eq!(
                fixture.fill(),
                vec![start],
                "Space keeps a cursor-only item"
            );
            assert_eq!(fixture.cursor(), Some(start + 1), "Space advances one row");
            assert!(
                !fixture.preview.is_enabled(),
                "Space must not toggle preview"
            );
            assert!(fixture.press(Key::j, ModifierType::empty()));
            assert_eq!(fixture.cursor(), Some(start + 2));
            assert_eq!(
                fixture.fill(),
                vec![start],
                "Browse motion keeps an explicit fill"
            );
            assert!(fixture.press(Key::space, ModifierType::empty()));
            assert_eq!(fixture.fill(), vec![start, start + 2]);
            assert_eq!(fixture.cursor(), Some(start + 3));
        },
    );
}

#[test]
fn minimal_i_toggles_preview() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_i_toggles_preview",
        || {
            let fixture = MinimalFixture::new();
            wait_until(|| miller_column_header_focus(&fixture));
            assert!(fixture.press(Key::i, ModifierType::empty()));
            assert!(fixture.preview.is_enabled());
            assert!(
                !fixture.preview.owns_keys_chrome(),
                "i must toggle the drawer without taking preview keyboard ownership"
            );
            assert!(
                miller_column_header_focus(&fixture),
                "i must not strip miller header chrome"
            );
            assert!(fixture.press(Key::i, ModifierType::empty()));
            assert!(!fixture.preview.is_enabled());
            assert!(!fixture.preview.owns_keys_chrome());
        },
    );
}

#[test]
fn minimal_right_and_l_preview_a_file() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_right_and_l_preview_a_file",
        || {
            let fixture = MinimalFixture::new();
            fixture.select_name("a.txt");
            let cursor = fixture.cursor();
            let location = fixture.view.browser().active_location();
            let opened = Rc::new(RefCell::new(None::<Location>));
            let opened_for = opened.clone();
            fixture.view.browser().observe(move |event| {
                if let BrowserEvent::OpenRequested { location } = event {
                    opened_for.replace(Some(location.clone()));
                }
            });
            for key in [Key::Right, Key::l, Key::KP_Right] {
                assert!(fixture.press(key, ModifierType::empty()));
                wait_until(|| fixture.preview.is_enabled());
                wait_until(|| rendered_name(&fixture.preview.widget(), "a.txt"));
                wait_until(|| !fixture.view.item_view_has_focus());
                wait_until(|| fixture.preview.owns_keys_chrome());
                wait_until(|| !miller_column_header_focus(&fixture));
                assert!(
                    opened.borrow().is_none(),
                    "{key:?} must not open the focused file"
                );
                assert_eq!(
                    fixture.cursor(),
                    cursor,
                    "{key:?} must not move the listing cursor"
                );
                assert_eq!(
                    fixture.view.browser().active_location(),
                    location,
                    "{key:?} must not change the current location"
                );
                assert!(fixture.press(key, ModifierType::empty()));
                pump_mainloop(Duration::from_millis(80));
                assert!(
                    fixture.preview.is_enabled(),
                    "{key:?} must not close the preview"
                );
                assert!(opened.borrow().is_none());
                assert_eq!(fixture.cursor(), cursor);
                assert!(
                    !fixture.view.item_view_has_focus(),
                    "{key:?} must keep preview keyboard ownership"
                );
                assert!(
                    fixture.preview.owns_keys_chrome(),
                    "{key:?} must keep the preview focus border"
                );
                assert!(
                    !miller_column_header_focus(&fixture),
                    "{key:?} must keep miller header chrome off"
                );
            }
            assert!(opened.borrow().is_none());
        },
    );
}

fn preview_scroll_adjustment(fixture: &MinimalFixture) -> gtk::Adjustment {
    wait_until_msg(
        || {
            fixture
                .preview
                .mapped_preview_scroller()
                .is_some_and(|scroll| {
                    let adjustment = scroll.vadjustment();
                    adjustment.page_size() > 0.0
                        && adjustment.upper() > adjustment.page_size() + 20.0
                })
        },
        "preview document did not become scrollable",
    );
    fixture
        .preview
        .mapped_preview_scroller()
        .expect("mapped preview scroller")
        .vadjustment()
}

#[test]
fn minimal_preview_folder_motion_scrolls_then_h_returns() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_preview_folder_motion_scrolls_then_h_returns",
        || {
            let fixture = MinimalFixture::new();
            fixture.select_name("a.txt");
            let cursor = fixture.cursor();
            let location = fixture.view.browser().active_location();
            assert!(fixture.press(Key::l, ModifierType::empty()));
            wait_until(|| fixture.preview.is_enabled());
            wait_until(|| rendered_name(&fixture.preview.widget(), "a.txt"));
            wait_until(|| !fixture.view.item_view_has_focus());
            wait_until(|| fixture.preview.owns_keys_chrome());
            wait_until(|| !miller_column_header_focus(&fixture));
            let adjustment = preview_scroll_adjustment(&fixture);
            let start = adjustment.value();

            assert!(fixture.press(Key::j, ModifierType::empty()));
            wait_until(|| adjustment.value() > start);
            assert_eq!(
                fixture.cursor(),
                cursor,
                "j must not move the listing cursor"
            );
            assert_eq!(fixture.view.browser().active_location(), location);
            assert!(!fixture.view.item_view_has_focus());

            let after_j = adjustment.value();
            assert!(fixture.press(Key::k, ModifierType::empty()));
            wait_until(|| adjustment.value() < after_j);
            assert_eq!(
                fixture.cursor(),
                cursor,
                "k must not move the listing cursor"
            );

            let before_page = adjustment.value();
            assert!(fixture.press(Key::Page_Down, ModifierType::empty()));
            wait_until(|| adjustment.value() > before_page);
            assert_eq!(
                fixture.cursor(),
                cursor,
                "Page Down must not move the listing cursor"
            );
            assert_eq!(fixture.view.browser().active_location(), location);
            assert!(fixture.preview.is_enabled());

            assert!(fixture.press(Key::h, ModifierType::empty()));
            wait_until(|| fixture.view.item_view_has_focus());
            assert!(
                fixture.preview.is_enabled(),
                "h must leave the preview open"
            );
            assert!(
                !fixture.preview.owns_keys_chrome(),
                "h must remove the preview focus border"
            );
            wait_until(|| miller_column_header_focus(&fixture));
            assert_eq!(fixture.cursor(), cursor);
            assert_eq!(
                fixture.view.browser().active_location(),
                location,
                "h from the preview must not navigate to the parent"
            );

            assert!(fixture.press(Key::h, ModifierType::empty()));
            wait_until(|| fixture.view.browser().active_location() != location);
        },
    );
}

#[test]
fn minimal_right_and_l_open_a_directory() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_right_and_l_open_a_directory",
        || {
            let fixture = MinimalFixture::new();
            let opened = Rc::new(RefCell::new(None::<Location>));
            let opened_for = opened.clone();
            fixture.view.browser().observe(move |event| {
                if let BrowserEvent::OpenRequested { location } = event {
                    opened_for.replace(Some(location.clone()));
                }
            });
            for key in [Key::Right, Key::l] {
                fixture.select_name("sub");
                assert!(fixture.press(key, ModifierType::empty()));
                wait_until(|| fixture.view.browser().column_snapshot(1).is_some());
                assert_eq!(
                    fixture.view.browser().active_location(),
                    Some(Location::local(fixture._directory.path().join("sub")))
                );
                assert!(
                    opened.borrow().is_none(),
                    "{key:?} must enter a directory instead of opening it as a file"
                );
                assert!(!fixture.preview.is_enabled());
                assert!(fixture.press(Key::h, ModifierType::empty()));
                wait_until(|| fixture.view.browser().column_snapshot(1).is_none());
            }
        },
    );
}

#[test]
fn minimal_right_and_l_skip_unpreviewable_files() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_right_and_l_skip_unpreviewable_files",
        || {
            let fixture = MinimalFixture::new();
            fixture.footer.observe_browser(&fixture.view);
            let zip = fixture._directory.path().join("archive.zip");
            std::fs::write(&zip, b"PK").expect("zip");
            fixture
                .view
                .browser()
                .navigate(Location::local(fixture._directory.path()));
            wait_until(|| rendered_name(&fixture.view.widget(), "archive.zip"));
            fixture.select_name("archive.zip");
            let opened = Rc::new(RefCell::new(None::<Location>));
            let opened_for = opened.clone();
            fixture.view.browser().observe(move |event| {
                if let BrowserEvent::OpenRequested { location } = event {
                    opened_for.replace(Some(location.clone()));
                }
            });
            for key in [Key::Right, Key::l] {
                assert!(fixture.press(key, ModifierType::empty()));
                wait_until(|| footer_shows(&fixture, "Nothing to preview"));
                pump_mainloop(Duration::from_millis(80));
                assert!(
                    !fixture.preview.is_enabled(),
                    "{key:?} must not open a placeholder drawer for an unpreviewable file"
                );
                assert!(
                    opened.borrow().is_none(),
                    "{key:?} must not open an unpreviewable file"
                );
            }
            assert_eq!(
                fixture.view.browser().active_location(),
                Some(Location::local(fixture._directory.path()))
            );
        },
    );
}

#[test]
fn minimal_right_and_l_flash_when_the_folder_is_empty() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_right_and_l_flash_when_the_folder_is_empty",
        || {
            let fixture = MinimalFixture::new();
            fixture.footer.observe_browser(&fixture.view);
            open_empty_sub(&fixture);
            assert!(fixture.view.browser().focused_entry().is_none());
            for key in [Key::Right, Key::l] {
                assert!(fixture.press(key, ModifierType::empty()));
                wait_until(|| footer_shows(&fixture, "Nothing to preview"));
                assert!(!fixture.preview.is_enabled());
            }
        },
    );
}

#[test]
fn minimal_q_leaves_and_big_q_closes() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_q_leaves_and_big_q_closes",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::q, ModifierType::empty()));
            assert!(!PreferenceManager::shared().minimal_mode());
            wait_until(|| footer_shows(&fixture, "Left minimal mode — Ctrl+Shift+M returns"));
            PreferenceManager::shared().set_minimal_mode(true);
            assert!(fixture.press(Key::Q, ModifierType::empty()));
        },
    );
}

#[test]
fn minimal_ctrl_shift_m_toggles_both_ways() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_ctrl_shift_m_toggles_both_ways",
        || {
            let fixture = MinimalFixture::new();
            let toggle = ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK;
            assert!(fixture.press(Key::M, toggle));
            assert!(!PreferenceManager::shared().minimal_mode());
            assert!(fixture.press(Key::M, toggle));
            assert!(PreferenceManager::shared().minimal_mode());
        },
    );
}

#[test]
fn minimal_gg_jumps_to_the_first_item() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_gg_jumps_to_the_first_item",
        || {
            let fixture = MinimalFixture::new();
            fixture.view.browser().select(0, 2);
            fixture.view.browser().focus_active();
            assert_eq!(fixture.cursor(), Some(2));
            assert!(fixture.press(Key::g, ModifierType::empty()));
            assert!(fixture.press(Key::g, ModifierType::empty()));
            assert_eq!(fixture.cursor(), Some(0));
        },
    );
}

#[test]
fn minimal_gh_goes_home_with_type_to_search_off() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_gh_goes_home_with_type_to_search_off",
        || {
            let fixture = MinimalFixture::new();
            PreferenceManager::shared().set_type_to_search(false);
            assert!(fixture.press(Key::g, ModifierType::empty()));
            assert!(fixture.press(Key::h, ModifierType::empty()));
            assert_eq!(
                fixture.view.browser().active_location(),
                Some(Location::local(home_directory()))
            );
        },
    );
}

#[test]
fn minimal_gh_stays_armed_until_the_second_key() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_gh_stays_armed_until_the_second_key",
        || {
            let fixture = MinimalFixture::new();
            fixture.select_name("sub");
            assert!(fixture.press(Key::l, ModifierType::empty()));
            wait_until(|| fixture.view.browser().column_snapshot(1).is_some());
            assert!(fixture.press(Key::g, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "g-"));
            pump_mainloop(Duration::from_millis(1000));
            assert!(
                footer_shows(&fixture, "g-"),
                "g- stays until Esc or a second key"
            );
            assert!(fixture.press(Key::h, ModifierType::empty()));
            assert_eq!(
                fixture.view.browser().active_location(),
                Some(Location::local(home_directory()))
            );
        },
    );
}

#[test]
fn minimal_h_closes_the_miller_child() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_h_closes_the_miller_child",
        || {
            let fixture = MinimalFixture::new();
            fixture.select_name("sub");
            assert!(fixture.press(Key::l, ModifierType::empty()));
            wait_until(|| fixture.view.browser().column_snapshot(1).is_some());
            assert!(fixture.press(Key::h, ModifierType::empty()));
            wait_until(|| fixture.view.browser().column_snapshot(1).is_none());
            assert_eq!(
                fixture.view.browser().active_location(),
                Some(Location::local(fixture._directory.path()))
            );
        },
    );
}

#[test]
fn minimal_icons_h_lands_on_the_folder_you_left() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_icons_h_lands_on_the_folder_you_left",
        || {
            let fixture = MinimalFixture::new();
            fixture.footer.observe_browser(&fixture.view);
            fixture.view.set_view_mode(BrowserMode::Icons);
            wait_until(|| rendered_name(&fixture.view.widget(), "sub"));
            fixture.select_name("sub");
            assert_eq!(fixture.cursor_name().as_deref(), Some("sub"));
            assert!(fixture.press(Key::l, ModifierType::empty()));
            let child = Location::local(fixture._directory.path().join("sub"));
            wait_until(|| {
                fixture.view.browser().active_location() == Some(child.clone())
                    && fixture
                        .view
                        .browser()
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading)
            });
            assert!(fixture.press(Key::h, ModifierType::empty()));
            wait_until(|| {
                fixture.view.browser().active_location()
                    == Some(Location::local(fixture._directory.path()))
                    && fixture.cursor_name().as_deref() == Some("sub")
            });
        },
    );
}

#[test]
fn minimal_visual_select_ranges_and_second_v_keeps_the_fill() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_visual_select_ranges_and_second_v_keeps_the_fill",
        || {
            let fixture = MinimalFixture::new();
            let start = fixture.cursor().expect("cursor on a.txt");
            assert!(fixture.press(Key::v, ModifierType::empty()));
            assert!(fixture.press(Key::j, ModifierType::empty()));
            assert_eq!(fixture.fill(), vec![start, start + 1]);
            assert!(fixture.press(Key::v, ModifierType::empty()));
            assert_eq!(
                fixture.fill(),
                vec![start, start + 1],
                "leaving visual keeps the fill"
            );
            assert!(fixture.press(Key::j, ModifierType::empty()));
            assert_eq!(fixture.cursor(), Some(start + 2));
            assert_eq!(
                fixture.fill(),
                vec![start, start + 1],
                "Browse motion keeps the visual fill"
            );
        },
    );
}

#[test]
fn minimal_visual_unset_subtracts_the_walked_span() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_visual_unset_subtracts_the_walked_span",
        || {
            let fixture = MinimalFixture::new();
            let total = fixture
                .view
                .browser()
                .column_snapshot(0)
                .expect("loaded column")
                .count;
            assert!(fixture.press(Key::a, ModifierType::CONTROL_MASK));
            assert_eq!(fixture.fill(), (0..total).collect::<Vec<_>>());
            let last = total - 1;
            assert_eq!(fixture.cursor(), Some(last));
            assert!(fixture.press(Key::V, ModifierType::empty()));
            assert!(fixture.press(Key::k, ModifierType::empty()));
            assert_eq!(fixture.fill(), (0..last - 1).collect::<Vec<_>>());
            assert_eq!(fixture.cursor(), Some(last - 1));
        },
    );
}

#[test]
fn minimal_ctrl_a_survives_browse_motion() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_ctrl_a_survives_browse_motion",
        || {
            let fixture = MinimalFixture::new();
            let total = fixture
                .view
                .browser()
                .column_snapshot(0)
                .expect("loaded column")
                .count;
            assert!(fixture.press(Key::a, ModifierType::CONTROL_MASK));
            assert_eq!(fixture.fill(), (0..total).collect::<Vec<_>>());
            let cursor = fixture.cursor().expect("cursor after select all");
            assert!(fixture.press(Key::k, ModifierType::empty()));
            assert_eq!(
                fixture.fill(),
                (0..total).collect::<Vec<_>>(),
                "Browse motion keeps Select All"
            );
            assert_ne!(fixture.cursor(), Some(cursor));
        },
    );
}

#[test]
fn minimal_ctrl_r_inverts_the_selection() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_ctrl_r_inverts_the_selection",
        || {
            let fixture = MinimalFixture::new();
            let start = fixture.cursor().expect("cursor on a.txt");
            assert_eq!(fixture.fill(), vec![start]);
            assert!(fixture.press(Key::r, ModifierType::CONTROL_MASK));
            let total = fixture
                .view
                .browser()
                .column_snapshot(0)
                .expect("loaded column")
                .count;
            let expected: Vec<usize> = (0..total).filter(|index| *index != start).collect();
            assert_eq!(fixture.fill(), expected);
            assert_eq!(fixture.cursor(), Some(start));
            assert!(fixture.press(Key::j, ModifierType::empty()));
            assert_eq!(
                fixture.fill(),
                expected,
                "Browse motion keeps an inverted fill"
            );
            assert_eq!(fixture.cursor(), Some(start + 1));
        },
    );
}

#[test]
fn minimal_delete_and_f2_stay_bound() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_delete_and_f2_stay_bound",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::F2, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "rename"));
            assert!(!fixture.view.rename_is_active());
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until(|| !footer_shows(&fixture, "rename"));
            assert!(fixture.press(Key::Delete, ModifierType::empty()));
            wait_until(|| {
                visible_widget_with_class(fixture.window.upcast_ref(), "app-modal-layer").is_some()
            });
            modal_button(fixture.window.upcast_ref(), "Cancel")
                .expect("trash confirmation")
                .emit_clicked();
            wait_until(|| {
                visible_widget_with_class(fixture.window.upcast_ref(), "app-modal-layer").is_none()
            });
        },
    );
}

#[test]
fn minimal_hidden_sort_and_copy_path() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_hidden_sort_and_copy_path",
        || {
            let fixture = MinimalFixture::new();
            let hidden_before = fixture.view.browser().preferences().show_hidden;
            assert!(fixture.press(Key::period, ModifierType::empty()));
            wait_until(|| fixture.view.browser().preferences().show_hidden != hidden_before);

            fixture.select_name("a.txt");
            assert!(fixture.press(Key::comma, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, ",-"));
            assert!(fixture.press(Key::s, ModifierType::empty()));
            wait_until(|| {
                fixture
                    .view
                    .browser()
                    .column_preferences(0)
                    .is_some_and(|prefs| prefs.sort_key == crate::model::SortKey::Size)
            });

            fixture.select_name("a.txt");
            assert!(fixture.press(Key::c, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "c-"));
            assert!(fixture.press(Key::c, ModifierType::empty()));
            wait_until(|| {
                glib::MainContext::default()
                    .block_on(fixture.window.clipboard().read_text_future())
                    .ok()
                    .flatten()
                    .is_some_and(|text| text.contains("a.txt"))
            });
            assert!(
                !fixture
                    .window
                    .clipboard()
                    .formats()
                    .contains_type(gtk::gdk::FileList::static_type()),
                "c c copies a path, not files"
            );

            assert!(fixture.press(Key::c, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "c-"));
            assert!(fixture.press(Key::n, ModifierType::empty()));
            wait_until(|| {
                glib::MainContext::default()
                    .block_on(fixture.window.clipboard().read_text_future())
                    .ok()
                    .flatten()
                    .as_deref()
                    == Some("a.txt")
            });

            assert!(fixture.press(Key::_2, ModifierType::CONTROL_MASK));
            assert_eq!(fixture.view.view_mode(), BrowserMode::Icons);
        },
    );
}

#[test]
fn minimal_d_opens_the_trash_confirmation() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_d_opens_the_trash_confirmation",
        || {
            let fixture = MinimalFixture::new();
            let path = fixture._directory.path().join("a.txt");
            assert!(fixture.press(Key::d, ModifierType::empty()));
            wait_until(|| {
                visible_widget_with_class(fixture.window.upcast_ref(), "app-modal-layer").is_some()
            });
            assert!(path.exists());
            pump_mainloop(Duration::from_millis(500));
            modal_button(fixture.window.upcast_ref(), "Cancel")
                .expect("cancel")
                .emit_clicked();
            wait_until(|| {
                visible_widget_with_class(fixture.window.upcast_ref(), "app-modal-layer").is_none()
            });
            assert!(path.exists());
        },
    );
}

#[test]
fn minimal_d_trash_confirmation_moves_the_file() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_d_trash_confirmation_moves_the_file",
        || {
            let fixture = MinimalFixture::new();
            let path = fixture._directory.path().join("a.txt");
            assert!(fixture.press(Key::d, ModifierType::empty()));
            wait_until(|| {
                visible_widget_with_class(fixture.window.upcast_ref(), "app-modal-layer").is_some()
            });
            pump_mainloop(Duration::from_millis(500));
            modal_button(fixture.window.upcast_ref(), "Move to Trash")
                .expect("confirm trash")
                .emit_clicked();
            wait_until(|| !path.exists());
        },
    );
}

#[test]
fn minimal_big_d_opens_the_permanent_confirmation() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_big_d_opens_the_permanent_confirmation",
        || {
            let fixture = MinimalFixture::new();
            let path = fixture._directory.path().join("a.txt");
            let window = fixture.window.upcast_ref::<gtk::Widget>();
            assert!(fixture.press(Key::D, ModifierType::empty()));
            wait_until(|| modal_button(window, "Cancel").is_some_and(|button| button.has_focus()));
            assert!(
                !modal_button(window, "Permanently delete 1 item")
                    .is_some_and(|button| button.has_focus()),
                "D must not focus the destructive button"
            );
            let layer = visible_widget_with_class(window, "app-modal-layer").expect("dialog");
            press_on(&layer, Key::Return, ModifierType::empty());
            wait_until(|| visible_widget_with_class(window, "app-modal-layer").is_none());
            assert!(path.exists());
        },
    );
}

#[test]
fn minimal_tilde_toggles_the_reference() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_tilde_toggles_the_reference",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::asciitilde, ModifierType::empty()));
            wait_until(|| {
                visible_widget_with_class(fixture.window.upcast_ref(), "shortcut-popover").is_some()
            });
            let popover =
                visible_widget_with_class(fixture.window.upcast_ref(), "shortcut-popover")
                    .expect("reference");
            assert!(
                rendered_name(&popover, "Move to Trash with confirmation"),
                "d confirms before trashing"
            );
            assert!(rendered_name(
                &popover,
                "Delete permanently with confirmation"
            ));
            assert!(fixture.press(Key::asciitilde, ModifierType::empty()));
            wait_until(|| {
                visible_widget_with_class(fixture.window.upcast_ref(), "shortcut-popover").is_none()
            });
        },
    );
}

#[test]
fn minimal_default_tilde_still_types_to_search() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_default_tilde_still_types_to_search",
        || {
            let fixture = MinimalFixture::new();
            PreferenceManager::shared().set_minimal_mode(false);
            PreferenceManager::shared().set_type_to_search(true);
            assert!(fixture.press(Key::asciitilde, ModifierType::empty()));
            assert!(
                fixture.view.filter_has_focus(),
                "default-map ~ stays type-to-search"
            );
        },
    );
}

#[test]
fn minimal_yank_paste_copies_into_the_subdirectory() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_yank_paste_copies_into_the_subdirectory",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::y, ModifierType::empty()));
            fixture.select_name("sub");
            assert!(fixture.press(Key::l, ModifierType::empty()));
            wait_until(|| fixture.view.browser().column_snapshot(1).is_some());
            assert!(fixture.press(Key::p, ModifierType::empty()));
            wait_until(|| fixture._directory.path().join("sub").join("a.txt").exists());
        },
    );
}

#[test]
fn minimal_p_flashes_when_clipboard_is_empty() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_p_flashes_when_clipboard_is_empty",
        || {
            let fixture = MinimalFixture::new();
            let clipboard = fixture.window.clipboard();
            let _ = clipboard.set_content(None::<&gtk::gdk::ContentProvider>);
            wait_until(|| {
                let formats = clipboard.formats();
                !formats.contains_type(gtk::gdk::FileList::static_type())
                    && !formats.contain_mime_type("text/uri-list")
            });
            let root = fixture._directory.path().to_path_buf();
            let before = std::fs::read_dir(&root)
                .expect("root")
                .filter_map(Result::ok)
                .count();
            assert!(fixture.press(Key::p, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "Nothing to paste"));
            let after = std::fs::read_dir(&root)
                .expect("root")
                .filter_map(Result::ok)
                .count();
            assert_eq!(before, after, "empty clipboard p must not create files");
        },
    );
}

#[test]
fn minimal_empty_folder_verbs_flash() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_empty_folder_verbs_flash",
        || {
            let fixture = MinimalFixture::new();
            let clipboard = fixture.window.clipboard();
            let _ = clipboard.set_content(None::<&gtk::gdk::ContentProvider>);
            wait_until(|| {
                !clipboard
                    .formats()
                    .contains_type(gtk::gdk::FileList::static_type())
            });
            open_empty_sub(&fixture);
            assert!(fixture.view.browser().focused_entry().is_none());

            assert!(fixture.press(Key::y, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "Nothing to yank"));
            assert!(
                !fixture
                    .window
                    .clipboard()
                    .formats()
                    .contains_type(gtk::gdk::FileList::static_type()),
                "empty-folder yank must not write the clipboard"
            );

            assert!(fixture.press(Key::x, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "Nothing to cut"));

            assert!(fixture.press(Key::d, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "Nothing to delete"));
            assert!(
                visible_widget_with_class(fixture.window.upcast_ref(), "app-modal-layer").is_none(),
                "empty-folder d must not open a trash dialog"
            );

            assert!(fixture.press(Key::r, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "Nothing to rename"));
            assert_eq!(prompt_entry_text(&fixture), "");

            assert!(fixture.press(Key::i, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "Nothing to preview"));
            assert!(!fixture.preview.is_enabled());

            assert!(fixture.press(Key::space, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "Nothing to select"));
            assert!(fixture.fill().is_empty());
            assert!(fixture.cursor().is_none());
        },
    );
}

#[test]
fn minimal_p_pastes_into_the_listing_not_a_hovered_folder() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_p_pastes_into_the_listing_not_a_hovered_folder",
        || {
            let fixture = MinimalFixture::new();
            let root = fixture._directory.path().to_path_buf();
            std::fs::write(root.join("sub").join("nested.txt"), b"nested").expect("nested file");
            fixture.select_name("sub");
            assert!(fixture.press(Key::l, ModifierType::empty()));
            wait_until(|| {
                fixture
                    .view
                    .browser()
                    .column_snapshot(1)
                    .is_some_and(|column| !column.loading && column.count >= 1)
            });
            assert!(fixture.press(Key::y, ModifierType::empty()));
            assert!(fixture.press(Key::h, ModifierType::empty()));
            wait_until(|| fixture.view.browser().column_snapshot(1).is_none());
            let sub = fixture.index_of("sub");
            assert_eq!(fixture.cursor(), Some(sub));
            assert_eq!(fixture.fill(), vec![sub]);
            assert!(fixture.press(Key::p, ModifierType::empty()));
            wait_until(|| root.join("nested.txt").exists());

            fixture.select_name("a.txt");
            assert!(fixture.press(Key::y, ModifierType::empty()));
            fixture.select_name("sub");
            let sub = fixture.index_of("sub");
            assert_eq!(fixture.fill(), vec![sub]);
            assert!(fixture.press(Key::space, ModifierType::empty()));
            assert_eq!(fixture.fill(), vec![sub]);
            assert!(fixture.press(Key::p, ModifierType::empty()));
            wait_until(|| root.join("sub").join("a.txt").exists());
        },
    );
}

#[test]
fn minimal_p_conflict_focuses_keep_both_p_focuses_replace() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_p_conflict_focuses_keep_both_p_focuses_replace",
        || {
            let fixture = MinimalFixture::new();
            let root = fixture._directory.path().to_path_buf();
            std::fs::write(root.join("sub").join("a.txt"), b"old").expect("collision");
            assert!(fixture.press(Key::y, ModifierType::empty()));
            let clipboard = gtk::gdk::Display::default()
                .expect("default display")
                .clipboard();
            wait_until(|| {
                clipboard
                    .formats()
                    .contains_type(gtk::gdk::FileList::static_type())
            });
            fixture.select_name("sub");
            assert!(fixture.press(Key::l, ModifierType::empty()));
            wait_until(|| {
                fixture
                    .view
                    .browser()
                    .column_snapshot(1)
                    .is_some_and(|column| !column.loading && column.count >= 1)
            });
            let window = fixture.window.upcast_ref::<gtk::Widget>();
            assert!(fixture.press(Key::p, ModifierType::empty()));
            wait_until(|| {
                modal_button(window, "Keep Both").is_some_and(|button| button.has_focus())
            });
            assert!(
                !modal_button(window, "Replace").is_some_and(|button| button.has_focus()),
                "p must not focus Replace"
            );
            modal_button(window, "Cancel")
                .expect("Cancel")
                .emit_clicked();
            wait_until(|| modal_button(window, "Replace").is_none());

            assert!(fixture.press(Key::P, ModifierType::empty()));
            wait_until(|| modal_button(window, "Replace").is_some_and(|button| button.has_focus()));
            assert!(
                !modal_button(window, "Keep Both").is_some_and(|button| button.has_focus()),
                "P must not focus Keep Both"
            );
            modal_button(window, "Cancel")
                .expect("Cancel")
                .emit_clicked();
        },
    );
}

#[test]
fn minimal_icons_rebuild_keeps_chrome_hidden() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_icons_rebuild_keeps_chrome_hidden",
        || {
            let fixture = MinimalFixture::new();
            fixture.view.set_view_mode(BrowserMode::Icons);
            wait_until(|| rendered_name(&fixture.view.widget(), "a.txt"));
            let refresh =
                widget_with_tooltip(&fixture.view.widget(), "Refresh (F5)").expect("icons refresh");
            assert!(!refresh.is_visible());
            let filter = widget_with_tooltip(&fixture.view.widget(), "Filter icons (Ctrl+F)")
                .expect("icons filter");
            assert!(!filter.is_visible());
            let thumbnails = widget_with_tooltip(&fixture.view.widget(), "Thumbnail size")
                .expect("thumbnail menu stays");
            assert!(thumbnails.is_visible());
        },
    );
}

#[test]
fn minimal_columns_chrome_hides_and_returns_with_the_mode() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_columns_chrome_hides_and_returns_with_the_mode",
        || {
            let fixture = MinimalFixture::new();
            let filter = widget_with_tooltip(&fixture.view.widget(), "Filter this pane (Ctrl+F)")
                .expect("pane filter");
            assert!(!filter.is_visible());
            let refresh =
                widget_with_tooltip(&fixture.view.widget(), "Refresh (F5)").expect("pane refresh");
            assert!(!refresh.is_visible());
            let sort = widget_with_tooltip(&fixture.view.widget(), "Choose sort field")
                .expect("pane sort menu");
            assert!(!sort.is_visible());
            fixture.select_name("sub");
            assert!(fixture.press(Key::l, ModifierType::empty()));
            wait_until(|| fixture.view.browser().column_snapshot(1).is_some());
            let close =
                widget_with_tooltip(&fixture.view.widget(), "Close this pane").expect("pane close");
            assert!(!close.is_visible());
            assert!(fixture.press(Key::q, ModifierType::empty()));
            for widget in [filter, refresh, sort, close] {
                assert!(widget.is_visible(), "chrome returns when leaving");
            }
        },
    );
}

#[test]
fn minimal_sidebar_hjkl_move_places_not_the_file_cursor() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_sidebar_hjkl_move_places_not_the_file_cursor",
        || {
            let fixture = MinimalFixture::new();
            let listing = Location::local(fixture._directory.path());
            fixture.sidebar.state.pin_location(
                Location::local(fixture._directory.path().join("sub")),
                "sub".into(),
            );
            wait_until(|| rendered_name(&fixture.sidebar.widget, "sub"));
            let sub = fixture.select_name("sub");

            assert!(fixture.press(
                Key::b,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK
            ));
            wait_until(|| sidebar_has_focus(&fixture));
            let first = focused_widget(&fixture).expect("sidebar place");
            assert_eq!(fixture.cursor(), Some(sub));

            assert!(fixture.press(Key::j, ModifierType::empty()));
            wait_until(|| {
                sidebar_has_focus(&fixture)
                    && focused_widget(&fixture).is_some_and(|focus| focus != first)
            });
            assert_eq!(fixture.cursor(), Some(sub));
            assert_eq!(
                fixture.view.browser().active_location(),
                Some(listing.clone())
            );

            assert!(fixture.press(Key::k, ModifierType::empty()));
            wait_until(|| sidebar_has_focus(&fixture));
            assert_eq!(fixture.cursor(), Some(sub));

            assert!(fixture.press(Key::h, ModifierType::empty()));
            wait_until(|| fixture.view.item_view_has_focus());
            assert_eq!(fixture.cursor(), Some(sub));

            assert!(fixture.press(
                Key::b,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK
            ));
            wait_until(|| sidebar_has_focus(&fixture));
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| {
                fixture.view.browser().active_location() == Some(Location::local(home_directory()))
            });
            assert!(
                fixture.view.browser().column_snapshot(1).is_none(),
                "Enter must activate the sidebar place, not the listing cursor"
            );
        },
    );
}

#[test]
fn minimal_g1_navigates_to_the_first_visible_pin() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_g1_navigates_to_the_first_visible_pin",
        || {
            let fixture = MinimalFixture::new();
            let target = fixture._directory.path().join("sub");
            fixture
                .sidebar
                .state
                .pin_location(Location::local(target.clone()), "sub".into());
            assert!(fixture.press(Key::g, ModifierType::empty()));
            assert!(fixture.press(Key::_1, ModifierType::empty()));
            assert_eq!(
                fixture.view.browser().active_location(),
                Some(Location::local(target))
            );
        },
    );
}

#[test]
fn minimal_enabling_dismisses_an_in_flight_filter() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_enabling_dismisses_an_in_flight_filter",
        || {
            let fixture = MinimalFixture::new();
            PreferenceManager::shared().set_minimal_mode(false);
            PreferenceManager::shared().set_type_to_search(true);
            assert!(fixture.view.show_filter_with_query("a"));
            assert!(fixture.view.filter_has_focus());
            PreferenceManager::shared().set_minimal_mode(true);
            assert!(!fixture.view.hidden_filter_active());
            assert!(!fixture.view.filter_has_focus());
            assert!(fixture.view.item_view_has_focus());
        },
    );
}

#[test]
fn minimal_reenable_cancels_a_pending_chord() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_reenable_cancels_a_pending_chord",
        || {
            let fixture = MinimalFixture::new();
            let home = fixture.view.browser().active_location();
            assert!(fixture.press(Key::g, ModifierType::empty()));
            PreferenceManager::shared().set_minimal_mode(false);
            PreferenceManager::shared().set_minimal_mode(true);
            assert!(fixture.press(Key::h, ModifierType::empty()));
            assert_ne!(
                fixture.view.browser().active_location(),
                Some(Location::local(home_directory())),
                "a stale chord must not navigate Home"
            );
            assert_eq!(
                fixture.view.browser().active_location().is_some(),
                home.is_some()
            );
        },
    );
}

#[test]
fn minimal_disabling_leaves_visual_keeping_the_fill() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_disabling_leaves_visual_keeping_the_fill",
        || {
            let fixture = MinimalFixture::new();
            let start = fixture.cursor().expect("cursor on a.txt");
            assert!(fixture.press(Key::v, ModifierType::empty()));
            assert!(fixture.press(Key::j, ModifierType::empty()));
            assert_eq!(fixture.fill(), vec![start, start + 1]);
            PreferenceManager::shared().set_minimal_mode(false);
            PreferenceManager::shared().set_minimal_mode(true);
            assert!(fixture.press(Key::j, ModifierType::empty()));
            assert_eq!(fixture.cursor(), Some(start + 2));
            assert_eq!(
                fixture.fill(),
                vec![start, start + 1],
                "re-enabled motion keeps the explicit fill"
            );
        },
    );
}

#[test]
fn minimal_icons_j_moves_in_listing_order() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_icons_j_moves_in_listing_order",
        || {
            let fixture = MinimalFixture::new();
            fixture.view.set_view_mode(BrowserMode::Icons);
            wait_until(|| rendered_name(&fixture.view.widget(), "a.txt"));
            // The rebuild settles native selection first; ownership sync
            // adopts it, so read the cursor after the switch.
            pump_mainloop(Duration::from_millis(100));
            let start = fixture.cursor().expect("cursor after the rebuild");
            assert!(fixture.press(Key::j, ModifierType::empty()));
            assert_eq!(fixture.cursor(), Some(start + 1));
            assert_eq!(fixture.fill(), vec![start + 1]);
            assert!(!fixture.view.hidden_filter_active());
        },
    );
}

fn prompt_entry_text(fixture: &MinimalFixture) -> String {
    fixture.footer.prompt_text()
}

fn listing_has_name(fixture: &MinimalFixture, name: &str) -> bool {
    let Some(count) = fixture
        .view
        .browser()
        .column_snapshot(0)
        .map(|column| column.count)
    else {
        return false;
    };
    (0..count).any(|index| {
        fixture
            .view
            .browser()
            .entry_at(0, index)
            .is_some_and(|entry| entry.display_name == name)
    })
}

fn prompt_has_focus(fixture: &MinimalFixture) -> bool {
    fixture
        .footer
        .is_prompt_entry(&gtk::prelude::RootExt::focus(&fixture.window))
}

fn listing_cursor_has_focus(fixture: &MinimalFixture) -> bool {
    use gtk::prelude::*;
    let Some(focused) = focused_widget(fixture) else {
        return false;
    };
    if prompt_has_focus(fixture) || focused.is::<gtk::Stack>() {
        return false;
    }
    fixture.view.item_view_has_focus()
        || focused.is::<gtk::ListBoxRow>()
        || focused.ancestor(gtk::ListBox::static_type()).is_some()
        || focused.is::<gtk::ListView>()
        || focused.is::<gtk::GridView>()
        || focused.ancestor(gtk::ListView::static_type()).is_some()
        || focused.ancestor(gtk::GridView::static_type()).is_some()
}

fn type_find_char(fixture: &MinimalFixture, text: &str) {
    use gtk::prelude::*;
    let entry = fixture.footer.prompt_entry_widget();
    assert!(
        prompt_has_focus(fixture),
        "prompt must own the keyboard before {text:?}"
    );
    let mut position = entry.position();
    entry.insert_text(text, &mut position);
    entry.set_position(position);
    pump_mainloop(Duration::from_millis(30));
    assert!(
        prompt_has_focus(fixture),
        "find must not move focus to the listing after {text:?}"
    );
}

fn footer_shows(fixture: &MinimalFixture, text: &str) -> bool {
    use gtk::prelude::*;
    rendered_name(fixture.footer.widget().upcast_ref(), text)
}

/// Footer labels stay on GtkStack pages; read them without waiting for a
/// stack transition to finish mapping the active page.
fn footer_has_label(fixture: &MinimalFixture, text: &str) -> bool {
    fn walk(widget: &gtk::Widget, text: &str) -> bool {
        use gtk::prelude::*;
        if widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.label() == text)
        {
            return true;
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            if walk(&next, text) {
                return true;
            }
            child = next.next_sibling();
        }
        false
    }
    walk(fixture.footer.widget().upcast_ref(), text)
}

fn filter_kept(fixture: &MinimalFixture, query: &str) -> bool {
    let mark = format!("filter: {query}");
    fixture.view.hidden_filter_active()
        && fixture.view.hidden_filter_query().trim() == query
        && footer_has_label(fixture, &mark)
        && rendered_name(&fixture.view.widget(), query)
        && !rendered_name(&fixture.view.widget(), "c.txt")
}

fn filter_kept_state(fixture: &MinimalFixture, query: &str) -> String {
    format!(
        "active={} query={:?} mark={} shows_{query}={} shows_c.txt={}",
        fixture.view.hidden_filter_active(),
        fixture.view.hidden_filter_query(),
        footer_has_label(fixture, &format!("filter: {query}")),
        rendered_name(&fixture.view.widget(), query),
        rendered_name(&fixture.view.widget(), "c.txt")
    )
}

fn wait_filter_kept(fixture: &MinimalFixture, query: &str, when: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !filter_kept(fixture, query) {
        assert!(
            Instant::now() < deadline,
            "{when}: {}",
            filter_kept_state(fixture, query)
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn funnel_stays_hidden(fixture: &MinimalFixture) -> bool {
    use gtk::prelude::*;
    if fixture.view.filter_has_focus() {
        return false;
    }
    [
        "Filter this pane (Ctrl+F)",
        "Filter icons (Ctrl+F)",
        "Filter list (Ctrl+F)",
    ]
    .iter()
    .filter_map(|tooltip| widget_with_tooltip(&fixture.view.widget(), tooltip))
    .all(|widget| !widget.is_visible())
}

fn open_and_cancel_prompt(fixture: &MinimalFixture, key: Key, name: &str) {
    assert!(
        fixture.press(key, ModifierType::empty()),
        "{name} should open"
    );
    wait_until_msg(
        || prompt_has_focus(fixture),
        &format!("{name} prompt focus"),
    );
    assert!(
        fixture.press(Key::Escape, ModifierType::empty()),
        "{name} should cancel"
    );
    wait_until_msg(
        || !prompt_has_focus(fixture),
        &format!("{name} prompt closed"),
    );
}

fn footer_count_text(fixture: &MinimalFixture) -> String {
    use gtk::prelude::*;
    visible_widget_with_class(
        fixture.footer.widget().upcast_ref(),
        "shortcut-footer-count",
    )
    .and_then(|widget| widget.downcast::<gtk::Label>().ok())
    .map(|label| label.text().to_string())
    .unwrap_or_default()
}

fn chord_hint_count(fixture: &MinimalFixture) -> usize {
    fn count(widget: &gtk::Widget) -> usize {
        use gtk::prelude::*;
        let mut total = 0;
        if widget.has_css_class("minimal-chord-hint") {
            total += 1;
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            total += count(&next);
            child = next.next_sibling();
        }
        total
    }
    count(fixture.sidebar.widget.upcast_ref())
}

#[test]
fn minimal_find_jumps_without_filtering_and_repeats() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_find_jumps_without_filtering_and_repeats",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::slash, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "/"));
            fixture.footer.prompt_entry_widget().set_text("b.txt");
            pump_mainloop(Duration::from_millis(50));
            assert_eq!(fixture.cursor(), Some(fixture.index_of("b.txt")));
            assert!(
                !fixture.view.hidden_filter_active(),
                "/ must jump, not filter"
            );
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| !footer_shows(&fixture, "/"));
            assert_eq!(fixture.cursor(), Some(fixture.index_of("b.txt")));
            // Empty `/` + Enter is a no-op; the prompt must still open again.
            assert!(fixture.press(Key::slash, ModifierType::empty()));
            wait_until_msg(
                || footer_shows(&fixture, "/") && prompt_has_focus(&fixture),
                "second prompt is visible",
            );
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| !footer_shows(&fixture, "/"));
            assert_eq!(fixture.cursor(), Some(fixture.index_of("b.txt")));
            assert_eq!(prompt_entry_text(&fixture), "");
        },
    );
}

#[test]
fn minimal_find_keeps_prompt_focus_while_typing() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_find_keeps_prompt_focus_while_typing",
        || {
            let fixture = MinimalFixture::new();
            let match_index = fixture.index_of("b.txt");
            assert!(fixture.press(Key::slash, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "/") && prompt_has_focus(&fixture));
            type_find_char(&fixture, "b");
            assert_eq!(fixture.cursor(), Some(match_index));
            assert!(!fixture.view.hidden_filter_active());
            // `d` trashes when the listing holds focus; it must not run here.
            assert!(
                !fixture.press(Key::d, ModifierType::empty()),
                "d must stay in the prompt instead of running as trash"
            );
            pump_mainloop(Duration::from_millis(50));
            assert!(
                fixture._directory.path().join("b.txt").exists(),
                "a stolen-focus d would trash the match"
            );
            assert!(footer_shows(&fixture, "/"));
            assert!(prompt_has_focus(&fixture));
            type_find_char(&fixture, ".");
            type_find_char(&fixture, "t");
            type_find_char(&fixture, "x");
            type_find_char(&fixture, "t");
            assert_eq!(prompt_entry_text(&fixture), "b.txt");
            assert_eq!(fixture.cursor(), Some(match_index));
            assert_eq!(
                fixture
                    .view
                    .browser()
                    .column_snapshot(0)
                    .expect("column")
                    .count,
                4
            );
        },
    );
}

#[test]
fn minimal_find_highlights_matching_substrings_live() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_find_highlights_matching_substrings_live",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::slash, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "/") && prompt_has_focus(&fixture));
            type_find_char(&fixture, "t");
            wait_until_msg(
                || {
                    highlighted_name(&fixture, "a.txt").as_deref() == Some("t")
                        && highlighted_name(&fixture, "b.txt").as_deref() == Some("t")
                        && highlighted_name(&fixture, "c.txt").as_deref() == Some("t")
                },
                "first typed character must highlight every match",
            );
            assert_eq!(highlighted_name(&fixture, "sub"), None);
            type_find_char(&fixture, "x");
            type_find_char(&fixture, "t");
            wait_until_msg(
                || {
                    highlighted_name(&fixture, "a.txt").as_deref() == Some("txt")
                        && highlighted_name(&fixture, "b.txt").as_deref() == Some("txt")
                        && highlighted_name(&fixture, "c.txt").as_deref() == Some("txt")
                        && highlighted_name(&fixture, "sub").is_none()
                },
                "the query must highlight the matching substring, not hide rows",
            );
            assert!(
                !fixture.view.hidden_filter_active(),
                "/ must not filter while highlighting"
            );
            assert_eq!(
                fixture
                    .view
                    .browser()
                    .column_snapshot(0)
                    .expect("column")
                    .count,
                4
            );
            fixture.footer.prompt_entry_widget().set_text("");
            pump_mainloop(Duration::from_millis(50));
            assert_eq!(highlighted_name(&fixture, "a.txt"), None);
            assert_eq!(highlighted_name(&fixture, "b.txt"), None);
            assert!(prompt_has_focus(&fixture));
        },
    );
}

#[test]
fn minimal_find_keeps_highlights_after_submit_until_escape() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_find_keeps_highlights_after_submit_until_escape",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List] {
                let fixture = find_highlight_fixture(mode);
                assert!(fixture.press(Key::slash, ModifierType::empty()));
                wait_until(|| footer_shows(&fixture, "/") && prompt_has_focus(&fixture));
                fixture.footer.prompt_entry_widget().set_text("txt");
                wait_until(|| txt_find_highlights(&fixture));
                assert!(listing_shows_all_fixture_names(&fixture));
                assert!(fixture.press(Key::Return, ModifierType::empty()));
                wait_until(|| !footer_shows(&fixture, "/"));
                wait_until_msg(
                    || txt_find_highlights(&fixture) && listing_shows_all_fixture_names(&fixture),
                    "submitted / must keep substring highlights on miller and list",
                );
                let after_submit = fixture.cursor();
                assert!(fixture.press(Key::n, ModifierType::empty()));
                pump_mainloop(Duration::from_millis(50));
                assert_ne!(
                    fixture.cursor(),
                    after_submit,
                    "n still jumps after a submitted find"
                );
                assert!(
                    txt_find_highlights(&fixture),
                    "n must not be required to restore highlights"
                );
                assert!(fixture.press(Key::Escape, ModifierType::empty()));
                wait_until_msg(
                    || no_find_highlights(&fixture) && listing_shows_all_fixture_names(&fixture),
                    "browse Esc must dismiss find highlights without hiding rows",
                );
                assert!(fixture.press(Key::n, ModifierType::empty()));
                pump_mainloop(Duration::from_millis(50));
                assert!(
                    no_find_highlights(&fixture),
                    "n repeats the jump without restoring dismissed highlights"
                );
            }
        },
    );
}

#[test]
fn minimal_find_prompt_escape_clears_live_highlights() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_find_prompt_escape_clears_live_highlights",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List] {
                let fixture = find_highlight_fixture(mode);
                assert!(fixture.press(Key::slash, ModifierType::empty()));
                wait_until(|| footer_shows(&fixture, "/") && prompt_has_focus(&fixture));
                fixture.footer.prompt_entry_widget().set_text("txt");
                wait_until(|| txt_find_highlights(&fixture));
                assert!(fixture.press(Key::Escape, ModifierType::empty()));
                wait_until(|| !footer_shows(&fixture, "/"));
                wait_until_msg(
                    || no_find_highlights(&fixture) && listing_shows_all_fixture_names(&fixture),
                    "Esc in the / prompt must not leave live highlights behind",
                );
            }
        },
    );
}

#[test]
fn minimal_find_reapplies_highlights_after_a_view_rebuild() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_find_reapplies_highlights_after_a_view_rebuild",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::slash, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "/") && prompt_has_focus(&fixture));
            fixture.footer.prompt_entry_widget().set_text("txt");
            wait_until(|| highlighted_name(&fixture, "a.txt").as_deref() == Some("txt"));
            fixture.view.set_view_mode(BrowserMode::Icons);
            wait_until_msg(
                || {
                    rendered_name(&fixture.view.widget(), "a.txt")
                        && highlighted_name(&fixture, "a.txt").as_deref() == Some("txt")
                        && highlighted_name(&fixture, "sub").is_none()
                        && prompt_has_focus(&fixture)
                },
                "Icons rebuild must keep live find highlights",
            );
            fixture.view.set_view_mode(BrowserMode::List);
            wait_until_msg(
                || {
                    rendered_name(&fixture.view.widget(), "b.txt")
                        && highlighted_name(&fixture, "b.txt").as_deref() == Some("txt")
                        && prompt_has_focus(&fixture)
                },
                "List rebuild must keep live find highlights",
            );
        },
    );
}

#[test]
fn minimal_search_does_not_highlight_hit_names_live() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_search_does_not_highlight_hit_names_live",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List] {
                let fixture = find_highlight_fixture(mode);
                assert!(fixture.press(Key::s, ModifierType::empty()));
                wait_until(|| footer_shows(&fixture, "search") && prompt_has_focus(&fixture));
                type_find_char(&fixture, "t");
                wait_until_msg(
                    || search_txt_hits_visible(&fixture) && prompt_has_focus(&fixture),
                    "s must still list matching hits while typing",
                );
                pump_mainloop(Duration::from_millis(80));
                assert!(
                    no_search_hit_full_highlights(&fixture),
                    "s must not paint each hit's entire name on {mode:?}"
                );
                assert!(
                    !rendered_name(&fixture.view.widget(), "sub")
                        || no_full_name_search_highlight(&fixture, "sub"),
                    "non-hits must not keep a search highlight"
                );
                fixture.footer.prompt_entry_widget().set_text("");
                wait_until_msg(
                    || {
                        rendered_name(&fixture.view.widget(), "sub")
                            && highlighted_name(&fixture, "a.txt").is_none()
                            && highlighted_name(&fixture, "b.txt").is_none()
                            && !search_hit_name_accent(&fixture, "a.txt")
                            && prompt_has_focus(&fixture)
                    },
                    "empty s query must restore the listing without highlights",
                );
            }
        },
    );
}

#[test]
fn minimal_search_keeps_hits_without_full_highlights_after_submit() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_search_keeps_hits_without_full_highlights_after_submit",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List] {
                for keep in [Key::Return, Key::Escape] {
                    let fixture = find_highlight_fixture(mode);
                    assert!(fixture.press(Key::s, ModifierType::empty()));
                    wait_until(|| footer_shows(&fixture, "search") && prompt_has_focus(&fixture));
                    fixture.footer.prompt_entry_widget().set_text("txt");
                    wait_until_msg(
                        || search_txt_hits_visible(&fixture) && prompt_has_focus(&fixture),
                        "live s results while the prompt is open",
                    );
                    pump_mainloop(Duration::from_millis(80));
                    assert!(
                        no_search_hit_full_highlights(&fixture),
                        "live s must not full-highlight names on {mode:?}"
                    );
                    assert!(fixture.press(keep, ModifierType::empty()));
                    wait_until(|| !prompt_has_focus(&fixture) && !footer_shows(&fixture, "search"));
                    wait_until_msg(
                        || search_txt_hits_visible(&fixture),
                        "submitted or first-Esc s must keep hits",
                    );
                    pump_mainloop(Duration::from_millis(80));
                    assert!(
                        no_search_hit_full_highlights(&fixture),
                        "{keep:?} must keep s hits without entire-name highlight on {mode:?}"
                    );
                    assert!(fixture.press(Key::Escape, ModifierType::empty()));
                    wait_until_msg(
                        || {
                            rendered_name(&fixture.view.widget(), "sub")
                                && highlighted_name(&fixture, "a.txt").is_none()
                                && !search_hit_name_accent(&fixture, "a.txt")
                        },
                        "dismissing search must leave names unhighlighted",
                    );
                }
            }
        },
    );
}

#[test]
fn minimal_search_results_persist_after_first_esc() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_search_results_persist_after_first_esc",
        || {
            let fixture = MinimalFixture::new();
            fixture.footer.observe_browser(&fixture.view);
            wait_until_msg(
                || !footer_count_text(&fixture).is_empty(),
                "listing footer count before search",
            );

            assert!(fixture.press(Key::s, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "search") && prompt_has_focus(&fixture));
            fixture.footer.prompt_entry_widget().set_text("txt");
            wait_until_msg(
                || {
                    fixture.view.selected_search_results().is_some()
                        && rendered_name(&fixture.view.widget(), "a.txt")
                        && rendered_name(&fixture.view.widget(), "b.txt")
                        && prompt_has_focus(&fixture)
                },
                "live s results while the prompt is open",
            );
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until_msg(
                || {
                    !prompt_has_focus(&fixture)
                        && fixture.view.selected_search_results().is_some()
                        && rendered_name(&fixture.view.widget(), "a.txt")
                        && rendered_name(&fixture.view.widget(), "b.txt")
                        && !rendered_name(&fixture.view.widget(), "sub")
                },
                "first Esc keeps search hits and closes the prompt",
            );
            wait_until_msg(
                || footer_count_text(&fixture) == "3 items",
                "footer count stays the hit count after first Esc",
            );

            let before_visual = search_names(&fixture);
            assert!(fixture.press(Key::v, ModifierType::empty()));
            assert!(fixture.press(Key::j, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            let after_visual = search_names(&fixture);
            assert!(
                after_visual.len() >= 2,
                "v then j must fill a range of search hits, got {after_visual:?} from {before_visual:?}"
            );
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until_msg(
                || {
                    fixture.view.selected_search_results().is_some()
                        && !rendered_name(&fixture.view.widget(), "sub")
                },
                "Esc after visual leaves search hits",
            );

            assert!(fixture.press(Key::i, ModifierType::empty()));
            wait_until(|| fixture.preview.is_enabled());
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until(|| !fixture.preview.is_enabled());
            wait_until_msg(
                || fixture.view.selected_search_results().is_some(),
                "Esc after preview keeps search hits",
            );

            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until_msg(
                || {
                    fixture.view.selected_search_results().is_none()
                        && rendered_name(&fixture.view.widget(), "sub")
                        && footer_count_text(&fixture) != "3 items"
                },
                "second Esc restores the directory listing",
            );
        },
    );
}

#[test]
fn minimal_empty_search_esc_cancels_without_results() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_empty_search_esc_cancels_without_results",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::s, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "search") && prompt_has_focus(&fixture));
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until_msg(
                || {
                    !prompt_has_focus(&fixture)
                        && fixture.view.selected_search_results().is_none()
                        && rendered_name(&fixture.view.widget(), "sub")
                },
                "empty s Esc cancels with no results",
            );
            assert!(fixture.press(Key::s, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "search") && prompt_has_focus(&fixture));
            fixture.footer.prompt_entry_widget().set_text("txt");
            wait_until_msg(
                || {
                    prompt_has_focus(&fixture)
                        && fixture.view.selected_search_results().is_some()
                        && rendered_name(&fixture.view.widget(), "a.txt")
                },
                "s after empty cancel still searches",
            );
        },
    );
}

#[test]
fn minimal_search_h_dismisses_kept_results_in_one_step() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_search_h_dismisses_kept_results_in_one_step",
        || {
            let fixture = MinimalFixture::new();
            fixture.footer.observe_browser(&fixture.view);
            submit_prompt(&fixture, Key::f, "a.txt");
            wait_until(|| {
                fixture.view.hidden_filter_active() && footer_shows(&fixture, "filter: a.txt")
            });

            assert!(fixture.press(Key::s, ModifierType::empty()));
            wait_until(|| prompt_has_focus(&fixture));
            fixture.footer.prompt_entry_widget().set_text("txt");
            wait_until(|| {
                fixture.view.selected_search_results().is_some() && prompt_has_focus(&fixture)
            });
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until_msg(
                || !prompt_has_focus(&fixture) && fixture.view.selected_search_results().is_some(),
                "first Esc keeps s hits over the remembered filter",
            );
            assert!(fixture.press(Key::h, ModifierType::empty()));
            wait_until_msg(
                || {
                    fixture.view.hidden_filter_active()
                        && footer_shows(&fixture, "filter: a.txt")
                        && rendered_name(&fixture.view.widget(), "a.txt")
                        && !rendered_name(&fixture.view.widget(), "b.txt")
                },
                "h must restore the remembered f in one step",
            );

            assert!(fixture.press(Key::s, ModifierType::empty()));
            wait_until(|| prompt_has_focus(&fixture));
            fixture.footer.prompt_entry_widget().set_text("txt");
            wait_until(|| fixture.view.selected_search_results().is_some());
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| {
                !prompt_has_focus(&fixture) && fixture.view.selected_search_results().is_some()
            });
            assert!(fixture.press(Key::h, ModifierType::empty()));
            wait_until_msg(
                || fixture.view.hidden_filter_active() && footer_shows(&fixture, "filter: a.txt"),
                "h after Enter still restores f in one step",
            );
        },
    );
}

#[test]
fn minimal_filter_does_not_full_highlight_names() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_filter_does_not_full_highlight_names",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::f, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "filter") && prompt_has_focus(&fixture));
            type_find_char(&fixture, "t");
            wait_until_msg(
                || rendered_name(&fixture.view.widget(), "a.txt") && prompt_has_focus(&fixture),
                "filter must still show matching names",
            );
            pump_mainloop(Duration::from_millis(80));
            assert_eq!(
                highlighted_name(&fixture, "a.txt"),
                None,
                "f must not apply search-hit full-name highlights"
            );
        },
    );
}

#[test]
fn minimal_listing_click_cancels_find_prompt_and_keeps_the_row() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_listing_click_cancels_find_prompt_and_keeps_the_row",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::slash, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "/") && prompt_has_focus(&fixture));
            let clicked = fixture.index_of("b.txt");
            fixture.select_name("b.txt");
            wait_until(|| {
                !footer_shows(&fixture, "/")
                    && listing_cursor_has_focus(&fixture)
                    && fixture.cursor() == Some(clicked)
            });
            assert_eq!(prompt_entry_text(&fixture), "");
            assert!(
                fixture.press(Key::j, ModifierType::empty()),
                "the next key must be a verb, not a leftover prompt cancel"
            );
            assert_ne!(
                fixture.cursor(),
                Some(clicked),
                "j must move from the clicked row"
            );
            assert!(
                !footer_shows(&fixture, "/"),
                "a leftover prompt would still show /"
            );
            assert!(
                fixture._directory.path().join("b.txt").exists(),
                "the click selection must survive; a stolen d would trash it"
            );
        },
    );
}

#[test]
fn minimal_n_repeats_and_big_n_reverses() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_n_repeats_and_big_n_reverses",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::slash, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            fixture.footer.prompt_entry_widget().set_text("txt");
            pump_mainloop(Duration::from_millis(50));
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            let after_find = fixture.cursor().expect("cursor after /txt");
            assert!(fixture.press(Key::n, ModifierType::empty()));
            let after_n = fixture.cursor().expect("cursor after n");
            assert_ne!(after_n, after_find, "n moves to the next match");
            assert!(fixture.press(Key::N, ModifierType::empty()));
            assert_eq!(
                fixture.cursor(),
                Some(after_find),
                "N reverses the last find"
            );
        },
    );
}

#[test]
fn minimal_filter_hides_without_funnel_and_esc_restores() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_filter_hides_without_funnel_and_esc_restores",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::f, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(100));
            fixture.footer.prompt_entry_widget().set_text("a.txt");
            pump_mainloop(Duration::from_millis(50));
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| fixture.view.hidden_filter_active());
            assert!(
                !fixture.view.filter_has_focus(),
                "footer f must not focus the funnel"
            );
            wait_until(|| footer_shows(&fixture, "filter: a.txt"));
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until(|| !fixture.view.hidden_filter_active());
            assert_eq!(
                fixture
                    .view
                    .browser()
                    .column_snapshot(0)
                    .expect("column")
                    .count,
                4
            );
        },
    );
}

#[test]
fn minimal_filter_stays_visible_until_dismissed() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_filter_stays_visible_until_dismissed",
        || {
            let fixture = MinimalFixture::new();
            fixture.footer.observe_browser(&fixture.view);
            fixture
                .footer
                .bind_preferences(&PreferenceManager::shared());

            submit_prompt(&fixture, Key::f, "a.txt");
            wait_filter_kept(&fixture, "a.txt", "applied filter after f Enter");
            assert!(
                funnel_stays_hidden(&fixture),
                "footer f must keep the pane funnel hidden"
            );

            assert!(fixture.press(Key::j, ModifierType::empty()));
            wait_filter_kept(&fixture, "a.txt", "filter after listing motion");

            open_and_cancel_prompt(&fixture, Key::slash, "find");
            wait_filter_kept(&fixture, "a.txt", "filter after cancelled find");
            open_and_cancel_prompt(&fixture, Key::s, "search");
            wait_filter_kept(&fixture, "a.txt", "filter after cancelled search");
            open_and_cancel_prompt(&fixture, Key::a, "create");
            wait_filter_kept(&fixture, "a.txt", "filter after cancelled create");
            open_and_cancel_prompt(&fixture, Key::r, "rename");
            wait_filter_kept(&fixture, "a.txt", "filter after cancelled rename");
            open_and_cancel_prompt(&fixture, Key::z, "history");
            wait_filter_kept(&fixture, "a.txt", "filter after cancelled history");

            assert!(fixture.press(Key::g, ModifierType::empty()));
            wait_until_msg(
                || footer_has_label(&fixture, "g-") || chord_hint_count(&fixture) > 0,
                "g chord armed",
            );
            assert!(fixture.press(Key::space, ModifierType::empty()));
            wait_until_msg(
                || prompt_has_focus(&fixture) && footer_has_label(&fixture, "go ›"),
                "g Space go prompt",
            );
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_filter_kept(&fixture, "a.txt", "filter after cancelled go");

            assert!(fixture.press(Key::v, ModifierType::empty()));
            wait_filter_kept(&fixture, "a.txt", "filter after visual");

            assert!(fixture.press(Key::_2, ModifierType::CONTROL_MASK));
            wait_until_msg(
                || {
                    fixture.view.view_mode() == BrowserMode::Icons
                        && filter_kept(&fixture, "a.txt")
                        && funnel_stays_hidden(&fixture)
                        && !fixture.view.filter_has_focus()
                },
                "Icons rebuild must keep the hidden filter",
            );
            assert!(fixture.press(Key::_3, ModifierType::CONTROL_MASK));
            wait_until_msg(
                || {
                    fixture.view.view_mode() == BrowserMode::List
                        && filter_kept(&fixture, "a.txt")
                        && funnel_stays_hidden(&fixture)
                        && !fixture.view.filter_has_focus()
                },
                "List rebuild must keep the hidden filter",
            );
            assert!(fixture.press(Key::_1, ModifierType::CONTROL_MASK));
            wait_until_msg(
                || {
                    fixture.view.view_mode() == BrowserMode::Columns
                        && filter_kept(&fixture, "a.txt")
                        && funnel_stays_hidden(&fixture)
                        && !fixture.view.filter_has_focus()
                },
                "Columns rebuild must keep the hidden filter",
            );

            let other = MinimalFixture::additional();
            other.footer.observe_browser(&other.view);
            submit_prompt(&other, Key::f, "b.txt");
            wait_filter_kept(&other, "b.txt", "other window applied filter");
            other.view.set_view_mode(BrowserMode::Icons);
            wait_until_msg(
                || {
                    other.view.view_mode() == BrowserMode::Icons
                        && filter_kept(&other, "b.txt")
                        && funnel_stays_hidden(&other)
                },
                "the other window must keep its own hidden filter",
            );
            wait_filter_kept(&fixture, "a.txt", "first window filter after second window");

            assert!(fixture.press(Key::f, ModifierType::empty()));
            wait_until_msg(
                || footer_has_label(&fixture, "filter") && prompt_has_focus(&fixture),
                "reopen f pre-fills",
            );
            assert_eq!(prompt_entry_text(&fixture), "a.txt");
            fixture.footer.prompt_entry_widget().set_text("");
            pump_mainloop(Duration::from_millis(50));
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until_msg(
                || {
                    !fixture.view.hidden_filter_active()
                        && !footer_shows(&fixture, "filter: a.txt")
                        && rendered_name(&fixture.view.widget(), "b.txt")
                        && rendered_name(&fixture.view.widget(), "sub")
                },
                "empty f Enter dismisses the filter",
            );

            submit_prompt(&fixture, Key::f, "a.txt");
            wait_filter_kept(&fixture, "a.txt", "re-applied filter before hints-off");
            PreferenceManager::shared().set_show_keybinding_hints(false);
            wait_until_msg(
                || filter_kept(&fixture, "a.txt") && fixture.footer.widget().is_visible(),
                "hints-off keeps the filter footer",
            );
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until_msg(
                || !fixture.view.hidden_filter_active() && !footer_shows(&fixture, "filter: a.txt"),
                "browse Esc dismisses the filter",
            );
        },
    );
}

#[test]
fn minimal_esc_clears_filter_before_selection() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_esc_clears_filter_before_selection",
        || {
            let fixture = MinimalFixture::new();
            fixture.footer.observe_browser(&fixture.view);
            PreferenceManager::shared().set_filter_include_subfolders(true);
            submit_prompt(&fixture, Key::f, "txt");
            wait_until(|| {
                fixture.view.hidden_filter_active()
                    && rendered_name(&fixture.view.widget(), "a.txt")
                    && rendered_name(&fixture.view.widget(), "b.txt")
                    && rendered_name(&fixture.view.widget(), "c.txt")
            });
            assert!(fixture.press(Key::a, ModifierType::CONTROL_MASK));
            wait_until(|| {
                fixture
                    .view
                    .selected_search_results()
                    .is_some_and(|entries| entries.len() >= 3)
            });
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until(|| !fixture.view.hidden_filter_active());
            wait_until(|| {
                let names: Vec<String> = fixture
                    .fill()
                    .into_iter()
                    .filter_map(|position| {
                        fixture
                            .view
                            .browser()
                            .entry_at(0, position)
                            .map(|entry| entry.display_name)
                    })
                    .collect();
                names.iter().any(|name| name == "a.txt")
                    && names.iter().any(|name| name == "b.txt")
                    && names.iter().any(|name| name == "c.txt")
            });
            wait_until(|| footer_count_text(&fixture).contains("3 files selected"));
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until(|| fixture.fill().is_empty());
            wait_until(|| !footer_count_text(&fixture).contains("selected"));
        },
    );
}

#[test]
fn minimal_icons_filter_and_list_search_j_moves_results() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_icons_filter_and_list_search_j_moves_results",
        || {
            let fixture = MinimalFixture::new();
            fixture.footer.observe_browser(&fixture.view);
            PreferenceManager::shared().set_filter_include_subfolders(true);
            fixture.view.set_view_mode(BrowserMode::Icons);
            wait_until_msg(
                || rendered_name(&fixture.view.widget(), "a.txt"),
                "Icons listing did not show a.txt",
            );
            pump_mainloop(Duration::from_millis(80));
            assert!(fixture.press(Key::f, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(80));
            fixture.footer.prompt_entry_widget().set_text("txt");
            wait_until_msg(
                || {
                    rendered_name(&fixture.view.widget(), "a.txt")
                        && rendered_name(&fixture.view.widget(), "b.txt")
                        && fixture.view.selected_search_results().is_some()
                },
                "Icons filter did not show live search results",
            );
            assert!(
                prompt_has_focus(&fixture),
                "filter prompt keeps the keyboard"
            );
            let before_down = search_names(&fixture);
            assert!(fixture.press(Key::Down, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            assert!(
                prompt_has_focus(&fixture),
                "Down stays in the filter prompt"
            );
            let after_down = search_names(&fixture);
            assert_ne!(
                after_down.first(),
                before_down.first(),
                "Down in the filter prompt must move results"
            );
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| {
                gtk::prelude::RootExt::focus(&fixture.window).is_some()
                    && !prompt_has_focus(&fixture)
                    && !search_names(&fixture).is_empty()
            });
            let after_enter = search_names(&fixture);
            assert!(fixture.press(Key::j, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            assert_ne!(
                search_names(&fixture).first(),
                after_enter.first(),
                "j must move Icons filter results after Enter"
            );

            fixture.view.dismiss_hidden_filter();
            wait_until(|| fixture.view.selected_search_results().is_none());
            fixture.view.set_view_mode(BrowserMode::List);
            wait_until_msg(
                || rendered_name(&fixture.view.widget(), "a.txt"),
                "List listing did not show a.txt",
            );
            pump_mainloop(Duration::from_millis(80));
            assert!(fixture.press(Key::s, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(80));
            fixture.footer.prompt_entry_widget().set_text("txt");
            wait_until_msg(
                || {
                    rendered_name(&fixture.view.widget(), "a.txt")
                        && rendered_name(&fixture.view.widget(), "b.txt")
                        && rendered_name(&fixture.view.widget(), "c.txt")
                        && fixture.view.selected_search_results().is_some()
                },
                "List search did not show live results",
            );
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| {
                gtk::prelude::RootExt::focus(&fixture.window).is_some()
                    && !prompt_has_focus(&fixture)
                    && !search_names(&fixture).is_empty()
            });
            let first = search_names(&fixture);
            assert!(fixture.press(Key::j, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            let after_j = search_names(&fixture);
            assert_ne!(
                after_j.first(),
                first.first(),
                "j must move List search results after Enter"
            );
            assert!(fixture.press(Key::G, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            assert_ne!(
                search_names(&fixture).first(),
                after_j.first(),
                "G must jump to the last List search result"
            );
        },
    );
}

#[test]
fn minimal_shift_s_does_not_search() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_shift_s_does_not_search",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::S, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            assert_eq!(prompt_entry_text(&fixture), "");
            assert!(!prompt_has_focus(&fixture));
            assert!(!fixture.view.hidden_filter_active());
            assert!(fixture.view.selected_search_results().is_none());
            assert!(!footer_shows(&fixture, "search"));
        },
    );
}

#[test]
fn minimal_g_shows_keycaps_and_second_key_clears() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_g_shows_keycaps_and_second_key_clears",
        || {
            let fixture = MinimalFixture::new();
            let target = fixture._directory.path().join("sub");
            fixture
                .sidebar
                .state
                .pin_location(Location::local(target), "sub".into());
            pump_mainloop(Duration::from_millis(100));
            assert!(fixture.press(Key::g, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "g-"));
            wait_until(|| chord_hint_count(&fixture) >= 2);
            wait_until(|| fixture.footer.chord_hints_visible());
            let labels = fixture.footer.chord_hint_labels();
            for expected in [
                "g first item",
                "h Home",
                "d Downloads",
                "c Config",
                "t Trash",
                "n Network",
                "r Recent",
                "k Documents",
                "p Pictures",
                "v Videos",
                "1–9 pins",
                "Space path",
            ] {
                assert!(
                    labels.iter().any(|line| line == expected),
                    "{expected} missing from {labels:?}"
                );
            }
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            assert_eq!(chord_hint_count(&fixture), 0);
            assert!(!fixture.footer.chord_hints_visible());
            assert!(!footer_shows(&fixture, "g-"));
        },
    );
}

#[test]
fn minimal_sort_chord_shows_option_hints() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_sort_chord_shows_option_hints",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::comma, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, ",-"));
            wait_until(|| fixture.footer.chord_hints_visible());
            let labels = fixture.footer.chord_hint_labels();
            for expected in ["a name", "m modified", "s size", "e type"] {
                assert!(
                    labels.iter().any(|line| line == expected),
                    "{expected} missing from {labels:?}"
                );
            }
            assert_eq!(
                fixture.footer.chord_hint_keycaps(),
                vec![
                    vec!["a".to_owned()],
                    vec!["m".to_owned()],
                    vec!["s".to_owned()],
                    vec!["e".to_owned()],
                ],
            );
            assert!(
                labels.iter().any(|line| line.contains("shift reverses")),
                "shift reverses missing from {labels:?}"
            );
            assert_eq!(chord_hint_count(&fixture), 0);
            assert!(fixture.press(Key::s, ModifierType::empty()));
            wait_until(|| {
                fixture
                    .view
                    .browser()
                    .column_preferences(0)
                    .is_some_and(|prefs| prefs.sort_key == crate::model::SortKey::Size)
            });
            assert!(!fixture.footer.chord_hints_visible());
            assert!(!footer_shows(&fixture, ",-"));
        },
    );
}

#[test]
fn minimal_sort_chord_second_keys_are_not_browse_verbs() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_sort_chord_second_keys_are_not_browse_verbs",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::comma, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, ",-"));
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            assert!(!footer_shows(&fixture, ",-"));
            assert!(
                !footer_shows(&fixture, "Unknown chord"),
                "Escape cancels the sort chord without a flash"
            );

            assert!(fixture.press(Key::comma, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, ",-"));
            wait_until(|| fixture.footer.chord_hints_visible());
            for shift_key in [Key::Shift_L, Key::Shift_R] {
                assert!(fixture.press(shift_key, ModifierType::SHIFT_MASK));
                pump_mainloop(Duration::from_millis(50));
                assert!(
                    footer_shows(&fixture, ",-"),
                    "{shift_key:?} must leave the sort chord armed"
                );
                assert!(
                    fixture.footer.chord_hints_visible(),
                    "{shift_key:?} must keep sort option hints up"
                );
                assert!(
                    !footer_shows(&fixture, "Unknown chord"),
                    "{shift_key:?} must not flash Unknown chord"
                );
            }
            assert!(fixture.press(Key::s, ModifierType::SHIFT_MASK));
            wait_until(|| {
                sort_is(
                    &fixture,
                    crate::model::SortKey::Size,
                    Some(crate::model::SortDirection::Descending),
                )
            });
            assert!(!footer_shows(&fixture, ",-"));
            assert!(!fixture.footer.chord_hints_visible());
            assert!(
                !prompt_has_focus(&fixture),
                ", then Shift+s must reverse size sort, not open search"
            );

            assert!(fixture.press(Key::comma, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, ",-"));
            assert!(fixture.press(Key::Shift_L, ModifierType::SHIFT_MASK));
            pump_mainloop(Duration::from_millis(50));
            assert!(footer_shows(&fixture, ",-"));
            assert!(fixture.press(Key::S, ModifierType::SHIFT_MASK));
            wait_until(|| {
                sort_is(
                    &fixture,
                    crate::model::SortKey::Size,
                    Some(crate::model::SortDirection::Descending),
                )
            });
            assert!(!footer_shows(&fixture, ",-"));
            assert!(
                !prompt_has_focus(&fixture),
                ", then Shift+S must reverse size sort, not open search"
            );

            assert!(fixture.press(Key::comma, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, ",-"));
            assert!(fixture.press(Key::Shift_L, ModifierType::SHIFT_MASK));
            assert!(fixture.press(Key::a, ModifierType::SHIFT_MASK));
            wait_until(|| {
                sort_is(
                    &fixture,
                    crate::model::SortKey::Name,
                    Some(crate::model::SortDirection::Descending),
                )
            });
            assert!(
                !prompt_has_focus(&fixture) && !footer_shows(&fixture, "new"),
                ", then Shift+a must reverse name sort, not create"
            );

            assert!(fixture.press(Key::comma, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, ",-"));
            assert!(fixture.press(Key::s, ModifierType::empty()));
            wait_until(|| {
                sort_is(
                    &fixture,
                    crate::model::SortKey::Size,
                    Some(crate::model::SortDirection::Ascending),
                )
            });

            assert!(fixture.press(Key::slash, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            fixture.footer.prompt_entry_widget().set_text("txt");
            pump_mainloop(Duration::from_millis(50));
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            let after_find = fixture.cursor().expect("cursor after /txt");
            assert!(fixture.press(Key::n, ModifierType::empty()));
            let after_n = fixture.cursor().expect("cursor after n");
            assert_ne!(after_n, after_find, "n repeats find when no chord is armed");

            assert!(fixture.press(Key::comma, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, ",-"));
            assert!(fixture.press(Key::n, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "Unknown chord"));
            assert_eq!(
                fixture.cursor(),
                Some(after_n),
                ",n must not sort or repeat find"
            );
            assert!(!prompt_has_focus(&fixture), ",n must not reopen find");

            assert!(fixture.press(Key::comma, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, ",-"));
            assert!(fixture.press(Key::S, ModifierType::empty()));
            wait_until(|| {
                sort_is(
                    &fixture,
                    crate::model::SortKey::Size,
                    Some(crate::model::SortDirection::Descending),
                )
            });
            assert!(
                !prompt_has_focus(&fixture),
                ",S must reverse size sort, not open search"
            );

            assert!(fixture.press(Key::comma, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, ",-"));
            assert!(fixture.press(Key::a, ModifierType::empty()));
            wait_until(|| sort_is(&fixture, crate::model::SortKey::Name, None));
            assert!(
                !prompt_has_focus(&fixture) && !footer_shows(&fixture, "new"),
                ",a must sort by name, not create"
            );

            let cursor = fixture.cursor();
            for key in [Key::j, Key::t] {
                assert!(fixture.press(Key::comma, ModifierType::empty()));
                wait_until(|| footer_shows(&fixture, ",-"));
                assert!(fixture.press(key, ModifierType::empty()));
                wait_until(|| footer_shows(&fixture, "Unknown chord"));
                assert_eq!(fixture.cursor(), cursor, "{key:?} must not complete a sort");
            }
            assert!(
                PreferenceManager::shared().minimal_mode(),
                "unknown sort completion must not leave the mode"
            );
        },
    );
}

#[test]
fn minimal_search_hits_do_not_steal_an_armed_chord() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_search_hits_do_not_steal_an_armed_chord",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::s, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "search") && prompt_has_focus(&fixture));
            fixture.footer.prompt_entry_widget().set_text("txt");
            wait_until(|| {
                fixture.view.selected_search_results().is_some() && prompt_has_focus(&fixture)
            });
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until(|| {
                !prompt_has_focus(&fixture) && fixture.view.selected_search_results().is_some()
            });

            let hits = search_names(&fixture);
            assert!(fixture.press(Key::g, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "g-"));
            assert!(fixture.press(Key::j, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "Unknown chord"));
            assert_eq!(
                search_names(&fixture),
                hits,
                "g then j must not move kept search hits"
            );

            assert!(fixture.press(Key::c, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "c-"));
            assert!(fixture.press(Key::j, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "Unknown chord"));
            assert_eq!(
                search_names(&fixture),
                hits,
                "c then j must not move kept search hits"
            );

            assert!(fixture.press(Key::g, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "g-"));
            assert!(fixture.press(Key::h, ModifierType::empty()));
            wait_until(|| {
                fixture.view.browser().active_location() == Some(Location::local(home_directory()))
            });
        },
    );
}

#[test]
fn minimal_copy_chord_hints_close_when_the_mode_turns_off() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_copy_chord_hints_close_when_the_mode_turns_off",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::c, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "c-"));
            wait_until(|| fixture.footer.chord_hints_visible());
            let labels = fixture.footer.chord_hint_labels();
            for expected in ["c path", "n name"] {
                assert!(
                    labels.iter().any(|line| line == expected),
                    "{expected} missing from {labels:?}"
                );
            }
            PreferenceManager::shared().set_minimal_mode(false);
            pump_mainloop(Duration::from_millis(50));
            assert!(!fixture.footer.chord_hints_visible());
            assert!(!footer_shows(&fixture, "c-"));
            PreferenceManager::shared().set_minimal_mode(true);
        },
    );
}

#[test]
fn minimal_g_special_dir_flashes_when_missing() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_g_special_dir_flashes_when_missing",
        || {
            for (key, name, directory) in [
                (Key::d, "Downloads", glib::UserDirectory::Downloads),
                (Key::k, "Documents", glib::UserDirectory::Documents),
                (Key::p, "Pictures", glib::UserDirectory::Pictures),
                (Key::v, "Videos", glib::UserDirectory::Videos),
            ] {
                assert!(
                    glib::user_special_dir(directory).is_none(),
                    "isolated HOME has no {name} special dir"
                );
                let fixture = MinimalFixture::new();
                let here = fixture.view.browser().active_location();
                assert!(fixture.press(Key::g, ModifierType::empty()));
                wait_until(|| footer_shows(&fixture, "g-"));
                assert!(fixture.press(key, ModifierType::empty()));
                wait_until(|| footer_shows(&fixture, &format!("No {name} folder")));
                assert_eq!(fixture.view.browser().active_location(), here);
                assert_eq!(chord_hint_count(&fixture), 0);
                assert!(
                    !prompt_has_focus(&fixture),
                    "g then {key:?} must not open a footer prompt"
                );
                assert!(PreferenceManager::shared().minimal_mode());
            }
        },
    );
}

#[test]
fn minimal_g_opens_network_and_recent() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_g_opens_network_and_recent",
        || {
            for (key, location) in [
                (Key::n, Location::uri("network:///")),
                (Key::r, Location::uri("recent:///")),
            ] {
                let fixture = MinimalFixture::new();
                assert!(fixture.press(Key::g, ModifierType::empty()));
                wait_until(|| footer_shows(&fixture, "g-"));
                assert!(fixture.press(key, ModifierType::empty()));
                wait_until(|| fixture.view.browser().active_location() == Some(location.clone()));
                assert_eq!(chord_hint_count(&fixture), 0);
                assert!(
                    !prompt_has_focus(&fixture),
                    "g then {key:?} must navigate instead of opening a prompt"
                );
            }
        },
    );
}

#[test]
fn minimal_unknown_chord_flashes_and_keeps_browsing() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_unknown_chord_flashes_and_keeps_browsing",
        || {
            let fixture = MinimalFixture::new();
            let cursor = fixture.cursor();
            assert!(fixture.press(Key::g, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "g-"));
            assert!(fixture.press(Key::q, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "Unknown chord"));
            assert_eq!(chord_hint_count(&fixture), 0);
            assert_eq!(fixture.cursor(), cursor);
            assert!(
                PreferenceManager::shared().minimal_mode(),
                "unknown chord must not leave the mode"
            );
        },
    );
}

#[test]
fn minimal_g_space_goes_to_the_typed_path() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_g_space_goes_to_the_typed_path",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::g, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "g-"));
            assert!(fixture.press(Key::space, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "go ›"));
            let target = fixture._directory.path().join("sub");
            fixture
                .footer
                .prompt_entry_widget()
                .set_text(target.to_string_lossy().as_ref());
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| {
                fixture.view.browser().active_location() == Some(Location::local(target.clone()))
            });
            assert_eq!(prompt_entry_text(&fixture), "", "secrets never linger");
        },
    );
}

#[test]
fn minimal_g_space_tab_cycles_matching_folders() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_g_space_tab_cycles_matching_folders",
        || {
            let fixture = MinimalFixture::new();
            let root = fixture._directory.path();
            std::fs::create_dir(root.join("docs")).expect("docs");
            std::fs::create_dir(root.join("documents")).expect("documents");
            std::fs::create_dir_all(root.join("nested").join("deep")).expect("nested/deep");
            std::fs::create_dir_all(root.join("nested").join("deeper")).expect("nested/deeper");
            fixture.view.browser().navigate(Location::local(root));
            wait_until(|| {
                listing_has_name(&fixture, "docs") && listing_has_name(&fixture, "nested")
            });

            assert!(fixture.press(Key::g, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "g-"));
            assert!(fixture.press(Key::space, ModifierType::empty()));
            wait_until(|| prompt_has_focus(&fixture) && footer_shows(&fixture, "go ›"));

            fixture.footer.prompt_entry_widget().set_text("d");
            assert!(
                fixture.press(Key::Tab, ModifierType::empty()),
                "Tab must stay in the go prompt"
            );
            assert_eq!(prompt_entry_text(&fixture), "docs");
            assert!(prompt_has_focus(&fixture), "Tab must not move GTK focus");
            assert!(fixture.press(Key::Tab, ModifierType::empty()));
            assert_eq!(prompt_entry_text(&fixture), "documents");
            assert!(fixture.press(Key::Tab, ModifierType::empty()));
            assert_eq!(prompt_entry_text(&fixture), "docs", "Tab wraps");
            assert!(fixture.press(Key::Tab, ModifierType::SHIFT_MASK));
            assert_eq!(
                prompt_entry_text(&fixture),
                "documents",
                "Shift+Tab reverses"
            );
            assert!(fixture.press(Key::ISO_Left_Tab, ModifierType::empty()));
            assert_eq!(prompt_entry_text(&fixture), "docs");
            assert!(prompt_has_focus(&fixture));

            fixture.footer.prompt_entry_widget().set_text("a");
            assert!(fixture.press(Key::Tab, ModifierType::empty()));
            assert_eq!(prompt_entry_text(&fixture), "a", "files are not completed");
            assert!(prompt_has_focus(&fixture));

            fixture.footer.prompt_entry_widget().set_text("zzz");
            assert!(fixture.press(Key::Tab, ModifierType::empty()));
            assert_eq!(prompt_entry_text(&fixture), "zzz");
            assert!(prompt_has_focus(&fixture));
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            assert_eq!(prompt_entry_text(&fixture), "");

            assert!(fixture.press(Key::g, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "g-"));
            assert!(fixture.press(Key::space, ModifierType::empty()));
            wait_until(|| prompt_has_focus(&fixture) && footer_shows(&fixture, "go ›"));
            fixture.footer.prompt_entry_widget().set_text("nested/de");
            assert!(fixture.press(Key::Tab, ModifierType::empty()));
            assert_eq!(prompt_entry_text(&fixture), "nested/deep");
            assert!(fixture.press(Key::Tab, ModifierType::empty()));
            assert_eq!(prompt_entry_text(&fixture), "nested/deeper");
            let target = root.join("nested").join("deeper");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| {
                fixture.view.browser().active_location() == Some(Location::local(target.clone()))
            });
            assert_eq!(prompt_entry_text(&fixture), "");
        },
    );
}

#[test]
fn minimal_z_picks_a_visible_history_candidate() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_z_picks_a_visible_history_candidate",
        || {
            let fixture = MinimalFixture::new();
            let visits = tempfile::tempdir().expect("history visits");
            let alpha = visits.path().join("alpha-project");
            let beta = visits.path().join("beta-project");
            std::fs::create_dir(&alpha).expect("alpha folder");
            std::fs::create_dir(&beta).expect("beta folder");
            let history = crate::services::NavigationHistory::shared();
            history.record(&alpha);
            history.record(&alpha);
            history.record(&beta);
            assert!(fixture.press(Key::z, ModifierType::empty()));
            wait_until(|| fixture.footer.history_candidates_visible());
            wait_until(|| prompt_has_focus(&fixture));
            assert_eq!(
                fixture.footer.history_candidate_names(),
                vec!["alpha-project".to_owned(), "beta-project".to_owned()],
                "empty z lists frecency instead of jumping blindly"
            );
            assert_eq!(
                fixture.footer.selected_history_path().as_deref(),
                Some(alpha.as_path())
            );
            assert!(fixture.press(Key::Down, ModifierType::empty()));
            wait_until(|| prompt_has_focus(&fixture));
            assert_eq!(
                fixture.footer.selected_history_path().as_deref(),
                Some(beta.as_path())
            );
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| {
                fixture.view.browser().active_location() == Some(Location::local(beta.clone()))
            });
            assert!(!fixture.footer.history_candidates_visible());
            assert_eq!(prompt_entry_text(&fixture), "");
        },
    );
}

#[test]
fn minimal_a_creates_file_and_folder_without_uniquifying() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_a_creates_file_and_folder_without_uniquifying",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::a, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(100));
            fixture.footer.prompt_entry_widget().set_text("created.txt");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| fixture._directory.path().join("created.txt").exists());
            wait_until(|| fixture.cursor_name().as_deref() == Some("created.txt"));
            let failed = Rc::new(std::cell::Cell::new(0u32));
            let failed_for = failed.clone();
            fixture.view.browser().observe(move |event| {
                if matches!(event, BrowserEvent::OperationFailed { .. }) {
                    failed_for.set(failed_for.get() + 1);
                }
            });
            assert!(fixture.press(Key::a, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(100));
            fixture.footer.prompt_entry_widget().set_text("created.txt");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| failed.get() >= 1);
            assert!(
                !fixture._directory.path().join("created.txt (1)").exists(),
                "file conflicts error instead of uniquifying"
            );
            dismiss_error_dialog(&fixture);
            assert!(fixture.press(Key::a, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(100));
            fixture.footer.prompt_entry_widget().set_text("newdir/");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| fixture._directory.path().join("newdir").is_dir());
            wait_until(|| fixture.cursor_name().as_deref() == Some("newdir"));
            assert!(fixture.press(Key::a, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(100));
            fixture.footer.prompt_entry_widget().set_text("newdir/");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| failed.get() >= 2);
            assert!(
                !fixture._directory.path().join("newdir (1)").exists(),
                "folder conflicts error instead of uniquifying"
            );
            assert!(!fixture.view.rename_is_active());
        },
    );
}

#[test]
fn minimal_a_selects_the_created_row() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_a_selects_the_created_row",
        || {
            let fixture = MinimalFixture::new();
            fixture.select_name("sub");
            assert_eq!(fixture.cursor_name().as_deref(), Some("sub"));
            assert!(fixture.press(Key::a, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(100));
            fixture.footer.prompt_entry_widget().set_text("created.txt");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| fixture.cursor_name().as_deref() == Some("created.txt"));
            assert_eq!(
                fixture
                    .view
                    .browser()
                    .selected_entries()
                    .iter()
                    .map(|entry| entry.display_name.as_str())
                    .collect::<Vec<_>>(),
                ["created.txt"]
            );
            assert!(!fixture.view.rename_is_active());

            fixture.select_name("sub");
            assert!(fixture.press(Key::a, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(100));
            fixture.footer.prompt_entry_widget().set_text("newdir/");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| fixture.cursor_name().as_deref() == Some("newdir"));
            assert!(!fixture.view.rename_is_active());
            assert_eq!(
                fixture.view.browser().active_location(),
                Some(Location::local(fixture._directory.path()))
            );
        },
    );
}

#[test]
fn minimal_a_rejects_a_bare_slash() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_a_rejects_a_bare_slash",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::a, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(100));
            fixture.footer.prompt_entry_widget().set_text("/");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(100));
            // Invalid names stay in the prompt so they can be fixed.
            assert_eq!(prompt_entry_text(&fixture), "/");
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            assert_eq!(prompt_entry_text(&fixture), "");
        },
    );
}

#[test]
fn minimal_r_renames_without_the_row_editor() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_r_renames_without_the_row_editor",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::r, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "rename"));
            fixture.footer.prompt_entry_widget().set_text("renamed.txt");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| fixture._directory.path().join("renamed.txt").exists());
            assert!(!fixture.view.rename_is_active());
            assert!(!fixture._directory.path().join("a.txt").exists());
        },
    );
}

#[test]
fn minimal_file_view_focus_restored_after_prompt_search_and_error() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_file_view_focus_restored_after_prompt_search_and_error",
        || {
            let fixture = MinimalFixture::new();
            assert!(fixture.press(Key::r, ModifierType::empty()));
            wait_until_msg(|| footer_shows(&fixture, "rename"), "rename prompt");
            fixture.footer.prompt_entry_widget().set_text("renamed.txt");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until_msg(
                || fixture._directory.path().join("renamed.txt").exists(),
                "rename wrote renamed.txt",
            );
            wait_until_msg(
                || listing_cursor_has_focus(&fixture),
                "rename Enter restores listing cursor",
            );

            assert!(fixture.press(Key::s, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(80));
            fixture.footer.prompt_entry_widget().set_text("txt");
            wait_until_msg(
                || fixture.view.selected_search_results().is_some() && prompt_has_focus(&fixture),
                "search prompt shows results",
            );
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until_msg(
                || listing_cursor_has_focus(&fixture),
                "search Enter restores listing cursor",
            );
            assert!(fixture.press(Key::h, ModifierType::empty()));
            wait_until_msg(
                || {
                    fixture.view.selected_search_results().is_none()
                        && listing_cursor_has_focus(&fixture)
                },
                "search h restores listing cursor",
            );

            let failed = Rc::new(std::cell::Cell::new(0u32));
            let failed_for = failed.clone();
            fixture.view.browser().observe(move |event| {
                if matches!(event, BrowserEvent::OperationFailed { .. }) {
                    failed_for.set(failed_for.get() + 1);
                }
            });
            assert!(fixture.press(Key::a, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(100));
            fixture.footer.prompt_entry_widget().set_text("renamed.txt");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until_msg(|| failed.get() >= 1, "create conflict dialog");
            let close = modal_button(fixture.window.upcast_ref(), "Close").expect("error Close");
            close.emit_clicked();
            wait_until_msg(
                || listing_cursor_has_focus(&fixture),
                "error dialog Close restores listing cursor",
            );

            let icons = MinimalFixture::new();
            icons.view.set_view_mode(BrowserMode::Icons);
            wait_until_msg(
                || rendered_name(&icons.view.widget(), "a.txt"),
                "icons shows listing",
            );
            pump_mainloop(Duration::from_millis(80));
            assert!(icons.press(Key::f, ModifierType::empty()));
            wait_until_msg(
                || prompt_has_focus(&icons) && footer_shows(&icons, "filter"),
                "icons filter prompt",
            );
            icons.footer.prompt_entry_widget().set_text("txt");
            pump_mainloop(Duration::from_millis(50));
            assert!(icons.press(Key::Return, ModifierType::empty()));
            wait_until_msg(
                || listing_cursor_has_focus(&icons),
                "icons filter Enter restores listing cursor",
            );
        },
    );
}

#[test]
fn minimal_prompt_teardown_on_disable_clears_secrets_and_hints() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_prompt_teardown_on_disable_clears_secrets_and_hints",
        || {
            let first = MinimalFixture::new();
            let second = MinimalFixture::additional();
            assert!(first.press(Key::g, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            assert!(first.press(Key::space, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            first
                .footer
                .prompt_entry_widget()
                .set_text("smb://user:pass@host/share");
            PreferenceManager::shared().set_minimal_mode(false);
            pump_mainloop(Duration::from_millis(100));
            assert_eq!(prompt_entry_text(&first), "");
            assert_eq!(chord_hint_count(&first), 0);
            assert!(!footer_shows(&first, "g-"));
            assert!(first.view.item_view_has_focus());
            assert_eq!(prompt_entry_text(&second), "");
            PreferenceManager::shared().set_minimal_mode(true);
        },
    );
}

fn open_empty_sub(fixture: &MinimalFixture) {
    fixture.view.set_view_mode(BrowserMode::Icons);
    wait_until(|| rendered_name(&fixture.view.widget(), "sub"));
    fixture.select_name("sub");
    assert!(fixture.press(Key::l, ModifierType::empty()));
    let child = Location::local(fixture._directory.path().join("sub"));
    wait_until(|| {
        fixture.view.browser().active_location() == Some(child.clone())
            && fixture
                .view
                .browser()
                .column_snapshot(0)
                .is_some_and(|column| !column.loading && column.count == 0)
    });
}

fn add_nested_hit(fixture: &MinimalFixture) -> std::path::PathBuf {
    let dir = fixture._directory.path().join("innerdir");
    std::fs::create_dir(&dir).expect("innerdir");
    let path = dir.join("unique-hit.txt");
    std::fs::write(&path, b"nested-hit-content").expect("nested file");
    fixture
        .view
        .browser()
        .navigate(Location::local(fixture._directory.path()));
    wait_until(|| {
        fixture
            .view
            .browser()
            .column_snapshot(0)
            .is_some_and(|column| !column.loading)
    });
    path
}

fn search_names(fixture: &MinimalFixture) -> Vec<String> {
    fixture
        .view
        .selected_search_results()
        .unwrap_or_default()
        .into_iter()
        .map(|entry| entry.display_name)
        .collect()
}

/// Names yank / cut / trash would use: overlay fill when that list is showing,
/// otherwise the directory selection. `Some(empty)` is an empty overlay fill.
fn yank_names(fixture: &MinimalFixture) -> Vec<String> {
    match fixture.view.selected_search_results() {
        Some(entries) => entries
            .into_iter()
            .map(|entry| entry.display_name)
            .collect(),
        None => fixture
            .view
            .browser()
            .selected_entries()
            .into_iter()
            .map(|entry| entry.display_name)
            .collect(),
    }
}

fn overlay_listing_names(fixture: &MinimalFixture) -> Vec<String> {
    fixture
        .view
        .search_result_listing()
        .unwrap_or_default()
        .into_iter()
        .map(|entry| entry.display_name)
        .collect()
}

fn listing_selected_names(fixture: &MinimalFixture) -> Vec<String> {
    fixture
        .fill()
        .into_iter()
        .filter_map(|position| {
            fixture
                .view
                .browser()
                .entry_at(0, position)
                .map(|entry| entry.display_name)
        })
        .collect()
}

fn wait_txt_overlay(fixture: &MinimalFixture, when: &str) {
    wait_until_msg(
        || {
            fixture.view.selected_search_results().is_some()
                && rendered_name(&fixture.view.widget(), "a.txt")
                && rendered_name(&fixture.view.widget(), "b.txt")
                && rendered_name(&fixture.view.widget(), "c.txt")
                && !rendered_name(&fixture.view.widget(), "sub")
        },
        when,
    );
    wait_until_msg(
        || overlay_listing_names(fixture).len() >= 2,
        "overlay listing should have at least two txt matches",
    );
    if yank_names(fixture).is_empty() {
        assert!(
            fixture.press(Key::j, ModifierType::empty()),
            "j should select the first overlay match"
        );
        wait_until_msg(
            || !yank_names(fixture).is_empty(),
            "j must land on a visible overlay match",
        );
    }
}

fn sort_is(
    fixture: &MinimalFixture,
    key: crate::model::SortKey,
    direction: Option<crate::model::SortDirection>,
) -> bool {
    fixture
        .view
        .browser()
        .column_preferences(0)
        .is_some_and(|prefs| {
            prefs.sort_key == key
                && direction.is_none_or(|expected| prefs.sort_direction == expected)
        })
}

fn submit_prompt(fixture: &MinimalFixture, key: Key, text: &str) {
    assert!(fixture.press(key, ModifierType::empty()));
    pump_mainloop(Duration::from_millis(80));
    fixture.footer.prompt_entry_widget().set_text(text);
    pump_mainloop(Duration::from_millis(50));
    assert!(fixture.press(Key::Return, ModifierType::empty()));
}

#[test]
fn minimal_s_recurses_when_include_subfolders_is_off_and_f_does_not() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_s_recurses_when_include_subfolders_is_off_and_f_does_not",
        || {
            let fixture = MinimalFixture::new();
            add_nested_hit(&fixture);
            PreferenceManager::shared().set_filter_include_subfolders(false);
            submit_prompt(&fixture, Key::s, "unique-hit");
            wait_until(|| rendered_name(&fixture.view.widget(), "unique-hit.txt"));
            assert!(
                search_names(&fixture)
                    .iter()
                    .any(|name| name == "unique-hit.txt")
                    || rendered_name(&fixture.view.widget(), "unique-hit.txt"),
                "s must return nested hits with include-subfolders off"
            );
            assert!(fixture.press(Key::h, ModifierType::empty()));
            wait_until(|| !rendered_name(&fixture.view.widget(), "unique-hit.txt"));
            submit_prompt(&fixture, Key::f, "unique-hit");
            pump_mainloop(Duration::from_millis(250));
            assert!(
                !rendered_name(&fixture.view.widget(), "unique-hit.txt"),
                "listing filter must not follow nested hits when include-subfolders is off"
            );
        },
    );
}

#[test]
fn minimal_search_footer_shows_result_count() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_search_footer_shows_result_count",
        || {
            let fixture = MinimalFixture::new();
            fixture.footer.observe_browser(&fixture.view);
            wait_until_msg(
                || !footer_count_text(&fixture).is_empty(),
                "search footer count is empty before query",
            );
            let before = footer_count_text(&fixture);
            submit_prompt(&fixture, Key::s, "txt");
            wait_until_msg(
                || {
                    rendered_name(&fixture.view.widget(), "a.txt")
                        && rendered_name(&fixture.view.widget(), "b.txt")
                        && rendered_name(&fixture.view.widget(), "c.txt")
                        && fixture.view.selected_search_results().is_some()
                },
                "Columns search results did not appear",
            );
            fixture.view.focus_first_search_result();
            wait_until_msg(
                || footer_count_text(&fixture) == "3 items",
                "Columns search footer did not show 3 items",
            );
            assert_ne!(
                footer_count_text(&fixture),
                before,
                "search must not keep the hidden listing's fill"
            );

            fixture.view.dismiss_hidden_filter();
            wait_until(|| fixture.view.selected_search_results().is_none());
            fixture.view.set_view_mode(BrowserMode::Icons);
            wait_until(|| rendered_name(&fixture.view.widget(), "a.txt"));
            submit_prompt(&fixture, Key::s, "txt");
            wait_until_msg(
                || {
                    rendered_name(&fixture.view.widget(), "a.txt")
                        && fixture.view.selected_search_results().is_some()
                },
                "Icons search results did not appear",
            );
            fixture.view.focus_first_search_result();
            wait_until_msg(
                || footer_count_text(&fixture) == "3 items",
                "Icons search footer did not show 3 items",
            );

            fixture.view.dismiss_hidden_filter();
            wait_until(|| fixture.view.selected_search_results().is_none());
            fixture.view.set_view_mode(BrowserMode::List);
            wait_until(|| rendered_name(&fixture.view.widget(), "a.txt"));
            pump_mainloop(Duration::from_millis(80));
            assert!(fixture.press(Key::s, ModifierType::empty()));
            fixture.footer.prompt_entry_widget().set_text("txt");
            wait_until_msg(
                || {
                    fixture
                        .view
                        .search_result_listing()
                        .is_some_and(|entries| entries.len() == 3)
                        && fixture.view.selected_search_results().is_some()
                },
                "List search results did not appear",
            );
            fixture.view.focus_first_search_result();
            wait_until_msg(
                || footer_count_text(&fixture) == "3 items",
                "List search footer did not show 3 items",
            );
        },
    );
}

#[test]
fn minimal_search_l_previews_files_and_enters_directories_in_columns_and_icons() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_search_l_previews_files_and_enters_directories_in_columns_and_icons",
        || {
            let fixture = MinimalFixture::new();
            fixture.footer.observe_browser(&fixture.view);
            let nested = add_nested_hit(&fixture);
            PreferenceManager::shared().set_filter_include_subfolders(false);
            let opened = Rc::new(RefCell::new(None::<Location>));
            let opened_for = opened.clone();
            fixture.view.browser().observe(move |event| {
                if let BrowserEvent::OpenRequested { location } = event {
                    opened_for.replace(Some(location.clone()));
                }
            });
            submit_prompt(&fixture, Key::s, "unique-hit");
            wait_until(|| rendered_name(&fixture.view.widget(), "unique-hit.txt"));
            fixture.view.focus_first_search_result();
            assert!(fixture.press(Key::l, ModifierType::empty()));
            wait_until(|| fixture.preview.is_enabled());
            wait_until(|| rendered_name(&fixture.preview.widget(), "unique-hit.txt"));
            assert!(
                opened.borrow().is_none(),
                "l must not open a search-hit file"
            );
            assert!(fixture.press(Key::l, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(80));
            assert!(
                fixture.preview.is_enabled(),
                "a second l must not close a search-hit preview"
            );
            assert!(opened.borrow().is_none());
            assert!(fixture.press(Key::h, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(80));
            assert!(
                fixture.preview.is_enabled(),
                "h must return to the hits without closing the preview"
            );
            assert!(
                fixture.view.selected_search_results().is_some(),
                "h from the preview must not dismiss search"
            );
            fixture.view.dismiss_hidden_filter();
            opened.replace(None);
            fixture.view.set_view_mode(BrowserMode::Icons);
            wait_until(|| rendered_name(&fixture.view.widget(), "a.txt"));
            pump_mainloop(Duration::from_millis(100));
            submit_prompt(&fixture, Key::s, "unique-hit");
            wait_until(|| rendered_name(&fixture.view.widget(), "unique-hit.txt"));
            fixture.view.focus_first_search_result();
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| opened.borrow().as_ref() == Some(&Location::local(&nested)));
            fixture.view.dismiss_hidden_filter();
            submit_prompt(&fixture, Key::s, "innerdir");
            wait_until(|| rendered_name(&fixture.view.widget(), "innerdir"));
            fixture.view.focus_first_search_result();
            assert!(fixture.press(Key::l, ModifierType::empty()));
            wait_until(|| {
                fixture.view.browser().active_location()
                    == Some(Location::local(fixture._directory.path().join("innerdir")))
            });
        },
    );
}

#[test]
fn minimal_search_l_previews_a_file_in_list() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_search_l_previews_a_file_in_list",
        || {
            let fixture = MinimalFixture::new();
            fixture.footer.observe_browser(&fixture.view);
            let nested = add_nested_hit(&fixture);
            PreferenceManager::shared().set_filter_include_subfolders(false);
            fixture.view.set_view_mode(BrowserMode::List);
            wait_until(|| rendered_name(&fixture.view.widget(), "a.txt"));
            pump_mainloop(Duration::from_millis(80));
            let opened = Rc::new(RefCell::new(None::<Location>));
            let opened_for = opened.clone();
            fixture.view.browser().observe(move |event| {
                if let BrowserEvent::OpenRequested { location } = event {
                    opened_for.replace(Some(location.clone()));
                }
            });
            submit_prompt(&fixture, Key::s, "unique-hit");
            wait_until(|| {
                rendered_name(&fixture.view.widget(), "unique-hit.txt")
                    && fixture.view.selected_search_results().is_some()
            });
            pump_mainloop(Duration::from_millis(80));
            assert!(fixture.press(Key::l, ModifierType::empty()));
            wait_until(|| fixture.preview.is_enabled());
            wait_until(|| rendered_name(&fixture.preview.widget(), "unique-hit.txt"));
            assert!(
                opened.borrow().is_none(),
                "l must not open a search-hit file"
            );
            assert!(fixture.press(Key::l, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(80));
            assert!(
                fixture.preview.is_enabled(),
                "a second l must not close a search-hit preview"
            );
            assert!(fixture.press(Key::h, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(80));
            assert!(
                fixture.preview.is_enabled(),
                "h must return to the hits without closing the preview"
            );
            assert!(
                fixture.view.selected_search_results().is_some(),
                "h from the preview must not dismiss search"
            );
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| opened.borrow().as_ref() == Some(&Location::local(&nested)));
            fixture.view.dismiss_hidden_filter();
            wait_until(|| fixture.view.selected_search_results().is_none());
            submit_prompt(&fixture, Key::s, "innerdir");
            wait_until(|| rendered_name(&fixture.view.widget(), "innerdir"));
            pump_mainloop(Duration::from_millis(80));
            fixture.view.focus_first_search_result();
            assert!(fixture.press(Key::l, ModifierType::empty()));
            wait_until(|| {
                fixture.view.browser().active_location()
                    == Some(Location::local(fixture._directory.path().join("innerdir")))
            });
        },
    );
}

#[test]
fn minimal_listing_l_previews_a_file_in_list() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_listing_l_previews_a_file_in_list",
        || {
            let fixture = MinimalFixture::new();
            fixture.view.set_view_mode(BrowserMode::List);
            wait_until(|| rendered_name(&fixture.view.widget(), "a.txt"));
            fixture.select_name("a.txt");
            let path = fixture._directory.path().join("a.txt");
            let opened = Rc::new(RefCell::new(None::<Location>));
            let opened_for = opened.clone();
            fixture.view.browser().observe(move |event| {
                if let BrowserEvent::OpenRequested { location } = event {
                    opened_for.replace(Some(location.clone()));
                }
            });
            assert!(fixture.press(Key::l, ModifierType::empty()));
            wait_until(|| fixture.preview.is_enabled());
            wait_until(|| rendered_name(&fixture.preview.widget(), "a.txt"));
            wait_until(|| !fixture.view.item_view_has_focus());
            assert!(
                opened.borrow().is_none(),
                "l must not open the focused file"
            );
            assert!(fixture.press(Key::l, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(80));
            assert!(
                fixture.preview.is_enabled(),
                "a second l must not close the preview"
            );
            assert!(opened.borrow().is_none());
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| opened.borrow().as_ref() == Some(&Location::local(&path)));
        },
    );
}

#[test]
fn minimal_search_i_previews_the_hit_and_space_does_not() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_search_i_previews_the_hit_and_space_does_not",
        || {
            let fixture = MinimalFixture::new();
            add_nested_hit(&fixture);
            PreferenceManager::shared().set_filter_include_subfolders(false);
            fixture.select_name("a.txt");
            submit_prompt(&fixture, Key::s, "unique-hit");
            wait_until(|| rendered_name(&fixture.view.widget(), "unique-hit.txt"));
            fixture.view.focus_first_search_result();
            assert!(fixture.press(Key::space, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(100));
            assert!(
                !fixture.preview.is_enabled(),
                "Space must not preview a search hit"
            );
            assert!(fixture.press(Key::i, ModifierType::empty()));
            wait_until(|| fixture.preview.is_enabled());
            wait_until(|| rendered_name(&fixture.preview.widget(), "unique-hit.txt"));
        },
    );
}

#[test]
fn minimal_search_right_previews_the_hit_and_does_not_open() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_search_right_previews_the_hit_and_does_not_open",
        || {
            let fixture = MinimalFixture::new();
            let nested = add_nested_hit(&fixture);
            PreferenceManager::shared().set_filter_include_subfolders(false);
            let opened = Rc::new(RefCell::new(None::<Location>));
            let opened_for = opened.clone();
            fixture.view.browser().observe(move |event| {
                if let BrowserEvent::OpenRequested { location } = event {
                    opened_for.replace(Some(location.clone()));
                }
            });
            submit_prompt(&fixture, Key::s, "unique-hit");
            wait_until(|| rendered_name(&fixture.view.widget(), "unique-hit.txt"));
            fixture.view.focus_first_search_result();
            assert!(fixture.press(Key::Right, ModifierType::empty()));
            wait_until(|| fixture.preview.is_enabled());
            wait_until(|| rendered_name(&fixture.preview.widget(), "unique-hit.txt"));
            pump_mainloop(Duration::from_millis(80));
            assert!(
                opened.borrow().is_none(),
                "Right must not open a search-hit file"
            );
            assert_ne!(
                fixture.view.browser().active_location(),
                Some(Location::local(fixture._directory.path().join("innerdir"))),
                "Right must not enter a file hit's parent directory"
            );
            assert!(fixture.press(Key::l, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(80));
            assert!(
                fixture.preview.is_enabled(),
                "l must not close a search-hit preview"
            );
            assert!(opened.borrow().is_none());
            assert!(fixture.press(Key::h, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(80));
            assert!(
                fixture.preview.is_enabled(),
                "h from the preview must leave keys on the hits without closing it"
            );
            assert!(
                fixture.view.selected_search_results().is_some(),
                "h from the preview must not dismiss search"
            );
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| opened.borrow().as_ref() == Some(&Location::local(&nested)));
        },
    );
}

#[test]
fn minimal_search_right_opens_a_directory_hit() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_search_right_opens_a_directory_hit",
        || {
            let fixture = MinimalFixture::new();
            add_nested_hit(&fixture);
            PreferenceManager::shared().set_filter_include_subfolders(false);
            let opened = Rc::new(RefCell::new(None::<Location>));
            let opened_for = opened.clone();
            fixture.view.browser().observe(move |event| {
                if let BrowserEvent::OpenRequested { location } = event {
                    opened_for.replace(Some(location.clone()));
                }
            });
            submit_prompt(&fixture, Key::s, "innerdir");
            wait_until(|| rendered_name(&fixture.view.widget(), "innerdir"));
            fixture.view.focus_first_search_result();
            assert!(fixture.press(Key::Right, ModifierType::empty()));
            wait_until(|| {
                fixture.view.browser().active_location()
                    == Some(Location::local(fixture._directory.path().join("innerdir")))
            });
            assert!(
                opened.borrow().is_none(),
                "Right must navigate a directory hit instead of opening it as a file"
            );
            assert!(!fixture.preview.is_enabled());
        },
    );
}

#[test]
fn minimal_alt_up_closes_the_miller_child() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_alt_up_closes_the_miller_child",
        || {
            let fixture = MinimalFixture::new();
            fixture.select_name("sub");
            assert!(fixture.press(Key::l, ModifierType::empty()));
            wait_until(|| fixture.view.browser().column_snapshot(1).is_some());
            assert!(fixture.press(Key::Up, ModifierType::ALT_MASK));
            wait_until(|| fixture.view.browser().column_snapshot(1).is_none());
            assert_eq!(
                fixture.view.browser().active_location(),
                Some(Location::local(fixture._directory.path()))
            );
        },
    );
}

#[test]
fn minimal_g_rebuild_cancels_the_chord_so_h_does_not_go_home() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_g_rebuild_cancels_the_chord_so_h_does_not_go_home",
        || {
            let fixture = MinimalFixture::new();
            let home = fixture.view.browser().active_location();
            assert!(fixture.press(Key::g, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "g-"));
            fixture.sidebar.state.pin_location(
                Location::local(fixture._directory.path().join("sub")),
                "sub".into(),
            );
            pump_mainloop(Duration::from_millis(50));
            assert!(fixture.press(Key::h, ModifierType::empty()));
            assert_ne!(
                fixture.view.browser().active_location(),
                Some(Location::local(home_directory())),
                "a rebuilt sidebar must cancel the pending g chord"
            );
            assert_eq!(
                fixture.view.browser().active_location().is_some(),
                home.is_some()
            );
        },
    );
}

#[test]
fn minimal_visual_ctrl_f_keeps_a_range() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_visual_ctrl_f_keeps_a_range",
        || {
            let fixture = MinimalFixture::new();
            for name in [
                "d.txt", "e.txt", "f.txt", "g.txt", "h.txt", "i.txt", "j.txt",
            ] {
                std::fs::write(fixture._directory.path().join(name), b"page").expect("extra file");
            }
            fixture
                .view
                .browser()
                .navigate(Location::local(fixture._directory.path()));
            wait_until(|| {
                fixture
                    .view
                    .browser()
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading && column.count >= 10)
            });
            fixture.select_name("a.txt");
            let start = fixture.cursor().expect("cursor on a.txt");
            assert!(fixture.press(Key::v, ModifierType::empty()));
            assert!(fixture.press(Key::f, ModifierType::CONTROL_MASK));
            let fill = fixture.fill();
            assert!(
                fill.len() > 1,
                "visual Ctrl+f must keep a range, not a single row: {fill:?}"
            );
            assert_eq!(fill[0], start);
        },
    );
}

#[test]
fn minimal_ctrl_comma_is_stopped() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_ctrl_comma_is_stopped",
        || {
            let fixture = MinimalFixture::new();
            assert!(
                fixture.press(Key::comma, ModifierType::CONTROL_MASK),
                "capture must Stop Ctrl+,"
            );
        },
    );
}

#[test]
fn minimal_filter_visual_and_multiselect_use_visible_matches() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_filter_visual_and_multiselect_use_visible_matches",
        || {
            let fixture = MinimalFixture::new();
            submit_prompt(&fixture, Key::f, "txt");
            wait_txt_overlay(&fixture, "applied f filter showing txt matches");
            let visible = overlay_listing_names(&fixture);
            assert!(
                visible.len() >= 2,
                "filter should leave at least two visible matches, got {visible:?}"
            );
            assert!(
                !visible.iter().any(|name| name == "sub"),
                "hidden-by-filter sub must not be a visible match: {visible:?}"
            );

            let start = yank_names(&fixture);
            assert!(
                !start.is_empty() && start.iter().all(|name| visible.contains(name)),
                "filter cursor should start on a visible match, got {start:?}"
            );
            assert!(fixture.press(Key::r, ModifierType::CONTROL_MASK));
            pump_mainloop(Duration::from_millis(50));
            let inverted = yank_names(&fixture);
            let mut expected: Vec<String> = visible
                .iter()
                .filter(|name| !start.contains(name))
                .cloned()
                .collect();
            expected.sort();
            let mut inverted_sorted = inverted.clone();
            inverted_sorted.sort();
            assert_eq!(
                inverted_sorted, expected,
                "Ctrl+R must invert among visible matches only, start {start:?} visible {visible:?}"
            );
            assert!(
                !inverted.iter().any(|name| name == "sub"),
                "invert must not include hidden-by-filter rows"
            );
            assert!(fixture.press(Key::j, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            let mut kept_invert = yank_names(&fixture);
            kept_invert.sort();
            assert_eq!(
                kept_invert, expected,
                "Browse j after invert keeps the match fill"
            );

            let before = yank_names(&fixture);
            assert!(fixture.press(Key::v, ModifierType::empty()));
            assert!(fixture.press(Key::j, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            let range = yank_names(&fixture);
            assert!(
                range.len() >= 2,
                "v then j must fill a range of visible matches, got {range:?} from {before:?}"
            );
            assert!(
                range.iter().all(|name| visible.contains(name)),
                "visual fill must stay on visible matches, got {range:?} visible {visible:?}"
            );
            assert!(
                !range.iter().any(|name| name == "sub"),
                "visual fill must not include hidden-by-filter rows: {range:?}"
            );
            assert!(
                fixture.view.selected_search_results().is_some(),
                "filter visual fill is the overlay set yank uses"
            );

            assert!(fixture.press(Key::v, ModifierType::empty()));
            assert_eq!(
                yank_names(&fixture),
                range,
                "second v leaves visual and keeps the match fill"
            );
            assert!(fixture.press(Key::j, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            assert_eq!(
                yank_names(&fixture),
                range,
                "Browse j after visual keeps the match fill"
            );

            assert!(fixture.press(Key::space, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            let after_space = yank_names(&fixture);
            assert!(
                after_space.iter().all(|name| visible.contains(name)),
                "Space must toggle visible matches only, got {after_space:?}"
            );
            assert!(
                !after_space.iter().any(|name| name == "sub"),
                "Space must not add hidden-by-filter rows"
            );
            assert!(
                !fixture.preview.is_enabled(),
                "Space must not preview a filter row"
            );

            assert!(fixture.press(Key::a, ModifierType::CONTROL_MASK));
            pump_mainloop(Duration::from_millis(50));
            let all = yank_names(&fixture);
            let mut all_sorted = all.clone();
            all_sorted.sort();
            let mut visible_sorted = visible.clone();
            visible_sorted.sort();
            assert_eq!(
                all_sorted, visible_sorted,
                "Ctrl+A must select every visible match and none of the hidden-by-filter rows"
            );
            assert!(
                !all.iter().any(|name| name == "sub"),
                "Ctrl+A must not include hidden-by-filter rows"
            );
            assert!(fixture.press(Key::j, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            assert_eq!(
                yank_names(&fixture),
                all,
                "Browse j after Ctrl+A keeps the match fill"
            );
        },
    );
}

#[test]
fn minimal_search_visual_and_multiselect_use_hits() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_search_visual_and_multiselect_use_hits",
        || {
            let fixture = MinimalFixture::new();
            submit_prompt(&fixture, Key::s, "txt");
            wait_txt_overlay(&fixture, "kept s hits showing txt matches");
            assert!(
                fixture.view.force_recursive_search(),
                "s must be recursive search, not a listing filter"
            );
            let hits = overlay_listing_names(&fixture);
            assert!(
                hits.len() >= 2,
                "s should leave at least two hits, got {hits:?}"
            );

            let before = yank_names(&fixture);
            assert!(fixture.press(Key::v, ModifierType::empty()));
            assert!(fixture.press(Key::j, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            let range = yank_names(&fixture);
            assert!(
                range.len() >= 2,
                "v then j must fill a range of search hits, got {range:?} from {before:?}"
            );
            assert!(
                range.iter().all(|name| hits.contains(name)),
                "visual fill must be hits, got {range:?} hits {hits:?}"
            );
            let overlay = fixture
                .view
                .selected_search_results()
                .expect("search visual fill is selected_search_results");
            assert_eq!(
                overlay.len(),
                range.len(),
                "yank names must be the hit fill, not leftover listing names {}",
                listing_selected_names(&fixture).join(",")
            );

            assert!(fixture.press(Key::v, ModifierType::empty()));
            assert_eq!(
                yank_names(&fixture),
                range,
                "second v leaves visual and keeps the hit fill"
            );
            assert!(fixture.press(Key::j, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            assert_eq!(
                yank_names(&fixture),
                range,
                "Browse j after visual keeps the hit fill"
            );

            assert!(fixture.press(Key::V, ModifierType::empty()));
            assert!(fixture.press(Key::k, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            let after_unset = yank_names(&fixture);
            assert!(
                after_unset.len() < range.len(),
                "V then k must subtract the walked span of hits, got {after_unset:?} from {range:?}"
            );
            assert!(
                after_unset.iter().all(|name| hits.contains(name)),
                "unset fill must stay on hits"
            );
            assert!(fixture.press(Key::V, ModifierType::empty()));

            assert!(fixture.press(Key::space, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            let after_space = yank_names(&fixture);
            assert!(
                after_space.iter().all(|name| hits.contains(name)),
                "Space must toggle hits, got {after_space:?}"
            );
            assert!(
                !fixture.preview.is_enabled(),
                "Space must not preview a search hit"
            );
            assert!(
                !listing_selected_names(&fixture)
                    .iter()
                    .any(|name| name == "sub"),
                "Space must not add the hidden directory's sub to the listing fill"
            );

            assert!(fixture.press(Key::a, ModifierType::CONTROL_MASK));
            pump_mainloop(Duration::from_millis(50));
            let all = yank_names(&fixture);
            let mut all_sorted = all.clone();
            all_sorted.sort();
            let mut hits_sorted = hits.clone();
            hits_sorted.sort();
            assert_eq!(
                all_sorted, hits_sorted,
                "Ctrl+A must select all current hits, not the hidden directory listing"
            );
            assert_eq!(
                fixture
                    .view
                    .selected_search_results()
                    .expect("Ctrl+A fill is selected_search_results")
                    .len(),
                hits.len()
            );
            assert!(fixture.press(Key::j, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            assert_eq!(
                yank_names(&fixture),
                all,
                "Browse j after Ctrl+A keeps the hit fill"
            );
        },
    );
}

#[test]
fn minimal_icons_filter_and_list_search_visual_fill() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_icons_filter_and_list_search_visual_fill",
        || {
            let fixture = MinimalFixture::new();
            fixture.view.set_view_mode(BrowserMode::Icons);
            wait_until_msg(
                || rendered_name(&fixture.view.widget(), "a.txt"),
                "Icons listing did not show a.txt",
            );
            pump_mainloop(Duration::from_millis(80));
            submit_prompt(&fixture, Key::f, "txt");
            wait_txt_overlay(&fixture, "Icons filter showing txt matches");
            let visible = overlay_listing_names(&fixture);
            assert!(visible.len() >= 2, "Icons filter matches: {visible:?}");
            assert!(fixture.press(Key::v, ModifierType::empty()));
            assert!(fixture.press(Key::j, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            let filtered = yank_names(&fixture);
            assert!(
                filtered.len() >= 2,
                "Icons filter v then j must fill visible matches, got {filtered:?}"
            );
            assert!(
                filtered.iter().all(|name| visible.contains(name)),
                "Icons filter fill must stay on matches, got {filtered:?}"
            );
            assert!(
                !filtered.iter().any(|name| name == "sub"),
                "Icons filter fill must not include hidden-by-filter rows"
            );
            assert!(fixture.press(Key::a, ModifierType::CONTROL_MASK));
            pump_mainloop(Duration::from_millis(50));
            let mut icons_all = yank_names(&fixture);
            icons_all.sort();
            let mut icons_visible = visible.clone();
            icons_visible.sort();
            assert_eq!(
                icons_all, icons_visible,
                "Icons filter Ctrl+A must select every visible match"
            );

            fixture.view.dismiss_hidden_filter();
            wait_until(|| fixture.view.selected_search_results().is_none());
            fixture.view.set_view_mode(BrowserMode::List);
            wait_until_msg(
                || rendered_name(&fixture.view.widget(), "a.txt"),
                "List listing did not show a.txt",
            );
            pump_mainloop(Duration::from_millis(80));
            submit_prompt(&fixture, Key::s, "txt");
            wait_txt_overlay(&fixture, "List search showing txt hits");
            assert!(fixture.view.force_recursive_search());
            let hits = overlay_listing_names(&fixture);
            assert!(hits.len() >= 2, "List search hits: {hits:?}");
            assert!(fixture.press(Key::v, ModifierType::empty()));
            assert!(fixture.press(Key::j, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            let range = yank_names(&fixture);
            assert!(
                range.len() >= 2,
                "List search v then j must fill hits, got {range:?}"
            );
            assert!(range.iter().all(|name| hits.contains(name)));
            assert!(fixture.press(Key::a, ModifierType::CONTROL_MASK));
            pump_mainloop(Duration::from_millis(50));
            let mut list_all = yank_names(&fixture);
            list_all.sort();
            let mut list_hits = hits.clone();
            list_hits.sort();
            assert_eq!(
                list_all, list_hits,
                "List search Ctrl+A must select all hits, not the hidden directory"
            );
            assert_eq!(
                fixture
                    .view
                    .selected_search_results()
                    .map(|entries| entries.len()),
                Some(hits.len())
            );
        },
    );
}
