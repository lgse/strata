// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{RenameCause, RenamedItem, TransferReport};

fn start_copy_into_archive(browser: &Rc<Browser>) -> StartedOperation {
    start_operation(
        browser,
        OperationSetup::Copy(Location::local("/fixture/archive")),
    )
}

fn renamed_in_folder(requested: &str, saved_as: &str, cause: RenameCause) -> RenamedItem {
    RenamedItem {
        requested: Location::local(format!("/fixture/archive/folder/{requested}")),
        saved_as: Location::local(format!("/fixture/archive/folder/{saved_as}")),
        cause,
    }
}

fn renamed_report(request_id: OperationRequestId, renamed: Vec<RenamedItem>) -> OperationEvent {
    OperationEvent::ItemsReported {
        request_id,
        report: TransferReport {
            renamed,
            ..Default::default()
        },
    }
}

/// Undoes the pending merge and checks the provider got exactly its paths and `placed`.
fn assert_the_pending_merge_undo_reverts(
    browser: &Rc<Browser>,
    placed: HashMap<Location, PlacedSnapshot>,
) {
    let Some((generation, created, overwritten)) = browser.pending_undo_merge() else {
        panic!("expected a pending merge undo");
    };
    assert!(browser.undo_merge(generation, created.clone(), overwritten.clone()));
    assert_eq!(
        requests_in(&UNDO_MERGE_REQUESTS),
        vec![(created, overwritten, HashMap::new(), placed)]
    );
}

fn assert_a_fixed_name_taken_for_another_is_undone_under_its_last_name(renamed: Vec<RenamedItem>) {
    let fixed = renamed_in_folder("a:b", "a_b", RenameCause::Invalid);
    let browser = operating_browser();
    let shown = Rc::new(RefCell::new(Vec::new()));
    let observed = shown.clone();
    browser.observe(move |event| {
        if let BrowserEvent::ItemsReported { report } = event {
            observed.borrow_mut().push(report.clone());
        }
    });
    let source = Location::local("/fixture/folder");
    let (request_id, emit) = start_copy_into_archive(&browser);

    emit(merged(
        request_id,
        &source,
        vec![fixed.saved_as],
        Vec::new(),
    ));
    emit(renamed_report(request_id, renamed));
    emit(pasted(request_id, vec![source]));

    assert_eq!(
        pending_undo_entry(),
        Some(merge_undo(
            vec![Location::local("/fixture/archive/folder/a_b (1)")],
            Vec::new(),
            HashMap::new(),
        ))
    );
    assert_eq!(
        *shown.borrow(),
        [TransferReport {
            renamed: vec![renamed_in_folder("a:b", "a_b (1)", RenameCause::Invalid)],
            ..Default::default()
        }]
    );
}

fn assert_a_kept_item_stays_out_of_the_merge_undo(
    cancelled: bool,
    created: Vec<Location>,
    expected: UndoEntry,
) {
    let kept = Location::local("/fixture/archive/kept.txt");
    let cut = Location::local("/fixture/archive/cut.txt");
    PENDING_UNDO.with(|pending| *pending.borrow_mut() = UndoState::default());
    let browser = operating_browser();
    let events = record_events(&browser);
    push_pending_undo(trash_undo(vec![Location::local("/fixture/earlier.txt")]));
    let (request_id, emit) = start_replay(
        &browser,
        merge_undo(created, Vec::new(), HashMap::new()),
        OperationSetup::Restoration,
    );

    emit(OperationEvent::ItemKept {
        request_id,
        location: kept.clone(),
    });
    emit(if cancelled {
        OperationEvent::Cancelled {
            request_id,
            result: CancelledOperation {
                failed: vec![kept.clone()],
                not_attempted: vec![cut],
                ..Default::default()
            },
        }
    } else {
        OperationEvent::CompletedWithErrors {
            request_id,
            deleted_locations: Vec::new(),
            retryable_locations: Vec::new(),
            has_non_retryable_failures: true,
            message: "kept.txt: It was kept".to_owned(),
        }
    });

    assert_eq!(pending_undo_entry(), Some(expected));
    assert!(
        !events.borrow().iter().any(|event| matches!(
            event,
            BrowserEvent::ItemRemoved { location, .. } if *location == kept
        )),
        "a kept item stays in view"
    );
}

#[test]
fn a_merged_copy_records_created_and_overwritten_paths_for_undo() {
    let browser = operating_browser();
    forget_requests(&UNDO_MERGE_REQUESTS);
    let source = Location::local("/fixture/folder");
    let created = Location::local("/fixture/archive/folder/incoming.txt");
    let overwritten = Location::local("/fixture/archive/folder/shared.txt");
    let (request_id, emit) = start_copy_into_archive(&browser);

    let placed = placed_at(&[&created, &overwritten]);
    emit(merged(
        request_id,
        &source,
        vec![created.clone()],
        vec![overwritten.clone()],
    ));
    emit(merge_placed(request_id, &source, placed.clone()));
    emit(pasted(request_id, vec![source]));

    assert_eq!(
        pending_undo_entry(),
        Some(merge_undo(vec![created], vec![overwritten], placed.clone()))
    );
    assert_the_pending_merge_undo_reverts(&browser, placed);
    assert_eq!(pending_undo_entry(), None);
}

#[test]
fn a_merge_undo_removes_merged_items_under_the_names_they_were_saved_as() {
    let browser = operating_browser();
    let source = Location::local("/fixture/folder");
    let renamed = vec![
        renamed_in_folder("makefile", "makefile (1)", RenameCause::Alias),
        renamed_in_folder("notes.txt", "notes (1).txt", RenameCause::Taken),
    ];
    let (saved_as, numbered) = (renamed[0].saved_as.clone(), renamed[1].saved_as.clone());
    let (request_id, emit) = start_copy_into_archive(&browser);

    emit(merged(
        request_id,
        &source,
        vec![renamed[0].requested.clone()],
        vec![renamed[1].requested.clone()],
    ));
    emit(renamed_report(request_id, renamed));
    let placed = placed_at(&[&saved_as, &numbered]);
    emit(merge_placed(request_id, &source, placed.clone()));
    emit(pasted(request_id, vec![source]));

    assert_eq!(
        pending_undo_entry(),
        Some(merge_undo(vec![saved_as, numbered], Vec::new(), placed))
    );
}

#[test]
fn a_merge_undo_removes_a_fixed_name_the_destination_took_for_another_under_its_last_name() {
    // The merge planned `a_b` for `a:b`; the folder then took `a_b` for an
    // entry it already held, so the copy was saved as `a_b (1)`.
    let fixed = renamed_in_folder("a:b", "a_b", RenameCause::Invalid);
    let alias = renamed_in_folder("a_b", "a_b (1)", RenameCause::Alias);
    for renamed in [
        vec![fixed.clone(), alias.clone()],
        vec![alias.clone(), fixed.clone()],
    ] {
        assert_a_fixed_name_taken_for_another_is_undone_under_its_last_name(renamed);
    }
}

#[test]
fn a_paste_mixing_plain_copies_and_a_merge_records_one_merge_undo() {
    let browser = operating_browser();
    let copied = Location::local("/fixture/archive/report.txt");
    let merged_created = Location::local("/fixture/archive/folder/incoming.txt");
    let merged_overwritten = Location::local("/fixture/archive/folder/shared.txt");
    let (report, folder) = (
        Location::local("/fixture/report.txt"),
        Location::local("/fixture/folder"),
    );
    let (request_id, emit) = start_copy_into_archive(&browser);

    emit(transfer_progress(request_id, &copied));
    let placed = placed_at(&[&copied]);
    emit(merge_placed(request_id, &report, placed.clone()));
    emit(merged(
        request_id,
        &folder,
        vec![merged_created.clone()],
        vec![merged_overwritten.clone()],
    ));
    emit(pasted(request_id, vec![report, folder]));

    assert_eq!(
        pending_undo_entry(),
        Some(merge_undo(
            vec![copied, merged_created],
            vec![merged_overwritten],
            placed,
        ))
    );
}

#[test]
fn a_cancelled_merge_still_records_the_staged_originals_for_undo() {
    let browser = operating_browser();
    let source = Location::local("/fixture/folder");
    let overwritten = Location::local("/fixture/archive/folder/shared.txt");
    let (request_id, emit) = start_copy_into_archive(&browser);

    emit(merged(
        request_id,
        &source,
        Vec::new(),
        vec![overwritten.clone()],
    ));
    emit(OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation {
            completed: Vec::new(),
            failed: vec![source.clone()],
            not_attempted: vec![Location::local("/fixture/second")],
            affected_locations: HashSet::new(),
        },
    });

    assert_eq!(
        pending_undo_entry(),
        Some(merge_undo(Vec::new(), vec![overwritten], HashMap::new())),
        "originals staged before the cancelled copy still need restoring"
    );
}

#[test]
fn a_partial_merge_undo_keeps_the_paths_still_to_revert() {
    let first = Location::local("/fixture/archive/first.txt");
    let second = Location::local("/fixture/archive/second.txt");
    let originals = HashMap::from([(second.clone(), folder_identity(42, 1))]);
    let placed = placed_at(&[&second]);

    fail_replay_after(
        UndoEntry::Merge {
            created: vec![first.clone()],
            overwritten: vec![second.clone()],
            originals: originals.clone(),
            placed: placed.clone(),
        },
        &first,
    );

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Merge {
            created: Vec::new(),
            overwritten: vec![second],
            originals,
            placed,
        })
    );
}

#[test]
fn a_retried_merge_undo_keeps_the_records_of_what_it_created() {
    let created = Location::local("/fixture/archive/created.txt");
    let entry = merge_undo(vec![created.clone()], Vec::new(), placed_at(&[&created]));
    push_pending_undo(entry.clone());
    let (generation, _) = claim_pending_undo(None).expect("undo claim");

    retain_replay_merge_items(false, generation, std::slice::from_ref(&created), &[]);
    finish_undo(generation, false);

    assert_eq!(pending_undo_entry(), Some(entry));
}

#[test]
fn a_merge_undo_leaves_what_it_kept_out_of_undo_history() {
    let kept = Location::local("/fixture/archive/kept.txt");
    let cut = Location::local("/fixture/archive/cut.txt");
    let earlier = trash_undo(vec![Location::local("/fixture/earlier.txt")]);
    let rows = [
        (false, vec![kept.clone()], earlier),
        (
            true,
            vec![kept, cut.clone()],
            merge_undo(vec![cut], Vec::new(), HashMap::new()),
        ),
    ];
    for (cancelled, created, expected) in rows {
        assert_a_kept_item_stays_out_of_the_merge_undo(cancelled, created, expected);
    }
}

#[test]
fn a_replaced_copy_records_the_overwritten_original_for_undo() {
    let browser = operating_browser();
    forget_requests(&UNDO_MERGE_REQUESTS);
    let source = Location::local("/fixture/report.txt");
    let target = Location::local("/fixture/archive/report.txt");
    let (request_id, emit) = start_copy_into_archive(&browser);

    // A replace reports its target through transfer progress like a plain
    // copy, then through Merged once the original is staged in Trash.
    emit(transfer_progress(request_id, &target));
    let placed = placed_at(&[&target]);
    emit(OperationEvent::Merged {
        request_id,
        source: source.clone(),
        created: Vec::new(),
        overwritten: vec![target.clone()],
        placed: placed.clone(),
    });
    emit(pasted(request_id, vec![source]));

    assert_eq!(
        pending_undo_entry(),
        Some(merge_undo(Vec::new(), vec![target], placed.clone())),
        "the replaced path must undo through the restore path, not be trashed"
    );
    assert_the_pending_merge_undo_reverts(&browser, placed);
}
