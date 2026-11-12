// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{KeptCause, KeptItem, RenameCause, RenamedItem, SpecialKind, TransferReport};

fn archived_report() -> MoveRecord {
    move_record("/fixture/report.txt", "/fixture/archive/report.txt")
}

/// The undo recorded after a move into `/fixture/archive` reports `events`.
fn move_undo_after(
    events: impl FnOnce(OperationRequestId) -> Vec<OperationEvent>,
) -> Option<UndoEntry> {
    let browser = operating_browser();
    let (request_id, emit) = start_operation(
        &browser,
        OperationSetup::Move(Location::local("/fixture/archive")),
    );
    for event in events(request_id) {
        emit(event);
    }
    pending_undo_entry()
}

fn renamed_in_archive(requested: &str, saved_as: &str, cause: RenameCause) -> RenamedItem {
    RenamedItem {
        requested: Location::local(format!("/fixture/archive/{requested}")),
        saved_as: Location::local(format!("/fixture/archive/{saved_as}")),
        cause,
    }
}

fn kept_report(request_id: OperationRequestId, left: &[&str], cause: &KeptCause) -> OperationEvent {
    OperationEvent::ItemsReported {
        request_id,
        report: TransferReport {
            kept: left
                .iter()
                .map(|left| KeptItem {
                    location: Location::local(left),
                    cause: cause.clone(),
                })
                .collect(),
            ..Default::default()
        },
    }
}

/// Entries a move left behind, the items it moved, and the items its report keeps out of undo.
struct LeftBehind<'a> {
    left: &'a [&'a str],
    cause: KeptCause,
    moving: Vec<Location>,
    kept_out_of_undo: Vec<Location>,
}

fn assert_left_behind_items_are_excluded_from_the_move_undo(case: LeftBehind) {
    let browser = operating_browser();
    let shown = Rc::new(RefCell::new(Vec::new()));
    let observed = shown.clone();
    browser.observe(move |event| {
        if let BrowserEvent::ItemsReported { report } = event {
            observed.borrow_mut().push(report.kept_out_of_undo.clone());
        }
    });
    let (request_id, emit) = start_operation(
        &browser,
        OperationSetup::Move(Location::local("/fixture/archive")),
    );

    emit(kept_report(request_id, case.left, &case.cause));
    let mut locations = case.moving;
    locations.push(Location::local("/fixture/report.txt"));
    emit(pasted(request_id, locations));

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Move(vec![archived_report()])),
        "moving the project back would replace the newer notes left in its folder"
    );
    assert_eq!(*shown.borrow(), [case.kept_out_of_undo], "{:?}", case.left);
}

#[test]
fn a_completed_move_records_where_each_item_landed() {
    let browser = operating_browser();

    transfer_one(&browser, "/fixture/archive", "/fixture/report.txt", true);

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Move(vec![archived_report()]))
    );
}

#[test]
fn a_move_into_the_items_own_directory_records_no_undo() {
    let browser = operating_browser();

    transfer_one(&browser, "/fixture", "/fixture/report.txt", true);

    assert_eq!(pending_undo_entry(), None);
}

#[test]
fn undoing_a_move_transfers_items_back_once() {
    let browser = operating_browser();
    forget_requests(&UNDO_MOVE_REQUESTS);
    transfer_one(&browser, "/fixture/archive", "/fixture/report.txt", true);
    let (generation, records) = browser.pending_undo_move().expect("pending move undo");

    assert!(browser.undo_move(generation, undo_move_items(records.clone())));

    assert_eq!(requests_in(&UNDO_MOVE_REQUESTS), vec![records]);
    assert_eq!(
        (pending_undo_entry(), browser.pending_undo_move()),
        (None, None)
    );
    assert!(!browser.undo_last_trash());
}

#[test]
fn an_undo_claim_from_an_earlier_operation_is_rejected() {
    let browser = operating_browser();
    let record = archived_report();
    push_pending_undo(UndoEntry::Move(vec![record.clone()]));
    let (stale_generation, _) = peek_replay(false).expect("pending undo");
    push_pending_undo(trash_undo(vec![Location::local("/fixture/note.txt")]));

    assert!(!browser.undo_move(stale_generation, undo_move_items([record])));
    assert!(browser.undo_last_trash());
}

#[test]
fn a_partial_move_undo_keeps_the_items_still_to_move_back() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let first = move_record("/fixture/first.txt", "/fixture/archive/first.txt");
    let second = move_record("/fixture/second.txt", "/fixture/archive/second.txt");
    let (request_id, emit) = start_replay(
        &browser,
        UndoEntry::Move(vec![first.clone(), second.clone()]),
        OperationSetup::MoveBack,
    );

    emit(OperationEvent::TransferFailed {
        request_id,
        completed_locations: vec![first.current.clone()],
        message: "injected failure".to_owned(),
    });

    assert_eq!(pending_undo_entry(), Some(UndoEntry::Move(vec![second])));
}

#[test]
fn a_partial_move_undo_does_not_retry_items_excluded_before_transfer() {
    let skipped = move_record("/fixture/skipped.txt", "/fixture/archive/skipped.txt");
    let completed = move_record("/fixture/completed.txt", "/fixture/archive/completed.txt");
    let retryable = move_record("/fixture/retryable.txt", "/fixture/archive/retryable.txt");
    push_pending_undo(UndoEntry::Move(vec![
        skipped,
        completed.clone(),
        retryable.clone(),
    ]));
    let (generation, _) = claim_pending_undo(None).expect("undo claim");
    let submitted = undo_move_items([completed.clone(), retryable.clone()]);

    retain_replay_move_items(false, generation, &submitted);
    mark_replay_item_completed(false, generation, &completed.current);
    finish_undo(generation, false);

    assert_eq!(pending_undo_entry(), Some(UndoEntry::Move(vec![retryable])));
}

#[test]
fn undoing_a_move_leaves_a_pending_cut_untouched() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = record_events(&browser);
    let record = archived_report();
    let (request_id, emit) = start_replay(
        &browser,
        UndoEntry::Move(vec![record.clone()]),
        OperationSetup::MoveBack,
    );

    emit(pasted(request_id, vec![record.current.clone()]));

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::TransferFinished { moved_locations } if moved_locations.is_empty()
    )));
}

#[test]
fn a_move_undo_record_keeps_each_renamed_entry_once_beneath_the_item_it_moved() {
    let item = renamed_in_archive;
    let undo = move_undo_after(|request_id| {
        vec![
            OperationEvent::ItemMoved {
                request_id,
                from: Location::local("/fixture/pro:ject"),
                to: Location::local("/fixture/archive/project"),
                merged: false,
            },
            OperationEvent::ItemsReported {
                request_id,
                report: TransferReport {
                    renamed: vec![
                        item("project/x:y", "project/x_y", RenameCause::Invalid),
                        item("project/x_y/c_d", "project/x_y/c_d (1)", RenameCause::Alias),
                        item("notes/e?f", "notes/e_f", RenameCause::Invalid),
                        item("pro:ject", "project", RenameCause::Invalid),
                        item("elsewhere/g:h", "elsewhere/g_h", RenameCause::Invalid),
                        item("project/x_y/c:d", "project/x_y/c_d", RenameCause::Invalid),
                    ],
                    ..Default::default()
                },
            },
            pasted(
                request_id,
                vec![
                    Location::local("/fixture/pro:ject"),
                    Location::local("/fixture/notes"),
                ],
            ),
        ]
    });

    let Some(UndoEntry::Move(records)) = undo else {
        panic!("the move's undo");
    };
    assert_eq!(
        records
            .into_iter()
            .map(|record| (record.original, record.renamed))
            .collect::<Vec<_>>(),
        [
            (
                Location::local("/fixture/pro:ject"),
                vec![
                    item("project/x:y", "project/x_y", RenameCause::Invalid),
                    item(
                        "project/x_y/c:d",
                        "project/x_y/c_d (1)",
                        RenameCause::Invalid
                    ),
                ]
            ),
            (
                Location::local("/fixture/notes"),
                vec![item("notes/e?f", "notes/e_f", RenameCause::Invalid)]
            ),
        ]
    );
}

#[test]
fn a_move_is_undone_from_where_the_item_went() {
    let source = Location::local("/fixture/report.txt");
    let kept_both = Location::local("/fixture/archive/report (1).txt");

    let undo = move_undo_after(|request_id| {
        vec![
            OperationEvent::ItemMoved {
                request_id,
                from: source.clone(),
                to: kept_both.clone(),
                merged: false,
            },
            pasted(request_id, vec![source.clone()]),
        ]
    });

    assert_eq!(
        undo,
        Some(UndoEntry::Move(vec![MoveRecord {
            original: source,
            current: kept_both,
            renamed: Vec::new(),
            replaced: None,
        }]))
    );
}

#[test]
fn a_move_undo_record_keeps_the_names_the_move_changed_inside_its_item() {
    let source = Location::local("/fixture/project");
    let alias = |folder: &str| {
        renamed_in_archive(
            &format!("{folder}/makefile"),
            &format!("{folder}/makefile (1)"),
            RenameCause::Alias,
        )
    };

    let undo = move_undo_after(|request_id| {
        vec![
            OperationEvent::ItemsReported {
                request_id,
                report: TransferReport {
                    renamed: vec![alias("project"), alias("elsewhere")],
                    ..Default::default()
                },
            },
            pasted(request_id, vec![source.clone()]),
        ]
    });

    assert_eq!(
        undo,
        Some(UndoEntry::Move(vec![MoveRecord {
            original: source,
            current: Location::local("/fixture/archive/project"),
            renamed: vec![alias("project")],
            replaced: None,
        }]))
    );
}

#[test]
fn an_item_whose_move_left_entries_behind_is_excluded_from_the_move_undo() {
    let project = Location::local("/fixture/project");
    let (zeta, alpha) = (
        Location::local("/fixture/zeta"),
        Location::local("/fixture/alpha"),
    );
    // An entry the move left in the project, the project itself, a link left
    // out whole, which never moved, and entries left in two items, which are
    // named in the order they moved.
    let cases = [
        LeftBehind {
            left: &["/fixture/project/notes.txt"],
            cause: KeptCause::Changed,
            moving: vec![project.clone()],
            kept_out_of_undo: vec![project.clone()],
        },
        LeftBehind {
            left: &["/fixture/project"],
            cause: KeptCause::Changed,
            moving: vec![project.clone()],
            kept_out_of_undo: vec![project.clone()],
        },
        LeftBehind {
            left: &["/fixture/link"],
            cause: KeptCause::Skipped(SpecialKind::Symlink),
            moving: Vec::new(),
            kept_out_of_undo: Vec::new(),
        },
        LeftBehind {
            left: &["/fixture/alpha/a.txt", "/fixture/zeta/z.txt"],
            cause: KeptCause::Changed,
            moving: vec![zeta.clone(), alpha.clone()],
            kept_out_of_undo: vec![zeta, alpha],
        },
    ];
    for case in cases {
        assert_left_behind_items_are_excluded_from_the_move_undo(case);
    }
}

#[test]
fn a_merged_move_source_is_excluded_from_the_move_undo() {
    let merged_source = Location::local("/fixture/folder");

    let undo = move_undo_after(|request_id| {
        vec![
            merged(
                request_id,
                &merged_source,
                vec![Location::local("/fixture/archive/folder/incoming.txt")],
                vec![Location::local("/fixture/archive/folder/shared.txt")],
            ),
            pasted(
                request_id,
                vec![
                    merged_source.clone(),
                    Location::local("/fixture/report.txt"),
                ],
            ),
        ]
    });

    assert_eq!(
        undo,
        Some(UndoEntry::Move(vec![archived_report()])),
        "a merged move deletes its source, so only the plain move can be undone"
    );
}

#[test]
fn a_replaced_move_keeps_the_staged_original_with_its_move_undo() {
    let identity = folder_identity(41, 1);
    let mut record = archived_report();
    record.replaced = Some(identity);
    let undo = move_undo_after(|request_id| {
        vec![
            OperationEvent::ItemTrashed {
                request_id,
                location: record.current.clone(),
                identity,
            },
            pasted(request_id, vec![record.original.clone()]),
        ]
    });

    assert_eq!(undo, Some(UndoEntry::Move(vec![record])));
}
