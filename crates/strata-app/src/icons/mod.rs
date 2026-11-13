// SPDX-License-Identifier: MIT

//! The Icons view: Strata's `Browser` on one side, strata-components' `IconsGrid` on the
//! other, over the one folder a [`Pane`](crate::pane::Pane) shows. What it shares with the
//! List view is [`crate::single`]'s: the events in, the intents out, and one cursor and one
//! selection, the pane's. A folder opens in place, as GTK's Icons view opens it
//! (`activate_in_place`).

use strata_components::{
    collection::keyboard::{KeySettings, Letters},
    context_menu::AnchorEdge,
    icons_grid::IconsGridState,
};
use strata_core::i18n::tr;
use strata_desktop::settings::Preferences;

use crate::{
    App,
    single::{Made, Single},
    slots::Later,
    thumbnails::Viewer,
};

mod part;
mod view;

/// The grid's key: its collection's, and the element that takes the keyboard. Every element
/// key of the view starts with it.
pub(crate) const VIEW: &str = "icons";

pub(crate) struct Icons {
    single: Single,
    grid: IconsGridState,
    /// The tile size in points, from 32 to 256.
    size: f32,
    /// The grid's viewer in the window's thumbnails.
    thumbs: Viewer,
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

impl Icons {
    fn with_settings(settings: &Preferences, later: Later<App>) -> Self {
        // `type_to_search_active` in `src/ui/preferences/input_settings.rs`.
        let typing = settings.type_to_search && !settings.tenxer_mode;
        let clicks = (settings.icons_file_clicks, settings.icons_folder_clicks);
        Self {
            single: Single::new(Made {
                key: VIEW,
                clicks,
                keys: keys(typing),
                described: tr("Icons view"),
                menu_at: AnchorEdge::Right,
            }),
            grid: IconsGridState::new(),
            // The saved size, which loading holds between 32 and 256 (`stored.rs` in
            // `strata_desktop::settings`). The size a folder remembers is B2's store's.
            size: f32::from(u16::try_from(settings.icons_thumbnail_size).unwrap_or(64)),
            thumbs: Viewer::new(later),
        }
    }
}
