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
        wait_until_msg(
            || {
                view.browser()
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading && column.count == 4)
            },
            "initial fixture listing loads a.txt, b.txt, c.txt and sub",
        );
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
        wait_until_msg(
            || fixture.view.item_view_has_focus(),
            "initial a.txt cursor receives file focus",
        );
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
    let names = ["a.txt", "b.txt", "c.txt"];
    names.iter().all(|name| {
        !rendered_name(fixture.view.widget().upcast_ref(), name)
            || highlighted_name(fixture, name).as_deref() == Some("txt")
    }) && names
        .iter()
        .any(|name| highlighted_name(fixture, name).as_deref() == Some("txt"))
        && highlighted_name(fixture, "sub").is_none()
}

fn no_find_highlights(fixture: &MinimalFixture) -> bool {
    highlighted_name(fixture, "a.txt").is_none()
        && highlighted_name(fixture, "b.txt").is_none()
        && highlighted_name(fixture, "c.txt").is_none()
        && highlighted_name(fixture, "sub").is_none()
}

fn search_txt_hits_visible(fixture: &MinimalFixture) -> bool {
    let listing = overlay_listing_names(fixture);
    let retained = ["a.txt", "b.txt", "c.txt"]
        .iter()
        .all(|name| listing.iter().any(|hit| hit == name));
    // Icon grids only build widgets for the viewport, so a retained hit can
    // exist before its card is instantiated.
    fixture.view.selected_search_results().is_some()
        && retained
        && !listing.iter().any(|name| name == "sub")
        && !rendered_name(&fixture.view.widget(), "sub")
        && ["a.txt", "b.txt", "c.txt"]
            .iter()
            .any(|name| rendered_name(&fixture.view.widget(), name))
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
        && let Some(slice) = crate::ui::browser::find_highlight::tests::highlighted_slice(
            label.text().as_str(),
            label.attributes().as_ref(),
        )
    {
        return Some(slice);
    }
    if let Some(label) = widget.downcast_ref::<gtk::Inscription>()
        && label.text().as_deref() == Some(name)
        && let Some(slice) = crate::ui::browser::find_highlight::tests::highlighted_slice(
            name,
            label.attributes().as_ref(),
        )
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

fn widget_contains_focus(widget: &gtk::Widget) -> bool {
    use gtk::prelude::{RootExt, WidgetExt};

    widget
        .root()
        .and_then(|root| root.focus())
        .is_some_and(|focus| focus == widget.clone() || widget.is_ancestor(&focus))
}

fn visible_dialog_confirm(widget: &gtk::Widget) -> Option<gtk::Button> {
    visible_widget_with_class(widget, "action-dialog-confirm")
        .and_then(|widget| widget.downcast().ok())
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

#[track_caller]
fn wait_until(condition: impl Fn() -> bool) {
    wait_until_msg(
        condition,
        &format!("condition at {}", std::panic::Location::caller()),
    );
}

fn wait_until_msg(condition: impl Fn() -> bool, msg: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "{msg}; focus={:?}",
            gtk::Window::list_toplevels()
                .iter()
                .filter_map(|widget| widget.downcast_ref::<gtk::Window>())
                .map(gtk::prelude::RootExt::focus)
                .collect::<Vec<_>>()
        );
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

fn install_text_activation_handler() {
    // The real browser observer launches files; isolate it from host handlers
    // and missing-handler chooser fallbacks.
    let applications = glib::user_data_dir().join("applications");
    std::fs::create_dir_all(&applications).expect("fixture applications directory");
    std::fs::write(
        applications.join("strata-minimal-preview.desktop"),
        "[Desktop Entry]\nType=Application\nName=Minimal preview fixture\nExec=/bin/true %U\nMimeType=text/plain;\n",
    ).expect("isolated text handler");
    std::fs::create_dir_all(glib::user_config_dir()).expect("fixture configuration directory");
    std::fs::write(
        glib::user_config_dir().join("mimeapps.list"),
        "[Default Applications]\ntext/plain=strata-minimal-preview.desktop;\n",
    )
    .expect("fixture text association");
}

#[test]
fn minimal_right_and_l_preview_a_file() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_right_and_l_preview_a_file",
        || {
            install_text_activation_handler();
            for mode in [BrowserMode::Columns, BrowserMode::List] {
                let fixture = find_highlight_fixture(mode);
                let cursor = fixture.cursor();
                let location = fixture.view.browser().active_location();
                let opened = Rc::new(RefCell::new(Vec::<Location>::new()));
                let observed = opened.clone();
                fixture.view.browser().observe(move |event| {
                    if let BrowserEvent::OpenRequested { location } = event {
                        observed.borrow_mut().push(location.clone());
                    }
                });
                for key in [Key::Right, Key::l, Key::KP_Right] {
                    assert!(fixture.press(key, ModifierType::empty()));
                    wait_until_msg(
                        || {
                            fixture.preview.is_enabled()
                                && rendered_name(&fixture.preview.widget(), "a.txt")
                                && !fixture.view.item_view_has_focus()
                                && fixture.preview.owns_keys_chrome()
                                && !miller_column_header_focus(&fixture)
                        },
                        &format!("{mode:?} {key:?} transfers keys to the a.txt preview"),
                    );
                    assert!(opened.borrow().is_empty());
                    assert_eq!(fixture.cursor(), cursor);
                    assert_eq!(fixture.view.browser().active_location(), location);
                    assert!(fixture.press(key, ModifierType::empty()));
                    assert!(
                        fixture.preview.is_enabled(),
                        "repeating {key:?} must not close preview"
                    );
                    assert!(fixture.preview.owns_keys_chrome());
                    assert!(!fixture.view.item_view_has_focus());
                    assert!(!miller_column_header_focus(&fixture));
                    assert!(opened.borrow().is_empty());
                    assert_eq!(fixture.cursor(), cursor);
                    if key == Key::l {
                        assert!(fixture.press(Key::Return, ModifierType::empty()));
                        assert_eq!(
                            *opened.borrow(),
                            [Location::local(fixture._directory.path().join("a.txt"))],
                            "Enter activates the listing file while preview owns keys"
                        );
                        opened.borrow_mut().clear();
                    }
                    assert!(fixture.press(Key::h, ModifierType::empty()));
                    wait_until_msg(
                        || listing_cursor_has_focus(&fixture),
                        "h restores the listing cursor without closing preview",
                    );
                    assert!(fixture.preview.is_enabled());
                    assert_eq!(fixture.view.browser().active_location(), location);
                    assert!(fixture.press(Key::i, ModifierType::empty()));
                    assert!(!fixture.preview.is_enabled());
                }
                for key in [Key::Return, Key::o] {
                    assert!(fixture.press(key, ModifierType::empty()));
                    assert_eq!(
                        *opened.borrow(),
                        [Location::local(fixture._directory.path().join("a.txt"))],
                        "{mode:?} {key:?} activates a.txt exactly once"
                    );
                    opened.borrow_mut().clear();
                }
                assert!(!fixture.preview.is_enabled());
                assert_eq!(fixture.view.browser().active_location(), location);
            }
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
            let pane = fixture
                .preview
                .widget()
                .first_child()
                .expect("preview pane");
            let prior_focusability = (pane.can_focus(), pane.is_focusable());
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
            assert_eq!((pane.can_focus(), pane.is_focusable()), prior_focusability);
            wait_until(|| miller_column_header_focus(&fixture));
            assert_eq!(fixture.cursor(), cursor);
            assert_eq!(
                fixture.view.browser().active_location(),
                location,
                "h from the preview must not navigate to the parent"
            );

            assert!(fixture.press(Key::l, ModifierType::empty()));
            wait_until(|| fixture.preview.owns_keys_chrome());
            fixture.preview.action().activate(None);
            assert!(!fixture.preview.is_enabled());
            assert_eq!((pane.can_focus(), pane.is_focusable()), prior_focusability);
            fixture.view.restore_file_view_focus();
            wait_until(|| fixture.view.item_view_has_focus());
            assert!(fixture.press(Key::l, ModifierType::empty()));
            wait_until(|| fixture.preview.owns_keys_chrome());
            PreferenceManager::shared().set_minimal_mode(false);
            wait_until(|| fixture.view.item_view_has_focus());
            assert!(fixture.preview.is_enabled());
            assert!(!fixture.preview.owns_keys_chrome());
            assert_eq!((pane.can_focus(), pane.is_focusable()), prior_focusability);
            assert_ne!(focused_widget(&fixture), Some(pane.clone()));
            assert!(
                !pane.is_focusable(),
                "default Tab traversal must not stop on the preview container"
            );
            fixture.window.child_focus(gtk::DirectionType::TabForward);
            assert_ne!(focused_widget(&fixture), Some(pane));
            PreferenceManager::shared().set_minimal_mode(true);
            fixture.view.restore_file_view_focus();
            wait_until(|| fixture.view.item_view_has_focus());
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
                wait_until_msg(
                    || {
                        fixture.view.browser().column_snapshot(1).is_none()
                            && fixture.view.browser().active_location()
                                == Some(Location::local(fixture._directory.path()))
                    },
                    "h closes the Miller child and restores the exact parent",
                );
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
            let unpreviewable = fixture._directory.path().join("compressed.gz");
            std::fs::write(&unpreviewable, b"\x1f\x8b").expect("gzip stream");
            fixture
                .view
                .browser()
                .navigate(Location::local(fixture._directory.path()));
            wait_until(|| rendered_name(&fixture.view.widget(), "compressed.gz"));
            fixture.select_name("compressed.gz");
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
            let closed = Rc::new(std::cell::Cell::new(0));
            let observed = closed.clone();
            fixture.window.connect_close_request(move |_| {
                observed.set(observed.get() + 1);
                glib::Propagation::Proceed
            });
            assert!(fixture.press(Key::q, ModifierType::empty()));
            assert!(!PreferenceManager::shared().minimal_mode());
            wait_until(|| footer_shows(&fixture, "Left minimal mode — Ctrl+Shift+M returns"));
            assert!(fixture.window.is_mapped(), "q leaves the window open");
            assert_eq!(closed.get(), 0);
            PreferenceManager::shared().set_minimal_mode(true);
            assert!(fixture.press(Key::Q, ModifierType::empty()));
            wait_until_msg(
                || closed.get() == 1 && !fixture.window.is_visible(),
                "Q dispatches one close request and closes the window",
            );
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
            for key in [Key::m, Key::M] {
                fixture.view.begin_location_edit();
                let focus = focused_widget(&fixture).expect("location focus");
                let text = focus.downcast_ref::<gtk::Text>().expect("editing location");
                let content = text.text();
                text.select_region(1, 3);
                assert!(fixture.press(key, toggle));
                assert!(!PreferenceManager::shared().minimal_mode());
                assert_eq!(text.text(), content);
                fixture.view.begin_location_edit();
                assert!(fixture.press(key, toggle));
                assert!(PreferenceManager::shared().minimal_mode());
                assert_eq!(text.text(), content);
            }
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
fn minimal_icons_parent_is_backspace_not_h() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_icons_parent_is_backspace_not_h",
        || {
            let fixture = MinimalFixture::new();
            fixture.footer.observe_browser(&fixture.view);
            fixture.view.set_view_mode(BrowserMode::Icons);
            wait_until(|| rendered_name(&fixture.view.widget(), "sub"));
            fixture.select_name("sub");
            assert_eq!(fixture.cursor_name().as_deref(), Some("sub"));
            let root = Location::local(fixture._directory.path());
            let child = Location::local(fixture._directory.path().join("sub"));
            assert!(fixture.press(Key::l, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(80));
            assert_eq!(
                fixture.view.browser().active_location(),
                Some(root.clone()),
                "Icons l must not enter a folder"
            );
            assert!(!fixture.preview.is_enabled());
            assert!(!fixture.preview.owns_keys_chrome());
            fixture.select_name("sub");
            assert_eq!(fixture.cursor_name().as_deref(), Some("sub"));
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| {
                fixture.view.browser().active_location() == Some(child.clone())
                    && fixture
                        .view
                        .browser()
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading)
            });
            assert!(fixture.press(Key::h, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(80));
            assert_eq!(
                fixture.view.browser().active_location(),
                Some(child.clone()),
                "Icons h must not leave the current folder"
            );
            assert!(fixture.press(Key::BackSpace, ModifierType::empty()));
            wait_until(|| {
                fixture.view.browser().active_location() == Some(root.clone())
                    && fixture.cursor_name().as_deref() == Some("sub")
            });
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| fixture.view.browser().active_location() == Some(child.clone()));
            assert!(fixture.press(Key::Up, ModifierType::ALT_MASK));
            wait_until(|| {
                fixture.view.browser().active_location() == Some(root.clone())
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
            wait_until_msg(
                || {
                    rendered_name(&popover, "Move to Trash (with confirmation)")
                        && rendered_name(&popover, "Delete permanently (with confirmation)")
                },
                "the shared reference renders both destructive-action confirmation notes",
            );
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
            open_empty_icons_sub(&fixture);
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
fn minimal_ctrl_v_pastes_like_p_for_a_cursor_only_folder() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_ctrl_v_pastes_like_p_for_a_cursor_only_folder",
        || {
            let fixture = MinimalFixture::new();
            let root = fixture._directory.path().to_path_buf();
            std::fs::write(root.join("sub").join("from-p.txt"), b"p").expect("p source");
            std::fs::write(root.join("sub").join("from-ctrl.txt"), b"v").expect("ctrl source");

            yank_child_file(&fixture, "from-p.txt");
            focus_only_parent_folder(&fixture, "sub");
            let window = fixture.window.upcast_ref::<gtk::Widget>();
            assert!(fixture.press(Key::P, ModifierType::empty()));
            wait_until_msg(
                || root.join("from-p.txt").is_file() && modal_button(window, "Replace").is_none(),
                "P pastes a cursor-only folder into the listing",
            );

            yank_child_file(&fixture, "from-ctrl.txt");
            focus_only_parent_folder(&fixture, "sub");
            assert!(fixture.press(Key::v, ModifierType::CONTROL_MASK));
            wait_until_msg(
                || {
                    root.join("from-ctrl.txt").is_file()
                        && modal_button(window, "Replace").is_none()
                },
                "Ctrl+V pastes a cursor-only folder into the same listing as P",
            );
            assert!(
                root.join("from-p.txt").is_file() && root.join("from-ctrl.txt").is_file(),
                "P and Ctrl+V both land in the parent listing"
            );

            let clipboard = fixture.window.clipboard();
            let _ = clipboard.set_content(None::<&gtk::gdk::ContentProvider>);
            wait_until(|| {
                !clipboard
                    .formats()
                    .contains_type(gtk::gdk::FileList::static_type())
            });
            let before = std::fs::read_dir(&root)
                .expect("root")
                .filter_map(Result::ok)
                .count();
            assert!(fixture.press(Key::v, ModifierType::CONTROL_MASK));
            wait_until(|| footer_shows(&fixture, "Nothing to paste"));
            let after = std::fs::read_dir(&root)
                .expect("root")
                .filter_map(Result::ok)
                .count();
            assert_eq!(before, after, "empty Ctrl+V must not create files");
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
            let pins = tempfile::tempdir().expect("pins");
            let first = Location::local(pins.path().join("first"));
            let second = Location::local(pins.path().join("second"));
            for location in [&first, &second] {
                std::fs::create_dir(location.native_path().expect("local pin"))
                    .expect("pin folder");
            }
            save_pinned_places(&[
                (Location::local(home_directory()), "Home duplicate".into()),
                (second.clone(), "Second".into()),
                (first.clone(), "First".into()),
            ])
            .expect("stored standard place precedes visible pins");
            let fixture = MinimalFixture::new();
            assert_eq!(fixture.sidebar.state.pinned_places.borrow().len(), 3);
            for order in [[second.clone(), first.clone()], [first, second]] {
                for (key, location) in [Key::_1, Key::_2].into_iter().zip(&order) {
                    assert!(fixture.press(Key::g, ModifierType::empty()));
                    wait_until_msg(
                        || sidebar_keycap(&fixture, location).is_some(),
                        "visible PINNED row receives a digit",
                    );
                    assert_eq!(
                        sidebar_keycap(&fixture, location).as_deref(),
                        Some(if key == Key::_1 { "1" } else { "2" })
                    );
                    assert!(fixture.press(key, ModifierType::empty()));
                    wait_until_msg(
                        || {
                            fixture.view.browser().active_location().as_ref() == Some(location)
                                && listing_cursor_has_focus(&fixture)
                        },
                        "pin digit navigates in visible PINNED order",
                    );
                }
                fixture.sidebar.state.reorder_pinned_place(2, 1, false);
                wait_until_msg(
                    || !fixture.footer.chord_hints_visible(),
                    "pin reorder dismisses chord hints",
                );
            }
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
            let parent = fixture._directory.path().parent().expect("fixture parent");
            for (key, mark) in [(Key::g, "g-"), (Key::c, "c-")] {
                fixture
                    .view
                    .browser()
                    .navigate(Location::local(fixture._directory.path()));
                wait_until_msg(
                    || listing_has_name(&fixture, "a.txt"),
                    "root reload before chord teardown",
                );
                fixture.select_name("a.txt");
                wait_until_msg(
                    || listing_cursor_has_focus(&fixture),
                    "root cursor before chord teardown",
                );
                assert!(fixture.press(key, ModifierType::empty()));
                wait_until_msg(
                    || footer_shows(&fixture, mark) && fixture.footer.chord_hints_visible(),
                    &format!("{mark} hints are visible before disable"),
                );
                if key == Key::c {
                    assert_eq!(fixture.footer.chord_hint_labels(), ["c path", "n name"]);
                }
                PreferenceManager::shared().set_minimal_mode(false);
                assert!(!fixture.footer.chord_hints_visible());
                assert!(!footer_shows(&fixture, mark));
                assert_eq!(chord_hint_count(&fixture), 0);
                PreferenceManager::shared().set_minimal_mode(true);
                wait_until_msg(
                    || listing_cursor_has_focus(&fixture),
                    "reenabled listing cursor before h",
                );
                assert!(fixture.press(Key::h, ModifierType::empty()));
                assert_eq!(
                    fixture.view.browser().active_location(),
                    Some(Location::local(parent)),
                    "h after {mark} teardown is parent motion, not a stale chord"
                );
            }
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
fn minimal_icons_hjkl_and_arrows_move_spatially() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_icons_hjkl_and_arrows_move_spatially",
        || {
            install_text_activation_handler();
            let fixture = MinimalFixture::new();
            populate_icons_grid(&fixture);
            fixture.view.set_view_mode(BrowserMode::Icons);
            wait_until(|| rendered_name(&fixture.view.widget(), "a.txt"));
            pin_icons_grid(&fixture, 3);
            fixture.select_name("a.txt");
            wait_until(|| fixture.view.item_view_has_focus());
            pump_mainloop(Duration::from_millis(100));
            let start = fixture.cursor().expect("cursor after the rebuild");
            let location = fixture.view.browser().active_location();
            let opened = Rc::new(RefCell::new(Vec::<Location>::new()));
            let observed = opened.clone();
            fixture.view.browser().observe(move |event| {
                if let BrowserEvent::OpenRequested { location } = event {
                    observed.borrow_mut().push(location.clone());
                }
            });

            for key in [Key::Right, Key::l, Key::KP_Right] {
                fixture.select_name("a.txt");
                assert!(fixture.press(key, ModifierType::empty()));
                wait_until_msg(
                    || {
                        fixture.cursor_name().as_deref() == Some("b.txt")
                            && listing_cursor_has_focus(&fixture)
                    },
                    &format!("Icons {key:?} must move one tile right"),
                );
                assert_eq!(fixture.view.browser().active_location(), location);
                assert!(
                    !fixture.preview.is_enabled(),
                    "{key:?} must not open preview"
                );
                assert!(
                    !fixture.preview.owns_keys_chrome(),
                    "{key:?} must not take preview keys"
                );
                assert!(opened.borrow().is_empty());
            }

            for key in [Key::Down, Key::j, Key::KP_Down] {
                fixture.select_name("a.txt");
                assert!(fixture.press(key, ModifierType::empty()));
                wait_until_msg(
                    || {
                        fixture.cursor() != Some(start)
                            && fixture.cursor() != Some(start + 1)
                            && listing_cursor_has_focus(&fixture)
                    },
                    &format!("Icons {key:?} must move down a row, not next in listing order"),
                );
                assert_eq!(fixture.cursor_name().as_deref(), Some("d.txt"));
                assert_eq!(fixture.view.browser().active_location(), location);
                assert!(!fixture.preview.is_enabled());
                assert!(!fixture.preview.owns_keys_chrome());
            }

            for key in [Key::Left, Key::h, Key::KP_Left] {
                fixture.select_name("b.txt");
                assert!(fixture.press(key, ModifierType::empty()));
                wait_until_msg(
                    || fixture.cursor_name().as_deref() == Some("a.txt"),
                    &format!("Icons {key:?} must move one tile left"),
                );
                assert_eq!(fixture.view.browser().active_location(), location);
                assert!(!fixture.preview.is_enabled());
                assert!(!fixture.preview.owns_keys_chrome());
            }

            for key in [Key::Up, Key::k, Key::KP_Up] {
                fixture.select_name("d.txt");
                assert!(fixture.press(key, ModifierType::empty()));
                wait_until_msg(
                    || fixture.cursor_name().as_deref() == Some("a.txt"),
                    &format!("Icons {key:?} must move one tile up"),
                );
                assert_eq!(fixture.view.browser().active_location(), location);
            }

            fixture.select_name("a.txt");
            assert!(fixture.press(Key::i, ModifierType::empty()));
            wait_until(|| fixture.preview.is_enabled());
            assert!(
                !fixture.preview.owns_keys_chrome(),
                "Icons i must toggle preview without taking keys"
            );
            assert!(listing_cursor_has_focus(&fixture));
            let with_preview = fixture.cursor();
            assert!(fixture.press(Key::l, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            assert!(
                fixture.preview.is_enabled(),
                "l with an open preview must not close it"
            );
            assert!(!fixture.preview.owns_keys_chrome());
            assert!(listing_cursor_has_focus(&fixture));
            assert_eq!(fixture.view.browser().active_location(), location);
            assert_ne!(
                fixture.cursor(),
                with_preview,
                "l with preview open still moves among tiles"
            );
            assert!(fixture.press(Key::i, ModifierType::empty()));
            wait_until(|| !fixture.preview.is_enabled());
            assert!(listing_cursor_has_focus(&fixture));

            fixture.select_name("a.txt");
            for key in [Key::Return, Key::o] {
                assert!(fixture.press(key, ModifierType::empty()));
                assert_eq!(
                    *opened.borrow(),
                    [Location::local(fixture._directory.path().join("a.txt"))],
                    "Icons {key:?} activates a.txt exactly once"
                );
                opened.borrow_mut().clear();
            }
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

fn populate_icons_grid(fixture: &MinimalFixture) {
    for name in ["d.txt", "e.txt", "f.txt"] {
        std::fs::write(fixture._directory.path().join(name), b"preview").expect("grid file");
    }
    fixture.view.refresh();
    wait_until_msg(
        || {
            fixture
                .view
                .browser()
                .column_snapshot(0)
                .is_some_and(|column| !column.loading && column.count == 7)
        },
        "icons grid listing includes a–f.txt and sub",
    );
}

fn find_icons_grid(widget: &gtk::Widget) -> Option<gtk::GridView> {
    use gtk::prelude::*;
    if let Ok(icons) = widget.clone().downcast::<gtk::GridView>()
        && icons.is_mapped()
    {
        return Some(icons);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(icons) = find_icons_grid(&widget) {
            return Some(icons);
        }
        child = widget.next_sibling();
    }
    None
}

fn pin_icons_grid(fixture: &MinimalFixture, columns: u32) {
    wait_until_msg(
        || find_icons_grid(fixture.view.widget().upcast_ref()).is_some(),
        "Icons GridView is mapped",
    );
    let grid = find_icons_grid(fixture.view.widget().upcast_ref()).expect("icons grid");
    grid.set_min_columns(columns);
    grid.set_max_columns(columns);
    pump_mainloop(Duration::from_millis(80));
    grid.set_min_columns(columns);
    grid.set_max_columns(columns);
    pump_mainloop(Duration::from_millis(80));
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

fn sidebar_keycap(fixture: &MinimalFixture, location: &Location) -> Option<String> {
    let rows = fixture.sidebar.state.place_rows.borrow();
    let (_, row) = rows.iter().find(|(candidate, _)| candidate == location)?;
    visible_widget_with_class(row.upcast_ref(), "minimal-chord-hint")
        .and_downcast::<gtk::Label>()
        .map(|label| label.text().to_string())
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
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                let fixture = find_highlight_fixture(mode);
                assert!(fixture.press(Key::slash, ModifierType::empty()));
                wait_until(|| footer_has_label(&fixture, "/") && prompt_has_focus(&fixture));
                fixture.footer.prompt_entry_widget().set_text("txt");
                if mode != BrowserMode::Columns {
                    fixture.view.set_view_mode(BrowserMode::Columns);
                    wait_until(|| txt_find_highlights(&fixture));
                    fixture.view.set_view_mode(mode);
                    wait_until(|| rendered_name(&fixture.view.widget(), "a.txt"));
                }
                wait_until_msg(
                    || txt_find_highlights(&fixture),
                    &format!("/txt must highlight matches in {mode:?}"),
                );
                assert!(listing_shows_all_fixture_names(&fixture));
                assert!(fixture.press(Key::Return, ModifierType::empty()));
                wait_until(|| !footer_shows(&fixture, "/"));
                wait_until_msg(
                    || {
                        txt_find_highlights(&fixture)
                            && listing_shows_all_fixture_names(&fixture)
                            && listing_cursor_has_focus(&fixture)
                    },
                    &format!("submitted / must keep highlights and focus the {mode:?} listing"),
                );
                let after_submit = fixture.cursor();
                assert!(fixture.press(Key::n, ModifierType::empty()));
                wait_until_msg(
                    || fixture.cursor() != after_submit && txt_find_highlights(&fixture),
                    "n still jumps after a submitted find and keeps highlights",
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
fn minimal_search_results_persist_after_first_esc() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_search_results_persist_after_first_esc",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                let fixture = find_highlight_fixture(mode);
                fixture.footer.observe_browser(&fixture.view);
                for keep in [Key::Return, Key::Escape] {
                    query_prompt_with_results(
                        &fixture,
                        Key::s,
                        "txt",
                        &["a.txt", "b.txt", "c.txt"],
                    );
                    assert!(fixture.press(keep, ModifierType::empty()));
                    wait_for_search_cursor(
                        &fixture,
                        0,
                        &format!("{mode:?} {keep:?} retains txt results"),
                    );
                    wait_until_msg(
                        || search_txt_hits_visible(&fixture),
                        &format!("{mode:?} {keep:?} renders all retained txt hits"),
                    );
                    assert!(!footer_shows(&fixture, "search"));
                    wait_until_msg(
                        || footer_count_text(&fixture) == "3 items",
                        &format!("{mode:?} {keep:?} retains the three-hit footer total"),
                    );

                    assert!(fixture.press(Key::v, ModifierType::empty()));
                    assert!(fixture.press(Key::j, ModifierType::empty()));
                    wait_until_msg(
                        || {
                            fixture.view.search_hit_index() == Some(1)
                                && search_names(&fixture).len() == 2
                        },
                        "v/j selects two retained txt hits",
                    );
                    assert!(fixture.press(Key::Escape, ModifierType::empty()));
                    assert!(
                        search_txt_hits_visible(&fixture),
                        "visual Escape keeps hits"
                    );
                    assert!(fixture.press(Key::i, ModifierType::empty()));
                    wait_until_msg(
                        || fixture.preview.is_enabled(),
                        "i opens retained-hit preview",
                    );
                    assert!(fixture.press(Key::Escape, ModifierType::empty()));
                    wait_until_msg(
                        || !fixture.preview.is_enabled(),
                        "Escape closes preview before search",
                    );
                    assert!(search_txt_hits_visible(&fixture));
                    assert!(fixture.press(Key::Escape, ModifierType::empty()));
                    wait_until_msg(
                        || {
                            fixture.view.selected_search_results().is_none()
                                && rendered_name(&fixture.view.widget(), "sub")
                                && footer_count_text(&fixture) != "3 items"
                                && listing_cursor_has_focus(&fixture)
                        },
                        &format!("{mode:?} browse Escape restores the directory after {keep:?}"),
                    );
                }
            }
        },
    );
}

#[test]
fn minimal_empty_search_esc_cancels_without_results() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_empty_search_esc_cancels_without_results",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List] {
                let fixture = find_highlight_fixture(mode);
                assert!(fixture.press(Key::s, ModifierType::empty()));
                wait_until_msg(
                    || prompt_has_focus(&fixture),
                    "empty search prompt has focus",
                );
                assert!(fixture.press(Key::Escape, ModifierType::empty()));
                wait_until_msg(
                    || {
                        !prompt_has_focus(&fixture)
                            && fixture.view.selected_search_results().is_none()
                            && rendered_name(&fixture.view.widget(), "sub")
                    },
                    "empty s Escape cancels without results",
                );
                assert!(fixture.press(Key::s, ModifierType::empty()));
                wait_until_msg(
                    || prompt_has_focus(&fixture),
                    "reopened search prompt has focus",
                );
                type_find_char(&fixture, "t");
                wait_until_msg(
                    || search_txt_hits_visible(&fixture) && prompt_has_focus(&fixture),
                    "typing t after empty cancel publishes all three txt hits with prompt focus",
                );
                fixture.footer.prompt_entry_widget().set_text("");
                wait_until_msg(
                    || {
                        fixture.view.selected_search_results().is_none()
                            && rendered_name(&fixture.view.widget(), "sub")
                            && prompt_has_focus(&fixture)
                    },
                    "clearing live s restores the directory while keeping prompt focus",
                );
            }
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
fn minimal_filter_click_away_commits_a_query_and_clears_an_empty_one() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_filter_click_away_commits_a_query_and_clears_an_empty_one",
        || {
            let fixture = MinimalFixture::new();
            fixture.footer.observe_browser(&fixture.view);
            fixture
                .footer
                .bind_preferences(&PreferenceManager::shared());

            assert!(fixture.press(Key::f, ModifierType::empty()));
            wait_until_msg(
                || prompt_has_focus(&fixture),
                "f prompt is focused before the query",
            );
            fixture.footer.prompt_entry_widget().set_text("a.txt");
            wait_until_msg(
                || {
                    fixture.view.hidden_filter_query().trim() == "a.txt"
                        && rendered_name(&fixture.view.widget(), "a.txt")
                        && !rendered_name(&fixture.view.widget(), "c.txt")
                },
                "live f query filters before Enter",
            );
            fixture.select_name("a.txt");
            wait_filter_kept(&fixture, "a.txt", "click-away commits the visible filter");
            assert!(!prompt_has_focus(&fixture));

            dismiss_search_over_filter(&fixture, "txt");
            wait_filter_kept(
                &fixture,
                "a.txt",
                "search dismiss restores the filter committed by click-away",
            );

            assert!(fixture.press(Key::f, ModifierType::empty()));
            wait_until_msg(
                || prompt_has_focus(&fixture) && prompt_entry_text(&fixture) == "a.txt",
                "reopened f pre-fills the committed query",
            );
            fixture.footer.prompt_entry_widget().set_text("");
            wait_until_msg(
                || {
                    fixture.view.hidden_filter_query().trim().is_empty()
                        && rendered_name(&fixture.view.widget(), "c.txt")
                        && rendered_name(&fixture.view.widget(), "sub")
                },
                "clearing the prompt shows the full listing",
            );
            fixture.select_name("b.txt");
            wait_until_msg(
                || {
                    !prompt_has_focus(&fixture)
                        && !fixture.view.hidden_filter_active()
                        && !footer_has_label(&fixture, "filter: a.txt")
                        && fixture.cursor_name().as_deref() == Some("b.txt")
                },
                "click-away of an empty query cancels the filter and keeps the clicked row",
            );

            dismiss_search_over_filter(&fixture, "txt");
            wait_until_msg(
                || {
                    !fixture.view.hidden_filter_active()
                        && !footer_has_label(&fixture, "filter: a.txt")
                        && rendered_name(&fixture.view.widget(), "c.txt")
                        && rendered_name(&fixture.view.widget(), "sub")
                },
                "search dismiss leaves the cancelled filter cleared",
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
fn minimal_columns_prompt_up_down_walks_filter_and_search_hits() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_columns_prompt_up_down_walks_filter_and_search_hits",
        || {
            let fixture = MinimalFixture::new();
            std::fs::write(fixture._directory.path().join("bx"), b"between").expect("gap file");
            fixture.view.refresh();
            wait_until_msg(
                || {
                    fixture
                        .view
                        .browser()
                        .column_snapshot(0)
                        .is_some_and(|column| {
                            !column.loading && column.count == 5 && listing_has_name(&fixture, "bx")
                        })
                },
                "bx sits between txt names so directory steps and hit steps diverge",
            );
            fixture.select_name("a.txt");
            assert_eq!(fixture.cursor_name().as_deref(), Some("a.txt"));

            for key in [Key::f, Key::s] {
                assert!(fixture.press(key, ModifierType::empty()));
                wait_until_msg(
                    || prompt_has_focus(&fixture),
                    "prompt owns the keyboard before the query",
                );
                fixture.footer.prompt_entry_widget().set_text("txt");
                wait_until_msg(
                    || {
                        fixture.view.selected_search_results().is_some()
                            && overlay_listing_names(&fixture).len() >= 3
                            && rendered_name(&fixture.view.widget(), "c.txt")
                            && !rendered_name(&fixture.view.widget(), "bx")
                    },
                    "columns prompt shows txt hits and hides bx",
                );
                let before = fixture.view.search_hit_index();
                let listing = overlay_listing_names(&fixture);
                assert!(fixture.press(Key::Down, ModifierType::empty()));
                assert!(fixture.press(Key::Down, ModifierType::empty()));
                let deadline = Instant::now() + Duration::from_secs(5);
                let moved = loop {
                    let after = fixture.view.search_hit_index();
                    let selected = yank_names(&fixture);
                    let cursor_hit = after.and_then(|index| listing.get(index as usize).cloned());
                    if prompt_has_focus(&fixture)
                        && fixture.cursor_name().as_deref() == Some("a.txt")
                        && after.is_some_and(|index| index > 0 && Some(index) != before)
                        && selected.len() == 1
                        && selected.first() == cursor_hit.as_ref()
                        && selected.first().is_some_and(|name| name != "bx")
                    {
                        break after.expect("moved search cursor");
                    }
                    assert!(
                        Instant::now() < deadline,
                        "Down walks hits; before={before:?} after={after:?} selected={selected:?} listing={listing:?} cursor={:?} prompt={}",
                        fixture.cursor_name(),
                        prompt_has_focus(&fixture),
                    );
                    glib::MainContext::default().iteration(false);
                    std::thread::sleep(Duration::from_millis(2));
                };
                assert!(fixture.press(Key::Up, ModifierType::empty()));
                wait_until_msg(
                    || {
                        prompt_has_focus(&fixture)
                            && fixture.cursor_name().as_deref() == Some("a.txt")
                            && fixture.view.search_hit_index() == Some(moved - 1)
                    },
                    "Up walks back along the same hit list",
                );
                assert!(fixture.press(Key::Escape, ModifierType::empty()));
                wait_until_msg(
                    || !prompt_has_focus(&fixture),
                    "Escape closes the columns prompt",
                );
                if key == Key::s && fixture.view.selected_search_results().is_some() {
                    assert!(fixture.press(Key::h, ModifierType::empty()));
                }
                wait_until_msg(
                    || fixture.view.selected_search_results().is_none(),
                    "overlay closes before the next prompt",
                );
                fixture.select_name("a.txt");
            }
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
                .pin_location(Location::local(&target), "sub".into());
            wait_until_msg(
                || rendered_name(&fixture.sidebar.widget, "sub"),
                "pinned sub row is visible",
            );
            let device_location = Location::local(fixture._directory.path().join("mounted-device"));
            let device = sidebar_button(crate::assets::icons::HARD_DRIVE, "USB Backup");
            fixture.sidebar.state.bind_place_row(
                &device,
                device_location.clone(),
                PlaceNavigation::Validate,
            );
            let shell = sidebar_device_row(&device, None, None);
            fixture.sidebar.state.widget.append(&shell);
            assert!(fixture.press(Key::g, ModifierType::empty()));
            wait_until_msg(
                || sidebar_keycap(&fixture, &Location::local(&target)).as_deref() == Some("1"),
                "g labels the visible sub pin as 1",
            );
            assert_eq!(
                sidebar_keycap(&fixture, &Location::local(home_directory())).as_deref(),
                Some("h")
            );
            assert_eq!(
                sidebar_keycap(&fixture, &device_location),
                None,
                "device destinations must not advertise place keycaps"
            );
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
fn minimal_g_special_dir_flashes_when_missing() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_g_special_dir_flashes_when_missing",
        || {
            let fixture = MinimalFixture::new();
            let here = fixture.view.browser().active_location();
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

fn write_action_manifest(directory: &std::path::Path, id: &str, manifest: &str) {
    let action_directory = directory.join(id);
    std::fs::create_dir_all(&action_directory).expect("action directory");
    std::fs::write(action_directory.join("action.toml"), manifest).expect("manifest");
}

fn command_action_manifest(id: &str, name: &str, enabled: bool, extensions: &str) -> String {
    format!(
        r#"
schema_version = 1
id = "{id}"
name = "{name}"
enabled = {enabled}

[when]
kinds = ["file"]
extensions = [{extensions}]

[run]
runtime = "command"
program = "true"
args = ["{{paths}}"]
"#
    )
}

fn seed_action_chord_catalog(matching: usize, extras: bool) {
    let actions = crate::storage::config_directory().join("actions");
    let _ = std::fs::remove_dir_all(&actions);
    std::fs::create_dir_all(&actions).expect("actions directory");
    for index in 1..=matching {
        let id = format!("chord-{index:02}");
        let name = format!("Chord {index:02}");
        write_action_manifest(
            &actions,
            &id,
            &command_action_manifest(&id, &name, true, r#""txt""#),
        );
    }
    if extras {
        write_action_manifest(
            &actions,
            "png-chord",
            &command_action_manifest("png-chord", "PNG chord", true, r#""png""#),
        );
        write_action_manifest(
            &actions,
            "disabled-chord",
            &command_action_manifest("disabled-chord", "A disabled chord", false, r#""txt""#),
        );
    }
    crate::ui::actions::shared().reload();
}

fn action_job_names() -> Vec<String> {
    crate::ui::jobs::shared()
        .snapshot()
        .into_iter()
        .map(|job| job.action_name)
        .collect()
}

#[test]
fn minimal_action_chord_lists_matching_slots_and_runs_the_first() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_action_chord_lists_matching_slots_and_runs_the_first",
        || {
            assert!(
                crate::ui::shortcut_reference::shortcuts(true).any(|shortcut| {
                    shortcut.1 == "Run a custom action" && shortcut.3 == "; 1–9 / 0"
                }),
                "minimal map must list ; 1–9 / 0 for custom actions"
            );
            seed_action_chord_catalog(11, true);
            let fixture = MinimalFixture::new();
            fixture.select_name("a.txt");
            assert!(fixture.press(Key::semicolon, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, ";-"));
            wait_until(|| fixture.footer.chord_hints_visible());
            let labels = fixture.footer.chord_hint_labels();
            for (index, expected) in (1..=9)
                .map(|index| format!("{index} Chord {index:02}"))
                .chain(std::iter::once("0 Chord 10".to_owned()))
                .enumerate()
            {
                assert!(
                    labels.iter().any(|line| line == &expected),
                    "{expected} missing from {labels:?} at slot {index}"
                );
            }
            for omitted in ["Chord 11", "PNG chord", "A disabled chord"] {
                assert!(
                    !labels.iter().any(|line| line.contains(omitted)),
                    "{omitted} must not appear in {labels:?}"
                );
            }
            assert_eq!(
                fixture.footer.chord_hint_keycaps(),
                (1..=9)
                    .map(|index| vec![index.to_string()])
                    .chain(std::iter::once(vec!["0".to_owned()]))
                    .collect::<Vec<_>>(),
            );

            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            pump_mainloop(Duration::from_millis(50));
            assert!(!footer_shows(&fixture, ";-"));
            assert!(!fixture.footer.chord_hints_visible());
            assert!(
                !footer_shows(&fixture, "Unknown chord"),
                "Escape cancels the action chord without a flash"
            );
            let cursor = fixture.cursor();
            assert!(fixture.press(Key::j, ModifierType::empty()));
            wait_until(|| fixture.cursor() != cursor);

            let before = action_job_names();
            assert!(fixture.press(Key::semicolon, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, ";-"));
            assert!(fixture.press(Key::x, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "Unknown chord"));
            assert_eq!(
                action_job_names(),
                before,
                "an unmatched letter must not enqueue a custom action"
            );
            assert!(!footer_shows(&fixture, ";-"));
            assert!(!fixture.footer.chord_hints_visible());
            let cursor = fixture.cursor();
            assert!(fixture.press(Key::j, ModifierType::empty()));
            wait_until(|| fixture.cursor() != cursor);

            fixture.select_name("a.txt");
            assert!(fixture.press(Key::semicolon, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, ";-"));
            assert!(fixture.press(Key::_1, ModifierType::empty()));
            wait_until(|| action_job_names().iter().any(|name| name == "Chord 01"));
            assert!(!footer_shows(&fixture, ";-"));
            assert!(!fixture.footer.chord_hints_visible());
            let cursor = fixture.cursor();
            assert!(fixture.press(Key::j, ModifierType::empty()));
            wait_until(|| fixture.cursor() != cursor);
        },
    );
}

#[test]
fn minimal_action_chord_vacant_slot_does_not_run_another_action() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_action_chord_vacant_slot_does_not_run_another_action",
        || {
            seed_action_chord_catalog(2, false);
            let fixture = MinimalFixture::new();
            fixture.select_name("a.txt");
            let before = action_job_names();
            assert!(fixture.press(Key::semicolon, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, ";-"));
            wait_until(|| fixture.footer.chord_hints_visible());
            let labels = fixture.footer.chord_hint_labels();
            assert!(
                labels.iter().any(|line| line == "1 Chord 01"),
                "first match missing from {labels:?}"
            );
            assert!(
                labels.iter().any(|line| line == "2 Chord 02"),
                "second match missing from {labels:?}"
            );
            assert!(
                !labels.iter().any(|line| line.starts_with('0')),
                "vacant tenth slot must not appear in {labels:?}"
            );
            assert!(fixture.press(Key::_0, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "No action 0"));
            assert_eq!(
                action_job_names(),
                before,
                "a vacant slot must not enqueue some other action"
            );
            assert!(!footer_shows(&fixture, ";-"));
            assert!(!fixture.footer.chord_hints_visible());
            let cursor = fixture.cursor();
            assert!(fixture.press(Key::j, ModifierType::empty()));
            wait_until(|| fixture.cursor() != cursor);
        },
    );
}

#[test]
fn minimal_open_with_yields_to_a_new_prompt_and_mode_transition() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_open_with_yields_to_a_new_prompt_and_mode_transition",
        || {
            let fixture = MinimalFixture::new();
            for replace_with_prompt in [true, false] {
                fixture.select_name("a.txt");
                assert!(fixture.press(Key::O, ModifierType::SHIFT_MASK));
                assert!(
                    visible_widget_with_class(fixture.window.upcast_ref(), "app-modal-layer")
                        .is_none(),
                    "O returns before metadata and app lookup"
                );
                if replace_with_prompt {
                    assert!(fixture.press(Key::g, ModifierType::empty()));
                    assert!(fixture.press(Key::space, ModifierType::empty()));
                    wait_until_msg(
                        || prompt_has_focus(&fixture) && footer_shows(&fixture, "go ›"),
                        "new go prompt owns focus instead of a stale Open With chooser",
                    );
                    assert!(
                        visible_widget_with_class(fixture.window.upcast_ref(), "app-modal-layer")
                            .is_none()
                    );
                    assert!(fixture.press(Key::Escape, ModifierType::empty()));
                } else {
                    let preferences = PreferenceManager::shared();
                    preferences.set_minimal_mode(false);
                    preferences.set_minimal_mode(true);
                }
                wait_until_msg(
                    || fixture.view.item_view_has_focus(),
                    "file focus after replacing the pending Open With lookup",
                );
                assert!(
                    visible_widget_with_class(fixture.window.upcast_ref(), "app-modal-layer")
                        .is_none()
                );
            }
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
            fixture.view.refresh();
            wait_until_msg(
                || {
                    ["docs", "documents", "nested"]
                        .iter()
                        .all(|name| listing_has_name(&fixture, name))
                },
                "completion fixture reload includes docs, documents and nested",
            );
            let expect_completion = |expected: &str| {
                wait_until_msg(
                    || prompt_entry_text(&fixture) == expected && prompt_has_focus(&fixture),
                    &format!("go completion publishes {expected:?} without leaving the prompt"),
                );
            };

            assert!(fixture.press(Key::g, ModifierType::empty()));
            wait_until_msg(
                || footer_shows(&fixture, "g-"),
                "g arms Go before folder completion",
            );
            assert!(fixture.press(Key::space, ModifierType::empty()));
            wait_until_msg(
                || prompt_has_focus(&fixture) && footer_shows(&fixture, "go ›"),
                "Go prompt owns focus before completing d",
            );

            fixture.footer.prompt_entry_widget().set_text("d");
            assert!(
                fixture.press(Key::Tab, ModifierType::empty()),
                "Tab must stay in the go prompt"
            );
            expect_completion("docs");
            assert!(prompt_has_focus(&fixture), "Tab must not move GTK focus");
            assert!(fixture.press(Key::Tab, ModifierType::empty()));
            expect_completion("documents");
            assert!(fixture.press(Key::Tab, ModifierType::empty()));
            expect_completion("docs");
            assert!(fixture.press(Key::Tab, ModifierType::SHIFT_MASK));
            expect_completion("documents");
            assert!(fixture.press(Key::ISO_Left_Tab, ModifierType::empty()));
            expect_completion("docs");
            assert!(prompt_has_focus(&fixture));

            fixture.footer.prompt_entry_widget().set_text("a");
            assert!(fixture.press(Key::Tab, ModifierType::empty()));
            assert_eq!(prompt_entry_text(&fixture), "a", "files are not completed");
            assert!(prompt_has_focus(&fixture));

            fixture.footer.prompt_entry_widget().set_text("zzz");
            assert!(fixture.press(Key::Tab, ModifierType::empty()));
            assert_eq!(prompt_entry_text(&fixture), "zzz");
            assert!(prompt_has_focus(&fixture));
            fixture.footer.prompt_entry_widget().set_text("missing/de");
            assert!(fixture.press(Key::Tab, ModifierType::empty()));
            wait_until_msg(
                || footer_shows(&fixture, "Check or refine the folder path"),
                "failed go completion reports a non-secret hint",
            );
            assert_eq!(prompt_entry_text(&fixture), "missing/de");
            assert!(prompt_has_focus(&fixture));
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            assert_eq!(prompt_entry_text(&fixture), "");

            assert!(fixture.press(Key::g, ModifierType::empty()));
            wait_until_msg(
                || footer_shows(&fixture, "g-"),
                "g re-arms after completion error cancellation",
            );
            assert!(fixture.press(Key::space, ModifierType::empty()));
            wait_until_msg(
                || prompt_has_focus(&fixture) && footer_shows(&fixture, "go ›"),
                "Go prompt reopens before nested completion",
            );
            fixture.footer.prompt_entry_widget().set_text("nested/de");
            assert!(fixture.press(Key::Tab, ModifierType::empty()));
            fixture.footer.prompt_entry_widget().set_text("d");
            assert!(fixture.press(Key::Tab, ModifierType::empty()));
            expect_completion("docs");
            assert!(
                !footer_shows(&fixture, "Check or refine the folder path"),
                "editing clears stale completion errors"
            );
            fixture.footer.prompt_entry_widget().set_text("nested/de");
            assert!(fixture.press(Key::Tab, ModifierType::empty()));
            expect_completion("nested/deep");
            assert!(fixture.press(Key::Tab, ModifierType::empty()));
            expect_completion("nested/deeper");
            let target = root.join("nested").join("deeper");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until_msg(
                || {
                    fixture.view.browser().active_location()
                        == Some(Location::local(target.clone()))
                },
                "Go Enter navigates to the completed nested/deeper folder",
            );
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
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_secs();
            let store = glib::user_state_dir().join("strata/navigation-history.json");
            std::fs::create_dir_all(store.parent().expect("history parent"))
                .expect("history directory");
            std::fs::write(&store, serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "entries": [
                    {"uri": gio::File::for_path(&alpha).uri().as_str(), "rank": 8.0, "last_accessed": now - 60},
                    {"uri": gio::File::for_path(&beta).uri().as_str(), "rank": 1.0, "last_accessed": now}
                ]
            })).expect("history JSON")).expect("seed distinct last visits");
            let _history = crate::services::NavigationHistory::shared();
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
            fixture.view.restore_file_view_focus();
            wait_until_msg(
                || listing_cursor_has_focus(&fixture),
                "history destination owns file focus",
            );
            assert!(fixture.press(Key::Z, ModifierType::empty()));
            wait_until_msg(
                || fixture.footer.history_candidates_visible() && prompt_has_focus(&fixture),
                "Z opens recent folders",
            );
            assert_eq!(
                fixture.footer.history_candidate_names(),
                ["beta-project", "alpha-project"],
                "Z uses last visit, not z frecency or alphabetical order"
            );
            assert_eq!(
                fixture.footer.selected_history_path().as_deref(),
                Some(beta.as_path())
            );
            fixture
                .footer
                .prompt_entry_widget()
                .set_text("no-such-history-folder");
            wait_until_msg(
                || footer_shows(&fixture, "No matching folders"),
                "Z miss displays No matching folders",
            );
            assert!(fixture.footer.selected_history_path().is_none());
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until_msg(
                || !prompt_has_focus(&fixture) && footer_shows(&fixture, "No matching folders"),
                "submitting history miss reports failure",
            );
            assert_eq!(
                fixture.view.browser().active_location(),
                Some(Location::local(beta))
            );
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
            let failed = Rc::new(std::cell::Cell::new(0u32));
            let failed_for = failed.clone();
            fixture.view.browser().observe(move |event| {
                if matches!(event, BrowserEvent::OperationFailed { .. }) {
                    failed_for.set(failed_for.get() + 1);
                }
            });
            for (input, name, folder) in [
                ("created.txt", "created.txt", false),
                ("newdir/", "newdir", true),
            ] {
                fixture.select_name("sub");
                assert_eq!(fixture.cursor_name().as_deref(), Some("sub"));
                submit_prompt(&fixture, Key::a, input);
                let path = fixture._directory.path().join(name);
                wait_until_msg(
                    || {
                        path.exists()
                            && fixture.cursor_name().as_deref() == Some(name)
                            && listing_cursor_has_focus(&fixture)
                    },
                    &format!("create {input:?} publishes and focuses {name:?}"),
                );
                assert_eq!(path.is_dir(), folder);
                assert_eq!(path.is_file(), !folder);
                assert_eq!(listing_selected_names(&fixture), [name]);
                assert!(!fixture.view.rename_is_active());
                assert_eq!(
                    fixture.view.browser().active_location(),
                    Some(Location::local(fixture._directory.path()))
                );
                let before = failed.get();
                submit_prompt(&fixture, Key::a, input);
                wait_until_msg(
                    || {
                        failed.get() == before + 1
                            && modal_button(fixture.window.upcast_ref(), "Close").is_some()
                    },
                    &format!("duplicate {input:?} reports an error"),
                );
                assert!(
                    !fixture
                        ._directory
                        .path()
                        .join(format!("{name} (1)"))
                        .exists(),
                    "conflicts must not uniquify"
                );
                assert!(path.exists());
                dismiss_error_dialog(&fixture);
            }
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
            wait_until_msg(
                || footer_shows(&fixture, "rename") && prompt_has_focus(&fixture),
                "rename prompt owns focus before editing a.txt",
            );
            fixture.footer.prompt_entry_widget().set_text("renamed.txt");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until_msg(
                || fixture._directory.path().join("renamed.txt").exists(),
                "rename wrote renamed.txt",
            );
            wait_until_msg(
                || {
                    listing_cursor_has_focus(&fixture)
                        && fixture.cursor_name().as_deref() == Some("renamed.txt")
                        && rendered_name(&fixture.view.widget(), "renamed.txt")
                        && !rendered_name(&fixture.view.widget(), "a.txt")
                },
                "rename Enter must publish renamed.txt and restore its listing cursor",
            );

            submit_prompt_with_results(&fixture, Key::s, "txt", &["b.txt", "c.txt", "renamed.txt"]);
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
            submit_prompt(&fixture, Key::a, "renamed.txt");
            wait_until_msg(
                || {
                    failed.get() == 1
                        && visible_dialog_confirm(fixture.window.upcast_ref()).is_some()
                },
                "create renamed.txt conflict exposes its Close action",
            );
            let close = visible_dialog_confirm(fixture.window.upcast_ref()).expect("error Close");
            close.emit_clicked();
            wait_until_msg(
                || listing_cursor_has_focus(&fixture),
                "error dialog Close restores listing cursor",
            );

            let conflict = MinimalFixture::new();
            assert!(conflict.press(Key::a, ModifierType::empty()));
            wait_until_msg(
                || prompt_has_focus(&conflict),
                "create prompt owns focus before the footer steals it",
            );
            conflict.footer.prompt_entry_widget().set_text("a.txt");
            assert!(conflict.press(Key::Return, ModifierType::empty()));
            let shortcuts =
                visible_widget_with_class(conflict.window.upcast_ref(), "shortcut-footer-button")
                    .expect("shortcuts button");
            shortcuts.grab_focus();
            wait_until_msg(
                || visible_dialog_confirm(conflict.window.upcast_ref()).is_some(),
                "duplicate a.txt reports an error while the shortcuts button is focused",
            );
            let close = visible_dialog_confirm(conflict.window.upcast_ref()).expect("error Close");
            close.emit_clicked();
            wait_until_msg(
                || listing_cursor_has_focus(&conflict) && !widget_contains_focus(&shortcuts),
                "closing the error dialog returns to the listing, not the shortcuts button",
            );

            let icons = MinimalFixture::new();
            icons.view.set_view_mode(BrowserMode::Icons);
            wait_until_msg(
                || rendered_name(&icons.view.widget(), "a.txt"),
                "icons shows listing",
            );
            submit_prompt_with_results(&icons, Key::f, "txt", &["a.txt", "b.txt", "c.txt"]);
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
            wait_until_msg(
                || footer_shows(&first, "g-"),
                "first window Go chord is armed",
            );
            assert!(first.press(Key::space, ModifierType::empty()));
            wait_until_msg(
                || prompt_has_focus(&first),
                "first window Go prompt owns focus",
            );
            first
                .footer
                .prompt_entry_widget()
                .set_text("smb://user:pass@host/share");
            assert!(second.press(
                Key::M,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK
            ));
            wait_until_msg(
                || first.view.item_view_has_focus() && second.view.item_view_has_focus(),
                "another-window disable restores both file cursors",
            );
            assert_eq!(prompt_entry_text(&first), "");
            assert_eq!(chord_hint_count(&first), 0);
            assert!(!footer_shows(&first, "g-"));
            assert!(first.view.item_view_has_focus());
            assert_eq!(prompt_entry_text(&second), "");
            PreferenceManager::shared().set_minimal_mode(true);
        },
    );
}

fn yank_child_file(fixture: &MinimalFixture, name: &str) {
    fixture.select_name("sub");
    assert!(fixture.press(Key::l, ModifierType::empty()));
    wait_until_msg(
        || {
            fixture
                .view
                .browser()
                .column_snapshot(1)
                .is_some_and(|column| {
                    !column.loading && column.count >= 1 && listing_has_name_at(fixture, 1, name)
                })
        },
        "child column publishes the yanked file",
    );
    move_cursor_to_name(fixture, name);
    assert!(fixture.press(Key::y, ModifierType::empty()));
    wait_until(|| {
        fixture
            .window
            .clipboard()
            .formats()
            .contains_type(gtk::gdk::FileList::static_type())
    });
}

fn listing_has_name_at(fixture: &MinimalFixture, depth: usize, name: &str) -> bool {
    let Some(count) = fixture
        .view
        .browser()
        .column_snapshot(depth)
        .map(|column| column.count)
    else {
        return false;
    };
    (0..count).any(|index| {
        fixture
            .view
            .browser()
            .entry_at(depth, index)
            .is_some_and(|entry| entry.display_name == name)
    })
}

fn move_cursor_to_name(fixture: &MinimalFixture, name: &str) {
    if fixture.cursor_name().as_deref() == Some(name) {
        return;
    }
    for key in [Key::j, Key::k] {
        for _ in 0..12 {
            if fixture.cursor_name().as_deref() == Some(name) {
                return;
            }
            assert!(fixture.press(key, ModifierType::empty()));
        }
    }
    assert_eq!(
        fixture.cursor_name().as_deref(),
        Some(name),
        "cursor should reach {name}"
    );
}

/// `j`/`k` onto `name` so the one-item fill is a real cursor, not a load cursor.
fn focus_only_parent_folder(fixture: &MinimalFixture, name: &str) {
    assert!(fixture.press(Key::h, ModifierType::empty()));
    wait_until(|| fixture.view.browser().column_snapshot(1).is_none());
    let target = fixture.index_of(name);
    if fixture.cursor() == Some(target) {
        let away = if target == 0 { Key::j } else { Key::k };
        assert!(fixture.press(away, ModifierType::empty()));
    }
    let target_now = fixture.index_of(name);
    for _ in 0..12 {
        if fixture.cursor() == Some(target_now) {
            break;
        }
        let current = fixture.cursor().unwrap_or(0);
        let key = if current < target_now { Key::j } else { Key::k };
        assert!(fixture.press(key, ModifierType::empty()));
    }
    assert_eq!(fixture.cursor(), Some(fixture.index_of(name)));
    assert!(
        !fixture.view.browser().selection_is_load_cursor(),
        "j/k must clear the load cursor before paste"
    );
    assert_eq!(fixture.fill(), vec![fixture.index_of(name)]);
}

fn clipboard_file_names(fixture: &MinimalFixture) -> Vec<String> {
    use gtk::prelude::*;
    let Ok(value) = glib::MainContext::default().block_on(
        fixture
            .window
            .clipboard()
            .read_value_future(gtk::gdk::FileList::static_type(), glib::Priority::DEFAULT),
    ) else {
        return Vec::new();
    };
    let Ok(files) = value.get::<gtk::gdk::FileList>() else {
        return Vec::new();
    };
    files
        .files()
        .into_iter()
        .filter_map(|file| {
            file.path().and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
        })
        .collect()
}

fn open_empty_sub(fixture: &MinimalFixture) {
    fixture.select_name("sub");
    assert!(fixture.press(Key::l, ModifierType::empty()));
    let child = Location::local(fixture._directory.path().join("sub"));
    wait_until(|| {
        fixture.view.browser().active_location() == Some(child.clone())
            && fixture
                .view
                .browser()
                .column_snapshot(1)
                .is_some_and(|column| !column.loading && column.count == 0)
    });
}

fn open_empty_icons_sub(fixture: &MinimalFixture) {
    fixture.view.set_view_mode(BrowserMode::Icons);
    wait_until(|| rendered_name(&fixture.view.widget(), "sub"));
    fixture.select_name("sub");
    assert!(fixture.press(Key::Return, ModifierType::empty()));
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
    fixture.view.refresh();
    wait_until_msg(
        || {
            listing_has_name(fixture, "innerdir")
                && rendered_name(&fixture.view.widget(), "innerdir")
        },
        "fixture refresh publishes innerdir before recursive search",
    );
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

fn clipboard_text(fixture: &MinimalFixture) -> Option<String> {
    glib::MainContext::default()
        .block_on(fixture.window.clipboard().read_text_future())
        .ok()
        .flatten()
        .map(|text| text.to_string())
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

fn query_prompt_with_results(fixture: &MinimalFixture, key: Key, query: &str, expected: &[&str]) {
    assert!(fixture.press(key, ModifierType::empty()));
    wait_until_msg(
        || prompt_has_focus(fixture),
        &format!("{key:?} prompt must have focus before typing {query:?}"),
    );
    fixture.footer.prompt_entry_widget().set_text(query);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut names = overlay_listing_names(fixture);
        names.sort();
        if fixture.view.hidden_filter_query() == query
            && names == expected
            && prompt_has_focus(fixture)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{key:?} query {query:?} expected {expected:?}; actual query={:?}, rows={names:?}, prompt_focus={}",
            fixture.view.hidden_filter_query(),
            prompt_has_focus(fixture)
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn wait_for_search_cursor(fixture: &MinimalFixture, index: u32, when: &str) {
    wait_until_msg(
        || {
            fixture.view.search_hit_index() == Some(index)
                && listing_cursor_has_focus(fixture)
                && focused_widget(fixture).is_some_and(|focus| {
                    fixture.view.search_result_listing().is_some_and(|entries| {
                        entries
                            .get(index as usize)
                            .is_some_and(|entry| rendered_name(&focus, &entry.display_name))
                    })
                })
        },
        when,
    );
}

fn submit_prompt_with_results(fixture: &MinimalFixture, key: Key, query: &str, expected: &[&str]) {
    query_prompt_with_results(fixture, key, query, expected);
    assert!(fixture.press(Key::Return, ModifierType::empty()));
    wait_for_search_cursor(
        fixture,
        0,
        &format!("{key:?} query {query:?} focuses result 0 after submit"),
    );
}

fn dismiss_search_over_filter(fixture: &MinimalFixture, query: &str) {
    assert!(fixture.press(Key::s, ModifierType::empty()));
    wait_until_msg(
        || prompt_has_focus(fixture),
        "s prompt opens over the filter",
    );
    fixture.footer.prompt_entry_widget().set_text(query);
    wait_until_msg(
        || {
            fixture.view.force_recursive_search()
                && fixture.view.selected_search_results().is_some()
                && overlay_listing_names(fixture).len() >= 2
        },
        "s replaces the filter with hits",
    );
    assert!(fixture.press(Key::Escape, ModifierType::empty()));
    wait_until_msg(
        || !prompt_has_focus(fixture) && fixture.view.selected_search_results().is_some(),
        "Esc keeps the search hits",
    );
    assert!(fixture.press(Key::h, ModifierType::empty()));
    wait_until_msg(
        || !fixture.view.force_recursive_search(),
        "h dismisses the search",
    );
}

fn submit_prompt(fixture: &MinimalFixture, key: Key, text: &str) {
    assert!(fixture.press(key, ModifierType::empty()));
    wait_until_msg(
        || prompt_has_focus(fixture),
        &format!("{key:?} prompt has focus before {text:?}"),
    );
    fixture.footer.prompt_entry_widget().set_text(text);
    if matches!(key, Key::s | Key::f) {
        wait_until_msg(
            || fixture.view.hidden_filter_query() == text,
            &format!("{key:?} applies query {text:?}"),
        );
    }
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
            submit_prompt_with_results(&fixture, Key::s, "unique-hit", &["unique-hit.txt"]);
            let selected = fixture
                .view
                .selected_search_results()
                .expect("recursive results");
            assert_eq!(selected.len(), 1);
            assert_eq!(
                selected[0].location,
                Location::local(fixture._directory.path().join("innerdir/unique-hit.txt"))
            );
            assert!(fixture.press(Key::h, ModifierType::empty()));
            wait_until_msg(
                || !fixture.view.hidden_filter_active() && listing_cursor_has_focus(&fixture),
                "h dismisses recursive hits",
            );
            submit_prompt_with_results(&fixture, Key::f, "txt", &["a.txt", "b.txt", "c.txt"]);
            assert!(!fixture.view.force_recursive_search());
            assert!(fixture.press(Key::a, ModifierType::CONTROL_MASK));
            let selected = fixture
                .view
                .selected_search_results()
                .expect("local matches");
            assert_eq!(selected.len(), 3);
            assert!(selected.iter().all(|entry| {
                entry.location.native_path().and_then(|path| path.parent())
                    == Some(fixture._directory.path())
            }));
            assert!(!rendered_name(&fixture.view.widget(), "unique-hit.txt"));
        },
    );
}

#[test]
fn minimal_disable_clears_retained_search_and_restores_default_scope_in_both_windows() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_disable_clears_retained_search_and_restores_default_scope_in_both_windows",
        || {
            let first = MinimalFixture::new();
            let second = MinimalFixture::additional();
            add_nested_hit(&first);
            add_nested_hit(&second);
            for fixture in [&first, &second] {
                fixture.view.refresh();
                wait_until_msg(
                    || listing_has_name(fixture, "innerdir"),
                    "fixture reload includes innerdir",
                );
            }
            let preferences = PreferenceManager::shared();
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                for finish in [Key::Return, Key::Escape] {
                    preferences.set_filter_include_subfolders(false);
                    preferences.set_minimal_mode(true);
                    for fixture in [&first, &second] {
                        fixture.view.set_view_mode(mode);
                        fixture.select_name("a.txt");
                        fixture.view.restore_file_view_focus();
                        wait_until_msg(
                            || listing_cursor_has_focus(fixture),
                            "listing cursor before prompt",
                        );
                    }
                    for (fixture, key, expected) in [
                        (
                            &first,
                            Key::s,
                            vec!["a.txt", "b.txt", "c.txt", "unique-hit.txt"],
                        ),
                        (&second, Key::f, vec!["a.txt", "b.txt", "c.txt"]),
                    ] {
                        assert!(fixture.press(key, ModifierType::empty()));
                        wait_until_msg(|| prompt_has_focus(fixture), "footer prompt before typing");
                        fixture.footer.prompt_entry_widget().set_text("txt");
                        wait_until_msg(
                            || {
                                let mut names = overlay_listing_names(fixture);
                                names.sort();
                                names == expected && prompt_has_focus(fixture)
                            },
                            &format!("{mode:?} {key:?} must publish {expected:?}"),
                        );
                        assert!(fixture.press(finish, ModifierType::empty()));
                        wait_until_msg(
                            || listing_cursor_has_focus(fixture),
                            "retained results own focus",
                        );
                    }
                    assert!(second.press(
                        Key::M,
                        ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK
                    ));
                    assert!(!preferences.minimal_mode());
                    for fixture in [&first, &second] {
                        wait_until_msg(
                            || {
                                !fixture.view.hidden_filter_active()
                                    && fixture.view.selected_search_results().is_none()
                                    && listing_cursor_has_focus(fixture)
                            },
                            &format!("{mode:?} {finish:?} disable restores the listing"),
                        );
                        assert!(!fixture.view.force_recursive_search());
                        wait_until_msg(
                            || {
                                ["a.txt", "b.txt", "c.txt", "sub", "innerdir"]
                                    .iter()
                                    .all(|name| listing_has_name(fixture, name))
                            },
                            "disable restores every fixture row",
                        );
                        assert_eq!(fixture.footer.prompt_text(), "");
                        assert!(!footer_has_label(fixture, "filter: txt"));
                        assert!(!funnel_stays_hidden(fixture));
                        assert!(fixture.press(Key::f, ModifierType::CONTROL_MASK));
                        let entry = focused_widget(fixture)
                            .and_then(|focus| focus.ancestor(gtk::Entry::static_type()))
                            .and_downcast::<gtk::Entry>()
                            .expect("default Ctrl+F focuses the active filter entry");
                        entry.set_text("txt");
                        wait_until_msg(
                            || {
                                let mut names = overlay_listing_names(fixture);
                                names.sort();
                                names == ["a.txt", "b.txt", "c.txt"]
                            },
                            &format!("{mode:?} default filter must not retain forced recursion"),
                        );
                    }
                    preferences.set_filter_include_subfolders(true);
                    for fixture in [&first, &second] {
                        wait_until_msg(
                            || {
                                overlay_listing_names(fixture)
                                    .contains(&"unique-hit.txt".to_owned())
                            },
                            "default filter must follow the live Include subfolders preference",
                        );
                        fixture.view.dismiss_hidden_filter();
                    }
                }
            }
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
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                fixture.view.set_view_mode(mode);
                wait_until_msg(
                    || rendered_name(&fixture.view.widget(), "a.txt"),
                    &format!("{mode:?} listing rebuild"),
                );
                fixture.select_name("a.txt");
                wait_until_msg(
                    || {
                        !footer_count_text(&fixture).is_empty()
                            && footer_count_text(&fixture) != "3 items"
                    },
                    "footer describes directory selection before search",
                );
                let before = footer_count_text(&fixture);
                submit_prompt_with_results(&fixture, Key::s, "txt", &["a.txt", "b.txt", "c.txt"]);
                wait_until_msg(
                    || footer_count_text(&fixture) == "3 items",
                    &format!("{mode:?} footer describes three displayed hits, not hidden fill"),
                );
                assert_ne!(footer_count_text(&fixture), before);
                fixture.view.dismiss_hidden_filter();
                wait_until_msg(
                    || {
                        fixture.view.selected_search_results().is_none()
                            && rendered_name(&fixture.view.widget(), "sub")
                            && footer_count_text(&fixture) != "3 items"
                    },
                    &format!("{mode:?} dismissal restores directory footer"),
                );
            }
        },
    );
}

#[test]
fn minimal_search_l_and_right_preview_files_and_enter_directories() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_search_l_and_right_preview_files_and_enter_directories",
        || {
            install_text_activation_handler();
            for mode in [BrowserMode::Columns, BrowserMode::List] {
                let fixture = find_highlight_fixture(mode);
                fixture.footer.observe_browser(&fixture.view);
                let nested = add_nested_hit(&fixture);
                let root = Location::local(fixture._directory.path());
                let child = Location::local(nested.parent().expect("hit parent"));
                PreferenceManager::shared().set_filter_include_subfolders(false);
                let opened = Rc::new(RefCell::new(Vec::<Location>::new()));
                let observed = opened.clone();
                fixture.view.browser().observe(move |event| {
                    if let BrowserEvent::OpenRequested { location } = event {
                        observed.borrow_mut().push(location.clone());
                    }
                });
                for key in [Key::l, Key::Right] {
                    submit_prompt_with_results(&fixture, Key::s, "unique-hit", &["unique-hit.txt"]);
                    assert!(fixture.press(key, ModifierType::empty()));
                    wait_until_msg(
                        || {
                            fixture.preview.is_enabled()
                                && fixture.preview.owns_keys_chrome()
                                && rendered_name(&fixture.preview.widget(), "unique-hit.txt")
                        },
                        &format!("{mode:?} {key:?} previews the exact nested file"),
                    );
                    assert!(opened.borrow().is_empty());
                    assert_eq!(fixture.view.browser().active_location(), Some(root.clone()));
                    assert!(fixture.press(key, ModifierType::empty()));
                    assert!(
                        fixture.preview.is_enabled(),
                        "repeating {key:?} keeps preview"
                    );
                    assert!(fixture.preview.owns_keys_chrome());
                    assert!(opened.borrow().is_empty());
                    assert!(fixture.press(Key::h, ModifierType::empty()));
                    wait_for_search_cursor(&fixture, 0, "preview h returns to the same search hit");
                    assert!(fixture.preview.is_enabled());
                    assert_eq!(overlay_listing_names(&fixture), ["unique-hit.txt"]);
                    assert!(fixture.press(Key::Return, ModifierType::empty()));
                    wait_until_msg(
                        || *opened.borrow() == [Location::local(&nested)],
                        "Enter requests the exact nested file once",
                    );
                    fixture.preview.action().activate(None);
                    fixture.view.dismiss_hidden_filter();
                    fixture.view.restore_file_view_focus();
                    wait_until_msg(
                        || listing_cursor_has_focus(&fixture),
                        "listing focus after hit activation",
                    );
                    opened.borrow_mut().clear();
                    submit_prompt_with_results(&fixture, Key::s, "innerdir", &["innerdir"]);
                    assert!(fixture.press(key, ModifierType::empty()));
                    wait_until_msg(
                        || {
                            fixture.view.browser().active_location() == Some(child.clone())
                                && rendered_name(&fixture.view.widget(), "unique-hit.txt")
                                && listing_cursor_has_focus(&fixture)
                        },
                        &format!("{mode:?} {key:?} enters directory hit and focuses its contents"),
                    );
                    assert!(
                        opened.borrow().is_empty(),
                        "directory hits navigate without OpenRequested"
                    );
                    assert!(!fixture.preview.is_enabled());
                    assert!(fixture.press(Key::h, ModifierType::empty()));
                    wait_until_msg(
                        || {
                            fixture.view.browser().active_location() == Some(root.clone())
                                && listing_cursor_has_focus(&fixture)
                        },
                        "h returns from directory hit to search root",
                    );
                }
            }
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
            let parent = fixture._directory.path().parent().expect("fixture parent");
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
                fixture.view.browser().active_location(),
                Some(Location::local(parent))
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
fn minimal_filter_visual_and_multiselect_use_visible_matches() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_filter_visual_and_multiselect_use_visible_matches",
        || {
            let fixture = MinimalFixture::new();
            submit_prompt_with_results(&fixture, Key::f, "txt", &["a.txt", "b.txt", "c.txt"]);
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
            submit_prompt_with_results(&fixture, Key::s, "txt", &["a.txt", "b.txt", "c.txt"]);
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
            submit_prompt_with_results(&fixture, Key::f, "txt", &["a.txt", "b.txt", "c.txt"]);
            wait_until_msg(
                || {
                    fixture.view.search_hit_index() == Some(0)
                        && overlay_listing_names(&fixture).len() >= 3
                        && yank_names(&fixture).len() == 1
                },
                "Icons filter cursor rests on the first txt match before visual fill",
            );
            let visible = overlay_listing_names(&fixture);
            assert!(visible.len() >= 2, "Icons filter matches: {visible:?}");
            assert!(fixture.press(Key::v, ModifierType::empty()));
            wait_until_msg(
                || yank_names(&fixture).len() == 1,
                "Icons visual mode anchors the focused match",
            );
            assert!(fixture.press(Key::j, ModifierType::empty()));
            wait_until_msg(
                || fixture.view.search_hit_index() == Some(1) && yank_names(&fixture).len() == 2,
                "Icons query txt: v/j must move to result 1 and select two matches",
            );
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
            wait_until_msg(
                || yank_names(&fixture).len() == visible.len(),
                "Icons Ctrl+A selects all txt matches",
            );
            let mut icons_all = yank_names(&fixture);
            icons_all.sort();
            let mut icons_visible = visible.clone();
            icons_visible.sort();
            assert_eq!(
                icons_all, icons_visible,
                "Icons filter Ctrl+A must select every visible match"
            );

            fixture.view.dismiss_hidden_filter();
            wait_until_msg(
                || fixture.view.selected_search_results().is_none(),
                "Icons filter dismissed before List rebuild",
            );
            fixture.view.set_view_mode(BrowserMode::List);
            wait_until_msg(
                || rendered_name(&fixture.view.widget(), "a.txt"),
                "List listing did not show a.txt",
            );
            submit_prompt_with_results(&fixture, Key::s, "txt", &["a.txt", "b.txt", "c.txt"]);
            wait_until_msg(
                || {
                    fixture.view.force_recursive_search()
                        && fixture.view.search_hit_index() == Some(0)
                        && overlay_listing_names(&fixture).len() >= 3
                        && yank_names(&fixture).len() == 1
                },
                "List search cursor rests on the first txt hit before visual fill",
            );
            let hits = overlay_listing_names(&fixture);
            assert!(hits.len() >= 2, "List search hits: {hits:?}");
            assert!(fixture.press(Key::v, ModifierType::empty()));
            wait_until_msg(
                || yank_names(&fixture).len() == 1,
                "List visual mode anchors the focused hit",
            );
            assert!(fixture.press(Key::j, ModifierType::empty()));
            wait_until_msg(
                || fixture.view.search_hit_index() == Some(1) && yank_names(&fixture).len() == 2,
                "List query txt: v/j must move to result 1 and select two hits",
            );
            let range = yank_names(&fixture);
            assert!(
                range.len() >= 2,
                "List search v then j must fill hits, got {range:?}"
            );
            assert!(range.iter().all(|name| hits.contains(name)));
            assert!(fixture.press(Key::a, ModifierType::CONTROL_MASK));
            wait_until_msg(
                || yank_names(&fixture).len() == hits.len(),
                "List Ctrl+A selects all txt hits",
            );
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

#[test]
fn minimal_r_renames_the_focused_search_result() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_r_renames_the_focused_search_result",
        || {
            let fixture = MinimalFixture::new();
            add_nested_hit(&fixture);
            PreferenceManager::shared().set_filter_include_subfolders(false);
            fixture.select_name("a.txt");
            submit_prompt_with_results(&fixture, Key::s, "unique-hit", &["unique-hit.txt"]);
            assert_eq!(overlay_listing_names(&fixture), ["unique-hit.txt"]);
            assert_eq!(fixture.cursor_name().as_deref(), Some("a.txt"));

            assert!(fixture.press(Key::r, ModifierType::empty()));
            wait_until_msg(
                || {
                    footer_shows(&fixture, "rename")
                        && prompt_has_focus(&fixture)
                        && prompt_entry_text(&fixture) == "unique-hit.txt"
                },
                "rename prompt is prefilled with the displayed search hit",
            );

            assert!(fixture.press(Key::Down, ModifierType::empty()));
            wait_until_msg(
                || prompt_has_focus(&fixture) && prompt_entry_text(&fixture) == "unique-hit.txt",
                "prompt Up/Down must keep the captured rename name",
            );
            fixture
                .footer
                .prompt_entry_widget()
                .set_text("renamed-hit.txt");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until_msg(
                || {
                    fixture
                        ._directory
                        .path()
                        .join("innerdir/renamed-hit.txt")
                        .exists()
                },
                "submit must rename the captured overlay hit",
            );
            assert!(fixture._directory.path().join("a.txt").exists());
            assert!(fixture._directory.path().join("b.txt").exists());
            assert!(
                !fixture
                    ._directory
                    .path()
                    .join("innerdir/unique-hit.txt")
                    .exists()
            );
        },
    );
}

#[test]
fn minimal_empty_overlay_fill_uses_the_search_cursor() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_empty_overlay_fill_uses_the_search_cursor",
        || {
            let fixture = MinimalFixture::new();
            fixture.select_name("sub");
            submit_prompt_with_results(&fixture, Key::f, "txt", &["a.txt", "b.txt", "c.txt"]);
            let hits = overlay_listing_names(&fixture);
            assert!(hits.len() >= 3, "txt hits: {hits:?}");
            let first = hits[0].clone();
            assert!(fixture.press(Key::j, ModifierType::empty()));
            wait_until_msg(
                || {
                    fixture.view.search_hit_index() == Some(1)
                        && yank_names(&fixture).len() == 1
                        && yank_names(&fixture)
                            .first()
                            .is_some_and(|name| name != &first)
                },
                "j leaves the search cursor on a later hit",
            );
            let cursor_hit = yank_names(&fixture).into_iter().next().expect("cursor hit");
            assert_ne!(cursor_hit, "sub");
            assert_eq!(
                fixture.cursor_name().as_deref(),
                Some("sub"),
                "the hidden directory row stays on sub"
            );

            assert!(fixture.press(Key::a, ModifierType::CONTROL_MASK));
            wait_until_msg(
                || yank_names(&fixture).len() == hits.len(),
                "Ctrl+A fills every visible match",
            );
            assert!(fixture.press(Key::y, ModifierType::empty()));
            wait_until_msg(
                || {
                    let yanked = clipboard_file_names(&fixture);
                    yanked.len() == hits.len() && yanked.iter().any(|name| name == &cursor_hit)
                },
                "a non-empty overlay fill yanks every match",
            );

            assert!(fixture.press(Key::r, ModifierType::CONTROL_MASK));
            wait_until_msg(
                || yank_names(&fixture).is_empty(),
                "Ctrl+R after Ctrl+A clears the overlay fill",
            );
            assert_eq!(fixture.cursor_name().as_deref(), Some("sub"));
            assert!(fixture.press(Key::c, ModifierType::empty()));
            assert!(fixture.press(Key::c, ModifierType::empty()));
            wait_until_msg(
                || {
                    clipboard_text(&fixture)
                        .is_some_and(|text| text.contains(&cursor_hit) && !text.contains(&first))
                },
                "c c on an empty fill copies the search cursor, not hit 0",
            );

            assert!(fixture.press(Key::a, ModifierType::CONTROL_MASK));
            assert!(fixture.press(Key::r, ModifierType::CONTROL_MASK));
            wait_until_msg(
                || yank_names(&fixture).is_empty(),
                "fill is empty again before yank",
            );
            assert!(fixture.press(Key::y, ModifierType::empty()));
            wait_until_msg(
                || clipboard_file_names(&fixture) == vec![cursor_hit.clone()],
                "y on an empty fill yanks the search cursor",
            );
            assert_eq!(fixture.cursor_name().as_deref(), Some("sub"));
        },
    );
}

#[test]
fn minimal_search_l_i_enter_use_the_independent_cursor() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_search_l_i_enter_use_the_independent_cursor",
        || {
            install_text_activation_handler();
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                let fixture = find_highlight_fixture(mode);
                let opened = Rc::new(RefCell::new(Vec::<Location>::new()));
                let observed = opened.clone();
                fixture.view.browser().observe(move |event| {
                    if let BrowserEvent::OpenRequested { location } = event {
                        observed.borrow_mut().push(location.clone());
                    }
                });
                submit_prompt_with_results(&fixture, Key::s, "txt", &["a.txt", "b.txt", "c.txt"]);
                let hits = overlay_listing_names(&fixture);
                assert!(
                    hits.len() >= 2,
                    "{mode:?} search must show at least two txt hits, got {hits:?}"
                );
                let selected = hits[0].clone();
                let cursor = hits[1].clone();
                assert_eq!(yank_names(&fixture), [selected.as_str()]);

                assert!(fixture.press(Key::space, ModifierType::empty()));
                pump_mainloop(Duration::from_millis(50));
                assert_eq!(
                    yank_names(&fixture),
                    [selected.as_str()],
                    "{mode:?} Space must keep the first hit selected"
                );

                assert!(fixture.press(Key::i, ModifierType::empty()));
                wait_until_msg(
                    || {
                        fixture.preview.is_enabled()
                            && rendered_name(&fixture.preview.widget(), &cursor)
                            && !rendered_name(&fixture.preview.widget(), &selected)
                    },
                    &format!("{mode:?} i previews the cursor file, not the leftover selection"),
                );
                assert!(fixture.press(Key::i, ModifierType::empty()));
                wait_until_msg(
                    || !fixture.preview.is_enabled(),
                    &format!("{mode:?} second i closes preview"),
                );

                if mode == BrowserMode::Icons {
                    assert!(fixture.press(Key::l, ModifierType::empty()));
                    pump_mainloop(Duration::from_millis(80));
                    assert!(
                        !fixture.preview.is_enabled(),
                        "Icons search l must not preview"
                    );
                    assert!(!fixture.preview.owns_keys_chrome());
                    assert_eq!(overlay_listing_names(&fixture), hits);
                } else {
                    assert!(fixture.press(Key::l, ModifierType::empty()));
                    wait_until_msg(
                        || {
                            fixture.preview.is_enabled()
                                && rendered_name(&fixture.preview.widget(), &cursor)
                                && !rendered_name(&fixture.preview.widget(), &selected)
                        },
                        &format!("{mode:?} l previews the cursor file, not the leftover selection"),
                    );
                    assert!(fixture.press(Key::h, ModifierType::empty()));
                    wait_until_msg(
                        || overlay_listing_names(&fixture) == hits,
                        &format!("{mode:?} h returns to the search overlay"),
                    );
                }

                opened.borrow_mut().clear();
                assert!(fixture.press(Key::Return, ModifierType::empty()));
                wait_until_msg(
                    || {
                        *opened.borrow()
                            == [Location::local(fixture._directory.path().join(&cursor))]
                    },
                    &format!("{mode:?} Enter opens the cursor file, not the leftover selection"),
                );
            }
        },
    );
}

#[test]
fn minimal_icons_search_arrows_do_not_preview_or_enter() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_icons_search_arrows_do_not_preview_or_enter",
        || {
            install_text_activation_handler();
            let fixture = find_highlight_fixture(BrowserMode::Icons);
            fixture.footer.observe_browser(&fixture.view);
            add_nested_hit(&fixture);
            let root = Location::local(fixture._directory.path());
            PreferenceManager::shared().set_filter_include_subfolders(false);
            query_prompt_with_results(&fixture, Key::s, "unique-hit", &["unique-hit.txt"]);
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until_msg(
                || overlay_listing_names(&fixture) == ["unique-hit.txt"],
                "Icons recursive search retains unique-hit.txt",
            );
            fixture.view.focus_first_search_result();
            for key in [Key::l, Key::Right, Key::KP_Right, Key::h, Key::Left] {
                assert!(fixture.press(key, ModifierType::empty()));
                pump_mainloop(Duration::from_millis(50));
                assert!(
                    !fixture.preview.is_enabled(),
                    "Icons search {key:?} must not preview"
                );
                assert!(!fixture.preview.owns_keys_chrome());
                assert_eq!(fixture.view.browser().active_location(), Some(root.clone()));
                assert_eq!(overlay_listing_names(&fixture), ["unique-hit.txt"]);
            }
            assert!(fixture.press(Key::i, ModifierType::empty()));
            wait_until_msg(
                || {
                    fixture.preview.is_enabled()
                        && rendered_name(&fixture.preview.widget(), "unique-hit.txt")
                        && !fixture.preview.owns_keys_chrome()
                },
                "Icons search i toggles preview without taking keys",
            );
            fixture.preview.action().activate(None);
            fixture.view.dismiss_hidden_filter();
            fixture.view.restore_file_view_focus();
            wait_until_msg(
                || listing_cursor_has_focus(&fixture),
                "listing focus after closing the hit preview",
            );
            query_prompt_with_results(&fixture, Key::s, "innerdir", &["innerdir"]);
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until_msg(
                || overlay_listing_names(&fixture) == ["innerdir"],
                "Icons recursive search retains the innerdir name hit",
            );
            for key in [Key::l, Key::Right] {
                assert!(fixture.press(key, ModifierType::empty()));
                pump_mainloop(Duration::from_millis(80));
                assert_eq!(
                    fixture.view.browser().active_location(),
                    Some(root.clone()),
                    "Icons search {key:?} must not enter a directory hit"
                );
                assert!(!fixture.preview.is_enabled());
                assert_eq!(overlay_listing_names(&fixture), ["innerdir"]);
            }
        },
    );
}

#[test]
fn minimal_copy_name_and_path_use_search_overlay_selection() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_copy_name_and_path_use_search_overlay_selection",
        || {
            let fixture = MinimalFixture::new();
            add_nested_hit(&fixture);
            PreferenceManager::shared().set_filter_include_subfolders(false);
            fixture.select_name("a.txt");
            submit_prompt_with_results(&fixture, Key::s, "unique-hit", &["unique-hit.txt"]);
            assert_eq!(overlay_listing_names(&fixture), ["unique-hit.txt"]);
            assert_eq!(fixture.cursor_name().as_deref(), Some("a.txt"));

            assert!(fixture.press(Key::c, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "c-"));
            assert!(fixture.press(Key::n, ModifierType::empty()));
            wait_until_msg(
                || clipboard_text(&fixture).as_deref() == Some("unique-hit.txt"),
                "c n copies the overlay hit name, not the hidden listing cursor",
            );

            assert!(fixture.press(Key::c, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "c-"));
            assert!(fixture.press(Key::c, ModifierType::empty()));
            wait_until_msg(
                || {
                    clipboard_text(&fixture).is_some_and(|text| {
                        text.contains("unique-hit.txt") && !text.contains("a.txt")
                    })
                },
                "c c copies the overlay hit path, not the hidden listing cursor",
            );
        },
    );
}

#[test]
fn minimal_columns_find_skips_hidden_entries() {
    gtk_test(
        "ui::window::tests::minimal_mode::minimal_columns_find_skips_hidden_entries",
        || {
            let fixture = MinimalFixture::new();
            if fixture.view.browser().preferences().show_hidden {
                assert!(fixture.press(Key::period, ModifierType::empty()));
                wait_until(|| !fixture.view.browser().preferences().show_hidden);
            }
            std::fs::write(fixture._directory.path().join(".secret"), b"hidden")
                .expect("hidden file");
            fixture.view.refresh();
            wait_until_msg(
                || listing_has_name(&fixture, ".secret"),
                "refresh publishes .secret into the column snapshot",
            );
            fixture.select_name("a.txt");
            wait_until_msg(
                || !rendered_name(&fixture.view.widget(), ".secret"),
                ".secret must have no visible row while hidden files are off",
            );

            assert!(fixture.press(Key::slash, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "/") && prompt_has_focus(&fixture));
            fixture.footer.prompt_entry_widget().set_text("secret");
            pump_mainloop(Duration::from_millis(50));
            assert_ne!(
                fixture.cursor_name().as_deref(),
                Some(".secret"),
                "find must not select a hidden file"
            );
            assert_eq!(fixture.cursor_name().as_deref(), Some("a.txt"));
            assert!(
                !listing_selected_names(&fixture)
                    .iter()
                    .any(|name| name == ".secret"),
                "find must not fill a hidden file"
            );
            assert!(!rendered_name(&fixture.view.widget(), ".secret"));
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until(|| !footer_shows(&fixture, "/"));

            assert!(fixture.press(Key::period, ModifierType::empty()));
            wait_until(|| fixture.view.browser().preferences().show_hidden);
            wait_until_msg(
                || rendered_name(&fixture.view.widget(), ".secret"),
                "showing hidden files must render .secret",
            );
            fixture.select_name("a.txt");
            assert!(fixture.press(Key::slash, ModifierType::empty()));
            wait_until(|| footer_shows(&fixture, "/") && prompt_has_focus(&fixture));
            fixture.footer.prompt_entry_widget().set_text("secret");
            wait_until_msg(
                || fixture.cursor_name().as_deref() == Some(".secret"),
                "find may land on .secret once hidden files are visible",
            );
        },
    );
}
