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

use std::{cell::Cell, rc::Rc};

use rune::Tx;
use strata_components::{
    collection::{
        element::DropOver,
        keyboard::{KeySettings, Letters},
        selection::{Listing, SyncRule},
    },
    columns::{ColumnSources, ColumnsState, Focus, Opening},
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
    handover::{self, Held, Kept, Moving},
    listing::{Column, Edit, Load, Row},
    slots::{Ids, Setup, Worn},
    surface::Surface,
};

mod editing;
mod fills;
mod filtered;
mod intents;
mod listings;
mod mirror;
mod part;
mod selection;
pub(crate) mod thumbs;
mod view;
mod width;

pub(crate) use intents::spring_open;
pub(crate) use selection::Input;
#[cfg(test)]
pub(crate) use view::row_marks;
pub(crate) use view::view;

use fills::ColumnFills;
use listings::Listings;
use mirror::Mirror;
use thumbs::Thumbs;

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
    /// For each column, the neighbour kept after a move out of its folder
    /// ([`handover::Kept`]).
    kept: Vec<Kept>,
    /// The cursor the child column last followed, or was told of: its column and entry.
    followed: Option<(usize, EntryId)>,
    /// Entries a rename gave another id, until the listing shows the change.
    moving: Moving,
    mirror: Mirror,
    tenxer: Cell<bool>,
    labels: view::Labels,
    /// What a column says while it shows no entry.
    surface: Surface,
    /// The rows' thumbnails: a store viewer for each open column.
    thumbs: Thumbs,
    /// The rows' metadata fills: one for each open column.
    fills: ColumnFills,
    /// Set by `Reset` or a root `Added(0)` until the rebuilt strip builds: keeps replay and
    /// return-follow additions in that transaction instant (`ColumnsState::replace`).
    pub(super) rebuilding: Cell<bool>,
    /// The window's display scale at the last build, for drag preview icons.
    pub(super) scale: Cell<f32>,
    /// Whether the drag over `columns-host` is refused (`Some(has_uris)`), or accepted (`None`).
    pub(super) strip_refused: Option<bool>,
}

/// With 10xer mode on, C1c's 10xer map moves the cursor without changing the selection; with
/// "Type to search" off, `h j k l` alias the arrows (`Context::of` in `src/ui/keys/alias.rs`).
pub(super) fn live_keys(settings: &Preferences, multiple: bool) -> Option<KeySettings> {
    if settings.tenxer_mode {
        Some(KeySettings::tenxer().multiple(multiple))
    } else if !settings.type_to_search {
        Some(
            KeySettings::default()
                .letters(Letters::Arrows)
                .multiple(multiple),
        )
    } else if !multiple {
        Some(KeySettings::default().multiple(false))
    } else {
        None
    }
}

pub(crate) fn use_chooser_widths(app: &mut crate::App) {
    let width = width::chooser_saved(crate::shell::preferences(app));
    app.columns.state.set_default_width(width);
}

impl Columns {
    fn with_setup(setup: &Setup<'_>) -> Self {
        let settings: &Preferences = setup.settings;
        Self {
            listings: Listings::translated(setup.worn.clone()),
            state: {
                let mut state = ColumnsState::new(VIEW, Autoscroll::new());
                state.set_default_width(width::saved(settings));
                state
            },
            ids: setup.ids.clone(),
            held: Vec::new(),
            kept: Vec::new(),
            followed: None,
            moving: Moving::default(),
            mirror: Mirror::new(
                settings.columns_mirror_selection,
                settings.single_click_previews,
                setup.shown.clone(),
            ),
            tenxer: Cell::new(settings.tenxer_mode),
            labels: view::Labels::translated(),
            surface: Surface::translated(),
            thumbs: Thumbs::new(setup.later.clone()),
            fills: ColumnFills::new(settings.columns_show_sizes),
            rebuilding: Cell::new(true),
            scale: Cell::new(1.0),
            strip_refused: None,
        }
    }

    pub(crate) fn set_worn(&mut self, worn: &Rc<Worn>) {
        self.listings.set_worn(worn);
    }

    /// "Show sizes in Columns view" changed: the open columns ask for their folders' counts,
    /// or stop. The badges follow the saved setting at the next build.
    pub(crate) fn show_sizes(&mut self, show: bool) {
        self.fills.show_sizes(show);
    }

    /// Whether the selection's folder opens as the child column: the saved choice.
    pub(crate) fn mirrors(&self) -> bool {
        self.mirror.enabled()
    }

    pub(crate) fn refresh_mirror_preferences(&self, settings: &Preferences) {
        self.tenxer.set(settings.tenxer_mode);
        self.mirror.set_preferences(
            settings.columns_mirror_selection,
            settings.single_click_previews,
        );
    }

    pub(crate) fn last_edge_lit(&self) -> bool {
        self.state.last_edge_lit()
    }

    pub(crate) fn is_resizing(&self) -> bool {
        self.state.is_resizing()
    }

    pub(crate) fn set_width_cap(&mut self, cap: Option<f32>) {
        self.state.set_width_cap(cap);
    }

    pub(crate) fn reveal_active(&mut self, look: &Look, tx: &mut Tx) {
        self.state.reveal_active(look, tx);
    }

    pub(crate) fn clear_selection(&mut self) {
        self.state.clear_selection(&self.listings);
    }

    pub(crate) fn has_column_key(&self, key: &str) -> bool {
        (0..self.open()).any(|depth| self.state.column_key(depth).as_deref() == Some(key))
    }

    pub(crate) fn clear_drag_over(&mut self) {
        self.strip_refused = None;
        self.state.drag_over(None);
        for depth in 0..self.state.len() {
            if let Some(column) = self.state.column_mut(depth) {
                column.collection.set_drop_over(None);
            }
        }
    }

    pub(crate) fn hovered_folder_drop(&self) -> Option<(usize, EntryId)> {
        for depth in 0..self.open() {
            if self.shows_hits(depth) {
                continue;
            }
            if let Some(DropOver::Row(id)) = self
                .state
                .column(depth)
                .and_then(|column| column.collection.drop_over())
                && self.listings.open_child(depth) != Some(id)
            {
                return Some((depth, id));
            }
        }
        None
    }

    /// The scroll viewport path of each open column's listing, leading column first.
    pub(crate) fn listing_viewports(&self) -> Vec<String> {
        (0..self.state.len())
            .filter_map(|depth| self.state.column_key(depth))
            .map(|key| strata_components::collection::element::viewport_path(&key))
            .collect()
    }

    pub(crate) fn autoscroll(&self) -> &Autoscroll {
        self.state.autoscroll()
    }

    /// Column 0's source, collection state, and viewport path, for a leading marquee origin.
    pub(crate) fn leading_marquee_parts(
        &mut self,
    ) -> Option<(
        &dyn EntrySource,
        &mut strata_components::collection::element::CollectionState,
        String,
    )> {
        let key = self.state.column_key(0)?;
        let source = self.listings.shown(0)?;
        let column = self.state.column_mut(0)?;
        let viewport = strata_components::collection::element::viewport_path(&key);
        Some((source, &mut column.collection, viewport))
    }

    pub(crate) fn activate_leading(&mut self, look: &Look, tx: &mut Tx) {
        if self.state.active() != 0 && !self.state.is_empty() {
            self.state.set_active(0, look, tx);
        }
    }

    pub(crate) fn in_band(&self) -> bool {
        (0..self.state.len()).any(|depth| {
            self.state
                .column(depth)
                .is_some_and(|column| column.collection.selection.in_band())
        })
    }

    /// How many columns both the listings and the view hold.
    fn open(&self) -> usize {
        self.state.len().min(self.listings.columns.len())
    }

    /// Visible `(depth, EntryId)` pairs across open columns, for external file provider queries.
    pub(crate) fn visible_rows(&self) -> Vec<(usize, EntryId)> {
        let cap = strata_fs::services::file_providers::PATH_LIMIT;
        let mut out = Vec::new();
        for depth in 0..self.open() {
            let Some(shown) = self.listings.shown(depth) else {
                continue;
            };
            let len = shown.len();
            let vis = self
                .state
                .column(depth)
                .map_or(0..0, |col| col.collection.visible_range());
            let range = if vis.is_empty() {
                0..len.min(cap)
            } else {
                vis.start..vis.end.min(len)
            };
            out.extend(range.map(|i| (depth, shown.id_at(i))));
        }
        out
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
                self.rebuilding.set(true);
                self.listings.drop_hits();
                self.listings.columns.clear();
                self.held.clear();
                self.kept.clear();
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
            Change::Rows {
                depth, edit, marks, ..
            } => self.edit(depth, edit, marks, hands),
            Change::Loaded {
                depth, load, marks, ..
            } => self.loaded(depth, load, &marks, hands),
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
        self.listings.drop_hits_from(len);
        self.listings.columns.truncate(len);
        self.held.truncate(len);
        self.kept.truncate(len);
        // A sibling takes the place of the column at `len` when it is added, with no exit.
        if !replacing {
            if !self.rebuilding.get() && hands.look.animations {
                hands.tx.animated();
            }
            self.state.truncate(len, hands.look, hands.tx);
        }
    }

    fn add(&mut self, added: Added, hands: &mut Hands<'_>) {
        let Added {
            depth,
            location,
            show_hidden,
            active,
            ..
        } = added;
        self.listings.drop_hits_from(depth);
        self.listings.columns.truncate(depth);
        self.held.truncate(depth);
        self.kept.truncate(depth);
        // The browser adds columns in order, each after the last.
        if self.listings.columns.len() != depth {
            return;
        }
        // The row of the column before it that this folder is: the open path.
        let from = depth.checked_sub(1).map(|_| self.ids.of(&location));
        let worn = self.listings.worn.clone();
        self.listings
            .columns
            .push(Column::new(location, show_hidden, from).with_worn(worn));
        if depth == 0 {
            self.rebuilding.set(true);
            // Another location: the old columns go with no exit.
            self.state
                .replace(&self.listings, 0, &|_| None, hands.look, hands.tx);
            // The GTK window gives a new listing the keyboard too.
            hands.tx.focus(VIEW, false);
        } else {
            // Browser observer callbacks drain from `poll` (no input cause in Rune 631e8a67),
            // including mirroring's 75 ms timer: mark child opens animated unless rebuilding.
            if !self.rebuilding.get() && hands.look.animations {
                hands.tx.animated();
            }
            let opening = Opening::new(&self.listings.columns[depth], Listing::Loading);
            // The browser places the cursor of a folder it opens, once it is listed.
            let focus = if active { Focus::Column } else { Focus::No };
            self.state
                .swap_from(depth, opening, focus, hands.look, hands.tx);
        }
        self.read_menu_keys(depth);
        self.held.push(self.held_now(depth));
        self.kept.push(Kept::default());
    }

    /// Puts the context menu's keys ahead of a new column's own. GTK opens a Columns entry's
    /// menu at its right edge (`ColumnView::context_menu_target`).
    fn read_menu_keys(&mut self, depth: usize) {
        if let Some(column) = self.state.column_mut(depth) {
            commands::view_keys(&mut column.collection, AnchorEdge::Right);
        }
    }

    /// Folders that moved keep their columns, under their new names and with every row read
    /// again: a row's id comes from its location, which changed.
    fn relocate(&mut self, from_depth: usize, moved: Vec<Moved>, hands: &mut Hands<'_>) {
        self.listings.drop_hits_from(from_depth);
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
        let before = column.key_version();
        column.apply(edit);
        if marks.is_none() && before == column.key_version() {
            return;
        }
        self.arrived(depth);
        match marks {
            Some(marks) => self.settle(depth, &marks, hands),
            None => self.sync(depth, hands.tx),
        }
    }

    /// Brings column `depth`'s view state in step with what it shows after that changed: its
    /// rows, or a filter's results in their place.
    fn sync(&mut self, depth: usize, tx: &mut Tx) {
        self.sync_by(depth, handover::sync_rule, tx);
    }

    /// [`Columns::sync`] by `rule`: [`handover::settle_rule`] when the browser's marks follow.
    fn sync_by(&mut self, depth: usize, rule: fn(&Column) -> SyncRule, tx: &mut Tx) {
        let Some(column) = self.listings.shown(depth) else {
            return;
        };
        self.state.sync(depth, column, rule(column), tx);
    }

    /// After column `depth`'s rows changed: the view catches up with them, and then the
    /// browser's cursor and selection stand. The one exception is a cursor the view was
    /// waiting to place until the folder was listed (Right into a column still loading):
    /// that is the user's, and [`Columns::publish`] hands it to the browser.
    fn settle(&mut self, depth: usize, marks: &Marks, hands: &mut Hands<'_>) {
        let before = self.cursor(depth);
        self.sync_by(depth, handover::settle_rule, hands.tx);
        let placed = self
            .state
            .column(depth)
            .is_some_and(|column| handover::placed(before, &column.collection));
        if !placed {
            let kept = self.kept_neighbour(depth, before, marks);
            self.adopt(depth, kept.as_ref().unwrap_or(marks));
            if let Some(slot) = self.kept.get_mut(depth) {
                slot.keep(kept.as_ref());
            }
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
        self.follow_active(active, hands.look, hands.tx);
    }

    pub(crate) fn follow_active(&mut self, active: Option<usize>, look: &Look, tx: &mut Tx) {
        if let Some(active) = active.filter(|active| *active < self.open())
            && active != self.state.active()
        {
            self.state.set_active(active, look, tx);
        }
    }

    fn reveal_cursor(&self, depth: usize, tx: &mut Tx) {
        let (Some(column), Some(state), Some(key)) = (
            self.listings.shown(depth),
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
    /// One poll of what each column says while it shows no entry and its header spinner grace
    /// period: a read is announced once it outlasts GTK's grace period, and its header spinner
    /// shows once it outlasts `COLUMN_SPINNER_DELAY`. True when the view has to run.
    pub(crate) fn watch_surface(&mut self, clock: f64, tx: &mut rune::Tx) -> bool {
        let surface = self
            .surface
            .watch(self.listings.columns.iter().map(Some), clock, tx);
        let spinner = self.listings.watch_spinners(clock, tx);
        surface | spinner
    }

    /// Appends every [`EntryId`] referenced by open columns, active filter hits, or kept
    /// selection neighbours into `out`.
    pub(crate) fn collect_ids(&self, out: &mut impl Extend<EntryId>) {
        self.listings.collect_ids(out);
        for kept in &self.kept {
            if let Some(id) = kept.id() {
                out.extend(std::iter::once(id));
            }
        }
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

    /// What the view holds of the browser's columns: every listing, and each column the strip
    /// draws with its collection.
    pub(crate) fn presented(&self) -> handover::Presented<'_> {
        let shown = (0..self.open())
            .filter_map(|depth| {
                Some(handover::Shown {
                    depth,
                    column: self.listings.columns.get(depth)?,
                    collection: &self.state.column(depth)?.collection,
                })
            })
            .collect();
        let pending = (0..self.open())
            .filter(|depth| self.held.get(*depth) != Some(&self.held_now(*depth)))
            .collect();
        let results: Vec<usize> = (0..self.open())
            .filter(|depth| self.listings.hits_at(*depth))
            .collect();
        let drawn = results
            .iter()
            .filter_map(|depth| {
                Some(handover::Drawn {
                    depth: *depth,
                    column: self.listings.shown(*depth)?,
                    collection: &self.state.column(*depth)?.collection,
                })
            })
            .collect();
        handover::Presented {
            active: Some(self.state.active()),
            strip: Some(self.state.len()),
            listings: self.listings.columns.iter().collect(),
            shown,
            pending,
            results,
            drawn,
            kept: (0..self.open())
                .filter_map(|depth| Some((depth, self.kept.get(depth)?.id()?)))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests;
