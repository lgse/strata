// SPDX-License-Identifier: MIT

//! What the Columns view asks of its host, taken to `Browser`.

use rune::Tx;
use strata_components::{
    collection::element::{Activation, Intent},
    columns::ColumnsIntent,
    contract::EntryId,
};

use super::Input;
use crate::shell::{self, App};

impl App {
    /// Whether the browser's focused item is `position` of column `depth`.
    fn browser_is_on(&self, depth: usize, position: usize) -> bool {
        self.browser
            .focused_item()
            .is_some_and(|(focused_depth, focused, _)| {
                (focused_depth, focused) == (depth, position)
            })
    }

    fn activate(&self, depth: usize, id: EntryId, how: Activation) {
        let Some(position) = self.columns.position(depth, id) else {
            return;
        };
        match how {
            // Return opens a folder with the cursor on its first entry: `activate_focused`
            // in `src/ui/browser/view_navigation.rs`.
            Activation::Key if self.browser_is_on(depth, position) => {
                self.browser.activate_focused();
            }
            // A click opens it with no cursor: `activate_column_entry` in
            // `src/ui/browser/columns/state.rs`.
            _ => self.browser.activate(depth, position),
        }
    }

    /// Right on a folder whose column is not open: `enter_focused_directory`, as
    /// `src/ui/window/keyboard/items.rs` calls it.
    fn enter(&self, depth: usize, id: EntryId) {
        let Some(position) = self.columns.position(depth, id) else {
            return;
        };
        if self.browser_is_on(depth, position) {
            self.browser.enter_focused_directory();
        } else {
            self.browser.activate(depth, position);
        }
    }

    /// A plain click on a file in a column before the last closes the columns after it:
    /// `PendingActivationKind::CloseDeeper` in `src/ui/browser/columns/rows/click.rs`.
    fn clicked(&self, depth: usize, id: EntryId) {
        let file = self
            .columns
            .row(depth, id)
            .is_some_and(|row| !row.is_folder());
        if file && self.browser.location_at(depth + 1).is_some() {
            self.browser.close_column(depth + 1);
        }
    }

    fn collection(&self, depth: usize, intent: Intent) {
        match intent {
            Intent::Activate { id, how } => self.activate(depth, id, how),
            Intent::Clicked { id } => self.clicked(depth, id),
            // V9 opens the editor here.
            Intent::BeginRename { .. } => tracing::debug!(depth, "the view asked for a rename"),
            // Context menus, drags and new tabs are interface the slice leaves out. Up and
            // Down at a column's edge have nowhere to go without a header to focus.
            _ => {}
        }
    }
}

pub(super) fn heard(app: &mut App, intent: ColumnsIntent, tx: &mut Tx) {
    // What the input did to the cursor and the selection goes first: the browser acts on
    // its own.
    let input = match tx.by_keys() {
        true => Input::Keys {
            extending: tx.mods().shift,
        },
        false => Input::Pointer,
    };
    app.columns.publish(&app.browser, Some(input));
    match intent {
        ColumnsIntent::Collection { depth, intent } => app.collection(depth, intent),
        ColumnsIntent::EnterFolder { depth, id } => app.enter(depth, id),
        ColumnsIntent::CloseFrom(depth) => app.browser.close_column(depth),
        // Published above.
        ColumnsIntent::Cursor { .. } | ColumnsIntent::Selection { .. } => {}
        // `LeftOfRoot` is the sidebar's, which the slice has not. `Width`, `HeaderMenu` and
        // `Dropped` are saved widths, menus and drag and drop: out of the slice.
        _ => {}
    }
    shell::settle(app, tx);
}
