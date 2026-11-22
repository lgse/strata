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
    collection::pointer::PointerIntent,
    context_menu::AnchorEdge,
    list_table::{ColumnLayout, ColumnWidths, ListTableLabels, ListTableState, MenuAt},
    scroll::{Autoscroll, AutoscrollPointer, Target},
};
use strata_core::{i18n::tr, util::DateFormat};
use strata_desktop::settings::Preferences;

use crate::{
    App,
    fills::Fills,
    single::{Made, Single},
    slots::Setup,
};

mod columns;
mod dates;
mod part;
mod thumbs;
pub(crate) mod view;

pub(crate) use columns::is_visible as is_column_visible;
pub(crate) use part::tick_active;

/// The table's key: its collection's, and the element that takes the keyboard. Every
/// element key of the view starts with it.
pub(crate) const VIEW: &str = "list";

pub(crate) fn origin_drag(app: &mut App, d: &mut rune::Drag, clears: bool, tx: &mut Tx) {
    let c = if app.filter.pane.shows() {
        view::results(app)
    } else {
        view::ticked(app)
    };
    c.origin_drag(app, (d, clears), tx);
}

pub(crate) fn use_chooser_widths(app: &mut App) {
    app.list.widths = columns::chooser_widths(crate::shell::preferences(app));
}

pub(crate) struct List {
    pub(crate) single: Single,
    table: ListTableState,
    autoscroll: Autoscroll,
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
    /// The active date format, synced with preferences so `cells_version` avoids reparsing it
    /// per row.
    date_format: DateFormat,
    /// Whether the saved date format is relative ("5m ago"), whose text moves with the clock.
    relative_dates: bool,
    /// When the relative dates read again.
    dates: dates::Dates,
    /// The rows' thumbnails, from the window's store.
    thumbs: thumbs::Thumbs,
}

impl List {
    fn with_setup(setup: &Setup<'_>) -> Self {
        let settings: &Preferences = setup.settings;
        let table = ListTableState::new();
        let autoscroll = Autoscroll::new();
        let mut single = Single::new(Made {
            key: VIEW,
            wrap_rows: false,
            left_leaves: true,
            described: tr("List view"),
            menu_at: AnchorEdge::Center,
            worn: setup.worn.clone(),
        });
        single.pane.parts_mut().1.controllers.pointer = Some(Box::new(
            AutoscrollPointer::new(
                Some(Box::new(PointerIntent::new())),
                autoscroll.clone(),
                VIEW,
                table.feed().clone(),
            )
            .sideways(Target::horizontal(
                format!("{VIEW}-scroll"),
                table.sideways_feed().clone(),
            )),
        ));
        let date_format = DateFormat::parse(&settings.date_format);
        let layout = columns::layout(settings);
        let fills = Fills::new(columns::fill_details(&layout));
        Self {
            single,
            table,
            autoscroll,
            layout,
            widths: columns::widths(settings),
            headings: columns::Headings::translated(),
            menu_labels: columns::menu_labels(),
            fills,
            date_format,
            relative_dates: date_format == DateFormat::Relative,
            dates: dates::Dates::default(),
            thumbs: thumbs::Thumbs::new(setup.later.clone()),
        }
    }

    pub(crate) fn autoscroll(&self) -> &Autoscroll {
        &self.autoscroll
    }

    pub(crate) fn sync_columns(&mut self, settings: &Preferences, chooser: bool) {
        self.layout = columns::layout(settings);
        self.widths = if chooser {
            columns::chooser_widths(settings)
        } else {
            columns::widths(settings)
        };
        self.fills.set_details(columns::fill_details(&self.layout));
    }

    pub(crate) fn sync_date_format(&mut self, format: DateFormat) {
        self.date_format = format;
        self.relative_dates = format == DateFormat::Relative;
    }

    pub(crate) fn is_relative_dates(&self) -> bool {
        self.relative_dates
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
