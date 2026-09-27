// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use super::*;
use crate::{
    adapters::LocalFileSource,
    services::{
        DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LocationValidationError,
        MetadataRequest,
    },
    ui::{
        go_completion::{
            GioFolders, MAX_MATCHING_FOLDERS,
            tests::{Controlled, folder},
        },
        tenxer_mode::Prompt,
    },
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

fn fixture_with(folders: Rc<dyn crate::ui::go_completion::FolderSource>) -> KeyboardFixture {
    KeyboardFixture::with_parts(Rc::new(TextPreview), folders, browser_for_window)
}

fn enable_tenxer(fixture: &KeyboardFixture) {
    let preferences = PreferenceManager::shared();
    fixture.shortcuts.bind_preferences(&preferences);
    preferences.set_tenxer_mode(true);
    pump(50);
}

/// Adds folders beside the fixture's a.txt, b.txt, and c.txt.
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

fn shift_tab(fixture: &KeyboardFixture) {
    assert!(fixture.press(Key::ISO_Left_Tab, ModifierType::SHIFT_MASK));
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
            let root = seed_folders(&fixture, &["nested"]);
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
            assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            assert!(
                fixture.shortcuts.prompt().text().is_empty(),
                "submit clears the text"
            );
            wait_until(|| browser.active_location() == Some(Location::local(root.join("nested"))));
            wait_until(|| fixture.view.item_view_has_focus());

            open_go(&fixture);
            fixture.shortcuts.prompt().set_text("../");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| browser.active_location() == Some(Location::local(&root)));
            wait_loaded(&browser, 0);
            open_go(&fixture);
            fixture.shortcuts.prompt().set_text("nested");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| browser.active_location() == Some(Location::local(root.join("nested"))));

            let before = browser.active_location();
            open_go(&fixture);
            fixture
                .shortcuts
                .prompt()
                .set_text("/strata-go-missing/nowhere");
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
            let folders = Rc::new(Controlled::default());
            let fixture =
                KeyboardFixture::with_parts(Rc::new(TextPreview), folders.clone(), || {
                    BrowserView::new(source, crate::ui::browser::PeekBehavior::default())
                });
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            let origin = browser.active_location();
            let uri = "sftp://user:hunter2@inert.invalid/srv/with/slashes";

            open_go(&fixture);
            type_text(&fixture, uri);
            tab(&fixture);
            shift_tab(&fixture);
            pump(50);
            assert_eq!(prompt_text(&fixture), uri, "Tab leaves a URI unchanged");
            assert!(fixture.shortcuts.prompt_has_focus());
            assert_eq!(
                fixture.shortcuts.prompt_hint().as_deref(),
                Some("URIs are not completed")
            );
            assert!(folders.requested().is_empty(), "no folder was enumerated");
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
fn tenxer_go_tab_cycles_matching_folders_from_real_folders() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::go_prompt::tenxer_go_tab_cycles_matching_folders_from_real_folders",
        || {
            let home = PathBuf::from(std::env::var_os("HOME").expect("isolated HOME"));
            std::fs::create_dir_all(home.join("Projects")).expect("home folder");
            let fixture = fixture_with(Rc::new(GioFolders));
            enable_tenxer(&fixture);
            let root = seed_folders(&fixture, &["alpha", "Alder", "beta", ".alcove"]);
            std::fs::create_dir(root.join("alpha/inner")).expect("nested folder");
            std::fs::write(root.join("alpine.txt"), b"file").expect("fixture file");
            let browser = fixture.view.browser();

            open_go(&fixture);
            type_text(&fixture, "al");
            tab(&fixture);
            assert_eq!(
                prompt_text(&fixture),
                "Alder/",
                "the listing completes without a slash"
            );
            assert_eq!(fixture.shortcuts.prompt_hint().as_deref(), Some("1 of 2"));
            tab(&fixture);
            assert_eq!(prompt_text(&fixture), "alpha/");
            shift_tab(&fixture);
            assert_eq!(prompt_text(&fixture), "Alder/");
            shift_tab(&fixture);
            assert_eq!(prompt_text(&fixture), "alpha/", "Shift+Tab wraps");
            assert!(fixture.shortcuts.prompt_has_focus());

            type_text(&fixture, "i");
            assert_eq!(
                fixture.shortcuts.prompt_hint(),
                None,
                "an edit clears the hint"
            );
            tab(&fixture);
            wait_until(|| prompt_text(&fixture) == "alpha/inner/");

            let absolute = format!("{}/b", root.display());
            fixture.shortcuts.prompt().set_text(&absolute);
            tab(&fixture);
            wait_until(|| prompt_text(&fixture) == format!("{}/beta/", root.display()));

            fixture.shortcuts.prompt().set_text("~/Pro");
            tab(&fixture);
            wait_until(|| prompt_text(&fixture) == "~/Projects/");

            fixture.shortcuts.prompt().set_text("zz");
            tab(&fixture);
            assert_eq!(prompt_text(&fixture), "zz", "no match keeps the text");
            assert_eq!(
                fixture.shortcuts.prompt_hint().as_deref(),
                Some("No matching folders")
            );
            assert!(fixture.shortcuts.prompt_has_focus());

            browser.toggle_hidden();
            assert!(!browser.preferences().show_hidden);
            fixture.shortcuts.prompt().set_text(".al");
            tab(&fixture);
            assert_eq!(
                prompt_text(&fixture),
                ".alcove/",
                "a dot prefix completes hidden folders the listing hides"
            );

            fixture.shortcuts.prompt().set_text("al");
            tab(&fixture);
            tab(&fixture);
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| browser.active_location() == Some(Location::local(root.join("alpha"))));
        },
    );
}

#[test]
fn tenxer_go_completion_stays_responsive_and_drops_stale_answers() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::go_prompt::tenxer_go_completion_stays_responsive_and_drops_stale_answers",
        || {
            let folders = Rc::new(Controlled::default());
            let fixture = fixture_with(folders.clone());
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            let origin = browser.active_location();
            let pending = |fixture: &KeyboardFixture| {
                fixture.shortcuts.prompt().set_text("/slow/pr");
                tab(fixture);
                pump(20);
                assert_eq!(folders.requested().len(), 1);
                assert_eq!(
                    fixture.shortcuts.prompt_hint().as_deref(),
                    Some("Listing folders\u{2026}")
                );
            };
            let dropped = |fixture: &KeyboardFixture, route: &str| {
                wait_until(|| folders.abandoned(0));
                assert!(
                    !folders.reply(0, Ok(vec![vec![folder("projects")]])),
                    "{route}"
                );
                pump(20);
                assert!(
                    !prompt_text(fixture).contains("projects"),
                    "{route}: a late answer replaces nothing"
                );
            };

            open_go(&fixture);
            pending(&fixture);
            type_text(&fixture, "o");
            assert_eq!(
                prompt_text(&fixture),
                "/slow/pro",
                "edits are accepted while pending"
            );
            assert!(fixture.shortcuts.prompt_has_focus());
            dropped(&fixture, "edit");

            pending(&fixture);
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            dropped(&fixture, "Escape");
            assert_eq!(
                fixture.shortcuts.open_prompt_kind(),
                None,
                "nothing reopens the prompt"
            );

            open_go(&fixture);
            pending(&fixture);
            fixture.shortcuts.open_prompt(Prompt::Find);
            dropped(&fixture, "replace");
            assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Find));
            assert!(fixture.press(Key::Escape, ModifierType::empty()));

            open_go(&fixture);
            pending(&fixture);
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            dropped(&fixture, "submit");
            wait_until(|| modal_visible(&fixture.overlay));
            assert_eq!(browser.active_location(), origin);
            drop(fixture);

            let fixture = fixture_with(folders.clone());
            enable_tenxer(&fixture);
            open_go(&fixture);
            pending(&fixture);
            PreferenceManager::shared().set_tenxer_mode(false);
            dropped(&fixture, "mode exit");
            assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            enable_tenxer(&fixture);

            for (reply, hint) in [
                (
                    Err(crate::ui::go_completion::Unreadable),
                    "Can\u{2019}t read that folder \u{2014} check the path",
                ),
                (
                    Ok(vec![
                        (0..=MAX_MATCHING_FOLDERS)
                            .map(|index| folder(&format!("pr{index}")))
                            .collect(),
                    ]),
                    "Too many entries \u{2014} refine the path",
                ),
            ] {
                open_go(&fixture);
                pending(&fixture);
                assert!(folders.reply(0, reply));
                wait_until(|| fixture.shortcuts.prompt_hint().as_deref() == Some(hint));
                assert_eq!(
                    prompt_text(&fixture),
                    "/slow/pr",
                    "{hint}: the text is kept"
                );
                assert!(fixture.shortcuts.prompt_has_focus());
                assert!(fixture.press(Key::Escape, ModifierType::empty()));
            }

            open_go(&fixture);
            pending(&fixture);
            fixture.window.close();
            dropped(&fixture, "window close");
        },
    );
}
