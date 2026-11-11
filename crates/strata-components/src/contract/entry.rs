//! Entries: the ids, kinds and display fields every collection view and its controllers read,
//! through the index-based [`EntrySource`].
//!
//! Sizes, dates and other columns stay out of this contract: the List view reads them through
//! its own accessors.

use std::cmp::Ordering;
use std::collections::HashMap;

#[cfg(test)]
mod tests;

/// An entry's opaque id, assigned by the host.
///
/// The same entry keeps the same id for the life of the process: across re-sorts, live
/// updates, a full re-list (a refresh) and a revisit of its folder. So selections, cursors and
/// drags held by id survive a re-sort or a refresh, and a selection remembered per folder
/// still names the right entries when the folder is shown again. It is unique across every
/// source the process shows at once (every pane, column and window), not just within one
/// listing: a drag between panes carries ids, and the Columns view holds one selection per
/// column. A host that numbers each listing on its own must intern its ids instead, for
/// example through a process-wide location-to-id table.
///
/// An id never names a different entry later in the process: when its entry goes, the id is
/// retired, never reused. So a selection remembered for a folder can't pick up the wrong
/// entry when the folder is shown again.
///
/// A rename is the exception to the lifetime rule: it may give the entry a new id. A host
/// that keeps the id across a rename says so in its [`EntrySource`] docs. A host that changes
/// it moves what it holds by the old id to the new one; C1a's selection model provides
/// `Selection::relocate(from, to)` for that.
///
/// Used by: C1a, C1b, C1c, C1d, C1e, C2, C3, C4, C14b, C22, C25, X2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct EntryId(u64);

impl EntryId {
    /// The id with the host's raw value `raw`.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The host's raw value.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// What an entry is, as far as the views care: whether it opens as a folder, takes drops and
/// which icon it falls back to.
///
/// Used by: C1b (click modes per kind), C1e, C2, C3, C4, C22.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A folder.
    Folder,
    /// A symbolic link, with what it points to.
    Symlink(LinkTarget),
    /// Anything else: a device, socket or pipe.
    Special,
}

impl EntryKind {
    /// Whether the entry behaves as a folder: a folder, or a link to one. Folder-like entries
    /// open as folders, are drop candidates, and fall back to the Folder icon.
    pub const fn is_folder_like(self) -> bool {
        matches!(self, Self::Folder | Self::Symlink(LinkTarget::Folder))
    }
}

/// What a symbolic link points to.
///
/// Used by: C1b, C1e, C2, C3, C4, C22 (through [`EntryKind`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkTarget {
    /// A file, or anything that isn't a folder.
    File,
    /// A folder.
    Folder,
    /// Nothing: the link is dangling, or its target couldn't be read.
    Broken,
}

/// An entry's thumbnail key: an opaque value the host derives from what the thumbnail shows
/// (a hash of the URI, size and modification time, say), so it changes when the content does.
///
/// An entry with no thumbnail key never gets one (folders, unsupported types).
///
/// Used by: C1e, C3, C25.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ThumbKey(u64);

impl ThumbKey {
    /// The key with the host's raw value `raw`.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The host's raw value.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One entry's display fields, borrowed from its [`EntrySource`], so a mounted row costs one
/// call and copies nothing.
///
/// Fields may be added: build one with [`Entry::new`].
///
/// Used by: C1c (type-ahead), C1e, C2, C3, C4, C14b, C22.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Entry<'a> {
    /// The entry's id.
    pub id: EntryId,
    /// The name the row shows.
    pub name: &'a str,
    /// What the entry is.
    pub kind: EntryKind,
    /// The icon's catalog name: the output of the host's file-name-to-icon mapping, a plain
    /// string so this contract needs no icon type.
    ///
    /// A component resolves it through the Look's `icon_named(icon)`. When that finds nothing
    /// (an unknown or empty name), it draws the Look's semantic Folder icon if
    /// [`EntryKind::is_folder_like`], else its File icon.
    pub icon: &'a str,
    /// The thumbnail key, if the entry can have a thumbnail.
    pub thumbnail: Option<ThumbKey>,
}

impl<'a> Entry<'a> {
    /// An entry with no thumbnail.
    pub const fn new(id: EntryId, name: &'a str, kind: EntryKind, icon: &'a str) -> Self {
        Self {
            id,
            name,
            kind,
            icon,
            thumbnail: None,
        }
    }

    /// The same entry, with thumbnail key `key`.
    pub const fn thumbnail(mut self, key: ThumbKey) -> Self {
        self.thumbnail = Some(key);
        self
    }
}

/// The entries a collection shows, in display order, read by index so that 100,000 rows never
/// copy into the view.
///
/// Controllers (selection, keyboard, marquee) take `&dyn EntrySource` with no app state;
/// elements take it by accessor, `fn(&S) -> &E` with `E: EntrySource + ?Sized`. Sorting and
/// filtering are the host's: a re-sort or a refresh may change the order and the entries, never
/// an entry's id (see [`EntryId`]).
///
/// Used by: C1a, C1c, C1d, C1e, C2, C3, C4, C14b, C22.
pub trait EntrySource {
    /// How many entries there are.
    fn len(&self) -> usize;

    /// Whether there are none.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The id of the entry at `index`, which is below [`EntrySource::len`]. Callers call it
    /// in loops over ranges, so keep it cheap.
    fn id_at(&self, index: usize) -> EntryId;

    /// Where the entry `id` is now, or `None` if it's gone. Expected to be O(1): selections
    /// resolve ranges through it.
    fn index_of(&self, id: EntryId) -> Option<usize>;

    /// The display fields of the entry at `index`, which is below [`EntrySource::len`].
    fn entry(&self, index: usize) -> Entry<'_>;
}

/// An owned entry in a [`ListSource`].
///
/// Used by: the tests and gallery pages of C1a, C1c, C1d, C1e, C2, C3, C4, C14b and C22.
#[derive(Clone, Debug, PartialEq)]
pub struct ListEntry {
    /// The entry's id: unique across every source shown at once, and the same for the same
    /// entry for the life of the process (see [`EntryId`]).
    pub id: EntryId,
    /// The name the row shows.
    pub name: String,
    /// What the entry is.
    pub kind: EntryKind,
    /// The icon's catalog name; see [`Entry::icon`].
    pub icon: String,
    /// The thumbnail key, if any.
    pub thumbnail: Option<ThumbKey>,
}

impl ListEntry {
    /// An entry with no thumbnail.
    pub fn new(
        id: EntryId,
        name: impl Into<String>,
        kind: EntryKind,
        icon: impl Into<String>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            kind,
            icon: icon.into(),
            thumbnail: None,
        }
    }
}

/// An in-memory [`EntrySource`] for tests and gallery pages: a list of entries with an id
/// index, which it keeps up to date across re-sorts, removals and replacements, the way a
/// live listing changes.
///
/// Used by: the tests and gallery pages of C1a, C1c, C1d, C1e, C2, C3, C4, C14b and C22.
#[derive(Clone, Debug, Default)]
pub struct ListSource {
    entries: Vec<ListEntry>,
    index: HashMap<EntryId, usize>,
}

impl ListSource {
    /// A source over `entries`, in order. Ids must be unique.
    pub fn new(entries: Vec<ListEntry>) -> Self {
        let mut source = Self {
            entries,
            index: HashMap::new(),
        };
        source.reindex();
        source
    }

    /// The entries, in display order.
    pub fn entries(&self) -> &[ListEntry] {
        &self.entries
    }

    /// Replaces every entry, as a refresh does. Ids must be unique, and an entry that is still
    /// there keeps its id (a rename aside, see [`EntryId`]).
    pub fn replace(&mut self, entries: Vec<ListEntry>) {
        self.entries = entries;
        self.reindex();
    }

    /// Re-sorts the entries, keeping their ids.
    pub fn sort_by(&mut self, compare: impl FnMut(&ListEntry, &ListEntry) -> Ordering) {
        self.entries.sort_by(compare);
        self.reindex();
    }

    /// Keeps only the entries `keep` returns true for, as a live removal does.
    pub fn retain(&mut self, keep: impl FnMut(&ListEntry) -> bool) {
        self.entries.retain(keep);
        self.reindex();
    }

    fn reindex(&mut self) {
        self.index.clear();
        for (i, e) in self.entries.iter().enumerate() {
            let previous = self.index.insert(e.id, i);
            debug_assert!(previous.is_none(), "duplicate entry id {:?}", e.id);
        }
    }
}

impl EntrySource for ListSource {
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn id_at(&self, index: usize) -> EntryId {
        self.entries[index].id
    }

    fn index_of(&self, id: EntryId) -> Option<usize> {
        self.index.get(&id).copied()
    }

    fn entry(&self, index: usize) -> Entry<'_> {
        let e = &self.entries[index];
        let entry = Entry::new(e.id, &e.name, e.kind, &e.icon);
        match e.thumbnail {
            Some(key) => entry.thumbnail(key),
            None => entry,
        }
    }
}
