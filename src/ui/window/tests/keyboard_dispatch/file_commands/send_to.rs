// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::shortcut_footer::CandidateKeys;

struct Destinations {
    directory: PathBuf,
    destination: PathBuf,
    inner: PathBuf,
    other: PathBuf,
}

impl Destinations {
    fn create(fixture: &KeyboardFixture) -> Self {
        let directory = fixture._directory.path().to_path_buf();
        let destination = directory.join("dest");
        let inner = destination.join("inner");
        let other = directory.join("other");
        std::fs::create_dir_all(&inner).expect("destination");
        std::fs::create_dir(&other).expect("sibling destination");
        Self {
            directory,
            destination,
            inner,
            other,
        }
    }
}

struct FolderPrompt<'a> {
    fixture: &'a KeyboardFixture,
    browser: Rc<crate::app::Browser>,
    origin: Option<Location>,
}

impl FolderPrompt<'_> {
    fn open(&self, key: Key, kind: Prompt, label: &str) {
        focus_files(self.fixture);
        shifted(self.fixture, key);
        assert_eq!(self.fixture.shortcuts.open_prompt_kind(), Some(kind));
        assert_eq!(
            self.fixture.shortcuts.prompt_label().as_deref(),
            Some(label)
        );
    }

    fn open_move(&self) {
        self.open(Key::M, Prompt::MoveTo, "move to \u{203a}");
    }

    fn open_copy(&self) {
        self.open(Key::C, Prompt::CopyTo, "copy to \u{203a}");
    }

    fn listed(&self, text: &str, expected: &[&Path]) {
        self.fixture.shortcuts.prompt().set_text(text);
        wait_until(|| self.fixture.shortcuts.candidates() == expected);
        assert!(self.fixture.shortcuts.candidates_shown(), "{text:?}");
    }

    fn hint(&self) -> Option<String> {
        self.fixture.shortcuts.prompt_hint()
    }

    fn refused(&self, text: &str, expected: &str) {
        self.fixture.shortcuts.prompt().set_text(text);
        plain(self.fixture, Key::Return);
        wait_until(|| self.hint().as_deref() == Some(expected));
        assert_eq!(
            self.fixture.shortcuts.open_prompt_kind(),
            Some(Prompt::MoveTo)
        );
    }

    fn sorted_candidates(&self) -> Vec<PathBuf> {
        let mut found = self.fixture.shortcuts.candidates();
        found.sort();
        found
    }

    fn assert_stays_put(&self, verb: &str) {
        assert_eq!(
            self.browser.active_location(),
            self.origin,
            "{verb} stays put"
        );
    }
}

fn copy_the_fill(prompt: &FolderPrompt, paths: &Destinations) {
    fill_b_and_c(prompt.fixture);
    prompt.open_copy();
    prompt.listed("dest", &[&paths.destination, &paths.inner]);
    assert_eq!(prompt.hint().as_deref(), Some("1 of 2"));
    plain(prompt.fixture, Key::Return);
    wait_until(|| prompt.fixture.shortcuts.open_prompt_kind().is_none());
    wait_until(|| directory_names(&paths.destination) == ["b.txt", "c.txt", "inner"]);
    assert!(paths.directory.join("b.txt").exists() && paths.directory.join("c.txt").exists());
    prompt.assert_stays_put("copying");
}

/// Moving the cursor item leaves its neighbor cursor-only.
fn move_the_cursor(prompt: &FolderPrompt, paths: &Destinations) {
    let browser = &prompt.browser;
    browser.clear_active_selection();
    move_to_named(prompt.fixture, browser, "a.txt");
    prompt.open_move();
    prompt.listed("dest in", &[&paths.inner]);
    plain(prompt.fixture, Key::Return);
    wait_until(|| paths.inner.join("a.txt").exists() && !paths.directory.join("a.txt").exists());
    assert_eq!(prompt.fixture.shortcuts.open_prompt_kind(), None);
    prompt.assert_stays_put("moving");
    wait_until(|| focused_name(browser) == "b.txt");
    assert!(
        fill_names(browser).is_empty(),
        "the neighbor is cursor-only"
    );
    plain(prompt.fixture, Key::j);
    assert_eq!(focused_name(browser), "c.txt");
    assert!(fill_names(browser).is_empty());
}

fn refusals_move_nothing(prompt: &FolderPrompt, paths: &Destinations) {
    let fixture = prompt.fixture;
    let browser = &prompt.browser;
    browser.clear_active_selection();
    move_to_named(fixture, browser, "b.txt");
    prompt.open_move();
    plain(fixture, Key::Return);
    assert_eq!(
        fixture.shortcuts.open_prompt_kind(),
        None,
        "empty Enter closes"
    );
    for (text, expected) in [
        ("nowhere", "No matching folders"),
        ("smb://host/share", "Only local folders can be chosen"),
        ("~someone/x", "Only ~ and ~/ are supported"),
        ("./", "Already in this folder"),
        ("./c.txt/", "Not a folder"),
        ("./nowhere/", "No such folder"),
    ] {
        prompt.open_move();
        plain(fixture, Key::Down);
        assert_eq!(focused_name(browser), "b.txt", "the target is fixed");
        prompt.refused(text, expected);
        plain(fixture, Key::Escape);
        assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
    }
    assert!(
        paths.directory.join("b.txt").exists(),
        "refusals move nothing"
    );

    prompt.open_move();
    fixture.shortcuts.prompt().set_text("./");
    wait_until(|| {
        prompt.sorted_candidates()
            == [
                paths.destination.clone(),
                paths.inner.clone(),
                paths.other.clone(),
            ]
    });
    plain(fixture, Key::Escape);
}

fn a_folder_is_never_sent_inside_itself(prompt: &FolderPrompt, paths: &Destinations) {
    let fixture = prompt.fixture;
    move_to_named(fixture, &prompt.browser, "dest");
    prompt.open_move();
    fixture.shortcuts.prompt().set_text("inner");
    wait_until(|| prompt.hint().as_deref() == Some("No matching folders"));
    assert!(
        fixture.shortcuts.candidates().is_empty(),
        "a folder is never offered inside itself"
    );
    prompt.refused("./dest/inner/", "Can\u{2019}t put a folder inside itself");
    plain(fixture, Key::Escape);
    prompt.open_copy();
    prompt.listed("./dest/../other/", &[&paths.other]);
    plain(fixture, Key::Return);
    wait_until(|| paths.other.join("dest/inner/a.txt").exists());
    assert!(
        paths.inner.join("a.txt").exists(),
        "copy preserves the source"
    );
}

fn an_absolute_path_reaches_the_conflict_dialog(prompt: &FolderPrompt, paths: &Destinations) {
    let fixture = prompt.fixture;
    move_to_named(fixture, &prompt.browser, "b.txt");
    prompt.open_copy();
    let typed = format!("{}/", paths.destination.display());
    prompt.listed(&typed, &[&paths.destination, &paths.inner]);
    plain(fixture, Key::Return);
    wait_until(|| modal_visible(&fixture.overlay));
    wait_focused_button(fixture, "Keep Both");
    close_modal(fixture);
    assert_eq!(
        directory_names(&paths.destination),
        ["b.txt", "c.txt", "inner"]
    );
}

/// Up and Down step through the matches, later results keep the stepped
/// choice, and Tab writes it into the prompt without sending.
fn stepping_chooses_and_tab_completes(prompt: &FolderPrompt, paths: &Destinations) {
    let fixture = prompt.fixture;
    let browser = &prompt.browser;
    move_to_named(fixture, browser, "c.txt");
    prompt.open_move();
    fixture.shortcuts.prompt().set_text("inner");
    wait_until(|| fixture.shortcuts.candidates().len() == 2);
    assert_eq!(
        prompt.sorted_candidates(),
        [paths.inner.clone(), paths.other.join("dest/inner")]
    );
    for (key, position) in [
        (Key::Up, "2 of 2"),
        (Key::Down, "1 of 2"),
        (Key::Down, "2 of 2"),
    ] {
        plain(fixture, key);
        assert_eq!(prompt.hint().as_deref(), Some(position), "{key:?}");
    }
    assert_eq!(focused_name(browser), "c.txt", "stepping keeps the cursor");
    let chosen = fixture.shortcuts.candidates()[1].clone();
    let later = fixture.shortcuts.candidates();
    fixture
        .shortcuts
        .show_candidates(later, CandidateKeys::default());
    assert_eq!(
        fixture.shortcuts.chosen_candidate().as_ref(),
        Some(&chosen),
        "results that arrive later keep the stepped choice"
    );
    let relative = chosen
        .strip_prefix(&paths.directory)
        .expect("candidate below");
    plain(fixture, Key::Tab);
    assert_eq!(
        fixture.shortcuts.prompt_text(),
        format!("./{}/", relative.display()),
        "Tab writes the chosen folder into the prompt"
    );
    assert!(paths.directory.join("c.txt").exists(), "Tab never sends");
    wait_until(|| fixture.shortcuts.candidates() == [chosen.clone()]);
    plain(fixture, Key::Return);
    wait_until(|| chosen.join("c.txt").exists() && !paths.directory.join("c.txt").exists());
}

fn cancelled_validation_sends_nothing_and_a_click_copies(
    prompt: &FolderPrompt,
    paths: &Destinations,
) {
    let fixture = prompt.fixture;
    move_to_named(fixture, &prompt.browser, "b.txt");
    prompt.open_move();
    prompt.listed("^other$", &[&paths.other]);
    plain(fixture, Key::Return);
    plain(fixture, Key::Escape);
    pump(200);
    assert!(
        paths.directory.join("b.txt").exists(),
        "cancelled validation must not move"
    );
    assert!(!paths.other.join("b.txt").exists());

    prompt.open_copy();
    prompt.listed("^other$", &[&paths.other]);
    fixture.shortcuts.click_candidate(0);
    wait_until(|| paths.other.join("b.txt").exists());
    assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
    assert!(paths.directory.join("b.txt").exists());
}

#[test]
fn tenxer_move_and_copy_prompts_send_targets_to_a_picked_folder() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::send_to::tenxer_move_and_copy_prompts_send_targets_to_a_picked_folder",
        || {
            let fixture = KeyboardFixture::new();
            let paths = Destinations::create(&fixture);
            fixture.view.refresh();
            wait_until(|| rendered_name(&fixture.view.widget(), "dest"));
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            let prompt = FolderPrompt {
                fixture: &fixture,
                origin: browser.active_location(),
                browser,
            };

            copy_the_fill(&prompt, &paths);
            move_the_cursor(&prompt, &paths);
            refusals_move_nothing(&prompt, &paths);
            a_folder_is_never_sent_inside_itself(&prompt, &paths);
            an_absolute_path_reaches_the_conflict_dialog(&prompt, &paths);
            stepping_chooses_and_tab_completes(&prompt, &paths);
            cancelled_validation_sends_nothing_and_a_click_copies(&prompt, &paths);

            open_empty_folder(&fixture);
            for (key, message) in [(Key::M, "Nothing to move"), (Key::C, "Nothing to copy")] {
                shifted(&fixture, key);
                assert_eq!(feedback(&fixture), message);
                assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            }
        },
    );
}

/// Headless GVfs cannot list Trash; restore behavior is covered by the trash tests.
#[test]
fn tenxer_restore_key_refuses_items_outside_trash() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::send_to::tenxer_restore_key_refuses_items_outside_trash",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();

            fill_b_and_c(&fixture);
            shifted(&fixture, Key::R);
            assert_eq!(feedback(&fixture), "Only items in Trash can be restored");
            assert!(!modal_visible(&fixture.overlay));

            let trash = Location::uri("trash:///");
            browser.navigate(trash.clone());
            wait_until(|| browser.active_location() == Some(trash.clone()));
            wait_loaded(&browser, 0);
            focus_files(&fixture);
            shifted(&fixture, Key::R);
            assert_eq!(feedback(&fixture), "Nothing to restore");
            assert!(!modal_visible(&fixture.overlay));
        },
    );
}
