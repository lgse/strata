// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::services::*;

type Callback = Rc<dyn Fn(OperationEvent)>;

pub(crate) fn entry(location: crate::model::Location) -> crate::model::FileEntry {
    use crate::model::{EntryKind, FileEntry, MetadataValue};
    let name = location.display_name();
    FileEntry {
        location,
        native_name: name.clone().into(),
        thumbnail_path: None,
        display_name: name,
        kind: EntryKind::File,
        size: MetadataValue::Known(100),
        modified_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        recent_uri: None,
    }
}

pub(crate) fn operation_event(event: &crate::app::BrowserEvent) -> &crate::app::BrowserEvent {
    match event {
        crate::app::BrowserEvent::BackgroundOperation { event, .. } => event,
        event => event,
    }
}

type BeforeReturn = Rc<dyn Fn(OperationRequestId)>;

#[derive(Default)]
pub(crate) struct HeldOperations {
    callbacks: RefCell<HashMap<OperationRequestId, Callback>>,
    cancelled: Rc<RefCell<HashSet<OperationRequestId>>>,
    pub(crate) before_return: RefCell<Option<BeforeReturn>>,
    /// Every held delete as `(id, permanent, entry count)`, in request order.
    pub(crate) delete_requests: RefCell<Vec<(OperationRequestId, bool, usize)>>,
}

impl HeldOperations {
    pub(crate) fn emit(&self, request_id: OperationRequestId, event: OperationEvent) {
        let callback = self
            .callbacks
            .borrow()
            .get(&request_id)
            .cloned()
            .expect("held operation");
        callback(event);
    }
    pub(crate) fn cancelled(&self, request_id: OperationRequestId) -> bool {
        self.cancelled.borrow().contains(&request_id)
    }
    fn hold(&self, id: OperationRequestId, callback: Callback) -> LoadHandle {
        self.callbacks.borrow_mut().insert(id, callback);
        let cancelled = self.cancelled.clone();
        let load = LoadHandle::new(move || {
            cancelled.borrow_mut().insert(id);
        });
        let hook = self.before_return.borrow().clone();
        if let Some(hook) = hook {
            hook(id);
        }
        load
    }
}

macro_rules! unsupported {
    ($method:ident, $request:ty) => {
        fn $method(&self, _: $request, _: Callback) -> LoadHandle {
            panic!(concat!("unexpected held operation: ", stringify!($method)))
        }
    };
}

impl OperationProvider for HeldOperations {
    fn paste(&self, request: PasteRequest, callback: Callback) -> LoadHandle {
        self.hold(request.id, callback)
    }
    fn compress(&self, request: CompressRequest, callback: Callback) -> LoadHandle {
        let load = self.hold(request.id, callback.clone());
        callback(OperationEvent::ArchiveStarted {
            request_id: request.id,
            total: request.entries.len(),
        });
        load
    }
    fn extract(&self, request: ExtractRequest, callback: Callback) -> LoadHandle {
        let load = self.hold(request.id, callback.clone());
        callback(OperationEvent::ArchiveStarted {
            request_id: request.id,
            total: 1,
        });
        load
    }
    fn rename(&self, request: RenameRequest, callback: Callback) -> LoadHandle {
        self.hold(request.id, callback)
    }
    unsupported!(create_directory, CreateDirectoryRequest);
    unsupported!(create_file, CreateFileRequest);
    unsupported!(undo_move, UndoMoveRequest);
    unsupported!(undo_rename, UndoRenameRequest);
    unsupported!(undo_copy, UndoCopyRequest);
    unsupported!(undo_merge, UndoMergeRequest);
    fn delete(&self, request: DeleteRequest, callback: Callback) -> LoadHandle {
        self.delete_requests.borrow_mut().push((
            request.id,
            request.permanent,
            request.entries.len(),
        ));
        self.hold(request.id, callback)
    }
    unsupported!(restore, RestoreRequest);
}
