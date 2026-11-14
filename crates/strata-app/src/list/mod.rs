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
    collection::keyboard::{KeySettings, Letters},
    context_menu::AnchorEdge,
    list_table::{ColumnLayout, ColumnWidths, ListTableLabels, ListTableState, MenuAt},
};
use strata_core::{i18n::tr, util::DateFormat};
use strata_desktop::settings::Preferences;

use crate::{
    App,
    fills::{Details, Fills},
    single::{Made, Single},
    slots::Later,
};

mod columns;
mod dates;
mod part;
mod thumbs;
mod view;

/// The table's key: its collection's, and the element that takes the keyboard. Every
/// element key of the view starts with it.
pub(crate) const VIEW: &str = "list";

pub(crate) struct List {
    single: Single,
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

/// The keys of the List table. Every row is at the pane's left edge (`at_left_edge` in
/// `src/ui/browser_modes/focus.rs`), so Left leaves it with or without a cursor. With "Type
/// to search" off a plain letter starts no search, so `h j k l` are the arrows:
/// `Context::of` in `src/ui/keys/alias.rs`.
fn keys(typing: bool) -> KeySettings {
    let mut keys = KeySettings::default();
    keys.left_leaves_without_cursor = true;
    match typing {
        true => keys,
        false => keys.letters(Letters::Arrows),
    }
}

impl List {
    fn with_settings(settings: &Preferences, later: Later<App>) -> Self {
        // `type_to_search_active` in `src/ui/preferences/input_settings.rs`.
        let typing = settings.type_to_search && !settings.tenxer_mode;
        // `explorer_file_clicks` and `explorer_folder_clicks`: the List view's.
        let clicks = (settings.list_file_clicks, settings.list_folder_clicks);
        Self {
            single: Single::new(Made {
                key: VIEW,
                clicks,
                keys: keys(typing),
                described: tr("List view"),
                menu_at: AnchorEdge::Center,
            }),
            table: ListTableState::new(),
            layout: columns::layout(settings),
            widths: columns::widths(settings),
            headings: columns::Headings::translated(),
            menu_labels: columns::menu_labels(),
            fills: Fills::new(Details::Folders),
            relative_dates: DateFormat::parse(&settings.date_format) == DateFormat::Relative,
            dates: dates::Dates::default(),
            thumbs: thumbs::Thumbs::new(later),
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
mod tests;
