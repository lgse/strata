// SPDX-License-Identifier: MIT

use super::*;

#[path = "../operation_updates/tests.rs"]
mod operation_updates;

struct TreeSource {
    root: Location,
    renamed_root: RefCell<Option<Location>>,
    missing: RefCell<Vec<Location>>,
    renamed: Cell<bool>,
    loads: RefCell<Vec<Location>>,
    watches: RefCell<Vec<Location>>,
    cancelled_watches: Rc<RefCell<Vec<Location>>>,
    identities: FolderIdentities,
}

fn child(parent: &Location, name: &str) -> Location {
    parent
        .child(std::ffi::OsStr::new(name))
        .expect("child location")
}

fn named(parent: &Location, name: &str, directory: bool) -> FileEntry {
    FileEntry {
        location: child(parent, name),
        native_name: name.into(),
        display_name: name.into(),
        kind: if directory {
            EntryKind::Directory
        } else {
            EntryKind::File
        },
        thumbnail_path: None,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        recent_uri: None,
    }
}

impl FileSource for TreeSource {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        if self.missing.borrow().contains(location) {
            Err(LocationValidationError::Missing)
        } else {
            Ok(())
        }
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        self.loads.borrow_mut().push(request.location.clone());
        let entries = if request.location == self.root
            || self.renamed_root.borrow().as_ref() == Some(&request.location)
        {
            vec![
                named(
                    &request.location,
                    if self.renamed.get() { "renamed" } else { "old" },
                    true,
                ),
                named(&request.location, "sibling.txt", false),
            ]
        } else if request.location.file_name().as_deref() == Some(std::ffi::OsStr::new("nested")) {
            vec![named(&request.location, "leaf.txt", false)]
        } else {
            vec![named(&request.location, "nested", true)]
        };
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries,
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }

    fn watch(&self, location: Location, _: bool, _: WatchCallback) -> Option<LoadHandle> {
        self.watches.borrow_mut().push(location.clone());
        let cancelled = self.cancelled_watches.clone();
        Some(LoadHandle::new(move || {
            cancelled.borrow_mut().push(location)
        }))
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
        emit(self.identities.find(&parent, identity));
        LoadHandle::new(|| {})
    }
}

fn tree(remote: bool) -> (Rc<Browser>, Rc<TreeSource>) {
    let root = if remote {
        Location::uri("smb://host/share")
    } else {
        Location::local("/fixture")
    };
    let source = Rc::new(TreeSource {
        root: root.clone(),
        renamed_root: RefCell::new(None),
        missing: RefCell::new(Vec::new()),
        renamed: Cell::new(false),
        loads: RefCell::new(Vec::new()),
        watches: RefCell::new(Vec::new()),
        cancelled_watches: Rc::new(RefCell::new(Vec::new())),
        identities: FolderIdentities::default(),
    });
    source.identities.identify(&root);
    let browser = Browser::new(source.clone());
    browser.navigate(root);
    browser.activate(0, 0);
    browser.activate(1, 0);
    browser.select(2, 0);
    (browser, source)
}

#[test]
fn successful_open_directory_rename_preserves_parent_and_descendant_selections() {
    for remote in [false, true] {
        for sibling_selected in [false, true] {
            let (browser, source) = tree(remote);
            browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
            let entry = browser.entry_at(0, 0).expect("rename target");
            browser.select(0, usize::from(sibling_selected));
            let parent_request = browser.column_request_id(0);
            let events = Rc::new(RefCell::new(Vec::new()));
            let observed = events.clone();
            browser.observe(move |event| observed.borrow_mut().push(event.clone()));
            source.renamed.set(true);
            browser.rename(entry, "renamed".into());

            let renamed = child(&source.root, "renamed");
            assert_eq!(browser.location_at(1), Some(renamed.clone()));
            assert_eq!(browser.location_at(2), Some(child(&renamed, "nested")));
            assert_eq!(browser.active_depth(), Some(0));
            assert_eq!(
                browser.selected_entries()[0].display_name,
                if sibling_selected {
                    "sibling.txt"
                } else {
                    "renamed"
                }
            );
            assert_eq!(browser.selected_positions(1), [0]);
            assert_eq!(browser.selected_positions(2), [0]);
            assert!(!events.borrow().iter().any(|event| matches!(
                event,
                BrowserEvent::Reset
                    | BrowserEvent::FocusChanged { .. }
                    | BrowserEvent::OpenRequested { .. }
            )));
            if !remote {
                assert_eq!(browser.column_request_id(0), parent_request);
            }
            assert!(
                source
                    .cancelled_watches
                    .borrow()
                    .contains(&child(&source.root, "old"))
            );
            assert!(!source.cancelled_watches.borrow().contains(&source.root));
            assert!(source.watches.borrow().contains(&renamed));

            let (generation, _, _) = browser.pending_undo_rename().expect("pending Rename undo");
            source.renamed.set(false);
            assert!(browser.undo_rename(generation));

            let restored = child(&source.root, "old");
            assert_eq!(browser.location_at(1), Some(restored.clone()));
            assert_eq!(browser.location_at(2), Some(child(&restored, "nested")));
            assert_eq!(browser.active_depth(), Some(0));
            assert_eq!(
                browser.selected_entries()[0].display_name,
                if sibling_selected {
                    "sibling.txt"
                } else {
                    "old"
                }
            );
            assert_eq!(browser.selected_positions(1), [0]);
            assert_eq!(browser.selected_positions(2), [0]);
            assert_eq!(
                location_changes(&events.borrow()),
                [
                    relocated(&restored, &renamed),
                    relocated(&renamed, &restored)
                ]
            );
        }
    }
}

#[test]
fn external_directory_moves_relocate_only_the_open_suffix_and_ignore_stale_watchers() {
    let (browser, source) = tree(false);
    let entry = named(&source.root, "renamed", true);
    let old = child(&source.root, "old");
    let root_request = browser.column_request_id(0);
    let descendant_request = browser.column_request_id(2);
    source.renamed.set(true);
    browser.handle_directory_change(
        0,
        &source.root,
        DirectoryChange::Move {
            from: old.clone(),
            entry: entry.clone(),
        },
    );
    assert_eq!(browser.column_request_id(0), root_request);
    assert_ne!(browser.column_request_id(2), descendant_request);
    assert_eq!(browser.active_depth(), Some(2));
    assert_eq!(
        browser.selected_entries()[0].location,
        child(&child(&entry.location, "nested"), "leaf.txt")
    );
    let loads = source.loads.borrow().len();
    browser.handle_directory_change(
        1,
        &old,
        DirectoryChange::Remove(child(&entry.location, "nested")),
    );
    assert_eq!(source.loads.borrow().len(), loads);
    assert_eq!(
        browser.location_at(2),
        Some(child(&entry.location, "nested"))
    );
}

#[test]
fn an_external_root_rename_relocates_every_open_column_and_keeps_the_selection() {
    let (browser, source) = tree(false);
    let could_go_back = browser.can_go_back();
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let renamed = Location::local("/fixture-renamed");
    source.renamed_root.replace(Some(renamed.clone()));
    source.identities.rename(&source.root, &renamed);
    source.missing.borrow_mut().push(source.root.clone());
    let old_paths = (0..3)
        .map(|depth| browser.location_at(depth).expect("open column"))
        .collect::<Vec<_>>();

    browser.handle_directory_change(
        0,
        &source.root,
        DirectoryChange::Remove(source.root.clone()),
    );

    assert_eq!(browser.location_at(0), Some(renamed.clone()));
    assert_eq!(browser.location_at(1), Some(child(&renamed, "old")));
    assert_eq!(
        browser.location_at(2),
        Some(child(&child(&renamed, "old"), "nested"))
    );
    assert_eq!(browser.active_depth(), Some(2));
    assert_eq!(
        browser.selected_entries()[0].location,
        child(&child(&child(&renamed, "old"), "nested"), "leaf.txt")
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnsRelocated { from_depth: 0 }))
    );
    assert!(!events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::Reset
            | BrowserEvent::NavigationStarting { .. }
            | BrowserEvent::ColumnReloaded { .. }
    )));
    assert!(
        location_changes(&events.borrow()).is_empty(),
        "item customizations do not follow changes made outside Strata"
    );
    assert_eq!(browser.can_go_back(), could_go_back);
    for old in old_paths {
        assert!(source.cancelled_watches.borrow().contains(&old), "{old:?}");
    }
    let watches = source.watches.borrow();
    assert_eq!(
        watches[watches.len() - 3..],
        [
            renamed.clone(),
            child(&renamed, "old"),
            child(&child(&renamed, "old"), "nested")
        ]
    );
}

#[test]
fn removing_the_root_with_open_child_columns_returns_to_the_nearest_existing_ancestor() {
    let (browser, source) = tree(false);
    let could_go_back = browser.can_go_back();
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let open_child = child(&source.root, "old");
    source
        .missing
        .borrow_mut()
        .extend([source.root.clone(), open_child.clone()]);

    // `rm -r` reports the open child and the root itself in one batch; the child's
    // removal reloads the root before the root's own removal is handled.
    for removed in [open_child, source.root.clone()] {
        browser.handle_directory_change(0, &source.root, DirectoryChange::Remove(removed));
    }

    let columns = (0..)
        .map_while(|depth| browser.location_at(depth))
        .collect::<Vec<_>>();
    assert_eq!(columns, [Location::local("/")]);
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::Reset))
    );
    assert_eq!(browser.can_go_back(), could_go_back);
}

#[test]
fn a_rename_superseded_by_another_operation_still_reports_its_relocation() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let operations = Rc::new(crate::test_support::operations::HeldOperations::default());
    browser.set_operation_provider(operations.clone());
    let old = Location::local("/fixture/docs");
    let rename = browser
        .rename(fixture_entry("/fixture/docs"), "docs2".into())
        .expect("rename started");
    browser.delete(vec![fixture_entry("/fixture/other.txt")], false);
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    operations.emit(rename, OperationEvent::Renamed { request_id: rename });

    assert_eq!(
        location_changes(&events.borrow()),
        [relocated(&old, &Location::local("/fixture/docs2"))]
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::RenameCompleted { .. }))
    );
}
