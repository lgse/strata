// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::browser::CreateRefusal;

const GROUP: ModifierType = ModifierType::CONTROL_MASK.union(ModifierType::ALT_MASK);

fn submit_create(fixture: &KeyboardFixture, text: &str) {
    focus_files(fixture);
    plain(fixture, Key::a);
    assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Create));
    assert_eq!(
        fixture.shortcuts.prompt_label().as_deref(),
        Some("create \u{203a}")
    );
    fixture.shortcuts.prompt().set_text(text);
    plain(fixture, Key::Return);
}

fn creates_exact_names(fixture: &KeyboardFixture, directory: &Path) {
    for name in ["new.txt", "  spaced  ", "日本語 ✓"] {
        submit_create(fixture, name);
        wait_until(|| fixture.shortcuts.open_prompt_kind().is_none());
        wait_until(|| directory.join(name).is_file());
        assert_eq!(contents(&directory.join(name)), "");
    }
    submit_create(fixture, "folder/");
    wait_until(|| directory.join("folder").is_dir());
}

/// Existing names, including a dangling link, and an overlong name keep the
/// prompt open with the typed text so it can be fixed.
fn refuses_existing_and_overlong_names(fixture: &KeyboardFixture, directory: &Path) {
    std::fs::write(directory.join("new.txt"), "keep").expect("existing contents");
    std::os::unix::fs::symlink(directory.join("missing"), directory.join("link"))
        .expect("dangling link");
    let too_long = "x".repeat(300);
    let refusals = ["new.txt", "folder/", "folder", "link", "link/"]
        .map(|name| {
            let bare = name.trim_end_matches('/');
            (name, format!("\u{201c}{bare}\u{201d} already exists"))
        })
        .into_iter()
        .chain([(too_long.as_str(), "The name is too long".to_owned())]);
    for (name, hint) in refusals {
        submit_create(fixture, name);
        wait_until(|| fixture.shortcuts.prompt_hint().as_ref() == Some(&hint));
        assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Create));
        assert_eq!(
            fixture.shortcuts.prompt_text(),
            name,
            "the name can be fixed"
        );
        plain(fixture, Key::Escape);
    }
    assert_eq!(contents(&directory.join("new.txt")), "keep");
    assert!(
        !directory.join("missing").exists(),
        "links are not followed"
    );
}

fn invalid_names_hint_until_edited(fixture: &KeyboardFixture) {
    for (text, hint) in [
        ("", "Enter a name"),
        ("   ", "Enter a name"),
        ("/", "Enter a name"),
        (".", "That name is reserved"),
        ("../", "That name is reserved"),
        ("nested/name", "Names cannot contain /"),
        ("/absolute", "Names cannot contain /"),
        ("twice//", "Names cannot contain /"),
    ] {
        submit_create(fixture, text);
        assert_eq!(
            fixture.shortcuts.prompt_hint().as_deref(),
            Some(hint),
            "{text:?}"
        );
        fixture.shortcuts.prompt().set_text("edited");
        assert_eq!(
            fixture.shortcuts.prompt_hint(),
            None,
            "editing clears the hint"
        );
        plain(fixture, Key::Escape);
        assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
    }
    assert_eq!(
        fixture.view.prepare_typed_entry("nul\0name").err(),
        Some(CreateRefusal::Invalid(
            "Names cannot contain NUL characters"
        ))
    );
}

fn type_pending_create(fixture: &KeyboardFixture, name: &str) {
    focus_files(fixture);
    plain(fixture, Key::a);
    fixture.shortcuts.prompt().set_text(name);
}

/// Escape, a row click, another prompt, and leaving tenxer mode all discard a
/// typed name.
fn discarded_prompts_create_nothing(fixture: &KeyboardFixture, directory: &Path) {
    let names = directory_names(directory);
    type_pending_create(fixture, "escaped.txt");
    plain(fixture, Key::Escape);
    assert!(fixture.view.item_view_has_focus());
    type_pending_create(fixture, "clicked.txt");
    assert!(press_file_row(&fixture.view.widget(), "b.txt"));
    focus_files(fixture);
    wait_until(|| fixture.shortcuts.open_prompt_kind().is_none());
    assert!(fixture.shortcuts.prompt().text().is_empty());
    type_pending_create(fixture, "replaced.txt");
    fixture.shortcuts.open_prompt(Prompt::Find);
    assert!(fixture.shortcuts.prompt().text().is_empty());
    plain(fixture, Key::Escape);
    type_pending_create(fixture, "mode-exit.txt");
    PreferenceManager::shared().set_tenxer_mode(false);
    pump(50);
    assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
    pump(200);
    assert_eq!(
        directory_names(directory),
        names,
        "discarded prompts create nothing"
    );
}

#[test]
fn tenxer_create_prompt_makes_exact_files_and_folders() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::create::tenxer_create_prompt_makes_exact_files_and_folders",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let directory = fixture._directory.path().to_path_buf();

            creates_exact_names(&fixture, &directory);
            refuses_existing_and_overlong_names(&fixture, &directory);
            invalid_names_hint_until_edited(&fixture);
            discarded_prompts_create_nothing(&fixture, &directory);

            focus_files(&fixture);
            assert!(fixture.press(
                Key::n,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK
            ));
            assert!(
                fixture.view.new_entry_is_active(),
                "Ctrl+Shift+N keeps inline naming"
            );
            plain(&fixture, Key::Escape);
        },
    );
}

fn all_exist(directory: &Path, names: &[&str]) -> bool {
    names.iter().all(|name| directory.join(name).exists())
}

fn all_files(directory: &Path, names: &[&str]) -> bool {
    names.iter().all(|name| directory.join(name).is_file())
}

fn wait_for_named_new_folder(fixture: &KeyboardFixture, folder: &Path) {
    wait_until(|| folder.join("new folder").is_dir());
    wait_until(|| fixture.view.rename_is_active());
}

#[test]
fn ctrl_alt_n_groups_selected_items_into_a_named_folder() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::create::ctrl_alt_n_groups_selected_items_into_a_named_folder",
        || {
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();
            assert!(
                browser.select_entries_by_name_at(0, &["a.txt".to_owned(), "b.txt".to_owned()],)
            );
            focus_files(&fixture);
            assert!(fixture.press(Key::n, GROUP));
            wait_until(|| directory.join("new folder").is_dir());
            wait_until(|| {
                directory.join("new folder/a.txt").is_file()
                    && directory.join("new folder/b.txt").is_file()
            });
            wait_until(|| fixture.view.rename_is_active());
            assert!(!directory.join("a.txt").exists());
            assert!(!directory.join("b.txt").exists());
            assert_eq!(
                directory_names(&directory.join("new folder")),
                ["a.txt", "b.txt"]
            );

            let field = fixture.view.active_rename_field().expect("rename field");
            field.set_text("grouped");
            field.emit_activate();
            wait_until(|| directory.join("grouped/b.txt").is_file());
            assert!(!directory.join("new folder").exists());

            undo(&fixture);
            wait_until(|| {
                directory.join("a.txt").is_file()
                    && directory.join("b.txt").is_file()
                    && !directory.join("grouped").exists()
            });
            assert!(
                !directory.join("new folder").exists(),
                "one undo reverts the whole gesture"
            );

            open_empty_folder(&fixture);
            let empty = directory.join("empty");
            assert!(fixture.press(Key::n, GROUP));
            wait_for_named_new_folder(&fixture, &empty);
            assert_eq!(directory_names(&empty), ["new folder"]);
            plain(&fixture, Key::Escape);
        },
    );
}

/// Ctrl+Alt+N with nothing selected names an empty `new folder` and leaves
/// every listed file in place.
fn creates_an_empty_folder_beside(fixture: &KeyboardFixture, folder: &Path, kept: &[&str]) {
    assert!(fixture.press(Key::n, GROUP));
    wait_for_named_new_folder(fixture, folder);
    assert_eq!(
        directory_names(&folder.join("new folder")),
        Vec::<String>::new()
    );
    assert!(all_files(folder, kept), "{kept:?} stay in place");
    plain(fixture, Key::Escape);
}

#[test]
fn ctrl_alt_n_with_no_selection_creates_plain_new_folder() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::create::ctrl_alt_n_with_no_selection_creates_plain_new_folder",
        || {
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();

            browser.clear_active_selection();
            assert_eq!(browser.selected_count(), 0);
            focus_files(&fixture);
            creates_an_empty_folder_beside(&fixture, &directory, &["a.txt", "b.txt", "c.txt"]);

            let sub = directory.join("sub");
            std::fs::create_dir(&sub).expect("sub directory");
            std::fs::write(sub.join("x.txt"), b"x").expect("file");
            std::fs::write(sub.join("y.txt"), b"y").expect("file");
            browser.navigate(Location::local(&sub));
            wait_until(|| browser.active_location() == Some(Location::local(&sub)));
            wait_loaded(&browser, 0);
            focus_files(&fixture);
            wait_until(|| browser.selection_is_load_cursor());
            creates_an_empty_folder_beside(&fixture, &sub, &["x.txt", "y.txt"]);
        },
    );
}

struct RenameHistory<'a> {
    directory: &'a Path,
    browser: Rc<crate::app::Browser>,
}

impl RenameHistory<'_> {
    fn renamed(&self) -> bool {
        self.directory.join("renamed.txt").exists()
            && !self.directory.join("a.txt").exists()
            && self.browser.pending_undo_rename().is_some()
    }

    fn restored(&self) -> bool {
        self.directory.join("a.txt").exists()
            && !self.directory.join("renamed.txt").exists()
            && self.browser.pending_redo_rename().is_some()
    }
}

fn rename_a_to_renamed(fixture: &KeyboardFixture, history: &RenameHistory) {
    focus_files(fixture);
    assert!(fixture.view.begin_rename());
    let field = fixture.view.active_rename_field().expect("rename field");
    field.set_text("renamed.txt");
    field.emit_activate();
    wait_until(|| history.renamed());
}

fn reference_search_keeps_ctrl_z(fixture: &KeyboardFixture, history: &RenameHistory) {
    focus_files(fixture);
    assert!(fixture.press(Key::F1, ModifierType::empty()));
    wait_until(|| fixture.shortcuts.reference_is_open());
    assert!(
        !fixture.press(Key::z, ModifierType::CONTROL_MASK),
        "the F1 search takes Ctrl+Z"
    );
    pump(100);
    assert!(
        history.renamed(),
        "Ctrl+Z in the F1 search must not undo the rename"
    );
    assert!(fixture.press(Key::Escape, ModifierType::empty()));
    wait_until(|| !fixture.shortcuts.reference_is_open());
}

fn near_misses_keep(
    fixture: &KeyboardFixture,
    chords: &[(Key, ModifierType)],
    kept: impl Fn() -> bool,
    verb: &str,
) {
    for &(key, modifiers) in chords {
        focus_files(fixture);
        fixture.press(key, modifiers);
        pump(100);
        assert!(kept(), "{modifiers:?}+{key:?} must not {verb}");
    }
}

#[test]
fn undo_and_redo_run_only_on_their_exact_chords() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::create::undo_and_redo_run_only_on_their_exact_chords",
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_type_to_search(false);
            let directory = fixture._directory.path().to_path_buf();
            let history = RenameHistory {
                directory: &directory,
                browser: fixture.view.browser(),
            };
            let none = ModifierType::empty();
            let control = ModifierType::CONTROL_MASK;
            let shift = ModifierType::SHIFT_MASK;
            let alt = ModifierType::ALT_MASK;
            rename_a_to_renamed(&fixture, &history);
            reference_search_keeps_ctrl_z(&fixture, &history);

            near_misses_keep(
                &fixture,
                &[
                    (Key::z, none),
                    (Key::z, control | shift),
                    (Key::z, control | alt),
                    (Key::z, control | ModifierType::SUPER_MASK),
                ],
                || history.renamed(),
                "undo",
            );
            focus_files(&fixture);
            assert!(fixture.press(Key::z, control));
            wait_until(|| history.restored());

            near_misses_keep(
                &fixture,
                &[
                    (Key::z, none),
                    (Key::z, shift),
                    (Key::z, control),
                    (Key::z, control | shift | alt),
                    (Key::y, control | shift),
                    (Key::y, control | alt),
                ],
                || history.restored(),
                "redo",
            );
            for (key, modifiers) in [(Key::Z, control | shift), (Key::y, control)] {
                focus_files(&fixture);
                assert!(fixture.press(key, modifiers), "{modifiers:?}+{key:?}");
                wait_until(|| history.renamed());
                focus_files(&fixture);
                assert!(fixture.press(Key::z, control));
                wait_until(|| history.restored());
            }
        },
    );
}

#[derive(Clone, Copy, PartialEq)]
enum GroupNamingDismissal {
    PendingEscape,
    Unmap,
    RenameFailure,
}

/// Escape lands while the grouping transfer is still pending.
fn escape_when_the_transfer_starts(fixture: &KeyboardFixture) {
    let keys = RefCell::new(Some(fixture.keys.clone()));
    fixture.view.browser().observe(move |event| {
        if !matches!(event, BrowserEvent::TransferStarted { .. }) {
            return;
        }
        if let Some(keys) = keys.take() {
            assert!(keys.emit_by_name::<bool>(
                "key-pressed",
                &[&Key::Escape, &0u32, &ModifierType::empty()]
            ));
        }
    });
}

/// Set once a transfer reports its end, which is when its undo entry lands; the
/// files reach disk earlier, so their presence alone races that entry.
fn transfer_finished(browser: &crate::app::Browser) -> Rc<Cell<bool>> {
    let finished = Rc::new(Cell::new(false));
    let seen = finished.clone();
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::TransferFinished { .. }) {
            seen.set(true);
        }
    });
    finished
}

fn abandon_group_naming(fixture: &KeyboardFixture, dismissal: GroupNamingDismissal) {
    wait_until(|| fixture.view.rename_is_active());
    let field = fixture.view.active_rename_field().expect("gesture field");
    if dismissal == GroupNamingDismissal::Unmap {
        field.set_visible(false);
    } else {
        field.set_text("c.txt");
        field.emit_activate();
        wait_until(|| modal_visible(&fixture.overlay));
        close_modal(fixture);
    }
    wait_until(|| !fixture.view.rename_is_active());
}

/// A later F2 rename of the group folder undoes on its own before the grouping.
fn later_rename_undoes_separately(fixture: &KeyboardFixture, directory: &Path) {
    let browser = fixture.view.browser();
    wait_until(|| browser.select_entries_by_name_at(0, &["new folder".to_owned()]));
    focus_files(fixture);
    plain(fixture, Key::F2);
    wait_until(|| fixture.view.rename_is_active());
    let field = fixture.view.active_rename_field().expect("later rename");
    field.set_text("later");
    field.emit_activate();
    wait_until(|| directory.join("later/a.txt").exists());
    undo(fixture);
    wait_until(|| directory.join("new folder/a.txt").exists());
    assert!(directory.join("new folder/b.txt").exists());
    assert!(!directory.join("a.txt").exists());
    assert!(!directory.join("later").exists());
    undo(fixture);
    wait_until(|| {
        all_exist(directory, &["a.txt", "b.txt"]) && !directory.join("new folder").exists()
    });
}

fn abandoned_group_naming_case(mode: BrowserMode, dismissal: GroupNamingDismissal) {
    let fixture = KeyboardFixture::new();
    fixture.view.set_view_mode(mode);
    let directory = fixture._directory.path();
    let browser = fixture.view.browser();
    wait_until(|| browser.select_entries_by_name_at(0, &["a.txt".to_owned(), "b.txt".to_owned()]));
    if dismissal == GroupNamingDismissal::PendingEscape {
        escape_when_the_transfer_starts(&fixture);
    }
    let grouped = transfer_finished(&browser);
    focus_files(&fixture);
    assert!(fixture.press(Key::n, GROUP));
    wait_until(|| grouped.get());
    assert!(all_exist(
        directory,
        &["new folder/a.txt", "new folder/b.txt"]
    ));
    if dismissal != GroupNamingDismissal::PendingEscape {
        abandon_group_naming(&fixture, dismissal);
    }
    later_rename_undoes_separately(&fixture, directory);
}

#[test]
fn abandoned_group_naming_keeps_a_later_rename_separate() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::create::abandoned_group_naming_keeps_a_later_rename_separate",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                for dismissal in [
                    GroupNamingDismissal::PendingEscape,
                    GroupNamingDismissal::Unmap,
                    GroupNamingDismissal::RenameFailure,
                ] {
                    abandoned_group_naming_case(mode, dismissal);
                }
            }
        },
    );
}
