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
    cancelled: Rc<Cell<bool>>,
    pending: Rc<RefCell<HashMap<PendingMonitorKey, PendingMonitorChange>>>,
    timeout: Rc<RefCell<Option<glib::SourceId>>>,
    monitor: monitors::BareMonitor,
}

pub(super) fn connect_monitor(
    monitor: monitors::BareMonitor,
    location: Location,
    include_hidden: bool,
    notify: Rc<dyn Fn(DirectoryChange)>,
) -> ConnectedMonitor {
    let cancelled = Rc::new(Cell::new(false));
    let pending = Rc::new(RefCell::new(HashMap::<
        PendingMonitorKey,
        PendingMonitorChange,
    >::new()));
    let timeout = Rc::new(RefCell::new(None::<glib::SourceId>));
    let numbered = Rc::new(RefCell::new(NumberedNames::default()));
    let awaited = RefCell::new(AwaitedHints::default());
    let pending_for_change = pending.clone();
    let timeout_for_change = timeout.clone();
    let cancelled_for_change = cancelled.clone();
    let watched = location.clone();
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
                        notify.clone(),
                        cancelled_for_change.clone(),
                    );
                    return;
                }
            }
            if !awaited.borrow_mut().needs_query(&change, event) {
                return;
            }
            if pending_for_change.borrow().contains_key(&None) {
                return;
            }
            if matches!(change, PendingMonitorChange::Rescan) {
                tracing::debug!(?event, "directory monitor requested reconciliation");
            }
            let key = match &change {
                PendingMonitorChange::Upsert(location) | PendingMonitorChange::Remove(location) => {
                    Some(location.clone())
                }
                PendingMonitorChange::Move { to, .. } => Some(to.clone()),
                PendingMonitorChange::Rescan => None,
            };
            if !queue_monitor_change(&mut pending_for_change.borrow_mut(), key, change) {
                return;
            }

            if timeout_for_change.borrow().is_none() {
                let pending = pending_for_change.clone();
                let timeout = timeout_for_change.clone();
                let notify = notify.clone();
                let cancelled = cancelled_for_change.clone();
                let source = glib::timeout_add_local_once(Duration::from_millis(100), move || {
                    timeout.take();
                    flush_monitor_changes(&pending, &notify, &cancelled);
                });
                timeout_for_change.replace(Some(source));
            }
        });

    ConnectedMonitor {
        handler: Some(handler),
        cancelled,
        pending,
        timeout,
        monitor,
    }
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
        self.cancelled.set(true);
        if let Some(source) = self.timeout.take() {
            source.remove();
        }
        self.pending.borrow_mut().clear();
        if let Some(handler) = self.handler.take() {
            self.monitor.monitor().disconnect(handler);
        }
    }
}
