// SPDX-License-Identifier: MIT

//! Commands a map's own stages leave out, run from a key a user binds to them
//! in that map's table. Each runs the handler its own map already uses.

use gtk::glib::Propagation;

use super::{Dispatcher, KeyEvent, KeyResult};
use crate::ui::{
    keys::{self, KeyPress, Map, bindings::*},
    tenxer_mode::Prompt,
};

impl Dispatcher {
    fn active_map(&self) -> Map {
        if self.type_to_search.preferences.tenxer_mode() {
            Map::Tenxer
        } else {
            Map::Default
        }
    }

    /// Whether a typed key belongs to a direct command, which the focused 10xer
    /// listing must pass on instead of keeping.
    pub(super) fn runs_directly(&self, press: &KeyPress) -> bool {
        self.chooser.is_none() && press.is_any(keys::direct_commands(self.active_map()))
    }

    /// Escape in the default map ends a search a direct key started, as the
    /// 10xer Escape stage does; an open preview closes first.
    pub(super) fn dismiss_direct_search(&self, event: &KeyEvent) -> bool {
        self.active_map() == Map::Default
            && !event.text_has_focus()
            && !self.preview.is_enabled()
            && self.view.item_view_has_focus()
            && self.view.listing_search_active()
            && self.view.dismiss_listing_search()
    }

    /// Runs outside text fields, after the window's commands and before inline
    /// editing. File choosers run none of these, as before.
    pub(super) fn direct_commands(&self, event: &KeyEvent) -> KeyResult {
        if event.text_has_focus() || self.chooser.is_some() {
            return None;
        }
        let press = &event.press;
        let map = self.active_map();
        let listing = self.view.item_view_has_focus();
        if map == Map::Default
            && let Some(index) = keys::CUSTOM_ACTIONS.iter().position(|slot| press.is(*slot))
        {
            if !listing {
                return None;
            }
            self.run_listed_action(index);
            return Some(Propagation::Stop);
        }
        // Shift does not reverse a direct sort key, which would take the
        // Shifted key from whatever else holds it.
        let command = keys::direct_commands(map)
            .iter()
            .copied()
            .find(|command| press.is(*command))?;
        if command == DUPLICATE {
            return self
                .view
                .duplicate_entries(&self.view.command_targets())
                .then_some(Propagation::Stop);
        }
        if command == JUMP_TO_FOLDER || command == ARROW_SCOPE {
            return Some(self.window_action(press));
        }
        if !listing {
            return None;
        }
        if command == OPEN_WITH {
            self.open_with_targets();
        } else if command == MOVE_TO {
            self.open_transfer_prompt(Prompt::MoveTo);
        } else if command == COPY_TO {
            self.open_transfer_prompt(Prompt::CopyTo);
        } else if command == RESTORE {
            self.restore_targets();
        } else if command == INVERT_SELECTION {
            self.invert_selection();
        } else if command == SEARCH_SUBFOLDERS {
            self.open_search_prompt();
        } else if command == JUMP_VISITED {
            self.open_history_prompt(Prompt::Jump);
        } else if command == JUMP_RECENT {
            self.open_history_prompt(Prompt::Recent);
        } else if command == HALF_PAGE_UP || command == HALF_PAGE_DOWN {
            let direction = if command == HALF_PAGE_UP { -1 } else { 1 };
            // The selection moves, as Page Down's does, so default-map verbs
            // never act on an item paged out of view. Results keep their stepping.
            if self.view.selected_search_results().is_some()
                || !self.view.half_page_selection(direction)
            {
                self.page_cursor(direction, true);
            }
        } else if command == PIN {
            // The search hit under the cursor, not the directory hidden behind it.
            if let Some(entry) = self.view.focused_target() {
                self.view.pin_entry(entry);
            }
        } else if !(self.complete_sort(press)
            || self.complete_copy(press)
            || self.complete_folder_action(press))
        {
            return None;
        }
        Some(Propagation::Stop)
    }
}
