// SPDX-License-Identifier: MIT

use super::*;

/// The undo recorded once an extraction reports `created`, named by `first_name`.
fn extraction_undo(first_name: Option<&str>, created: Vec<Location>) -> Option<UndoEntry> {
    let emit = Rc::new(RefCell::new(None));
    let request_id = Rc::new(Cell::new(None));
    let provider = Rc::new(HeldExtractProvider {
        cancelled: Rc::new(Cell::new(false)),
        emit: emit.clone(),
        request_id: request_id.clone(),
    });
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(provider);
    browser.extract(
        fixture_entry("/fixture/archive.zip"),
        Location::local("/fixture"),
        false,
        None,
    );

    let request_id = request_id.get().expect("extract request");
    let callback = emit.borrow().clone().expect("extract callback");
    callback(OperationEvent::Extracted {
        request_id,
        first_name: first_name.map(Into::into),
        created,
    });
    pending_undo_entry()
}

fn assert_a_replaced_archive_is_restored_on_undo(browser: &Rc<Browser>, cancel_requested: bool) {
    let archive = Location::local("/fixture/report.zip");
    let original = folder_identity(42, 1);
    forget_requests(&UNDO_MERGE_REQUESTS);
    let (request_id, emit) = start_operation(browser, OperationSetup::Archive);
    if cancel_requested {
        browser.cancel_file_operation();
    }

    let placed = PlacedSnapshot::new(());
    emit(OperationEvent::Compressed {
        request_id,
        archive_name: "report.zip".to_owned(),
        archive: archive.clone(),
        original: Some(ReplacedOriginal::Trashed(Some(original))),
        placed: Some(placed.clone()),
    });

    let (generation, created, overwritten) =
        browser.pending_undo_merge().expect("replacement undo");
    assert_eq!(
        (created.clone(), overwritten.clone()),
        (Vec::new(), vec![archive.clone()])
    );
    assert!(browser.undo_merge(generation, created, overwritten));
    assert_eq!(
        (requests_in(&UNDO_MERGE_REQUESTS), pending_undo_entry()),
        (
            vec![(
                Vec::new(),
                vec![archive.clone()],
                HashMap::from([(archive.clone(), original)]),
                HashMap::from([(archive, placed)]),
            )],
            None
        )
    );
}

#[test]
fn a_completed_compression_records_the_archive_for_undo() {
    let browser = operating_browser();

    browser.compress(
        vec![fixture_entry("/fixture/report.txt")],
        Location::local("/fixture"),
        "report.zip".to_owned(),
        TransferConflict::FailIfExists,
        ArchiveFormat::Zip,
        None,
    );

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Copy(vec![Location::local(
            "/fixture/report.zip"
        )]))
    );
}

#[test]
fn a_completed_extraction_records_created_entries_for_undo() {
    let created = vec![Location::local("/fixture/readme.txt")];

    assert_eq!(
        extraction_undo(Some("readme.txt"), created.clone()),
        Some(UndoEntry::Copy(created))
    );
}

#[test]
fn an_empty_extraction_records_no_undo() {
    assert_eq!(extraction_undo(None, Vec::new()), None);
}

#[test]
fn a_replaced_archive_restores_the_original_on_undo_even_after_a_cancel_request() {
    let browser = operating_browser();
    for cancel_requested in [false, true] {
        assert_a_replaced_archive_is_restored_on_undo(&browser, cancel_requested);
    }
}

#[test]
fn an_archive_that_replaced_its_original_without_trash_records_no_undo() {
    let browser = operating_browser();
    let (request_id, emit) = start_operation(&browser, OperationSetup::Archive);

    emit(OperationEvent::Compressed {
        request_id,
        archive_name: "report.zip".to_owned(),
        archive: Location::local("/fixture/report.zip"),
        original: Some(ReplacedOriginal::Deleted),
        placed: None,
    });

    assert_eq!(pending_undo_entry(), None);
}
