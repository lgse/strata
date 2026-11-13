// SPDX-License-Identifier: MIT

//! The listings a view reads: for each of the browser's columns, a copy of what its rows show.
//! They belong to the contract ([`crate::slots`]), with the reader that fills them
//! ([`crate::events`]): every view shows the same rows, so no view's package owns them.
//!
//! `EntrySource::entry` lends names, and `Browser` keeps its entries behind a `RefCell`, so
//! the view cannot read them in place. A column here holds one [`Row`] for each of the
//! browser's entries, at the browser's position, and a list of the positions it shows: the
//! browser's column holds hidden entries too, which the views leave out
//! (`entry_matches` in `src/ui/browser/entry.rs`).

use std::collections::HashMap;

use strata_components::contract::{Entry, EntryId, EntryKind, EntrySource, LinkTarget, ThumbKey};
use strata_core::{
    i18n::tr,
    model::{EntryKind as Kind, FileEntry, Location, MetadataValue, ViewPreferences},
};
use strata_fs::app::compare_type_groups;

use crate::slots::Ids;

mod type_group;

pub(crate) use type_group::{TypeGroup, entry_type_of};

/// What a row is, as assistive technology hears it: `entry_kind_name` in
/// `src/ui/accessibility.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Described {
    Folder,
    FolderLink,
    File,
    FileLink,
    BrokenLink,
    Other,
    LockedFolder,
    LockedFolderLink,
}

impl Described {
    /// The catalog's English for each, in the order they are declared.
    const WORDS: [&'static str; 8] = [
        "Folder",
        "Folder link",
        "File",
        "File link",
        "Broken link",
        "Other",
        "Folder that requires administrator access",
        "Folder link that requires administrator access",
    ];

    /// A folder the user may not enter is described as one.
    fn locked(self) -> Self {
        match self {
            Self::Folder => Self::LockedFolder,
            Self::FolderLink => Self::LockedFolderLink,
            other => other,
        }
    }
}

/// What the view draws an entry as, and what it is described as.
fn kinds(entry: &FileEntry) -> (EntryKind, Described) {
    let (kind, described) = match entry.kind {
        Kind::Directory => (EntryKind::Folder, Described::Folder),
        Kind::DirectorySymbolicLink => (
            EntryKind::Symlink(LinkTarget::Folder),
            Described::FolderLink,
        ),
        Kind::File => (EntryKind::File, Described::File),
        Kind::FileSymbolicLink => (EntryKind::Symlink(LinkTarget::File), Described::FileLink),
        Kind::SymbolicLink => (
            EntryKind::Symlink(LinkTarget::Broken),
            Described::BrokenLink,
        ),
        Kind::Other => (EntryKind::Special, Described::Other),
    };
    match entry.is_locked_folder() {
        true => (kind, described.locked()),
        false => (kind, described),
    }
}

/// One translation for each thing a row can be described as: the language changes only on
/// restart, and a view asks for a row's description on every build.
pub(crate) struct Words([String; 8]);

impl Words {
    pub(crate) fn translated() -> Self {
        Self(Described::WORDS.map(tr))
    }
}

/// What the browser knows of an entry beyond its name, as the views show it: the details line
/// under an Icons tile, the List view's size column. Each is `None` until the browser knows it
/// (a metadata fill) and for an entry that has none. [`crate::details`] makes the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct Facts {
    /// A file's size in bytes.
    pub(crate) size: Option<u64>,
    /// How many entries a folder holds.
    pub(crate) child_count: Option<u64>,
    /// A video's or a sound's length, in whole seconds.
    pub(crate) duration_seconds: Option<u64>,
    /// An image's width and height, in pixels.
    pub(crate) dimensions: Option<(u32, u32)>,
    /// When it was last modified, in seconds since the epoch: the List view's Modified column.
    pub(crate) modified: Option<i64>,
    /// Its permission bits: the List view's Mode column.
    pub(crate) mode: Option<u32>,
}

impl Facts {
    fn of(entry: &FileEntry) -> Self {
        Self {
            size: known(&entry.size),
            child_count: known(&entry.child_count),
            duration_seconds: known(&entry.duration_seconds),
            dimensions: known(&entry.image_dimensions),
            modified: known(&entry.modified_unix_seconds),
            mode: known(&entry.mode),
        }
    }
}

/// The value, when the browser knows it.
pub(crate) fn known<T: Copy>(value: &MetadataValue<T>) -> Option<T> {
    match value {
        MetadataValue::Known(value) => Some(*value),
        MetadataValue::Unknown | MetadataValue::Unavailable => None,
    }
}

/// What one row shows, copied from the browser's entry.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    id: EntryId,
    name: String,
    kind: EntryKind,
    described: Described,
    hidden: bool,
    /// The key of the entry's thumbnail, for a file that gets one ([`crate::thumbnails`]): a
    /// hash, so a row holds eight bytes for it and no path. A view finds the file again
    /// through the browser.
    thumbnail: Option<ThumbKey>,
    /// The browser's kind of entry: what its file type is read from with its name.
    entry_kind: Kind,
    /// Its file type, as the List view groups by it: read only while its folder is grouped
    /// ([`Column::group`]), as a MIME lookup for every row would cost every view.
    group: Option<TypeGroup>,
    facts: Facts,
}

impl Row {
    /// Keeps `previous`'s thumbnail key when this is the same entry read again, both rows
    /// have a key (a file replaced by a folder of its name has none), and no time or size
    /// known in both differs. A metadata fill that only makes them known, or a reload that
    /// lists them unknown again, is then the same file: a new key would ask for its thumbnail
    /// again and cancel the request out. GTK's key changes on the first fill too; GTK does not
    /// ask again because a slot already showing a thumbnail of that path is not asked
    /// (`displayed_thumbnail_matches`), nor a key already requested (`already_requested`).
    ///
    /// The window it leaves: a thumbnail rendered while the time and size were both unknown,
    /// of a file then replaced, stays while every later row brings only newly known facts,
    /// until a known time or size changes. GTK's window is wider: a slot showing a thumbnail
    /// of that path is not asked again until it is bound anew.
    fn keep_key_of(&mut self, previous: &Row) {
        let differs = |was: Option<i64>, now: Option<i64>| matches!((was, now), (Some(was), Some(now)) if was != now);
        let resized = matches!(
            (previous.facts.size, self.facts.size),
            (Some(was), Some(now)) if was != now
        );
        let same_file = self.id == previous.id
            && !differs(previous.facts.modified, self.facts.modified)
            && !resized;
        if same_file && previous.thumbnail.is_some() && self.thumbnail.is_some() {
            self.thumbnail = previous.thumbnail;
        }
    }

    pub(crate) fn of(entry: &FileEntry, ids: &Ids) -> Self {
        let (kind, described) = kinds(entry);
        Self {
            id: ids.of(&entry.location),
            name: entry.display_name.clone(),
            kind,
            described,
            hidden: entry.is_hidden,
            thumbnail: crate::thumbnails::key_of(entry),
            entry_kind: entry.kind,
            group: None,
            facts: Facts::of(entry),
        }
    }

    pub(crate) fn is_folder(&self) -> bool {
        self.kind.is_folder_like()
    }

    pub(crate) fn facts(&self) -> &Facts {
        &self.facts
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// The browser's kind of entry, which with the name says its file type.
    pub(crate) fn entry_kind(&self) -> Kind {
        self.entry_kind
    }
}

/// A change to a column's rows, as the browser's event described it. Positions are the
/// browser's.
#[derive(Debug)]
pub(crate) enum Edit {
    /// These are all the rows now.
    Replace(Vec<Row>),
    /// Rows put in at a position, each insertion counted after the ones before it.
    Insert(Vec<(usize, Vec<Row>)>),
    /// `removed` rows at a position give way to these, each splice counted after the ones
    /// before it.
    Splice(Vec<(usize, usize, Vec<Row>)>),
    /// Rows that stay where they are and show something new.
    Refill(Vec<(usize, Row)>),
}

/// Whether a column's folder has been read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Load {
    Loading,
    Listed,
    Failed(String),
}

pub(crate) struct Column {
    location: Location,
    title: String,
    /// The browser's entries, each at the browser's position.
    rows: Vec<Row>,
    /// The positions the view shows, in order: a view index is an index into this. In the
    /// browser's order, or in file-type groups while [`Column::grouping`] says so.
    shown: Vec<usize>,
    /// While the rows are grouped, the view index of each position the view shows
    /// (`usize::MAX` for one it does not). Empty otherwise, when `shown` is in the browser's
    /// order and a binary search finds an index.
    back: Vec<usize>,
    positions: HashMap<EntryId, usize>,
    show_hidden: bool,
    version: u64,
    load: Load,
    /// A sort or a refresh is running over a listing that stays up.
    busy: bool,
    /// The row of the column before this one whose folder this is.
    from: Option<EntryId>,
    /// The browser's preferences for this folder when it groups its rows by file type
    /// (`Browser::shows_type_groups`), and nothing when it does not.
    grouping: Option<ViewPreferences>,
}

impl Column {
    pub(crate) fn new(location: Location, show_hidden: bool, from: Option<EntryId>) -> Self {
        Self {
            // `src/ui/browser/columns/append.rs` gives the GTK pane the same name.
            title: location.display_name(),
            location,
            rows: Vec::new(),
            shown: Vec::new(),
            back: Vec::new(),
            positions: HashMap::new(),
            show_hidden,
            version: 0,
            load: Load::Loading,
            busy: false,
            from,
            grouping: None,
        }
    }

    /// The same column under its folder's new location.
    pub(crate) fn relocate(&mut self, location: Location, from: Option<EntryId>) {
        self.title = location.display_name();
        self.location = location;
        self.from = from;
    }

    pub(crate) fn title(&self) -> &str {
        &self.title
    }

    pub(crate) fn location(&self) -> &Location {
        &self.location
    }

    /// Whether the browser groups this folder's rows by file type, and under which
    /// preferences: what a view that groups orders them by.
    pub(crate) fn grouping(&self) -> Option<ViewPreferences> {
        self.grouping
    }

    /// The browser now groups this folder's rows under `grouping`, or does not. True when the
    /// order the view shows changed.
    pub(crate) fn set_grouping(&mut self, grouping: Option<ViewPreferences>) -> bool {
        if self.grouping == grouping {
            return false;
        }
        self.grouping = grouping;
        self.index();
        true
    }

    pub(crate) fn load(&self) -> &Load {
        &self.load
    }

    pub(crate) fn set_load(&mut self, load: Load) {
        self.load = load;
    }

    pub(crate) fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
    }

    pub(crate) fn set_show_hidden(&mut self, show_hidden: bool) {
        if self.show_hidden != show_hidden {
            self.show_hidden = show_hidden;
            self.index();
        }
    }

    pub(crate) fn apply(&mut self, edit: Edit) {
        match edit {
            Edit::Replace(mut rows) => {
                self.keep_keys(&mut rows);
                self.rows = rows;
                self.index();
            }
            Edit::Insert(insertions) => {
                for (position, rows) in insertions {
                    self.insert(position, rows);
                }
            }
            Edit::Splice(splices) => {
                for (position, removed, mut rows) in splices {
                    self.keep_keys(&mut rows);
                    let start = position.min(self.rows.len());
                    let end = start.saturating_add(removed).min(self.rows.len());
                    self.rows.splice(start..end, rows);
                }
                self.index();
            }
            Edit::Refill(updates) => {
                for (position, row) in updates {
                    self.refill(position, row);
                }
                self.version += 1;
            }
        }
    }

    /// Rows on the end join the index as they are, which is how a folder still loading
    /// publishes them; rows anywhere else move the ones after them.
    fn insert(&mut self, position: usize, rows: Vec<Row>) {
        // A grouped listing places a row by its type, not at the end.
        if position < self.rows.len() || self.grouping.is_some() {
            self.rows.splice(position..position, rows);
            self.index();
            return;
        }
        for row in rows {
            let position = self.rows.len();
            self.positions.insert(row.id, position);
            if self.show_hidden || !row.hidden {
                self.shown.push(position);
            }
            self.rows.push(row);
        }
        self.version += 1;
    }

    /// A refill never moves a row; one that changes whether it is hidden still changes which
    /// rows show.
    fn refill(&mut self, position: usize, row: Row) {
        let Some(current) = self.rows.get_mut(position) else {
            return;
        };
        // A row that changed its name or its kind may have changed its type.
        let retyped = current.name != row.name || current.entry_kind != row.entry_kind;
        let regroup = self.grouping.is_some() && retyped;
        let moved = current.id != row.id || current.hidden != row.hidden || regroup;
        // A row read again carries no type (`Row::of` reads none): one whose type did not
        // change keeps the one its group was found by, which `group_at` reads.
        let group = current.group.filter(|_| !retyped);
        let mut row = row;
        row.keep_key_of(current);
        *current = row;
        current.group = group;
        if moved {
            self.index();
        }
    }

    /// Rows read again keep the thumbnail keys of the rows they replace ([`Row::keep_key_of`]).
    fn keep_keys(&self, rows: &mut [Row]) {
        for row in rows {
            if let Some(previous) = self.row_of(row.id) {
                row.keep_key_of(previous);
            }
        }
    }

    fn index(&mut self) {
        self.positions.clear();
        self.shown.clear();
        for (position, row) in self.rows.iter().enumerate() {
            self.positions.insert(row.id, position);
            if self.show_hidden || !row.hidden {
                self.shown.push(position);
            }
        }
        self.group();
        self.version += 1;
    }

    /// Puts the rows shown in file-type groups, as GTK's grouped List shows them: by
    /// strata-fs's own rule (`compare_type_groups`, under the folder's preferences), each
    /// group keeping the browser's order inside it. Ungrouped, it leaves the browser's order
    /// and allocates nothing.
    fn group(&mut self) {
        self.back.clear();
        let Some(preferences) = self.grouping else {
            return;
        };
        let folder = self.location.clone();
        let mut types: Vec<TypeGroup> = Vec::new();
        let mut known: HashMap<TypeGroup, usize> = HashMap::new();
        for position in &self.shown {
            let row = &mut self.rows[*position];
            let group = *row
                .group
                .get_or_insert_with(|| TypeGroup::of_row(&folder, row.entry_kind, &row.name));
            known.entry(group).or_insert_with(|| {
                types.push(group);
                types.len() - 1
            });
        }
        let names: Vec<_> = types.iter().map(|group| group.description()).collect();
        let mut ranked: Vec<usize> = (0..types.len()).collect();
        ranked.sort_by(|a, b| compare_type_groups(&names[*a], &names[*b], preferences));
        let rank: HashMap<TypeGroup, usize> = ranked
            .into_iter()
            .enumerate()
            .map(|(place, kind)| (types[kind], place))
            .collect();
        // One pass into a bucket for each group, in the browser's order: linear in the rows,
        // so a folder that arrives in many batches costs one pass a batch, not a sort.
        let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); types.len()];
        for position in &self.shown {
            let group = self.rows[*position].group;
            let place = group
                .and_then(|group| rank.get(&group))
                .copied()
                .unwrap_or(0);
            buckets[place].push(*position);
        }
        self.shown = buckets.concat();
        self.back = vec![usize::MAX; self.rows.len()];
        for (index, position) in self.shown.iter().enumerate() {
            self.back[*position] = index;
        }
    }

    /// The browser's position of the row the view shows at `index`.
    pub(crate) fn position_at(&self, index: usize) -> Option<usize> {
        self.shown.get(index).copied()
    }

    /// The browser's position of the entry `id`, shown or not.
    pub(crate) fn position_of(&self, id: EntryId) -> Option<usize> {
        self.positions.get(&id).copied()
    }

    pub(crate) fn row_of(&self, id: EntryId) -> Option<&Row> {
        self.position_of(id)
            .and_then(|position| self.rows.get(position))
    }

    /// Moves with every change to what the column shows.
    pub(crate) fn version(&self) -> u64 {
        self.version
    }

    /// The folder is being read, or a sort or a refresh runs over its listing.
    pub(crate) fn is_loading(&self) -> bool {
        self.busy || self.load == Load::Loading
    }

    /// The row of the column before this one whose folder this is.
    pub(crate) fn from(&self) -> Option<EntryId> {
        self.from
    }

    /// The file type of the row shown at `index`.
    pub(crate) fn group_at(&self, index: usize) -> Option<TypeGroup> {
        self.rows.get(self.position_at(index)?)?.group
    }

    /// What the row the view shows at `index` is, as assistive technology hears it.
    pub(crate) fn description<'a>(&self, index: usize, words: &'a Words) -> Option<&'a str> {
        let row = self.rows.get(self.position_at(index)?)?;
        Some(&words.0[row.described as usize])
    }
}

impl EntrySource for Column {
    fn len(&self) -> usize {
        self.shown.len()
    }

    fn id_at(&self, index: usize) -> EntryId {
        self.rows[self.shown[index]].id
    }

    fn index_of(&self, id: EntryId) -> Option<usize> {
        let position = *self.positions.get(&id)?;
        if self.grouping.is_some() {
            return self
                .back
                .get(position)
                .copied()
                .filter(|index| *index != usize::MAX);
        }
        self.shown.binary_search(&position).ok()
    }

    fn entry(&self, index: usize) -> Entry<'_> {
        let row = &self.rows[self.shown[index]];
        // No icon name: the view falls back to its folder and file icons.
        let entry = Entry::new(row.id, &row.name, row.kind, "");
        match row.thumbnail {
            Some(key) => entry.thumbnail(key),
            None => entry,
        }
    }
}

#[cfg(test)]
mod tests;
