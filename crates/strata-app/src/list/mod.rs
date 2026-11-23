// SPDX-License-Identifier: MIT

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
pub(crate) mod dates;
mod part;
mod thumbs;
pub(crate) mod view;

pub(crate) use columns::is_visible as is_column_visible;
pub(crate) use part::tick_active;

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
    headings: columns::Headings,
    /// The columns menu's and its button's words (`column_options.rs`, `list_pane.rs`).
    menu_labels: ListTableLabels,
    /// The metadata fills of the rows near the viewport, with a folder's item count.
    fills: Fills,
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
