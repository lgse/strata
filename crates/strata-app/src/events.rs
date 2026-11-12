// SPDX-License-Identifier: MIT

//! What a `BrowserEvent` changed in the listings, read for a view's listener
//! ([`crate::slots`]). It belongs to the contract, not to a view: a change here is a
//! `contract` change.
//!
//! It is copied here, while the browser still holds exactly that: by the time the queue is
//! applied, later events may have moved its entries on. So a change carries its rows, and the
//! browser's cursor and selection as they stood.

use std::ops::Range;

use strata_components::contract::EntryId;
use strata_core::model::{FileEntry, Location};
use strata_fs::app::{Browser, BrowserEvent};

use crate::{
    columns::{Edit, Load, Row},
    slots::Ids,
};

/// The browser's cursor and selection in one column.
#[derive(Debug, Default)]
pub(crate) struct Marks {
    pub(crate) cursor: Option<EntryId>,
    pub(crate) selected: Vec<EntryId>,
    /// The selection is the cursor a load placed, which the user has not chosen.
    pub(crate) provisional: bool,
}

/// A column the browser opened.
#[derive(Debug)]
pub(crate) struct Added {
    pub(crate) depth: usize,
    pub(crate) location: Location,
    pub(crate) show_hidden: bool,
    /// The browser made it the active column, as opening a folder does and mirroring one
    /// does not.
    pub(crate) active: bool,
}

/// A column whose folder moved, with everything it shows read again.
#[derive(Debug)]
pub(crate) struct Moved {
    pub(crate) location: Location,
    pub(crate) rows: Vec<Row>,
    pub(crate) marks: Marks,
}

/// Where the browser put the cursor, for the child column that follows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Landed {
    pub(crate) position: usize,
    /// A deletion moved it, which mirrors whatever the last input was.
    pub(crate) by_removal: bool,
}

/// What a `BrowserEvent` asks of a view.
#[derive(Debug)]
pub(crate) enum Change {
    Reset,
    Truncated {
        len: usize,
        /// A sibling's `Added` at `len` follows at once.
        replacing: bool,
    },
    Added(Added),
    Relocated {
        from_depth: usize,
        columns: Vec<Moved>,
    },
    Rows {
        depth: usize,
        edit: Edit,
        /// `None` when the edit changes neither which rows there are nor their order.
        marks: Option<Marks>,
    },
    Loaded {
        depth: usize,
        load: Load,
        marks: Marks,
    },
    Busy {
        depth: usize,
        busy: bool,
    },
    Hidden {
        show_hidden: bool,
        /// One for each column, root first.
        marks: Vec<Marks>,
    },
    Selection {
        depth: usize,
        marks: Marks,
        active: Option<usize>,
        /// The browser asks for the cursor's row to be shown.
        reveal: bool,
        landed: Option<Landed>,
    },
    /// One of the browser's operations gave an entry another location, and so another id.
    /// It comes before the listing shows it: what the view holds by the old id moves first.
    Moved {
        from: EntryId,
        to: EntryId,
    },
}

/// What one event changed, in the order a view applies it.
pub(crate) struct Read {
    pub(crate) changes: Vec<Change>,
    /// How many rows of listings were copied out of the browser for them.
    pub(crate) copied: u64,
}

/// Reads out of `browser` what `event` changed in the listings. Call it from a listener,
/// inside the browser's observer.
pub(crate) fn read(event: &BrowserEvent, browser: &Browser, ids: &Ids) -> Read {
    let mut reader = Reader {
        browser,
        ids,
        copied: 0,
        changes: Vec::new(),
    };
    reader.read(event);
    Read {
        changes: reader.changes,
        copied: reader.copied,
    }
}

/// Reads out of the browser what one event changed.
struct Reader<'a> {
    browser: &'a Browser,
    ids: &'a Ids,
    copied: u64,
    changes: Vec<Change>,
}

impl Reader<'_> {
    fn rows(&mut self, entries: &[FileEntry]) -> Vec<Row> {
        self.copied += entries.len() as u64;
        entries
            .iter()
            .map(|entry| Row::of(entry, self.ids))
            .collect()
    }

    fn range(&mut self, depth: usize, range: Range<usize>) -> Vec<Row> {
        let browser = self.browser;
        browser
            .with_entries(depth, range, |entries| self.rows(entries))
            .unwrap_or_default()
    }

    fn column(&mut self, depth: usize) -> Vec<Row> {
        let browser = self.browser;
        browser
            .with_column_entries(depth, |entries| self.rows(entries))
            .unwrap_or_default()
    }

    fn marks(&self, depth: usize) -> Marks {
        let browser = self.browser;
        let positions = browser.selected_positions(depth);
        let selected = browser
            .with_column_entries(depth, |entries| {
                positions
                    .iter()
                    .filter_map(|position| entries.get(*position))
                    .map(|entry| self.ids.of(&entry.location))
                    .collect()
            })
            .unwrap_or_default();
        Marks {
            cursor: browser
                .cursor_entry(depth)
                .map(|entry| self.ids.of(&entry.location)),
            selected,
            // The browser says so of its active column only, and only a column it made
            // active loads with a cursor.
            provisional: browser.active_depth() == Some(depth)
                && browser.selection_is_load_cursor(),
        }
    }

    fn show_hidden(&self, depth: usize) -> bool {
        self.browser
            .column_preferences(depth)
            .unwrap_or_else(|| self.browser.preferences())
            .show_hidden
    }

    fn edited(&mut self, depth: usize, edit: Edit) {
        let marks = Some(self.marks(depth));
        self.changes.push(Change::Rows { depth, edit, marks });
    }

    fn loaded(&mut self, depth: usize, load: Load) {
        let marks = self.marks(depth);
        self.changes.push(Change::Loaded { depth, load, marks });
    }

    fn selection(&mut self, depth: usize, reveal: bool, landed: Option<Landed>) {
        self.changes.push(Change::Selection {
            depth,
            marks: self.marks(depth),
            active: self.browser.active_depth(),
            reveal,
            landed,
        });
    }

    fn relocated(&mut self, from_depth: usize) {
        let mut columns = Vec::new();
        for depth in from_depth.. {
            let Some(location) = self.browser.location_at(depth) else {
                break;
            };
            columns.push(Moved {
                location,
                rows: self.column(depth),
                marks: self.marks(depth),
            });
        }
        self.changes.push(Change::Relocated {
            from_depth,
            columns,
        });
    }

    fn read(&mut self, event: &BrowserEvent) {
        let read = self.read_columns(event)
            || self.read_rows(event)
            || self.read_load(event)
            || self.read_moved(event);
        if !read {
            self.read_selection(event);
        }
    }

    /// The event that gives an entry another id. False when `event` is not it.
    fn read_moved(&mut self, event: &BrowserEvent) -> bool {
        let BrowserEvent::ItemRelocated { from, to, .. } = event else {
            return false;
        };
        self.changes.push(Change::Moved {
            from: self.ids.of(from),
            to: self.ids.of(to),
        });
        true
    }

    /// The events that open, close and move columns. False when `event` is not one.
    fn read_columns(&mut self, event: &BrowserEvent) -> bool {
        match event {
            BrowserEvent::Reset => self.changes.push(Change::Reset),
            BrowserEvent::ColumnsTruncated { len, replacing } => {
                self.changes.push(Change::Truncated {
                    len: *len,
                    replacing: *replacing,
                });
            }
            BrowserEvent::ColumnAdded { depth, location } => {
                self.changes.push(Change::Added(Added {
                    depth: *depth,
                    location: location.clone(),
                    show_hidden: self.show_hidden(*depth),
                    active: self.browser.active_depth() == Some(*depth),
                }));
            }
            BrowserEvent::ColumnsRelocated { from_depth } => self.relocated(*from_depth),
            BrowserEvent::HiddenToggled { show_hidden } => {
                let marks = (0..)
                    .map_while(|depth| self.browser.location_at(depth).map(|_| self.marks(depth)))
                    .collect();
                self.changes.push(Change::Hidden {
                    show_hidden: *show_hidden,
                    marks,
                });
            }
            _ => return false,
        }
        true
    }

    /// The events that change what a column lists. False when `event` is not one.
    fn read_rows(&mut self, event: &BrowserEvent) -> bool {
        match event {
            BrowserEvent::EntriesInserted { depth, insertions } => {
                let insertions = insertions
                    .iter()
                    .map(|insertion| (insertion.position, self.rows(&insertion.entries)))
                    .collect();
                self.edited(*depth, Edit::Insert(insertions));
            }
            // The browser holds the whole listing by now and publishes it in parts: only the
            // first `count` entries are out.
            BrowserEvent::EntriesReplaced { depth, count } => {
                let rows = self.range(*depth, 0..*count);
                self.edited(*depth, Edit::Replace(rows));
            }
            BrowserEvent::EntriesPublished {
                depth,
                position,
                count,
            } => {
                let rows = self.range(*depth, *position..position.saturating_add(*count));
                self.edited(*depth, Edit::Insert(vec![(*position, rows)]));
            }
            BrowserEvent::EntriesSpliced { depth, splices } => {
                let splices = splices
                    .iter()
                    .map(|splice| (splice.position, splice.removed, self.rows(&splice.entries)))
                    .collect();
                self.edited(*depth, Edit::Splice(splices));
            }
            // Rows that are shown already: they count as no copy of the listing.
            BrowserEvent::MetadataFilled { depth, updates } => {
                let updates = updates
                    .iter()
                    .map(|(position, entry)| (*position, Row::of(entry, self.ids)))
                    .collect();
                self.changes.push(Change::Rows {
                    depth: *depth,
                    edit: Edit::Refill(updates),
                    marks: None,
                });
            }
            _ => return false,
        }
        true
    }

    /// The events that say how a column's load stands. False when `event` is not one.
    fn read_load(&mut self, event: &BrowserEvent) -> bool {
        match event {
            BrowserEvent::SortingStarted { depth } | BrowserEvent::ColumnRefreshing { depth } => {
                self.changes.push(Change::Busy {
                    depth: *depth,
                    busy: true,
                });
            }
            BrowserEvent::SortingFinished { depth } => {
                self.changes.push(Change::Busy {
                    depth: *depth,
                    busy: false,
                });
            }
            // The column has been emptied and its folder is being read again.
            BrowserEvent::ColumnReloaded { depth } => {
                self.loaded(*depth, Load::Loading);
                let rows = self.column(*depth);
                self.edited(*depth, Edit::Replace(rows));
            }
            BrowserEvent::LoadFinished { depth, .. } => self.loaded(*depth, Load::Listed),
            BrowserEvent::LoadFailed { depth, message, .. } => {
                self.loaded(*depth, Load::Failed(message.clone()));
            }
            _ => return false,
        }
        true
    }

    fn read_selection(&mut self, event: &BrowserEvent) {
        match event {
            BrowserEvent::FocusChanged {
                depth,
                position,
                triggered_by_removal,
            } => {
                let landed = position
                    .filter(|_| !self.browser.child_mirror_suppressed())
                    .map(|position| Landed {
                        position,
                        by_removal: *triggered_by_removal,
                    });
                self.selection(*depth, position.is_some(), landed);
            }
            BrowserEvent::SelectionSetChanged {
                depth, take_focus, ..
            } => {
                let reveal = *take_focus || self.browser.active_depth() == Some(*depth);
                self.selection(*depth, reveal, None);
            }
            // Ignored on purpose:
            // - The rename answers are the rename editor's (`crate::rename`).
            // - `SelectionSynced` answers a selection the adapter itself wrote.
            // - `ColumnReloading` comes before the column is emptied; `ColumnReloaded` is
            //   the one that changes what shows.
            // - `NavigationStarting` asks a view to save what it shows; the slice restores
            //   no scroll position.
            // - `EntryCreated` is New Folder's and New File's, which the slice leaves out.
            // - `NavigationRejected`, `ColumnNotResponding`, the peek, preview, open and
            //   extract requests, folder counts, sort choices and every transfer, deletion
            //   and restore event belong to interface the slice leaves out
            //   (`docs/rune/slice/README.md`, "Scope").
            _ => {}
        }
    }
}
