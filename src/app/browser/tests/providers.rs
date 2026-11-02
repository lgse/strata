// SPDX-License-Identifier: MIT

use super::*;

pub(super) type UndoMergeRecord = (
    Vec<Location>,
    Vec<Location>,
    HashMap<Location, ItemIdentity>,
    HashMap<Location, crate::services::PlacedSnapshot>,
);

thread_local! {
    pub(super) static UNDO_MOVE_REQUESTS: RefCell<Vec<Vec<MoveRecord>>> = const { RefCell::new(Vec::new()) };
    pub(super) static UNDO_MOVE_CLEANUPS: RefCell<Vec<Vec<Location>>> = const { RefCell::new(Vec::new()) };
    pub(super) static UNDO_MOVE_NAMES: RefCell<Vec<NameMode>> = const { RefCell::new(Vec::new()) };
    pub(super) static UNDO_COPY_REQUESTS: RefCell<Vec<Vec<Location>>> = const { RefCell::new(Vec::new()) };
    pub(super) static UNDO_MERGE_REQUESTS: RefCell<Vec<UndoMergeRecord>> =
        const { RefCell::new(Vec::new()) };
    pub(super) static UNDO_RENAME_REQUESTS: RefCell<Vec<(Location, Location)>> = const { RefCell::new(Vec::new()) };
    pub(super) static TRASHED_IDENTITIES: RefCell<HashMap<Location, ItemIdentity>> = RefCell::default();
    pub(super) static FORWARD_RENAME_OUTCOME: Cell<Option<ForwardRenameOutcome>> = const { Cell::new(None) };
    pub(super) static SWEPT_PROBES: RefCell<Vec<(Location, Vec<OsString>)>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Copy)]
pub(super) enum ForwardRenameOutcome {
    Failed,
    Cancelled,
}

pub(super) struct ImmediateOperationProvider;

pub(super) fn report_trashed(
    request_id: OperationRequestId,
    locations: &[Location],
    emit: &Rc<dyn Fn(OperationEvent)>,
) {
    for location in locations {
        let identity = TRASHED_IDENTITIES.with(|trashed| trashed.borrow().get(location).copied());
        if let Some(identity) = identity {
            emit(OperationEvent::ItemTrashed {
                request_id,
                location: location.clone(),
                identity,
            });
        }
    }
}

fn created_event(
    request_id: OperationRequestId,
    parent: &Location,
    name: &str,
    unique_name: bool,
) -> OperationEvent {
    if !unique_name {
        return OperationEvent::Created { request_id };
    }
    let child = crate::adapters::gio_file_for_location(parent).child(name);
    OperationEvent::EntryCreated {
        request_id,
        location: crate::adapters::location_for_file(&child).expect("created location"),
    }
}

/// The destination a moved item lands on, when the move actually relocates it.
fn move_target(request: &PasteRequest, source: &Location) -> Option<Location> {
    if !request.move_sources {
        return None;
    }
    source
        .transfer_target(&request.destination)
        .filter(|target| target != source)
}

impl OperationProvider for ImmediateOperationProvider {
    fn rename(&self, request: RenameRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        match FORWARD_RENAME_OUTCOME.with(Cell::take) {
            Some(ForwardRenameOutcome::Failed) => emit(OperationEvent::Failed {
                request_id: request.id,
                message: "rename failed".to_owned(),
                password_failure: None,
            }),
            Some(ForwardRenameOutcome::Cancelled) => emit(OperationEvent::Cancelled {
                request_id: request.id,
                result: Default::default(),
            }),
            None => emit(OperationEvent::Renamed {
                request_id: request.id,
            }),
        }
        LoadHandle::new(|| {})
    }

    fn place_entry(
        &self,
        request: PlaceEntryRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        let mut folder = request.entry.location.parent().expect("new entry parent");
        let placed = request
            .path
            .folders
            .iter()
            .chain([&request.path.name])
            .map(|name| {
                folder = folder.child(OsStr::new(name)).expect("placed location");
                folder.clone()
            })
            .collect();
        emit(OperationEvent::EntryPlaced {
            request_id: request.id,
            placed,
        });
        LoadHandle::new(|| {})
    }

    fn create_directory(
        &self,
        request: CreateDirectoryRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        emit(created_event(
            request.id,
            &request.parent,
            &request.name,
            request.unique_name,
        ));
        LoadHandle::new(|| {})
    }

    fn create_file(
        &self,
        request: CreateFileRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        emit(created_event(
            request.id,
            &request.parent,
            &request.name,
            request.unique_name,
        ));
        LoadHandle::new(|| {})
    }

    fn paste(&self, request: PasteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        for (index, item) in request.items.iter().enumerate() {
            emit(OperationEvent::TransferProgress {
                request_id: request.id,
                completed_items: index + 1,
                completed_files: index + 1,
                total_files: None,
                current_file: None,
                transferred_bytes: 0,
                total_bytes: None,
                created_location: (!request.move_sources)
                    .then(|| item.source.transfer_target(&request.destination))
                    .flatten(),
            });
            if let Some(target) = move_target(&request, &item.source) {
                emit(OperationEvent::ItemMoved {
                    request_id: request.id,
                    from: item.source.clone(),
                    to: target,
                    merged: item.conflict == TransferConflict::Merge,
                });
            }
        }
        emit(OperationEvent::Pasted {
            request_id: request.id,
            locations: request.items.into_iter().map(|item| item.source).collect(),
        });
        LoadHandle::new(|| {})
    }

    fn undo_move(&self, request: UndoMoveRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        UNDO_MOVE_NAMES.with(|names| names.borrow_mut().push(request.names));
        UNDO_MOVE_CLEANUPS.with(|requests| {
            requests
                .borrow_mut()
                .push(request.cleanup_locations.clone())
        });
        UNDO_MOVE_REQUESTS.with(|requests| {
            requests.borrow_mut().push(
                request
                    .items
                    .iter()
                    .map(|item| item.record.clone())
                    .collect(),
            )
        });
        for item in &request.items {
            emit(OperationEvent::ItemMoved {
                request_id: request.id,
                from: item.record.current.clone(),
                to: item.record.original.clone(),
                merged: false,
            });
        }
        emit(OperationEvent::Pasted {
            request_id: request.id,
            locations: request
                .items
                .into_iter()
                .map(|item| item.record.current)
                .collect(),
        });
        LoadHandle::new(|| {})
    }

    fn undo_rename(
        &self,
        request: UndoRenameRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        UNDO_RENAME_REQUESTS.with(|requests| {
            requests
                .borrow_mut()
                .push((request.current.clone(), request.original.clone()))
        });
        emit(OperationEvent::Renamed {
            request_id: request.id,
        });
        LoadHandle::new(|| {})
    }

    fn undo_copy(&self, request: UndoCopyRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        UNDO_COPY_REQUESTS.with(|requests| requests.borrow_mut().push(request.locations.clone()));
        report_trashed(request.id, &request.locations, &emit);
        emit(OperationEvent::Deleted {
            request_id: request.id,
            locations: request.locations,
        });
        LoadHandle::new(|| {})
    }

    fn undo_merge(
        &self,
        request: UndoMergeRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        UNDO_MERGE_REQUESTS.with(|requests| {
            requests.borrow_mut().push((
                request.created.clone(),
                request.overwritten.clone(),
                request.originals.clone(),
                request.placed.clone(),
            ));
        });
        emit(OperationEvent::Restored {
            request_id: request.id,
            locations: Vec::new(),
            restored: Vec::new(),
        });
        LoadHandle::new(|| {})
    }

    fn delete(&self, request: DeleteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let locations: Vec<_> = request
            .entries
            .into_iter()
            .map(|entry| entry.location)
            .collect();
        report_trashed(request.id, &locations, &emit);
        emit(OperationEvent::Deleted {
            request_id: request.id,
            locations,
        });
        LoadHandle::new(|| {})
    }

    fn restore(&self, request: RestoreRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let restored = match &request.source {
            RestoreSource::TrashEntries(items) => items
                .iter()
                .map(|item| Location::local(item.destination.clone()))
                .collect(),
            RestoreSource::OriginalLocations { locations, .. } => locations.clone(),
        };
        emit(OperationEvent::Restored {
            request_id: request.id,
            locations: Vec::new(),
            restored,
        });
        LoadHandle::new(|| {})
    }

    fn recover(
        &self,
        request: crate::services::RecoverRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        emit(OperationEvent::Restored {
            request_id: request.id,
            locations: vec![request.container],
            restored: Vec::new(),
        });
        LoadHandle::new(|| {})
    }

    fn sweep_probes(&self, folder: &Location, names: &[OsString]) {
        SWEPT_PROBES.with_borrow_mut(|swept| swept.push((folder.clone(), names.to_vec())));
    }

    fn compress(&self, request: CompressRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let archive = request
            .destination
            .child(std::ffi::OsStr::new(&request.archive_name))
            .unwrap_or_else(|| request.destination.clone());
        emit(OperationEvent::Compressed {
            request_id: request.id,
            archive_name: request.archive_name,
            archive,
            original: None,
            placed: None,
        });
        LoadHandle::new(|| {})
    }

    fn extract(&self, request: ExtractRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        emit(OperationEvent::Extracted {
            request_id: request.id,
            first_name: None,
            created: Vec::new(),
        });
        LoadHandle::new(|| {})
    }
}

pub(super) type OperationEmit = Rc<dyn Fn(OperationEvent)>;

pub(super) struct HeldExtractProvider {
    pub(super) cancelled: Rc<Cell<bool>>,
    pub(super) emit: Rc<RefCell<Option<OperationEmit>>>,
    pub(super) request_id: Rc<Cell<Option<OperationRequestId>>>,
}

impl OperationProvider for HeldExtractProvider {
    fn rename(&self, request: RenameRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        ImmediateOperationProvider.rename(request, emit)
    }

    fn place_entry(
        &self,
        request: PlaceEntryRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        ImmediateOperationProvider.place_entry(request, emit)
    }

    fn create_directory(
        &self,
        request: CreateDirectoryRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        ImmediateOperationProvider.create_directory(request, emit)
    }

    fn create_file(
        &self,
        request: CreateFileRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        ImmediateOperationProvider.create_file(request, emit)
    }

    fn paste(&self, request: PasteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        ImmediateOperationProvider.paste(request, emit)
    }

    fn undo_move(&self, request: UndoMoveRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        ImmediateOperationProvider.undo_move(request, emit)
    }

    fn undo_rename(
        &self,
        request: UndoRenameRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        ImmediateOperationProvider.undo_rename(request, emit)
    }

    fn undo_copy(&self, request: UndoCopyRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        ImmediateOperationProvider.undo_copy(request, emit)
    }

    fn undo_merge(
        &self,
        request: UndoMergeRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        ImmediateOperationProvider.undo_merge(request, emit)
    }

    fn delete(&self, request: DeleteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        ImmediateOperationProvider.delete(request, emit)
    }

    fn restore(&self, request: RestoreRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        ImmediateOperationProvider.restore(request, emit)
    }

    fn recover(
        &self,
        request: crate::services::RecoverRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        ImmediateOperationProvider.recover(request, emit)
    }

    fn compress(&self, request: CompressRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        ImmediateOperationProvider.compress(request, emit)
    }

    fn extract(&self, request: ExtractRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        self.request_id.set(Some(request.id));
        self.emit.replace(Some(emit));
        let cancelled = self.cancelled.clone();
        LoadHandle::new(move || cancelled.set(true))
    }
}
