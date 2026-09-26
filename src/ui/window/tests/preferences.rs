// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::*;
use crate::ui::browser_modes::BrowserMode;
use crate::ui::preferences::PreferenceManager;
use crate::ui::tenxer_mode::UNUSED_SUBTITLE;

#[test]
fn default_chrome_stays_operable_without_a_saved_tenxer_mode() {
    gtk_test(
        "ui::window::tests::preferences::default_chrome_stays_operable_without_a_saved_tenxer_mode",
        || {
            let manager = PreferenceManager::shared();
            assert!(!manager.tenxer_mode());
            let open = OpenWindow::open();
            let _directory = load_folder(&open);
            assert!(!open.content.footer().tag_visible());
            assert_controls(&open, true);
            press(
                &open.window,
                gtk::gdk::Key::q,
                gtk::gdk::ModifierType::empty(),
            );
            assert!(window_listed(&open.window));
            assert!(!manager.tenxer_mode());
            assert!(!open.content.footer().tag_visible());
        },
    );
}

#[test]
fn saved_tenxer_mode_applies_before_settings_and_to_lazy_views() {
    gtk_test(
        "ui::window::tests::preferences::saved_tenxer_mode_applies_before_settings_and_to_lazy_views",
        || {
            write_settings("tenxer_mode = true\n");
            let manager = PreferenceManager::shared();
            assert!(manager.tenxer_mode());
            let first = OpenWindow::open();
            let second = OpenWindow::open();
            assert!(settings_closed(&first));
            assert!(settings_closed(&second));
            assert!(first.content.footer().tag_visible());
            assert!(second.content.footer().tag_visible());
            let directory = load_folder(&first);
            second
                .content
                .browser
                .navigate_location(crate::model::Location::local(directory.path()));
            wait_until(|| !column_loading(&second, 0));
            for open in [&first, &second] {
                assert_controls(open, false);
                assert!(open.content.close_button().is_visible());
                assert!(open.content.close_button().is_sensitive());
            }
            open_child_column(&first);
            assert!(
                !controls_in_shown_pane(&first.content.browser.widget(), "Close this pane")
                    .is_empty()
            );
            assert_controls(&first, false);
            first.content.browser.set_view_mode(BrowserMode::Icons);
            wait_until(|| !column_loading(&first, 0));
            assert_controls(&first, false);
            first.content.browser.set_view_mode(BrowserMode::List);
            wait_until(|| !column_loading(&first, 0));
            assert_controls(&first, false);
            let heading = list_heading(&first.content.browser.widget(), "Name");
            assert!(heading.is_visible() && heading.is_sensitive());
            let depth = first
                .content
                .browser
                .browser()
                .active_depth()
                .expect("list depth");
            let before = first
                .content
                .browser
                .browser()
                .column_preferences(depth)
                .expect("list preferences")
                .sort_direction;
            heading.emit_clicked();
            settle_for(std::time::Duration::from_millis(50));
            let after = first
                .content
                .browser
                .browser()
                .column_preferences(depth)
                .expect("sorted list preferences")
                .sort_direction;
            assert_ne!(before, after);
            manager.set_tenxer_mode(false);
            settle();
            assert_controls(&first, true);
            assert!(!first.content.footer().tag_visible());
            drop(directory);
        },
    );
}

#[test]
fn browsing_control_and_shortcut_update_both_windows() {
    gtk_test(
        "ui::window::tests::preferences::browsing_control_and_shortcut_update_both_windows",
        || {
            let manager = PreferenceManager::shared();
            let first = OpenWindow::open();
            let second = OpenWindow::open();
            let directory = load_folder(&first);
            second
                .content
                .browser
                .navigate_location(crate::model::Location::local(directory.path()));
            wait_until(|| !column_loading(&second, 0));
            press(
                &first.window,
                gtk::gdk::Key::m,
                gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
            );
            settle();
            assert!(manager.tenxer_mode());
            assert_controls(&first, false);
            assert_controls(&second, false);
            assert!(first.content.footer().tag_visible());
            assert!(second.content.footer().tag_visible());
            first.content.settings_button().emit_clicked();
            second.content.settings_button().emit_clicked();
            settle();
            let first_switch = switch_named(first.content.overlay(), "10xer mode");
            let second_switch = switch_named(second.content.overlay(), "10xer mode");
            assert!(first_switch.is_active());
            assert!(second_switch.is_active());
            second_switch.set_active(false);
            settle();
            assert!(!manager.tenxer_mode());
            assert!(!first_switch.is_active());
            assert_controls(&first, true);
            assert_controls(&second, true);
            assert!(!second.content.footer().tag_visible());
            let saved = std::fs::read_to_string(settings_file()).expect("saved settings");
            assert!(saved.contains("tenxer_mode = false"));
        },
    );
}

#[test]
fn q_leaves_tenxer_and_shift_q_closes_only_the_current_window() {
    gtk_test(
        "ui::window::tests::preferences::q_leaves_tenxer_and_shift_q_closes_only_the_current_window",
        || {
            let manager = PreferenceManager::shared();
            let first = OpenWindow::open();
            let second = OpenWindow::open();
            manager.set_tenxer_mode(true);
            settle();
            press(
                &first.window,
                gtk::gdk::Key::q,
                gtk::gdk::ModifierType::empty(),
            );
            settle();
            assert!(!manager.tenxer_mode());
            assert!(window_listed(&first.window));
            assert!(window_listed(&second.window));
            assert!(!first.content.footer().tag_visible());
            assert!(!second.content.footer().tag_visible());
            manager.set_tenxer_mode(true);
            settle();
            press(
                &first.window,
                gtk::gdk::Key::Q,
                gtk::gdk::ModifierType::SHIFT_MASK,
            );
            settle();
            assert!(!window_listed(&first.window));
            assert!(window_listed(&second.window));
            assert!(manager.tenxer_mode());
            assert!(second.content.footer().tag_visible());
        },
    );
}

#[test]
fn browsing_preferences_stay_saved_but_unused_until_exit() {
    gtk_test(
        "ui::window::tests::preferences::browsing_preferences_stay_saved_but_unused_until_exit",
        || {
            let manager = PreferenceManager::shared();
            manager.set_type_to_search(true);
            manager.set_arrow_navigation_scoped(true);
            manager.set_columns_mirror_selection(true);
            let open = OpenWindow::open();
            let directory = load_folder(&open);
            assert!(open.content.browser.columns_mirror_selection_enabled());
            open.content.browser.browser().focus_active();
            wait_until(|| open.content.browser.item_view_has_focus());
            press(
                &open.window,
                gtk::gdk::Key::x,
                gtk::gdk::ModifierType::empty(),
            );
            settle();
            assert!(
                filter_buttons(&open)
                    .iter()
                    .any(|button| button.is_active()),
                "type to search filters before 10xer is enabled"
            );
            for button in filter_buttons(&open) {
                if button.is_active() {
                    button.set_active(false);
                }
            }
            open.content.settings_button().emit_clicked();
            settle();
            for title in [
                "Type to search",
                "Keep arrows in file list",
                "Mirror columns selection",
            ] {
                let switch = switch_named(open.content.overlay(), title);
                assert!(switch.is_active() && switch.is_sensitive());
                assert_ne!(
                    description_named(open.content.overlay(), title),
                    UNUSED_SUBTITLE
                );
            }
            manager.set_tenxer_mode(true);
            settle();
            assert!(manager.type_to_search());
            assert!(manager.arrow_navigation_scoped());
            assert!(manager.columns_mirror_selection());
            assert!(!open.content.browser.columns_mirror_selection_enabled());
            for title in [
                "Type to search",
                "Keep arrows in file list",
                "Mirror columns selection",
            ] {
                let switch = switch_named(open.content.overlay(), title);
                assert!(switch.is_active() && switch.is_sensitive());
                assert_eq!(
                    description_named(open.content.overlay(), title),
                    UNUSED_SUBTITLE
                );
            }
            let type_to_search = switch_named(open.content.overlay(), "Type to search");
            type_to_search.set_active(false);
            settle();
            assert!(!manager.type_to_search());
            type_to_search.set_active(true);
            settle();
            assert!(manager.type_to_search());
            assert!(!open.content.browser.columns_mirror_selection_enabled());
            close_settings(&open);
            open.content.browser.browser().focus_active();
            wait_until(|| open.content.browser.item_view_has_focus());
            press(
                &open.window,
                gtk::gdk::Key::x,
                gtk::gdk::ModifierType::empty(),
            );
            settle();
            assert!(
                filter_buttons(&open)
                    .iter()
                    .all(|button| !button.is_active()),
                "typing does not filter while 10xer is on"
            );
            manager.set_tenxer_mode(false);
            settle();
            assert!(manager.type_to_search());
            assert!(open.content.browser.columns_mirror_selection_enabled());
            open.content.settings_button().emit_clicked();
            settle();
            for title in [
                "Type to search",
                "Keep arrows in file list",
                "Mirror columns selection",
            ] {
                assert_ne!(
                    description_named(open.content.overlay(), title),
                    UNUSED_SUBTITLE
                );
            }
            close_settings(&open);
            open.content.browser.set_view_mode(BrowserMode::Columns);
            wait_until(|| !column_loading(&open, 0));
            open.content.browser.browser().focus_active();
            wait_until(|| open.content.browser.item_view_has_focus());
            press(
                &open.window,
                gtk::gdk::Key::x,
                gtk::gdk::ModifierType::empty(),
            );
            settle();
            assert!(
                filter_buttons(&open)
                    .iter()
                    .any(|button| button.is_active()),
                "type to search filters again after leaving 10xer"
            );
            drop(directory);
        },
    );
}

#[test]
fn entering_tenxer_hides_an_open_filter_and_leaving_restores_it() {
    gtk_test(
        "ui::window::tests::preferences::entering_tenxer_hides_an_open_filter_and_leaving_restores_it",
        || {
            let manager = PreferenceManager::shared();
            let open = OpenWindow::open();
            let directory = load_folder(&open);
            let filter =
                controls_in_shown_pane(&open.content.browser.widget(), "Filter this pane (Ctrl+F)")
                    .into_iter()
                    .next()
                    .expect("filter button");
            filter
                .downcast_ref::<gtk::ToggleButton>()
                .expect("filter toggle")
                .emit_clicked();
            settle();
            let revealer = revealer_for_filter(&filter);
            assert!(revealer.reveals_child());
            manager.set_tenxer_mode(true);
            settle();
            assert!(!revealer.reveals_child());
            assert_controls(&open, false);
            manager.set_tenxer_mode(false);
            settle();
            assert!(revealer.reveals_child());
            assert_controls(&open, true);
            drop(directory);
        },
    );
}

#[test]
fn ctrl_f_cannot_activate_hidden_filter_or_displace_existing_column_filter() {
    gtk_test(
        "ui::window::tests::preferences::ctrl_f_cannot_activate_hidden_filter_or_displace_existing_column_filter",
        || {
            let manager = PreferenceManager::shared();
            let open = OpenWindow::open();
            let _directory = load_folder(&open);
            let first = filter_buttons(&open)
                .into_iter()
                .next()
                .expect("first filter");
            let first_revealer = revealer_for_filter(first.upcast_ref());
            manager.set_tenxer_mode(true);
            press(
                &open.window,
                gtk::gdk::Key::f,
                gtk::gdk::ModifierType::CONTROL_MASK,
            );
            settle();
            assert!(!first.is_active());
            manager.set_tenxer_mode(false);
            settle();
            assert!(!first_revealer.reveals_child());

            open_child_column(&open);
            first.set_active(true);
            settle();
            assert!(first.is_active());
            manager.set_tenxer_mode(true);
            press(
                &open.window,
                gtk::gdk::Key::f,
                gtk::gdk::ModifierType::CONTROL_MASK,
            );
            settle();
            assert!(first.is_active(), "existing filter remains active");
            assert_eq!(
                filter_buttons(&open)
                    .iter()
                    .filter(|button| button.is_active())
                    .count(),
                1
            );
            manager.set_tenxer_mode(false);
            settle();
            assert!(first_revealer.reveals_child());
        },
    );
}

struct OpenWindow {
    window: gtk::ApplicationWindow,
    content: super::super::composition::WindowContent,
}

impl OpenWindow {
    fn open() -> Self {
        let preferences = PreferenceManager::shared();
        let window = gtk::ApplicationWindow::builder()
            .application(&test_application())
            .title("Strata")
            .default_width(1200)
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

fn assert_controls(open: &OpenWindow, operable: bool) {
    assert_eq!(
        open.content.search_button().is_visible() && open.content.search_button().is_sensitive(),
        operable
    );
    if !operable {
        open.content.search_button().emit_clicked();
        assert!(
            !open.content.search_button().has_css_class("active"),
            "hidden Search does not open"
        );
    }
    let mut required = vec!["Refresh (F5)"];
    if open.content.browser.view_mode() != BrowserMode::List {
        required.push("Choose sort field");
    }
    for tooltip in required {
        let controls = controls_in_shown_pane(&open.content.browser.widget(), tooltip);
        assert!(!controls.is_empty(), "{tooltip}");
        for control in controls {
            assert_eq!(
                control.is_visible() && control.is_sensitive(),
                operable,
                "{tooltip}"
            );
            if !operable {
                assert!(!control.is_sensitive(), "{tooltip}");
            }
        }
    }
    let filters = [
        "Filter this pane (Ctrl+F)",
        "Filter icons (Ctrl+F)",
        "Filter list (Ctrl+F)",
    ];
    let mut found_filter = false;
    for tooltip in filters {
        for control in controls_in_shown_pane(&open.content.browser.widget(), tooltip) {
            found_filter = true;
            assert_eq!(
                control.is_visible() && control.is_sensitive(),
                operable,
                "{tooltip}"
            );
            if !operable {
                assert!(!control.is_sensitive(), "{tooltip}");
            }
        }
    }
    assert!(found_filter, "shown pane has a filter control");
    for control in class_in_shown_pane(&open.content.browser.widget(), "tenxer-sort-direction") {
        assert_eq!(control.is_visible() && control.is_sensitive(), operable);
        if !operable {
            assert!(!control.is_sensitive());
        }
    }
    for control in controls_in_shown_pane(&open.content.browser.widget(), "Close this pane") {
        assert_eq!(control.is_visible() && control.is_sensitive(), operable);
        if !operable {
            assert!(!control.is_sensitive());
        }
    }
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
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "timed out");
        settle_for(std::time::Duration::from_millis(20));
    }
}

fn filter_buttons(open: &OpenWindow) -> Vec<gtk::ToggleButton> {
    controls_in_shown_pane(&open.content.browser.widget(), "Filter this pane (Ctrl+F)")
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::ToggleButton>().ok())
        .collect()
}

fn controls_in_shown_pane(root: &gtk::Widget, tooltip: &str) -> Vec<gtk::Widget> {
    widgets_in_shown_pane(root, |widget| {
        widget.tooltip_text().as_deref() == Some(tooltip)
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
    let mut current = Some(button.clone());
    while let Some(widget) = current {
        let mut found = None;
        walk(&widget, &mut |child| {
            if found.is_none()
                && child.has_css_class("tenxer-filter-revealer")
                && let Ok(revealer) = child.clone().downcast::<gtk::Revealer>()
            {
                found = Some(revealer);
            }
        });
        if let Some(revealer) = found {
            return revealer;
        }
        current = widget.parent();
    }
    panic!("filter revealer");
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
    let mut current = label.parent();
    while let Some(widget) = current {
        let mut found = None;
        walk(&widget, &mut |child| {
            if found.is_none()
                && let Ok(switch) = child.clone().downcast::<gtk::Switch>()
            {
                found = Some(switch);
            }
        });
        if let Some(switch) = found {
            return switch;
        }
        current = widget.parent();
    }
    panic!("switch for {title}");
}

fn description_named(root: &impl gtk::prelude::IsA<gtk::Widget>, title: &str) -> String {
    let label = label_named(root.upcast_ref(), title);
    let mut current = label.parent();
    while let Some(widget) = current {
        let mut found = None;
        walk(&widget, &mut |child| {
            if found.is_none()
                && child.has_css_class("settings-option-description")
                && let Some(description) = child.downcast_ref::<gtk::Label>()
            {
                found = Some(description.text().to_string());
            }
        });
        if let Some(text) = found {
            return text;
        }
        current = widget.parent();
    }
    panic!("description for {title}");
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
    let mut found = None;
    walk(root, &mut |widget| {
        if found.is_none()
            && let Some(label) = widget.downcast_ref::<gtk::Label>()
            && label.text() == text
            && label.is_visible()
        {
            found = Some(label.clone());
        }
    });
    found.unwrap_or_else(|| panic!("label {text}"))
}

fn walk(widget: &gtk::Widget, visit: &mut impl FnMut(&gtk::Widget)) {
    visit(widget);
    let mut child = widget.first_child();
    while let Some(widget) = child {
        walk(&widget, visit);
        child = widget.next_sibling();
    }
}
