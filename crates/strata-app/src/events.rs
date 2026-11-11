// SPDX-License-Identifier: MIT

//! `BrowserEvent`s on their way to the interface.
//!
//! A browser observer runs inside GLib's dispatch. The host is then in the middle of `poll` or
//! between two frames, and Rune holds the application state either way. So an observer only
//! queues, and the interface applies the queue in order.
//!
//! What an event changed is copied here, while the browser still holds exactly that: by the
//! time the queue is applied, later events may have moved its entries on. So a change carries
//! its rows, and the browser's cursor and selection as they stood.

use std::{cell::RefCell, collections::VecDeque, ops::Range, rc::Rc};

use strata_components::contract::EntryId;
use strata_core::model::{FileEntry, Location, ViewPreferences};
use strata_fs::app::{Browser, BrowserEvent};

use crate::columns::{Edit, Ids, Load, Row};

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

/// What a `BrowserEvent` asks of the interface.
#[derive(Debug)]
pub(crate) enum Change {
    /// The columns or the active one changed, so the window may show another location.
    Path,
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
    /// The browser's sort or hidden-files choice changed: the saved settings follow it.
    View(ViewPreferences),
}

#[derive(Default)]
struct Queue {
    changes: VecDeque<Change>,
    heard: u64,
    copied: u64,
}

impl Queue {
    fn push(&mut self, change: Change) {
        let repeats =
            matches!(change, Change::Path) && matches!(self.changes.back(), Some(Change::Path));
        if !repeats {
            self.changes.push_back(change);
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Inbox(Rc<RefCell<Queue>>);

impl Inbox {
    pub(crate) fn observing(browser: &Rc<Browser>, ids: Ids) -> Self {
        let inbox = Self::default();
        let queue = inbox.0.clone();
        let observed = Rc::downgrade(browser);
        browser.observe(move |event| {
            let Some(browser) = observed.upgrade() else {
                return;
            };
            let mut reader = Reader {
                browser: &browser,
                ids: &ids,
                copied: 0,
                changes: Vec::new(),
            };
            reader.read(event);
            let mut queue = queue.borrow_mut();
            queue.heard += 1;
            queue.copied += reader.copied;
            for change in reader.changes {
                queue.push(change);
            }
        });
        let queue = inbox.0.clone();
        browser.observe_preferences(move |view| queue.borrow_mut().push(Change::View(view)));
        inbox
    }

    pub(crate) fn pop(&self) -> Option<Change> {
        self.0.borrow_mut().changes.pop_front()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.borrow().changes.is_empty()
    }

    /// How many events the browser has sent, shown or not.
    pub(crate) fn heard(&self) -> u64 {
        self.0.borrow().heard
    }

    /// How many rows of listings those events had copied out of the browser.
    pub(crate) fn copied(&self) -> u64 {
        self.0.borrow().copied
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
        self.changes.push(Change::Path);
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
        self.changes.push(Change::Path);
    }

    fn read(&mut self, event: &BrowserEvent) {
        let read = self.read_columns(event) || self.read_rows(event) || self.read_load(event);
        if !read {
            self.read_selection(event);
        }
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
            BrowserEvent::ColumnsRelocated { from_depth } => {
                self.relocated(*from_depth);
                return true;
            }
            BrowserEvent::HiddenToggled { show_hidden } => {
                let marks = (0..)
                    .map_while(|depth| self.browser.location_at(depth).map(|_| self.marks(depth)))
                    .collect();
                self.changes.push(Change::Hidden {
                    show_hidden: *show_hidden,
                    marks,
                });
                return true;
            }
            _ => return false,
        }
        self.changes.push(Change::Path);
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
            // - `SelectionSynced` answers a selection the adapter itself wrote.
            // - `ColumnReloading` comes before the column is emptied; `ColumnReloaded` is
            //   the one that changes what shows.
            // - `NavigationStarting` asks a view to save what it shows; the slice restores
            //   no scroll position.
            // - The rename events (`RenameCompleted`, `RenameAbandoned`, `RenameFailed`,
            //   `EntryCreated`) and `ItemRelocated` are V9's.
            // - `NavigationRejected`, `ColumnNotResponding`, the peek, preview, open and
            //   extract requests, folder counts, sort choices and every transfer, deletion
            //   and restore event belong to interface the slice leaves out
            //   (`docs/rune/slice/README.md`, "Scope").
            _ => {}
        }
    }
}
