// SPDX-License-Identifier: MIT

use super::*;

type ItemCustomization = (
    &'static str,
    Option<crate::model::FolderColor>,
    Option<&'static str>,
);

fn customize_items(root: &std::path::Path, items: &[ItemCustomization]) {
    let manager = PreferenceManager::shared();
    for (relative, color, icon) in items {
        let path = root.join(relative);
        std::fs::create_dir_all(&path).expect("customized folder");
        manager.set_folder_color(&path, color.map(crate::model::FolderColorValue::Preset));
        manager.set_custom_icon(&path, *icon);
    }
}

fn assert_item_customizations(root: &std::path::Path, expected: &[ItemCustomization], case: &str) {
    let manager = PreferenceManager::shared();
    crate::storage::writer::settle();
    let saved = std::fs::read_to_string(settings_file()).unwrap_or_default();
    for (relative, color, icon) in expected {
        let path = root.join(relative);
        assert_eq!(
            (
                manager.folder_color(&path),
                manager.custom_icon(&path).as_deref(),
                saved.contains(&format!("\"{}\"", path.display()))
            ),
            (
                color.map(crate::model::FolderColorValue::Preset),
                *icon,
                color.is_some() || icon.is_some()
            ),
            "{case}: color, icon and saved key of {relative}"
        );
    }
}

/// Deletions dock as background operations as soon as they start.
fn wait_for_operation(open: &OpenWindow) {
    let browser = open.content.browser.browser();
    let request = browser.last_started_operation().expect("operation started");
    wait_until(|| !browser.is_current_operation(request) && !browser.has_background_operations());
    // Customization saves coalesce on idle.
    settle();
}

fn rendered_folder(path: &std::path::Path) -> Vec<u8> {
    let image = gtk::Image::new();
    crate::ui::thumbnail::show_customized_icon_image(
        &image,
        path,
        crate::assets::icons::FOLDER,
        24,
    );
    let texture = image
        .paintable()
        .and_downcast::<gtk::gdk::Texture>()
        .expect("folder icon texture");
    texture_pixels(&texture)
}

#[test]
fn renamed_folder_keeps_its_customization_and_a_new_folder_at_the_old_path_does_not() {
    gtk_test(
        "ui::window::tests::preferences::item_customizations::renamed_folder_keeps_its_customization_and_a_new_folder_at_the_old_path_does_not",
        || {
            use crate::assets::icons::FILE_CODE;
            use crate::model::FolderColor::Red;
            use crate::model::{FolderSort, Location, SortDirection, SortKey};

            let first = OpenWindow::open();
            let second = OpenWindow::open();
            let directory = load_folder(&first);
            let old = directory.path().join("child");
            let renamed = directory.path().join("renamed");
            let plain = rendered_folder(&old);
            customize_items(directory.path(), &[("child", Some(Red), Some(FILE_CODE))]);
            let manager = PreferenceManager::shared();
            let folder_sort =
                |path: &std::path::Path| manager.resolve_folder_sort(&Location::local(path), false);
            let size_sort = FolderSort::Saved(
                SortKey::Size,
                SortDirection::Descending,
                crate::model::NameComparison::default(),
            );
            manager.set_folder_sort(
                &Location::local(&old),
                SortKey::Size,
                SortDirection::Descending,
            );
            let customized = rendered_folder(&old);
            assert_ne!(customized, plain);

            first
                .content
                .browser
                .browser()
                .rename(loaded_entry(&first, &old), "renamed".into());
            wait_for_operation(&first);
            assert_item_customizations(
                directory.path(),
                &[
                    ("renamed", Some(Red), Some(FILE_CODE)),
                    ("child", None, None),
                ],
                "after the rename",
            );
            assert_eq!(rendered_folder(&renamed), customized);
            assert_eq!(folder_sort(&renamed), size_sort);
            assert_eq!(folder_sort(&old), FolderSort::Default);

            std::fs::create_dir(&old).expect("new folder at the old path");
            loaded_entry(&first, &old);
            assert_eq!(rendered_folder(&old), plain);

            first
                .content
                .browser
                .browser()
                .delete(vec![loaded_entry(&first, &renamed)], false);
            wait_for_operation(&first);
            assert_item_customizations(
                directory.path(),
                &[("renamed", None, None)],
                "after the trash",
            );
            assert_eq!(folder_sort(&renamed), FolderSort::Default);

            // Tombstones are shared: another window's undo restores them.
            second
                .content
                .browser
                .navigate_location(crate::model::Location::local(directory.path()));
            wait_until(|| !column_loading(&second, 0));
            assert!(second.content.browser.undo_last_operation());
            wait_for_operation(&second);
            assert!(renamed.is_dir());
            assert_item_customizations(
                directory.path(),
                &[("renamed", Some(Red), Some(FILE_CODE))],
                "after another window undoes the trash",
            );
        },
    );
}

#[derive(Clone, Copy)]
enum ItemOperation {
    MakeFolder(&'static str),
    MakeFile(&'static str),
    Customize(ItemCustomization),
    Rename,
    Move(crate::services::TransferConflict),
    Copy(crate::services::TransferConflict),
    Trash,
    /// Restores the n-th item this case trashed to its original path.
    PutBack(usize),
    PermanentDelete,
    Undo,
    Redo,
}

/// The conflict cases, each meeting a customized destination.
mod conflict_steps {
    use super::ItemCustomization;
    use super::ItemOperation::{self, *};
    use crate::assets::icons::{FILE_CODE, PICTURES};
    use crate::model::FolderColor::{Blue, Green, Purple, Red};
    use crate::services::TransferConflict;

    type Step = (ItemOperation, &'static [ItemCustomization]);

    const EXISTING: [Step; 2] = [
        (Customize(("dest/docs", Some(Green), Some(PICTURES))), &[]),
        (Customize(("dest/docs/kept", Some(Purple), None)), &[]),
    ];
    pub(super) const REPLACE: &[Step] = &[
        EXISTING[0],
        EXISTING[1],
        (
            Move(TransferConflict::ReplaceExisting),
            &[
                ("dest/docs", Some(Red), Some(FILE_CODE)),
                ("dest/docs/inner", Some(Blue), None),
                ("dest/docs/kept", None, None),
                ("docs", None, None),
            ],
        ),
        (
            Undo,
            &[
                ("docs", Some(Red), Some(FILE_CODE)),
                ("docs/inner", Some(Blue), None),
                ("dest/docs", Some(Green), Some(PICTURES)),
                ("dest/docs/kept", Some(Purple), None),
                ("dest/docs/inner", None, None),
            ],
        ),
    ];
    pub(super) const MERGE: &[Step] = &[
        EXISTING[0],
        EXISTING[1],
        (MakeFile("docs/note.txt"), &[]),
        (MakeFile("dest/docs/note.txt"), &[]),
        (Customize(("dest/docs/note.txt", None, Some(PICTURES))), &[]),
        (
            Move(TransferConflict::Merge),
            &[
                ("dest/docs", Some(Green), Some(PICTURES)),
                ("dest/docs/inner", Some(Blue), None),
                ("dest/docs/kept", Some(Purple), None),
                ("dest/docs/note.txt", None, None),
                ("docs", None, None),
                ("docs/inner", None, None),
            ],
        ),
    ];
    pub(super) const COPY_REPLACE: &[Step] = &[
        EXISTING[0],
        EXISTING[1],
        (
            Copy(TransferConflict::ReplaceExisting),
            &[
                ("dest/docs", None, None),
                ("dest/docs/inner", None, None),
                ("dest/docs/kept", None, None),
                ("docs", Some(Red), Some(FILE_CODE)),
            ],
        ),
        (
            Undo,
            &[
                ("dest/docs", Some(Green), Some(PICTURES)),
                ("dest/docs/kept", Some(Purple), None),
            ],
        ),
    ];
}

#[test]
fn folder_customizations_follow_file_operations_performed_in_strata() {
    gtk_test(
        "ui::window::tests::preferences::item_customizations::folder_customizations_follow_file_operations_performed_in_strata",
        || {
            use crate::assets::icons::FILE_CODE;
            use crate::model::FolderColor::{Blue, Green, Red};
            use crate::model::Location;
            use crate::services::{PasteItem, RestoreTrashItem, TransferConflict};
            use ItemOperation::*;

            let source = [
                ("docs", Some(Red), Some(FILE_CODE)),
                ("docs/inner", Some(Blue), None),
            ];
            let at_source = [
                ("docs", Some(Red), Some(FILE_CODE)),
                ("docs/inner", Some(Blue), None),
                ("docs2", None, None),
                ("dest/docs", None, None),
            ];
            let gone_from_source = [("docs", None, None), ("docs/inner", None, None)];
            let plain_docs = [("docs", None, None)];
            let green_docs = [("docs", Some(Green), None)];
            let renamed = [
                ("docs2", Some(Red), Some(FILE_CODE)),
                ("docs2/inner", Some(Blue), None),
                ("docs", None, None),
            ];
            let moved = [
                ("dest/docs", Some(Red), Some(FILE_CODE)),
                ("dest/docs/inner", Some(Blue), None),
                ("docs", None, None),
            ];
            type Steps<'a> = &'a [(ItemOperation, &'a [ItemCustomization])];
            let cases: [(&str, Steps); 11] = [
                (
                    "rename, undo and redo",
                    &[(Rename, &renamed), (Undo, &at_source), (Redo, &renamed)],
                ),
                (
                    "move, undo and redo",
                    &[
                        (Move(TransferConflict::FailIfExists), &moved),
                        (Undo, &at_source),
                        (Redo, &moved),
                    ],
                ),
                (
                    "Keep both lands under a new name",
                    &[
                        (MakeFolder("dest/docs"), &[]),
                        (
                            Move(TransferConflict::KeepBoth),
                            &[
                                ("dest/docs (1)", Some(Red), Some(FILE_CODE)),
                                ("dest/docs (1)/inner", Some(Blue), None),
                                ("dest/docs", None, None),
                                ("docs", None, None),
                            ],
                        ),
                    ],
                ),
                (
                    "Replace and undo restore each folder's customizations",
                    conflict_steps::REPLACE,
                ),
                (
                    "Merge keeps the destination folder's own customization",
                    conflict_steps::MERGE,
                ),
                (
                    "a copy that replaces drops the replaced folder's customizations until undo",
                    conflict_steps::COPY_REPLACE,
                ),
                (
                    "trash and undo",
                    &[(Trash, &gone_from_source), (Undo, &at_source)],
                ),
                (
                    "trash and Put back",
                    &[(Trash, &gone_from_source), (PutBack(0), &at_source)],
                ),
                (
                    "a plain folder trashed from the same path stays plain",
                    &[
                        (Trash, &gone_from_source),
                        (MakeFolder("docs"), &plain_docs),
                        (Trash, &plain_docs),
                        (PutBack(1), &plain_docs),
                        (PermanentDelete, &plain_docs),
                        (PutBack(0), &at_source),
                    ],
                ),
                (
                    "two trashed folders from one path keep their own customizations",
                    &[
                        (Trash, &gone_from_source),
                        (MakeFolder("docs"), &plain_docs),
                        (Customize(("docs", Some(Green), None)), &green_docs),
                        (Trash, &plain_docs),
                        (PutBack(0), &at_source),
                        (PermanentDelete, &plain_docs),
                        (PutBack(1), &green_docs),
                    ],
                ),
                ("permanent delete", &[(PermanentDelete, &gone_from_source)]),
            ];

            let open = OpenWindow::open();
            let trash_files = gtk::glib::user_data_dir().join("Trash/files");
            let trash_listing = || -> std::collections::HashSet<std::path::PathBuf> {
                std::fs::read_dir(&trash_files)
                    .map(|items| items.flatten().map(|item| item.path()).collect())
                    .unwrap_or_default()
            };
            for (case, steps) in cases {
                let root = tempfile::tempdir().expect("case root");
                let docs = root.path().join("docs");
                let dest = root.path().join("dest");
                std::fs::create_dir(&dest).expect("destination");
                customize_items(root.path(), &source);
                open.content
                    .browser
                    .navigate_location(Location::local(root.path()));
                wait_until(|| !column_loading(&open, 0));
                let browser = open.content.browser.browser();
                let mut known_trash = trash_listing();
                let mut trashed = Vec::new();
                let transfer = |conflict, move_sources| {
                    browser.transfer(
                        Location::local(&dest),
                        vec![PasteItem {
                            source: Location::local(&docs),
                            conflict,
                            target_name: None,
                        }],
                        move_sources,
                        false,
                    )
                };

                for (operation, expected) in steps {
                    match *operation {
                        MakeFolder(relative) => {
                            std::fs::create_dir(root.path().join(relative)).expect("new folder")
                        }
                        MakeFile(relative) => {
                            std::fs::write(root.path().join(relative), relative).expect("new file")
                        }
                        Customize(item) => {
                            let manager = PreferenceManager::shared();
                            let (relative, color, icon) = item;
                            let path = root.path().join(relative);
                            if !path.exists() {
                                std::fs::create_dir_all(&path).expect("customized folder");
                            }
                            manager.set_folder_color(
                                &path,
                                color.map(crate::model::FolderColorValue::Preset),
                            );
                            manager.set_custom_icon(&path, icon);
                        }
                        Rename => {
                            browser.rename(loaded_entry(&open, &docs), "docs2".into());
                        }
                        Move(conflict) => transfer(conflict, true),
                        Copy(conflict) => transfer(conflict, false),
                        Trash | PermanentDelete => browser.delete(
                            vec![loaded_entry(&open, &docs)],
                            matches!(operation, PermanentDelete),
                        ),
                        PutBack(index) => {
                            let mut entry = crate::test_support::operations::entry(
                                Location::local(&trashed[index]),
                            );
                            entry.kind = crate::model::EntryKind::Directory;
                            browser.restore(vec![RestoreTrashItem {
                                entry,
                                destination: docs.clone(),
                            }]);
                        }
                        Undo | Redo => {
                            let previous = browser.last_started_operation();
                            if matches!(operation, Undo) {
                                wait_until(|| open.content.browser.undo_last_operation());
                            } else {
                                wait_until(|| open.content.browser.redo_last_operation());
                            }
                            wait_until(|| browser.last_started_operation() != previous);
                        }
                    }
                    if !matches!(operation, MakeFolder(_) | MakeFile(_) | Customize(_)) {
                        wait_for_operation(&open);
                    }
                    if matches!(operation, MakeFolder(_)) {
                        loaded_entry(&open, &docs);
                    }
                    if matches!(operation, Trash) {
                        let listing = trash_listing();
                        trashed.extend(listing.difference(&known_trash).cloned());
                        known_trash = listing;
                    }
                    assert_item_customizations(root.path(), expected, case);
                }
            }
        },
    );
}
