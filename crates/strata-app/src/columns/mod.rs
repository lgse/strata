// SPDX-License-Identifier: MIT

//! The Columns adapter: Strata's `Browser` on one side, strata-components' Columns view on
//! the other. Nothing else knows both.
//!
//! - **Events in.** The view's listener queues what each event changed
//!   ([`crate::events`]); [`Columns::apply`] puts it into the view's listings and state.
//! - **Intents out.** What the view asks goes to the browser ([`intents`]).
//! - **One cursor and one selection.** The view interprets keys and the pointer, and the
//!   browser holds the result ([`selection`]). `docs/rune/slice/V6-columns-adapter.md` has
//!   the evidence for that choice.
//! - **Mirroring** is view policy, ported from the GTK views ([`mirror`]).

use std::rc::Rc;

use rune::Tx;
use strata_components::{
    collection::{
        keyboard::{KeySettings, Letters},
        pointer::{ClickCount, ClickModes},
        selection::Listing,
    },
    columns::{ColumnsState, Focus, Opening},
    context_menu::AnchorEdge,
    contract::{EntryId, EntrySource},
    scroll::Autoscroll,
};
use strata_desktop::settings::Preferences;
use strata_fs::app::Browser;
use strata_kit::look::Look;

use crate::{
    commands,
    events::{Added, Change, Marks, Moved},
    handover::{self, Held, Moving},
    listing::{Column, Edit, Load, Row},
    slots::Ids,
    surface::Surface,
};

mod editing;
mod intents;
mod listings;
mod mirror;
mod part;
mod selection;
mod view;

pub(crate) use selection::Input;
pub(crate) use view::view;

use listings::Listings;
use mirror::Mirror;

/// The view's key: every element key of the strip starts with it.
pub(crate) const VIEW: &str = "columns";

/// What applying a change needs beside the change.
struct Hands<'a> {
    browser: &'a Rc<Browser>,
    look: &'a Look,
    tx: &'a mut Tx,
}

pub(crate) struct Columns {
    listings: Listings,
    state: ColumnsState,
    ids: Ids,
    /// For each column, the revision of its selection that the browser holds. Another
    /// revision is a change the view made, which [`Columns::publish`] hands over.
    held: Vec<Held>,
    /// The cursor the child column last followed, or was told of: its column and entry.
    followed: Option<(usize, EntryId)>,
    /// Entries a rename gave another id, until the listing shows the change.
    moving: Moving,
    mirror: Mirror,
    clicks: ClickModes,
    /// The keys the view reads other than by its default, when the saved settings ask.
    keys: Option<KeySettings>,
    labels: view::Labels,
    /// What a column says while it shows no entry.
    surface: Surface,
}

/// With "Type to search" off a plain letter starts no search, so `h j k l` are the arrows:
/// `Context::of` in `src/ui/keys/alias.rs`. On, the view's default passes letters on.
fn keys(typing: bool) -> Option<KeySettings> {
    (!typing).then(|| KeySettings::default().letters(Letters::Arrows))
}

fn clicks(count: u8) -> ClickCount {
    match count {
        1 => ClickCount::One,
        _ => ClickCount::Two,
    }
}

impl Columns {
    fn with_settings(settings: &Preferences, ids: Ids) -> Self {
        // `type_to_search_active` in `src/ui/preferences/input_settings.rs`.
        let typing = settings.type_to_search && !settings.tenxer_mode;
        Self {
            listings: Listings::translated(),
            state: ColumnsState::new(VIEW, Autoscroll::new()),
            ids,
            held: Vec::new(),
            followed: None,
            moving: Moving::default(),
            mirror: Mirror::new(settings.columns_mirror_selection),
            clicks: ClickModes::new(
                clicks(settings.columns_file_clicks),
                clicks(settings.columns_folder_clicks),
            ),
            keys: keys(typing),
            labels: view::Labels::translated(),
            surface: Surface::translated(),
        }
    }

    /// How many columns both the listings and the view hold.
    fn open(&self) -> usize {
        self.state.len().min(self.listings.columns.len())
    }

    /// The browser's position of entry `id` in column `depth`.
    pub(crate) fn position(&self, depth: usize, id: EntryId) -> Option<usize> {
        self.listings.columns.get(depth)?.position_of(id)
    }

    fn row(&self, depth: usize, id: EntryId) -> Option<&Row> {
        self.listings.columns.get(depth)?.row_of(id)
    }

    /// Applies one change the view's listener queued.
    fn apply(&mut self, change: Change, hands: &mut Hands<'_>) {
        match change {
            // The first column's `Added` follows, and replaces the strip.
            Change::Reset => {
                self.listings.columns.clear();
                self.held.clear();
            }
            Change::Truncated { len, replacing, .. } => self.truncate(len, replacing, hands),
            Change::Added(added) => self.add(added, hands),
            Change::Relocated {
                from_depth,
                columns,
            } => self.relocate(from_depth, columns, hands),
            Change::Moved { from, to } => self.moved(from, to),
            within => self.apply_within(within, hands),
        }
    }

    /// The changes inside a column: its rows, its load, its cursor and selection.
    fn apply_within(&mut self, change: Change, hands: &mut Hands<'_>) {
        match change {
            Change::Rows { depth, edit, marks } => self.edit(depth, edit, marks, hands),
            Change::Loaded { depth, load, marks } => self.loaded(depth, load, &marks, hands),
            Change::Busy { depth, busy } => {
                if let Some(column) = self.listings.columns.get_mut(depth) {
                    column.set_busy(busy);
                }
            }
            Change::Hidden { show_hidden, marks } => {
                for column in &mut self.listings.columns {
                    column.set_show_hidden(show_hidden);
                }
                for (depth, marks) in marks.iter().enumerate() {
                    self.settle(depth, marks, hands);
                }
            }
            Change::Selection {
                depth,
                marks,
                active,
                reveal,
                landed,
            } => {
                self.adopt(depth, &marks);
                self.activate(active, hands);
                if reveal {
                    self.reveal_cursor(depth, hands.tx);
                }
                if let Some(landed) = landed {
                    self.mirror
                        .follow(hands.browser, depth, landed.position, landed.by_removal);
                }
            }
            // `apply` took the rest.
            _ => {}
        }
    }

    fn loaded(&mut self, depth: usize, load: Load, marks: &Marks, hands: &mut Hands<'_>) {
        if let Some(column) = self.listings.columns.get_mut(depth) {
            column.set_load(load);
            column.set_busy(false);
            self.settle(depth, marks, hands);
        }
    }

    fn truncate(&mut self, len: usize, replacing: bool, hands: &mut Hands<'_>) {
        self.listings.columns.truncate(len);
        self.held.truncate(len);
        // A sibling takes the place of the column at `len` when it is added, with no exit.
        if !replacing {
            self.state.truncate(len, hands.look, hands.tx);
        }
    }

    fn add(&mut self, added: Added, hands: &mut Hands<'_>) {
        let Added {
            depth,
            location,
            show_hidden,
            active,
        } = added;
        self.listings.columns.truncate(depth);
        self.held.truncate(depth);
        // The browser adds columns in order, each after the last.
        if self.listings.columns.len() != depth {
            return;
        }
        // The row of the column before it that this folder is: the open path.
        let from = depth.checked_sub(1).map(|_| self.ids.of(&location));
        self.listings
            .columns
            .push(Column::new(location, show_hidden, from));
        if depth == 0 {
            // Another location: the old columns go with no exit.
            self.state
                .replace(&self.listings, 0, &|_| None, hands.look, hands.tx);
            // The GTK window gives a new listing the keyboard too.
            hands.tx.focus(VIEW, false);
        } else {
            let opening = Opening::new(&self.listings.columns[depth], Listing::Loading);
            // The browser places the cursor of a folder it opens, once it is listed.
            let focus = if active { Focus::Column } else { Focus::No };
            self.state
                .swap_from(depth, opening, focus, hands.look, hands.tx);
        }
        self.read_menu_keys(depth);
        self.held.push(self.held_now(depth));
    }

    /// Puts the context menu's keys ahead of a new column's own. GTK opens a Columns entry's
    /// menu at its right edge (`ColumnView::context_menu_target`).
    fn read_menu_keys(&mut self, depth: usize) {
        if let Some(column) = self.state.column_mut(depth) {
            let keyboard = &mut column.collection.controllers.keyboard;
            *keyboard = Some(commands::view_keys(keyboard.take(), AnchorEdge::Right));
        }
    }

    /// Folders that moved keep their columns, under their new names and with every row read
    /// again: a row's id comes from its location, which changed.
    fn relocate(&mut self, from_depth: usize, moved: Vec<Moved>, hands: &mut Hands<'_>) {
        for (depth, moved) in (from_depth..).zip(moved) {
            let Some(column) = self.listings.columns.get_mut(depth) else {
                return;
            };
            let from = depth.checked_sub(1).map(|_| self.ids.of(&moved.location));
            column.relocate(moved.location, from);
            column.apply(Edit::Replace(moved.rows));
            self.settle(depth, &moved.marks, hands);
        }
    }

    fn edit(&mut self, depth: usize, edit: Edit, marks: Option<Marks>, hands: &mut Hands<'_>) {
        let Some(column) = self.listings.columns.get_mut(depth) else {
            return;
        };
        column.apply(edit);
        self.arrived(depth);
        match marks {
            Some(marks) => self.settle(depth, &marks, hands),
            None => self.sync(depth, hands.tx),
        }
    }

    /// Brings column `depth`'s view state in step with its rows after they changed.
    fn sync(&mut self, depth: usize, tx: &mut Tx) {
        let Some(column) = self.listings.columns.get(depth) else {
            return;
        };
        self.state
            .sync(depth, column, handover::sync_rule(column), tx);
    }

    /// After column `depth`'s rows changed: the view catches up with them, and then the
    /// browser's cursor and selection stand. The one exception is a cursor the view was
    /// waiting to place until the folder was listed (Right into a column still loading):
    /// that is the user's, and [`Columns::publish`] hands it to the browser.
    fn settle(&mut self, depth: usize, marks: &Marks, hands: &mut Hands<'_>) {
        let before = self.cursor(depth);
        self.sync(depth, hands.tx);
        let placed = self
            .state
            .column(depth)
            .is_some_and(|column| handover::placed(before, &column.collection));
        if !placed {
            self.adopt(depth, marks);
        }
        if let Some(column) = self.listings.columns.get(depth) {
            tracing::trace!(
                depth,
                "the Columns view lists {} rows of {}",
                column.len(),
                column.title()
            );
        }
    }

    fn activate(&mut self, active: Option<usize>, hands: &mut Hands<'_>) {
        if let Some(active) = active.filter(|active| *active < self.open())
            && active != self.state.active()
        {
            self.state.set_active(active, hands.look, hands.tx);
        }
    }

    fn reveal_cursor(&self, depth: usize, tx: &mut Tx) {
        let (Some(column), Some(state), Some(key)) = (
            self.listings.columns.get(depth),
            self.state.column(depth),
            self.state.column_key(depth),
        ) else {
            return;
        };
        if let Some(index) = state.collection.selection.cursor_index(column) {
            state.collection.reveal(&key, column, index, tx);
        }
    }
}

impl Columns {
    /// One poll of what each column says while it shows no entry: a read is announced once
    /// it outlasts GTK's grace period. True when the view has to run.
    pub(crate) fn watch_surface(&mut self, clock: f64, tx: &mut rune::Tx) -> bool {
        let listings = self.listings.columns.iter().map(Some);
        self.surface.watch(listings, clock, tx)
    }
}

/// What the offscreen host's readers ask of the adapter beside the tree.
#[cfg(any(test, feature = "qa"))]
impl Columns {
    pub(crate) fn load(&self, depth: usize) -> Option<&Load> {
        self.listings.columns.get(depth).map(Column::load)
    }
}

#[cfg(test)]
impl Columns {
    /// How many rows column `depth` shows, mounted or not.
    pub(crate) fn rows_shown(&self, depth: usize) -> usize {
        self.listings.columns.get(depth).map_or(0, Column::len)
    }
}

#[cfg(feature = "qa")]
impl Columns {
    /// The key of each column's entry list in the tree, the first column's first.
    pub(crate) fn list_keys(&self) -> Vec<String> {
        (0..)
            .map_while(|depth| self.state.column_key(depth))
            .collect()
    }
}

#[cfg(test)]
mod tests;
