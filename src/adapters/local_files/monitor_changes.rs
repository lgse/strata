// SPDX-License-Identifier: MIT
//! Coalescing directory monitor events into directory changes.

use super::*;

pub(super) const MAX_PENDING_MONITOR_CHANGES: usize = 4_096;

#[derive(Clone)]
pub(super) enum PendingMonitorChange {
    Upsert(Location),
    Remove(Location),
    Move { from: Location, to: Location },
    Rescan,
}

pub(super) type PendingMonitorKey = Option<Location>;

pub(super) struct ConnectedMonitor {
    handler: Option<glib::SignalHandlerId>,
    batches: MonitorBatches,
    monitor: monitors::BareMonitor,
}

#[derive(Clone)]
pub(super) struct MonitorBatches {
    pending: Rc<RefCell<HashMap<PendingMonitorKey, PendingMonitorChange>>>,
    timeout: Rc<RefCell<Option<glib::SourceId>>>,
    notify: Rc<dyn Fn(DirectoryChange)>,
    cancelled: Rc<Cell<bool>>,
    departure_checks: Rc<RefCell<DepartureChecks>>,
}

impl MonitorBatches {
    fn new(
        watched: Location,
        began_on: Option<ItemIdentity>,
        notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Self {
        let cancelled = Rc::new(Cell::new(false));
        let departure_checks = Rc::new(RefCell::new(DepartureChecks {
            watched,
            began_on,
            notify: notify.clone(),
            cancelled: cancelled.clone(),
            running: None,
            again: false,
        }));
        Self {
            pending: Rc::default(),
            timeout: Rc::default(),
            notify,
            cancelled,
            departure_checks,
        }
    }
}

pub(super) fn connect_monitor(
    monitor: monitors::BareMonitor,
    request: WatchRequest,
    notify: Rc<dyn Fn(DirectoryChange)>,
) -> ConnectedMonitor {
    let WatchRequest {
        location: watched,
        include_hidden,
        began_on,
    } = request;
    let batches = MonitorBatches::new(watched.clone(), began_on, notify);
    let numbered = Rc::new(RefCell::new(NumberedNames::default()));
    let awaited = RefCell::new(AwaitedHints::default());
    let shared = batches.clone();
    let handler = monitor
        .monitor()
        .connect_changed(move |_, file, other_file, event| {
            let change = pending_monitor_change(
                &watched,
                location_for_file(file),
                other_file.and_then(location_for_file),
                event,
            );
            let Some(change) =
                change.and_then(|change| visible_monitor_change(&watched, change, include_hidden))
            else {
                return;
            };
            // Arrivals are learned even while a rescan is pending.
            let admission = numbered.borrow_mut().admit(&change, event);
            match admission {
                NumberedAdmission::Queue => {}
                NumberedAdmission::Skip => return,
                NumberedAdmission::Read(folder) => {
                    read_numbered_names(
                        numbered.clone(),
                        folder,
                        shared.notify.clone(),
                        shared.cancelled.clone(),
                    );
                    return;
                }
            }
            if !awaited.borrow_mut().needs_query(&change, event) {
                return;
            }
            if shared.pending.borrow().contains_key(&None) {
                return;
            }
            if matches!(change, PendingMonitorChange::Rescan) {
                tracing::debug!(?event, "directory monitor requested reconciliation");
            }
            let key = pending_key(&change);
            if queue_monitor_change(&mut shared.pending.borrow_mut(), key, change) {
                schedule_flush(&shared);
            }
        });

    ConnectedMonitor {
        handler: Some(handler),
        batches,
        monitor,
    }
}

fn pending_key(change: &PendingMonitorChange) -> PendingMonitorKey {
    match change {
        PendingMonitorChange::Upsert(location) | PendingMonitorChange::Remove(location) => {
            Some(location.clone())
        }
        PendingMonitorChange::Move { to, .. } => Some(to.clone()),
        PendingMonitorChange::Rescan => None,
    }
}

fn schedule_flush(batches: &MonitorBatches) {
    if batches.timeout.borrow().is_some() {
        return;
    }
    let batch = batches.clone();
    let source = glib::timeout_add_local_once(Duration::from_millis(100), move || {
        batch.timeout.take();
        let reported = reports_departure(
            &batch.pending.borrow(),
            &batch.departure_checks.borrow().watched,
        );
        flush_monitor_changes(&batch.pending, &batch.notify, &batch.cancelled);
        if !reported {
            report_if_departed(&batch.departure_checks);
        }
    });
    batches.timeout.replace(Some(source));
}

pub(super) fn monitor_location_is_hidden(location: &Location) -> bool {
    location
        .file_name()
        .is_some_and(|name| name.as_encoded_bytes().first() == Some(&b'.'))
}

pub(super) fn visible_monitor_change(
    watched: &Location,
    change: PendingMonitorChange,
    include_hidden: bool,
) -> Option<PendingMonitorChange> {
    if include_hidden {
        return Some(change);
    }
    match change {
        // The open folder's own departure counts even when its name is hidden.
        PendingMonitorChange::Remove(location) if location == *watched => {
            Some(PendingMonitorChange::Remove(location))
        }
        PendingMonitorChange::Upsert(location) if monitor_location_is_hidden(&location) => None,
        PendingMonitorChange::Remove(location) if monitor_location_is_hidden(&location) => None,
        PendingMonitorChange::Move { from, to } => {
            match (
                monitor_location_is_hidden(&from),
                monitor_location_is_hidden(&to),
            ) {
                (true, true) => None,
                (true, false) => Some(PendingMonitorChange::Upsert(to)),
                (false, true) => Some(PendingMonitorChange::Remove(from)),
                (false, false) => Some(PendingMonitorChange::Move { from, to }),
            }
        }
        change => Some(change),
    }
}

/// GLib follows every write with `Changed` before the `ChangesDoneHint` that ends the
/// name's changes, so a hint after a `Created` or `Changed` this watch queried adds nothing.
/// A file linked into place, as copies publish files, gets its hint two seconds late.
#[derive(Default)]
pub(super) struct AwaitedHints(HashSet<Location>);

impl AwaitedHints {
    fn needs_query(&mut self, change: &PendingMonitorChange, event: gio::FileMonitorEvent) -> bool {
        let PendingMonitorChange::Upsert(location) = change else {
            return true;
        };
        if location.native_path().is_none() {
            return true;
        }
        match event {
            gio::FileMonitorEvent::Created | gio::FileMonitorEvent::Changed => {
                self.0.insert(location.clone());
                true
            }
            gio::FileMonitorEvent::ChangesDoneHint => !self.0.remove(location),
            _ => true,
        }
    }
}

/// Writes to an `O_TMPFILE` reach its folder's watch under the name `#<inode>`, which no
/// listing holds, so changes to such names are queried only once the folder shows them.
#[derive(Default)]
pub(super) struct NumberedNames {
    known: HashSet<OsString>,
    listing: NumberedListing,
}

#[derive(Default)]
pub(super) enum NumberedListing {
    #[default]
    Unread,
    Reading(HashSet<Location>),
    Read,
    Unavailable,
}

pub(super) enum NumberedAdmission {
    Queue,
    Skip,
    Read(PathBuf),
}

pub(super) fn is_numbered_name(name: &OsStr) -> bool {
    match name.as_encoded_bytes() {
        [b'#', digits @ ..] => !digits.is_empty() && digits.iter().all(u8::is_ascii_digit),
        _ => false,
    }
}

pub(super) fn numbered_name(location: &Location) -> Option<&OsStr> {
    location
        .native_path()
        .and_then(Path::file_name)
        .filter(|name| is_numbered_name(name))
}

impl NumberedNames {
    fn admit(
        &mut self,
        change: &PendingMonitorChange,
        event: gio::FileMonitorEvent,
    ) -> NumberedAdmission {
        let (location, arrived) = match change {
            PendingMonitorChange::Upsert(location) => (
                location,
                matches!(
                    event,
                    gio::FileMonitorEvent::Created
                        | gio::FileMonitorEvent::MovedIn
                        | gio::FileMonitorEvent::Moved
                        | gio::FileMonitorEvent::Renamed
                ),
            ),
            PendingMonitorChange::Move { to, .. } => (to, true),
            PendingMonitorChange::Remove(_) | PendingMonitorChange::Rescan => {
                return NumberedAdmission::Queue;
            }
        };
        let Some(name) = numbered_name(location) else {
            return NumberedAdmission::Queue;
        };
        if arrived {
            self.known.insert(name.to_owned());
        }
        if self.known.contains(name) {
            return NumberedAdmission::Queue;
        }
        match &mut self.listing {
            NumberedListing::Unread => {
                let Some(folder) = location.native_path().and_then(Path::parent) else {
                    return NumberedAdmission::Queue;
                };
                self.listing = NumberedListing::Reading(HashSet::from([location.clone()]));
                NumberedAdmission::Read(folder.to_path_buf())
            }
            NumberedListing::Reading(held) => {
                held.insert(location.clone());
                NumberedAdmission::Skip
            }
            NumberedListing::Read => NumberedAdmission::Skip,
            NumberedListing::Unavailable => NumberedAdmission::Queue,
        }
    }

    fn finish_reading(&mut self, names: Option<HashSet<OsString>>) -> Vec<Location> {
        let NumberedListing::Reading(held) = std::mem::take(&mut self.listing) else {
            return Vec::new();
        };
        let Some(names) = names else {
            self.listing = NumberedListing::Unavailable;
            return held.into_iter().collect();
        };
        self.known.extend(names);
        self.listing = NumberedListing::Read;
        held.into_iter()
            .filter(|location| {
                numbered_name(location).is_some_and(|name| self.known.contains(name))
            })
            .collect()
    }
}

pub(super) fn read_numbered_names(
    numbered: Rc<RefCell<NumberedNames>>,
    folder: PathBuf,
    notify: Rc<dyn Fn(DirectoryChange)>,
    cancelled: Rc<Cell<bool>>,
) {
    glib::MainContext::default().spawn_local(async move {
        let names = gio::spawn_blocking(move || numbered_names_in(&folder))
            .await
            .ok()
            .and_then(Result::ok);
        if cancelled.get() {
            return;
        }
        let listed = numbered.borrow_mut().finish_reading(names);
        for location in listed {
            query_monitored_entry(location, None, notify.clone(), cancelled.clone());
        }
    });
}

pub(super) fn numbered_names_in(folder: &Path) -> std::io::Result<HashSet<OsString>> {
    let mut names = HashSet::new();
    for entry in file_activity::list(folder)? {
        let name = entry?.file_name();
        if is_numbered_name(&name) {
            names.insert(name);
        }
    }
    Ok(names)
}

pub(super) fn pending_monitor_change(
    watched: &Location,
    changed: Option<Location>,
    other: Option<Location>,
    event: gio::FileMonitorEvent,
) -> Option<PendingMonitorChange> {
    if watched.is_recent_root() {
        return Some(PendingMonitorChange::Rescan);
    }

    let changed = monitored_change_target(watched, changed, event);
    let change = match event {
        gio::FileMonitorEvent::Deleted | gio::FileMonitorEvent::MovedOut => {
            changed.map(PendingMonitorChange::Remove)
        }
        gio::FileMonitorEvent::Created | gio::FileMonitorEvent::MovedIn => {
            changed.map(PendingMonitorChange::Upsert)
        }
        gio::FileMonitorEvent::Changed
        | gio::FileMonitorEvent::ChangesDoneHint
        | gio::FileMonitorEvent::AttributeChanged => changed.map(PendingMonitorChange::Upsert),
        gio::FileMonitorEvent::Moved | gio::FileMonitorEvent::Renamed => changed
            .zip(other)
            .map(|(from, to)| PendingMonitorChange::Move { from, to }),
        gio::FileMonitorEvent::PreUnmount | gio::FileMonitorEvent::Unmounted => {
            Some(PendingMonitorChange::Rescan)
        }
        _ => Some(PendingMonitorChange::Rescan),
    };
    if watched.is_camera_photo_root() {
        Some(PendingMonitorChange::Rescan)
    } else {
        change
    }
}

// GVfs can report content changes against the watched directory itself; keep only departures.
pub(super) fn monitored_change_target(
    watched: &Location,
    changed: Option<Location>,
    event: gio::FileMonitorEvent,
) -> Option<Location> {
    let changed = changed?;
    let departed = matches!(
        event,
        gio::FileMonitorEvent::Deleted | gio::FileMonitorEvent::MovedOut
    );
    (&changed != watched || departed).then_some(changed)
}

pub(super) fn queue_monitor_change(
    pending: &mut HashMap<PendingMonitorKey, PendingMonitorChange>,
    key: PendingMonitorKey,
    change: PendingMonitorChange,
) -> bool {
    if pending.contains_key(&None) {
        return false;
    }
    if let PendingMonitorChange::Move { ref from, .. } = change
        && matches!(
            pending.get(&Some(from.clone())),
            Some(PendingMonitorChange::Upsert(_))
        )
    {
        pending.remove(&Some(from.clone()));
    }
    pending
        .entry(key)
        .and_modify(|pending| {
            *pending = merge_pending_change(pending.clone(), change.clone());
        })
        .or_insert(change);
    if pending.len() > MAX_PENDING_MONITOR_CHANGES {
        tracing::debug!(
            pending = pending.len(),
            "directory monitor burst requested reconciliation"
        );
        pending.clear();
        pending.insert(None, PendingMonitorChange::Rescan);
    }
    true
}

pub(super) fn merge_pending_change(
    existing: PendingMonitorChange,
    incoming: PendingMonitorChange,
) -> PendingMonitorChange {
    match (&existing, &incoming) {
        (PendingMonitorChange::Rescan, _) | (_, PendingMonitorChange::Rescan) => {
            PendingMonitorChange::Rescan
        }
        // An exchange can report arrival before departure; query the name's final state.
        (PendingMonitorChange::Move { .. }, PendingMonitorChange::Upsert(_))
        | (PendingMonitorChange::Upsert(_), PendingMonitorChange::Remove(_)) => existing,
        (PendingMonitorChange::Move { .. }, PendingMonitorChange::Remove(_)) => {
            PendingMonitorChange::Rescan
        }
        (_, PendingMonitorChange::Move { .. }) => incoming,
        _ => incoming,
    }
}

pub(super) fn flush_monitor_changes(
    pending: &RefCell<HashMap<PendingMonitorKey, PendingMonitorChange>>,
    notify: &Rc<dyn Fn(DirectoryChange)>,
    cancelled: &Rc<Cell<bool>>,
) {
    let changes: Vec<_> = pending
        .borrow_mut()
        .drain()
        .map(|(_, change)| change)
        .collect();
    let needs_rescan = {
        let moved_sources: HashSet<_> = changes
            .iter()
            .filter_map(|change| match change {
                PendingMonitorChange::Move { from, .. } => Some(from),
                _ => None,
            })
            .collect();
        // A move's query can finish after the recreated source's query and remove its new row.
        changes.iter().any(|change| match change {
            PendingMonitorChange::Rescan => true,
            PendingMonitorChange::Upsert(location) => moved_sources.contains(location),
            _ => false,
        })
    };
    if needs_rescan {
        notify(DirectoryChange::Rescan);
        return;
    }

    for change in changes {
        match change {
            PendingMonitorChange::Remove(location) => {
                notify(DirectoryChange::Remove(location));
            }
            PendingMonitorChange::Upsert(location) => {
                query_monitored_entry(location, None, notify.clone(), cancelled.clone())
            }
            PendingMonitorChange::Move { from, to } => {
                query_monitored_entry(to, Some(from), notify.clone(), cancelled.clone())
            }
            PendingMonitorChange::Rescan => {}
        }
    }
}

pub(super) fn reports_departure(
    pending: &HashMap<PendingMonitorKey, PendingMonitorChange>,
    watched: &Location,
) -> bool {
    pending.values().any(|change| {
        matches!(change, PendingMonitorChange::Remove(location)
            | PendingMonitorChange::Move { from: location, .. } if location == watched)
    })
}

/// A background request is skipped while another holds its lane, so a watch looks at its
/// folder's path once at a time, and the batches that arrive meanwhile ask for one more
/// look when it ends.
pub(super) struct DepartureChecks {
    watched: Location,
    began_on: Option<ItemIdentity>,
    notify: Rc<dyn Fn(DirectoryChange)>,
    cancelled: Rc<Cell<bool>>,
    running: Option<glib::JoinHandle<()>>,
    again: bool,
}

/// Inotify watches the folder, not its path, so it reports no rename or move of an
/// ancestor and goes on to report changes under the old path. After each batch, the
/// folder is looked up at its path as background work on its mount's lane, and its
/// removal is reported when the path holds no folder, or one proven to be another than
/// the folder the load that asked for the watch found (`began_on`). A busy or stalled
/// lane leaves the look to the next batch.
pub(super) fn report_if_departed(checks: &Rc<RefCell<DepartureChecks>>) {
    if checks.borrow().running.is_some() {
        checks.borrow_mut().again = true;
        return;
    }
    let (watched, began_on) = {
        let checks = checks.borrow();
        (checks.watched.clone(), checks.began_on)
    };
    let Some(path) = watched.native_path().map(Path::to_path_buf) else {
        return;
    };
    let job = mount_lanes::run(&path.clone(), Class::Background, move |progress| {
        departed_from(&path, began_on, progress)
    });
    let finished = checks.clone();
    let task = glib::MainContext::default().spawn_local(async move {
        let departed = job.await.unwrap_or(false);
        let (again, notify, cancelled) = {
            let mut checks = finished.borrow_mut();
            checks.running = None;
            let again = std::mem::take(&mut checks.again);
            (again, checks.notify.clone(), checks.cancelled.get())
        };
        if cancelled {
            return;
        }
        if departed {
            notify(DirectoryChange::Remove(watched));
        } else if again {
            report_if_departed(&finished);
        }
    });
    checks.borrow_mut().running = Some(task);
}

/// Mounts that renumber folders prove nothing either way, so only proof of another folder
/// counts as a departure there.
pub(super) fn departed_from(
    path: &Path,
    began_on: Option<ItemIdentity>,
    progress: &Progress,
) -> bool {
    match folder_identity_at(path, progress) {
        Ok(Some(identity)) => {
            identity.evidence.kind != FileType::Directory
                || began_on.is_some_and(|began_on| began_on.proof(&identity) == Proof::Different)
        }
        Ok(None) => false,
        Err(error) => matches!(error, rustix::io::Errno::NOENT | rustix::io::Errno::NOTDIR),
    }
}

pub(super) fn log_monitor_metadata_error(location: &Location, error: &glib::Error) {
    tracing::debug!(
        location = %location.diagnostic_path(),
        error = %sanitize_failure_message(&error.to_string()),
        "monitor metadata unavailable"
    );
}

pub(super) fn query_monitored_entry(
    location: Location,
    moved_from: Option<Location>,
    notify: Rc<dyn Fn(DirectoryChange)>,
    cancelled: Rc<Cell<bool>>,
) {
    glib::MainContext::default().spawn_local(async move {
        let result = query_entry(
            location.clone(),
            FULL_ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
        )
        .await;
        if cancelled.get() {
            return;
        }
        match result {
            Ok(entry) => {
                if let Some(from) = moved_from {
                    notify(DirectoryChange::Move { from, entry });
                } else {
                    notify(DirectoryChange::Upsert(entry));
                }
            }
            Err(error) if error.matches(gio::IOErrorEnum::NotFound) => {
                let removed = moved_from.unwrap_or(location);
                if !cancelled.get() {
                    notify(DirectoryChange::Remove(removed));
                }
            }
            Err(error) => {
                log_monitor_metadata_error(&location, &error);
                if !cancelled.get() {
                    notify(DirectoryChange::Rescan);
                }
            }
        }
    });
}

impl Drop for ConnectedMonitor {
    fn drop(&mut self) {
        let batches = &self.batches;
        batches.cancelled.set(true);
        if let Some(source) = batches.timeout.take() {
            source.remove();
        }
        batches.pending.borrow_mut().clear();
        if let Some(task) = batches.departure_checks.borrow_mut().running.take() {
            task.abort();
        }
        if let Some(handler) = self.handler.take() {
            self.monitor.monitor().disconnect(handler);
        }
    }
}
