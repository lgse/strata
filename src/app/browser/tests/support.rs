// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn assert_invalid_creation_is_rejected(name: &str, create: impl FnOnce(&Rc<Browser>)) {
    let expected = validate_basename(name).expect_err("invalid fixture name");
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    create(&browser);

    assert_eq!(browser.current_operation.get(), None);
    assert!(browser.operation_load.borrow().is_none());
    assert!(matches!(
        events.borrow().as_slice(),
        [BrowserEvent::OperationFailed { message, .. }] if message == expected
    ));
}

pub(super) type WatchCallback = Rc<dyn Fn(DirectoryChange)>;

/// Identities of fake folders, so a source can find a folder after a rename.
#[derive(Default)]
pub(super) struct FolderIdentities(RefCell<HashMap<Location, ItemIdentity>>);

pub(super) fn trash_undo(locations: Vec<Location>) -> UndoEntry {
    UndoEntry::Trash {
        locations,
        originals: HashMap::new(),
    }
}

pub(super) fn folder_identity(inode: u64, born: i64) -> ItemIdentity {
    ItemIdentity {
        evidence: Evidence {
            device: 1,
            inode,
            kind: rustix::fs::FileType::Directory,
            birth: Some((born, 0)),
            handle: None,
            attributes: None,
        },
        traits: IdentityTraits {
            numbers: InodeNumbers::Kept,
            folder_births: true,
            made_up_births: false,
        },
    }
}

pub(super) fn identified_stamp(identity: ItemIdentity) -> DirectoryStamp {
    DirectoryStamp {
        modified: (0, 0),
        changed: Some((0, 0)),
        identity: Some(identity),
        mount: Some(MountToken::Native(1)),
        outside_writers: false,
        coarse_window: None,
        daemon_cache: None,
        read_at: std::time::UNIX_EPOCH,
    }
}

impl FolderIdentities {
    pub(super) fn identify(&self, location: &Location) {
        let mut identities = self.0.borrow_mut();
        let born = identities.len() as u64 + 1;
        identities.insert(location.clone(), folder_identity(born, born as i64));
    }

    /// A new folder at `to` that the filesystem gave the deleted `from` folder's inode.
    pub(super) fn reuse_inode(&self, from: &Location, to: &Location) {
        let mut identities = self.0.borrow_mut();
        let deleted = identities.remove(from).expect("an identified folder");
        identities.insert(to.clone(), folder_identity(deleted.evidence.inode, 1_000));
    }

    pub(super) fn rename(&self, from: &Location, to: &Location) {
        let mut identities = self.0.borrow_mut();
        let identity = identities.remove(from).expect("an identified folder");
        identities.insert(to.clone(), identity);
    }

    pub(super) fn stamp(&self, request: &DirectoryRequest, emit: &Rc<dyn Fn(DirectoryEvent)>) {
        if let Some(identity) = self.0.borrow().get(&request.location).copied() {
            emit(DirectoryEvent::Stamp {
                request_id: request.id,
                stamp: Box::new(identified_stamp(identity)),
            });
        }
    }

    /// Like the local adapter, searches the nearest ancestor of `departed` that is not
    /// `missing` for the folder, renamed itself or moved with an ancestor renamed there.
    pub(super) fn find(
        &self,
        departed: &Location,
        identity: ItemIdentity,
        missing: impl Fn(&Location) -> bool,
    ) -> Option<Location> {
        let mut renamed = departed.clone();
        let mut below = Vec::new();
        let searched = loop {
            let parent = renamed.parent()?;
            if !missing(&parent) {
                break parent;
            }
            below.push(renamed.file_name()?);
            renamed = parent;
        };
        let moved_with_a_renamed_entry = |location: &Location| {
            below
                .iter()
                .try_fold(location.clone(), |folder, name| {
                    (folder.file_name().as_ref() == Some(name))
                        .then(|| folder.parent())
                        .flatten()
                })
                .and_then(|renamed| renamed.parent())
                .is_some_and(|parent| parent == searched)
        };
        self.0
            .borrow()
            .iter()
            .find(|(location, candidate)| {
                identity.same(candidate) && moved_with_a_renamed_entry(location)
            })
            .map(|(location, _)| location.clone())
    }

    /// Renames the folder at `from`, carrying along every identified folder inside it, and
    /// returns the old locations of those folders.
    pub(super) fn rename_tree(&self, from: &Location, to: &Location) -> Vec<Location> {
        let mut identities = self.0.borrow_mut();
        let moved = identities
            .keys()
            .filter(|location| location.is_within(from))
            .cloned()
            .collect::<Vec<_>>();
        for location in &moved {
            let identity = identities.remove(location).expect("an identified folder");
            let relocated = location
                .rebase(from, to)
                .expect("a folder inside the renamed one");
            identities.insert(relocated, identity);
        }
        moved
    }
}
pub(super) fn fixture_entry(path: &str) -> FileEntry {
    entry_at(Location::local(path), EntryKind::File)
}

pub(super) fn directory_entry(path: &str) -> FileEntry {
    entry_at(Location::local(path), EntryKind::Directory)
}

/// An entry named after its location with every detail still unknown.
pub(super) fn entry_at(location: Location, kind: EntryKind) -> FileEntry {
    let name = location.display_name();
    FileEntry {
        location,
        thumbnail_path: None,
        native_name: OsString::from(name.clone()),
        display_name: name,
        kind,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        recent_uri: None,
    }
}
pub(super) fn batch_entry(name: &str) -> FileEntry {
    FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        native_name: OsString::from(name),
        thumbnail_path: None,
        display_name: name.into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        recent_uri: None,
    }
}

pub(super) fn trash_entry(name: &str) -> FileEntry {
    FileEntry {
        location: Location::uri(format!("trash:///{name}")),
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        ..batch_entry(name)
    }
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum LocationChange {
    Relocated {
        from: Location,
        to: Location,
        merged: bool,
    },
    Removed {
        location: Location,
        trash_identity: Option<ItemIdentity>,
    },
    Restored(Location, Option<ItemIdentity>),
}

pub(super) fn location_changes(events: &[BrowserEvent]) -> Vec<LocationChange> {
    events
        .iter()
        .filter_map(|event| match event {
            BrowserEvent::ItemRelocated { from, to, merged } => Some(LocationChange::Relocated {
                from: from.clone(),
                to: to.clone(),
                merged: *merged,
            }),
            BrowserEvent::ItemRemoved {
                location,
                trash_identity,
            } => Some(LocationChange::Removed {
                location: location.clone(),
                trash_identity: *trash_identity,
            }),
            BrowserEvent::ItemRestored { location, identity } => {
                Some(LocationChange::Restored(location.clone(), *identity))
            }
            _ => None,
        })
        .collect()
}

pub(super) fn relocated(from: &Location, to: &Location) -> LocationChange {
    LocationChange::Relocated {
        from: from.clone(),
        to: to.clone(),
        merged: false,
    }
}

pub(super) fn removed(location: &Location) -> LocationChange {
    LocationChange::Removed {
        location: location.clone(),
        trash_identity: None,
    }
}

/// The deadline only turns a hang into a deterministic failure.
pub(super) fn pump_until(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !condition() && std::time::Instant::now() < deadline {
        glib::MainContext::default().iteration(true);
    }
}

pub(super) fn finish_count(events: &RefCell<Vec<BrowserEvent>>) -> usize {
    events
        .borrow()
        .iter()
        .filter(|event| matches!(event, BrowserEvent::SortingFinished { .. }))
        .count()
}

pub(super) fn start_count(events: &RefCell<Vec<BrowserEvent>>) -> usize {
    events
        .borrow()
        .iter()
        .filter(|event| matches!(event, BrowserEvent::SortingStarted { .. }))
        .count()
}

pub(super) fn replaced_count(events: &RefCell<Vec<BrowserEvent>>) -> usize {
    events
        .borrow()
        .iter()
        .filter(|event| matches!(event, BrowserEvent::EntriesReplaced { .. }))
        .count()
}

pub(super) fn column_names(browser: &Browser, depth: usize) -> Vec<String> {
    browser.state.borrow().columns[depth]
        .entries
        .iter()
        .map(|entry| entry.display_name.clone())
        .collect()
}

pub(super) fn staged_entry(
    name: &str,
    kind: EntryKind,
    size: MetadataValue<u64>,
    modified: i64,
) -> FileEntry {
    FileEntry {
        kind,
        size,
        modified_unix_seconds: MetadataValue::Known(modified),
        created_unix_seconds: MetadataValue::Unknown,
        ..batch_entry(name)
    }
}
