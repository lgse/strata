// SPDX-License-Identifier: MIT

//! The List view: Strata's `Browser` on one side, strata-components' `ListTable` on the
//! other, over the one folder a [`Pane`](crate::pane::Pane) shows. What it shares with the
//! Icons view is [`crate::single`]'s: the events in, the intents out, and one cursor and one
//! selection, the pane's. A folder opens in place, as GTK's List view opens it
//! (`activate_in_place`).
//!
//! The table has GTK's five columns ([`columns`]): Name, Mode, Size, Type and Modified, in the
//! saved order and visibility, at the saved widths. What a row does not know yet (a size, a
//! date, a mode) is asked for while it is near the viewport ([`crate::fills`]).
//! An image's row draws its thumbnail at 18 points, from the window's store ([`thumbs`]).

use rune::Tx;
use strata_components::{
    context_menu::AnchorEdge,
    list_table::{ColumnLayout, ColumnWidths, ListTableLabels, ListTableState, MenuAt},
};
use strata_core::{i18n::tr, util::DateFormat};
use strata_desktop::settings::Preferences;

use crate::{
    App,
    fills::{Details, Fills},
    single::{Made, Single},
    slots::Setup,
};

mod columns;
mod dates;
mod part;
mod thumbs;
mod view;

pub(crate) use part::tick_active;

/// The table's key: its collection's, and the element that takes the keyboard. Every
/// element key of the view starts with it.
pub(crate) const VIEW: &str = "list";

pub(crate) struct List {
    pub(crate) single: Single,
    table: ListTableState,
    /// The columns shown, in order: the saved `list_column_order` and `list_hidden_columns`.
    layout: ColumnLayout,
    /// The column widths set by hand: the saved `browser_list_columns`.
    widths: ColumnWidths,
    /// The columns' headings (`list_headings.rs` in `src/ui/browser_modes`).
    headings: columns::Headings,
    /// The columns menu's and its button's words (`column_options.rs`, `list_pane.rs`).
    menu_labels: ListTableLabels,
    /// The metadata fills of the rows near the viewport, with a folder's item count.
    fills: Fills,
    /// The saved date format is relative ("5m ago"), whose text moves with the clock. Read
    /// at start: a live change is A18's.
    relative_dates: bool,
    /// When the relative dates read again.
    dates: dates::Dates,
    /// The rows' thumbnails, from the window's store.
    thumbs: thumbs::Thumbs,
}

impl List {
    fn with_setup(setup: &Setup<'_>) -> Self {
        let settings: &Preferences = setup.settings;
        // `explorer_file_clicks` and `explorer_folder_clicks`: the List view's.
        let clicks = (settings.list_file_clicks, settings.list_folder_clicks);
        Self {
            single: Single::new(Made {
                key: VIEW,
                clicks,
                wrap_rows: false,
                left_leaves: true,
                described: tr("List view"),
                menu_at: AnchorEdge::Center,
                worn: setup.worn.clone(),
            }),
            table: ListTableState::new(),
            layout: columns::layout(settings),
            widths: columns::widths(settings),
            headings: columns::Headings::translated(),
            menu_labels: columns::menu_labels(),
            fills: Fills::new(Details::Folders),
            relative_dates: DateFormat::parse(&settings.date_format) == DateFormat::Relative,
            dates: dates::Dates::default(),
            thumbs: thumbs::Thumbs::new(setup.later.clone()),
        }
    }
}

/// "Customize columns…": the columns menu opens under the heading strip's right end, as
/// GTK's opens on the headings (`open_list_columns_popover`). The keyboard goes to the table
/// first, so the menu gives it back there when it closes: GTK's popover is the headings box's
/// and gives it to the first heading, "Name", which C4 keeps off the focus chain, the table
/// being one stop (lgse/strata#2331, the owner's). The List view is shown.
pub(crate) fn customize_columns(app: &mut App, tx: &mut Tx) {
    tx.focus(VIEW, true);
    app.list.table.open_columns_menu(MenuAt::Headings(None));
}

#[cfg(test)]
impl List {
    pub(crate) fn source(&self) -> &dyn strata_components::contract::EntrySource {
        self.single.pane.source()
    }
}

#[cfg(test)]
mod tests;
