// SPDX-License-Identifier: MIT

use super::*;

type FindAnswer = (
    Location,
    ItemIdentity,
    Rc<dyn Fn(Option<Location>)>,
    Rc<Cell<bool>>,
);

/// Watches every column like the local adapter's directory monitor, on a fake
/// filesystem where folders keep their identity through a rename.
#[derive(Default)]
pub(super) struct DepartingSource {
    /// Holds parent lookups until `answer_finds`, like the adapter's worker thread.
    pub(super) defer_finds: bool,
    pub(super) pending_finds: RefCell<Vec<FindAnswer>>,
    pub(super) boundary: Option<Location>,
    pub(super) missing: RefCell<HashSet<Location>>,
    pub(super) unreachable: RefCell<HashSet<Location>>,
    pub(super) unlistable: RefCell<HashSet<Location>>,
    pub(super) identities: FolderIdentities,
    pub(super) watches: RefCell<Vec<(Location, WatchCallback)>>,
}

impl DepartingSource {
    pub(super) fn directory_monitor(&self, location: &Location) -> WatchCallback {
        self.watches
            .borrow()
            .iter()
            .rev()
            .find(|(watched, _)| watched == location)
            .map(|(_, notify)| notify.clone())
            .expect("the directory monitor should be installed")
    }

    pub(super) fn rename(&self, from: &Location, to: &Location) {
        self.identities.rename(from, to);
        self.missing.borrow_mut().insert(from.clone());
    }

    fn find(&self, parent: &Location, identity: ItemIdentity) -> Option<Location> {
        let listable = !self.unlistable.borrow().contains(parent);
        listable
            .then(|| self.identities.find(parent, identity))
            .flatten()
    }

    pub(super) fn answer_finds(&self) {
        let pending = self.pending_finds.take();
        for (parent, identity, emit, live) in pending {
            if live.get() {
                emit(self.find(&parent, identity));
            }
        }
    }
}

impl FileSource for DepartingSource {
    fn allows_navigation(&self, location: &Location) -> bool {
        self.boundary
            .as_ref()
            .is_none_or(|boundary| location.is_within(boundary))
    }

    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        if self.missing.borrow().contains(location) {
            Err(LocationValidationError::Missing)
        } else if self.unreachable.borrow().contains(location) {
            Err(LocationValidationError::Unavailable("offline".into()))
        } else {
            Ok(())
        }
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        if self.missing.borrow().contains(&request.location) {
            emit(DirectoryEvent::Failed {
                request_id: request.id,
                message: "No such file or folder".into(),
                refused: false,
                denied: false,
            });
        } else {
            self.identities.stamp(&request, &emit);
            emit(DirectoryEvent::Finished {
                request_id: request.id,
                truncated: false,
                can_trash: None,
                can_delete: None,
            });
        }
        LoadHandle::new(|| {})
    }

    fn watch(
        &self,
        location: Location,
        _include_hidden: bool,
        notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        self.watches.borrow_mut().push((location, notify));
        Some(LoadHandle::new(|| {}))
    }

    fn find_by_identity(
        &self,
        parent: Location,
        identity: ItemIdentity,
        emit: Rc<dyn Fn(Option<Location>)>,
    ) -> LoadHandle {
        if !self.defer_finds {
            emit(self.find(&parent, identity));
            return LoadHandle::new(|| {});
        }
        let live = Rc::new(Cell::new(true));
        self.pending_finds
            .borrow_mut()
            .push((parent, identity, emit, live.clone()));
        LoadHandle::new(move || live.set(false))
    }
}
