// SPDX-License-Identifier: MIT

//! The List view: Strata's `Browser` on one side, strata-components' `ListTable` on the
//! other, over the one folder a [`Pane`] shows.
//!
//! - **Events in.** The view's listener queues what each event changed ([`crate::events`]),
//!   and the pane puts what is for its folder into its listing and its collection.
//! - **Intents out.** What the table asks goes to the browser ([`intents`]): a folder opens
//!   in place, as GTK's List view opens it (`activate_in_place`).
//! - **One cursor and one selection,** the pane's: the table interprets keys and the
//!   pointer, and the browser holds the result.
//! - **The columns.** The table shows the Name column alone for now: the cells of Mode,
//!   Size, Type and Modified need text the listing does not copy yet, and their formatters
//!   are the GTK application's (`src/ui/browser_modes/metadata.rs`).

use strata_components::{
    collection::{
        element::CollectionState,
        keyboard::{KeySettings, Keyboard, Letters},
        marquee::Marquee,
        pointer::{ClickCount, ClickModes, PointerIntent},
    },
    list_table::{ColumnLayout, ColumnWidths, ListTableState},
};
use strata_desktop::settings::Preferences;

use crate::{listing::Words, pane::Pane};

mod intents;
mod part;
mod view;

/// The table's key: its collection's, and the element that takes the keyboard. Every
/// element key of the view starts with it.
pub(crate) const VIEW: &str = "list";

pub(crate) struct List {
    pane: Pane,
    table: ListTableState,
    /// The columns shown, in order.
    layout: ColumnLayout,
    /// The column widths set by hand in this window.
    widths: ColumnWidths,
    clicks: ClickModes,
    /// The keys the table reads other than by its default.
    keys: KeySettings,
    words: Words,
    labels: view::Labels,
}

fn clicks(count: u8) -> ClickCount {
    match count {
        1 => ClickCount::One,
        _ => ClickCount::Two,
    }
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

/// The collection's controllers: the pointer's clicks, the keyboard and the marquee.
fn collection() -> CollectionState {
    let mut collection = CollectionState::new();
    collection.controllers.pointer = Some(Box::new(PointerIntent::new()));
    collection.controllers.keyboard = Some(Box::new(Keyboard::default()));
    collection.controllers.marquee = Some(Box::new(Marquee::new()));
    collection
}

impl List {
    fn with_settings(settings: &Preferences) -> Self {
        // `type_to_search_active` in `src/ui/preferences/input_settings.rs`.
        let typing = settings.type_to_search && !settings.tenxer_mode;
        Self {
            pane: Pane::new(VIEW, collection()),
            table: ListTableState::new(),
            layout: view::layout(),
            widths: ColumnWidths::default(),
            // `explorer_file_clicks` and `explorer_folder_clicks`: the List view's.
            clicks: ClickModes::new(
                clicks(settings.list_file_clicks),
                clicks(settings.list_folder_clicks),
            ),
            keys: keys(typing),
            words: Words::translated(),
            labels: view::Labels::translated(),
        }
    }
}

#[cfg(test)]
mod tests;
