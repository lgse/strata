// SPDX-License-Identifier: MIT

//! What the Columns view asks of its host, taken to `Browser`.

use rune::Tx;
use strata_components::{
    collection::element::{Activation, Intent},
    columns::ColumnsIntent,
    contract::EntryId,
};

use super::Input;
use crate::{
    App, filter,
    menus::{self, Request},
    rename, shell,
    slots::At,
};

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

    fn collection(&mut self, depth: usize, intent: Intent, tx: &mut Tx) {
        // A filter's results open as GTK's hits do, and are renamed nowhere yet.
        if self.columns.shows_hits(depth) && filter::hit_intent(self, depth, &intent, tx) {
            return;
        }
        match intent {
            Intent::Activate { id, how } => self.activate(depth, id, how),
            Intent::Clicked { id } => self.clicked(depth, id),
            // A slow second click on the entry.
            Intent::BeginRename { id } => rename::begin_on(self, depth, id, tx),
            Intent::ContextMenu { target, anchor } => {
                let menu = Request {
                    depth,
                    entry: target,
                    anchor,
                };
                menus::open(self, menu, tx);
            }
            // Drags and new tabs are interface the slice leaves out. Up and
            // Down at a column's edge have nowhere to go without a header to focus: the view
            // has selected the cursor's entry alone again, as the browser does there.
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
            rested: matches!(intent, ColumnsIntent::Cursor { .. }),
        },
        false => Input::Pointer,
    };
    app.columns.publish(&app.browser, Some(input));
    match intent {
        ColumnsIntent::Collection { depth, intent } => app.collection(depth, intent, tx),
        ColumnsIntent::EnterFolder { depth, id } if app.columns.shows_hits(depth) => {
            filter::open_hit(app, depth, id);
        }
        ColumnsIntent::EnterFolder { depth, id } => app.enter(depth, id),
        ColumnsIntent::CloseFrom(depth) => app.browser.close_column(depth),
        // The user picked among a filter's results: Enter follows the pick from now on.
        ColumnsIntent::Cursor { depth, .. } | ColumnsIntent::Selection { depth }
            if app.columns.shows_hits(depth) =>
        {
            filter::picked(app);
        }
        // Published above, and with it the active column a key moved the keyboard to.
        ColumnsIntent::Cursor { .. }
        | ColumnsIntent::Selection { .. }
        | ColumnsIntent::Active { .. } => {}
        ColumnsIntent::HeaderMenu { depth, anchor } => {
            let folder = Request {
                depth,
                entry: None,
                anchor,
            };
            menus::open(app, folder, tx);
        }
        // The sidebar takes the keyboard, when there is one.
        ColumnsIntent::LeftOfRoot => {
            shell::leave_view_at_start(app, tx);
        }
        ColumnsIntent::Width(width) => super::width::keep(app, width),
        // `Dropped` is drag and drop's.
        _ => {}
    }
    shell::settle(app, tx);
}

/// Opens the entry of a row as a click on it does: what the context menu's Open asks
/// (`open_context_entry` in `src/ui/browser/context_menu/item_menu.rs`).
pub(super) fn clicked_open(app: &mut App, (depth, id): At) {
    if app.columns.shows_hits(depth) {
        filter::open_hit(app, depth, id);
        return;
    }
    app.activate(depth, id, Activation::Click);
}
