// SPDX-License-Identifier: MIT

//! 10xer listing **Esc**: each press ends exactly one interaction, in the
//! order documented under "Escape precedence" in `docs/10xer-mode.md`.

use gtk::glib::Propagation;

use super::{Dispatcher, KeyResult};
use crate::app::Browser;

impl Dispatcher {
    /// Runs once no modal, prompt, chord, or preview owns the key. Always
    /// consumes it: with nothing left to dismiss, **Esc** neither closes a
    /// Miller column nor reaches the window.
    pub(super) fn tenxer_escape(&self, browser: &Browser) -> KeyResult {
        self.dismiss_one_interaction(browser);
        Some(Propagation::Stop)
    }

    fn dismiss_one_interaction(&self, browser: &Browser) -> bool {
        if browser.close_peek() {
            return true;
        }
        if self.view.listing_search_active() {
            return self.view.leave_visual()
                || self.view.dismiss_find_highlight()
                || self.close_open_preview(browser)
                || self.view.dismiss_listing_search();
        }
        // A range over f results lives on those results, so it must end
        // before the filter that shows them.
        self.view.leave_result_visual()
            || self.view.clear_listing_filter()
            || self.view.dismiss_find_highlight()
            || self.view.leave_visual()
            || self.close_open_preview(browser)
            || browser.clear_active_selection()
    }

    fn close_open_preview(&self, browser: &Browser) -> bool {
        if !self.preview.is_enabled() {
            return false;
        }
        self.close_preview(browser);
        true
    }
}
