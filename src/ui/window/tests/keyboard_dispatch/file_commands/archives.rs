// SPDX-License-Identifier: MIT

use super::*;

fn seed_archive_fixture(fixture: &KeyboardFixture, directory: &Path) -> PathBuf {
    let source = tempfile::tempdir().expect("archive source");
    std::fs::write(source.path().join("notes.txt"), "notes").expect("archived file");
    crate::adapters::write_compression_fixture(
        &directory.join("bundle.tar"),
        &[source.path().join("notes.txt")],
        crate::services::ArchiveFormat::Tar,
        None,
    )
    .expect("fixture archive");
    let destination = directory.join("dest");
    std::fs::create_dir(&destination).expect("destination");
    std::fs::create_dir(directory.join("nested.zip")).expect("folder named like an archive");
    fixture.view.refresh();
    wait_until(|| {
        rendered_name(&fixture.view.widget(), "bundle.tar")
            && rendered_name(&fixture.view.widget(), "nested.zip")
    });
    destination
}

/// Runs `;` followed by `key`, which must complete the action chord.
fn action(fixture: &KeyboardFixture, key: Key, modifiers: ModifierType) {
    focus_files(fixture);
    plain(fixture, Key::semicolon);
    assert!(fixture.press(key, modifiers), "; {key:?}");
    assert_eq!(fixture.shortcuts.armed_chord(), None);
}

fn extract(fixture: &KeyboardFixture) {
    action(fixture, Key::e, ModifierType::empty());
}

fn extract_to(fixture: &KeyboardFixture) {
    action(fixture, Key::E, ModifierType::SHIFT_MASK);
}

fn non_archives_are_refused(fixture: &KeyboardFixture, destination: &Path) {
    let browser = fixture.view.browser();
    move_to_named(fixture, &browser, "a.txt");
    extract(fixture);
    assert_eq!(feedback(fixture), "Not an archive");
    action(fixture, Key::c, ModifierType::empty());
    wait_until(|| modal_visible(&fixture.overlay));
    close_modal(fixture);
    assert_eq!(directory_names(destination), Vec::<String>::new());

    move_to_named(fixture, &browser, "nested.zip");
    extract(fixture);
    assert_eq!(feedback(fixture), "Not an archive");
    extract_to(fixture);
    assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
    assert_eq!(feedback(fixture), "Not an archive");
}

fn extract_here_fills_the_result(fixture: &KeyboardFixture, directory: &Path) {
    let browser = fixture.view.browser();
    let origin = browser.active_location();
    move_to_named(fixture, &browser, "bundle.tar");
    extract(fixture);
    wait_until(|| std::fs::read_to_string(directory.join("notes.txt")).is_ok());
    assert_eq!(contents(&directory.join("notes.txt")), "notes");
    assert_eq!(browser.active_location(), origin);
    wait_until(|| fill_names(&browser) == ["notes.txt"]);
}

fn extract_to_a_picked_folder(fixture: &KeyboardFixture, destination: &Path) {
    let browser = fixture.view.browser();
    let origin = browser.active_location();
    browser.clear_active_selection();
    move_to_named(fixture, &browser, "bundle.tar");
    extract_to(fixture);
    assert_eq!(
        fixture.shortcuts.open_prompt_kind(),
        Some(Prompt::ExtractTo)
    );
    assert_eq!(
        fixture.shortcuts.prompt_label().as_deref(),
        Some("extract to \u{203a}")
    );
    fixture.shortcuts.prompt().set_text("missing");
    wait_until(|| fixture.shortcuts.prompt_hint().as_deref() == Some("No matching folders"));
    plain(fixture, Key::Return);
    assert_eq!(
        fixture.shortcuts.prompt_hint().as_deref(),
        Some("No matching folders")
    );
    fixture.shortcuts.prompt().set_text("dest");
    wait_until(|| fixture.shortcuts.candidates() == [destination.to_path_buf()]);
    plain(fixture, Key::Return);
    wait_until(|| std::fs::read_to_string(destination.join("notes.txt")).is_ok());
    assert_eq!(contents(&destination.join("notes.txt")), "notes");
    assert_eq!(browser.active_location(), origin, "extracting stays put");
    pump(500);
    assert!(
        fill_names(&browser).is_empty(),
        "the open folder's notes.txt is not the extracted item"
    );
}

#[test]
fn tenxer_action_chord_compresses_and_extracts_archives() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::archives::tenxer_action_chord_compresses_and_extracts_archives",
        || {
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let destination = seed_archive_fixture(&fixture, &directory);
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();

            non_archives_are_refused(&fixture, &destination);
            extract_here_fills_the_result(&fixture, &directory);
            extract_to_a_picked_folder(&fixture, &destination);

            for name in ["b.txt", "bundle.tar"] {
                move_to_named(&fixture, &browser, name);
                plain(&fixture, Key::space);
            }
            assert_eq!(fill_names(&browser), ["b.txt", "bundle.tar"]);
            extract(&fixture);
            assert_eq!(feedback(&fixture), "Extract one archive at a time");
            browser.clear_active_selection();

            open_empty_folder(&fixture);
            for (key, message) in [
                (Key::c, "Nothing to compress"),
                (Key::e, "Nothing to extract"),
            ] {
                action(&fixture, key, ModifierType::empty());
                assert_eq!(feedback(&fixture), message, "; {key:?}");
            }
        },
    );
}
