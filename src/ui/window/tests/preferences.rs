// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::super::{home_directory, startup_location};
use super::*;
use crate::ui::browser_modes::{BrowserDensity, BrowserMode, ClickActivation, ClickCount};
use crate::ui::omastrata_mode::UNUSED_SUBTITLE;

#[test]
fn live_preferences_reach_existing_and_future_browsers_but_preserve_chooser_policy() {
    gtk_test(
        "ui::window::tests::preferences::live_preferences_reach_existing_and_future_browsers_but_preserve_chooser_policy",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            let first = browser_for_window();
            let second = browser_for_window();
            let chooser = crate::ui::browser::BrowserView::new_chooser(
                std::rc::Rc::new(crate::adapters::LocalFileSource),
                false,
            );
            for enabled in [true, false, true] {
                manager.set_folder_peeking(enabled);
                manager.set_single_click_previews(enabled);
                manager.set_columns_mirror_selection(enabled);
                manager.set_group_by_type(enabled);
                manager.set_auto_refresh_interval(if enabled { 60 } else { 0 });
                manager.set_browser_density(if enabled {
                    BrowserDensity::Compact
                } else {
                    BrowserDensity::Airy
                });
                manager.set_icons_thumbnail_size(if enabled { 128 } else { 64 });
                for mode in [BrowserMode::Icons, BrowserMode::List, BrowserMode::Columns] {
                    manager.set_browser_mode(mode);
                    manager.set_icons_thumbnail_size(if enabled { 203 } else { 95 });
                    manager.set_click_activation(
                        mode,
                        ClickActivation {
                            files: if enabled {
                                ClickCount::Two
                            } else {
                                ClickCount::One
                            },
                            folders: if enabled {
                                ClickCount::One
                            } else {
                                ClickCount::Two
                            },
                        },
                    );
                    for view in [&first, &second] {
                        assert_eq!(view.view_mode(), mode);
                        view.assert_saved_preferences(&manager);
                        view.assert_peek_scheduling(enabled);
                    }
                    assert_eq!(chooser.view_mode(), mode);
                    chooser.assert_peek_scheduling(false);
                }
                let third = browser_for_window();
                third.assert_saved_preferences(&manager);
                assert_eq!(third.view_mode(), manager.browser_mode());
            }
            first.browser().toggle_hidden();
            assert_eq!(
                second.browser().preferences(),
                first.browser().preferences()
            );
            assert_eq!(
                chooser.browser().preferences(),
                first.browser().preferences()
            );
        },
    );
}

#[test]
fn sidebar_order_and_update_notices_follow_preferences_without_settings() {
    use std::rc::Rc;
    gtk_test(
        "ui::window::tests::preferences::sidebar_order_and_update_notices_follow_preferences_without_settings",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            let first = super::super::build_sidebar(browser_for_window(), manager.clone(), true);
            let second = super::super::build_sidebar(browser_for_window(), manager.clone(), true);
            manager.set_sidebar_order(vec![
                "recent".into(),
                "downloads".into(),
                "videos".into(),
                "documents".into(),
                "pictures".into(),
                "desktop".into(),
                "home".into(),
                "trash".into(),
                "network".into(),
            ]);
            assert_eq!(
                *first.state.place_order.borrow(),
                [
                    "recent",
                    "downloads",
                    "videos",
                    "documents",
                    "pictures",
                    "desktop",
                    "home",
                    "trash",
                    "network"
                ]
            );
            assert_eq!(
                *first.state.place_order.borrow(),
                *second.state.place_order.borrow()
            );
            let cleared = Rc::new(Cell::new(0));
            let observe = cleared.clone();
            let notice: crate::ui::settings::UpdateNoticeHandler = Rc::new(move |value| {
                assert!(value.is_none());
                observe.set(observe.get() + 1);
            });
            let anchors = [
                gtk::Box::new(gtk::Orientation::Vertical, 0),
                gtk::Box::new(gtk::Orientation::Vertical, 0),
            ];
            for anchor in &anchors {
                super::super::bind_update_notice_preferences(anchor, &manager, &notice);
            }
            assert_eq!(cleared.get(), 0);
            manager.set_checks_for_updates(true);
            assert_eq!(cleared.get(), 2);
            manager.set_release_channel(crate::services::Channel::Stable);
            assert_eq!(cleared.get(), 4);
            manager.set_checks_for_updates(false);
            assert_eq!(cleared.get(), 6);
        },
    );
}

#[test]
fn saved_browser_preferences_apply_without_settings_and_survive_view_changes() {
    gtk_test(
        "ui::window::tests::preferences::saved_browser_preferences_apply_without_settings_and_survive_view_changes",
        || {
            let directory = glib::user_config_dir().join("strata");
            std::fs::create_dir_all(&directory).expect("isolated settings directory");
            let path = directory.join("settings.toml");
            let saved = r#"
mode = "theme"
theme = "azure-glow"
folder_peeking = false
single_click_previews = false
browser_mode = "list"
browser_density = "airy"
group_by_type = true
list_file_clicks = 1
list_folder_clicks = 2
grid_file_clicks = 1
grid_folder_clicks = 1
explorer_file_clicks = 1
explorer_folder_clicks = 1
show_hidden = true
folders_first = false
sort_key = "size"
sort_direction = "descending"
auto_refresh_interval = 600
icons_thumbnail_size = 203
"#;
            std::fs::write(&path, saved).expect("persist non-default startup preferences");
            let manager = PreferenceManager::shared();
            assert!(!manager.folder_peeking());
            assert!(!manager.single_click_previews());
            assert_eq!(manager.browser_mode(), BrowserMode::List);
            for _ in 0..2 {
                let browser = browser_for_window();
                assert_eq!(browser.view_mode(), BrowserMode::List);
                browser.assert_saved_preferences(&manager);
                browser.assert_peek_scheduling(false);
                for mode in [BrowserMode::Icons, BrowserMode::Columns, BrowserMode::List] {
                    browser.set_view_mode(mode);
                    browser.assert_saved_preferences(&manager);
                    browser.assert_peek_scheduling(false);
                }
            }
            assert_eq!(
                std::fs::read_to_string(path).expect("unchanged preferences"),
                saved
            );
        },
    );
}

#[test]
fn startup_directory_loads_without_settings_and_clears_stale_paths() {
    gtk_test(
        "ui::window::tests::preferences::startup_directory_loads_without_settings_and_clears_stale_paths",
        || {
            let directory = tempfile::tempdir().expect("startup fixture");
            let chosen = directory.path().join("chosen");
            std::fs::create_dir(&chosen).expect("chosen folder");
            let config = glib::user_config_dir().join("strata/settings.toml");
            std::fs::create_dir_all(config.parent().expect("config parent"))
                .expect("config directory");
            let saved = toml::Table::from_iter([(
                "default_directory".into(),
                toml::Value::String(chosen.to_str().expect("UTF-8 fixture path").into()),
            )]);
            std::fs::write(
                &config,
                toml::to_string(&saved).expect("serialized preferences"),
            )
            .expect("saved preferences");
            let manager = PreferenceManager::shared();
            assert_eq!(startup_location(&manager), Location::local(&chosen));
            std::fs::remove_dir(&chosen).expect("remove chosen folder");
            assert_eq!(
                startup_location(&manager),
                Location::local(home_directory())
            );
            assert_eq!(manager.default_directory(), None);
            let persisted: toml::Table = std::fs::read_to_string(&config)
                .expect("persisted preferences")
                .parse()
                .expect("valid preferences");
            assert!(!persisted.contains_key("default_directory"));
            std::fs::create_dir(&chosen).expect("recreate chosen folder");
            assert_eq!(
                startup_location(&manager),
                Location::local(home_directory())
            );
            manager.set_default_directory(Some(chosen.clone()));
            assert_eq!(startup_location(&manager), Location::local(&chosen));
            let file = directory.path().join("not-a-directory");
            std::fs::write(&file, "fixture").expect("regular file fixture");
            manager.set_default_directory(Some(file));
            assert_eq!(
                startup_location(&manager),
                Location::local(home_directory())
            );
            assert_eq!(manager.default_directory(), None);
        },
    );
}

#[test]
fn default_browser_preferences_allow_peeking_without_settings() {
    gtk_test(
        "ui::window::tests::preferences::default_browser_preferences_allow_peeking_without_settings",
        || {
            let manager = PreferenceManager::shared();
            let browser = browser_for_window();
            assert!(manager.folder_peeking());
            browser.assert_saved_preferences(&manager);
            browser.assert_peek_scheduling(true);
            browser.set_peek_enabled(false);
            browser.assert_peek_scheduling(false);
            browser.set_peek_enabled(true);
            browser.assert_peek_scheduling(true);
        },
    );
}

#[test]
fn startup_applies_disabled_single_click_previews_before_the_first_click() {
    gtk_test(
        "ui::window::tests::preferences::startup_applies_disabled_single_click_previews_before_the_first_click",
        || {
            let manager = PreferenceManager::shared();
            manager.set_single_click_previews(false);
            let browser = browser_for_window();
            assert!(!browser.single_click_previews_enabled());
        },
    );
}

#[test]
fn startup_applies_disabled_columns_mirror_before_the_first_selection() {
    gtk_test(
        "ui::window::tests::preferences::startup_applies_disabled_columns_mirror_before_the_first_selection",
        || {
            let manager = PreferenceManager::shared();
            manager.set_columns_mirror_selection(false);
            let browser = browser_for_window();
            assert!(!browser.columns_mirror_selection_enabled());
        },
    );
}

#[test]
fn default_chrome_stays_operable_without_a_saved_omastrata_mode() {
    gtk_test(
        "ui::window::tests::preferences::default_chrome_stays_operable_without_a_saved_omastrata_mode",
        || {
            let manager = PreferenceManager::shared();
            assert!(!manager.omastrata_mode());
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
            assert!(!manager.omastrata_mode());
            assert!(!open.content.footer().tag_visible());
        },
    );
}

#[test]
fn saved_omastrata_mode_applies_before_settings_and_to_lazy_views() {
    gtk_test(
        "ui::window::tests::preferences::saved_omastrata_mode_applies_before_settings_and_to_lazy_views",
        || {
            write_settings("omastrata_mode = true\n");
            let manager = PreferenceManager::shared();
            assert!(manager.omastrata_mode());
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
            manager.set_omastrata_mode(false);
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
            assert!(manager.omastrata_mode());
            assert_controls(&first, false);
            assert_controls(&second, false);
            assert!(first.content.footer().tag_visible());
            assert!(second.content.footer().tag_visible());
            first.content.settings_button().emit_clicked();
            second.content.settings_button().emit_clicked();
            settle();
            let first_switch = switch_named(first.content.overlay(), "Omastrata mode");
            let second_switch = switch_named(second.content.overlay(), "Omastrata mode");
            assert!(first_switch.is_active());
            assert!(second_switch.is_active());
            second_switch.set_active(false);
            settle();
            assert!(!manager.omastrata_mode());
            assert!(!first_switch.is_active());
            assert_controls(&first, true);
            assert_controls(&second, true);
            assert!(!second.content.footer().tag_visible());
            let saved = std::fs::read_to_string(settings_file()).expect("saved settings");
            assert!(saved.contains("omastrata_mode = false"));
        },
    );
}

#[test]
fn q_leaves_omastrata_and_shift_q_closes_only_the_current_window() {
    gtk_test(
        "ui::window::tests::preferences::q_leaves_omastrata_and_shift_q_closes_only_the_current_window",
        || {
            let manager = PreferenceManager::shared();
            let first = OpenWindow::open();
            let second = OpenWindow::open();
            manager.set_omastrata_mode(true);
            settle();
            press(
                &first.window,
                gtk::gdk::Key::q,
                gtk::gdk::ModifierType::empty(),
            );
            settle();
            assert!(!manager.omastrata_mode());
            assert!(window_listed(&first.window));
            assert!(window_listed(&second.window));
            assert!(!first.content.footer().tag_visible());
            assert!(!second.content.footer().tag_visible());
            manager.set_omastrata_mode(true);
            settle();
            press(
                &first.window,
                gtk::gdk::Key::Q,
                gtk::gdk::ModifierType::SHIFT_MASK,
            );
            settle();
            assert!(!window_listed(&first.window));
            assert!(window_listed(&second.window));
            assert!(manager.omastrata_mode());
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
                "type to search filters before Omastrata is enabled"
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
            manager.set_omastrata_mode(true);
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
                "typing does not filter while Omastrata is on"
            );
            manager.set_omastrata_mode(false);
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
                "type to search filters again after leaving Omastrata"
            );
            drop(directory);
        },
    );
}

#[test]
fn entering_omastrata_hides_an_open_filter_and_leaving_restores_it() {
    gtk_test(
        "ui::window::tests::preferences::entering_omastrata_hides_an_open_filter_and_leaving_restores_it",
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
            manager.set_omastrata_mode(true);
            settle();
            assert!(!revealer.reveals_child());
            assert_controls(&open, false);
            manager.set_omastrata_mode(false);
            settle();
            assert!(revealer.reveals_child());
            assert_controls(&open, true);
            drop(directory);
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
    for control in class_in_shown_pane(&open.content.browser.widget(), "omastrata-sort-direction") {
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
                && child.has_css_class("omastrata-filter-revealer")
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
