// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn trashing_and_restoring_report_each_items_identity_and_a_permanent_delete_reports_none() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::deletion::trash::trashing_and_restoring_report_each_items_identity_and_a_permanent_delete_reports_none",
        || {
            for permanent in [false, true] {
                let root = tempfile::tempdir().expect("root");
                let folder = root.path().join("folder");
                fs::create_dir(&folder).expect("folder");
                let identity = numbers_at(&folder);
                let events = Rc::new(RefCell::new(Vec::new()));
                let emitted = events.clone();
                let _operation = LocalOperationProvider.delete(
                    DeleteRequest {
                        id: OperationRequestId(95),
                        entries: vec![directory_entry(&folder)],
                        permanent,
                    },
                    Rc::new(move |event| emitted.borrow_mut().push(event)),
                );
                wait_for_operation(&events, |event| {
                    matches!(
                        event,
                        OperationEvent::Deleted { .. } | OperationEvent::CompletedWithErrors { .. }
                    )
                });

                let trashed = {
                    let events = events.borrow();
                    assert!(matches!(
                        events.last(),
                        Some(OperationEvent::Deleted { .. })
                    ));
                    assert_eq!(
                        trashed_items(&events),
                        if permanent {
                            Vec::new()
                        } else {
                            vec![(Location::local(&folder), identity)]
                        }
                    );
                    events.iter().find_map(|event| match event {
                        OperationEvent::ItemTrashed { identity, .. } => Some(*identity),
                        _ => None,
                    })
                };
                let Some(trashed) = trashed else {
                    continue;
                };
                let restoring = Rc::new(RefCell::new(Vec::new()));
                let emitted = restoring.clone();
                let _operation = LocalOperationProvider.restore(
                    RestoreRequest {
                        id: OperationRequestId(96),
                        source: RestoreSource::OriginalLocations {
                            locations: vec![Location::local(&folder)],
                            originals: HashMap::new(),
                        },
                    },
                    Rc::new(move |event| emitted.borrow_mut().push(event)),
                );
                wait_for_operation(&restoring, |event| {
                    matches!(
                        event,
                        OperationEvent::Restored { .. }
                            | OperationEvent::RestoreCompletedWithErrors { .. }
                            | OperationEvent::Failed { .. }
                    )
                });
                let restoring = restoring.borrow();
                let restored = restoring
                    .iter()
                    .filter_map(|event| match event {
                        OperationEvent::ItemRestored {
                            location, identity, ..
                        } => Some((location.clone(), *identity)),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert!(
                    matches!(&restored[..], [(location, restored)]
                        if *location == Location::local(&folder) && restored.unchanged(&trashed)),
                    "{restored:?} after {:?}",
                    restoring.last()
                );
            }
        },
    );
}

#[derive(Clone, Copy, Debug)]
enum Swap {
    Untouched,
    RenamedOver,
    MovedAside,
}

#[test]
fn trash_reports_the_item_it_moved_when_another_took_the_name_first() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::deletion::trash::trash_reports_the_item_it_moved_when_another_took_the_name_first",
        || {
            let rows: [(Preset, Op, Swap, bool); 5] = [
                (Quirks::ext4, Op::Trash, Swap::Untouched, false),
                (Quirks::ext4, Op::Trash, Swap::RenamedOver, true),
                (Quirks::ext4, Op::OpenAt, Swap::RenamedOver, true),
                (Quirks::sshfs, Op::Trash, Swap::MovedAside, true),
                (Quirks::nfs3, Op::Trash, Swap::RenamedOver, false),
            ];
            for (preset, op, swap, reports_arrival) in rows {
                let root = tempfile::tempdir().expect("root");
                let item = root.path().join("report.txt");
                let arrival = root.path().join("arrival.txt");
                fs::write(&item, b"checked").expect("item");
                fs::write(&arrival, b"arrived later").expect("arrival");
                let (checked, arrived) = (numbers_at(&item), numbers_at(&arrival));
                let (from, to, aside) = (
                    arrival.clone(),
                    item.clone(),
                    root.path().join("report.txt~"),
                );
                let racer: Racer = Box::new(move || match swap {
                    Swap::Untouched => {}
                    Swap::RenamedOver => fs::rename(&from, &to).expect("renamed over"),
                    Swap::MovedAside => {
                        fs::rename(&to, &aside).expect("moved aside");
                        fs::rename(&from, &to).expect("renamed in");
                    }
                });
                let _quirks = FsQuirks::install(
                    root.path(),
                    Quirks {
                        before: vec![(op, "report.txt", 0, racer)],
                        ..preset()
                    },
                );
                let events = Rc::new(RefCell::new(Vec::new()));
                let emitted = events.clone();
                let _operation = LocalOperationProvider.delete(
                    DeleteRequest {
                        id: OperationRequestId(13),
                        entries: vec![file_entry(&item)],
                        permanent: false,
                    },
                    Rc::new(move |event| emitted.borrow_mut().push(event)),
                );
                wait_for_operation(&events, |event| {
                    matches!(
                        event,
                        OperationEvent::Deleted { .. } | OperationEvent::CompletedWithErrors { .. }
                    )
                });

                let events = events.borrow();
                assert!(
                    matches!(events.last(), Some(OperationEvent::Deleted { .. })),
                    "{op:?}, {swap:?}: {:?}",
                    events.last()
                );
                assert_eq!(
                    trashed_items(&events),
                    [(
                        Location::local(&item),
                        if reports_arrival { arrived } else { checked }
                    )],
                    "{:?}, {op:?}, {swap:?}",
                    preset().mount_type
                );
            }
        },
    );
}

#[test]
fn verified_trash_learns_fuse_refusals_and_cleans_leftovers_on_both_worker_routes() {
    const NAME: &str = "adapters::local_operations::tests::deletion::trash::verified_trash_learns_fuse_refusals_and_cleans_leftovers_on_both_worker_routes";
    crate::test_support::gtk_test(NAME, || {
        use crate::adapters::trash_support::{self, TrashFailure, UnsupportedReason};
        let Some((_home, volume)) = crate::test_support::distinct_device_dirs(NAME) else {
            return;
        };
        for synchronous in [false, true] {
            for trash in [
                Trash::FalseWithoutError,
                Trash::SucceedsWithoutMoving,
                Trash::ProtocolRefused,
            ] {
                let root = tempfile::TempDir::new_in(volume.path()).expect("root");
                let folder = root.path().join("folder");
                fs::create_dir(&folder).expect("folder");
                fs::write(folder.join("keep"), b"keep").expect("contents");
                let leftovers = crate::adapters::trash_support::tests::leftover_trash(root.path());
                let created = leftovers.clone();
                let _quirks = FsQuirks::install(
                    root.path(),
                    Quirks {
                        trash,
                        before: vec![(
                            Op::Trash,
                            "folder",
                            0,
                            Box::new(move || {
                                fs::create_dir_all(created.join("files")).expect("files");
                                fs::create_dir_all(created.join("info")).expect("info");
                            }),
                        )],
                        ..Quirks::default()
                    },
                );
                crate::adapters::volume::set_mountinfo_for_isolated_test(Some(format!(
                    "9001 1 0:91 / {} rw - fuse.sshfs fixture rw\n",
                    root.path().display()
                )));
                let context = glib::MainContext::default();
                let result = if synchronous {
                    let path = folder.clone();
                    context
                        .block_on(super::super::super::run_blocking(move || {
                            let parent =
                                open_local_parent_directory(path.parent().expect("parent"))
                                    .expect("open");
                            let at = super::super::super::Entry::new(
                                &parent,
                                path.file_name().expect("name"),
                            );
                            let stat = super::super::super::fs().stat(at, false).expect("stat");
                            Ok(super::super::super::trash::trash_verified(
                                at,
                                &path,
                                &stat,
                                &gio::Cancellable::new(),
                            )
                            .map(drop))
                        }))
                        .expect("worker")
                } else {
                    context
                        .block_on(trash_support::trash_with_cleanup(
                            &gio::File::for_path(&folder),
                            &gio::Cancellable::new(),
                        ))
                        .map(drop)
                };
                let expected = if matches!(trash, Trash::ProtocolRefused) {
                    UnsupportedReason::FuseRefused
                } else {
                    UnsupportedReason::Reported
                };
                assert_eq!(
                    result.expect_err("not moved").failure,
                    TrashFailure::Unsupported(expected)
                );
                assert_eq!(
                    trash_support::unsupported_reason_for(&folder).map(|(_, reason)| reason),
                    Some(expected)
                );
                assert!(!leftovers.exists(), "{synchronous:?} {trash:?}");
                assert_eq!(fs::read(folder.join("keep")).expect("kept"), b"keep");
                trash_support::forget_all();
                crate::adapters::volume::set_mountinfo_for_isolated_test(None);
            }
        }
    });
}

#[test]
fn a_trash_that_moves_nothing_is_retryable_and_records_no_undo() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::deletion::trash::a_trash_that_moves_nothing_is_retryable_and_records_no_undo",
        || {
            let rows: [(Trash, Preset, Option<bool>); 5] = [
                (Trash::FalseWithoutError, Quirks::default, None),
                (Trash::SucceedsWithoutMoving, Quirks::default, None),
                (Trash::SucceedsWithoutMoving, Quirks::vfat, Some(false)),
                (Trash::SucceedsWithoutMoving, Quirks::sshfs, Some(false)),
                (Trash::SucceedsWithoutMoving, Quirks::exfat, Some(false)),
            ];
            for (trash, preset, evicted) in rows {
                let root = tempfile::tempdir().expect("root");
                let folder = root.path().join("folder");
                fs::create_dir(&folder).expect("folder");
                let evicting = Arc::new(Mutex::new(None));
                let (path, outcome) = (folder.clone(), evicting.clone());
                let racer: super::super::fs_quirks::Racer = Box::new(move || {
                    *outcome.lock().expect("outcome") = Some(super::super::fs_quirks::evict(&path));
                });
                let _quirks = FsQuirks::install(
                    root.path(),
                    Quirks {
                        trash,
                        after: if evicted.is_some() {
                            vec![(Op::Trash, "folder", 0, racer)]
                        } else {
                            Vec::new()
                        },
                        ..preset()
                    },
                );
                let events = Rc::new(RefCell::new(Vec::new()));
                let emitted = events.clone();
                let _operation = LocalOperationProvider.delete(
                    DeleteRequest {
                        id: OperationRequestId(4060),
                        entries: vec![directory_entry(&folder)],
                        permanent: false,
                    },
                    Rc::new(move |event| emitted.borrow_mut().push(event)),
                );
                wait_for_operation(&events, |event| {
                    matches!(
                        event,
                        OperationEvent::Deleted { .. } | OperationEvent::CompletedWithErrors { .. }
                    )
                });

                let events = events.borrow();
                assert!(
                    matches!(
                        events.last(),
                        Some(OperationEvent::CompletedWithErrors { deleted_locations, retryable_locations, .. })
                            if deleted_locations.is_empty()
                                && retryable_locations == &[Location::local(&folder)]
                    ),
                    "{trash:?}: {:?}",
                    events.last()
                );
                assert!(trashed_items(&events).is_empty());
                assert!(folder.is_dir());
                assert_eq!(
                    *evicting.lock().expect("outcome"),
                    evicted,
                    "{:?}",
                    preset().mount_type
                );
            }
        },
    );
}
