// SPDX-License-Identifier: MIT

use super::*;

struct Collision {
    _fixture: tempfile::TempDir,
    source_dir: PathBuf,
    destination: PathBuf,
}

fn collision(source_files: &[(&str, &[u8])], destination_files: &[(&str, &[u8])]) -> Collision {
    let fixture = tempfile::tempdir().expect("conflict fixture");
    let source_dir = fixture.path().join("source");
    let destination = fixture.path().join("destination");
    std::fs::create_dir_all(&source_dir).expect("source dir");
    std::fs::create_dir_all(&destination).expect("destination dir");
    write_files(&source_dir, source_files);
    write_files(&destination, destination_files);
    Collision {
        _fixture: fixture,
        source_dir,
        destination,
    }
}

impl Collision {
    fn paste(&self, view: &crate::ui::browser::BrowserView, names: &[&str], moving: bool) {
        view.state.start_transfer(
            Location::local(&self.destination),
            names
                .iter()
                .map(|name| Location::local(self.source_dir.join(name)))
                .collect(),
            moving,
        );
    }
}

fn assert_conflict_dialog(overlay: &gtk::Overlay) {
    assert!(
        wait_for_modal_layer(overlay),
        "conflict dialog modal did not appear"
    );
}

#[test]
fn same_folder_paste_creates_a_numbered_copy_without_a_dialog() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::conflicts::same_folder_paste_creates_a_numbered_copy_without_a_dialog",
        || {
            let fixture = tempfile::tempdir().expect("conflict fixture");
            let folder = fixture.path().join("folder");
            std::fs::create_dir_all(&folder).expect("folder");
            std::fs::write(folder.join("photo.jpg"), b"photo").expect("photo");
            let (view, overlay, window) = conflict_view();

            view.state.start_transfer(
                Location::local(&folder),
                vec![Location::local(folder.join("photo.jpg"))],
                false,
            );

            let source = folder.join("photo.jpg");
            let copy = folder.join("photo (1).jpg");
            wait_until(|| holds(&copy, b"photo"), "the numbered duplicate copy");
            assert!(
                !modal_layer_shown(&overlay),
                "same-folder duplicates must not open the conflict dialog"
            );
            assert_eq!(std::fs::read(&source).expect("original contents"), b"photo");
            window.destroy();
        },
    );
}

#[test]
fn conflict_dialog_offers_skip_for_a_multi_item_paste() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::conflicts::conflict_dialog_offers_skip_for_a_multi_item_paste",
        || {
            let collision = collision(
                &[("a.txt", b"new a"), ("b.txt", b"new b")],
                &[("a.txt", b"old a"), ("b.txt", b"old b")],
            );
            let (view, overlay, window) = conflict_view();

            collision.paste(&view, &["a.txt", "b.txt"], false);

            assert_conflict_dialog(&overlay);
            assert!(
                has_visible_button(&overlay, "Skip"),
                "skip must remain for multi-item pastes"
            );
            assert!(
                has_visible_button(&overlay, "Apply to All"),
                "apply to all must appear while further conflicts remain"
            );
            window.destroy();
        },
    );
}

#[test]
fn merging_a_folder_combines_contents_through_the_conflict_dialog() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::conflicts::merging_a_folder_combines_contents_through_the_conflict_dialog",
        || {
            let collision = collision(
                &[
                    ("folder/incoming.txt", b"new"),
                    ("folder/shared.txt", b"incoming wins"),
                ],
                &[
                    ("folder/stays.txt", b"keep me"),
                    ("folder/shared.txt", b"old"),
                ],
            );
            let source_folder = collision.source_dir.join("folder");
            let (view, overlay, window) = conflict_view();

            collision.paste(&view, &["folder"], false);

            assert_conflict_dialog(&overlay);
            assert!(
                has_visible_button(&overlay, "Merge"),
                "a folder collision must offer Merge"
            );
            assert!(
                !visible_texts(&overlay)
                    .iter()
                    .any(|text| text.contains("Merging removes the source folder")),
                "a copy merge keeps its source and its undo"
            );

            click_button(&overlay, "Merge");
            let merged = collision.destination.join("folder");
            // The overwritten original is staged in Trash before the copy, so
            // shared.txt is briefly absent: wait for both files' final state.
            wait_until(
                || {
                    holds(&merged.join("incoming.txt"), b"new")
                        && holds(&merged.join("shared.txt"), b"incoming wins")
                },
                "the merge to finish",
            );
            assert_eq!(
                std::fs::read(merged.join("shared.txt")).expect("shared file"),
                b"incoming wins",
                "the incoming item overwrites a same-named destination item"
            );
            assert_eq!(
                std::fs::read(merged.join("stays.txt")).expect("destination-only file"),
                b"keep me",
                "destination-only contents survive the merge"
            );
            assert!(source_folder.is_dir(), "a copy merge keeps the source");
            window.destroy();
        },
    );
}

#[test]
fn merge_is_only_offered_for_a_folder_onto_a_folder() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::conflicts::merge_is_only_offered_for_a_folder_onto_a_folder",
        || {
            let collision = collision(&[("file.txt", b"new")], &[("file.txt", b"old")]);
            std::fs::create_dir_all(collision.source_dir.join("folder")).expect("source folder");
            std::fs::create_dir_all(collision.destination.join("folder"))
                .expect("destination folder");
            let (view, overlay, window) = conflict_view();

            collision.paste(&view, &["file.txt"], false);
            assert_conflict_dialog(&overlay);
            assert!(
                !has_visible_button(&overlay, "Merge"),
                "a file collision cannot merge"
            );
            click_button(&overlay, "Cancel");
            wait_until(
                || !modal_layer_shown(&overlay),
                "the conflict dialog to dismiss",
            );

            collision.paste(&view, &["folder"], true);
            assert!(
                wait_for_modal_layer(&overlay),
                "move conflict dialog modal did not appear"
            );
            assert!(
                has_visible_button(&overlay, "Merge"),
                "a moved folder collision must offer Merge"
            );
            window.destroy();
        },
    );
}

#[test]
fn a_file_meeting_a_folder_is_never_offered_replace_even_through_apply_to_all() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::conflicts::a_file_meeting_a_folder_is_never_offered_replace_even_through_apply_to_all",
        || {
            let collision = collision(
                &[("b.txt", b"new b"), ("data", b"file")],
                &[("b.txt", b"old b"), ("data/kept.txt", b"kept")],
            );
            let destination = &collision.destination;
            let (view, overlay, window) = conflict_view();

            collision.paste(&view, &["b.txt", "data"], false);
            assert_conflict_dialog(&overlay);
            check_button_with_label(&overlay, "Apply to All").set_active(true);
            click_button(&overlay, "Replace");

            wait_until(
                || {
                    visible_texts(&overlay).iter().any(|text| text == "data")
                        && !has_visible_button(&overlay, "Replace")
                },
                "the file meeting a folder to keep its own prompt, without Replace",
            );
            assert!(
                has_visible_button(&overlay, "Keep Both"),
                "the file can still be kept beside the folder"
            );
            click_button(&overlay, "Skip");

            wait_until(
                || holds(&destination.join("b.txt"), b"new b"),
                "Apply to All to replace the file that met a file",
            );
            assert_eq!(
                std::fs::read(destination.join("data/kept.txt")).expect("folder contents"),
                b"kept",
                "the folder the file met stays"
            );
            window.destroy();
        },
    );
}

#[test]
fn skipping_the_only_collision_still_transfers_accepted_items() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::conflicts::skipping_the_only_collision_still_transfers_accepted_items",
        || {
            let collision = collision(
                &[
                    ("a.txt", b"new a"),
                    ("b.txt", b"new b"),
                    ("c.txt", b"new c"),
                    ("d.txt", b"new d"),
                ],
                &[("a.txt", b"old a")],
            );
            let (source_dir, destination) = (&collision.source_dir, &collision.destination);
            let (view, overlay, window) = conflict_view();

            collision.paste(&view, &["a.txt", "b.txt", "c.txt", "d.txt"], false);

            assert_conflict_dialog(&overlay);
            assert!(
                has_visible_button(&overlay, "Replace"),
                "the single conflicting item must still be resolvable"
            );
            assert_skip_offered_alone(
                &overlay,
                "skip must stay visible while other items are already accepted",
                "apply to all has nothing left to apply to",
            );

            click_button(&overlay, "Skip");
            for name in ["b.txt", "c.txt", "d.txt"] {
                let copied = destination.join(name);
                let expected = std::fs::read(source_dir.join(name)).expect("source contents");
                wait_until(
                    || holds(&copied, &expected),
                    "the non-conflicting transfer to finish",
                );
                assert_eq!(
                    std::fs::read(&copied).expect("copied contents"),
                    expected,
                    "the accepted items must still be pasted"
                );
            }
            assert_eq!(
                std::fs::read(destination.join("a.txt")).expect("colliding file"),
                b"old a",
                "skipping must leave the conflicting file alone"
            );
            assert!(
                !destination.join("a (1).txt").exists(),
                "skipping must not create a numbered copy"
            );
            window.destroy();
        },
    );
}

#[test]
fn skip_stays_visible_for_the_final_conflict_after_keep_both() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::conflicts::skip_stays_visible_for_the_final_conflict_after_keep_both",
        || {
            let collision = collision(
                &[("a.txt", b"new a"), ("b.txt", b"new b")],
                &[("a.txt", b"old a"), ("b.txt", b"old b")],
            );
            let destination = &collision.destination;
            let (view, overlay, window) = conflict_view();

            collision.paste(&view, &["a.txt", "b.txt"], false);

            assert_conflict_dialog(&overlay);
            click_button(&overlay, "Keep Both");

            wait_until(
                || {
                    visible_texts(&overlay).iter().any(|text| text == "b.txt")
                        && !has_visible_button(&overlay, "Apply to All")
                },
                "the final conflict dialog once the first dialog has dismissed",
            );
            assert_skip_offered_alone(
                &overlay,
                "skip must stay available for the final conflict after earlier work is accepted",
                "apply to all has no further conflicts left to apply to",
            );
            click_button(&overlay, "Skip");

            let kept_both = destination.join("a (1).txt");
            wait_until(|| holds(&kept_both, b"new a"), "the first Keep Both copy");
            assert_eq!(
                std::fs::read(&kept_both).expect("kept-both copy"),
                b"new a",
                "earlier Keep Both choices must be preserved"
            );
            assert_eq!(
                std::fs::read(destination.join("b.txt")).expect("colliding file"),
                b"old b",
                "skipping the final conflict must leave it alone"
            );
            assert!(
                !destination.join("b (1).txt").exists(),
                "skipping must not create a numbered copy"
            );
            window.destroy();
        },
    );
}

#[expect(
    deprecated,
    reason = "GTK 4.10 deprecated style_context lookup without a public replacement for reading a widget's resolved theme color"
)]
fn resolved_dialog_surface(overlay: &gtk::Overlay) -> String {
    let dialog = find_widget_with_class(overlay, "action-dialog").expect("conflict dialog");
    let color = dialog
        .style_context()
        .lookup_color("strata_surface")
        .unwrap_or_else(|| {
            panic!("the dialog style context must resolve the active theme surface")
        });
    color.to_string()
}

#[test]
fn conflict_dialog_verifies_theme_following() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::conflicts::conflict_dialog_verifies_theme_following",
        || {
            let themes = crate::ui::theme::ThemeManager::shared();
            themes.select_theme("tokyo-night");

            let collision = collision(&[("cast.txt", b"new")], &[("cast.txt", b"old")]);
            let (view, overlay, window) = conflict_view();

            collision.paste(&view, &["cast.txt"], false);
            assert_conflict_dialog(&overlay);

            let first = resolved_dialog_surface(&overlay);
            themes.select_theme("everforest-light-medium");
            iterate_context(3);
            let second = resolved_dialog_surface(&overlay);
            assert_ne!(
                first, second,
                "the conflict dialog must re-theme when the active theme changes"
            );
            window.destroy();
        },
    );
}

#[test]
fn a_conflict_prompt_says_which_name_the_item_takes_and_what_a_merge_overwrites() {
    let destination = Location::local("/media/stick");
    let explanation = |source: &str, target: &str, fixes_inside, merge| {
        conflict_explanation(
            &TransferCollision {
                source: Location::local(format!("/home/user/{source}")),
                target_name: Some(ShownName {
                    name: OsString::from(target),
                    cause: (source != target).then_some(RenameCause::Invalid),
                }),
                kinds: crate::adapters::TransferTargetCollision::between(true, true),
                fixes_inside,
            },
            &destination,
            merge,
        )
    };
    let saved_as = rust_i18n::t!(
        "“%{name}” will be saved as “%{new_name}” in this location.",
        name = "a:b",
        new_name = "a_b"
    );
    for merge in [false, true] {
        let as_named = explanation("a_b", "a_b", false, merge);
        assert!(!as_named.contains("a:b"), "{as_named}");
        assert_eq!(
            explanation("a:b", "a_b", false, merge),
            crate::i18n::join_sentences(&saved_as, &as_named)
        );
    }
    let replacing = explanation("folder", "folder", true, false);
    assert_eq!(replacing, explanation("folder", "folder", false, false));
    let merging = explanation("folder", "folder", false, true);
    assert_eq!(
        explanation("folder", "folder", true, true),
        crate::i18n::join_sentences(
            &merging,
            &crate::i18n::tr(
                "Items whose names change to suit this location overwrite items with the new name."
            )
        )
    );
}

struct FatPaste {
    sources: &'static [&'static str],
    files: &'static [(&'static str, &'static str)],
    disk: &'static [(&'static str, &'static str)],
    prompt: (&'static str, &'static str),
    choice: (&'static str, bool),
    holds: &'static [(&'static str, &'static str)],
}

fn fat_pastes() -> [FatPaste; 4] {
    [
        FatPaste {
            sources: &["a:b"],
            files: &[("a:b", "incoming")],
            disk: &[("a_b", "old")],
            prompt: ("a:b", "a_b"),
            choice: ("Replace", false),
            holds: &[("a_b", "incoming")],
        },
        FatPaste {
            sources: &["a:b", "c:d"],
            files: &[("a:b", "first"), ("c:d", "second")],
            disk: &[("a_b", "old"), ("c_d", "old")],
            prompt: ("a:b", "a_b"),
            choice: ("Replace", true),
            holds: &[("a_b", "first"), ("c_d", "second")],
        },
        FatPaste {
            sources: &["a:b", "a?b"],
            files: &[("a:b", "first"), ("a?b", "second")],
            disk: &[("a_b", "old")],
            prompt: ("a:b", "a_b"),
            choice: ("Skip", false),
            holds: &[("a_b", "old"), ("a_b (1)", "second")],
        },
        FatPaste {
            sources: &["x:y"],
            files: &[("x:y/new.txt", "new")],
            disk: &[("x_y/old.txt", "old")],
            prompt: ("x:y", "x_y"),
            choice: ("Merge", false),
            holds: &[("x_y/new.txt", "new"), ("x_y/old.txt", "old")],
        },
    ]
}

fn seed_fat_paste(root: &Path, row: &FatPaste) -> (PathBuf, PathBuf) {
    let source = root.join("source");
    let disk = root.join("disk");
    for (root, files) in [(&source, row.files), (&disk, row.disk)] {
        for (path, contents) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("fixture folder");
            std::fs::write(path, contents).expect("fixture file");
        }
    }
    (source, disk)
}

fn assert_prompt_names_the_planned_name(overlay: &gtk::Overlay, row: &FatPaste) {
    let (name, new_name) = row.prompt;
    assert!(
        visible_texts(overlay)
            .iter()
            .any(|text| text.contains(name) && text.contains(new_name)),
        "the prompt names the item and the name it takes: {:?}",
        visible_texts(overlay)
    );
}

fn answer_prompt(overlay: &gtk::Overlay, row: &FatPaste) {
    let (choice, apply_to_all) = row.choice;
    if apply_to_all {
        check_button_with_label(overlay, "Apply to All").set_active(true);
    }
    click_button(overlay, choice);
}

fn assert_only_planned_names_land(disk: &Path, row: &FatPaste) {
    let choice = row.choice.0;
    wait_until(
        || {
            row.holds
                .iter()
                .all(|(path, contents)| holds(&disk.join(path), contents.as_bytes()))
        },
        &format!("{choice} to give each item the name its prompt planned"),
    );
    let mut listed = std::fs::read_dir(disk)
        .expect("disk listing")
        .map(|entry| entry.expect("disk entry").file_name())
        .collect::<Vec<_>>();
    listed.sort();
    let mut expected = row
        .holds
        .iter()
        .map(|(path, _)| path.split('/').next().expect("top-level name").into())
        .collect::<Vec<std::ffi::OsString>>();
    expected.dedup();
    assert_eq!(listed, expected, "{choice}: nothing else lands beside them");
}

#[test]
fn a_paste_onto_fat_asks_about_fixed_names_and_uses_the_names_it_planned() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::conflicts::a_paste_onto_fat_asks_about_fixed_names_and_uses_the_names_it_planned",
        || {
            for row in fat_pastes() {
                let fixture = tempfile::tempdir().expect("conflict fixture");
                let (source, disk) = seed_fat_paste(fixture.path(), &row);
                let _disk =
                    crate::adapters::FsQuirks::install(&disk, crate::adapters::Quirks::vfat());
                let (view, overlay, window) = conflict_view();

                view.state.start_transfer(
                    Location::local(&disk),
                    row.sources
                        .iter()
                        .map(|name| Location::local(source.join(name)))
                        .collect(),
                    false,
                );

                assert_conflict_dialog(&overlay);
                assert_prompt_names_the_planned_name(&overlay, &row);
                answer_prompt(&overlay, &row);
                assert_only_planned_names_land(&disk, &row);
                window.destroy();
            }
        },
    );
}
