// SPDX-License-Identifier: MIT

use super::*;
use crate::{services::TypedPath, test_support::operations::HeldOperations};

fn typed_path(path: &str, new_item: bool) -> TypedPath {
    TypedPath::parse(path, false, new_item).expect("path")
}

fn held_operations(browser: &Rc<Browser>) -> Rc<HeldOperations> {
    let operations = Rc::new(HeldOperations::default());
    browser.set_operation_provider(operations.clone());
    operations
}

#[test]
fn a_created_folder_can_be_undone_by_trashing_it() {
    let browser = operating_browser();
    forget_requests(&UNDO_COPY_REQUESTS);
    let folder = Location::local("/fixture/new folder");

    browser.create_new_folder(Location::local("/fixture"));

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Copy(vec![folder.clone()]))
    );
    assert!(undo_pending_copy(&browser));
    assert_eq!(
        (requests_in(&UNDO_COPY_REQUESTS), pending_undo_entry()),
        (vec![vec![folder]], None)
    );
}

#[test]
fn a_created_file_can_be_undone_by_trashing_it() {
    let browser = operating_browser();

    browser.create_new_file(Location::local("/fixture"));

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Copy(vec![Location::local("/fixture/new file")]))
    );
}

#[test]
fn naming_a_created_file_with_a_path_makes_undo_trash_the_outermost_new_folder() {
    let browser = operating_browser();
    forget_requests(&UNDO_COPY_REQUESTS);
    let events = record_events(&browser);
    browser.create_new_file(Location::local("/fixture"));
    let docs = Location::local("/fixture/docs");

    let request_id = browser.place_entry(
        fixture_entry("/fixture/new file"),
        typed_path("docs/today.txt", true),
        true,
    );

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::RenameCompleted { request_id: completed } if Some(*completed) == request_id
    )));
    let (generation, locations) = browser.pending_undo_copy().expect("pending copy undo");
    assert_eq!(locations, std::slice::from_ref(&docs));
    assert!(browser.undo_copy(generation, locations));
    assert_eq!(
        (requests_in(&UNDO_COPY_REQUESTS), pending_undo_entry()),
        (vec![vec![docs]], None),
        "the creation is undone only once"
    );
}

#[test]
fn naming_a_created_file_makes_one_undo_trash_it_under_its_new_name() {
    let browser = operating_browser();
    forget_requests(&UNDO_COPY_REQUESTS);
    browser.create_new_file(Location::local("/fixture"));
    let named = Location::local("/fixture/notes.txt");

    browser.name_new_item(fixture_entry("/fixture/new file"), "notes.txt".to_owned());

    let (generation, locations) = browser.pending_undo_copy().expect("pending copy undo");
    assert_eq!(locations, std::slice::from_ref(&named));
    assert!(browser.undo_copy(generation, locations));
    assert_eq!(
        (requests_in(&UNDO_COPY_REQUESTS), pending_undo_entry()),
        (vec![vec![named]], None),
        "the creation and its name are undone in one step"
    );
}

#[test]
fn a_failed_path_naming_leaves_the_created_file_to_undo() {
    let browser = operating_browser();
    browser.create_new_file(Location::local("/fixture"));
    let operations = held_operations(&browser);
    let events = record_events(&browser);

    let request_id = browser
        .place_entry(
            fixture_entry("/fixture/new file"),
            typed_path("todo.txt/today.txt", true),
            true,
        )
        .expect("placement started");
    operations.emit(request_id, failed(request_id, "“todo.txt” is not a folder"));

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::RenameFailed { request_id: Some(failed), .. } if *failed == request_id
    )));
    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Copy(vec![Location::local("/fixture/new file")]))
    );
}

#[test]
fn renaming_an_item_into_new_folders_undoes_as_a_move_back_that_trashes_them() {
    let browser = operating_browser();
    let events = record_events(&browser);
    let original = Location::local("/fixture/report.txt");
    let current = Location::local("/fixture/a/b/report.txt");

    browser.place_entry(
        fixture_entry("/fixture/report.txt"),
        typed_path("a/b/report.txt", false),
        false,
    );

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Group {
            folder: Location::local("/fixture/a"),
            records: vec![MoveRecord {
                original: original.clone(),
                current: current.clone(),
                renamed: Vec::new(),
                replaced: None,
            }],
        })
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::ItemRelocated { from, to, .. } if *from == original && *to == current
    )));
}

#[test]
fn renaming_an_item_into_an_existing_folder_undoes_as_a_move() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let operations = held_operations(&browser);
    let original = Location::local("/fixture/todo.txt");
    let current = Location::local("/fixture/kept/todo.txt");

    let request_id = browser
        .place_entry(
            fixture_entry("/fixture/todo.txt"),
            typed_path("kept/todo.txt", false),
            false,
        )
        .expect("placement started");
    operations.emit(
        request_id,
        OperationEvent::EntryPlaced {
            request_id,
            placed: vec![current.clone()],
        },
    );

    assert_eq!(
        pending_undo_entry(),
        Some(UndoEntry::Move(vec![MoveRecord {
            original,
            current,
            renamed: Vec::new(),
            replaced: None,
        }]))
    );
}
