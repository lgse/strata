// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use super::*;
use crate::ui::window::keyboard::chords::{GoTarget, go_target};

struct Places {
    home: PathBuf,
    downloads: PathBuf,
    music: PathBuf,
    pictures: PathBuf,
    config: PathBuf,
    first_pin: PathBuf,
    second_pin: PathBuf,
}

/// Downloads, Music, and Pictures exist; Documents and Videos are configured but
/// missing. Pins are stored beta, Downloads (hidden as a standard place), alpha.
fn disposable_places() -> Places {
    let home = PathBuf::from(std::env::var_os("HOME").expect("isolated HOME"));
    let config_home =
        PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").expect("isolated config home"));
    let places = Places {
        downloads: home.join("Downloads"),
        music: home.join("Audio Library"),
        pictures: home.join("Pictures"),
        config: home.join(".config"),
        first_pin: home.join("pins/beta"),
        second_pin: home.join("pins/alpha"),
        home,
    };
    for directory in [
        &places.downloads,
        &places.music,
        &places.pictures,
        &places.config,
        &places.first_pin,
        &places.second_pin,
    ] {
        std::fs::create_dir_all(directory).expect("place directory");
    }
    std::fs::create_dir_all(config_home.join("gtk-3.0")).expect("bookmark directory");
    std::fs::write(
        config_home.join("user-dirs.dirs"),
        "XDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\n\
         XDG_DOCUMENTS_DIR=\"$HOME/Documents\"\n\
         XDG_MUSIC_DIR=\"$HOME/Audio Library\"\n\
         XDG_PICTURES_DIR=\"$HOME/Pictures\"\n\
         XDG_VIDEOS_DIR=\"$HOME/Videos\"\n",
    )
    .expect("user dirs");
    let bookmark =
        |path: &Path, name: &str| format!("{} {name}\n", gtk::gio::File::for_path(path).uri());
    std::fs::write(
        config_home.join("gtk-3.0/bookmarks"),
        [
            bookmark(&places.first_pin, "Beta"),
            bookmark(&places.downloads, "Downloads"),
            bookmark(&places.second_pin, "Alpha"),
        ]
        .concat(),
    )
    .expect("bookmarks");
    glib::reload_user_special_dirs_cache();
    places
}

fn visible_keycaps(fixture: &KeyboardFixture) -> Vec<String> {
    fn collect(widget: &gtk::Widget, keycaps: &mut Vec<String>) {
        if widget.has_css_class("sidebar-keycap")
            && let Some(label) = widget.downcast_ref::<gtk::Label>()
            && label.is_visible()
        {
            keycaps.push(label.text().to_string());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            collect(&current, keycaps);
            child = current.next_sibling();
        }
    }
    let mut keycaps = Vec::new();
    collect(&fixture.sidebar.widget, &mut keycaps);
    keycaps
}

fn chord(fixture: &KeyboardFixture, second: Key) {
    focus_files(fixture);
    fixture.shortcuts.dismiss_feedback();
    fixture.press(Key::g, ModifierType::empty());
    fixture.press(second, ModifierType::empty());
    assert_eq!(
        fixture.shortcuts.armed_chord(),
        None,
        "{second:?} ends the chord"
    );
}

#[test]
fn go_chord_resolves_uris_and_visible_pin_order() {
    let pins = [
        Location::local("/pins/beta"),
        Location::local("/pins/alpha"),
    ];
    for (key, location, validate) in [
        (Key::t, Location::uri("trash:///"), false),
        (Key::n, Location::uri("network:///"), true),
        (Key::r, Location::uri("recent:///"), false),
        (Key::_1, pins[0].clone(), true),
        (Key::KP_2, pins[1].clone(), true),
    ] {
        assert_eq!(
            go_target(key, &pins),
            Some(GoTarget::Place { location, validate }),
            "{key:?}"
        );
    }
    assert_eq!(
        go_target(Key::_9, &pins),
        Some(GoTarget::Missing("No pin 9".into()))
    );
    assert_eq!(go_target(Key::space, &pins), Some(GoTarget::Prompt));
    for key in [Key::z, Key::G, Key::q, Key::_0] {
        assert_eq!(go_target(key, &pins), None, "{key:?} is not a place");
    }
}

#[test]
fn tenxer_go_chord_reaches_places_and_cancels_cleanly() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::place_chords::tenxer_go_chord_reaches_places_and_cancels_cleanly",
        || {
            let places = disposable_places();
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(true);
            preferences.set_sidebar_show_downloads(true);
            preferences.set_sidebar_show_documents(true);
            preferences.set_sidebar_show_music(true);
            preferences.set_sidebar_show_pictures(true);
            preferences.set_sidebar_show_videos(true);
            fixture.shortcuts.bind_preferences(&preferences);
            let browser = fixture.view.browser();
            let origin = browser.active_location();
            assert!(visible_keycaps(&fixture).is_empty());

            focus_files(&fixture);
            fixture.press(Key::g, ModifierType::empty());
            assert_eq!(fixture.shortcuts.chord().text(), "g-");
            assert!(fixture.shortcuts.chord().is_visible());
            let option = |key: &str, action: &str| (key.to_owned(), action.to_owned());
            wait_until(|| {
                fixture.shortcuts.chord_options().is_some_and(|options| {
                    options.contains(&option("h", "Home"))
                        && options.contains(&option("m", "Music"))
                        && options.contains(&option("1–9", "Pins"))
                })
            });
            let keycaps = visible_keycaps(&fixture);
            for key in ["h", "d", "k", "m", "p", "v", "1", "2"] {
                assert!(
                    keycaps.contains(&key.to_owned()),
                    "{key} keycap in {keycaps:?}"
                );
            }
            assert!(
                !keycaps.contains(&"3".to_owned()),
                "hidden pins get no keycap"
            );
            fixture.press(Key::Escape, ModifierType::empty());
            assert!(!fixture.shortcuts.chord().is_visible());
            assert_eq!(fixture.shortcuts.chord_options(), None);
            assert!(visible_keycaps(&fixture).is_empty());
            fixture.press(Key::d, ModifierType::empty());
            pump(50);
            assert_eq!(browser.active_location(), origin, "Esc left no pending d");
            wait_until(|| modal_visible(&fixture.overlay));
            assert!(click_class(&fixture.overlay, "action-dialog-close"));
            wait_until(|| !modal_visible(&fixture.overlay));
            focus_files(&fixture);

            for (key, path, feedback) in [
                (Key::d, &places.downloads, "No Downloads folder"),
                (
                    Key::k,
                    &places.home.join("Documents"),
                    "No Documents folder",
                ),
                (Key::m, &places.music, "No Music folder"),
                (Key::p, &places.pictures, "No Pictures folder"),
                (Key::v, &places.home.join("Videos"), "No Videos folder"),
                (Key::c, &places.config, "No .config folder"),
            ] {
                if path.exists() {
                    std::fs::remove_dir_all(path).expect("remove place");
                }
                chord(&fixture, key);
                assert_eq!(fixture.shortcuts.feedback_text(), feedback, "g {key:?}");
                pump(20);
                assert_eq!(browser.active_location(), origin, "g {key:?} stays");
                std::fs::create_dir_all(path).expect("restore place");
            }
            for (key, feedback) in [
                (Key::z, "Unknown chord"),
                (Key::q, "Unknown chord"),
                (Key::f, "Nothing to reveal"),
                (Key::_3, "No pin 3"),
            ] {
                chord(&fixture, key);
                assert_eq!(fixture.shortcuts.feedback_text(), feedback, "g {key:?}");
                pump(20);
                assert_eq!(browser.active_location(), origin, "g {key:?} stays");
                assert!(preferences.tenxer_mode(), "g {key:?} is not q");
            }

            for mode in [BrowserMode::Columns, BrowserMode::Icons] {
                fixture.view.set_view_mode(mode);
                focus_files(&fixture);
                fixture.press(Key::G, ModifierType::SHIFT_MASK);
                wait_until(|| focused_index(&browser) == 2);
                chord(&fixture, Key::g);
                wait_until(|| focused_index(&browser) == 0);
                assert_eq!(browser.active_location(), origin, "{mode:?} g g");
            }
            fixture.view.set_view_mode(BrowserMode::Columns);

            for (key, destination) in [
                (Key::h, &places.home),
                (Key::d, &places.downloads),
                (Key::m, &places.music),
                (Key::p, &places.pictures),
                (Key::c, &places.config),
                (Key::_1, &places.first_pin),
                (Key::_2, &places.second_pin),
            ] {
                chord(&fixture, key);
                wait_until(|| browser.active_location() == Some(Location::local(destination)));
                wait_loaded(&browser, 0);
            }
            chord(&fixture, Key::t);
            assert_eq!(browser.active_location(), Some(Location::uri("trash:///")));

            browser.navigate(Location::local(&places.home));
            wait_loaded(&browser, 0);
            focus_files(&fixture);
            fixture.press(Key::g, ModifierType::empty());
            preferences.set_tenxer_mode(false);
            pump(20);
            assert_eq!(fixture.shortcuts.armed_chord(), None);
            assert!(!fixture.shortcuts.chord().is_visible());
            assert!(visible_keycaps(&fixture).is_empty());
            preferences.set_tenxer_mode(true);
            focus_files(&fixture);
            fixture.press(Key::d, ModifierType::empty());
            pump(50);
            assert_eq!(
                browser.active_location(),
                Some(Location::local(&places.home)),
                "mode exit left no pending g"
            );
            if modal_visible(&fixture.overlay) {
                assert!(click_class(&fixture.overlay, "action-dialog-close"));
                wait_until(|| !modal_visible(&fixture.overlay));
            }
            focus_files(&fixture);

            fixture.press(Key::g, ModifierType::empty());
            fixture.press(Key::comma, ModifierType::CONTROL_MASK);
            assert_eq!(
                fixture.shortcuts.armed_chord(),
                None,
                "Settings entry cancels"
            );

            fixture.press(Key::g, ModifierType::empty());
            assert_eq!(
                fixture.shortcuts.armed_chord(),
                Some(crate::ui::tenxer_mode::Chord::Go)
            );
            fixture.window.destroy();
            assert_eq!(
                fixture.shortcuts.armed_chord(),
                None,
                "window destruction cancels"
            );
        },
    );
}

#[test]
fn tenxer_pin_chords_pin_the_cursor_folder_or_the_current_folder() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::place_chords::tenxer_pin_chords_pin_the_cursor_folder_or_the_current_folder",
        || {
            use std::{cell::RefCell, rc::Rc};

            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(true);
            fixture.shortcuts.bind_preferences(&preferences);
            let directory = fixture._directory.path().to_path_buf();
            let folder = directory.join("folder");
            std::fs::create_dir(&folder).expect("folder");
            let pinned: Rc<RefCell<Vec<(Location, String)>>> = Rc::default();
            let unavailable: Rc<RefCell<Option<Location>>> = Rc::default();
            let (pins, unpins, status) = (pinned.clone(), pinned.clone(), pinned.clone());
            let refused = unavailable.clone();
            fixture.view.set_pin_handlers(
                Rc::new(move |location, name| pins.borrow_mut().push((location, name))),
                Rc::new(move |location| unpins.borrow_mut().retain(|(pin, _)| pin != location)),
                Rc::new(move |location| {
                    if refused.borrow().as_ref() == Some(location) {
                        PinStatus::Unavailable
                    } else if status.borrow().iter().any(|(pin, _)| pin == location) {
                        PinStatus::Pinned
                    } else {
                        PinStatus::Available
                    }
                }),
            );
            fixture.view.refresh();
            let browser = fixture.view.browser();
            wait_until(|| rendered_name(&fixture.view.widget(), "folder"));
            let pin_chord = |key: Key, modifiers: ModifierType| {
                focus_files(&fixture);
                fixture.shortcuts.dismiss_feedback();
                fixture.press(Key::g, ModifierType::empty());
                assert!(fixture.press(key, modifiers), "g {key:?}");
                assert_eq!(fixture.shortcuts.armed_chord(), None);
                fixture.shortcuts.feedback_text()
            };
            let folder_pin = (Location::local(&folder), "folder".to_owned());

            move_to_named(&fixture, &browser, "folder");
            assert_eq!(
                pin_chord(Key::plus, ModifierType::SHIFT_MASK),
                "Pinned \u{201c}folder\u{201d}"
            );
            assert_eq!(*pinned.borrow(), std::slice::from_ref(&folder_pin));
            assert_eq!(
                pin_chord(Key::KP_Add, ModifierType::empty()),
                "\u{201c}folder\u{201d} is already pinned"
            );
            assert_eq!(*pinned.borrow(), std::slice::from_ref(&folder_pin));
            assert_eq!(
                pin_chord(Key::minus, ModifierType::empty()),
                "Unpinned \u{201c}folder\u{201d}"
            );
            assert!(pinned.borrow().is_empty());
            assert_eq!(
                pin_chord(Key::KP_Subtract, ModifierType::empty()),
                "\u{201c}folder\u{201d} isn\u{2019}t pinned"
            );

            let current = Location::local(&directory);
            let current_name = current.display_name();
            move_to_named(&fixture, &browser, "a.txt");
            assert_eq!(
                pin_chord(Key::plus, ModifierType::SHIFT_MASK),
                format!("Pinned \u{201c}{current_name}\u{201d}")
            );
            assert_eq!(
                *pinned.borrow(),
                [(current.clone(), current_name.clone())],
                "a file pins the folder it is in"
            );
            pinned.borrow_mut().clear();

            unavailable.replace(Some(current.clone()));
            assert_eq!(
                pin_chord(Key::plus, ModifierType::SHIFT_MASK),
                format!("Can\u{2019}t pin \u{201c}{current_name}\u{201d}")
            );
            assert!(pinned.borrow().is_empty(), "standard places stay unpinned");
            assert_eq!(
                browser.active_location(),
                Some(current),
                "pinning stays put"
            );
        },
    );
}

fn shortcut_reference_visible(fixture: &KeyboardFixture) -> bool {
    widget_with_class(fixture.window.upcast_ref(), "shortcut-popover")
        .is_some_and(|popover| popover.is_visible())
}

fn arm_go(fixture: &KeyboardFixture) {
    focus_files(fixture);
    fixture.shortcuts.dismiss_feedback();
    fixture.press(Key::g, ModifierType::empty());
    assert_eq!(
        fixture.shortcuts.armed_chord(),
        Some(crate::ui::tenxer_mode::Chord::Go)
    );
    assert_eq!(fixture.shortcuts.chord().text(), "g-");
}

fn assert_place_key_does_not_jump(
    fixture: &KeyboardFixture,
    origin: &Option<crate::model::Location>,
) {
    let reference = shortcut_reference_visible(fixture);
    if !reference {
        focus_files(fixture);
    }
    fixture.press(Key::d, ModifierType::empty());
    if !reference {
        wait_until(|| modal_visible(&fixture.overlay));
        assert!(click_class(&fixture.overlay, "action-dialog-close"));
        wait_until(|| !modal_visible(&fixture.overlay));
    }
    pump(50);
    assert_eq!(fixture.view.browser().active_location(), *origin);
    assert_eq!(fixture.shortcuts.armed_chord(), None);
}

#[test]
fn armed_chord_yields_to_earlier_capture_handlers() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::place_chords::armed_chord_yields_to_earlier_capture_handlers",
        || {
            let places = disposable_places();
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(true);
            preferences.set_sidebar_show_downloads(true);
            fixture.shortcuts.bind_preferences(&preferences);
            let origin = fixture.view.browser().active_location();
            preferences.set_text_size(crate::ui::preferences::TextSize::new(20));

            for (key, expected) in [(Key::plus, 21), (Key::minus, 19), (Key::_0, 13)] {
                preferences.set_text_size(crate::ui::preferences::TextSize::new(20));
                arm_go(&fixture);
                assert!(fixture.press(key, ModifierType::CONTROL_MASK));
                assert_eq!(fixture.shortcuts.armed_chord(), None, "{key:?} clears g-");
                assert!(!fixture.shortcuts.chord().is_visible());
                assert_eq!(preferences.text_size().root_font_px(), expected, "{key:?}");
                assert_place_key_does_not_jump(&fixture, &origin);
            }

            for key in [Key::F1, Key::asciitilde, Key::grave] {
                let modifiers = if key == Key::grave {
                    ModifierType::SHIFT_MASK
                } else {
                    ModifierType::empty()
                };
                arm_go(&fixture);
                assert!(fixture.press(key, modifiers), "{key:?}");
                assert_eq!(fixture.shortcuts.armed_chord(), None, "{key:?} clears g-");
                wait_until(|| shortcut_reference_visible(&fixture));
                assert_place_key_does_not_jump(&fixture, &origin);
                assert!(fixture.press(Key::Escape, ModifierType::empty()));
                wait_until(|| !shortcut_reference_visible(&fixture));
                assert_eq!(fixture.view.browser().active_location(), origin);
            }

            arm_go(&fixture);
            assert!(fixture.press(Key::F1, ModifierType::empty()));
            wait_until(|| shortcut_reference_visible(&fixture));
            fixture
                .shortcuts
                .arm_chord(crate::ui::tenxer_mode::Chord::Go);
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            assert_eq!(fixture.shortcuts.armed_chord(), None, "Escape clears g-");
            wait_until(|| !shortcut_reference_visible(&fixture));
            assert_place_key_does_not_jump(&fixture, &origin);

            arm_go(&fixture);
            assert!(fixture.press(Key::F1, ModifierType::empty()));
            wait_until(|| shortcut_reference_visible(&fixture));
            fixture
                .shortcuts
                .arm_chord(crate::ui::tenxer_mode::Chord::Go);
            fixture.shortcuts.dismiss_feedback();
            assert!(fixture.press(Key::l, ModifierType::CONTROL_MASK));
            assert!(fixture.press(Key::Delete, ModifierType::empty()));
            assert_eq!(
                fixture.shortcuts.armed_chord(),
                None,
                "a key swallowed by the open reference clears g-"
            );
            assert!(shortcut_reference_visible(&fixture));
            assert_ne!(
                fixture.shortcuts.feedback_text(),
                "Unknown chord",
                "Delete is consumed by the reference"
            );
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until(|| !shortcut_reference_visible(&fixture));
            assert_place_key_does_not_jump(&fixture, &origin);

            preferences.set_show_keybinding_hints(false);
            pump(20);
            arm_go(&fixture);
            assert!(fixture.press(Key::F1, ModifierType::empty()));
            assert_eq!(fixture.shortcuts.armed_chord(), None);
            assert!(
                !shortcut_reference_visible(&fixture),
                "a hidden shortcuts button defers the popover"
            );
            fixture
                .shortcuts
                .arm_chord(crate::ui::tenxer_mode::Chord::Go);
            fixture.shortcuts.dismiss_feedback();
            assert!(fixture.press(Key::Down, ModifierType::empty()));
            assert_eq!(
                fixture.shortcuts.armed_chord(),
                None,
                "a key swallowed by the pending reference clears g-"
            );
            assert_ne!(fixture.shortcuts.feedback_text(), "Unknown chord");
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            pump(50);
            assert!(!shortcut_reference_visible(&fixture));
            assert_place_key_does_not_jump(&fixture, &origin);
            preferences.set_show_keybinding_hints(true);

            let scroll =
                widget_with_class(fixture.view.widget().upcast_ref(), "browser-listing-scroll")
                    .and_then(|widget| widget.downcast::<gtk::ScrolledWindow>().ok())
                    .expect("listing scroll");
            arm_go(&fixture);
            assert!(crate::ui::scrolling::begin_autoscroll_for_test(&scroll));
            assert!(crate::ui::scrolling::autoscroll_is_running());
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            assert!(!crate::ui::scrolling::autoscroll_is_running());
            assert_eq!(
                fixture.shortcuts.armed_chord(),
                None,
                "autoscroll Escape clears g-"
            );
            assert_place_key_does_not_jump(&fixture, &origin);

            let _places = places;
        },
    );
}

#[test]
fn settings_and_footer_list_the_same_place_chords() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::place_chords::settings_and_footer_list_the_same_place_chords",
        || {
            PreferenceManager::shared().set_tenxer_mode(true);
            let settings: Vec<_> = crate::ui::shortcut_reference::settings_bindings(true)
                .iter()
                .filter(|binding| binding.category == "Places")
                .copied()
                .collect();
            let sections = crate::ui::shortcut_reference::reference_sections(BrowserMode::Columns);
            let footer = &sections
                .iter()
                .find(|section| section.title == "Places")
                .expect("footer place rows")
                .rows;
            assert_eq!(settings.len(), footer.len());
            for (binding, row) in settings.iter().zip(footer) {
                let (chord, meaning) = *row;
                assert!(
                    meaning == binding.action || meaning == binding.note,
                    "{chord} names {meaning:?}; settings action {:?} note {:?}",
                    binding.action,
                    binding.note
                );
                assert!(
                    chord == binding.keys || chord.strip_prefix(binding.keys).is_some(),
                    "{chord} vs {}",
                    binding.keys
                );
            }
            let preview = "Top of the document or first archive member";
            assert!(
                settings
                    .iter()
                    .any(|binding| binding.action == preview && binding.note == "Preview")
            );
            assert!(
                footer.iter().any(|(chord, meaning)| {
                    *chord == "g g in the preview" && *meaning == preview
                })
            );
        },
    );
}
