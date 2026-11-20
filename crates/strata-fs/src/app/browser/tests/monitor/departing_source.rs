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

    /// Renames `from` with everything in it, which leaves every folder that it held,
    /// and that the identities know, missing at its old location.
    pub(super) fn rename_tree(&self, from: &Location, to: &Location) {
        let moved = self.identities.rename_tree(from, to);
        let mut missing = self.missing.borrow_mut();
        missing.insert(from.clone());
        for folder in moved {
            missing.extend(
                std::iter::successors(Some(folder), Location::parent)
                    .take_while(|folder| folder.is_within(from)),
            );
        }
    }

    fn find(&self, departed: &Location, identity: ItemIdentity) -> Option<Location> {
        let missing = |location: &Location| self.missing.borrow().contains(location);
        let searched = std::iter::successors(departed.parent(), Location::parent)
            .find(|ancestor| !missing(ancestor))?;
        let listable = !self.unlistable.borrow().contains(&searched);
        listable
            .then(|| self.identities.find(departed, identity, missing))
            .flatten()
    }

    pub(super) fn answer_finds(&self) {
        let pending = self.pending_finds.take();
        for (departed, identity, emit, live) in pending {
            if live.get() {
                emit(self.find(&departed, identity));
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
        departed: Location,
        identity: ItemIdentity,
        emit: Rc<dyn Fn(Option<Location>)>,
    ) -> LoadHandle {
        if !self.defer_finds {
            emit(self.find(&departed, identity));
            return LoadHandle::new(|| {});
        }
        let live = Rc::new(Cell::new(true));
        self.pending_finds
            .borrow_mut()
            .push((departed, identity, emit, live.clone()));
        LoadHandle::new(move || live.set(false))
    }
}
