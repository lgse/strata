// SPDX-License-Identifier: MIT

//! A browser with a file operation running in the background: a delete the provider holds
//! and never finishes, sent to the background as the progress dock's button sends one.

use std::rc::Rc;

use strata_core::model::FileEntry;
use strata_fs::{
    app::Browser,
    services::{
        CompressRequest, CreateDirectoryRequest, CreateFileRequest, DeleteRequest, ExtractRequest,
        LoadHandle, OperationEvent, OperationProvider, PasteRequest, PlaceEntryRequest,
        RecoverRequest, RenameRequest, RestoreRequest, UndoCopyRequest, UndoMergeRequest,
        UndoMoveRequest, UndoRenameRequest,
    },
};

type Emit = Rc<dyn Fn(OperationEvent)>;

/// Holds every delete; nothing else is asked of it.
struct Held;

macro_rules! not_asked {
    ($($method:ident: $request:ty),* $(,)?) => {
        $(fn $method(&self, _: $request, _: Emit) -> LoadHandle {
            unreachable!(concat!("only a delete is held, not ", stringify!($method)))
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

/// `browser` deletes its first column's entry `name` in the background, and never finishes.
pub(super) fn busy(browser: &Rc<Browser>, name: &str) {
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
