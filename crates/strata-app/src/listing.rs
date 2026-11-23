// SPDX-License-Identifier: MIT

use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    hash::{BuildHasherDefault, Hasher},
    path::Path,
    rc::Rc,
};

use strata_components::contract::{Entry, EntryId, EntryKind, EntrySource, LinkTarget, ThumbKey};
use strata_core::{
    i18n::tr,
    model::{EntryKind as Kind, FileEntry, Location, MetadataValue, ViewPreferences},
};

use crate::slots::{Ids, Worn};

mod groups;
mod type_group;

use groups::GroupedBuckets;
pub(crate) use type_group::{TypeGroup, entry_type_of};

/// Fast non-cryptographic hasher for in-process sequential [`EntryId`] keys.
#[derive(Default)]
struct EntryIdHasher(u64);

impl Hasher for EntryIdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self.0.wrapping_mul(0x517cc1b727220a95) ^ u64::from(b);
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = value.wrapping_mul(0x9e3779b97f4a7c15);
    }
}

type EntryIdMap<V> = HashMap<EntryId, V, BuildHasherDefault<EntryIdHasher>>;

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

pub(crate) struct Words([String; 8]);

impl Words {
    pub(crate) fn translated() -> Self {
        Self(Described::WORDS.map(tr))
    }
}

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
    /// When it was created, in seconds since the epoch: the List view's Created column.
    pub(crate) created: Option<i64>,
    /// Its permission bits: the List view's Mode column.
    pub(crate) mode: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct KeyedFacts {
    modified: Option<i64>,
    size: Option<u64>,
}

impl KeyedFacts {
    fn of(facts: &Facts) -> Self {
        Self {
            modified: facts.modified,
            size: facts.size,
        }
    }

    /// These facts, with a time or a size still unknown here taken from `now`.
    fn learning(self, now: &Facts) -> Self {
        Self {
            modified: self.modified.or(now.modified),
            size: self.size.or(now.size),
        }
    }

    /// Whether a time or a size known in both differs: the file changed.
    fn contradict(self, other: &Facts) -> bool {
        fn differ<T: PartialEq>(a: Option<T>, b: Option<T>) -> bool {
            matches!((a, b), (Some(a), Some(b)) if a != b)
        }
        differ(self.modified, other.modified) || differ(self.size, other.size)
    }
}

impl Facts {
    fn of(entry: &FileEntry) -> Self {
        Self {
            size: known(&entry.size),
            child_count: known(&entry.child_count),
            duration_seconds: known(&entry.duration_seconds),
            dimensions: known(&entry.image_dimensions),
            modified: known(&entry.modified_unix_seconds),
            created: known(&entry.created_unix_seconds),
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
    icon: &'static str,
    described: Described,
    hidden: bool,
    thumbnail: Option<ThumbKey>,
    /// Whether `thumbnail` is an STL model gated by `stl_thumbnails`.
    rendered_model: bool,
    /// The browser's kind of entry: what its file type is read from with its name.
    entry_kind: Kind,
    group: Option<TypeGroup>,
    facts: Facts,
    keyed: KeyedFacts,
}

impl Row {
    fn keep_key_of(&mut self, previous: &Row) {
        if self.same_file_as(previous) {
            self.thumbnail = previous.thumbnail;
            self.keyed = previous.keyed.learning(&self.facts);
        }
    }

    fn same_file_as(&self, previous: &Row) -> bool {
        let keyed = previous.thumbnail.is_some() && self.thumbnail.is_some();
        let same_entry = self.id == previous.id;
        keyed && same_entry && !previous.keyed.contradict(&self.facts)
    }

    pub(crate) fn of(entry: &FileEntry, ids: &Ids) -> Self {
        Self::of_with_id(entry, ids.of(&entry.location))
    }

    pub(crate) fn of_with_id(entry: &FileEntry, id: EntryId) -> Self {
        let (kind, described) = kinds(entry);
        let (thumbnail, rendered_model) = crate::thumbnails::classify(entry);
        let facts = Facts::of(entry);
        Self {
            id,
            name: entry.display_name.clone(),
            kind,
            icon: crate::entry_icon::entry_icon(entry),
            described,
            hidden: entry.is_hidden,
            thumbnail,
            rendered_model,
            entry_kind: entry.kind,
            group: None,
            keyed: KeyedFacts::of(&facts),
            facts,
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

    pub(crate) fn thumb_origin(&self) -> Option<ThumbOrigin> {
        Some(ThumbOrigin {
            key: self.thumbnail?,
            entry_kind: self.entry_kind,
            extension: Path::new(&self.name).extension().map(OsStr::to_os_string),
            keyed: self.keyed,
        })
    }

    pub(crate) fn relocated_thumbnail(&self, origin: &ThumbOrigin) -> Option<(ThumbKey, ThumbKey)> {
        let to = self.thumbnail?;
        let same_ext = Path::new(&self.name).extension() == origin.extension.as_deref();
        let reuse = origin.key != to
            && self.entry_kind == origin.entry_kind
            && same_ext
            && !origin.keyed.contradict(&self.facts);
        reuse.then_some((origin.key, to))
    }
}

/// What a renamed row needs from the row it replaced to reuse its thumbnail in the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ThumbOrigin {
    key: ThumbKey,
    entry_kind: Kind,
    extension: Option<OsString>,
    keyed: KeyedFacts,
}

#[derive(Debug)]
pub(crate) enum Edit {
    /// These are all the rows now.
    Replace(Vec<Row>),
    /// Rows put in at a position, each insertion counted after the ones before it.
    Insert(Vec<(usize, Vec<Row>)>),
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
    shown: Vec<usize>,
    back: Vec<usize>,
    grouped: GroupedBuckets,
    positions: EntryIdMap<usize>,
    show_hidden: bool,
    /// Moves on every edit to the column's rows (including metadata refills).
    version: u64,
    key_version: u64,
    /// Moves when row keys, order, or any row's thumbnail key/model kind changes.
    thumb_version: u64,
    load: Load,
    /// A sort or a refresh is running over a listing that stays up.
    busy: bool,
    /// The row of the column before this one whose folder this is.
    from: Option<EntryId>,
    grouping: Option<ViewPreferences>,
    /// The window's saved item colours and icons, indexed by [`EntryId`].
    worn: Rc<Worn>,
}

impl Column {
    pub(crate) fn new(location: Location, show_hidden: bool, from: Option<EntryId>) -> Self {
        Self {
            title: location.display_name(),
            location,
            rows: Vec::new(),
            shown: Vec::new(),
            back: Vec::new(),
            grouped: GroupedBuckets::default(),
            positions: EntryIdMap::default(),
            show_hidden,
            version: 0,
            key_version: 0,
            thumb_version: 0,
            load: Load::Loading,
            busy: false,
            from,
            grouping: None,
            worn: Worn::empty(),
        }
    }

    pub(crate) fn with_worn(mut self, worn: Rc<Worn>) -> Self {
        self.worn = worn;
        self
    }

    pub(crate) fn set_worn(&mut self, worn: &Rc<Worn>) {
        self.worn = worn.clone();
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

    pub(crate) fn grouping(&self) -> Option<ViewPreferences> {
        self.grouping
    }

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

    fn insert(&mut self, position: usize, rows: Vec<Row>) {
        if position < self.rows.len() {
            self.rows.splice(position..position, rows);
            self.index();
            return;
        }
        if let Some(preferences) = self.grouping {
            self.append_grouped(rows, preferences);
            return;
        }
        self.rows.reserve(rows.len());
        self.positions.reserve(rows.len());
        self.shown.reserve(rows.len());
        for row in rows {
            let position = self.rows.len();
            self.positions.insert(row.id, position);
            if self.show_hidden || !row.hidden {
                self.shown.push(position);
            }
            self.rows.push(row);
        }
        self.version += 1;
        self.key_version += 1;
        self.thumb_version += 1;
    }

    fn append_grouped(&mut self, rows: Vec<Row>, preferences: ViewPreferences) {
        if rows.is_empty() {
            return;
        }
        let start = self.rows.len();
        self.rows.reserve(rows.len());
        self.positions.reserve(rows.len());
        let folder = &self.location;
        for mut row in rows {
            let position = self.rows.len();
            self.positions.insert(row.id, position);
            if self.show_hidden || !row.hidden {
                let group = *row
                    .group
                    .get_or_insert_with(|| TypeGroup::of_row(folder, row.entry_kind, &row.name));
                self.grouped.push(position, group, preferences);
            }
            self.rows.push(row);
        }
        self.grouped.refresh_runs();
        if self.grouped.single_group() {
            self.append_single_group_tail(start);
        } else {
            self.grouped
                .flatten_into(self.rows.len(), &mut self.shown, &mut self.back);
        }
        self.version += 1;
        self.key_version += 1;
        self.thumb_version += 1;
    }

    fn append_single_group_tail(&mut self, start: usize) {
        self.back.resize(self.rows.len(), usize::MAX);
        for position in start..self.rows.len() {
            if self.show_hidden || !self.rows[position].hidden {
                let index = self.shown.len();
                self.shown.push(position);
                self.back[position] = index;
            }
        }
    }

    fn refill(&mut self, position: usize, row: Row) {
        let Some(current) = self.rows.get_mut(position) else {
            return;
        };
        // A row that changed its name or its kind may have changed its type.
        let retyped = current.name != row.name || current.entry_kind != row.entry_kind;
        let regroup = self.grouping.is_some() && retyped;
        let moved = current.id != row.id || current.hidden != row.hidden || regroup;
        let old_thumb = current.thumbnail;
        let old_model = current.rendered_model;
        let group = current.group.filter(|_| !retyped);
        let mut row = row;
        row.keep_key_of(current);
        *current = row;
        current.group = group;
        if moved {
            self.index();
        } else if current.thumbnail != old_thumb || current.rendered_model != old_model {
            self.thumb_version += 1;
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
        self.positions.reserve(self.rows.len());
        self.shown.clear();
        self.shown.reserve(self.rows.len());
        for (position, row) in self.rows.iter().enumerate() {
            self.positions.insert(row.id, position);
            if self.show_hidden || !row.hidden {
                self.shown.push(position);
            }
        }
        self.group();
        self.version += 1;
        self.key_version += 1;
        self.thumb_version += 1;
    }

    fn group(&mut self) {
        self.back.clear();
        let Some(preferences) = self.grouping else {
            self.grouped.clear();
            return;
        };
        let folder = &self.location;
        let rows = &mut self.rows;
        self.grouped.rebuild(
            preferences,
            self.shown.iter().map(|&position| {
                let row = &mut rows[position];
                let group = *row
                    .group
                    .get_or_insert_with(|| TypeGroup::of_row(folder, row.entry_kind, &row.name));
                (position, group)
            }),
        );
        self.grouped
            .flatten_into(self.rows.len(), &mut self.shown, &mut self.back);
    }

    /// The file-type runs of this column's shown rows, in O(groups). Empty when ungrouped.
    pub(crate) fn group_runs(&self) -> &[(TypeGroup, usize)] {
        self.grouped.runs()
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

    /// The row shown at view index `index`, in O(1) without a hash lookup.
    pub(crate) fn row_at(&self, index: usize) -> Option<&Row> {
        self.rows.get(*self.shown.get(index)?)
    }

    /// Moves with every change to what the column shows.
    pub(crate) fn version(&self) -> u64 {
        self.version ^ self.worn.version().wrapping_shl(24)
    }

    /// Moves only when the shown row keys or their order change (`virtual_rows_version`).
    pub(crate) fn key_version(&self) -> u64 {
        self.key_version
    }

    /// Moves when row keys, order, thumbnail keys, or worn customizations change.
    pub(crate) fn thumb_version(&self) -> u64 {
        self.thumb_version ^ self.worn.version().wrapping_shl(24)
    }

    /// The folder is being read, or a sort or a refresh runs over its listing.
    pub(crate) fn is_loading(&self) -> bool {
        self.busy || self.load == Load::Loading
    }

    /// A sort or a refresh is actively running over this column's listing.
    pub(crate) fn is_busy(&self) -> bool {
        self.busy
    }

    /// The row of the column before this one whose folder this is.
    pub(crate) fn from(&self) -> Option<EntryId> {
        self.from
    }

    /// Appends every [`EntryId`] currently referenced by this listing into `out`.
    pub(crate) fn collect_ids(&self, out: &mut impl Extend<EntryId>) {
        out.extend(self.rows.iter().map(|row| row.id));
        if let Some(from) = self.from {
            out.extend(std::iter::once(from));
        }
    }

    /// The file type of the row shown at `index`.
    #[cfg(test)]
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
        let (color, glyph) = self.worn.get(row.id).unwrap_or((None, None));
        let has_glyph = glyph.is_some();
        let entry = Entry::new(row.id, &row.name, row.kind, row.icon)
            .maybe_color(color)
            .maybe_glyph(glyph);
        let allow_thumbnail = !has_glyph && (!row.rendered_model || self.worn.stl_thumbnails());
        match row.thumbnail.filter(|_| allow_thumbnail) {
            Some(key) => entry.thumbnail(key),
            None => entry,
        }
    }
}

#[cfg(test)]
mod tests;
