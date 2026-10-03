// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use super::*;
use crate::{
    adapters::LocalFileSource,
    services::{
        DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LocationValidationError,
        MetadataRequest,
    },
    ui::tenxer_mode::Prompt,
};

/// Local browsing, except that URI validation is recorded and refused so no
/// test reaches a network backend.
#[derive(Default)]
struct InertUris {
    validated: Rc<RefCell<Vec<Location>>>,
}

impl FileSource for InertUris {
    fn allows_entry(&self, entry: &crate::model::FileEntry) -> bool {
        LocalFileSource.allows_entry(entry)
    }

    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        LocalFileSource.validate_location(location)
    }

    fn validate_location_async(
        &self,
        location: Location,
        emit: Rc<dyn Fn(Result<(), LocationValidationError>)>,
    ) -> LoadHandle {
        if location.native_path().is_some() {
            return LocalFileSource.validate_location_async(location, emit);
        }
        self.validated.borrow_mut().push(location);
        emit(Err(LocationValidationError::Unavailable("inert".into())));
        LoadHandle::new(|| {})
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        LocalFileSource.enumerate(request, emit)
    }

    fn supports_metadata_fill(&self, location: &Location) -> bool {
        LocalFileSource.supports_metadata_fill(location)
    }

    fn fill_metadata(
        &self,
        request: MetadataRequest,
        emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        LocalFileSource.fill_metadata(request, emit)
    }

    fn watch(
        &self,
        location: Location,
        include_hidden: bool,
        notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        LocalFileSource.watch(location, include_hidden, notify)
    }
}

#[test]
fn tenxer_go_works_after_dialog_close_and_chained_confirmation() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::go_prompt::tenxer_go_works_after_dialog_close_and_chained_confirmation",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            install_modal_focus_trap(&fixture.window);
            let row = sidebar_button(crate::assets::icons::HARD_DRIVE, "Drive");
            fixture.sidebar.state.places_for_test().append(&row);
            let action = gtk::Button::with_label("Open drive dialog");
            let popover = gtk::Popover::builder()
                .child(&action)
                .has_arrow(false)
                .build();
            popover.add_css_class("folder-context-popover");
            popover.set_parent(&row);
            let opened = Rc::new(RefCell::new(None));
            let entry_focus = Rc::new(Cell::new(false));
            let requested_entry = entry_focus.clone();
            let dialog = opened.clone();
            let view = fixture.view.clone();
            let menu = popover.downgrade();
            action.connect_clicked(move |_| {
                super::super::super::drive_dialogs::open_from_sidebar(
                    &view,
                    menu.upgrade().as_ref(),
                    || {
                        dialog.replace(Some(
                            super::super::super::drive_dialogs::tests::focus_dialog_fixture(
                                &view.widget(),
                                requested_entry.get(),
                            ),
                        ));
                    },
                );
            });
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                fixture.view.set_view_mode(mode);
                for (with_entry, chained) in [(false, false), (true, false), (true, true)] {
                    entry_focus.set(with_entry);
                    row.grab_focus();
                    assert!(fixture.press(Key::F10, ModifierType::SHIFT_MASK));
                    wait_until(|| popover.is_visible());
                    action.grab_focus();
                    action.emit_clicked();
                    let (first, close) = opened.borrow_mut().take().expect("drive dialog");
                    let (last, close) = if chained {
                        let next = super::super::super::drive_dialogs::tests::focus_dialog_fixture(
                            &fixture.view.widget(),
                            false,
                        );
                        close.emit_clicked();
                        wait_until(|| first.parent().is_none());
                        assert!(!fixture.view.item_view_has_focus());
                        fixture.view.set_view_mode(if mode == BrowserMode::Columns {
                            BrowserMode::List
                        } else {
                            BrowserMode::Columns
                        });
                        next
                    } else {
                        (first, close)
                    };
                    close.emit_clicked();
                    wait_until(|| last.parent().is_none());
                    assert!(
                        fixture.view.item_view_has_focus(),
                        "{mode:?}, entry={with_entry}, chained={chained}"
                    );
                    assert!(fixture.press(Key::g, ModifierType::empty()));
                    assert!(fixture.press(Key::space, ModifierType::empty()));
                    assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Go));
                    assert!(fixture.shortcuts.prompt_has_focus());
                    assert!(fixture.press(Key::Escape, ModifierType::empty()));
                }
            }
        },
    );
}

fn enable_tenxer(fixture: &KeyboardFixture) {
    let preferences = PreferenceManager::shared();
    fixture.shortcuts.bind_preferences(&preferences);
    preferences.set_tenxer_mode(true);
    pump(50);
}

fn seed_folders(fixture: &KeyboardFixture, names: &[&str]) -> PathBuf {
    let root = fixture._directory.path().to_path_buf();
    for name in names {
        std::fs::create_dir(root.join(name)).expect("fixture folder");
    }
    let browser = fixture.view.browser();
    fixture.view.refresh();
    wait_loaded(&browser, 0);
    wait_until(|| entry_count(&browser) == 3 + names.len());
    root
}

fn open_go(fixture: &KeyboardFixture) {
    focus_files(fixture);
    assert!(fixture.press(Key::g, ModifierType::empty()));
    assert!(fixture.press(Key::space, ModifierType::empty()));
    assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Go));
    assert!(fixture.shortcuts.prompt_has_focus());
    assert!(
        fixture.shortcuts.prompt().text().is_empty(),
        "no earlier text is recovered"
    );
}

fn tab(fixture: &KeyboardFixture) {
    assert!(fixture.press(Key::Tab, ModifierType::empty()));
}

fn listed(fixture: &KeyboardFixture, text: &str, expected: &[PathBuf]) {
    fixture.shortcuts.prompt().set_text(text);
    wait_until(|| fixture.shortcuts.candidates() == expected);
}

fn prompt_text(fixture: &KeyboardFixture) -> String {
    fixture.shortcuts.prompt().text().to_string()
}

/// Types like a user, so GTK's undo history would see the text.
fn type_text(fixture: &KeyboardFixture, text: &str) {
    let editable = fixture
        .shortcuts
        .prompt()
        .upcast_ref::<gtk::Editable>()
        .clone();
    let mut position = editable.text().chars().count() as i32;
    editable.insert_text(text, &mut position);
}

fn undo(fixture: &KeyboardFixture) {
    if let Some(text) = fixture.shortcuts.prompt().delegate() {
        let _ = text.activate_action("text.undo", None);
    }
}

#[test]
fn tenxer_g_space_opens_go_and_submits_typed_paths() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::go_prompt::tenxer_g_space_opens_go_and_submits_typed_paths",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let root = seed_folders(&fixture, &["nested", "ghosts"]);
            std::fs::write(root.join("hosts"), b"hosts").expect("fixture file");
            let browser = fixture.view.browser();
            let origin = browser.active_location();
            move_to_named(&fixture, &browser, "a.txt");
            let fill = fill_names(&browser);

            open_go(&fixture);
            assert_eq!(
                fixture.shortcuts.prompt_label().as_deref(),
                Some("go \u{203a}")
            );
            assert_eq!(
                fill_names(&browser),
                fill,
                "Space opened the prompt, not selection"
            );
            let generation = browser.navigation_generation();
            type_text(&fixture, &root.join("nested").to_string_lossy());
            wait_until(|| fixture.shortcuts.candidates() == [root.join("nested")]);
            for key in [Key::j, Key::space, Key::h] {
                fixture.press(key, ModifierType::empty());
            }
            pump(50);
            assert_eq!(
                browser.navigation_generation(),
                generation,
                "typing never navigates"
            );
            assert_eq!(browser.active_location(), origin);

            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| fixture.shortcuts.open_prompt_kind().is_none());
            assert!(
                fixture.shortcuts.prompt().text().is_empty(),
                "submit clears the text"
            );
            wait_until(|| browser.active_location() == Some(Location::local(root.join("nested"))));
            wait_until(|| fixture.view.item_view_has_focus());

            open_go(&fixture);
            fixture.shortcuts.prompt().set_text("../");
            wait_until(|| fixture.shortcuts.candidates().first() == Some(&root));
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| browser.active_location() == Some(Location::local(&root)));
            wait_loaded(&browser, 0);
            open_go(&fixture);
            type_text(&fixture, "nest");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| browser.active_location() == Some(Location::local(root.join("nested"))));
            assert_eq!(
                fixture.shortcuts.open_prompt_kind(),
                None,
                "Enter before the results waits for them"
            );

            open_go(&fixture);
            let file = root.join("b.txt");
            fixture.shortcuts.prompt().set_text(&file.to_string_lossy());
            wait_until(|| {
                fixture.shortcuts.prompt_hint().as_deref() == Some("No matching folders")
            });
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| browser.active_location() == Some(Location::local(&root)));
            wait_until(|| focused_name(&browser) == "b.txt");

            open_go(&fixture);
            listed(
                &fixture,
                &root.join("hosts").to_string_lossy(),
                &[root.join("ghosts")],
            );
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| focused_name(&browser) == "hosts");
            assert_eq!(
                browser.active_location(),
                Some(Location::local(&root)),
                "a typed file opens rather than a folder it fuzzily matches"
            );

            let before = browser.active_location();
            open_go(&fixture);
            fixture
                .shortcuts
                .prompt()
                .set_text("/strata-go-missing/nowhere");
            wait_until(|| {
                fixture.shortcuts.prompt_hint().as_deref() == Some("No matching folders")
            });
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| modal_visible(&fixture.overlay));
            pump(50);
            assert_eq!(
                browser.active_location(),
                before,
                "a missing destination reports an error and stays put"
            );
        },
    );
}

#[test]
fn tenxer_go_clears_typed_text_on_every_dismissal() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::go_prompt::tenxer_go_clears_typed_text_on_every_dismissal",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            let origin = browser.active_location();
            let secret = "sftp://user:hunter2@inert.invalid/srv";

            open_go(&fixture);
            type_text(&fixture, secret);
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            assert!(
                fixture.shortcuts.prompt().text().is_empty(),
                "Escape clears"
            );
            assert!(fixture.view.item_view_has_focus());
            open_go(&fixture);
            undo(&fixture);
            assert!(
                !prompt_text(&fixture).contains("hunter2"),
                "undo cannot bring the text back"
            );

            type_text(&fixture, secret);
            move_to_named(&fixture, &browser, "a.txt");
            assert!(press_file_row(&fixture.view.widget(), "b.txt"));
            wait_until(|| fixture.shortcuts.open_prompt_kind().is_none());
            assert!(
                fixture.shortcuts.prompt().text().is_empty(),
                "focus loss clears"
            );
            assert_eq!(
                focused_name(&browser),
                "b.txt",
                "the clicked row keeps its selection"
            );

            open_go(&fixture);
            type_text(&fixture, secret);
            fixture.shortcuts.open_prompt(Prompt::Find);
            assert!(
                fixture.shortcuts.prompt().text().is_empty(),
                "a replacing prompt starts empty"
            );
            assert!(fixture.press(Key::Escape, ModifierType::empty()));

            open_go(&fixture);
            type_text(&fixture, secret);
            PreferenceManager::shared().set_tenxer_mode(false);
            pump(50);
            assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            assert!(
                fixture.shortcuts.prompt().text().is_empty(),
                "mode exit clears"
            );
            assert!(fixture.view.item_view_has_focus());
            assert_eq!(browser.active_location(), origin, "nothing was submitted");
        },
    );
}

#[test]
fn tenxer_go_submits_uris_unchanged_and_never_probes_them_first() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::go_prompt::tenxer_go_submits_uris_unchanged_and_never_probes_them_first",
        || {
            let source = Rc::new(InertUris::default());
            let validated = source.validated.clone();
            let fixture = KeyboardFixture::with_parts(Rc::new(TextPreview), || {
                BrowserView::new(source, crate::ui::browser::PeekBehavior::default())
            });
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            let origin = browser.active_location();
            let uri = "sftp://user:hunter2@inert.invalid/srv/with/slashes";

            open_go(&fixture);
            type_text(&fixture, uri);
            tab(&fixture);
            pump(50);
            assert_eq!(prompt_text(&fixture), uri, "Tab leaves a URI unchanged");
            assert!(fixture.shortcuts.prompt_has_focus());
            assert_eq!(fixture.shortcuts.prompt_hint(), None);
            assert!(
                fixture.shortcuts.candidates().is_empty(),
                "no folder was searched"
            );
            assert!(
                validated.borrow().is_empty(),
                "nothing is probed before Enter"
            );

            assert!(fixture.press(Key::Return, ModifierType::empty()));
            assert!(fixture.shortcuts.prompt().text().is_empty());
            wait_until(|| modal_visible(&fixture.overlay));
            let validated = validated.borrow();
            assert_eq!(validated.len(), 1, "Enter hands the URI to navigation once");
            let submitted = validated[0].uri_value().expect("URI location");
            assert!(submitted.contains("inert.invalid/srv/with/slashes"));
            assert!(
                !submitted.contains("hunter2"),
                "credentials go to the mount, not the URI"
            );
            assert_eq!(browser.active_location(), origin);
        },
    );
}

#[test]
fn tenxer_go_lists_matching_folders_and_tab_fills_the_chosen_one() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::go_prompt::tenxer_go_lists_matching_folders_and_tab_fills_the_chosen_one",
        || {
            let home = PathBuf::from(std::env::var_os("HOME").expect("isolated HOME"));
            std::fs::create_dir_all(home.join("Projects")).expect("home folder");
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let root = seed_folders(&fixture, &["alpha", "beta", ".alcove"]);
            std::fs::create_dir(root.join("alpha/inner")).expect("nested folder");
            std::fs::write(root.join("alpine.txt"), b"file").expect("fixture file");
            let browser = fixture.view.browser();
            let origin = browser.active_location();
            let hint = || fixture.shortcuts.prompt_hint();

            open_go(&fixture);
            type_text(&fixture, "al in");
            wait_until(|| fixture.shortcuts.candidates() == [root.join("alpha/inner")]);
            assert!(fixture.shortcuts.candidates_shown());
            assert_eq!(hint(), None);
            assert!(
                fixture.shortcuts.candidate_keys().contains("Tab Complete"),
                "the strip names what Tab does"
            );

            listed(
                &fixture,
                "alp",
                &[root.join("alpha"), root.join("alpha/inner")],
            );
            assert_eq!(hint().as_deref(), Some("1 of 2"), "files are never listed");
            assert!(fixture.press(Key::Down, ModifierType::empty()));
            assert_eq!(hint().as_deref(), Some("2 of 2"));
            tab(&fixture);
            assert_eq!(prompt_text(&fixture), "./alpha/inner/");
            wait_until(|| fixture.shortcuts.candidates() == [root.join("alpha/inner")]);
            assert_eq!(browser.active_location(), origin, "Tab never navigates");
            assert!(fixture.shortcuts.prompt_has_focus());

            for _ in 0..3 {
                crate::services::NavigationHistory::shared().record(&root.join("beta"));
            }
            fixture.shortcuts.prompt().set_text("./");
            wait_until(|| fixture.shortcuts.candidates().len() == 5);
            let listing = fixture.shortcuts.candidates();
            assert_eq!(
                listing[..2],
                [root.clone(), root.join("beta")],
                "the typed folder, then the most visited below it"
            );
            assert_eq!(listing[4], root.join("alpha/inner"), "then the shallowest");

            fixture.shortcuts.prompt().set_text("~/Pro");
            wait_until(|| fixture.shortcuts.candidates().first() == Some(&home.join("Projects")));
            tab(&fixture);
            assert_eq!(prompt_text(&fixture), "~/Projects/");

            browser.toggle_hidden();
            assert!(!browser.preferences().show_hidden);
            fixture.shortcuts.prompt().set_text("alc");
            wait_until(|| hint().as_deref() == Some("No matching folders"));
            listed(&fixture, ".alc", &[root.join(".alcove")]);

            listed(
                &fixture,
                "alp",
                &[root.join("alpha"), root.join("alpha/inner")],
            );
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            wait_until(|| browser.active_location() == Some(Location::local(root.join("alpha"))));
        },
    );
}
