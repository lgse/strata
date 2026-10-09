// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
};

use sourceview5::prelude::BufferExt as _;

use super::super::*;
use crate::{
    model::{
        EntryKind, FileEntry, Location, MetadataValue, SortDirection, SortKey, ViewPreferences,
    },
    test_support::gtk_test,
    ui::{
        browser_modes::{BrowserDensity, BrowserMode, ClickCount},
        preferences::fixtures::{
            non_default_preferences, seed_omarchy_colors_for_test, seed_omarchy_for_test,
            seed_saved_preferences_for_test,
        },
        theme::ThemeManager,
    },
};

#[test]
fn saved_language_applies_before_settings_and_changes_only_after_restart() {
    gtk_test(
        "ui::preferences::tests::preferences::saved_language_applies_before_settings_and_changes_only_after_restart",
        || {
            let mut preferences = non_default_preferences();
            preferences.language = crate::i18n::Language::French;
            fs::create_dir_all(settings_path().parent().expect("settings directory"))
                .expect("create settings directory");
            fs::write(
                settings_path(),
                toml::to_string(&preferences).expect("serialize settings"),
            )
            .expect("save fixture");
            let manager = PreferenceManager::load();
            assert_eq!(crate::i18n::tr("Language"), "Langue");
            assert!(!manager.language_restart_required());
            manager.set_language(crate::i18n::Language::Japanese);
            assert!(manager.language_restart_required());
            assert_eq!(crate::i18n::tr("Language"), "Langue");
            assert_eq!(
                read_preferences().expect("read saved language").language,
                crate::i18n::Language::Japanese
            );
            manager.set_language(crate::i18n::Language::French);
            assert!(!manager.language_restart_required());
            manager.set_language(crate::i18n::Language::Japanese);
            let restarted = PreferenceManager::load();
            assert!(!restarted.language_restart_required());
            assert_eq!(&*rust_i18n::locale(), "ja");
        },
    );
}

#[test]
fn recent_sort_is_not_stored_as_an_ordinary_folder_default() {
    gtk_test(
        "ui::preferences::tests::preferences::recent_sort_is_not_stored_as_an_ordinary_folder_default",
        || {
            seed_saved_preferences_for_test();
            let manager = PreferenceManager::load();
            let saved = manager.preferences.borrow().clone();

            manager.set_sort_preferences(ViewPreferences {
                sort_key: SortKey::Recency,
                sort_direction: SortDirection::Descending,
                ..ViewPreferences::default()
            });

            assert_eq!(*manager.preferences.borrow(), saved);
            let persisted: Preferences =
                toml::from_str(&fs::read_to_string(settings_path()).expect("saved preferences"))
                    .expect("persisted preferences");
            assert_eq!(persisted, saved);
        },
    );
}

#[test]
fn older_preferences_keep_backward_compatible_behavior_defaults() {
    let mut saved = toml::Table::try_from(non_default_preferences()).expect("saved preferences");
    saved.remove("filter_include_subfolders");
    saved.remove("open_folder_after_drop");
    saved.remove("date_format");
    saved.remove("send_to_recent_destinations");
    saved.remove("tenxer_mode");
    saved.remove("omarchy_variant");
    saved.remove("folder_peeking");
    saved.remove("language");
    let restored: Preferences = saved.try_into().expect("backward-compatible preferences");
    assert_eq!(
        restored,
        Preferences {
            language: crate::i18n::Language::Auto,
            filter_include_subfolders: true,
            open_folder_after_drop: false,
            date_format: "relative".into(),
            send_to_recent_destinations: HashMap::new(),
            tenxer_mode: false,
            omarchy_variant: OmarchyVariant::Original,
            folder_peeking: false,
            ..non_default_preferences()
        }
    );
}

#[test]
fn empty_send_to_history_is_omitted_from_saved_preferences() {
    let serialized = toml::Table::try_from(Preferences::default()).expect("default preferences");
    assert!(!serialized.contains_key("send_to_recent_destinations"));
}

#[test]
fn malformed_send_to_history_does_not_discard_other_preferences() {
    let mut saved = toml::Table::try_from(non_default_preferences()).expect("saved preferences");
    saved.insert("send_to_recent_destinations".into(), "invalid".into());
    assert!(saved.clone().try_into::<Preferences>().is_err());

    assert_eq!(
        salvage_preferences(saved),
        Preferences {
            send_to_recent_destinations: HashMap::new(),
            ..non_default_preferences()
        }
    );
}

#[test]
fn send_to_history_is_device_scoped_deduplicated_capped_and_persistent() {
    gtk_test(
        "ui::preferences::tests::preferences::send_to_history_is_device_scoped_deduplicated_capped_and_persistent",
        || {
            let manager = PreferenceManager::shared();
            for path in ["A", "B", "C", "B", "D"] {
                manager.remember_send_to_destination("volume:kingston", Path::new(path), None);
            }
            manager.remember_send_to_destination("volume:sandisk", Path::new("Backup"), None);
            manager.remember_send_to_destination("volume:drive-root", Path::new(""), None);

            let expected = vec![PathBuf::from("D"), PathBuf::from("B"), PathBuf::from("C")];
            assert_eq!(
                manager.send_to_recent_destinations("volume:kingston"),
                expected
            );
            assert_eq!(
                manager.send_to_recent_destinations("volume:sandisk"),
                [PathBuf::from("Backup")]
            );
            assert!(
                manager
                    .send_to_recent_destinations("volume:drive-root")
                    .is_empty()
            );

            let reloaded = PreferenceManager::load();
            assert_eq!(
                reloaded.send_to_recent_destinations("volume:kingston"),
                expected
            );
            assert_eq!(
                reloaded.send_to_recent_destinations("volume:sandisk"),
                [PathBuf::from("Backup")]
            );
            let saved: Preferences =
                toml::from_str(&fs::read_to_string(settings_path()).expect("saved preferences"))
                    .expect("persisted preferences reload");
            assert!(
                !saved
                    .send_to_recent_destinations
                    .contains_key("volume:drive-root")
            );
        },
    );
}

#[test]
fn a_malformed_preference_does_not_discard_the_others() {
    let mut saved = toml::Table::try_from(non_default_preferences()).expect("saved preferences");
    saved.insert("show_hidden".into(), "yes".into());
    saved.insert("omarchy_variant".into(), "unknown".into());
    assert!(saved.clone().try_into::<Preferences>().is_err());

    assert_eq!(
        salvage_preferences(saved),
        Preferences {
            show_hidden: false,
            omarchy_variant: OmarchyVariant::Original,
            ..non_default_preferences()
        }
    );
}

#[test]
fn a_missing_required_preference_does_not_discard_the_others() {
    let mut saved = toml::Table::try_from(non_default_preferences()).expect("saved preferences");
    saved.remove("theme");
    assert!(saved.clone().try_into::<Preferences>().is_err());

    assert_eq!(
        salvage_preferences(saved),
        Preferences {
            theme: Preferences::default().theme,
            ..non_default_preferences()
        }
    );
}

fn assert_recovered_preferences_survive_save(
    corrupt: impl FnOnce(&mut toml::Table),
    mut expected: Preferences,
) {
    seed_saved_preferences_for_test();
    let mut saved = toml::Table::try_from(non_default_preferences()).expect("saved preferences");
    corrupt(&mut saved);
    let malformed = toml::to_string(&saved).expect("syntactically valid TOML");
    fs::write(settings_path(), &malformed).expect("persist malformed preferences");

    let manager = PreferenceManager::shared();
    assert_eq!(*manager.preferences.borrow(), expected);
    assert_eq!(
        fs::read_to_string(settings_path()).expect("unchanged settings file"),
        malformed
    );
    expected.folder_peeking = false;
    manager.set_folder_peeking(false);

    let persisted: Preferences =
        toml::from_str(&fs::read_to_string(settings_path()).expect("saved file"))
            .expect("save repairs invalid preferences");
    assert_eq!(persisted, expected);
    assert_eq!(read_preferences().expect("saved preferences"), expected);
}

#[test]
fn unreadable_preferences_are_preserved_while_live_changes_still_apply() {
    gtk_test(
        "ui::preferences::tests::preferences::unreadable_preferences_are_preserved_while_live_changes_still_apply",
        || {
            seed_saved_preferences_for_test();
            let valid = fs::read(settings_path()).expect("saved fixture");
            for suffix in [b"\nthis is not valid toml [".as_slice(), b"\xff"] {
                let mut broken = valid.clone();
                broken.extend_from_slice(suffix);
                fs::write(settings_path(), &broken).expect("broken settings");
                let manager = PreferenceManager::load();
                let window = save_notice_window(&manager);
                let anchors = [
                    gtk::Box::new(gtk::Orientation::Vertical, 0),
                    gtk::Box::new(gtk::Orientation::Vertical, 0),
                ];
                let observations = anchors.each_ref().map(|anchor| {
                    let values = Rc::new(RefCell::new(Vec::new()));
                    let observed = values.clone();
                    manager.bind_preference(
                        anchor,
                        PreferenceManager::folder_peeking,
                        move |_, value| {
                            observed.borrow_mut().push(value);
                        },
                    );
                    values
                });
                assert!(save_notices(&window).is_empty(), "nothing changed yet");
                manager.set_folder_peeking(true);
                manager.set_folder_peeking(true);
                for values in observations {
                    assert_eq!(*values.borrow(), [false, true]);
                }
                let notices = save_notices(&window);
                assert_eq!(notices.len(), 1, "{notices:?}");
                assert_notice(
                    &notices[0],
                    "Settings file can't be read",
                    &format!(
                        "Strata couldn't read “{}” when it started:",
                        settings_path().display()
                    ),
                );
                assert_eq!(
                    fs::read(settings_path()).expect("preserved settings"),
                    broken
                );
                fs::write(settings_path(), &valid).expect("repair settings");
                manager.set_folder_peeking(false);
                assert_eq!(
                    fs::read(settings_path()).expect("repair left untouched"),
                    valid
                );
                assert_eq!(save_notices(&window).len(), 1, "one notice per session");
                window.destroy();
                drop(manager);
            }
            let manager = PreferenceManager::load();
            let window = save_notice_window(&manager);
            assert_eq!(*manager.preferences.borrow(), non_default_preferences());
            manager.set_folder_peeking(false);
            assert!(save_notices(&window).is_empty());
            assert!(
                !read_preferences()
                    .expect("saving resumes after reload")
                    .folder_peeking
            );
        },
    );
}

#[test]
fn missing_settings_allow_first_run_saves() {
    gtk_test(
        "ui::preferences::tests::preferences::missing_settings_allow_first_run_saves",
        || {
            assert!(!settings_path().exists());
            let manager = PreferenceManager::load();
            assert!(!manager.folder_peeking());
            manager.set_folder_peeking(true);
            assert!(read_preferences().expect("first run save").folder_peeking);
            assert!(PreferenceManager::load().folder_peeking());
            manager.set_folder_peeking(false);
            assert!(!PreferenceManager::load().folder_peeking());
        },
    );
}

#[test]
fn symlinked_settings_save_through_to_the_target() {
    gtk_test(
        "ui::preferences::tests::preferences::symlinked_settings_save_through_to_the_target",
        || {
            seed_saved_preferences_for_test();
            let config_home = std::env::var_os("XDG_CONFIG_HOME").expect("isolated config home");
            let target = Path::new(&config_home)
                .parent()
                .expect("sandbox root")
                .join("dotfiles/settings.toml");
            fs::create_dir_all(target.parent().expect("dotfiles directory"))
                .expect("dotfiles directory");
            fs::rename(settings_path(), &target).expect("move settings into dotfiles");
            std::os::unix::fs::symlink(&target, settings_path()).expect("dotfiles link");

            let manager = PreferenceManager::load();
            assert!(manager.folder_peeking());
            manager.set_folder_peeking(false);

            assert!(
                !manager.persistence_dirty.get(),
                "the save through the link must not stay pending"
            );
            assert!(
                fs::symlink_metadata(settings_path())
                    .expect("settings link")
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(
                fs::read_link(settings_path()).expect("settings link"),
                target
            );
            let saved: Preferences =
                toml::from_str(&fs::read_to_string(&target).expect("dotfiles settings"))
                    .expect("dotfiles settings parse");
            assert!(!saved.folder_peeking);
            drop(manager);
            assert!(!PreferenceManager::load().folder_peeking());
        },
    );
}

#[test]
fn failed_saves_show_one_notice_per_failure_streak() {
    gtk_test(
        "ui::preferences::tests::preferences::failed_saves_show_one_notice_per_failure_streak",
        || {
            let manager = PreferenceManager::load();
            let window = save_notice_window(&manager);
            fs::create_dir_all(settings_path()).expect("block the settings file with a directory");

            let other = gtk::Window::new();
            other.present();
            wait_for(|| !window.is_active(), "another window to take focus");
            manager.set_folder_peeking(true);
            assert!(
                save_notices(&window).is_empty(),
                "a change while no browser window is active only logs"
            );
            other.destroy();
            window.present();
            wait_for(|| window.is_active(), "the notice window to become active");
            manager.set_folder_peeking(true);
            window.set_visible(false);
            assert!(
                save_notices(&window).is_empty(),
                "a window hidden before the notice opens shows nothing"
            );
            window.present();
            wait_for(|| window.is_active(), "the notice window to return");

            manager.set_folder_peeking(true);
            manager.set_type_to_search(!manager.type_to_search());
            let notices = save_notices(&window);
            assert_eq!(
                notices.len(),
                1,
                "one notice per failure streak: {notices:?}"
            );
            assert_notice(
                &notices[0],
                "Settings can't be saved",
                &format!(
                    "Strata couldn't write “{path}”: The destination “{path}” is not a regular file.",
                    path = settings_path().display()
                ),
            );

            fs::remove_dir(settings_path()).expect("repair the settings file");
            manager.set_folder_peeking(true);
            assert!(read_preferences().expect("retried save").folder_peeking);
            assert_eq!(save_notices(&window).len(), 1);

            fs::remove_file(settings_path()).expect("saved settings file");
            fs::create_dir(settings_path()).expect("block the settings file again");
            manager.set_folder_peeking(false);
            assert_eq!(
                save_notices(&window).len(),
                2,
                "a failure after a successful save starts a new streak"
            );
            window.destroy();
        },
    );
}

#[derive(Debug)]
struct SaveNoticeText {
    title: String,
    summary: String,
    detail: String,
}

fn save_notice_window(manager: &PreferenceManager) -> gtk::Window {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&gtk::Button::with_label("Origin")));
    let window = gtk::Window::builder().child(&overlay).build();
    window.present();
    wait_for(|| window.is_active(), "the notice window to become active");
    manager.register_save_notice_window(&window);
    window
}

fn wait_for(condition: impl Fn() -> bool, what: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "waiting for {what}");
        glib::MainContext::default().iteration(false);
    }
}

fn save_notices(window: &gtk::Window) -> Vec<SaveNoticeText> {
    while glib::MainContext::default().iteration(false) {}
    fn descendants(widget: &gtk::Widget, found: &mut Vec<gtk::Widget>) {
        found.push(widget.clone());
        let mut child = widget.first_child();
        while let Some(current) = child {
            descendants(&current, found);
            child = current.next_sibling();
        }
    }
    let mut widgets = Vec::new();
    descendants(window.upcast_ref(), &mut widgets);
    widgets
        .iter()
        .filter(|widget| widget.has_css_class("app-modal-layer"))
        .map(|layer| {
            let mut inner = Vec::new();
            descendants(layer, &mut inner);
            let text = |class: &str| {
                inner
                    .iter()
                    .find(|widget| widget.has_css_class(class))
                    .and_then(|widget| {
                        widget
                            .downcast_ref::<gtk::Label>()
                            .map(|label| label.text())
                    })
                    .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
                    .unwrap_or_default()
            };
            SaveNoticeText {
                title: text("action-dialog-title"),
                summary: text("action-dialog-subtitle"),
                detail: text("action-dialog-description"),
            }
        })
        .collect()
}

fn assert_notice(notice: &SaveNoticeText, title: &str, detail_start: &str) {
    let compact = |text: &str| text.split_whitespace().collect::<String>();
    assert_eq!(notice.title, title);
    assert_eq!(notice.summary, "Changes last only until Strata closes");
    assert!(
        compact(&notice.detail).starts_with(&compact(detail_start)),
        "{notice:?}"
    );
}

#[test]
fn malformed_preferences_survive_startup_and_an_unrelated_save() {
    gtk_test(
        "ui::preferences::tests::preferences::malformed_preferences_survive_startup_and_an_unrelated_save",
        || {
            assert_recovered_preferences_survive_save(
                |saved| {
                    saved.insert("show_hidden".into(), "yes".into());
                },
                Preferences {
                    show_hidden: false,
                    ..non_default_preferences()
                },
            );
        },
    );
}

#[test]
fn missing_required_preferences_survive_startup_and_an_unrelated_save() {
    gtk_test(
        "ui::preferences::tests::preferences::missing_required_preferences_survive_startup_and_an_unrelated_save",
        || {
            assert_recovered_preferences_survive_save(
                |saved| {
                    saved.remove("theme");
                },
                Preferences {
                    theme: Preferences::default().theme,
                    ..non_default_preferences()
                },
            );
        },
    );
}

#[test]
fn multiple_invalid_preferences_do_not_block_later_valid_entries() {
    gtk_test(
        "ui::preferences::tests::preferences::multiple_invalid_preferences_do_not_block_later_valid_entries",
        || {
            assert_recovered_preferences_survive_save(
                |saved| {
                    saved.insert("auto_refresh_interval".into(), (-1).into());
                    saved.insert("hardware_accelerated_video_previews".into(), "no".into());
                    saved.insert("show_hidden".into(), "yes".into());
                    saved.insert("future_preference".into(), true.into());
                },
                Preferences {
                    auto_refresh_interval: 0,
                    hardware_accelerated_video_previews: None,
                    show_hidden: false,
                    ..non_default_preferences()
                },
            );
        },
    );
}

#[test]
fn unlisted_auto_refresh_intervals_round_up_on_load_and_set() {
    gtk_test(
        "ui::preferences::tests::preferences::unlisted_auto_refresh_intervals_round_up_on_load_and_set",
        || {
            for (stored, expected) in [
                (0, 0),
                (1, 60),
                (59, 60),
                (60, 60),
                (61, 300),
                (120, 300),
                (601, 600),
                (3600, 600),
            ] {
                assert_recovered_preferences_survive_save(
                    |saved| {
                        saved.insert("auto_refresh_interval".into(), i64::from(stored).into());
                    },
                    Preferences {
                        auto_refresh_interval: expected,
                        ..non_default_preferences()
                    },
                );
            }

            seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            for (requested, expected) in [(45, 60), (u32::MAX, 600), (300, 300), (0, 0)] {
                manager.set_auto_refresh_interval(requested);
                assert_eq!(manager.auto_refresh_interval(), expected);
                assert_eq!(
                    read_preferences()
                        .expect("saved preferences")
                        .auto_refresh_interval,
                    expected
                );
            }
        },
    );
}

#[test]
fn fresh_preferences_select_tokyo_night_before_settings_opens() {
    gtk_test(
        "ui::preferences::tests::preferences::fresh_preferences_select_tokyo_night_before_settings_opens",
        || {
            assert!(!settings_path().exists());
            let themes = ThemeManager::shared();
            let manager = PreferenceManager::shared();
            assert_eq!(themes.selected_id(), "tokyo-night");
            assert!(manager.filter_include_subfolders());
            assert!(!themes.follows_omarchy());
            assert_eq!(
                themes.current_tokens().expect("selected theme").name,
                "Tokyo Night"
            );
            assert!(!settings_path().exists());
        },
    );
}

#[test]
fn fresh_preferences_still_follow_available_omarchy_theme() {
    gtk_test(
        "ui::preferences::tests::preferences::fresh_preferences_still_follow_available_omarchy_theme",
        || {
            seed_omarchy_for_test();
            let themes = ThemeManager::shared();
            assert!(themes.follows_omarchy());
            assert_eq!(themes.selected_id(), "tokyo-night");
            themes.set_follow_omarchy(false);
            assert_eq!(
                themes.current_tokens().expect("selected theme").name,
                "Tokyo Night"
            );
        },
    );
}

#[test]
fn every_saved_preference_loads_before_any_settings_page_exists() {
    gtk_test(
        "ui::preferences::tests::preferences::every_saved_preference_loads_before_any_settings_page_exists",
        || {
            seed_saved_preferences_for_test();
            let themes = ThemeManager::shared();
            let manager = PreferenceManager::shared();
            assert_eq!(*manager.preferences.borrow(), non_default_preferences());
            assert!(!themes.follows_omarchy());
            assert_eq!(themes.selected_id(), "nord");
            assert!(manager.folder_peeking());
            assert!(!manager.single_click_previews());
            assert!(!manager.columns_mirror_selection());
            assert!(!manager.hardware_accelerated_video_previews());
            assert_eq!(manager.video_preview_backend(), MediaPreviewBackend::Vulkan);
            assert_eq!(
                manager.media_preview_backend(),
                MediaPreviewBackend::Software
            );
            assert!(manager.search_open_files_directly());
            assert_eq!(
                manager.search_exclusions(),
                vec![".venv", "/fixture/custom_excluded"]
            );
            assert!(!manager.type_to_search());
            assert!(manager.arrow_navigation_scoped());
            assert!(manager.tenxer_mode());
            assert!(!manager.filter_include_subfolders());
            assert!(!manager.show_keybinding_hints());
            assert!(!manager.window_show_close());
            assert!(manager.window_show_minimize());
            assert!(manager.window_show_maximize());
            assert!(manager.reduce_motion());
            assert!(!manager.element_glow());
            let windows = [gtk::Window::new(), gtk::Window::new()];
            for enabled in [false, true, false] {
                manager.set_element_glow(enabled);
                for window in &windows {
                    let surface = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    window.set_child(Some(&surface));
                    #[expect(
                        deprecated,
                        reason = "GTK has no replacement API for resolving named CSS colors"
                    )]
                    let (glow, accent) = {
                        let style = surface.style_context();
                        (
                            style.lookup_color("strata_glow").expect("glow color"),
                            style.lookup_color("strata_accent").expect("accent color"),
                        )
                    };
                    if enabled {
                        assert_eq!(glow, accent);
                    } else {
                        assert_eq!(glow.alpha(), 0.0);
                        assert!(accent.alpha() > 0.0);
                    }
                }
            }
            let display = gtk::gdk::Display::default().expect("test display");
            let user_css = gtk::CssProvider::new();
            user_css.load_from_string(
                "@define-color theme_bg #ff00ff; @define-color theme_accent #00ff00;",
            );
            gtk::style_context_add_provider_for_display(
                &display,
                &user_css,
                gtk::STYLE_PROVIDER_PRIORITY_USER,
            );
            for theme in ["nord", "azure-glow", "nord"] {
                themes.select_theme(theme);
                let tokens = themes.current_tokens().expect("active theme");
                for window in &windows {
                    #[expect(
                        deprecated,
                        reason = "GTK has no replacement API for resolving named CSS colors"
                    )]
                    let style = window.style_context();
                    #[expect(
                        deprecated,
                        reason = "GTK has no replacement API for resolving named CSS colors"
                    )]
                    for (name, expected) in [
                        ("strata_bg", tokens.background.as_str()),
                        ("strata_accent", tokens.accent.as_str()),
                        ("theme_bg", "#ff00ff"),
                        ("theme_accent", "#00ff00"),
                    ] {
                        assert_eq!(
                            style.lookup_color(name).expect("named color"),
                            gtk::gdk::RGBA::parse(expected).expect("token color"),
                            "{theme}: {name}",
                        );
                    }
                }
            }
            gtk::style_context_remove_provider_for_display(&display, &user_css);
            for window in windows {
                window.close();
            }
            assert!(!crate::ui::motion::animations_enabled());
            assert_eq!(manager.browser_mode(), BrowserMode::List);
            assert_eq!(manager.browser_density(), BrowserDensity::Airy);
            assert!(manager.group_by_type());
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                let activation = manager.click_activation(mode);
                assert_eq!(activation.files, ClickCount::One);
                assert_eq!(
                    activation.folders,
                    if mode == BrowserMode::Columns {
                        ClickCount::Two
                    } else {
                        ClickCount::One
                    }
                );
            }
            assert_eq!(
                manager.sidebar_order(),
                non_default_preferences().sidebar_order
            );
            assert!(!manager.sidebar_show_home());
            assert!(!manager.sidebar_show_trash());
            assert!(!manager.sidebar_show_network());
            assert!(!manager.sidebar_show_recent());
            assert!(!manager.sidebar_show_desktop());
            assert!(!manager.sidebar_show_documents());
            assert!(!manager.sidebar_show_downloads());
            assert!(!manager.sidebar_show_music());
            assert!(!manager.sidebar_show_pictures());
            assert!(!manager.sidebar_show_videos());
            assert!(!manager.sidebar_expanded());
            assert_eq!(
                manager.sidebar_places_visibility(),
                [
                    false, false, false, false, false, false, false, false, false, false
                ]
            );
            assert_eq!(manager.text_size(), TextSize::new(24));
            assert_eq!(
                manager.sort_preferences(),
                ViewPreferences {
                    show_hidden: true,
                    folders_first: false,
                    sort_key: SortKey::Size,
                    sort_direction: SortDirection::Descending,
                }
            );
            assert!(!manager.checks_for_updates());
            assert_eq!(manager.release_channel(), Channel::Nightly);
            assert!(manager.preview_muted());
            assert_eq!(manager.preview_volume(), 0.35);
            assert!(manager.preview_text_wrap());
            assert!(manager.preview_autoplay());
            assert_eq!(manager.auto_refresh_interval(), 600);
            assert_eq!(manager.thumbnail_workers(), 6);
            assert_eq!(
                manager.cross_volume_drop_strategy(),
                CrossVolumeDropStrategy::Move
            );
            assert_eq!(manager.date_format(), crate::util::DateFormat::Iso8601);
            assert_eq!(
                manager.device_label("volume:fixture-kingston").as_deref(),
                Some("Research drive")
            );
            assert_eq!(
                manager.send_to_recent_destinations("volume:fixture-kingston"),
                [PathBuf::from("Academia/2026"), PathBuf::from("Teaching")]
            );
            assert_eq!(
                manager.send_to_recent_destinations("volume:fixture-sandisk"),
                [PathBuf::from("Backup")]
            );
            assert_eq!(
                manager.default_directory(),
                Some(std::path::PathBuf::from("/fixture/default"))
            );
            assert_eq!(
                manager.folder_color(Path::new("/fixture/folder")),
                FolderColorValue::parse("red")
            );
            assert_eq!(
                manager.custom_icon(Path::new("/fixture/folder")).as_deref(),
                Some(crate::assets::icons::HOME)
            );
        },
    );
}

#[test]
fn saved_omarchy_mode_loads_and_changes_through_the_same_binding() {
    gtk_test(
        "ui::preferences::tests::preferences::saved_omarchy_mode_loads_and_changes_through_the_same_binding",
        || {
            seed_saved_preferences_for_test();
            seed_omarchy_for_test();
            let mut preferences = non_default_preferences();
            preferences.mode = "omarchy".into();
            fs::write(
                settings_path(),
                toml::to_string(&preferences).expect("Omarchy preference fixture"),
            )
            .expect("persist Omarchy mode");
            let manager = ThemeManager::shared();
            let anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let values = Rc::new(RefCell::new(Vec::new()));
            let observed = values.clone();
            manager.bind_theme_preference(
                &anchor,
                ThemeManager::follows_omarchy,
                move |_, value| observed.borrow_mut().push(value),
            );
            assert_eq!(*values.borrow(), [true]);
            let preferences = PreferenceManager::shared();
            let windows = [gtk::Window::new(), gtk::Window::new()];
            let buffer = gtk::TextBuffer::new(None);
            buffer.create_tag(Some("document-accent"), &[]);
            crate::ui::theme::register_document_buffer(&buffer);
            let source = sourceview5::Buffer::new(None);
            crate::ui::theme::register_source_buffer(&source);
            let startup = manager.appearance_tokens();
            for variant in [
                OmarchyVariant::Darker,
                OmarchyVariant::Original,
                OmarchyVariant::HighContrast,
                OmarchyVariant::Darker,
            ] {
                preferences.set_omarchy_variant(variant);
                let tokens = manager.appearance_tokens();
                assert_eq!(tokens == startup, variant == OmarchyVariant::Darker);
                for window in &windows {
                    assert_theme_colors(window, &tokens);
                    let rebuilt_view = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    window.set_child(Some(&rebuilt_view));
                    assert_theme_colors(&rebuilt_view, &tokens);
                }
                let rebuilt_buffer = gtk::TextBuffer::new(None);
                rebuilt_buffer.create_tag(Some("document-accent"), &[]);
                crate::ui::theme::register_document_buffer(&rebuilt_buffer);
                for buffer in [&buffer, &rebuilt_buffer] {
                    assert_eq!(
                        buffer
                            .tag_table()
                            .lookup("document-accent")
                            .expect("document accent tag")
                            .foreground_rgba(),
                        Some(gtk::gdk::RGBA::parse(&tokens.accent).expect("valid accent"))
                    );
                }
                assert_eq!(
                    source
                        .style_scheme()
                        .expect("active source scheme")
                        .style("text")
                        .expect("source text style")
                        .background()
                        .as_deref(),
                    Some(tokens.surface.as_str())
                );
                assert_eq!(
                    read_preferences().expect("saved variant").omarchy_variant,
                    variant
                );
            }
            let before_change = manager.appearance_tokens();
            fs::write(
                crate::ui::theme::omarchy_state_dir().join("theme/colors.toml"),
                "background = '#201a12'\nforeground = '#ffeedd'\naccent = '#eebb99'\n",
            )
            .expect("update Omarchy colors");
            wait_for_theme(|| {
                manager.appearance_tokens() != before_change && {
                    #[expect(deprecated, reason = "GTK has no replacement for named CSS colors")]
                    let accent = windows[0]
                        .style_context()
                        .lookup_color("strata_accent")
                        .expect("applied accent");
                    accent
                        == gtk::gdk::RGBA::parse(&manager.appearance_tokens().accent)
                            .expect("valid updated accent")
                }
            });
            assert_eq!(preferences.omarchy_variant(), OmarchyVariant::Darker);
            for window in &windows {
                assert_theme_colors(window, &manager.appearance_tokens());
            }
            manager.set_follow_omarchy(false);
            let builtin = manager.appearance_tokens();
            preferences.set_omarchy_variant(OmarchyVariant::HighContrast);
            assert_eq!(manager.appearance_tokens(), builtin);
            for window in &windows {
                assert_theme_colors(window, &builtin);
            }
            manager.set_follow_omarchy(true);
            assert_ne!(manager.appearance_tokens(), builtin);
            assert_eq!(*values.borrow(), [true, false, true]);
            for window in windows {
                window.close();
            }
            assert_eq!(
                read_preferences().expect("saved theme mode").mode,
                "omarchy"
            );
        },
    );
}

fn assert_theme_colors(widget: &impl IsA<gtk::Widget>, tokens: &crate::ui::theme::ThemeTokens) {
    for (name, expected) in [
        ("strata_bg", &tokens.background),
        ("strata_surface", &tokens.surface),
        ("strata_accent", &tokens.accent),
        ("strata_text", &tokens.text),
    ] {
        #[expect(deprecated, reason = "GTK has no replacement for named CSS colors")]
        let actual = widget
            .style_context()
            .lookup_color(name)
            .expect("applied theme color");
        assert_eq!(
            actual,
            gtk::gdk::RGBA::parse(expected).expect("valid token"),
            "{name}"
        );
    }
}

#[test]
fn omarchy_colors_in_gtk_only_hex_forms_apply_to_css_and_icons() {
    gtk_test(
        "ui::preferences::tests::preferences::omarchy_colors_in_gtk_only_hex_forms_apply_to_css_and_icons",
        || {
            seed_omarchy_colors_for_test(
                "background = '#112233'\nforeground = '#ddeeff'\naccent = '#000aaafff'\nselection = '#1111222233338888'\n",
            );
            let themes = ThemeManager::shared();
            assert!(themes.follows_omarchy());
            let tokens = themes.appearance_tokens();
            assert_eq!(tokens.accent, "#00aaff");
            assert_eq!(tokens.highlight, "#11223388");
            let window = gtk::Window::new();
            for (name, expected) in [
                ("strata_accent", "#00aaff"),
                ("strata_highlight", "#11223388"),
            ] {
                #[expect(deprecated, reason = "GTK has no replacement for named CSS colors")]
                let applied = window.style_context().lookup_color(name);
                assert_eq!(
                    applied,
                    Some(gtk::gdk::RGBA::parse(expected).expect("canonical color")),
                    "{name}"
                );
            }
            assert_eq!(crate::assets::primary_icon_color(), "#00aaff");
            window.close();
        },
    );
}

#[test]
fn invalid_omarchy_palette_while_running_keeps_the_applied_palette() {
    gtk_test(
        "ui::preferences::tests::preferences::invalid_omarchy_palette_while_running_keeps_the_applied_palette",
        || {
            seed_saved_preferences_for_test();
            seed_omarchy_for_test();
            let mut preferences = non_default_preferences();
            preferences.mode = "omarchy".into();
            preferences.omarchy_variant = OmarchyVariant::Original;
            fs::write(
                settings_path(),
                toml::to_string(&preferences).expect("Omarchy preference fixture"),
            )
            .expect("persist Omarchy mode");
            let themes = ThemeManager::shared();
            assert_eq!(crate::assets::primary_icon_color(), "#445566");
            assert_eq!(themes.appearance_tokens().accent, "#445566");

            seed_omarchy_colors_for_test(
                "background = '#112233'\nforeground = '#ddeeff'\naccent = '0x7aa2f7'\n",
            );
            themes.refresh_omarchy_state();
            assert!(themes.follows_omarchy());
            assert!(themes.is_omarchy_available());
            assert_eq!(crate::assets::primary_icon_color(), "#445566");
            assert_eq!(themes.appearance_tokens().accent, "#445566");
            let buffer = gtk::TextBuffer::new(None);
            buffer.create_tag(Some("document-accent"), &[]);
            crate::ui::theme::register_document_buffer(&buffer);
            assert_eq!(
                buffer
                    .tag_table()
                    .lookup("document-accent")
                    .expect("document accent tag")
                    .foreground_rgba(),
                Some(gtk::gdk::RGBA::parse("#445566").expect("accent"))
            );
            let mut preview = themes.starter_tokens();
            preview.accent = "#13579b".to_owned();
            themes.preview(&preview).expect("valid preview");
            assert_eq!(themes.appearance_tokens().accent, "#445566");
            themes.cancel_preview();
            themes.set_follow_omarchy(false);
            themes.set_follow_omarchy(true);
            assert!(themes.follows_omarchy());
            assert_eq!(
                themes.appearance_tokens(),
                themes.starter_tokens(),
                "a built-in theme applied since then replaces the remembered Omarchy palette"
            );

            fs::remove_file(crate::ui::theme::omarchy_state_dir().join("theme.name"))
                .expect("remove Omarchy theme name");
            themes.refresh_omarchy_state();
            assert!(!themes.follows_omarchy());
            assert_eq!(read_preferences().expect("saved theme mode").mode, "theme");
        },
    );
}

fn wait_for_theme(mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ready() {
        assert!(
            std::time::Instant::now() < deadline,
            "live Omarchy refresh timed out"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn all_preference_setters_publish_and_persist_without_duplicate_notifications() {
    gtk_test(
        "ui::preferences::tests::preferences::all_preference_setters_publish_and_persist_without_duplicate_notifications",
        || {
            seed_saved_preferences_for_test();
            seed_omarchy_for_test();
            let manager = PreferenceManager::shared();
            let themes = ThemeManager::shared();
            let anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let observations = Rc::new(RefCell::new(Vec::new()));
            let observed = observations.clone();
            manager.bind_preference(
                &anchor,
                |manager| manager.preferences.borrow().clone(),
                move |_, value| observed.borrow_mut().push(value),
            );
            let preference_setters: &[fn(&PreferenceManager)] = &[
                |m| m.set_folder_peeking(false),
                |m| m.set_single_click_previews(true),
                |m| m.set_columns_mirror_selection(true),
                |m| m.set_render_documents_by_default(true),
                |m| m.set_hardware_accelerated_video_previews(true),
                |m| m.set_video_preview_backend(MediaPreviewBackend::VaApi),
                |m| m.set_search_open_files_directly(false),
                |m| m.set_search_exclusions(vec!["changed_exclusion".to_owned()]),
                |m| m.set_type_to_search(true),
                |m| m.set_arrow_navigation_scoped(false),
                |m| m.set_tenxer_mode(false),
                |m| m.set_filter_include_subfolders(true),
                |m| m.set_show_keybinding_hints(true),
                |m| m.set_window_show_close(true),
                |m| m.set_window_show_minimize(false),
                |m| m.set_window_show_maximize(false),
                |m| m.set_reduce_motion(false),
                |m| m.set_element_glow(true),
                |m| m.set_omarchy_variant(OmarchyVariant::HighContrast),
                |m| m.set_browser_mode(BrowserMode::Icons),
                |m| m.set_browser_density(BrowserDensity::Compact),
                |m| m.set_group_by_type(false),
                |m| {
                    m.set_click_activation(
                        BrowserMode::Columns,
                        crate::ui::browser_modes::ClickActivation::default_for(
                            BrowserMode::Columns,
                        ),
                    )
                },
                |m| {
                    m.set_click_activation(
                        BrowserMode::Icons,
                        crate::ui::browser_modes::ClickActivation::default_for(BrowserMode::Icons),
                    )
                },
                |m| {
                    m.set_click_activation(
                        BrowserMode::List,
                        crate::ui::browser_modes::ClickActivation::default_for(BrowserMode::List),
                    )
                },
                |m| m.set_sidebar_order(default_sidebar_order()),
                |m| m.set_sidebar_show_home(true),
                |m| m.set_sidebar_show_trash(true),
                |m| m.set_sidebar_show_network(true),
                |m| m.set_sidebar_show_recent(true),
                |m| m.set_sidebar_show_desktop(true),
                |m| m.set_sidebar_show_documents(true),
                |m| m.set_sidebar_show_downloads(true),
                |m| m.set_sidebar_show_music(true),
                |m| m.set_sidebar_show_pictures(true),
                |m| m.set_sidebar_show_videos(true),
                |m| m.set_sidebar_expanded(true),
                |m| m.set_sort_preferences(ViewPreferences::default()),
                |m| m.set_text_size(TextSize::new(11)),
                |m| m.set_interface_renderer(InterfaceRenderer::System),
                |m| m.set_language(crate::i18n::Language::Auto),
                |m| m.set_checks_for_updates(true),
                |m| m.set_release_channel(Channel::Stable),
                |m| m.set_preview_muted(false),
                |m| m.set_preview_volume(0.8),
                |m| m.set_preview_text_wrap(false),
                |m| m.set_preview_autoplay(false),
                |m| m.set_auto_refresh_interval(60),
                |m| m.set_thumbnail_workers(3),
                |m| m.set_icons_thumbnail_size(96),
                |m| m.set_chooser_column_width(Some(360)),
                |m| m.set_chooser_list_columns(None),
                |m| m.set_browser_column_width(Some(400)),
                |m| m.set_browser_list_columns(None),
                |m| m.set_cross_volume_drop_strategy(CrossVolumeDropStrategy::Copy),
                |m| m.set_date_format(crate::util::DateFormat::Long),
                |m| m.set_default_directory(None),
                |m| m.set_restore_tabs(true),
                |m| m.set_device_label("volume:fixture-kingston", "Photos / 📁"),
                |m| m.set_device_label("volume:fixture-kingston", ""),
                |m| {
                    m.remember_send_to_destination(
                        "volume:fixture-kingston",
                        Path::new("Research"),
                        None,
                    )
                },
                |m| m.set_open_folder_after_drop(false),
                |m| m.set_folder_color(Path::new("/fixture/folder"), None),
                |m| m.set_custom_icon(Path::new("/fixture/folder"), None),
            ];
            let theme_setters: &[fn(&ThemeManager)] = &[
                |m| m.set_follow_omarchy(true),
                |m| m.select_theme("azure-glow"),
            ];
            let mut previous = toml::Table::try_from(&*manager.preferences.borrow())
                .expect("preference inventory");
            let all_keys = previous
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            let mut changed_keys = std::collections::BTreeSet::new();
            let mut published = 1;
            let mut assert_setter = |published: &mut usize| {
                let expected = manager.preferences.borrow().clone();
                let current =
                    toml::Table::try_from(&expected).expect("changed preference inventory");
                for key in previous.keys().chain(current.keys()) {
                    if previous.get(key) != current.get(key) {
                        changed_keys.insert(key.clone());
                    }
                }
                previous = current;
                assert_eq!(
                    observations.borrow().len(),
                    *published,
                    "setter publishes exactly once"
                );
                assert_eq!(observations.borrow().last(), Some(&expected));
                assert_eq!(read_preferences().expect("saved preferences"), expected);
            };
            for setter in preference_setters {
                setter(&manager);
                published += 1;
                assert_setter(&mut published);
                setter(&manager);
                assert_eq!(
                    observations.borrow().len(),
                    published,
                    "preference setter ignores redundant values"
                );
            }
            for setter in theme_setters {
                setter(&themes);
                published += 1;
                assert_setter(&mut published);
                setter(&themes);
                assert_eq!(
                    observations.borrow().len(),
                    published,
                    "theme setter ignores redundant values"
                );
            }
            assert_eq!(
                changed_keys, all_keys,
                "every stored field needs setter and notification coverage"
            );
        },
    );
}

#[test]
fn saved_date_format_renders_before_settings_and_updates_bound_labels() {
    gtk_test(
        "ui::preferences::tests::preferences::saved_date_format_renders_before_settings_and_updates_bound_labels",
        || {
            seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            let seconds = glib::DateTime::now_local().expect("local time").to_unix() - 120;
            let entry = FileEntry {
                location: Location::local("/fixture/recent.txt"),
                native_name: "recent.txt".into(),
                display_name: "recent.txt".into(),
                thumbnail_path: None,
                kind: EntryKind::File,
                size: MetadataValue::Known(4),
                modified_unix_seconds: MetadataValue::Known(seconds),
                mode: MetadataValue::Known(0o100644),
                recent_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
                recent_uri: None,
            };
            let absolute = |pattern: &str| {
                glib::DateTime::from_unix_local(seconds)
                    .expect("modified date")
                    .format(pattern)
                    .expect("format")
                    .to_string()
            };
            let windows = [gtk::Window::new(), gtk::Window::new()];
            let labels: Vec<gtk::Label> = windows
                .iter()
                .map(|window| {
                    let label = gtk::Label::new(None);
                    window.set_child(Some(&label));
                    crate::util::set_modified_date(&label, Some(&entry), "—");
                    label
                })
                .collect();
            for label in &labels {
                assert_eq!(label.label(), absolute("%Y-%m-%d %H:%M"));
            }
            manager.set_date_format(crate::util::DateFormat::Long);
            for label in &labels {
                assert_eq!(label.label(), absolute("%B %-d, %Y, %H:%M"));
            }
            crate::util::set_modified_date(&labels[0], None, "unknown");
            manager.set_date_format(crate::util::DateFormat::Relative);
            assert_eq!(labels[0].label(), "unknown");
            let text = labels[1].label();
            assert!(text == "Just now" || text.ends_with(" ago"), "{text}");
            let mut rebound = entry.clone();
            rebound.modified_unix_seconds = MetadataValue::Known(seconds - 86400);
            crate::util::set_modified_date(&labels[0], Some(&rebound), "—");
            manager.set_date_format(crate::util::DateFormat::Iso8601);
            let expected = glib::DateTime::from_unix_local(seconds - 86400)
                .expect("rebound date")
                .format("%Y-%m-%d %H:%M")
                .expect("format");
            assert_eq!(labels[0].label(), expected);
            assert_eq!(labels[1].label(), absolute("%Y-%m-%d %H:%M"));
            let rebuilt = gtk::Label::new(None);
            windows[1].set_child(Some(&rebuilt));
            crate::util::set_modified_date(&rebuilt, Some(&entry), "—");
            assert_eq!(rebuilt.label(), absolute("%Y-%m-%d %H:%M"));
            manager.set_date_format(crate::util::DateFormat::Long);
            assert_eq!(rebuilt.label(), absolute("%B %-d, %Y, %H:%M"));
            for window in windows {
                window.close();
            }
        },
    );
}
