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

use strata_components::contract::{Entry, EntryId, EntryKind, EntrySource, LinkTarget};
use strata_core::{
    i18n::tr,
    model::{EntryKind as Kind, FileEntry, Location},
};

use crate::slots::Ids;

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

/// What one row shows, copied from the browser's entry.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    id: EntryId,
    name: String,
    kind: EntryKind,
    described: Described,
    hidden: bool,
}

impl Row {
    pub(crate) fn of(entry: &FileEntry, ids: &Ids) -> Self {
        let (kind, described) = kinds(entry);
        Self {
            id: ids.of(&entry.location),
            name: entry.display_name.clone(),
            kind,
            described,
            hidden: entry.is_hidden,
        }
    }

    pub(crate) fn is_folder(&self) -> bool {
        self.kind.is_folder_like()
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
    /// The positions the view shows, in order: a view index is an index into this.
    shown: Vec<usize>,
    positions: HashMap<EntryId, usize>,
    show_hidden: bool,
    version: u64,
    load: Load,
    /// A sort or a refresh is running over a listing that stays up.
    busy: bool,
    /// The row of the column before this one whose folder this is.
    from: Option<EntryId>,
}

impl Column {
    pub(crate) fn new(location: Location, show_hidden: bool, from: Option<EntryId>) -> Self {
        Self {
            // `src/ui/browser/columns/append.rs` gives the GTK pane the same name.
            title: location.display_name(),
            location,
            rows: Vec::new(),
            shown: Vec::new(),
            positions: HashMap::new(),
            show_hidden,
            version: 0,
            load: Load::Loading,
            busy: false,
            from,
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
            Edit::Replace(rows) => {
                self.rows = rows;
                self.index();
            }
            Edit::Insert(insertions) => {
                for (position, rows) in insertions {
                    self.insert(position, rows);
                }
            }
            Edit::Splice(splices) => {
                for (position, removed, rows) in splices {
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
        if position < self.rows.len() {
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
        let moved = current.id != row.id || current.hidden != row.hidden;
        *current = row;
        if moved {
            self.index();
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
        self.version += 1;
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
        self.shown.binary_search(self.positions.get(&id)?).ok()
    }

    fn entry(&self, index: usize) -> Entry<'_> {
        let row = &self.rows[self.shown[index]];
        // No icon name: the view falls back to its folder and file icons.
        Entry::new(row.id, &row.name, row.kind, "")
    }
}

#[cfg(test)]
mod tests;
