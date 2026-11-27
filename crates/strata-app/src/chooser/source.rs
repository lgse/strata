// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    ops::RangeInclusive,
    path::{Component, Path, PathBuf},
    rc::Rc,
};

use strata_core::model::{FileEntry, Location};
use strata_fs::{
    adapters::{LocalFileSource, probes},
    services::{
        DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, ItemIdentity, LoadHandle,
        LocationValidationError, MetadataRequest, Responsiveness, StampAnswer, WatchRequest,
        Watching, guess_content_type,
    },
};

use crate::portal::{FilterRule, PortalFileFilter};

pub(crate) const OUTSIDE_ROOT: &str = "Choose an existing folder inside this removable device.";
pub(crate) const LOCAL_ONLY_REFUSAL: &str =
    "The system file chooser supports local files and folders only.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompiledFilter {
    pub(crate) portal: PortalFileFilter,
}

impl CompiledFilter {
    pub(crate) fn new(portal: PortalFileFilter) -> Self {
        Self { portal }
    }

    pub(crate) fn matches_entry(&self, entry: &FileEntry) -> bool {
        if entry.is_directory() {
            return true;
        }
        let name = entry.native_name.to_string_lossy();
        let (content_type, _) = guess_content_type(Path::new(&entry.native_name));
        self.matches_file(&name, &content_type)
    }

    pub(crate) fn matches_file(&self, file_name: &str, content_type: &str) -> bool {
        self.portal.rules.iter().any(|rule| match rule {
            FilterRule::Pattern(pattern) => glob_matches(pattern, file_name),
            FilterRule::MimeType(mime) => mime_matches(mime, content_type),
        })
    }
}

fn mime_matches(filter_mime: &str, content_type: &str) -> bool {
    if filter_mime == "*/*" {
        return true;
    }
    if gio::content_type_from_mime_type(filter_mime)
        .is_some_and(|filter_ct| gio::content_type_is_a(content_type, &filter_ct))
    {
        return true;
    }
    if let Some(prefix) = filter_mime.strip_suffix("/*") {
        let actual_mime = gio::content_type_get_mime_type(content_type);
        if let Some(actual) = actual_mime {
            return actual
                .as_str()
                .split_once('/')
                .is_some_and(|(kind, _)| kind.eq_ignore_ascii_case(prefix));
        }
    }
    false
}

pub(crate) fn glob_matches(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_step(&pat, &txt)
}

fn glob_step(pat: &[char], txt: &[char]) -> bool {
    let Some(&first) = pat.first() else {
        return txt.is_empty();
    };
    let rest = &pat[1..];
    match first {
        '*' => glob_star(rest, txt),
        '?' => !txt.is_empty() && glob_step(rest, &txt[1..]),
        '[' => glob_class(rest, txt),
        literal => txt.first() == Some(&literal) && glob_step(rest, &txt[1..]),
    }
}

/// A `*`, with `pat` what follows it: any run of characters, the empty one included.
fn glob_star(pat: &[char], txt: &[char]) -> bool {
    let rest = pat
        .iter()
        .position(|c| *c != '*')
        .map_or(&[][..], |at| &pat[at..]);
    (0..=txt.len()).any(|skip| glob_step(rest, &txt[skip..]))
}

/// A `[`, with `pat` what follows it: a class, or a literal `[` when no `]` closes it.
fn glob_class(pat: &[char], txt: &[char]) -> bool {
    match match_char_class(pat, txt.first().copied()) {
        Some((after_class, matched)) => matched && glob_step(after_class, &txt[1..]),
        None => txt.first() == Some(&'[') && glob_step(pat, &txt[1..]),
    }
}

fn class_member(pat: &[char]) -> (RangeInclusive<char>, usize) {
    match pat {
        [low, '-', high, ..] if *high != ']' => (*low..=*high, 3),
        _ => (pat[0]..=pat[0], 1),
    }
}

fn match_char_class(pat: &[char], ch: Option<char>) -> Option<(&[char], bool)> {
    let negated = matches!(pat.first(), Some('!' | '^'));
    let start = usize::from(negated);
    let mut idx = start;
    let mut matched = false;
    while let Some(&member) = pat.get(idx) {
        if member == ']' && idx > start {
            return Some((&pat[idx + 1..], ch.is_some() && matched != negated));
        }
        let (members, width) = class_member(&pat[idx..]);
        matched |= ch.is_some_and(|target| members.contains(&target));
        idx += width;
    }
    None
}

pub(crate) struct ChooserFileSource {
    source: Rc<dyn FileSource>,
    filter: Rc<RefCell<Option<CompiledFilter>>>,
    pub(crate) directory_only: Rc<Cell<bool>>,
    pub(crate) root_limit: Option<PathBuf>,
}

impl ChooserFileSource {
    pub(crate) fn new(root_limit: Option<PathBuf>) -> Rc<Self> {
        Rc::new(Self {
            source: Rc::new(LocalFileSource),
            filter: Rc::new(RefCell::new(None)),
            directory_only: Rc::new(Cell::new(false)),
            root_limit,
        })
    }

    pub(crate) fn set_filter(&self, filter: Option<CompiledFilter>) {
        self.filter.replace(filter);
    }

    fn refusal(&self, location: &Location) -> Result<(), LocationValidationError> {
        if !self.allows_navigation(location) {
            return Err(LocationValidationError::Refused(OUTSIDE_ROOT));
        }
        if location.native_path().is_none() && !location.is_recent_root() {
            return Err(LocationValidationError::Refused(LOCAL_ONLY_REFUSAL));
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
            emit(filter_directory_event(
                filter.borrow().as_ref(),
                directory_only.get(),
                root_limit.as_deref(),
                event,
            ));
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
        request: WatchRequest,
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
        let (Some(root), Some(path)) = (self.root_limit.clone(), request.location.native_path())
        else {
            return self.source.start_watch(request, notify, report);
        };
        let path = path.to_owned();
        let source = self.source.clone();
        after_containment(root, path, move |answer| match answer {
            Ok(()) => Some(source.start_watch(request, notify, report)),
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

    fn responsiveness(&self, location: &Location) -> Responsiveness {
        self.source.responsiveness(location)
    }

    fn follow_responsiveness(&self, notify: Rc<dyn Fn()>) -> LoadHandle {
        self.source.follow_responsiveness(notify)
    }

    fn find_by_identity(
        &self,
        departed: Location,
        identity: ItemIdentity,
        emit: Rc<dyn Fn(Option<Location>)>,
    ) -> LoadHandle {
        self.source.find_by_identity(departed, identity, emit)
    }
}

fn after_containment(
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
        slot.replace(proven(answer));
    });
    LoadHandle::new(move || {
        task.abort();
        started.take();
    })
}

pub(crate) fn chooser_location_allowed(root: Option<&Path>, location: &Location) -> bool {
    root.is_none_or(|root| {
        location.native_path().is_some_and(|path| {
            path.starts_with(root) && !path.components().any(|part| part == Component::ParentDir)
        })
    })
}

pub(crate) fn chooser_entry_allowed(
    filter: Option<&CompiledFilter>,
    directory_only: bool,
    root_limit: Option<&Path>,
    entry: &FileEntry,
) -> bool {
    chooser_location_allowed(root_limit, &entry.location)
        && entry.location.native_path().is_some()
        && (!directory_only || entry.is_directory())
        && filter.is_none_or(|filter| filter.matches_entry(entry))
}

fn filter_directory_event(
    filter: Option<&CompiledFilter>,
    directory_only: bool,
    root_limit: Option<&Path>,
    event: DirectoryEvent,
) -> DirectoryEvent {
    match event {
        DirectoryEvent::Batch {
            request_id,
            mut entries,
        } => {
            entries
                .retain(|entry| chooser_entry_allowed(filter, directory_only, root_limit, entry));
            DirectoryEvent::Batch {
                request_id,
                entries,
            }
        }
        other => other,
    }
}

fn filter_directory_change(
    filter: Option<&CompiledFilter>,
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
        other => other,
    }
}
