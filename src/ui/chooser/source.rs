// SPDX-License-Identifier: MIT

use super::*;

pub(super) struct ChooserFileSource {
    pub(super) source: Rc<dyn FileSource>,
    pub(super) filter: Rc<RefCell<Option<gtk::FileFilter>>>,
    pub(super) directory_only: Rc<Cell<bool>>,
    pub(super) root_limit: Option<PathBuf>,
}

impl ChooserFileSource {
    pub(super) fn new() -> Rc<Self> {
        Rc::new(Self {
            source: Rc::new(LocalFileSource),
            filter: Rc::new(RefCell::new(None)),
            directory_only: Rc::new(Cell::new(false)),
            root_limit: None,
        })
    }

    pub(super) fn set_filter(&self, filter: Option<gtk::FileFilter>) {
        self.filter.replace(filter);
    }

    pub(super) fn refusal(&self, location: &Location) -> Result<(), LocationValidationError> {
        if !self.allows_navigation(location) {
            return Err(LocationValidationError::Refused(OUTSIDE_ROOT));
        }
        if location.native_path().is_none() && !location.is_recent_root() {
            return Err(LocationValidationError::Refused(
                "The system file chooser supports local files and folders only.",
            ));
        }
        Ok(())
    }
}

impl FileSource for ChooserFileSource {
    fn allows_navigation(&self, location: &Location) -> bool {
        chooser_location_allowed(self.root_limit.as_deref(), location)
    }

    fn allows_entry(&self, entry: &FileEntry) -> bool {
        chooser_entry_allowed(
            self.filter.borrow().as_ref(),
            self.directory_only.get(),
            self.root_limit.as_deref(),
            entry,
        )
    }

    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        self.refusal(location)?;
        if let (Some(root), Some(path)) = (&self.root_limit, location.native_path())
            && !path.canonicalize().is_ok_and(|real| real.starts_with(root))
        {
            return Err(LocationValidationError::Refused(OUTSIDE_ROOT));
        }
        self.source.validate_location(location)
    }

    fn validate_location_async(
        &self,
        location: Location,
        emit: Rc<dyn Fn(Result<(), LocationValidationError>)>,
    ) -> LoadHandle {
        if let (Some(root), Some(path)) = (self.root_limit.clone(), location.native_path())
            && self.allows_navigation(&location)
        {
            return after_containment(root, path.to_owned(), move |answer| {
                emit(answer);
                None
            });
        }
        if let Err(refusal) = self.refusal(&location) {
            emit(Err(refusal));
            return LoadHandle::new(|| {});
        }
        self.source.validate_location_async(location, emit)
    }

    fn supports_metadata_fill(&self, location: &Location) -> bool {
        (location.native_path().is_some() || location.is_recent_root())
            && self.source.supports_metadata_fill(location)
    }

    fn fill_metadata(
        &self,
        request: MetadataRequest,
        emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        self.source.fill_metadata(request, emit)
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let filter = self.filter.clone();
        let directory_only = self.directory_only.clone();
        let root_limit = self.root_limit.clone();
        let emit: Rc<dyn Fn(DirectoryEvent)> = Rc::new(move |event| {
            let event = match event {
                DirectoryEvent::Batch {
                    request_id,
                    mut entries,
                } => {
                    entries.retain(|entry| {
                        chooser_entry_allowed(
                            filter.borrow().as_ref(),
                            directory_only.get(),
                            root_limit.as_deref(),
                            entry,
                        )
                    });
                    DirectoryEvent::Batch {
                        request_id,
                        entries,
                    }
                }
                event => event,
            };
            emit(event);
        });
        let (Some(root), Some(path)) = (self.root_limit.clone(), request.location.native_path())
        else {
            return self.source.enumerate(request, emit);
        };
        let path = path.to_owned();
        let source = self.source.clone();
        after_containment(root, path, move |answer| match answer {
            Ok(()) => Some(source.enumerate(request, emit)),
            Err(error) => {
                emit(DirectoryEvent::Failed {
                    request_id: request.id,
                    message: error.message(),
                    refused: false,
                    denied: false,
                });
                None
            }
        })
    }

    fn start_watch(
        &self,
        location: Location,
        include_hidden: bool,
        notify: Rc<dyn Fn(DirectoryChange)>,
        report: Rc<dyn Fn(Watching)>,
    ) -> LoadHandle {
        let filter = self.filter.clone();
        let directory_only = self.directory_only.clone();
        let root_limit = self.root_limit.clone();
        let notify: Rc<dyn Fn(DirectoryChange)> = Rc::new(move |change| {
            notify(filter_directory_change(
                filter.borrow().as_ref(),
                directory_only.get(),
                root_limit.as_deref(),
                change,
            ));
        });
        let (Some(root), Some(path)) = (self.root_limit.clone(), location.native_path()) else {
            return self
                .source
                .start_watch(location, include_hidden, notify, report);
        };
        let path = path.to_owned();
        let source = self.source.clone();
        after_containment(root, path, move |answer| match answer {
            Ok(()) => Some(source.start_watch(location, include_hidden, notify, report)),
            Err(_) => {
                report(Watching::Unavailable);
                None
            }
        })
    }

    fn confined_to(&self) -> Option<&Path> {
        self.root_limit.as_deref()
    }

    fn probe_stamp(&self, location: Location, emit: Rc<dyn Fn(StampAnswer)>) -> LoadHandle {
        self.source.probe_stamp(location, emit)
    }

    fn check_stamps(
        &self,
        locations: Vec<Location>,
        emit: Rc<dyn Fn(Location, StampAnswer)>,
    ) -> LoadHandle {
        self.source.check_stamps(locations, emit)
    }

    fn responsiveness(&self, location: &Location) -> crate::services::Responsiveness {
        self.source.responsiveness(location)
    }

    fn follow_responsiveness(&self, notify: Rc<dyn Fn()>) -> LoadHandle {
        self.source.follow_responsiveness(notify)
    }

    fn find_by_identity(
        &self,
        parent: Location,
        identity: ItemIdentity,
        emit: Rc<dyn Fn(Option<Location>)>,
    ) -> LoadHandle {
        self.source.find_by_identity(parent, identity, emit)
    }
}

pub(super) fn file_filter_matches(filter: &gtk::FileFilter, entry: &FileEntry) -> bool {
    if entry.is_directory() {
        return true;
    }
    let info = gio::FileInfo::new();
    info.set_name(Path::new(&entry.native_name));
    info.set_display_name(&entry.display_name);
    info.set_file_type(gio::FileType::Regular);
    let (content_type, _) = crate::services::guess_content_type(Path::new(&entry.native_name));
    info.set_content_type(&content_type);
    filter.match_(&info)
}

pub(super) const OUTSIDE_ROOT: &str = "Choose an existing folder inside this removable device.";

pub(super) fn after_containment(
    root: PathBuf,
    path: PathBuf,
    proven: impl FnOnce(Result<(), LocationValidationError>) -> Option<LoadHandle> + 'static,
) -> LoadHandle {
    let started: Rc<RefCell<Option<LoadHandle>>> = Rc::default();
    let slot = started.clone();
    let task = glib::MainContext::default().spawn_local(async move {
        let answer = match probes::real_folder(&path).await {
            Ok(real) if real.starts_with(&root) => Ok(()),
            Ok(_) => Err(LocationValidationError::Refused(OUTSIDE_ROOT)),
            Err(error) => Err(error),
        };
        let handle = proven(answer);
        slot.replace(handle);
    });
    LoadHandle::new(move || {
        task.abort();
        started.take();
    })
}

pub(super) fn chooser_location_allowed(root: Option<&Path>, location: &Location) -> bool {
    root.is_none_or(|root| {
        location.native_path().is_some_and(|path| {
            path.starts_with(root) && !path.components().any(|part| part == Component::ParentDir)
        })
    })
}

pub(super) fn chooser_entry_allowed(
    filter: Option<&gtk::FileFilter>,
    directory_only: bool,
    root_limit: Option<&Path>,
    entry: &FileEntry,
) -> bool {
    chooser_location_allowed(root_limit, &entry.location)
        && entry.location.native_path().is_some()
        && (!directory_only || entry.is_directory())
        && filter.is_none_or(|filter| file_filter_matches(filter, entry))
}

pub(super) fn filter_directory_change(
    filter: Option<&gtk::FileFilter>,
    directory_only: bool,
    root_limit: Option<&Path>,
    change: DirectoryChange,
) -> DirectoryChange {
    match change {
        DirectoryChange::Upsert(entry)
            if !chooser_entry_allowed(filter, directory_only, root_limit, &entry) =>
        {
            DirectoryChange::Remove(entry.location)
        }
        DirectoryChange::Move { from, entry }
            if !chooser_entry_allowed(filter, directory_only, root_limit, &entry) =>
        {
            DirectoryChange::Remove(from)
        }
        change => change,
    }
}
