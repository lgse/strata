// SPDX-License-Identifier: MIT

//! Where the filter is open, and what it asks of that view: a Columns column, whose rows its
//! results take the place of ([`crate::columns`]), or the one pane of Icons or List, where its
//! results show in a collection of their own ([`super::pane`]), as GTK's "Search results" list
//! or grid takes the pane's listing's place (`inline_search::wrap`).

use rune::Tx;
use strata_components::{
    collection::selection::{Position, Selection},
    contract::{EntryId, EntrySource},
};

use crate::{
    App,
    icons::Icons,
    list::List,
    listing::{Column, Load},
    pane::Pane,
    shell,
    single::SinglePane,
};

/// The view the filter is open in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Where {
    Columns,
    Icons,
    List,
}

impl Where {
    /// The view shown, when it is one the filter opens in.
    pub(crate) fn shown(app: &App) -> Option<Self> {
        let view = shell::shown_view(app)?;
        [
            ("columns", Self::Columns),
            ("icons", Self::Icons),
            ("list", Self::List),
        ]
        .into_iter()
        .find_map(|(mode, place)| (view.shows)(mode).then_some(place))
    }

    /// The key of the view's element that takes the keyboard: the results' too in a pane.
    pub(crate) fn view_key(self) -> &'static str {
        match self {
            Self::Columns => crate::columns::VIEW,
            Self::Icons => Icons::KEY,
            Self::List => List::KEY,
        }
    }

    /// The pane of Icons or List.
    pub(crate) fn pane(self, app: &App) -> Option<&Pane> {
        match self {
            Self::Columns => None,
            Self::Icons => Some(&Icons::single(app).pane),
            Self::List => Some(&List::single(app).pane),
        }
    }

    /// The column the filter opens in: the one the pointer or the keys last targeted in
    /// Columns (GTK's `destination_depth`), the folder shown in a pane.
    pub(crate) fn targeted(self, app: &App) -> Option<usize> {
        match self {
            Self::Columns => Some(app.columns.targeted()),
            pane => pane.pane(app)?.depth(),
        }
    }

    /// Column `depth`'s folder rows, when this view shows them.
    fn listing(self, app: &App, depth: usize) -> Option<&Column> {
        match self {
            Self::Columns => app.columns.listing(depth),
            pane => pane
                .pane(app)?
                .listing()
                .filter(|_| self.depth_shown(app, depth)),
        }
    }

    fn depth_shown(self, app: &App, depth: usize) -> bool {
        self.pane(app)
            .is_some_and(|pane| pane.depth() == Some(depth))
    }

    /// Whether the filter's results show in the place of column `depth`'s rows.
    pub(crate) fn shows_hits(self, app: &App, depth: usize) -> bool {
        match self {
            Self::Columns => app.columns.shows_hits(depth),
            _ => self.depth_shown(app, depth) && app.filter.pane.shows(),
        }
    }

    /// What column `depth` shows now, results or rows, and the selection over it.
    pub(crate) fn shown_with_selection(
        self,
        app: &App,
        depth: usize,
    ) -> Option<(&dyn EntrySource, &Selection)> {
        match self {
            Self::Columns => app.columns.shown_with_selection(depth),
            _ if self.shows_hits(app, depth) => app.filter.pane.shown_with_selection(),
            pane => {
                let pane = pane.pane(app).filter(|_| self.depth_shown(app, depth))?;
                Some((pane.source(), &pane.collection().selection))
            }
        }
    }

    /// How many of its folder's rows column `depth` has: what the placeholder counts.
    pub(crate) fn listing_len(self, app: &App, depth: usize) -> usize {
        self.listing(app, depth).map_or(0, EntrySource::len)
    }

    /// Column `depth`'s selection over its folder's rows. A pane keeps its own under the
    /// results, so it has none to give back.
    pub(crate) fn listing_position(self, app: &App, depth: usize) -> Position {
        match self {
            Self::Columns => app.columns.listing_position(depth),
            _ => Position::default(),
        }
    }

    /// Whether column `depth`'s folder has been read.
    pub(crate) fn listed(self, app: &App, depth: usize) -> bool {
        match self {
            Self::Columns => app.columns.listed(depth),
            _ => self
                .listing(app, depth)
                .is_some_and(|column| *column.load() == Load::Listed),
        }
    }

    /// Shows `hits` in the place of column `depth`'s rows, nothing selected when `clear`.
    pub(crate) fn show_hits(
        self,
        app: &mut App,
        depth: usize,
        hits: Column,
        clear: bool,
        tx: &mut Tx,
    ) {
        match self {
            Self::Columns => app.columns.show_hits(depth, hits, clear, tx),
            _ if self.depth_shown(app, depth) => {
                app.filter.pane.show(self, hits, clear, tx);
            }
            _ => {}
        }
    }

    /// Gives the rows back, with the selection `restore` they had in Columns.
    pub(crate) fn drop_hits(self, app: &mut App, restore: Option<&Position>, tx: &mut Tx) {
        match self {
            Self::Columns => app.columns.drop_hits(restore, tx),
            _ => app.filter.pane.drop_hits(),
        }
    }

    /// Scrolls result `index` of column `depth` into view.
    pub(crate) fn reveal_index(self, app: &App, depth: usize, index: usize, tx: &mut Tx) {
        match self {
            Self::Columns => app.columns.reveal_index(depth, index, tx),
            _ => app.filter.pane.reveal_index(self.view_key(), index, tx),
        }
    }

    /// Down from the field: the cursor on result `id`, which alone is selected, with the
    /// keyboard in the results.
    pub(crate) fn land(self, app: &mut App, depth: usize, id: EntryId, tx: &mut Tx) {
        match self {
            Self::Columns => app.columns.land(depth, id, &app.look, tx),
            _ => app.filter.pane.land(self.view_key(), id, tx),
        }
        tx.focus(self.view_key(), true);
    }
}
