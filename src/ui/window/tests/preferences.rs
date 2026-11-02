// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::*;
use crate::ui::browser_modes::BrowserMode;
use crate::ui::preferences::PreferenceManager;
use crate::ui::tenxer_mode::UNUSED_SUBTITLE;

mod browsing_prefs;
mod column_prefs;
mod folder_views;
mod item_customizations;
mod sidebar_prefs;
mod tenxer_prefs;

struct OpenWindow {
    window: gtk::ApplicationWindow,
    content: super::super::composition::WindowContent,
}

impl OpenWindow {
    fn open() -> Self {
        Self::open_with_width(1200)
    }

    fn open_with_width(width: i32) -> Self {
        let preferences = PreferenceManager::shared();
        let window = gtk::ApplicationWindow::builder()
            .application(&test_application())
            .title("Strata")
            .default_width(width)
            .default_height(760)
            .build();
        let content = super::super::composition::WindowContent::new(&window, &preferences);
        content.bind(&window, &preferences);
        window.present();
        Self { window, content }
    }
}

fn test_application() -> gtk::Application {
    if let Some(application) = gtk::gio::Application::default().and_downcast::<gtk::Application>() {
        return application;
    }
    let application = gtk::Application::new(None::<&str>, gtk::gio::ApplicationFlags::NON_UNIQUE);
    application
        .register(None::<&gtk::gio::Cancellable>)
        .expect("test application registration");
    application
}

fn write_settings(contents: &str) {
    let path = settings_file();
    std::fs::create_dir_all(path.parent().expect("settings directory"))
        .expect("settings directory");
    std::fs::write(path, contents).expect("seed settings");
}

fn settings_file() -> std::path::PathBuf {
    gtk::glib::user_config_dir().join("strata/settings.toml")
}

fn write_folder_views(contents: &str) {
    let path = crate::storage::state_directory().join("folder-views.toml");
    std::fs::create_dir_all(path.parent().expect("state directory")).expect("state directory");
    std::fs::write(path, contents).expect("seed folder settings");
}

fn folder_views_file() -> String {
    std::fs::read_to_string(crate::storage::state_directory().join("folder-views.toml"))
        .expect("folder settings file")
}

fn icons_scale(open: &OpenWindow) -> Option<gtk::Scale> {
    let mut scale = None;
    walk(open.window.upcast_ref(), &mut |widget| {
        if widget.has_css_class("icons-thumbnail-scale") {
            scale = widget.clone().downcast::<gtk::Scale>().ok();
        }
    });
    scale
}

fn icons_size(open: &OpenWindow) -> Option<i32> {
    icons_scale(open).map(|scale| scale.value().round() as i32)
}

fn load_folder(open: &OpenWindow) -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("folder fixture");
    std::fs::create_dir(directory.path().join("child")).expect("child folder");
    std::fs::write(directory.path().join("a.txt"), b"a").expect("file");
    open.content
        .browser
        .navigate_location(crate::model::Location::local(directory.path()));
    wait_until(|| !column_loading(open, 0));
    directory
}

fn open_child_column(open: &OpenWindow) {
    open.content.browser.browser().select(0, 0);
    open.content.browser.activate_focused();
    wait_until(|| !column_loading(open, 1));
}

fn column_loading(open: &OpenWindow, depth: usize) -> bool {
    open.content
        .browser
        .browser()
        .column_snapshot(depth)
        .is_none_or(|column| column.loading)
}

fn assert_operable(control: &gtk::Widget, operable: bool, what: &str) {
    assert_eq!(
        control.is_visible() && control.is_sensitive(),
        operable,
        "{what}"
    );
    if !operable {
        assert!(!control.is_sensitive(), "{what}");
    }
}

fn hidden_search_does_not_open(open: &OpenWindow) {
    open.content.search_button().emit_clicked();
    assert!(
        !open.content.search_button().has_css_class("active"),
        "hidden Search does not open"
    );
}

fn assert_pane_toolbar(open: &OpenWindow, operable: bool) {
    let mut required = vec!["Refresh (F5)"];
    if open.content.browser.view_mode() != BrowserMode::List {
        required.push("Choose sort field");
    }
    for tooltip in required {
        let controls = controls_in_shown_pane(&open.content.browser.widget(), tooltip);
        assert!(!controls.is_empty(), "{tooltip}");
        for control in controls {
            assert_operable(&control, operable, tooltip);
        }
    }
}

fn assert_filter_controls(open: &OpenWindow, operable: bool) {
    let filters = [
        "Filter this pane (Ctrl+F)",
        "Filter icons (Ctrl+F)",
        "Filter list (Ctrl+F)",
    ];
    let mut found_filter = false;
    for tooltip in filters {
        for control in controls_in_shown_pane(&open.content.browser.widget(), tooltip) {
            found_filter = true;
            assert_operable(&control, operable, tooltip);
        }
    }
    assert!(found_filter, "shown pane has a filter control");
}

fn assert_controls(open: &OpenWindow, operable: bool) {
    let search = open.content.search_button();
    assert_eq!(search.is_visible() && search.is_sensitive(), operable);
    if !operable {
        hidden_search_does_not_open(open);
    }
    if !operable && open.content.browser.view_mode() != BrowserMode::Columns {
        return;
    }
    assert_pane_toolbar(open, operable);
    assert_filter_controls(open, operable);
    let browser = open.content.browser.widget();
    for control in class_in_shown_pane(&browser, "tenxer-sort-direction") {
        assert_operable(&control, operable, "tenxer-sort-direction");
    }
    for control in controls_in_shown_pane(&browser, "Close this pane") {
        assert_operable(&control, operable, "Close this pane");
    }
}

/// The first widget below `root`, depth first, that `pick` maps to a value.
fn first_in<T>(root: &gtk::Widget, pick: impl Fn(&gtk::Widget) -> Option<T>) -> Option<T> {
    let mut found = None;
    walk(root, &mut |widget| {
        if found.is_none() {
            found = pick(widget);
        }
    });
    found
}

/// [`first_in`] each ancestor of `start` in turn, starting with `start` itself.
fn nearest_around<T>(
    start: Option<gtk::Widget>,
    pick: impl Fn(&gtk::Widget) -> Option<T>,
) -> Option<T> {
    std::iter::successors(start, gtk::Widget::parent).find_map(|widget| first_in(&widget, &pick))
}

fn settings_page_button(open: &OpenWindow, page: &str) -> gtk::Button {
    first_in(open.content.overlay().upcast_ref(), |widget| {
        let named = widget.widget_name() == page;
        named
            .then(|| widget.downcast_ref::<gtk::Button>().cloned())
            .flatten()
    })
    .unwrap_or_else(|| panic!("{page} navigation button"))
}

fn close_settings(open: &OpenWindow) {
    activate_named(open.content.overlay(), "Close settings");
    wait_until(|| settings_closed(open));
}

fn settings_closed(open: &OpenWindow) -> bool {
    class_in_shown_pane(open.content.overlay().upcast_ref(), "settings-dialog")
        .iter()
        .all(|widget| !widget.is_visible())
}

fn window_listed(window: &gtk::ApplicationWindow) -> bool {
    window.application().is_some_and(|application| {
        application
            .windows()
            .iter()
            .any(|candidate| candidate == window)
    })
}

fn press(
    window: &gtk::ApplicationWindow,
    key: gtk::gdk::Key,
    modifiers: gtk::gdk::ModifierType,
) -> bool {
    let controllers = window.observe_controllers();
    let mut handled = false;
    for index in 0..controllers.n_items() {
        if let Some(keys) = controllers
            .item(index)
            .and_downcast::<gtk::EventControllerKey>()
        {
            handled |= keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers]);
        }
    }
    handled
}

fn settle() {
    let context = gtk::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

fn settle_for(duration: std::time::Duration) {
    let main_loop = gtk::glib::MainLoop::new(None, false);
    let stop = main_loop.clone();
    gtk::glib::timeout_add_local_once(duration, move || stop.quit());
    main_loop.run();
}

fn wait_until(condition: impl Fn() -> bool) {
    wait_until_or(condition, || "timed out".to_owned());
}

/// Like [`wait_until`], with `report` describing what was seen on a timeout.
fn wait_until_or(condition: impl Fn() -> bool, report: impl Fn() -> String) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "{}", report());
        settle_for(std::time::Duration::from_millis(20));
    }
}

fn sidebar_report(windows: &[&OpenWindow], manager: &PreferenceManager) -> String {
    let windows = windows
        .iter()
        .enumerate()
        .map(|(index, open)| {
            format!(
                "window {index}: toggle active={} visible={} rail={}",
                open.content.sidebar_toggle().is_active(),
                open.content.sidebar_visible(),
                open.content.sidebar.state.rail.get()
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "{windows}; saved expanded={} compact={}",
        manager.sidebar_expanded(),
        manager.sidebar_compact()
    )
}

fn filter_buttons(open: &OpenWindow) -> Vec<gtk::ToggleButton> {
    controls_in_shown_pane(&open.content.browser.widget(), "Filter this pane (Ctrl+F)")
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::ToggleButton>().ok())
        .collect()
}

fn assert_icon_only_tooltips(root: &gtk::Widget) {
    walk(root, &mut |widget| {
        if !widget.is_visible() || widget.tooltip_text().is_none() {
            return;
        }
        assert!(
            widget.is::<gtk::Button>() || widget.is::<gtk::MenuButton>(),
            "non-button tooltip: {}",
            widget.type_().name()
        );
        walk(widget, &mut |child| {
            if let Some(label) = child.downcast_ref::<gtk::Label>() {
                assert!(
                    !label.is_visible() || label.text().is_empty(),
                    "labelled control has a tooltip: {}",
                    label.text()
                );
            }
        });
    });
}

fn controls_in_shown_pane(root: &gtk::Widget, tooltip: &str) -> Vec<gtk::Widget> {
    widgets_in_shown_pane(root, |widget| {
        widget.tooltip_text().as_deref() == Some(tooltip)
            || widget.downcast_ref::<gtk::Entry>().is_some_and(|entry| {
                tooltip == "Filter by name. Use * for any characters: *.png, IMG*, or IMG*.png."
                    && entry.has_css_class("column-filter-entry")
            })
    })
}

fn class_in_shown_pane(root: &gtk::Widget, class: &str) -> Vec<gtk::Widget> {
    widgets_in_shown_pane(root, |widget| widget.has_css_class(class))
}

fn widgets_in_shown_pane(
    root: &gtk::Widget,
    matches: impl Fn(&gtk::Widget) -> bool,
) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    walk(root, &mut |widget| {
        if matches(widget) && widget.parent().is_some_and(|parent| parent.is_visible()) {
            found.push(widget.clone());
        }
    });
    found
}

fn revealer_for_filter(button: &gtk::Widget) -> gtk::Revealer {
    nearest_around(Some(button.clone()), |child| {
        child
            .has_css_class("tenxer-filter-revealer")
            .then(|| child.clone().downcast::<gtk::Revealer>().ok())
            .flatten()
    })
    .expect("filter revealer")
}

fn list_heading(root: &gtk::Widget, name: &str) -> gtk::Button {
    let mut found = None;
    walk(root, &mut |widget| {
        if found.is_some() || !widget.has_css_class("list-heading-button") {
            return;
        }
        let mut label = None;
        walk(widget, &mut |child| {
            if label.is_none()
                && child
                    .downcast_ref::<gtk::Label>()
                    .is_some_and(|label| label.text() == name)
            {
                label = Some(child.clone());
            }
        });
        if label.is_some() {
            found = widget.clone().downcast::<gtk::Button>().ok();
        }
    });
    found.expect("list heading")
}

fn switch_named(root: &impl gtk::prelude::IsA<gtk::Widget>, title: &str) -> gtk::Switch {
    let label = label_named(root.upcast_ref(), title);
    nearest_around(label.parent(), |child| {
        child.clone().downcast::<gtk::Switch>().ok()
    })
    .unwrap_or_else(|| panic!("switch for {title}"))
}

fn description_named(root: &impl gtk::prelude::IsA<gtk::Widget>, title: &str) -> String {
    let label = label_named(root.upcast_ref(), title);
    nearest_around(label.parent(), |child| {
        child
            .has_css_class("settings-option-description")
            .then(|| {
                child
                    .downcast_ref::<gtk::Label>()
                    .map(|description| description.text().to_string())
            })
            .flatten()
    })
    .unwrap_or_else(|| panic!("description for {title}"))
}

fn activate_named(root: &impl gtk::prelude::IsA<gtk::Widget>, tooltip: &str) {
    let button = controls_in_shown_pane(root.upcast_ref(), tooltip)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("button {tooltip}"));
    button
        .downcast_ref::<gtk::Button>()
        .unwrap_or_else(|| panic!("button {tooltip}"))
        .emit_clicked();
}

fn label_named(root: &gtk::Widget, text: &str) -> gtk::Label {
    first_in(root, |widget| {
        widget
            .downcast_ref::<gtk::Label>()
            .filter(|label| label.text() == text && label.is_visible())
            .cloned()
    })
    .unwrap_or_else(|| panic!("label {text}"))
}

fn focusable_outside_the_browser(open: &OpenWindow) -> gtk::Widget {
    let browser = open.content.browser.widget();
    first_in(open.window.upcast_ref(), |widget| {
        let focusable = widget.is_focusable() && widget.is_mapped();
        let outside = *widget != browser && !widget.is_ancestor(&browser);
        (focusable && outside).then(|| widget.clone())
    })
    .expect("a focusable widget outside the browser")
}

fn walk(widget: &gtk::Widget, visit: &mut impl FnMut(&gtk::Widget)) {
    visit(widget);
    let mut child = widget.first_child();
    while let Some(widget) = child {
        walk(&widget, visit);
        child = widget.next_sibling();
    }
}

fn texture_pixels(texture: &gtk::gdk::Texture) -> Vec<u8> {
    let stride = texture.width() as usize * 4;
    let mut pixels = vec![0; stride * texture.height() as usize];
    texture.download(&mut pixels, stride);
    pixels
}

fn loaded_entry(open: &OpenWindow, path: &std::path::Path) -> crate::model::FileEntry {
    let location = crate::model::Location::local(path);
    let names = std::collections::HashSet::from([location.display_name()]);
    let browser = open.content.browser.browser();
    let find = || {
        browser
            .entries_named(&names)
            .into_iter()
            .find(|entry| entry.location == location)
    };
    wait_until(|| find().is_some());
    find().expect("loaded entry")
}
