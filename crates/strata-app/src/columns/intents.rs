// SPDX-License-Identifier: MIT

//! What the Columns view asks of its host, taken to `Browser`.

use rune::Tx;
use strata_components::{
    collection::element::{Activation, Intent},
    columns::ColumnsIntent,
    context_menu::{MenuIntent, MenuTarget},
    contract::{EntryId, Point, Rect},
};

use super::Input;
use crate::{
    menus::{self, EntryTarget, FolderTarget, Wired},
    shell::{self, App},
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

    /// The menu of entry `id` in column `depth`, or of the selection it is part of
    /// (`context_entries` in `src/ui/browser/context_menu.rs`).
    fn entry_menu(&mut self, depth: usize, id: EntryId, at: Point) {
        let position = self.columns.position(depth, id);
        let Some(entry) = position.and_then(|position| self.browser.entry_at(depth, position))
        else {
            return;
        };
        let selected = self.browser.selected_entries();
        let within = selected
            .iter()
            .any(|other| other.location == entry.location);
        let entries = if within { selected } else { vec![entry.clone()] };
        let model = menus::entry(&EntryTarget {
            entry: &entry,
            entries: &entries,
            can_trash: self.browser.can_trash_at(depth),
            can_delete: self.browser.can_delete_at(depth),
            typing: self.columns.typing(),
        });
        self.menu.show(depth, entry.display_name, MenuTarget::Entry(id), at, model);
    }

    /// The menu of column `depth`'s folder: a press on its empty space or its header.
    fn folder_menu(&mut self, depth: usize, at: Point) {
        let Some(title) = self.columns.title(depth).map(str::to_owned) else {
            return;
        };
        let model = menus::folder(&FolderTarget {
            has_entries: self.columns.rows_shown(depth) > 0,
            hidden_shown: self.browser.preferences().show_hidden,
            typing: self.columns.typing(),
        });
        self.menu.show(depth, title, MenuTarget::Background, at, model);
    }

    fn context_menu(&mut self, depth: usize, target: Option<EntryId>, anchor: Rect) {
        let at = Point::new(anchor.x, anchor.y);
        match target {
            Some(id) => self.entry_menu(depth, id, at),
            None => self.folder_menu(depth, at),
        }
    }

    fn collection(&mut self, depth: usize, intent: Intent) {
        match intent {
            Intent::Activate { id, how } => self.activate(depth, id, how),
            Intent::Clicked { id } => self.clicked(depth, id),
            // V9 opens the editor here.
            Intent::BeginRename { .. } => tracing::debug!(depth, "the view asked for a rename"),
            Intent::ContextMenu { target, anchor } => self.context_menu(depth, target, anchor),
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
        ColumnsIntent::Collection { depth, intent } => app.collection(depth, intent),
        ColumnsIntent::EnterFolder { depth, id } => app.enter(depth, id),
        ColumnsIntent::CloseFrom(depth) => app.browser.close_column(depth),
        // Published above, and with it the active column a key moved the keyboard to.
        ColumnsIntent::Cursor { .. }
        | ColumnsIntent::Selection { .. }
        | ColumnsIntent::Active { .. } => {}
        ColumnsIntent::HeaderMenu { depth, anchor } => {
            app.folder_menu(depth, Point::new(anchor.x, anchor.y));
        }
        // `LeftOfRoot` is the sidebar's, which the slice has not. `Width` and `Dropped` are
        // saved widths and drag and drop: out of the slice.
        _ => {}
    }
    shell::settle(app, tx);
}

/// A row of the open menu was chosen. The menu has closed, and the keyboard is back in the
/// view.
pub(crate) fn chosen(app: &mut App, intent: MenuIntent, tx: &mut Tx) {
    let MenuIntent::Activate { target, action } = intent;
    let depth = app.menu.depth();
    match menus::wired(&action) {
        // `open_context_entry` in `src/ui/browser/context_menu/item_menu.rs`.
        Some(Wired::Open) => {
            if let Some(id) = target.entry() {
                app.activate(depth, id, Activation::Click);
            }
        }
        // V9 opens the editor here.
        Some(Wired::Rename) => tracing::debug!(depth, "the menu asked for a rename"),
        Some(Wired::SelectAll) => app.columns.select_all(depth),
        Some(Wired::Refresh) => app.browser.retry_column(depth),
        Some(Wired::ToggleHidden) => app.browser.toggle_hidden(),
        None => tracing::debug!(%action, "a menu row for a part this interface has not"),
    }
    shell::settle(app, tx);
}
