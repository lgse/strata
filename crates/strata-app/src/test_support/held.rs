// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use strata_core::model::FileEntry;
use strata_fs::{
    app::Browser,
    services::{
        CompressRequest, CreateDirectoryRequest, CreateFileRequest, DeleteRequest, ExtractRequest,
        LoadHandle, OperationEvent, OperationProvider, OperationRequestId, PasteRequest,
        PlaceEntryRequest, RecoverRequest, RenameRequest, RestoreRequest, UndoCopyRequest,
        UndoMergeRequest, UndoMoveRequest, UndoRenameRequest,
    },
};

type Emit = Rc<dyn Fn(OperationEvent)>;

/// Holds every delete; nothing else is asked of it.
struct Held;

macro_rules! not_asked {
    ($($method:ident: $request:ty),* $(,)?) => {
        $(fn $method(&self, _: $request, _: Emit) -> LoadHandle {
            unreachable!(concat!("this provider does not hold ", stringify!($method)))
        })*
    };
}

impl OperationProvider for Held {
    fn delete(&self, _: DeleteRequest, _: Emit) -> LoadHandle {
        LoadHandle::new(|| {})
    }

    not_asked!(
        rename: RenameRequest,
        place_entry: PlaceEntryRequest,
        create_directory: CreateDirectoryRequest,
        create_file: CreateFileRequest,
        paste: PasteRequest,
        undo_move: UndoMoveRequest,
        undo_rename: UndoRenameRequest,
        undo_copy: UndoCopyRequest,
        undo_merge: UndoMergeRequest,
        restore: RestoreRequest,
        recover: RecoverRequest,
        compress: CompressRequest,
        extract: ExtractRequest,
    );
}

/// Holds every extraction until the test ends it; nothing else is asked of it.
#[derive(Default)]
pub(crate) struct HeldExtractions {
    asked: RefCell<Vec<(OperationRequestId, Emit)>>,
}

impl HeldExtractions {
    pub(crate) fn asked(&self) -> usize {
        self.asked.borrow().len()
    }

    /// Reports `event` for the extraction asked for last.
    pub(crate) fn report(&self, event: impl FnOnce(OperationRequestId) -> OperationEvent) {
        let (request_id, emit) = self.asked.borrow().last().cloned().expect("an extraction");
        emit(event(request_id));
    }
}

impl OperationProvider for HeldExtractions {
    fn extract(&self, request: ExtractRequest, emit: Emit) -> LoadHandle {
        self.asked.borrow_mut().push((request.id, emit));
        LoadHandle::new(|| {})
    }

    not_asked!(
        rename: RenameRequest,
        place_entry: PlaceEntryRequest,
        create_directory: CreateDirectoryRequest,
        create_file: CreateFileRequest,
        delete: DeleteRequest,
        paste: PasteRequest,
        undo_move: UndoMoveRequest,
        undo_rename: UndoRenameRequest,
        undo_copy: UndoCopyRequest,
        undo_merge: UndoMergeRequest,
        restore: RestoreRequest,
        recover: RecoverRequest,
        compress: CompressRequest,
    );
}

/// `browser` deletes its first column's entry `name` in the background, and never finishes.
pub(crate) fn busy(browser: &Rc<Browser>, name: &str) {
    let named = |entries: &[FileEntry]| {
        entries
            .iter()
            .find(|entry| entry.display_name == name)
            .cloned()
    };
    let entry = browser.with_column_entries(0, named).flatten();
    let entry = entry.expect("the entry is listed");
    browser.set_operation_provider(Rc::new(Held));
    browser.delete(vec![entry], true);
    let id = browser
        .backgroundable_operation()
        .expect("the delete started");
    assert!(browser.background_file_operation(id));
    assert!(browser.has_background_operations());
}
