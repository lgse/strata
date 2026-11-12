// SPDX-License-Identifier: MIT

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gdk::{Key, ModifierType};

use crate::app::BrowserEvent;
use crate::model::{SortDirection, SortKey};

use super::super::*;
use crate::services::{
    LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
};
mod audio_tracks;
mod context_menus;
mod direct_commands;
mod entry_keys;
mod escape_precedence;
mod file_commands;
mod file_verbs;
mod filter_focus;
mod first_column;
mod folder_jump;
mod footer_prompt;
mod go_prompt;
mod icon_motion;
mod jump_keys;
mod keymap_conflicts;
mod keymap_overrides;
mod letter_bookmarks;
mod list_motion;
mod mode_exit;
mod open_in_new;
mod overlay_focus;
mod pane_focus;
mod place_chords;
mod preview_ownership;
mod reload_cursor;
mod sidebar_sections;
mod structural_aliases;
mod tenxer_fill;
mod tenxer_panes;
mod tenxer_shortcuts;
mod thumbnail_size;
mod video_clips;
mod window_support;

use window_support::{
    child_commands, composed_window, default_recorder_app, press_phase, recorder_app,
    settings_layer,
};

use crate::ui::{
    preview::{DocumentScroll, PreviewDrawer},
    shortcut_footer::ShortcutFooter,
    top_bar_navigation::TopBarNavigation,
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
    history: Rc<crate::services::NavigationHistory>,
    _directory: tempfile::TempDir,
    _history_directory: tempfile::TempDir,
}

impl KeyboardFixture {
    fn new() -> Self {
        Self::with_provider(Rc::new(TextPreview))
    }

    fn with_provider(provider: Rc<dyn crate::services::PreviewProvider>) -> Self {
        Self::with_parts(provider, browser_for_window)
    }

    /// `view` runs after the preferences are seeded, which it reads on creation.
    fn with_parts(
        provider: Rc<dyn crate::services::PreviewProvider>,
        view: impl FnOnce() -> BrowserView,
    ) -> Self {
        let preferences = seed_fixture_preferences();
        let directory = fixture_directory();
        let history_directory = tempfile::tempdir().expect("history fixture");
        let history = Rc::new(crate::services::NavigationHistory::open(
            history_directory.path().join("history.json"),
        ));
        let view = view();
        view.set_view_mode(BrowserMode::Columns);
        let sidebar = build_sidebar(view.clone(), preferences.clone(), true);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let toggle = gtk::ToggleButton::builder().active(true).build();
        header.append(&toggle);
        header.append(&view.location_widget());
        let top_bar = TopBarNavigation::new(&header, &sidebar.widget, &toggle);
        let preview = PreviewDrawer::new(provider, false);
        let shortcuts = ShortcutFooter::new(BrowserMode::Columns);
        let overlay = fixture_overlay(
            &header,
            [
                sidebar.widget.upcast_ref(),
                &view.widget(),
                &preview.widget(),
            ],
            &shortcuts,
        );
        let window = gtk::ApplicationWindow::builder()
            .child(&overlay)
            .default_width(1000)
            .default_height(600)
            .build();
        keyboard::install(
            &window,
            &sidebar,
            keyboard::DispatcherParts {
                view: view.clone(),
                top_bar,
                preview: preview.clone(),
                type_to_search: TypeToSearch {
                    view: view.clone(),
                    preferences,
                },
                shortcuts: shortcuts.clone(),
                history: history.clone(),
            },
        );
        let keys = window_key_controller(&window);
        window.present();
        open_fixture_directory(&view, directory.path());
        Self {
            window,
            overlay,
            view,
            sidebar,
            preview,
            sidebar_toggle: toggle,
            shortcuts: shortcuts.clone(),
            keys,
            history,
            _directory: directory,
            _history_directory: history_directory,
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

fn seed_fixture_preferences() -> Rc<PreferenceManager> {
    PreferenceManager::seed_saved_preferences_for_test();
    let preferences = PreferenceManager::shared();
    preferences.set_sidebar_show_home(true);
    preferences.set_sidebar_pinned_collapsed(false);
    preferences.set_sidebar_devices_collapsed(false);
    preferences.set_tenxer_mode(false);
    preferences
}

fn fixture_directory() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("fixture");
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(directory.path().join(name), b"preview").expect("fixture file");
    }
    directory
}

fn fixture_overlay<const N: usize>(
    header: &gtk::Box,
    panes: [&gtk::Widget; N],
    shortcuts: &ShortcutFooter,
) -> gtk::Overlay {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    for pane in panes {
        row.append(pane);
    }
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(header);
    content.append(&row);
    content.append(shortcuts.widget());
    gtk::Overlay::builder().child(&content).build()
}

fn window_key_controller(window: &gtk::ApplicationWindow) -> gtk::EventControllerKey {
    let controllers = window.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .next()
        .expect("key controller")
}

fn open_fixture_directory(view: &BrowserView, directory: &std::path::Path) {
    view.browser().navigate(Location::local(directory));
    wait_until(|| {
        view.browser()
            .column_snapshot(0)
            .is_some_and(|column| !column.loading)
    });
    view.browser().select(0, 0);
    view.browser().focus_active();
    wait_until(|| view.item_view_has_focus() && rendered_name(&view.widget(), "a.txt"));
}

/// A real Tab press reaches the dispatcher first and GTK's `move-focus` binding only when
/// the dispatcher lets it through; `KeyboardFixture::press` emits just the former.
fn tab(fixture: &KeyboardFixture, direction: gtk::DirectionType) {
    let (key, modifiers) = if direction == gtk::DirectionType::TabBackward {
        (Key::ISO_Left_Tab, ModifierType::SHIFT_MASK)
    } else {
        (Key::Tab, ModifierType::empty())
    };
    if !fixture.press(key, modifiers) {
        fixture.window.emit_move_focus(direction);
    }
    settle_idles();
    pump(50);
}

/// A Tab into the listing hands focus to the cursor at idle, which a slow
/// frame can hold back past a fixed wait.
fn settle_idles() {
    let settled = Rc::new(Cell::new(false));
    let flag = settled.clone();
    glib::idle_add_local_once(move || flag.set(true));
    wait_until(|| settled.get());
}

fn focused_widget_shows(window: &gtk::ApplicationWindow, name: &str) -> bool {
    let Some(focused) = gtk::prelude::RootExt::focus(window) else {
        return false;
    };
    if focused.is::<gtk::ListView>() || focused.is::<gtk::GridView>() {
        return false;
    }
    rendered_name(&focused, name)
        && ["a.txt", "b.txt", "c.txt"]
            .into_iter()
            .filter(|candidate| *candidate != name && rendered_name(&focused, candidate))
            .count()
            == 0
}

fn settled_on_screen(widget: &gtk::Widget) -> bool {
    let transitioning = widget
        .downcast_ref::<gtk::Stack>()
        .is_some_and(|stack| stack.is_transition_running());
    widget.is_mapped() && widget.width() > 0 && !transitioning
}

fn shows_text(widget: &gtk::Widget, name: &str) -> bool {
    let label = widget
        .downcast_ref::<gtk::Label>()
        .is_some_and(|label| label.label() == name);
    label
        || widget
            .downcast_ref::<gtk::Inscription>()
            .is_some_and(|label| label.text().as_deref() == Some(name))
}

fn children(widget: &gtk::Widget) -> impl Iterator<Item = gtk::Widget> {
    std::iter::successors(widget.first_child(), gtk::Widget::next_sibling)
}

fn rendered_name(widget: &gtk::Widget, name: &str) -> bool {
    if !settled_on_screen(widget) {
        return false;
    }
    shows_text(widget, name) || children(widget).any(|child| rendered_name(&child, name))
}

fn widget_with_class(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if widget.has_css_class(class) {
        return Some(widget.clone());
    }
    children(widget).find_map(|child| widget_with_class(&child, class))
}

fn wait_until(condition: impl Fn() -> bool) {
    wait_until_or(condition, || "keyboard fixture did not settle".to_owned());
}

/// Like [`wait_until`], with `report` describing what was seen on a timeout.
fn wait_until_or(condition: impl Fn() -> bool, report: impl Fn() -> String) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "{}", report());
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn press_file_row(widget: &gtk::Widget, name: &str) -> bool {
    let Some(row) = file_row_named(widget, name) else {
        return false;
    };
    let controllers = row.observe_controllers();
    let Some(click) = (0..controllers.n_items()).find_map(|index| {
        controllers
            .item(index)
            .and_then(|controller| controller.downcast::<gtk::GestureClick>().ok())
            .filter(|click| click.button() == 1)
    }) else {
        return false;
    };
    click.emit_by_name::<()>("pressed", &[&1i32, &8.0f64, &8.0f64]);
    true
}

fn file_row_named(widget: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if widget.has_css_class("file-row") && rendered_name(widget, name) {
        return Some(widget.clone());
    }
    children(widget).find_map(|child| file_row_named(&child, name))
}

fn fill_names(browser: &crate::app::Browser) -> Vec<String> {
    let mut names: Vec<_> = browser
        .selected_entries()
        .into_iter()
        .map(|entry| entry.display_name)
        .collect();
    names.sort();
    names
}

fn preview_has_focus(fixture: &KeyboardFixture) -> bool {
    gtk::prelude::RootExt::focus(&fixture.window).is_some_and(|focused| {
        let preview = fixture.preview.widget();
        focused == preview || focused.is_ancestor(&preview)
    })
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

fn open_empty_folder(fixture: &KeyboardFixture) {
    let empty = fixture._directory.path().join("empty");
    if !empty.exists() {
        std::fs::create_dir(&empty).expect("empty folder");
    }
    let browser = fixture.view.browser();
    browser.navigate(Location::local(&empty));
    wait_until(|| browser.active_location() == Some(Location::local(&empty)));
    wait_loaded(&browser, 0);
    focus_files(fixture);
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

fn file_list_model(widget: &gtk::Widget) -> Option<gtk::SelectionModel> {
    let list = widget.downcast_ref::<gtk::ListView>()?;
    let file_list = list.is_visible() && list.has_css_class("file-list-mode");
    list.model().filter(|_| file_list)
}

fn row_display_name(model: &gtk::SelectionModel, position: u32) -> Option<String> {
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
}

fn list_display_names(widget: &gtk::Widget) -> Vec<String> {
    if let Some(model) = file_list_model(widget) {
        return (0..model.n_items())
            .filter_map(|position| row_display_name(&model, position))
            .collect();
    }
    children(widget)
        .map(|child| list_display_names(&child))
        .find(|names| !names.is_empty())
        .unwrap_or_default()
}

/// The mapped rows outlined as the one Enter in a filter field opens.
fn marked_rows(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut marked = Vec::new();
    if widget.is_mapped() && widget.has_css_class(crate::ui::enter_target::CSS_CLASS) {
        marked.push(widget.clone());
    }
    for child in std::iter::successors(widget.first_child(), gtk::Widget::next_sibling) {
        marked.extend(marked_rows(&child));
    }
    marked
}

fn pump(millis: u64) {
    let deadline = Instant::now() + Duration::from_millis(millis);
    while Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn modal_visible(overlay: &gtk::Overlay) -> bool {
    children(overlay.upcast_ref())
        .any(|widget| widget.is_visible() && widget.has_css_class("app-modal-layer"))
}

fn clickable_button<'a>(widget: &'a gtk::Widget, class: &str) -> Option<&'a gtk::Button> {
    let shown = widget.is_visible() && widget.has_css_class(class);
    widget.downcast_ref::<gtk::Button>().filter(|_| shown)
}

fn click_class(widget: &impl IsA<gtk::Widget>, class: &str) -> bool {
    let widget = widget.as_ref();
    if let Some(button) = clickable_button(widget, class) {
        button.emit_clicked();
        return true;
    }
    children(widget).any(|child| click_class(&child, class))
}

fn visible_menu(widget: &gtk::Widget) -> Option<gtk::PopoverMenu> {
    let menu = widget
        .downcast_ref::<gtk::PopoverMenu>()
        .filter(|_| widget.is_visible());
    if let Some(menu) = menu {
        return Some(menu.clone());
    }
    children(widget).find_map(|child| visible_menu(&child))
}

/// `/` as typed: the window leaves the key to the focused field's input method,
/// which commits it.
fn type_slash(fixture: &KeyboardFixture) {
    assert!(
        !fixture.press(Key::slash, ModifierType::empty()),
        "the input method gets /"
    );
    crate::test_support::commit_text(&focused_entry(&fixture.window), "/");
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

fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// Asserts the cursor sits on `cursor` while the fill is exactly `fill`.
fn assert_cursor_fill(
    browser: &crate::app::Browser,
    cursor: &str,
    fill: &[&str],
    context: impl std::fmt::Display,
) {
    assert_eq!(
        (focused_name(browser), fill_names(browser)),
        (cursor.to_owned(), owned(fill)),
        "{context}: (cursor, fill)"
    );
}

fn focused_depth(browser: &crate::app::Browser) -> Option<usize> {
    browser.focused_item().map(|(depth, _, _)| depth)
}
