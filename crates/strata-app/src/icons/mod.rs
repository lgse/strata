// SPDX-License-Identifier: MIT

//! The Icons view: Strata's `Browser` on one side, strata-components' `IconsGrid` on the
//! other, over the one folder a [`Pane`] shows.
//!
//! - **Events in.** The view's listener queues what each event changed ([`crate::events`]),
//!   and the pane puts what is for its folder into its listing and its collection.
//! - **Intents out.** What the grid asks goes to the browser ([`intents`]): a folder opens in
//!   place, as GTK's Icons view opens it (`activate_in_place`).
//! - **One cursor and one selection,** the pane's: the grid interprets keys and the pointer,
//!   and the browser holds the result.

use strata_components::{
    collection::{
        element::CollectionState,
        keyboard::{KeySettings, Keyboard, Letters},
        marquee::Marquee,
        pointer::{ClickCount, ClickModes, PointerIntent},
    },
    icons_grid::IconsGridState,
};
use strata_desktop::settings::Preferences;

use crate::{listing::Words, pane::Pane};

mod intents;
mod part;
mod view;

/// The grid's key: its collection's, and the element that takes the keyboard. Every element
/// key of the view starts with it.
pub(crate) const VIEW: &str = "icons";

pub(crate) struct Icons {
    pane: Pane,
    grid: IconsGridState,
    /// The tile size in points, from 32 to 256.
    size: f32,
    clicks: ClickModes,
    /// The keys the grid reads other than by its default.
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

/// The keys of an Icons grid: Left and Right cross the ends of rows, as GTK's GridView moves.
/// With "Type to search" off a plain letter starts no search, so `h j k l` are the arrows:
/// `Context::of` in `src/ui/keys/alias.rs`.
fn keys(typing: bool) -> KeySettings {
    let keys = KeySettings::default().wrap_rows(true);
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

impl Icons {
    fn with_settings(settings: &Preferences) -> Self {
        // `type_to_search_active` in `src/ui/preferences/input_settings.rs`.
        let typing = settings.type_to_search && !settings.tenxer_mode;
        Self {
            pane: Pane::new(VIEW, collection()),
            grid: IconsGridState::new(),
            // The saved size, which loading holds between 32 and 256 (`stored.rs` in
            // `strata_desktop::settings`). The size a folder remembers is B2's store's.
            size: f32::from(u16::try_from(settings.icons_thumbnail_size).unwrap_or(64)),
            clicks: ClickModes::new(
                clicks(settings.icons_file_clicks),
                clicks(settings.icons_folder_clicks),
            ),
            keys: keys(typing),
            words: Words::translated(),
            labels: view::Labels::translated(),
        }
    }
}

#[cfg(test)]
mod tests;
