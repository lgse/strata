// SPDX-License-Identifier: MIT

use super::*;

struct TreeSource {
    root: Location,
    renamed: Cell<bool>,
    loads: RefCell<Vec<Location>>,
    watches: RefCell<Vec<Location>>,
    cancelled_watches: Rc<RefCell<Vec<Location>>>,
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
        is_hidden: false,
    }
}

impl FileSource for TreeSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        self.loads.borrow_mut().push(request.location.clone());
        let entries = if request.location == self.root {
            vec![
                named(
                    &self.root,
                    if self.renamed.get() { "renamed" } else { "old" },
                    true,
                ),
                named(&self.root, "sibling.txt", false),
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
}

fn tree(remote: bool) -> (Rc<Browser>, Rc<TreeSource>) {
    let root = if remote {
        Location::uri("smb://host/share")
    } else {
        Location::local("/fixture")
    };
    let source = Rc::new(TreeSource {
        root: root.clone(),
        renamed: Cell::new(false),
        loads: RefCell::new(Vec::new()),
        watches: RefCell::new(Vec::new()),
        cancelled_watches: Rc::new(RefCell::new(Vec::new())),
    });
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
