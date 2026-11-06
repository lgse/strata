// SPDX-License-Identifier: MIT

use super::*;

pub(super) enum FillAnswer {
    Complete(Vec<(&'static str, u64)>),
    Chunks(Vec<Vec<(&'static str, u64)>>, MetadataOutcome),
    EmptyComplete,
    TerminalOnly(MetadataOutcome),
    Never,
}

pub(super) struct FillCall {
    pub(super) id: RequestId,
    pub(super) full: bool,
    pub(super) include_icon_details: bool,
    pub(super) entries: Vec<Location>,
    pub(super) emit: DirectoryEmit,
}

pub(super) type DirectoryEmit = Rc<dyn Fn(DirectoryEvent)>;

pub(super) struct ScriptedSource {
    pub(super) files: Vec<&'static str>,
    pub(super) dirs: Vec<&'static str>,
    pub(super) uri_base: Option<&'static str>,
    pub(super) script: RefCell<Vec<FillAnswer>>,
    pub(super) fill_calls: RefCell<Vec<FillCall>>,
    pub(super) enumerate_calls: RefCell<Vec<(RequestId, DirectoryEmit)>>,
    pub(super) manual_enumerate: bool,
    pub(super) missing: RefCell<HashSet<Location>>,
    pub(super) unreachable: RefCell<HashSet<Location>>,
    pub(super) identities: FolderIdentities,
    pub(super) watched: bool,
}

impl ScriptedSource {
    pub(super) fn scripted(files: Vec<&'static str>, script: Vec<FillAnswer>) -> Self {
        Self {
            files,
            dirs: Vec::new(),
            uri_base: None,
            script: RefCell::new(script),
            fill_calls: RefCell::new(Vec::new()),
            enumerate_calls: RefCell::new(Vec::new()),
            manual_enumerate: false,
            missing: RefCell::new(HashSet::new()),
            unreachable: RefCell::new(HashSet::new()),
            identities: FolderIdentities::default(),
            watched: false,
        }
    }

    pub(super) fn manual(files: Vec<&'static str>, script: Vec<FillAnswer>) -> Self {
        Self {
            manual_enumerate: true,
            ..Self::scripted(files, script)
        }
    }

    pub(super) fn entry_location(&self, name: &str) -> Location {
        match self.uri_base {
            Some(base) => Location::uri(format!("{base}/{name}")),
            None => Location::local(format!("/fixture/{name}")),
        }
    }

    pub(super) fn listed_entry(&self, name: &'static str, is_dir: bool) -> FileEntry {
        FileEntry {
            location: self.entry_location(name),
            native_name: std::ffi::OsString::from(name),
            thumbnail_path: None,
            display_name: name.into(),
            kind: if is_dir {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            size: MetadataValue::Unknown,
            modified_unix_seconds: MetadataValue::Unknown,
            created_unix_seconds: MetadataValue::Unknown,
            mode: MetadataValue::Unknown,
            recent_unix_seconds: MetadataValue::Unknown,
            is_hidden: name.starts_with('.'),
            image_dimensions: MetadataValue::Unknown,
            child_count: MetadataValue::Unknown,
            duration_seconds: MetadataValue::Unknown,
            listable: MetadataValue::Unknown,
            recent_uri: None,
        }
    }
    pub(super) fn answer(
        id: RequestId,
        uri_base: Option<&str>,
        answer: FillAnswer,
        emit: &Rc<dyn Fn(DirectoryEvent)>,
    ) {
        let locate = |name: &str| match uri_base {
            Some(base) => Location::uri(format!("{base}/{name}")),
            None => Location::local(format!("/fixture/{name}")),
        };
        let chunk = |rows: &[(&'static str, u64)]| DirectoryEvent::MetadataFilled {
            request_id: id,
            updates: rows
                .iter()
                .map(|(name, size)| MetadataUpdate {
                    location: locate(name),
                    size: MetadataValue::Known(*size),
                    modified_unix_seconds: MetadataValue::Known(7),
                    created_unix_seconds: MetadataValue::Unknown,
                    mode: MetadataValue::Unknown,
                    image_dimensions: MetadataValue::Unknown,
                    child_count: MetadataValue::Unknown,
                    duration_seconds: MetadataValue::Unknown,
                    listable: MetadataValue::Unknown,
                })
                .collect(),
        };
        match answer {
            FillAnswer::Complete(rows) => {
                emit(chunk(&rows));
                emit(DirectoryEvent::MetadataFinished {
                    request_id: id,
                    outcome: MetadataOutcome::Complete,
                });
            }
            FillAnswer::Chunks(chunks, outcome) => {
                for rows in &chunks {
                    emit(chunk(rows));
                }
                emit(DirectoryEvent::MetadataFinished {
                    request_id: id,
                    outcome,
                });
            }
            FillAnswer::EmptyComplete => emit(DirectoryEvent::MetadataFinished {
                request_id: id,
                outcome: MetadataOutcome::Complete,
            }),
            FillAnswer::TerminalOnly(outcome) => emit(DirectoryEvent::MetadataFinished {
                request_id: id,
                outcome,
            }),
            FillAnswer::Never => {}
        }
    }
}

impl FileSource for ScriptedSource {
    fn find_by_identity(
        &self,
        departed: Location,
        identity: ItemIdentity,
        emit: Rc<dyn Fn(Option<Location>)>,
    ) -> LoadHandle {
        let missing = |location: &Location| self.missing.borrow().contains(location);
        emit(self.identities.find(&departed, identity, missing));
        LoadHandle::new(|| {})
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

    fn supports_metadata_fill(&self, _location: &Location) -> bool {
        true
    }
    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        if self.manual_enumerate {
            self.enumerate_calls.borrow_mut().push((request.id, emit));
            return LoadHandle::new(|| {});
        }
        self.identities.stamp(&request, &emit);
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: self
                .files
                .iter()
                .map(|name| self.listed_entry(name, false))
                .chain(self.dirs.iter().map(|name| self.listed_entry(name, true)))
                .collect(),
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }

    fn fill_metadata(
        &self,
        request: MetadataRequest,
        emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        let id = request.id;
        let full = request.full;
        let entries = request.entries.clone();
        let answer = self.script.borrow_mut().pop();
        match answer {
            Some(FillAnswer::Never) | None => {
                self.fill_calls.borrow_mut().push(FillCall {
                    id,
                    full,
                    include_icon_details: request.include_icon_details,
                    entries,
                    emit,
                });
            }
            Some(answer) => {
                self.fill_calls.borrow_mut().push(FillCall {
                    id,
                    full,
                    include_icon_details: request.include_icon_details,
                    entries,
                    emit: emit.clone(),
                });
                Self::answer(id, self.uri_base, answer, &emit);
            }
        }
        LoadHandle::new(|| {})
    }

    fn watch(
        &self,
        _location: Location,
        _include_hidden: bool,
        _notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        self.watched.then(|| LoadHandle::new(|| {}))
    }
}

pub(super) fn scripted_browser(
    source: ScriptedSource,
) -> (
    Rc<Browser>,
    Rc<RefCell<Vec<BrowserEvent>>>,
    Rc<ScriptedSource>,
) {
    let source = Rc::new(source);
    let browser = Browser::new(source.clone());
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    (browser, events, source)
}
