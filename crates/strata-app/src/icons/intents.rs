// SPDX-License-Identifier: MIT

//! What the Icons grid asks of its host, taken to `Browser`.

use rune::Tx;
use strata_components::{
    collection::element::{Activation, Intent},
    contract::{Direction, EntryId},
};

use crate::{App, rename, shell, slots::At};

impl App {
    /// Opens entry `id` of column `depth` in place, as GTK's Icons view does: a folder
    /// becomes the folder shown. Return opens the browser's focused item
    /// (`activate_focused` in `src/ui/browser/view_navigation.rs`), a click the entry clicked
    /// (`activate_in_place` in `src/ui/browser_modes/clicks.rs`).
    fn open_in_place(&self, depth: usize, id: EntryId, how: Activation) {
        let Some(position) = self.icons.pane.position(depth, id) else {
            return;
        };
        let focused = self
            .browser
            .focused_item()
            .is_some_and(|(at, focused, _)| (at, focused) == (depth, position));
        match how {
            Activation::Key if focused => self.browser.activate_focused_in_place(),
            _ => self.browser.activate_in_place(depth, position),
        }
    }
}

pub(super) fn heard(app: &mut App, intent: Intent, tx: &mut Tx) {
    // What the input did to the cursor and the selection goes first: the browser acts on
    // its own.
    shell::publish(app);
    let Some(depth) = app.icons.pane.depth() else {
        return;
    };
    match intent {
        Intent::Activate { id, how } => app.open_in_place(depth, id, how),
        // A slow second click on the name.
        Intent::BeginRename { id } => rename::begin_on(app, depth, id, tx),
        // A press with any button outside the field commits an open rename in the GTK view.
        // The grid opens no context menu yet.
        Intent::ContextMenu { .. } => rename::commit(app, tx),
        // Left on the first tile: the sidebar takes the keyboard, when there is one.
        Intent::Edge(Direction::Left) => {
            shell::leave_view_at_start(app, tx);
        }
        // Drags, new tabs and the pane header an edge moves to are other packages' parts.
        _ => {}
    }
    shell::settle(app, tx);
}

/// The grid moved its cursor or changed its selection: the browser takes it now, so the
/// frame drawn for the input shows the browser in step.
pub(super) fn moved(app: &mut App, tx: &mut Tx) {
    shell::settle(app, tx);
}

/// Opens the entry of a row as a click on it does: what the context menu's Open asks
/// (`open_context_entry` in `src/ui/browser/context_menu/item_menu.rs`).
pub(super) fn clicked_open(app: &mut App, (depth, id): At) {
    app.open_in_place(depth, id, Activation::Click);
}
