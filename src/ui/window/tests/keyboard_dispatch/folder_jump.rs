// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use super::*;
use crate::ui::tenxer_mode::Prompt;

const DAY: u64 = 24 * 60 * 60;

struct Places {
    _root: tempfile::TempDir,
    frequent: PathBuf,
    exact: PathBuf,
    latest: PathBuf,
}

fn seed_history(fixture: &KeyboardFixture) -> Places {
    let root = tempfile::tempdir().expect("places");
    let folder = |name: &str| {
        let path = root.path().join(name);
        std::fs::create_dir(&path).expect("place folder");
        path
    };
    let places = Places {
        frequent: folder("report-archive"),
        exact: folder("report"),
        latest: folder("scratch"),
        _root: root,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    for _ in 0..30 {
        fixture.history.record_at(&places.frequent, now - 2 * DAY);
    }
    fixture.history.record_at(&places.exact, now - 21 * DAY);
    fixture.history.record_at(&places.latest, now - 60);
    fixture.history.record_at(fixture._directory.path(), now);
    places
}

fn enable_tenxer(fixture: &KeyboardFixture) {
    let preferences = PreferenceManager::shared();
    fixture.shortcuts.bind_preferences(&preferences);
    preferences.set_tenxer_mode(true);
    pump(50);
}

fn open(fixture: &KeyboardFixture, key: Key) {
    focus_files(fixture);
    let modifiers = if key == Key::Z {
        ModifierType::SHIFT_MASK
    } else {
        ModifierType::empty()
    };
    assert!(fixture.press(key, modifiers));
    assert!(fixture.shortcuts.prompt_has_focus());
}

fn press(fixture: &KeyboardFixture, key: Key) {
    assert!(fixture.press(key, ModifierType::empty()));
}

fn assert_closed_without_candidates(fixture: &KeyboardFixture) {
    assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
    assert!(fixture.shortcuts.candidates().is_empty());
    assert!(!fixture.shortcuts.candidates_shown());
}

#[test]
fn tenxer_z_ranks_history_and_opens_the_chosen_folder_once() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::folder_jump::tenxer_z_ranks_history_and_opens_the_chosen_folder_once",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let places = seed_history(&fixture);
            let browser = fixture.view.browser();
            let origin = browser.active_location();

            open(&fixture, Key::Z);
            assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Recent));
            assert_eq!(
                fixture.shortcuts.candidates(),
                vec![
                    places.latest.clone(),
                    places.frequent.clone(),
                    places.exact.clone()
                ],
                "last visit first; the open folder is left out"
            );
            fixture.shortcuts.prompt().set_text("report");
            assert_eq!(
                fixture.shortcuts.candidates(),
                vec![places.frequent.clone(), places.exact.clone()]
            );
            press(&fixture, Key::Escape);
            assert_closed_without_candidates(&fixture);

            open(&fixture, Key::z);
            assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Jump));
            assert_eq!(
                fixture.shortcuts.prompt_label().as_deref(),
                Some("jump \u{203a}")
            );
            assert_eq!(
                fixture.shortcuts.candidates(),
                vec![
                    places.frequent.clone(),
                    places.latest.clone(),
                    places.exact.clone()
                ],
                "empty input lists candidates by frecency"
            );
            assert!(fixture.shortcuts.candidates_shown());
            assert!(
                !fixture.shortcuts.candidate_keys().contains("Tab"),
                "Tab does nothing in jump"
            );
            assert_eq!(fixture.shortcuts.prompt_hint().as_deref(), Some("1 of 3"));

            press(&fixture, Key::Down);
            assert_eq!(
                fixture.shortcuts.chosen_candidate().as_ref(),
                Some(&places.latest)
            );
            press(&fixture, Key::Up);
            press(&fixture, Key::Up);
            assert_eq!(
                fixture.shortcuts.chosen_candidate().as_ref(),
                Some(&places.exact),
                "Up wraps to the last candidate"
            );
            assert_eq!(fixture.shortcuts.prompt_hint().as_deref(), Some("3 of 3"));
            assert!(fixture.shortcuts.prompt_has_focus());

            fixture.shortcuts.prompt().set_text("report");
            assert_eq!(
                fixture.shortcuts.candidates(),
                vec![places.exact.clone(), places.frequent.clone()],
                "a text match outranks frecency"
            );
            assert_eq!(
                fixture.shortcuts.chosen_candidate().as_ref(),
                Some(&places.exact),
                "a new query does not keep the earlier choice"
            );

            fixture.shortcuts.prompt().set_text("qqqq");
            assert!(fixture.shortcuts.candidates().is_empty());
            assert!(!fixture.shortcuts.candidates_shown());
            assert_eq!(
                fixture.shortcuts.prompt_hint().as_deref(),
                Some("No matching folders")
            );
            press(&fixture, Key::Return);
            assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Jump));
            pump(50);
            assert_eq!(browser.active_location(), origin, "a miss never navigates");

            fixture.shortcuts.prompt().set_text("report");
            press(&fixture, Key::Down);
            press(&fixture, Key::Return);
            assert_closed_without_candidates(&fixture);
            wait_until(|| browser.active_location() == Some(Location::local(&places.frequent)));
            wait_loaded(&browser, 0);
            wait_until(|| fixture.view.item_view_has_focus());
            let generation = browser.navigation_generation();
            pump(100);
            assert_eq!(
                browser.navigation_generation(),
                generation,
                "Enter navigates once"
            );
        },
    );
}

#[test]
fn tenxer_history_prompts_end_without_opening_the_chosen_folder() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::folder_jump::tenxer_history_prompts_end_without_opening_the_chosen_folder",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let places = seed_history(&fixture);
            let browser = fixture.view.browser();
            let origin = browser.active_location();

            open(&fixture, Key::z);
            press(&fixture, Key::Down);
            move_to_named(&fixture, &browser, "a.txt");
            assert!(press_file_row(&fixture.view.widget(), "b.txt"));
            wait_until(|| fixture.shortcuts.open_prompt_kind().is_none());
            assert_closed_without_candidates(&fixture);
            assert_eq!(focused_name(&browser), "b.txt");

            open(&fixture, Key::Z);
            fixture.shortcuts.open_prompt(Prompt::Find);
            assert!(
                fixture.shortcuts.candidates().is_empty(),
                "a replacing prompt drops the candidates"
            );
            press(&fixture, Key::Escape);

            open(&fixture, Key::z);
            press(&fixture, Key::Down);
            PreferenceManager::shared().set_tenxer_mode(false);
            pump(50);
            assert_closed_without_candidates(&fixture);
            pump(50);
            assert_eq!(browser.active_location(), origin);

            PreferenceManager::shared().set_tenxer_mode(true);
            pump(50);
            open(&fixture, Key::z);
            fixture.shortcuts.click_candidate(2);
            assert_closed_without_candidates(&fixture);
            wait_until(|| browser.active_location() == Some(Location::local(&places.exact)));
            wait_loaded(&browser, 0);
            let generation = browser.navigation_generation();
            pump(100);
            assert_eq!(browser.navigation_generation(), generation);
        },
    );
}
