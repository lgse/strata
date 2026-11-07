// SPDX-License-Identifier: MIT

use super::*;

/// A custom action manifest whose command appends its id, `STRATA_ACTION_SOURCE` and
/// paths to a recorder file.
struct ActionManifest<'a> {
    id: &'a str,
    name: &'a str,
    menu: &'a str,
    when: &'a str,
    extra: &'a str,
}

impl<'a> ActionManifest<'a> {
    fn new(id: &'a str, name: &'a str, menu: &'a str, when: &'a str) -> Self {
        Self {
            id,
            name,
            menu,
            when,
            extra: "",
        }
    }

    fn with_extra(self, extra: &'a str) -> Self {
        Self { extra, ..self }
    }

    fn write(&self, output: &Path) {
        let Self {
            id,
            name,
            menu,
            when,
            extra,
        } = self;
        let directory = crate::storage::config_directory().join("actions").join(id);
        std::fs::create_dir_all(&directory).expect("action folder");
        std::fs::write(
            directory.join("action.toml"),
            format!(
                "schema_version = 1\nid = \"{id}\"\nname = \"{name}\"\nmenu = \"{menu}\"\n{extra}\n[when]\n{when}\n\n[run]\nruntime = \"command\"\nprogram = \"/bin/sh\"\nargs = [\"-c\", \"printf '%s\\\\n' \\\"{id} $STRATA_ACTION_SOURCE\\\" \\\"$@\\\" >> '{}'\", \"sh\", \"{{paths}}\"]\n",
                output.display()
            ),
        )
        .expect("action manifest");
    }
}

fn run_jobs(duration: Duration) {
    let deadline = std::time::Instant::now() + duration;
    while std::time::Instant::now() < deadline {
        crate::ui::jobs::shared().pump();
        pump(20);
    }
}

fn wait_invocations(output: &Path, lines: usize) -> Vec<String> {
    wait_until(|| {
        crate::ui::jobs::shared().pump();
        invocations(output).len() >= lines
    });
    invocations(output)
}

fn invocations(output: &Path) -> Vec<String> {
    std::fs::read_to_string(output)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn reload_actions() {
    gtk::glib::MainContext::default().block_on(crate::ui::actions::shared().reload());
}

fn manifest_path(id: &str) -> PathBuf {
    crate::storage::config_directory().join(format!("actions/{id}/action.toml"))
}

/// Rewrites the action `id` so it asks for confirmation before running.
fn require_confirmation(id: &str) {
    let manifest = manifest_path(id);
    let edited = std::fs::read_to_string(&manifest)
        .expect("manifest")
        .replace(
            "runtime = \"command\"",
            "runtime = \"command\"\nconfirm = true",
        );
    std::fs::write(&manifest, edited).expect("confirming action");
}

fn write_numbered_actions(output: &Path) {
    ActionManifest::new("top-b", "Top B", "top", "extensions = [\"txt\"]").write(output);
    ActionManifest::new("top-a", "Top A", "top", "extensions = [\"txt\"]").write(output);
    for index in 1..=9 {
        let (id, name) = (format!("sub-{index}"), format!("Sub {index}"));
        ActionManifest::new(&id, &name, "submenu", "extensions = [\"txt\"]").write(output);
    }
    ActionManifest::new("disabled", "Aaa disabled", "top", "extensions = [\"txt\"]")
        .with_extra("enabled = false")
        .write(output);
    ActionManifest::new("images", "Aaa images", "top", "extensions = [\"png\"]").write(output);
    ActionManifest::new(
        "confirming",
        "Aaa confirm",
        "submenu",
        "extensions = [\"md\"]",
    )
    .write(output);
    require_confirmation("confirming");
    reload_actions();
}

fn path_text(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

fn chord_rows(fixture: &KeyboardFixture) -> Vec<(String, String)> {
    wait_until(|| fixture.shortcuts.chord_options().is_some());
    fixture.shortcuts.chord_options().expect("options")
}

/// The chord lists top-level actions first, omits disabled and non-matching ones
/// and drops the eleventh; a digit runs its listed action on the cursor.
fn digits_run_the_listed_action(fixture: &KeyboardFixture, directory: &Path, output: &Path) {
    move_to_named(fixture, &fixture.view.browser(), "a.txt");
    plain(fixture, Key::semicolon);
    assert_eq!(fixture.shortcuts.armed_chord(), Some(Chord::Action));
    assert_eq!(fixture.shortcuts.chord().text(), ";-");
    let mut listed = vec![("1", "Top A"), ("2", "Top B")];
    let subs: Vec<String> = (1..=8).map(|index| format!("Sub {index}")).collect();
    let keys = ["3", "4", "5", "6", "7", "8", "9", "0"];
    listed.extend(keys.iter().copied().zip(subs.iter().map(String::as_str)));
    listed.extend(LETTERED_ACTIONS);
    assert_eq!(
        chord_rows(fixture),
        owned_rows(&listed),
        "top-level first, disabled and non-matching omitted, eleventh dropped"
    );
    plain(fixture, Key::_1);
    assert_eq!(
        wait_invocations(output, 2),
        [
            "top-a selection".to_owned(),
            path_text(directory.join("a.txt"))
        ]
    );
    std::fs::remove_file(output).expect("reset recorder");
}

fn a_digit_runs_on_the_fill(fixture: &KeyboardFixture, directory: &Path, output: &Path) {
    focus_files(fixture);
    move_to_named(fixture, &fixture.view.browser(), "a.txt");
    plain(fixture, Key::space);
    plain(fixture, Key::space);
    plain(fixture, Key::semicolon);
    plain(fixture, Key::_0);
    assert_eq!(
        wait_invocations(output, 3),
        [
            "sub-8 selection".to_owned(),
            path_text(directory.join("a.txt")),
            path_text(directory.join("b.txt")),
        ],
        "the filled selection"
    );
    std::fs::remove_file(output).expect("reset recorder");
}

fn unlisted_keys_report_their_slot(fixture: &KeyboardFixture) {
    let browser = fixture.view.browser();
    focus_files(fixture);
    browser.clear_active_selection();
    move_to_named(fixture, &browser, "notes.md");
    plain(fixture, Key::semicolon);
    assert_eq!(
        chord_rows(fixture),
        owned_rows(&[&[("1", "Aaa confirm")], LETTERED_ACTIONS].concat())
    );
    plain(fixture, Key::_2);
    assert_eq!(feedback(fixture), "No action 2");
    plain(fixture, Key::semicolon);
    plain(fixture, Key::Escape);
    assert_eq!(fixture.shortcuts.armed_chord(), None);
    plain(fixture, Key::semicolon);
    plain(fixture, Key::x);
    assert_eq!(feedback(fixture), "Unknown chord");
}

fn a_cancelled_confirmation_runs_nothing(fixture: &KeyboardFixture, output: &Path) {
    plain(fixture, Key::semicolon);
    plain(fixture, Key::_1);
    wait_until(|| modal_visible(&fixture.overlay));
    assert!(click_class(&fixture.overlay, "action-dialog-close"));
    wait_until(|| !modal_visible(&fixture.overlay));
    run_jobs(Duration::from_millis(300));
    assert!(
        invocations(output).is_empty(),
        "a cancelled confirmation runs nothing"
    );
    wait_until(|| fixture.view.item_view_has_focus());
}

fn stale_targets_run_nothing(fixture: &KeyboardFixture, output: &Path) {
    let browser = fixture.view.browser();
    focus_files(fixture);
    move_to_named(fixture, &browser, "notes.md");
    plain(fixture, Key::semicolon);
    browser.select(0, 0);
    plain(fixture, Key::_1);
    assert_eq!(feedback(fixture), "Selection changed");
    run_jobs(Duration::from_millis(300));
    assert!(invocations(output).is_empty(), "stale targets run nothing");
    assert!(!modal_visible(&fixture.overlay));
}

/// Actions added, or edited outside Strata, after the chord was armed do not run.
fn changed_actions_run_nothing(fixture: &KeyboardFixture) {
    let browser = fixture.view.browser();
    let unlisted = fixture._history_directory.path().join("unlisted");
    browser.clear_active_selection();
    move_to_named(fixture, &browser, "notes.md");
    plain(fixture, Key::semicolon);
    ActionManifest::new("aa-new", "Aa new", "submenu", "extensions = [\"md\"]").write(&unlisted);
    reload_actions();
    plain(fixture, Key::_1);
    assert_eq!(feedback(fixture), "Actions changed");
    run_jobs(Duration::from_millis(300));
    assert!(!unlisted.exists() && !modal_visible(&fixture.overlay));

    let held = crate::storage::writer::hold();
    require_confirmation("aa-new");
    plain(fixture, Key::semicolon);
    plain(fixture, Key::_1);
    run_jobs(Duration::from_millis(300));
    drop(held);
    crate::storage::writer::settle();
    wait_until(|| modal_visible(&fixture.overlay));
    run_jobs(Duration::from_millis(300));
    assert!(
        !unlisted.exists(),
        "an action edited since the chord was armed does not run"
    );
    assert!(
        label_texts(fixture.overlay.upcast_ref())
            .iter()
            .any(|text| text == "Actions changed")
    );
    assert!(click_class(&fixture.overlay, "action-dialog-close"));
    wait_until(|| !modal_visible(&fixture.overlay));
}

#[test]
fn tenxer_numbered_actions_run_the_listed_match_on_the_current_targets() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::actions::tenxer_numbered_actions_run_the_listed_match_on_the_current_targets",
        || {
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let output = fixture._history_directory.path().join("invocations");
            write_numbered_actions(&output);
            std::fs::write(directory.join("notes.md"), "notes").expect("markdown");
            enable_tenxer(&fixture);
            refresh(&fixture, "notes.md");

            digits_run_the_listed_action(&fixture, &directory, &output);
            a_digit_runs_on_the_fill(&fixture, &directory, &output);
            unlisted_keys_report_their_slot(&fixture);
            a_cancelled_confirmation_runs_nothing(&fixture, &output);
            stale_targets_run_nothing(&fixture, &output);
            changed_actions_run_nothing(&fixture);

            open_empty_folder(&fixture);
            plain(&fixture, Key::semicolon);
            assert_eq!(
                chord_rows(&fixture),
                owned_rows(
                    &[
                        &[("1\u{2013}9 / 0", "No matching actions")],
                        LETTERED_ACTIONS
                    ]
                    .concat()
                )
            );
            plain(&fixture, Key::_1);
            assert_eq!(feedback(&fixture), "No action 1");
        },
    );
}

#[test]
fn tenxer_folder_actions_run_on_the_focused_folder_not_the_cursor_item() {
    use crate::ui::keys::{Binding, Keymap, Map, bindings::ACTIONS_FOLDER, use_keymap};
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::actions::tenxer_folder_actions_run_on_the_focused_folder_not_the_cursor_item",
        || {
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let output = fixture._history_directory.path().join("invocations");
            ActionManifest::new("folders", "Folders", "submenu", "kinds = [\"folder\"]")
                .write(&output);
            ActionManifest::new("texts", "Texts", "top", "extensions = [\"txt\"]").write(&output);
            reload_actions();
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();

            move_to_named(&fixture, &browser, "a.txt");
            plain(&fixture, Key::semicolon);
            plain(&fixture, Key::period);
            assert_eq!(fixture.shortcuts.armed_chord(), Some(Chord::FolderAction));
            assert_eq!(fixture.shortcuts.chord().text(), "; .-");
            assert_eq!(
                chord_rows(&fixture),
                owned_rows(&[("1", "Folders")]),
                "the folder's matches, not the cursor file's"
            );
            plain(&fixture, Key::_1);
            assert_eq!(
                wait_invocations(&output, 2),
                [
                    "folders background".to_owned(),
                    path_text(directory.clone())
                ]
            );
            std::fs::remove_file(&output).expect("reset recorder");

            focus_files(&fixture);
            plain(&fixture, Key::semicolon);
            plain(&fixture, Key::period);
            plain(&fixture, Key::_2);
            assert_eq!(feedback(&fixture), "No action 2");

            plain(&fixture, Key::semicolon);
            plain(&fixture, Key::period);
            open_empty_folder(&fixture);
            plain(&fixture, Key::_1);
            assert_eq!(feedback(&fixture), "Folder changed");
            run_jobs(Duration::from_millis(300));
            assert!(
                invocations(&output).is_empty(),
                "a folder left since the chord was armed runs nothing"
            );

            plain(&fixture, Key::semicolon);
            plain(&fixture, Key::period);
            plain(&fixture, Key::_1);
            assert_eq!(
                wait_invocations(&output, 2),
                [
                    "folders background".to_owned(),
                    path_text(directory.join("empty")),
                ],
                "an empty folder still offers its own actions"
            );
            std::fs::remove_file(&output).expect("reset recorder");

            let _keymap = use_keymap(Keymap::defaults().with(
                Map::Tenxer,
                ACTIONS_FOLDER,
                vec![Binding::plain(Key::f)],
            ));
            plain(&fixture, Key::semicolon);
            assert!(
                chord_rows(&fixture)
                    .contains(&("f".to_owned(), "Actions for this folder\u{2026}".to_owned())),
                "the panel shows the rebound key"
            );
            plain(&fixture, Key::f);
            assert_eq!(fixture.shortcuts.armed_chord(), Some(Chord::FolderAction));
            assert_eq!(fixture.shortcuts.chord().text(), "; f-");
            plain(&fixture, Key::_1);
            assert_eq!(
                wait_invocations(&output, 2),
                [
                    "folders background".to_owned(),
                    path_text(directory.join("empty")),
                ]
            );
        },
    );
}

/// Slot N is the Nth action Settings → Actions lists, disabled ones included,
/// so a slot never runs another action in its place.
#[test]
fn default_map_custom_action_slots_run_their_listed_action() {
    use crate::ui::keys::{Binding, CUSTOM_ACTIONS, Keymap, Map, use_keymap};
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::actions::default_map_custom_action_slots_run_their_listed_action",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            fixture.shortcuts.bind_preferences(&preferences);
            preferences.set_type_to_search(false);
            let directory = fixture._directory.path().to_path_buf();
            let output = fixture._history_directory.path().join("invocations");
            ActionManifest::new("a-off", "Off", "top", "extensions = [\"txt\"]")
                .with_extra("enabled = false")
                .write(&output);
            ActionManifest::new("b-images", "Images", "top", "extensions = [\"png\"]")
                .write(&output);
            ActionManifest::new("c-text", "Text", "submenu", "extensions = [\"txt\"]")
                .write(&output);
            reload_actions();
            let key = Binding::ctrl_alt(Key::e);
            let press_slot = |number: usize| {
                let _keymap = use_keymap(Keymap::defaults().with(
                    Map::Default,
                    CUSTOM_ACTIONS[number - 1],
                    vec![key],
                ));
                select_named(&fixture, "a.txt");
                assert!(fixture.press(Key::e, ModifierType::CONTROL_MASK | ModifierType::ALT_MASK));
            };

            press_slot(3);
            assert_eq!(
                wait_invocations(&output, 2),
                [
                    "c-text selection".to_owned(),
                    path_text(directory.join("a.txt"))
                ]
            );
            std::fs::remove_file(&output).expect("reset recorder");

            for (number, message) in [
                (1, "No matching actions"),
                (2, "No matching actions"),
                (4, "No action 4"),
            ] {
                press_slot(number);
                assert_eq!(feedback(&fixture), message, "slot {number}");
            }
            run_jobs(Duration::from_millis(300));
            assert!(
                invocations(&output).is_empty(),
                "no other action runs instead"
            );
        },
    );
}

const LETTERED_ACTIONS: &[(&str, &str)] = &[
    (".", "Actions for this folder\u{2026}"),
    ("t", "Open terminal here"),
    ("c", "Compress\u{2026}"),
    ("e", "Extract here"),
    ("E", "Extract to\u{2026}"),
];

fn owned_rows(rows: &[(&str, &str)]) -> Vec<(String, String)> {
    rows.iter()
        .map(|(key, name)| ((*key).to_owned(), (*name).to_owned()))
        .collect()
}

#[test]
fn tenxer_action_chord_steps_refuse_a_non_local_folder() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::actions::tenxer_action_chord_steps_refuse_a_non_local_folder",
        || {
            let fixture = KeyboardFixture::new();
            let output = fixture._history_directory.path().join("invocations");
            ActionManifest::new("folders", "Folders", "top", "kinds = [\"folder\"]").write(&output);
            reload_actions();
            // A named folder: the Trash root has no file name to match against.
            let trashed = gtk::glib::home_dir().join("trashed-folder");
            std::fs::create_dir(&trashed).expect("folder to trash");
            gtk::gio::File::for_path(&trashed)
                .trash(gtk::gio::Cancellable::NONE)
                .expect("trash the folder");
            let folder = Location::uri("trash:///trashed-folder");
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            browser.navigate(folder.clone());
            wait_until(|| browser.active_location().as_ref() == Some(&folder));
            wait_loaded(&browser, 0);
            focus_files(&fixture);

            plain(&fixture, Key::semicolon);
            plain(&fixture, Key::t);
            assert_eq!(feedback(&fixture), "Can\u{2019}t open a terminal here");
            assert!(!modal_visible(&fixture.overlay), "no error dialog");
            assert_eq!(fixture.shortcuts.armed_chord(), None);

            plain(&fixture, Key::semicolon);
            plain(&fixture, Key::period);
            assert_eq!(fixture.shortcuts.armed_chord(), Some(Chord::FolderAction));
            assert_eq!(
                chord_rows(&fixture),
                owned_rows(&[("1\u{2013}9 / 0", "No matching actions")]),
                "a folder without a native path offers no actions"
            );
            plain(&fixture, Key::_1);
            assert_eq!(feedback(&fixture), "No action 1");
            assert!(!modal_visible(&fixture.overlay), "no error dialog");
            run_jobs(Duration::from_millis(300));
            assert!(
                invocations(&output).is_empty(),
                "a folder action never runs on a non-local folder"
            );
        },
    );
}
