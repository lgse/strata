// SPDX-License-Identifier: MIT

use super::*;
use crate::services::WatchRequest;

#[derive(Clone)]
struct Folder {
    answer: StampAnswer,
    names: Vec<String>,
    watching: Option<Watching>,
}

type Emit = Rc<dyn Fn(DirectoryEvent)>;
type Check = (Location, Rc<dyn Fn(Location, StampAnswer)>);
type Notify = Rc<dyn Fn(DirectoryChange)>;
type Validation = (
    Location,
    Rc<dyn Fn(Result<(), LocationValidationError>)>,
    Rc<Cell<bool>>,
);

#[derive(Default)]
pub(super) struct Folders {
    folders: RefCell<HashMap<Location, Folder>>,
    loads: RefCell<Vec<Location>>,
    checks: RefCell<Vec<Location>>,
    pub(super) hold_loads: Cell<bool>,
    held_loads: RefCell<Vec<(DirectoryRequest, Emit)>>,
    pub(super) queue_loads: Cell<bool>,
    queued_loads: RefCell<Vec<(DirectoryRequest, Emit)>>,
    pub(super) fail_loads: Cell<bool>,
    pub(super) truncate_loads: Cell<bool>,
    pub(super) responsiveness: Cell<Responsiveness>,
    pub(super) followers: RefCell<Vec<Rc<dyn Fn()>>>,
    pub(super) probes_run_out_of_time: Cell<bool>,
    pub(super) hold_checks: Cell<bool>,
    held_checks: RefCell<Vec<Check>>,
    monitors: RefCell<HashMap<Location, Notify>>,
    watch_requests: RefCell<Vec<Location>>,
    pub(super) validations: RefCell<Vec<Location>>,
    pub(super) hold_validations: Cell<bool>,
    held_validations: RefCell<Vec<Validation>>,
}

impl Folders {
    pub(super) fn add(
        &self,
        path: &str,
        stamp: DirectoryStamp,
        watching: Option<Watching>,
    ) -> Location {
        let location = Location::local(path);
        self.folders.borrow_mut().insert(
            location.clone(),
            Folder {
                answer: StampAnswer::Stamp(Box::new(stamp)),
                names: vec!["listed".to_owned()],
                watching,
            },
        );
        location
    }

    pub(super) fn change(&self, location: &Location, names: &[&str]) {
        let mut folders = self.folders.borrow_mut();
        let folder = folders.get_mut(location).expect("a known folder");
        if let StampAnswer::Stamp(stamp) = &mut folder.answer {
            stamp.modified.0 += 1;
            stamp.changed = stamp.changed.map(|(seconds, nanos)| (seconds + 1, nanos));
        }
        folder.names = names.iter().map(|name| (*name).to_owned()).collect();
    }

    pub(super) fn change_names_only(&self, location: &Location, names: &[&str]) {
        let mut folders = self.folders.borrow_mut();
        let folder = folders.get_mut(location).expect("a known folder");
        folder.names = names.iter().map(|name| (*name).to_owned()).collect();
    }

    pub(super) fn loads_of(&self, location: &Location) -> usize {
        self.loads
            .borrow()
            .iter()
            .filter(|loaded| *loaded == location)
            .count()
    }

    pub(super) fn checks_of(&self, location: &Location) -> usize {
        self.checks
            .borrow()
            .iter()
            .filter(|checked| *checked == location)
            .count()
    }

    pub(super) fn answer_of(&self, location: &Location) -> StampAnswer {
        self.folders
            .borrow()
            .get(location)
            .map_or(StampAnswer::Missing, |folder| folder.answer.clone())
    }

    fn begin_load(&self, request: &DirectoryRequest, emit: &Emit) -> bool {
        let folder = self.folders.borrow().get(&request.location).cloned();
        let request_id = request.id;
        let Some(Folder {
            answer: StampAnswer::Stamp(stamp),
            names,
            ..
        }) = folder.filter(|_| self.answering())
        else {
            emit(DirectoryEvent::Failed {
                request_id,
                message: "The operation timed out".to_owned(),
                refused: !self.answering(),
                denied: false,
            });
            return false;
        };
        emit(DirectoryEvent::Stamp { request_id, stamp });
        emit(DirectoryEvent::Batch {
            request_id,
            entries: names
                .iter()
                .map(|name| entry(&request.location, name))
                .collect(),
        });
        if !self.truncate_loads.get() {
            emit(DirectoryEvent::ListingDigest {
                request_id,
                names_digest: names
                    .iter()
                    .fold(0, |digest: u64, name| digest.wrapping_add(name_hash(name))),
            });
        }
        true
    }

    fn finish_load(&self, request: &DirectoryRequest, emit: &Emit) {
        let request_id = request.id;
        emit(match self.fail_loads.get() {
            true => DirectoryEvent::Failed {
                request_id,
                message: "A device input/output error occurred".to_owned(),
                refused: false,
                denied: false,
            },
            false => DirectoryEvent::Finished {
                request_id,
                truncated: self.truncate_loads.get(),
                can_trash: None,
                can_delete: None,
            },
        });
    }

    pub(super) fn refuse_queued_loads(&self) {
        let queued = self.queued_loads.take();
        for (request, emit) in queued {
            emit(DirectoryEvent::Failed {
                request_id: request.id,
                message: "The operation timed out".to_owned(),
                refused: true,
                denied: false,
            });
        }
    }

    pub(super) fn run_queued_loads(&self) {
        let queued = self.queued_loads.take();
        for (request, emit) in queued {
            if self.begin_load(&request, &emit) {
                self.finish_load(&request, &emit);
            }
        }
    }

    pub(super) fn finish_held_loads(&self) {
        let held = self.held_loads.take();
        for (request, emit) in held {
            self.finish_load(&request, &emit);
        }
    }

    pub(super) fn answer(&self, location: &Location, answer: StampAnswer) {
        let mut folders = self.folders.borrow_mut();
        folders.get_mut(location).expect("a known folder").answer = answer;
    }

    pub(super) fn remove(&self, location: &Location) {
        self.folders.borrow_mut().remove(location);
    }

    pub(super) fn replace(&self, location: &Location, inode: u64, names: &[&str]) {
        let mut folders = self.folders.borrow_mut();
        let folder = folders.get_mut(location).expect("a known folder");
        if let StampAnswer::Stamp(stamp) = &mut folder.answer
            && let Some(identity) = &mut stamp.identity
        {
            identity.evidence.inode = inode;
        }
        folder.names = names.iter().map(|name| (*name).to_owned()).collect();
    }

    pub(super) fn answer_held_checks_for(&self, folder: &Location, answer: &StampAnswer) {
        let held = self.held_checks.take();
        for (location, emit) in held {
            let answer = match location == *folder {
                true => answer.clone(),
                false => self.answer_of(&location),
            };
            emit(location, answer);
        }
    }

    pub(super) fn monitors_requested(&self, location: &Location) -> usize {
        self.watch_requests
            .borrow()
            .iter()
            .filter(|watched| *watched == location)
            .count()
    }

    pub(super) fn notify(&self, location: &Location, change: DirectoryChange) {
        let notify = self.monitors.borrow().get(location).cloned();
        notify.expect("a requested monitor")(change);
    }

    pub(super) fn answer_held_checks(&self) {
        let held = self.held_checks.take();
        for (location, emit) in held {
            emit(location.clone(), self.answer_of(&location));
        }
    }

    pub(super) fn answer_held_check(&self, folder: &Location) {
        let held = std::mem::take(&mut *self.held_checks.borrow_mut());
        let (answered, waiting): (Vec<_>, Vec<_>) = held
            .into_iter()
            .partition(|(location, _)| location == folder);
        self.held_checks.replace(waiting);
        for (location, emit) in answered {
            emit(location.clone(), self.answer_of(&location));
        }
    }

    pub(super) fn answer_held_validations(&self) {
        let held = self.held_validations.take();
        for (location, emit, cancelled) in held {
            if !cancelled.get() {
                emit(self.validate_location(&location));
            }
        }
    }

    fn answering(&self) -> bool {
        self.responsiveness.get() == Responsiveness::Answering
    }

    fn mount_of(&self, location: &Location) -> Option<MountToken> {
        match self.answer_of(location) {
            StampAnswer::Stamp(stamp) => stamp.mount,
            _ => None,
        }
    }

    fn lane_busy(&self, location: &Location) -> bool {
        let Some(mount) = self.mount_of(location) else {
            return false;
        };
        self.background_load_on(&mount) || self.check_on(&mount)
    }

    fn background_load_on(&self, mount: &MountToken) -> bool {
        self.held_loads.borrow().iter().any(|(request, _)| {
            request.background && self.mount_of(&request.location).as_ref() == Some(mount)
        })
    }

    fn check_on(&self, mount: &MountToken) -> bool {
        self.held_checks
            .borrow()
            .iter()
            .any(|(checked, _)| self.mount_of(checked).as_ref() == Some(mount))
    }
}

impl FileSource for Folders {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        match self.answer_of(location) {
            StampAnswer::Stamp(_) => Ok(()),
            _ => Err(LocationValidationError::Missing),
        }
    }

    fn validate_location_async(
        &self,
        location: Location,
        emit: Rc<dyn Fn(Result<(), LocationValidationError>)>,
    ) -> LoadHandle {
        self.validations.borrow_mut().push(location.clone());
        if !self.hold_validations.get() {
            emit(self.validate_location(&location));
            return LoadHandle::new(|| {});
        }
        let cancelled = Rc::new(Cell::new(false));
        self.held_validations
            .borrow_mut()
            .push((location, emit, cancelled.clone()));
        LoadHandle::new(move || cancelled.set(true))
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Emit) -> LoadHandle {
        self.loads.borrow_mut().push(request.location.clone());
        if self.queue_loads.get() {
            self.queued_loads.borrow_mut().push((request, emit));
            return LoadHandle::new(|| {});
        }
        if request.background && self.lane_busy(&request.location) {
            emit(DirectoryEvent::Failed {
                request_id: request.id,
                message: "The operation timed out".to_owned(),
                refused: true,
                denied: false,
            });
            return LoadHandle::new(|| {});
        }
        if !self.answering() && self.probes_run_out_of_time.get() {
            emit(DirectoryEvent::Finished {
                request_id: request.id,
                truncated: true,
                can_trash: None,
                can_delete: None,
            });
            return LoadHandle::new(|| {});
        }
        if !self.begin_load(&request, &emit) {
            return LoadHandle::new(|| {});
        }
        if self.hold_loads.get() {
            self.held_loads.borrow_mut().push((request, emit));
        } else {
            self.finish_load(&request, &emit);
        }
        LoadHandle::new(|| {})
    }

    fn start_watch(
        &self,
        request: WatchRequest,
        notify: Notify,
        report: Rc<dyn Fn(Watching)>,
    ) -> LoadHandle {
        let location = request.location;
        self.monitors.borrow_mut().insert(location.clone(), notify);
        self.watch_requests.borrow_mut().push(location.clone());
        let watching = self
            .folders
            .borrow()
            .get(&location)
            .and_then(|folder| folder.watching);
        if let Some(watching) = watching {
            report(watching);
        }
        LoadHandle::new(|| {})
    }

    fn responsiveness(&self, _location: &Location) -> Responsiveness {
        self.responsiveness.get()
    }

    fn follow_responsiveness(&self, notify: Rc<dyn Fn()>) -> LoadHandle {
        self.followers.borrow_mut().push(notify);
        LoadHandle::new(|| {})
    }

    fn check_stamps(
        &self,
        locations: Vec<Location>,
        emit: Rc<dyn Fn(Location, StampAnswer)>,
    ) -> LoadHandle {
        let busy: Vec<bool> = locations
            .iter()
            .map(|location| self.lane_busy(location))
            .collect();
        for (location, busy) in locations.into_iter().zip(busy) {
            self.checks.borrow_mut().push(location.clone());
            if busy || !self.answering() {
                emit(location, StampAnswer::Unresponsive);
            } else if self.hold_checks.get() {
                self.held_checks.borrow_mut().push((location, emit.clone()));
            } else {
                emit(location.clone(), self.answer_of(&location));
            }
        }
        LoadHandle::new(|| {})
    }
}
