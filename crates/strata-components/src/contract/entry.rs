// SPDX-License-Identifier: MIT

use std::cmp::Ordering;
use std::collections::HashMap;

#[cfg(test)]
mod tests;

/// An entry's opaque id, assigned by the host.
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

/// What an entry is, as far as the views care: whether it opens as a folder, takes drops and which icon it falls back to.
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
    /// Whether the entry behaves as a folder: a folder, or a link to one.
    pub const fn is_folder_like(self) -> bool {
        matches!(self, Self::Folder | Self::Symlink(LinkTarget::Folder))
    }
}

/// What a symbolic link points to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkTarget {
    /// A file, or anything that isn't a folder.
    File,
    /// A folder.
    Folder,
    /// Nothing: the link is dangling, or its target couldn't be read.
    Broken,
}

/// An entry's thumbnail key: an opaque value the host derives from what the thumbnail shows (a hash of the URI.
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

/// A custom icon or emoji an entry wears in place of its default icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryGlyph<'a> {
    /// A catalog icon name (`Look::icon_named`).
    Icon(&'a str),
    /// An emoji sequence.
    Emoji(&'a str),
}

/// Owned `EntryGlyph` stored on a `ListEntry`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum OwnedGlyph {
    /// A catalog icon name (`Look::icon_named`).
    Icon(String),
    /// An emoji sequence.
    Emoji(String),
}

impl OwnedGlyph {
    /// Borrowed as an [`EntryGlyph`].
    pub fn as_glyph(&self) -> EntryGlyph<'_> {
        match self {
            Self::Icon(name) => EntryGlyph::Icon(name),
            Self::Emoji(emoji) => EntryGlyph::Emoji(emoji),
        }
    }
}

impl From<EntryGlyph<'_>> for OwnedGlyph {
    fn from(glyph: EntryGlyph<'_>) -> Self {
        match glyph {
            EntryGlyph::Icon(name) => Self::Icon(name.to_owned()),
            EntryGlyph::Emoji(emoji) => Self::Emoji(emoji.to_owned()),
        }
    }
}

/// One entry's display fields, borrowed from its `EntrySource`, so a mounted row costs one call and copies nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Entry<'a> {
    /// The entry's id.
    pub id: EntryId,
    /// The name the row shows.
    pub name: &'a str,
    /// What the entry is.
    pub kind: EntryKind,
    /// The icon's catalog name: the output of the host's file-name-to-icon mapping.
    pub icon: &'a str,
    /// The thumbnail key, if the entry can have a thumbnail.
    pub thumbnail: Option<ThumbKey>,
    /// A custom icon or emoji worn over `Entry::icon`.
    pub glyph: Option<EntryGlyph<'a>>,
    /// A custom colour worn over the view's default icon colour.
    pub color: Option<rune::Color>,
}

impl<'a> Entry<'a> {
    /// An entry with no thumbnail or customization.
    pub const fn new(id: EntryId, name: &'a str, kind: EntryKind, icon: &'a str) -> Self {
        Self {
            id,
            name,
            kind,
            icon,
            thumbnail: None,
            glyph: None,
            color: None,
        }
    }

    /// The same entry, with thumbnail key `key`.
    pub const fn thumbnail(mut self, key: ThumbKey) -> Self {
        self.thumbnail = Some(key);
        self
    }

    /// The same entry, wearing `glyph`.
    pub const fn glyph(mut self, glyph: EntryGlyph<'a>) -> Self {
        self.glyph = Some(glyph);
        self
    }

    /// The same entry, wearing `glyph` when `Some`.
    pub const fn maybe_glyph(mut self, glyph: Option<EntryGlyph<'a>>) -> Self {
        self.glyph = glyph;
        self
    }

    /// The same entry, coloured `color`.
    pub const fn color(mut self, color: rune::Color) -> Self {
        self.color = Some(color);
        self
    }

    /// The same entry, coloured `color` when `Some`.
    pub const fn maybe_color(mut self, color: Option<rune::Color>) -> Self {
        self.color = color;
        self
    }
}

/// The entries a collection shows, in display order, read by index so that 100,000 rows never copy into the view.
pub trait EntrySource {
    /// How many entries there are.
    fn len(&self) -> usize;

    /// Whether there are none.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The id of the entry at `index`, which is below `EntrySource::len`.
    fn id_at(&self, index: usize) -> EntryId;

    /// Where the entry `id` is now, or `None` if it's gone.
    fn index_of(&self, id: EntryId) -> Option<usize>;

    /// The display fields of the entry at `index`, which is below [`EntrySource::len`].
    fn entry(&self, index: usize) -> Entry<'_>;
}

/// An owned entry in a `ListSource`.
#[derive(Clone, Debug, PartialEq)]
pub struct ListEntry {
    /// The entry's id: unique across every source shown at once.
    pub id: EntryId,
    /// The name the row shows.
    pub name: String,
    /// What the entry is.
    pub kind: EntryKind,
    /// The icon's catalog name; see [`Entry::icon`].
    pub icon: String,
    /// The thumbnail key, if any.
    pub thumbnail: Option<ThumbKey>,
    /// A custom icon or emoji, if any; see [`Entry::glyph`].
    pub glyph: Option<OwnedGlyph>,
    /// A custom icon colour, if any; see [`Entry::color`].
    pub color: Option<rune::Color>,
}

impl ListEntry {
    /// An entry with no thumbnail or customization.
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
            glyph: None,
            color: None,
        }
    }
}

/// An in-memory `EntrySource` for tests and gallery pages.
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

    /// Replaces every entry, as a refresh does.
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
        let entry = Entry::new(e.id, &e.name, e.kind, &e.icon)
            .maybe_glyph(e.glyph.as_ref().map(OwnedGlyph::as_glyph))
            .maybe_color(e.color);
        match e.thumbnail {
            Some(key) => entry.thumbnail(key),
            None => entry,
        }
    }
}
