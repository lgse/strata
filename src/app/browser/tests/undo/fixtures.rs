// SPDX-License-Identifier: MIT

use super::*;
use std::thread::LocalKey;

pub(super) type RecordedEvents = Rc<RefCell<Vec<BrowserEvent>>>;

pub(super) fn operating_browser() -> Rc<Browser> {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    browser
}

pub(super) fn record_events(browser: &Rc<Browser>) -> RecordedEvents {
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    events
}

pub(super) fn requests_in<T: Clone>(requests: &'static LocalKey<RefCell<Vec<T>>>) -> Vec<T> {
    requests.with(|requests| requests.borrow().clone())
}

pub(super) fn forget_requests<T>(requests: &'static LocalKey<RefCell<Vec<T>>>) {
    requests.with(|requests| requests.borrow_mut().clear());
}

pub(super) fn trash_report(browser: &Rc<Browser>) -> Location {
    browser.delete(vec![fixture_entry("/fixture/report.txt")], false);
    Location::local("/fixture/report.txt")
}

pub(super) fn transfer_one(browser: &Rc<Browser>, destination: &str, source: &str, moving: bool) {
    browser.transfer(
        Location::local(destination),
        vec![PasteItem {
            source: Location::local(source),
            conflict: TransferConflict::FailIfExists,
            target_name: None,
        }],
        moving,
        true,
    );
}

pub(super) fn move_record(original: &str, current: &str) -> MoveRecord {
    MoveRecord {
        original: Location::local(original),
        current: Location::local(current),
        renamed: Vec::new(),
        replaced: None,
    }
}

pub(super) fn undo_move_items(records: impl IntoIterator<Item = MoveRecord>) -> Vec<UndoMoveItem> {
    records
        .into_iter()
        .map(|record| UndoMoveItem {
            record,
            conflict: TransferConflict::FailIfExists,
        })
        .collect()
}

pub(super) fn undo_pending_copy(browser: &Rc<Browser>) -> bool {
    let (generation, locations) = browser.pending_undo_copy().expect("pending copy undo");
    browser.undo_copy(generation, locations)
}

pub(super) fn undo_pending_move(browser: &Rc<Browser>) -> bool {
    let (generation, records) = browser.pending_undo_move().expect("pending move undo");
    browser.undo_move(generation, undo_move_items(records))
}

pub(super) fn undo_pending_rename(browser: &Rc<Browser>) -> bool {
    let (generation, _, _) = browser.pending_undo_rename().expect("pending Rename undo");
    browser.undo_rename(generation)
}

pub(super) fn rename_undo_entry() -> UndoEntry {
    let entry = fixture_entry("/fixture/original.txt");
    UndoEntry::Rename(RenameRecord {
        original: entry.location.clone(),
        current: Location::local("/fixture/renamed.txt"),
        native_name: entry.native_name,
        display_name: entry.display_name,
        is_hidden: entry.is_hidden,
    })
}

pub(super) fn merge_undo(
    created: Vec<Location>,
    overwritten: Vec<Location>,
    placed: HashMap<Location, PlacedSnapshot>,
) -> UndoEntry {
    UndoEntry::Merge {
        created,
        overwritten,
        originals: HashMap::new(),
        placed,
    }
}

/// How the operation a test drives was started, set before its callback exists.
pub(super) enum OperationSetup {
    Plain,
    Copy(Location),
    Move(Location),
    MoveBack,
    Restoration,
    Archive,
}

pub(super) type StartedOperation = (OperationRequestId, Rc<dyn Fn(OperationEvent)>);

fn start(
    browser: &Rc<Browser>,
    setup: OperationSetup,
    claim: Option<(u64, UndoEntry)>,
) -> StartedOperation {
    let request_id = browser.begin_operation();
    match setup {
        OperationSetup::Plain => {}
        OperationSetup::Copy(destination) => {
            browser.transfer_operation.set(Some(false));
            browser.transfer_destination.replace(Some(destination));
        }
        OperationSetup::Move(destination) => {
            browser.transfer_operation.set(Some(true));
            browser.transfer_destination.replace(Some(destination));
        }
        OperationSetup::MoveBack => browser.transfer_operation.set(Some(true)),
        OperationSetup::Restoration => browser.restoration_operation.set(true),
        OperationSetup::Archive => browser.archive_operation.set(true),
    }
    if let Some(claim) = claim {
        browser.undo_claim.replace(Some(claim));
    }
    let emit = browser.operation_callback(request_id, false, HashSet::new());
    (request_id, emit)
}

pub(super) fn start_operation(browser: &Rc<Browser>, setup: OperationSetup) -> StartedOperation {
    start(browser, setup, None)
}

/// Pushes `entry`, claims it, and starts the operation that replays it.
pub(super) fn start_replay(
    browser: &Rc<Browser>,
    entry: UndoEntry,
    setup: OperationSetup,
) -> StartedOperation {
    push_pending_undo(entry);
    let claim = claim_pending_undo(None).expect("undo claim");
    start(browser, setup, Some(claim))
}

/// Claims `entry`, completes only `completed`, and finishes the replay as failed.
pub(super) fn fail_replay_after(entry: UndoEntry, completed: &Location) {
    push_pending_undo(entry);
    let (generation, _) = claim_pending_undo(None).expect("undo claim");
    mark_replay_item_completed(false, generation, completed);
    finish_undo(generation, false);
}

pub(super) fn transfer_progress(
    request_id: OperationRequestId,
    created: &Location,
) -> OperationEvent {
    OperationEvent::TransferProgress {
        request_id,
        completed_items: 1,
        completed_files: 1,
        total_files: None,
        current_file: None,
        transferred_bytes: 0,
        total_bytes: None,
        created_location: Some(created.clone()),
    }
}

pub(super) fn pasted(request_id: OperationRequestId, locations: Vec<Location>) -> OperationEvent {
    OperationEvent::Pasted {
        request_id,
        locations,
    }
}

pub(super) fn merged(
    request_id: OperationRequestId,
    source: &Location,
    created: Vec<Location>,
    overwritten: Vec<Location>,
) -> OperationEvent {
    OperationEvent::Merged {
        request_id,
        source: source.clone(),
        created,
        overwritten,
        placed: HashMap::new(),
    }
}

pub(super) fn merge_placed(
    request_id: OperationRequestId,
    source: &Location,
    placed: HashMap<Location, PlacedSnapshot>,
) -> OperationEvent {
    OperationEvent::Merged {
        request_id,
        source: source.clone(),
        created: Vec::new(),
        overwritten: Vec::new(),
        placed,
    }
}

pub(super) fn placed_at(locations: &[&Location]) -> HashMap<Location, PlacedSnapshot> {
    locations
        .iter()
        .map(|location| ((*location).clone(), PlacedSnapshot::new(())))
        .collect()
}

pub(super) fn failed(request_id: OperationRequestId, message: &str) -> OperationEvent {
    OperationEvent::Failed {
        request_id,
        message: message.to_owned(),
        password_failure: None,
    }
}
