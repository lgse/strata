// SPDX-License-Identifier: MIT

use super::*;

struct RecentMonitorSource;

impl FileSource for RecentMonitorSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }

    fn watch(
        &self,
        location: Location,
        include_hidden: bool,
        notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        crate::adapters::LocalFileSource.watch(location, include_hidden, notify)
    }
}

#[test]
fn recent_navigation_keeps_the_normal_monitor_lifecycle() {
    let browser = Browser::new(Rc::new(RecentMonitorSource));

    browser.navigate(Location::uri("recent:///"));

    assert_eq!(browser.monitors.borrow().len(), 1);
    assert_eq!(browser.location_at(0), Some(Location::uri("recent:///")));
    assert_eq!(browser.column_snapshot(0).expect("Recent column").count, 0);
}

#[test]
fn filesystem_notifications_update_the_affected_column_incrementally() {
    let notify = Rc::new(RefCell::new(None::<WatchCallback>));
    let browser = Browser::new(Rc::new(WatchingFileSource {
        notify: notify.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    browser.move_selection(1);
    events.borrow_mut().clear();

    let callback = notify
        .borrow()
        .clone()
        .expect("the directory watcher should be installed");
    callback(DirectoryChange::Upsert(FileEntry {
        location: Location::local("/fixture/added"),
        native_name: OsString::from("added"),
        thumbnail_path: None,
        display_name: "added".into(),
        kind: EntryKind::File,
        size: MetadataValue::Known(4),
        modified_unix_seconds: MetadataValue::Known(1),
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        recent_uri: None,
    }));

    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::FocusChanged { .. }))
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::EntriesSpliced { depth: 0, splices, .. }
            if splices.len() == 1 && splices[0].removed == 0 && splices[0].entries.len() == 1
    )));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::SelectionSetChanged { .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { .. }))
    );
}

#[test]
fn background_directory_removal_does_not_request_focus() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let parent = Location::local("/fixture");
    browser.navigate(parent.clone());
    browser.handle_directory_change(0, &parent, DirectoryChange::Upsert(batch_entry("moved")));
    browser.preview(0, 0);
    assert_eq!(browser.active_depth(), Some(1));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.handle_directory_change(
        0,
        &parent,
        DirectoryChange::Remove(Location::local("/fixture/moved")),
    );

    assert_eq!(browser.active_depth(), Some(1));
    assert_eq!(
        browser
            .column_snapshot(0)
            .expect("parent column")
            .selected_positions,
        [0]
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesSpliced { depth: 0, .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::SelectionSetChanged { .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::FocusChanged { .. }))
    );
}

#[test]
fn active_directory_background_change_does_not_request_focus() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let parent = Location::local("/fixture");
    browser.navigate(parent.clone());
    browser.handle_directory_change(0, &parent, DirectoryChange::Upsert(batch_entry("alpha")));
    assert_eq!(browser.active_depth(), Some(0));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.handle_directory_change(0, &parent, DirectoryChange::Upsert(batch_entry("beta")));

    assert_eq!(browser.active_depth(), Some(0));
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesSpliced { depth: 0, .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::SelectionSetChanged { .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::FocusChanged { .. }))
    );
}

#[test]
fn ambiguous_filesystem_notifications_fall_back_to_reload() {
    let notify = Rc::new(RefCell::new(None::<WatchCallback>));
    let browser = Browser::new(Rc::new(WatchingFileSource {
        notify: notify.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    let callback = notify
        .borrow()
        .clone()
        .expect("the directory watcher should be installed");
    callback(DirectoryChange::Rescan);

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { depth: 0 }))
    );
}

fn await_result_paths(
    events: &std::sync::mpsc::Receiver<crate::services::SearchEvent>,
    expected: &[std::path::PathBuf],
) -> Result<(), String> {
    let expected = expected
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut last = None;
    while std::time::Instant::now() < deadline {
        if let Ok(crate::services::SearchEvent::Results {
            items, indexing, ..
        }) = events.recv_timeout(std::time::Duration::from_millis(100))
        {
            let paths = items
                .into_iter()
                .map(|item| item.path)
                .collect::<std::collections::BTreeSet<_>>();
            if !indexing && paths == expected {
                return Ok(());
            }
            last = Some(paths);
        }
    }
    Err(format!("expected {expected:?}, last results {last:?}"))
}

#[derive(Debug, Clone, Copy)]
enum ExternalChange {
    Upsert,
    Remove,
    Rescan,
    /// A rescan deferred during an in-app operation reloads the column in place.
    RescanAfterOperation,
}

#[test]
fn watched_directory_changes_refresh_the_filter_index_for_that_directory() {
    let mut failures = Vec::new();
    for change in [
        ExternalChange::Upsert,
        ExternalChange::Remove,
        ExternalChange::Rescan,
        ExternalChange::RescanAfterOperation,
    ] {
        let fixture = tempfile::tempdir().expect("fixture");
        let dir = fixture.path().to_path_buf();
        let needle = dir.join("needle.txt");
        if matches!(change, ExternalChange::Remove) {
            std::fs::write(&needle, "body").expect("needle");
        }
        let notify = Rc::new(RefCell::new(None::<WatchCallback>));
        let browser = Browser::new(Rc::new(WatchingFileSource {
            notify: notify.clone(),
        }));
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        browser.observe(move |event| observed.borrow_mut().push(event.clone()));
        browser.navigate(Location::local(&dir));
        let (handle, results) = crate::services::index_filter(dir.clone(), false, false);
        handle.query("needle");
        let initial = if matches!(change, ExternalChange::Remove) {
            vec![needle.clone()]
        } else {
            Vec::new()
        };
        await_result_paths(&results, &initial).expect("the initial filter results");
        events.borrow_mut().clear();
        let callback = notify
            .borrow()
            .clone()
            .expect("the directory watcher should be installed");

        let expected = match change {
            ExternalChange::Upsert => {
                std::fs::write(&needle, "body").expect("needle");
                callback(DirectoryChange::Upsert(fixture_entry(
                    needle.to_str().expect("utf-8 fixture"),
                )));
                vec![needle.clone()]
            }
            ExternalChange::Remove => {
                std::fs::remove_file(&needle).expect("remove needle");
                callback(DirectoryChange::Remove(Location::local(&needle)));
                Vec::new()
            }
            ExternalChange::Rescan => {
                std::fs::write(&needle, "body").expect("needle");
                callback(DirectoryChange::Rescan);
                assert!(
                    events
                        .borrow()
                        .iter()
                        .any(|event| matches!(event, BrowserEvent::ColumnReloaded { depth: 0 })),
                    "a Rescan reloads the column"
                );
                vec![needle.clone()]
            }
            ExternalChange::RescanAfterOperation => {
                std::fs::write(&needle, "body").expect("needle");
                browser.refresh_operation_columns(&[0]);
                vec![needle.clone()]
            }
        };
        if let Err(error) = await_result_paths(&results, &expected) {
            failures.push(format!("{change:?}: {error}"));
        }
        drop(handle);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

type FindAnswer = (
    Location,
    LocationIdentity,
    Rc<dyn Fn(Option<Location>)>,
    Rc<Cell<bool>>,
);

/// Watches every column like the local adapter's directory monitor, on a fake
/// filesystem where folders keep their identity through a rename.
#[derive(Default)]
struct DepartingSource {
    /// Holds parent lookups until `answer_finds`, like the adapter's worker thread.
    defer_finds: bool,
    pending_finds: RefCell<Vec<FindAnswer>>,
    boundary: Option<Location>,
    missing: RefCell<HashSet<Location>>,
    unreachable: RefCell<HashSet<Location>>,
    unlistable: RefCell<HashSet<Location>>,
    identities: FolderIdentities,
    watches: RefCell<Vec<(Location, WatchCallback)>>,
}

impl DepartingSource {
    fn directory_monitor(&self, location: &Location) -> WatchCallback {
        self.watches
            .borrow()
            .iter()
            .rev()
            .find(|(watched, _)| watched == location)
            .map(|(_, notify)| notify.clone())
            .expect("the directory monitor should be installed")
    }

    fn rename(&self, from: &Location, to: &Location) {
        self.identities.rename(from, to);
        self.missing.borrow_mut().insert(from.clone());
    }

    fn find(&self, parent: &Location, identity: LocationIdentity) -> Option<Location> {
        let listable = !self.unlistable.borrow().contains(parent);
        listable
            .then(|| self.identities.find(parent, identity))
            .flatten()
    }

    fn answer_finds(&self) {
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
            });
        } else {
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

    fn query_location_identity(
        &self,
        location: Location,
        emit: Rc<dyn Fn(Option<LocationIdentity>)>,
    ) -> LoadHandle {
        let present = !self.missing.borrow().contains(&location);
        emit(
            present
                .then(|| self.identities.identity(&location))
                .flatten(),
        );
        LoadHandle::new(|| {})
    }

    fn find_by_identity(
        &self,
        parent: Location,
        identity: LocationIdentity,
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

fn recorded_events(browser: &Rc<Browser>) -> Rc<RefCell<Vec<BrowserEvent>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    events
}

fn open_path(browser: &Browser) -> Vec<Location> {
    (0..)
        .map_while(|depth| browser.location_at(depth))
        .collect()
}

fn count_events(
    events: &RefCell<Vec<BrowserEvent>>,
    matches: impl Fn(&BrowserEvent) -> bool,
) -> usize {
    events
        .borrow()
        .iter()
        .filter(|event| matches(event))
        .count()
}

#[derive(Debug, Clone, Copy)]
enum RenameReport {
    ParentMonitor,
    /// The renamed folder is the open folder, whose own monitor reports only its removal.
    OwnMonitor,
}

#[test]
fn external_renames_rebase_filter_indexes_rooted_at_the_renamed_folder() {
    let mut failures = Vec::new();
    for report in [RenameReport::ParentMonitor, RenameReport::OwnMonitor] {
        let fixture = tempfile::tempdir().expect("fixture");
        let dir = fixture.path().to_path_buf();
        let old = dir.join("old");
        let new = dir.join("new");
        std::fs::create_dir(&old).expect("old folder");
        std::fs::write(old.join("needle.txt"), "body").expect("needle");
        let (old_location, new_location) = (Location::local(&old), Location::local(&new));
        let source = Rc::new(DepartingSource::default());
        source.identities.identify(&old_location);
        let browser = Browser::new(source.clone());
        let open = match report {
            RenameReport::ParentMonitor => &dir,
            RenameReport::OwnMonitor => &old,
        };
        browser.navigate(Location::local(open));
        let (handle, results) = crate::services::index_filter(old.clone(), false, true);
        handle.query("needle");
        await_result_paths(&results, &[old.join("needle.txt")])
            .expect("the initial filter results");

        std::fs::rename(&old, &new).expect("rename folder");
        source.rename(&old_location, &new_location);
        match report {
            RenameReport::ParentMonitor => {
                let mut entry = fixture_entry(new.to_str().expect("utf-8 fixture"));
                entry.kind = EntryKind::Directory;
                source.directory_monitor(&Location::local(&dir))(DirectoryChange::Move {
                    from: old_location.clone(),
                    entry,
                });
            }
            RenameReport::OwnMonitor => {
                source.directory_monitor(&old_location)(DirectoryChange::Remove(old_location))
            }
        }

        if let Err(error) = await_result_paths(&results, &[new.join("needle.txt")]) {
            failures.push(format!("{report:?}: {error}"));
        }
        drop(handle);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[derive(Debug, Clone, Copy)]
enum OutsideChange {
    RenamedInPlace,
    MovedIntoAnotherFolder,
    Deleted,
    DeletedWithItsParent,
    /// An execute-only parent cannot be listed to find the new name.
    RenamedUnderAnUnlistableParent,
    /// A new folder elsewhere in the parent got the deleted folder's inode.
    InodeReusedByAnotherFolder,
    /// Too many changes at once: the monitor asks for a rescan instead.
    DeletedInAMonitorBurst,
    Recreated,
    /// An unreachable folder may come back; leaving would lose the user's place.
    Unreachable,
}

#[test]
fn the_open_folder_reacts_to_its_own_removal_report() {
    let parent = Location::local("/fixture/a");
    let open = Location::local("/fixture/a/b");
    let renamed = Location::local("/fixture/a/renamed");
    let mut failures = Vec::new();
    for change in [
        OutsideChange::RenamedInPlace,
        OutsideChange::MovedIntoAnotherFolder,
        OutsideChange::Deleted,
        OutsideChange::DeletedWithItsParent,
        OutsideChange::RenamedUnderAnUnlistableParent,
        OutsideChange::InodeReusedByAnotherFolder,
        OutsideChange::DeletedInAMonitorBurst,
        OutsideChange::Recreated,
        OutsideChange::Unreachable,
    ] {
        let source = Rc::new(DepartingSource::default());
        source.identities.identify(&open);
        let browser = Browser::new(source.clone());
        browser.navigate(open.clone());
        let could_go_back = browser.can_go_back();
        let monitors_before = source.watches.borrow().len();
        let events = recorded_events(&browser);

        let expected = match change {
            OutsideChange::RenamedInPlace => {
                source.rename(&open, &renamed);
                renamed.clone()
            }
            OutsideChange::MovedIntoAnotherFolder => {
                source.rename(&open, &Location::local("/fixture/elsewhere/b"));
                parent.clone()
            }
            OutsideChange::Deleted => {
                source.missing.borrow_mut().insert(open.clone());
                parent.clone()
            }
            OutsideChange::DeletedWithItsParent => {
                source
                    .missing
                    .borrow_mut()
                    .extend([open.clone(), parent.clone()]);
                Location::local("/fixture")
            }
            OutsideChange::RenamedUnderAnUnlistableParent => {
                source.rename(&open, &renamed);
                source.unlistable.borrow_mut().insert(parent.clone());
                parent.clone()
            }
            OutsideChange::InodeReusedByAnotherFolder => {
                source
                    .identities
                    .reuse_inode(&open, &Location::local("/fixture/a/out"));
                source.missing.borrow_mut().insert(open.clone());
                parent.clone()
            }
            OutsideChange::DeletedInAMonitorBurst => {
                source.missing.borrow_mut().insert(open.clone());
                parent.clone()
            }
            OutsideChange::Recreated => {
                source.identities.identify(&open);
                open.clone()
            }
            OutsideChange::Unreachable => {
                source.unreachable.borrow_mut().insert(open.clone());
                open.clone()
            }
        };
        let report = match change {
            OutsideChange::DeletedInAMonitorBurst => DirectoryChange::Rescan,
            _ => DirectoryChange::Remove(open.clone()),
        };
        source.directory_monitor(&open)(report);

        let relocations = count_events(&events, |event| {
            matches!(event, BrowserEvent::ColumnsRelocated { from_depth: 0 })
        });
        let resets = count_events(&events, |event| matches!(event, BrowserEvent::Reset));
        let reloads = count_events(&events, |event| {
            matches!(event, BrowserEvent::ColumnReloaded { .. })
        });
        let rearmed = source.watches.borrow().len() > monitors_before;
        let (expected_relocations, expected_resets, expected_reloads) = match change {
            OutsideChange::RenamedInPlace => (1, 0, 0),
            OutsideChange::Unreachable => (0, 0, 0),
            // The rescan's reload fails, which shows the folder is gone.
            OutsideChange::DeletedInAMonitorBurst => (0, 1, 1),
            _ => (0, 1, 0),
        };
        if open_path(&browser) != [expected.clone()]
            || relocations != expected_relocations
            || resets != expected_resets
            || reloads != expected_reloads
            || rearmed == matches!(change, OutsideChange::Unreachable)
            || browser.can_go_back() != could_go_back
        {
            failures.push(format!(
                "{change:?}: expected [{expected:?}], got {:?}; ColumnsRelocated x{relocations} \
                 Reset x{resets} ColumnReloaded x{reloads} rearmed={rearmed} can_go_back \
                 {could_go_back}->{}",
                open_path(&browser),
                browser.can_go_back(),
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_root_removed_with_an_open_child_column_leaves_in_either_report_order() {
    let root = Location::local("/fixture/a");
    let open_child = Location::local("/fixture/a/b");
    let mut failures = Vec::new();
    for root_reported_first in [true, false] {
        let source = Rc::new(DepartingSource {
            defer_finds: true,
            ..DepartingSource::default()
        });
        source.identities.identify(&root);
        let browser = Browser::new(source.clone());
        browser.navigate(root.clone());
        browser.show_child(0, open_child.clone());
        source
            .missing
            .borrow_mut()
            .extend([root.clone(), open_child.clone()]);

        // One monitor batch, drained in either order; the parent lookup answers later.
        let reports = if root_reported_first {
            [root.clone(), open_child.clone()]
        } else {
            [open_child.clone(), root.clone()]
        };
        let monitor = source.directory_monitor(&root);
        for removed in reports {
            monitor(DirectoryChange::Remove(removed));
        }
        source.answer_finds();

        if open_path(&browser) != [Location::local("/fixture")] {
            failures.push(format!(
                "root reported first: {root_reported_first}, path {:?}",
                open_path(&browser)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_child_column_own_removal_report_is_left_to_its_parent_column() {
    let parent = Location::local("/fixture/a");
    let open = Location::local("/fixture/a/b");
    let source = Rc::new(DepartingSource::default());
    let browser = Browser::new(source.clone());
    browser.navigate(parent.clone());
    browser.show_child(0, open.clone());
    let events = recorded_events(&browser);
    source.missing.borrow_mut().insert(open.clone());

    source.directory_monitor(&open)(DirectoryChange::Remove(open.clone()));

    assert_eq!(open_path(&browser), [parent, open]);
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
}

#[derive(Debug, Clone, Copy)]
enum BoundaryDeparture {
    /// The boundary folder itself is renamed, so its new name lies outside.
    RenamedOutside,
    /// Every folder left inside the boundary is gone too.
    DeletedWithTheBoundary,
}

#[test]
fn a_departure_never_leaves_the_navigation_boundary() {
    let boundary = Location::local("/fixture/device");
    let mut failures = Vec::new();
    for departure in [
        BoundaryDeparture::RenamedOutside,
        BoundaryDeparture::DeletedWithTheBoundary,
    ] {
        let open = match departure {
            BoundaryDeparture::RenamedOutside => boundary.clone(),
            BoundaryDeparture::DeletedWithTheBoundary => Location::local("/fixture/device/open"),
        };
        let source = Rc::new(DepartingSource {
            boundary: Some(boundary.clone()),
            ..DepartingSource::default()
        });
        source.identities.identify(&open);
        let browser = Browser::new(source.clone());
        browser.navigate(open.clone());
        let events = recorded_events(&browser);
        match departure {
            BoundaryDeparture::RenamedOutside => {
                source.rename(&open, &Location::local("/fixture/device-renamed"))
            }
            BoundaryDeparture::DeletedWithTheBoundary => source
                .missing
                .borrow_mut()
                .extend([open.clone(), boundary.clone()]),
        }

        source.directory_monitor(&open)(DirectoryChange::Remove(open.clone()));

        // With nothing allowed to show instead, the folder shows its read error.
        let reloads = count_events(&events, |event| {
            matches!(event, BrowserEvent::ColumnReloaded { depth: 0 })
        });
        let navigated = count_events(&events, |event| {
            matches!(
                event,
                BrowserEvent::Reset | BrowserEvent::ColumnsRelocated { .. }
            )
        });
        if open_path(&browser) != [open.clone()] || reloads != 1 || navigated != 0 {
            failures.push(format!(
                "{departure:?}: path {:?}, ColumnReloaded x{reloads}, navigated x{navigated}",
                open_path(&browser)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[derive(Debug, Clone, Copy)]
enum Departure {
    Delete,
    Rename,
    MoveIntoAnotherFolder,
    DeleteParent,
    /// The removal of the open child column and of the folder share one monitor batch.
    DeleteWithAnOpenChildColumn,
    /// More entries than a monitor batch holds, so the monitor asks for a rescan.
    DeleteAFullFolder,
    /// Deleted and created again before the report arrives, as a build does to `dist/`.
    Recreate,
}

#[test]
fn the_open_directory_follows_an_outside_rename_and_leaves_an_outside_removal() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("async test lock");
    let context = gtk::glib::MainContext::default();
    let pump_for = |condition: &dyn Fn() -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !condition() && std::time::Instant::now() < deadline {
            while context.iteration(false) {}
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    };
    let mut failures = Vec::new();
    for departure in [
        Departure::Delete,
        Departure::Rename,
        Departure::MoveIntoAnotherFolder,
        Departure::DeleteParent,
        Departure::DeleteWithAnOpenChildColumn,
        Departure::DeleteAFullFolder,
        Departure::Recreate,
    ] {
        let fixture = tempfile::tempdir().expect("fixture");
        let parent = fixture.path().join("parent");
        let open = parent.join("open");
        std::fs::create_dir_all(open.join("inner")).expect("open folder");
        std::fs::write(open.join("kept.txt"), "kept").expect("file");
        std::fs::create_dir(parent.join("elsewhere")).expect("sibling folder");
        if matches!(departure, Departure::DeleteAFullFolder) {
            for index in 0..5_000 {
                std::fs::write(open.join(format!("file-{index}")), "").expect("file");
            }
        }
        let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
        let events = recorded_events(&browser);
        browser.navigate(Location::local(&open));
        pump_for(&|| {
            events
                .borrow()
                .iter()
                .any(|event| matches!(event, BrowserEvent::LoadFinished { depth: 0, .. }))
                && browser.root_identity.borrow().is_some()
        });
        if matches!(departure, Departure::DeleteWithAnOpenChildColumn) {
            browser.show_child(0, Location::local(open.join("inner")));
            pump_for(&|| {
                events
                    .borrow()
                    .iter()
                    .any(|event| matches!(event, BrowserEvent::LoadFinished { depth: 1, .. }))
            });
        }
        let could_go_back = browser.can_go_back();

        let expected = match departure {
            Departure::Delete => {
                std::fs::remove_dir_all(&open).expect("delete");
                parent.clone()
            }
            Departure::Rename => {
                let renamed = parent.join("open-renamed");
                std::fs::rename(&open, &renamed).expect("rename");
                renamed
            }
            Departure::MoveIntoAnotherFolder => {
                std::fs::rename(&open, parent.join("elsewhere").join("open")).expect("move");
                parent.clone()
            }
            Departure::DeleteParent => {
                std::fs::remove_dir_all(&parent).expect("delete parent");
                fixture.path().to_path_buf()
            }
            Departure::DeleteWithAnOpenChildColumn | Departure::DeleteAFullFolder => {
                std::fs::remove_dir_all(&open).expect("delete");
                parent.clone()
            }
            Departure::Recreate => {
                std::fs::remove_dir_all(&open).expect("delete");
                std::fs::create_dir(&open).expect("recreate");
                std::fs::write(open.join("fresh.txt"), "fresh").expect("file");
                open.clone()
            }
        };
        let expected = Location::local(&expected);
        pump_for(&|| browser.location_at(0).as_ref() == Some(&expected));
        if matches!(departure, Departure::Recreate) {
            pump_for(&|| column_names(&browser, 0) == ["fresh.txt"]);
            std::fs::write(open.join("later.txt"), "later").expect("file");
            pump_for(&|| column_names(&browser, 0) == ["fresh.txt", "later.txt"]);
            let names = column_names(&browser, 0);
            if names != ["fresh.txt", "later.txt"] {
                failures.push(format!(
                    "{departure:?}: the reloaded listing shows {names:?}"
                ));
            }
        }
        if open_path(&browser) != [expected.clone()] || browser.can_go_back() != could_go_back {
            failures.push(format!(
                "{departure:?}: expected [{:?}], got {:?}; can_go_back {could_go_back}->{}",
                expected.display_name(),
                open_path(&browser)
                    .iter()
                    .map(Location::display_name)
                    .collect::<Vec<_>>(),
                browser.can_go_back(),
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
