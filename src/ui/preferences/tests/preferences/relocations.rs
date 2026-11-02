// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy)]
enum ItemStep {
    Relocate {
        from: &'static str,
        to: Option<&'static str>,
        merged: bool,
    },
    Forget {
        root: &'static str,
        trashed: Option<TrashMount>,
    },
    Restore(&'static str),
    RestoreRenumbered(&'static str),
    /// Puts a different folder at `path`, keeping the original alive elsewhere.
    ReplaceOnDisk(&'static str),
}

#[derive(Clone, Copy)]
enum TrashMount {
    KeepsNumbers,
    RenewsNumbers,
    RenewsNumbersWithoutBirths,
    RenewsNumbersInOneBirthTick,
}

#[test]
fn item_customizations_follow_relocations_and_removals() {
    gtk_test(
        "ui::preferences::tests::preferences::relocations::item_customizations_follow_relocations_and_removals",
        || {
            use crate::assets::icons::{FILE_CODE, PICTURES};
            use crate::model::FolderColor::{Blue, Green, Red};
            use ItemStep::{Forget, Relocate, ReplaceOnDisk, Restore, RestoreRenumbered};
            use TrashMount::{
                KeepsNumbers, RenewsNumbers, RenewsNumbersInOneBirthTick,
                RenewsNumbersWithoutBirths,
            };

            seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            let notifications = Rc::new(std::cell::Cell::new(0));
            let counted = notifications.clone();
            manager.observe(Rc::new(move || counted.set(counted.get() + 1)));
            let relocate = |from, to| Relocate {
                from,
                to: Some(to),
                merged: false,
            };
            let trash = |root| Forget {
                root,
                trashed: Some(KeepsNumbers),
            };
            let trash_on = |root, mount| Forget {
                root,
                trashed: Some(mount),
            };
            let customized_docs = [
                ("docs", Some(Red), Some(FILE_CODE)),
                ("docs/inner", Some(Blue), None),
                ("docs2", Some(Green), None),
            ];
            let forgotten_docs = [
                ("docs", None, None),
                ("docs/inner", None, None),
                ("docs2", Some(Green), None),
            ];
            type Customization = (
                &'static str,
                Option<crate::model::FolderColor>,
                Option<&'static str>,
            );
            type Case<'a> = (
                &'a str,
                &'a [Customization],
                &'a [ItemStep],
                &'a [Customization],
                usize,
            );
            // One coalesced color save and one folder sort change per step that changed something.
            let cases: [Case<'_>; 15] = [
                (
                    "a rename carries the root and descendants but not a look-alike sibling",
                    &customized_docs,
                    &[relocate("docs", "notes")],
                    &[
                        ("notes", Some(Red), Some(FILE_CODE)),
                        ("notes/inner", Some(Blue), None),
                        ("docs", None, None),
                        ("docs/inner", None, None),
                        ("docs2", Some(Green), None),
                    ],
                    1,
                ),
                (
                    "a move drops stale keys at and under its destination",
                    &[
                        ("docs", Some(Red), Some(FILE_CODE)),
                        ("dest/docs", Some(Green), Some(PICTURES)),
                        ("dest/docs/old", Some(Blue), None),
                    ],
                    &[relocate("docs", "dest/docs")],
                    &[
                        ("dest/docs", Some(Red), Some(FILE_CODE)),
                        ("dest/docs/old", None, None),
                        ("docs", None, None),
                    ],
                    1,
                ),
                (
                    "a merge keeps the destination root and carries descendants over it",
                    &[
                        ("docs", Some(Red), Some(FILE_CODE)),
                        ("docs/inner", Some(Blue), None),
                        ("dest/docs", Some(Green), Some(PICTURES)),
                        ("dest/docs/inner", Some(Red), None),
                    ],
                    &[Relocate {
                        from: "docs",
                        to: Some("dest/docs"),
                        merged: true,
                    }],
                    &[
                        ("dest/docs", Some(Green), Some(PICTURES)),
                        ("dest/docs/inner", Some(Blue), None),
                        ("docs", None, None),
                        ("docs/inner", None, None),
                    ],
                    1,
                ),
                (
                    "a repeated relocation is a no-op",
                    &customized_docs,
                    &[relocate("docs", "notes"), relocate("docs", "notes")],
                    &[
                        ("notes", Some(Red), Some(FILE_CODE)),
                        ("notes/inner", Some(Blue), None),
                        ("docs", None, None),
                    ],
                    1,
                ),
                (
                    "a move off local storage drops the keys",
                    &customized_docs,
                    &[Relocate {
                        from: "docs",
                        to: None,
                        merged: false,
                    }],
                    &forgotten_docs,
                    1,
                ),
                (
                    "the trashed item restored at its path gets its keys back",
                    &customized_docs,
                    &[trash("docs"), Restore("docs")],
                    &customized_docs,
                    2,
                ),
                (
                    "another item restored at the trashed path gets nothing",
                    &customized_docs,
                    &[trash("docs"), ReplaceOnDisk("docs"), Restore("docs")],
                    &forgotten_docs,
                    1,
                ),
                (
                    "a later trashed item with the same identity supersedes the kept keys",
                    &customized_docs,
                    &[trash("docs"), trash("docs"), Restore("docs")],
                    &forgotten_docs,
                    1,
                ),
                (
                    "a permanent delete keeps nothing to restore",
                    &customized_docs,
                    &[
                        Forget {
                            root: "docs",
                            trashed: None,
                        },
                        Restore("docs"),
                    ],
                    &forgotten_docs,
                    1,
                ),
                (
                    "a trashed item its mount renumbered gets its keys back by its birth time",
                    &customized_docs,
                    &[trash_on("docs", RenewsNumbers), RestoreRenumbered("docs")],
                    &customized_docs,
                    2,
                ),
                (
                    "another number where numbers are kept is another item",
                    &customized_docs,
                    &[trash_on("docs", KeepsNumbers), RestoreRenumbered("docs")],
                    &forgotten_docs,
                    1,
                ),
                (
                    "where only the number tells, the same number gets the keys back",
                    &customized_docs,
                    &[
                        trash_on("docs", RenewsNumbersWithoutBirths),
                        Restore("docs"),
                    ],
                    &customized_docs,
                    2,
                ),
                (
                    "where only the number tells, another number gets nothing",
                    &customized_docs,
                    &[
                        trash_on("docs", RenewsNumbersWithoutBirths),
                        RestoreRenumbered("docs"),
                    ],
                    &forgotten_docs,
                    1,
                ),
                (
                    "a sibling sharing the trashed folder's birth tick keeps its keys apart",
                    &customized_docs,
                    &[
                        trash_on("docs", RenewsNumbersInOneBirthTick),
                        trash_on("docs2", RenewsNumbersInOneBirthTick),
                        Restore("docs"),
                    ],
                    &[
                        ("docs", Some(Red), Some(FILE_CODE)),
                        ("docs/inner", Some(Blue), None),
                        ("docs2", None, None),
                    ],
                    3,
                ),
                (
                    "uncustomized items change nothing",
                    &[("docs2", Some(Green), None)],
                    &[relocate("docs", "notes"), trash("docs"), Restore("docs")],
                    &[("docs2", Some(Green), None)],
                    0,
                ),
            ];

            let settle = || {
                let context = glib::MainContext::default();
                while context.pending() {
                    context.iteration(false);
                }
            };
            // Folder sorts follow the same operations as colors, so each color
            // seeds a distinct sort that must end up wherever the color does.
            let sort_for = |color| match color {
                Red => FolderSort::Saved(SortKey::Type, SortDirection::Ascending),
                Blue => FolderSort::Saved(SortKey::Modified, SortDirection::Ascending),
                _ => FolderSort::Saved(SortKey::Name, SortDirection::Descending),
            };
            manager.set_remember_folder_views(true);
            for (case, seed, steps, expected, notified) in cases {
                let root = tempfile::tempdir().expect("case root");
                let path = |relative: &str| root.path().join(relative);
                let location = |relative: &str| Location::local(path(relative));
                let identity = |relative: &str, mount: TrashMount, renumbered: bool| {
                    use std::os::unix::fs::MetadataExt;
                    let metadata = fs::symlink_metadata(path(relative)).expect("identity");
                    let numbers = match mount {
                        KeepsNumbers => crate::services::InodeNumbers::Kept,
                        RenewsNumbers
                        | RenewsNumbersWithoutBirths
                        | RenewsNumbersInOneBirthTick => crate::services::InodeNumbers::Renewed,
                    };
                    let born = match mount {
                        RenewsNumbersInOneBirthTick => 1,
                        _ => metadata.ino() as i64,
                    };
                    crate::services::ItemIdentity {
                        evidence: crate::services::Evidence {
                            device: metadata.dev(),
                            inode: metadata.ino() + u64::from(renumbered),
                            kind: rustix::fs::FileType::Directory,
                            birth: (!matches!(mount, RenewsNumbersWithoutBirths))
                                .then_some((born, 0)),
                            handle: None,
                            attributes: None,
                        },
                        traits: crate::services::IdentityTraits {
                            numbers,
                            ..Default::default()
                        },
                    }
                };
                let mut trashed_on = KeepsNumbers;
                fs::create_dir_all(path("docs")).expect("docs folder");
                for (relative, color, icon) in seed {
                    fs::create_dir_all(path(relative)).expect("customized folder");
                    manager.set_folder_color(&path(relative), color.map(FolderColorValue::Preset));
                    manager.set_custom_icon(&path(relative), *icon);
                    if let Some(FolderSort::Saved(sort_key, sort_direction)) = color.map(sort_for) {
                        manager.set_folder_sort(&location(relative), sort_key, sort_direction);
                    }
                }
                settle();
                notifications.set(0);
                let folder_revision = manager.folder_sorts_revision();

                for step in steps {
                    match *step {
                        Relocate { from, to, merged } => {
                            manager.relocate_item_customizations(
                                &path(from),
                                to.map(path).as_deref(),
                                merged,
                            );
                            let to = to.map_or_else(
                                || Location::uri("sftp://example.invalid/docs"),
                                location,
                            );
                            manager.relocate_folder_views(&location(from), &to, merged);
                        }
                        Forget { root, trashed } => {
                            trashed_on = trashed.unwrap_or(trashed_on);
                            let identity = trashed.map(|mount| identity(root, mount, false));
                            manager.forget_item_customizations(&path(root), identity);
                            manager.forget_folder_views_within(&location(root), identity);
                        }
                        Restore(root) | RestoreRenumbered(root) => {
                            let renumbered = matches!(step, RestoreRenumbered(_));
                            let restored = identity(root, trashed_on, renumbered);
                            manager.restore_item_customizations(&path(root), restored);
                            manager.restore_folder_views(&location(root), Some(restored));
                        }
                        ReplaceOnDisk(relative) => {
                            fs::rename(path(relative), root.path().join("kept-alive"))
                                .expect("move the original aside");
                            fs::create_dir(path(relative)).expect("another folder");
                        }
                    }
                    settle();
                }

                let folder_changes = manager
                    .folder_sorts_revision()
                    .wrapping_sub(folder_revision);
                assert_eq!(
                    (notifications.get(), folder_changes),
                    (notified * 2, notified as u64),
                    "{case}: notifications of color and folder sort changes"
                );
                crate::storage::writer::settle();
                let saved = read_preferences().expect("saved preferences");
                for (relative, color, icon) in expected {
                    assert_eq!(
                        manager.resolve_folder_sort(&location(relative), false),
                        color.map_or(FolderSort::Default, sort_for),
                        "{case}: folder sort of {relative}"
                    );
                    let path = path(relative);
                    let key = path.to_string_lossy().into_owned();
                    assert_eq!(
                        manager.folder_color(&path),
                        color.map(FolderColorValue::Preset),
                        "{case}: color of {relative}"
                    );
                    assert_eq!(
                        manager.custom_icon(&path).as_deref(),
                        *icon,
                        "{case}: icon of {relative}"
                    );
                    assert_eq!(
                        (
                            saved.folder_colors.contains_key(&key),
                            saved.custom_icons.contains_key(&key)
                        ),
                        (color.is_some(), icon.is_some()),
                        "{case}: saved keys of {relative}"
                    );
                }
            }
        },
    );
}

#[test]
fn settings_files_are_watched_from_the_monitors_worker_without_missing_a_save() {
    gtk_test(
        "ui::preferences::tests::preferences::relocations::settings_files_are_watched_from_the_monitors_worker_without_missing_a_save",
        || {
            use crate::adapters::{FsOp, FsQuirks, Quirks};

            seed_saved_preferences_for_test();
            let folder_views = crate::ui::preferences::folder_view_settings::folder_views_path();
            fs::create_dir_all(folder_views.parent().expect("state directory"))
                .expect("state directory");
            fs::write(&folder_views, "").expect("folder settings file");
            let sandbox = glib::home_dir()
                .parent()
                .expect("isolated sandbox")
                .to_path_buf();
            let gate = crate::test_support::Gate::default();
            let scope = FsQuirks::install(
                &sandbox,
                Quirks {
                    before: vec![(FsOp::Watch, "settings.toml", 0, gate.racer())],
                    ..Quirks::default()
                },
            );
            let ticked = Rc::new(std::cell::Cell::new(false));
            let tick = ticked.clone();
            glib::timeout_add_local_once(Duration::from_millis(10), move || tick.set(true));

            let manager = PreferenceManager::shared();
            wait_for(
                || gate.reached() && ticked.get(),
                "the settings watch to wait on the monitors worker while the window runs",
            );
            assert_eq!(
                manager.default_sort(),
                (SortKey::Size, SortDirection::Descending)
            );
            let mut saved = non_default_preferences();
            saved.sort_key = "modified".into();
            saved.sort_direction = "ascending".into();
            fs::write(
                settings_path(),
                toml::to_string(&saved).expect("serialize another process's save"),
            )
            .expect("another process saves");
            gate.release();
            wait_for(
                || manager.default_sort() == (SortKey::Modified, SortDirection::Ascending),
                "the save made before the watch was live",
            );
            assert_eq!(scope.count(FsOp::Watch), 2);

            drop(manager);
            wait_for(
                || scope.count(FsOp::Unwatch) == 2,
                "both watches to be released on the monitors worker",
            );
        },
    );
}

#[test]
fn a_folder_that_failed_to_open_forgets_its_settings_once_a_worker_proves_it_deleted() {
    gtk_test(
        "ui::preferences::tests::preferences::relocations::a_folder_that_failed_to_open_forgets_its_settings_once_a_worker_proves_it_deleted",
        || {
            use crate::adapters::{FsOp, FsQuirks, Quirks};

            let manager = PreferenceManager::shared();
            manager.set_remember_folder_views(true);
            let root = tempfile::tempdir().expect("fixture");
            let crowded = root.path().join("crowded");
            let empty = root.path().join("empty");
            fs::create_dir_all(crowded.join("kept")).expect("fixture folder");
            fs::create_dir(&empty).expect("fixture folder");
            let saved = FolderSort::Saved(SortKey::Type, SortDirection::Ascending);
            let folders = [
                crowded.join("gone"),
                crowded.join("kept"),
                empty.join("lost"),
            ]
            .map(Location::local);
            for folder in &folders {
                manager.set_folder_sort(folder, SortKey::Type, SortDirection::Ascending);
            }
            let gate = crate::test_support::Gate::default();
            let _scope = FsQuirks::install(
                root.path(),
                Quirks {
                    before: vec![(FsOp::Stat, "gone", 0, gate.racer())],
                    ..Quirks::default()
                },
            );
            let sorts = || {
                folders
                    .each_ref()
                    .map(|folder| manager.resolve_folder_sort(folder, false))
            };

            for folder in &folders {
                manager.forget_missing_folder(folder);
            }
            wait_for(|| gate.reached(), "the deletion check to reach the seam");
            assert_eq!(sorts(), [saved; 3], "kept until the check answers");

            gate.release();
            let forgotten = [FolderSort::Default, saved, saved];
            wait_for(
                || sorts() == forgotten,
                "the deleted folder to be forgotten",
            );
            let settled = std::time::Instant::now() + Duration::from_millis(200);
            while std::time::Instant::now() < settled {
                glib::MainContext::default().iteration(false);
            }
            assert_eq!(sorts(), forgotten, "only the folder missing beside others");
        },
    );
}
