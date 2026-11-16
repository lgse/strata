// SPDX-License-Identifier: MIT

//! What the Columns view asks of its host, taken to `Browser`.

use std::time::Duration;

use rune::{Mods, PointerEvent, Tx};
use strata_components::{
    collection::{
        element::{Activation, DropOver, Intent},
        keyboard::NothingToSelect,
    },
    columns::ColumnsIntent,
    contract::{
        EntryId, Point,
        dnd::{DropAction, DropHover, DropOffer, DropZone, Dropped, URI_LIST},
    },
};
use strata_core::model::{FileEntry, Location};

use super::Input;
use crate::{
    App,
    commands::{self, On},
    filter,
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

    fn previewable_click_position(&self, depth: usize, id: EntryId) -> Option<usize> {
        let preferences = shell::preferences(self);
        if crate::clicks::one_click_files(crate::clicks::columns(preferences))
            || !preferences.single_click_previews
        {
            return None;
        }
        let position = self.columns.position(depth, id)?;
        self.browser
            .entry_at(depth, position)
            .as_ref()
            .is_some_and(crate::preview::entry_supports_quick_preview)
            .then_some(position)
    }

    /// A plain click on a file previews it when single-click previews are enabled and two
    /// clicks open files (`PendingActivationKind::Standard { preview: true }`), or else closes
    /// the columns after it (`PendingActivationKind::CloseDeeper` in
    /// `src/ui/browser/columns/rows/click.rs`).
    fn clicked(&self, depth: usize, id: EntryId) {
        let file = self
            .columns
            .row(depth, id)
            .is_some_and(|row| !row.is_folder());
        if !file {
            return;
        }
        if let Some(position) = self.previewable_click_position(depth, id) {
            self.browser.preview(depth, position);
            return;
        }
        if self.browser.location_at(depth + 1).is_some() {
            self.browser.close_column(depth + 1);
        }
    }

    fn entry_at(&self, (depth, id): At) -> Option<FileEntry> {
        if self.columns.shows_hits(depth) {
            filter::targets::at(self, depth, id)
        } else {
            let position = self.columns.position(depth, id)?;
            self.browser.entry_at(depth, position)
        }
    }

    /// A middle click on a folder row (`open_middle_clicked` in
    /// `src/ui/browser/columns/rows/click.rs`): `open-in-tab`, or `open-in-window` with Ctrl or
    /// Shift.
    fn open_in_new(&mut self, at: At, window: bool, tx: &mut Tx) {
        let Some(entry) = self.entry_at(at) else {
            return;
        };
        let command = if window {
            "open-in-window"
        } else {
            "open-in-tab"
        };
        commands::run(self, command, On::Place(entry.location), tx);
    }

    /// A middle click on a file row (`MiddleClickAction::RevealParent` in
    /// `src/ui/browser/columns/rows/click.rs`): empties its folder's filter and reveals the file
    /// in its folder, closing deeper columns or navigating to a subfolder hit's parent.
    fn reveal(&mut self, at: At, tx: &mut Tx) {
        let Some(entry) = self.entry_at(at) else {
            return;
        };
        let Some(folder) = entry.location.parent() else {
            return;
        };
        if let Some(depth) = self.browser.open_depth(&folder) {
            filter::clear_for_reveal(self, depth, tx);
        }
        shell::publish(self);
        self.browser.reveal_locations(folder, vec![entry.location]);
        self.columns
            .follow_active(self.browser.active_depth(), &self.look, tx);
        tx.focus(super::VIEW, false);
    }

    fn collection(&mut self, depth: usize, intent: Intent, tx: &mut Tx) {
        if let Intent::BeginDrag { ids, .. } = intent {
            let entries: Vec<_> = ids
                .into_iter()
                .filter_map(|id| self.entry_at((depth, id)))
                .collect();
            crate::dnd::begin_entries(self, entries, self.columns.scale.get(), tx);
            return;
        }
        // A filter's results open as GTK's hits do, and are renamed nowhere yet.
        if self.columns.shows_hits(depth) && filter::hit_intent(self, depth, &intent, tx) {
            return;
        }
        match intent {
            Intent::Activate { id, how } => self.activate(depth, id, how),
            Intent::Clicked { id } => self.clicked(depth, id),
            Intent::OpenInNew { id, window } => self.open_in_new((depth, id), window, tx),
            Intent::Reveal { id } => self.reveal((depth, id), tx),
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
            Intent::Other(other) if other.downcast_ref::<NothingToSelect>().is_some() => {
                crate::tenxer::flash_nothing_to_select(self, tx);
            }
            // Drags are interface the slice leaves out. Up and Down at a
            // column's edge have nowhere to go without a header to focus: the view has
            // selected the cursor's entry alone again, as the browser does there.
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
    app.columns
        .refresh_mirror_preferences(shell::preferences(app));
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
        ColumnsIntent::Dropped {
            depth,
            target,
            dropped,
        } => {
            if let Some(dest) = drop_destination(app, depth, target) {
                crate::dnd::commit_file_drop(app, dest, &dropped, tx);
            }
        }
        _ => {}
    }
    shell::settle(app, tx);
}

fn drop_destination(app: &App, depth: usize, target: Option<EntryId>) -> Option<Location> {
    match target {
        Some(_) if app.columns.shows_hits(depth) => None,
        Some(id) => app
            .entry_at((depth, id))
            .filter(FileEntry::is_directory)
            .map(|entry| entry.location),
        None => app.browser.location_at(depth),
    }
}

pub(super) fn drop_action(
    app: &App,
    target: Option<(usize, Option<EntryId>)>,
    offer: &DropOffer,
    mods: Mods,
) -> Option<DropAction> {
    let (depth, row) = target?;
    let dest = drop_destination(app, depth, row)?;
    crate::dnd::drop_action(app, &dest, offer, mods)
}

pub(super) fn strip_action(
    app: &App,
    offer: &DropOffer,
    mods: Mods,
    point: PointerEvent,
) -> Option<DropAction> {
    let window = Point::new(point.window_x, point.window_y);
    let target = super::view(app).drop_target_at(app, window);
    drop_action(app, target, offer, mods)
}

pub(super) fn strip_zone() -> DropZone<App> {
    DropZone::new(strip_dropped)
        .on_hover(strip_hover)
        .on_move(strip_move)
        .on_reject(|app: &mut App, _, _| app.columns.clear_drag_over())
}

fn strip_dropped(app: &mut App, dropped: Dropped, tx: &mut Tx) {
    app.columns.clear_drag_over();
    let Some(point) = dropped.point else {
        return;
    };
    let window = Point::new(point.window_x, point.window_y);
    let Some((depth, target)) = super::view(app).drop_target_at(app, window) else {
        return;
    };
    if let Some(dest) = drop_destination(app, depth, target) {
        crate::dnd::commit_file_drop(app, dest, &dropped, tx);
    }
}

fn strip_hover(app: &mut App, hover: DropHover, offer: &DropOffer, tx: &mut Tx) {
    app.columns.strip_refused =
        (hover == DropHover::Rejecting).then(|| offer.offers(URI_LIST));
    if hover == DropHover::Left {
        strip_over(app, None, tx);
    }
}

fn strip_move(app: &mut App, point: PointerEvent, tx: &mut Tx) {
    if app.columns.strip_refused != Some(false) {
        strip_over(app, Some(Point::new(point.window_x, point.window_y)), tx);
    } else {
        tx.live_only();
    }
}

fn strip_over(app: &mut App, at: Option<Point>, tx: &mut Tx) {
    let target = at
        .filter(|_| app.columns.strip_refused.is_none())
        .and_then(|window| super::view(app).drop_target_at(app, window));
    let mut wake = app.columns.state.drag_over(at);
    for depth in 0..app.columns.state.len() {
        let drop = match target {
            Some((d, Some(id))) if d == depth => Some(DropOver::Row(id)),
            Some((d, None)) if d == depth => Some(DropOver::Background),
            _ => None,
        };
        if let Some(col) = app.columns.state.column_mut(depth)
            && col.collection.drop_over() != drop
        {
            col.collection.set_drop_over(drop);
            wake = true;
        }
    }
    if wake {
        tx.wake_after(Duration::ZERO);
    } else {
        tx.live_only();
    }
}

pub(crate) fn spring_open(app: &mut App, depth: usize, id: EntryId, tx: &mut Tx) {
    if app.columns.shows_hits(depth) {
        return;
    }
    let Some(entry) = app.entry_at((depth, id)).filter(FileEntry::is_directory) else {
        return;
    };
    shell::publish(app);
    app.browser.descend(depth, entry.location);
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
