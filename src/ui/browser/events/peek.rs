// SPDX-License-Identifier: MIT

//! The peek column's listing batches and completion.

use crate::model::FileEntry;
use crate::ui::browser::ViewState;
use crate::ui::browser::peek::append_peek_entries;
use gtk::prelude::*;

impl ViewState {
    pub(super) fn on_peek_entries_added(&self, entries: &[FileEntry]) {
        let peek = self.peek.borrow();
        let Some(peek) = peek.as_ref() else {
            return;
        };
        if !entries.is_empty() {
            peek.presentation.show_content();
        }
        append_peek_entries(peek, entries.to_vec(), self.peek_behavior.item_limit);
    }

    pub(super) fn on_peek_finished(&self) {
        let peek = self.peek.borrow();
        let Some(peek) = peek.as_ref() else {
            return;
        };
        peek.spinner.stop();
        peek.spinner.set_visible(false);
        if peek.entry_count.get() == 0 {
            peek.presentation.show_empty();
        } else {
            peek.presentation.show_content();
        }
    }

    pub(super) fn on_peek_failed(&self, message: &str) {
        let peek = self.peek.borrow();
        let Some(peek) = peek.as_ref() else {
            return;
        };
        peek.spinner.stop();
        peek.spinner.set_visible(false);
        peek.presentation.show_error(
            &rust_i18n::t!(
                "Unable to read this directory\n%{message}",
                message = message
            ),
            false,
        );
    }
}
