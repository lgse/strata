// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::prelude::*;

use crate::adapters::bookmarks::{BookmarkWatch, watch_changes};

use super::{SidebarState, load_pinned_places, show_error_dialog};

pub(super) fn watch_sidebar(state: &Rc<SidebarState>) -> BookmarkWatch {
    let weak = Rc::downgrade(state);
    watch_changes(move |result| {
        let Some(state) = weak.upgrade() else {
            return false;
        };
        let places = result
            .as_ref()
            .map_err(ToString::to_string)
            .and_then(|_| load_pinned_places().map_err(|error| error.to_string()));
        match places {
            Ok(places) => {
                if *state.pinned_places.borrow() != places {
                    state.pinned_places.replace(places);
                    state.rebuild();
                }
                true
            }
            Err(error) => {
                if state.view.widget().root().is_none() {
                    return false;
                }
                show_error_dialog(
                    &state.view.widget(),
                    "Unable to update pinned folders",
                    &error,
                );
                true
            }
        }
    })
}

#[cfg(test)]
mod tests;
