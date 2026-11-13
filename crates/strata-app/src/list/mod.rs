// SPDX-License-Identifier: MIT

//! The List view: Strata's `Browser` on one side, strata-components' `ListTable` on the
//! other, over the one folder a [`Pane`](crate::pane::Pane) shows. What it shares with the
//! Icons view is [`crate::single`]'s: the events in, the intents out, and one cursor and one
//! selection, the pane's. A folder opens in place, as GTK's List view opens it
//! (`activate_in_place`).
//!
//! The table shows the Name column alone for now: the cells of Mode, Size, Type and Modified
//! need text the listing does not copy yet, and their formatters are the GTK application's
//! (`src/ui/browser_modes/metadata.rs`).

use strata_components::{
    collection::keyboard::{KeySettings, Letters},
    context_menu::AnchorEdge,
    list_table::{ColumnLayout, ColumnWidths, ListTableState},
};
use strata_core::i18n::tr;
use strata_desktop::settings::Preferences;

use crate::single::{Made, Single};

mod part;
mod view;

/// The table's key: its collection's, and the element that takes the keyboard. Every
/// element key of the view starts with it.
pub(crate) const VIEW: &str = "list";

pub(crate) struct List {
    single: Single,
    table: ListTableState,
    /// The columns shown, in order.
    layout: ColumnLayout,
    /// The column widths set by hand in this window.
    widths: ColumnWidths,
    /// The Name column's heading (`list_headings.rs` in `src/ui/browser_modes`).
    name: String,
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
    fn with_settings(settings: &Preferences) -> Self {
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
            layout: view::layout(),
            widths: ColumnWidths::default(),
            name: view::name_heading(),
        }
    }
}

#[cfg(test)]
mod tests;
