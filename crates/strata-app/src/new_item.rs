// SPDX-License-Identifier: MIT

//! New Folder and New File: the browser creates the item under a free name, and the rename
//! editor opens on it to name it (`src/ui/browser/inline_edit/new_entry.rs`).
//!
//! - **Asking.** Ctrl+Shift+N at the dispatcher's `file_commands` stage, in the column the
//!   keyboard is in (`create_new_folder` in `src/ui/browser/view_edit.rs`), and the menus'
//!   New Folder and New File rows, in the column the menu was opened on. Nothing is created
//!   in the Trash or in Recent.
//! - **Waiting.** The browser says where it created the item (`BrowserEvent::EntryCreated`).
//!   Once the item is listed it is selected, and on a later poll the editor opens on it with
//!   `Naming::NewItem` ([`crate::rename::begin_new_item`]), as GTK's `created_entry_tick`
//!   does a frame apart. The wait ends after 5 s, or when its folder leaves its column, a
//!   new listing replaces the columns or an operation fails.
//! - **Escape** before the editor opens gives up the naming, and the item keeps its free
//!   name (`inline_editing` in `src/ui/window/keyboard/commands.rs`).
//!
//! Left out, with what brings each back:
//!
//! - **New Folder with Selection** (Ctrl+Alt+N): the folder takes the selection in by a move,
//!   which is a transfer, and `new-folder-with-selection-groups/columns` waits on finding
//!   lgse/strata#2162 and an owner ruling. The key is not bound yet, so GTK's fallback to a
//!   plain New Folder when nothing is selected (`file_commands`) is missing too.
//! - **Clearing the column's filter** before the item is made (`clear_filter`): the filter
//!   bar is A10's.
//! - **F2 on the created item itself while it waits** (`begin_rename`): GTK takes the naming
//!   over and opens the editor with `Naming::NewItem`; here the naming is given up and a plain
//!   rename opens. F2 or the menu's Rename on any other entry gives the naming up, as on GTK.
//! - **A pointer press or a scroll anywhere in the window gives up the naming on GTK**
//!   (`install_inline_edit_dismissal` in `src/ui/browser/inline_edit.rs`). Parts have no
//!   pointer hook, so here a click elsewhere during the wait is overridden by the next poll,
//!   which opens the editor; a hook is a `contract` change.
//! - **Scrolling the item into view** before the editor opens: the editor shows its row
//!   when it begins.
//!
//! The column of Ctrl+Shift+N is the browser's active one. GTK takes the focused column
//! first, so a resting pointer cannot redirect it (`new_folder_destination_depth`); here the
//! active column is the one the view's keyboard and selection are in, never the pointer's,
//! so the two are the same.

use std::time::{Duration, Instant};

use rune::Tx;
use strata_core::model::Location;
use strata_desktop::keys::{
    Modifiers,
    bindings::{ESCAPE, ESCAPE_ANY, NEW_FOLDER},
};
use strata_fs::{app::BrowserEvent, services::Naming};

use crate::{
    App,
    commands::{Action, Call, Keys, On, Outcome, Stage},
    rename, shell,
    slots::{Frame, Listener, Part, Place, Setup},
};

/// How long a created item has to be listed and take the editor (`CreatedEntryWatch`).
const PATIENCE: Duration = Duration::from_secs(5);

/// How soon a poll looks again while an item waits for its row.
const AGAIN: Duration = Duration::from_millis(16);

/// A new item asked for and not yet named (`PendingEntryRename`).
struct Pending {
    depth: usize,
    parent: Location,
    naming: Naming,
    /// Where the browser created it, and when the wait for its row ends.
    created: Option<(Location, Instant)>,
    selected: bool,
}

pub(crate) struct NewItem {
    pending: Option<Pending>,
}

/// What the browser's events mean for a naming that waits.
enum Heard {
    Created(Location),
    /// A new listing, columns cut back or a failed operation: the wait is over.
    Ended,
}

impl Heard {
    fn of(event: &BrowserEvent) -> Option<Self> {
        match event {
            BrowserEvent::EntryCreated { location } => Some(Self::Created(location.clone())),
            BrowserEvent::Reset
            | BrowserEvent::ColumnsTruncated { .. }
            | BrowserEvent::OperationFailed { .. } => Some(Self::Ended),
            _ => None,
        }
    }
}

/// Nothing of its own is drawn: the item's row and the rename editor show it.
fn nothing(_: &App, _: &mut Frame<'_, App>) -> Option<rune::El> {
    None
}

impl Part for NewItem {
    const PLACE: Place<App> = Place::Overlays(nothing);

    fn new(_: &Setup<'_>) -> Self {
        Self { pending: None }
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, _, heard| {
            if let Some(what) = Heard::of(event) {
                heard.then(move |app: &mut App, _: &mut Tx| {
                    match what {
                        Heard::Created(location) => created(app, location),
                        Heard::Ended => app.new_item.pending = None,
                    }
                    false
                });
            }
        }))
    }

    fn poll(app: &mut App, _: f64, tx: &mut Tx) -> bool {
        tick(app, tx)
    }

    const COMMANDS: &'static [Action<App>] = &[
        // `file_commands`: read wherever the press lands.
        Action::new("new-folder", new_folder)
            .command(NEW_FOLDER)
            .keys(&[Keys::at(
                Stage::FileCommands,
                &[NEW_FOLDER.ignoring(Modifiers::empty())],
            )]),
        Action::new("new-file", new_file),
        // `inline_editing`: Escape gives up a naming that waits (`cancel_new_entry`).
        Action::new("new-item-cancel", cancelled)
            .command(ESCAPE)
            .keys(&[Keys::at(Stage::InlineEditing, &[ESCAPE_ANY])
                .when(|app, _| app.new_item.pending.is_some())]),
    ];
}

fn new_folder(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    begin(app, call.on, true, tx)
}

fn new_file(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    begin(app, call.on, false, tx)
}

fn cancelled(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    if cancel(app) {
        Outcome::Ran
    } else {
        Outcome::Declined
    }
}

/// Gives up a naming that waits (`cancel_new_entry`), as a rename that begins elsewhere
/// does. False when none did.
pub(crate) fn cancel(app: &mut App) -> bool {
    let Some(pending) = app.new_item.pending.take() else {
        return false;
    };
    if let Some((created, _)) = &pending.created {
        app.browser.clear_group_folder_for(created);
    }
    true
}

/// The column a new item goes in: the menu's, or the browser's active one, or the last
/// (`new_entry_parent` and `new_folder_destination_depth` in `src/ui/browser.rs`).
fn column(app: &App, on: On) -> Option<usize> {
    match on {
        On::Row((depth, _)) | On::Column(depth) => Some(depth),
        On::Focus => app.browser.active_depth().or_else(|| last_column(app)),
    }
}

fn last_column(app: &App) -> Option<usize> {
    (0..)
        .take_while(|depth| app.browser.location_at(*depth).is_some())
        .last()
}

/// `may_create_in`: neither the Trash nor Recent takes a new item.
fn may_create_in(location: &Location) -> bool {
    let trash = location
        .uri_value()
        .is_some_and(|uri| uri.starts_with("trash:"));
    !trash && !location.is_recent_location()
}

/// `begin_new_entry`: an edit or a naming already open gives way, and the browser creates
/// the item. The key is taken wherever the folder is, as GTK takes it.
fn begin(app: &mut App, on: On, directory: bool, tx: &mut Tx) -> Outcome {
    let Some((depth, parent)) =
        column(app, on).and_then(|depth| Some((depth, app.browser.location_at(depth)?)))
    else {
        return Outcome::Ran;
    };
    if !may_create_in(&parent) {
        return Outcome::Ran;
    }
    cancel(app);
    rename::cancel(app, tx);
    app.new_item.pending = Some(Pending {
        depth,
        parent: parent.clone(),
        naming: Naming::NewItem,
        created: None,
        selected: false,
    });
    if directory {
        app.browser.create_new_folder(parent);
    } else {
        app.browser.create_new_file(parent);
    }
    Outcome::Ran
}

/// `rename_created_entry`: the browser made the item, in the folder a naming waits in.
fn created(app: &mut App, location: Location) {
    let Some(pending) = app.new_item.pending.as_mut() else {
        return;
    };
    if location.parent().as_ref() == Some(&pending.parent) {
        pending.created = Some((location, Instant::now() + PATIENCE));
    }
}

/// `created_entry_expired`: the time is up, or the folder left its column.
fn expired(app: &App, pending: &Pending, until: Instant) -> bool {
    Instant::now() >= until
        || app.browser.location_at(pending.depth).as_ref() != Some(&pending.parent)
}

/// Where the created item is listed in its column, once the column is read.
fn listed_at(app: &App, depth: usize, location: &Location) -> Option<usize> {
    let snapshot = app
        .browser
        .column_snapshot(depth)
        .filter(|snapshot| !snapshot.loading)?;
    app.browser
        .with_entries(depth, 0..snapshot.count, |entries| {
            entries.iter().position(|entry| entry.location == *location)
        })
        .flatten()
}

/// Whether the shown view is Columns, where the selected item's folder also opens beside it.
fn in_columns(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)("columns"))
}

/// `select_created_entry`. In Columns the browser shows the item as a click would, which can
/// cut the columns back: the naming's wait outlives what that tells it.
fn select(app: &mut App, depth: usize, position: usize, tx: &mut Tx) {
    if !in_columns(app) {
        app.browser.select(depth, position);
        return;
    }
    let pending = app.new_item.pending.take();
    app.browser.reveal_created_entry(depth, position);
    shell::settle(app, tx);
    app.new_item.pending = pending;
}

/// One poll of a naming that waits (`created_entry_tick`). True when the view has to run.
fn tick(app: &mut App, tx: &mut Tx) -> bool {
    let Some(pending) = &app.new_item.pending else {
        return false;
    };
    let Some((location, until)) = pending.created.clone() else {
        return false;
    };
    if expired(app, pending, until) {
        cancel(app);
        return false;
    }
    tx.wake_after(AGAIN);
    let (depth, naming, selected) = (pending.depth, pending.naming, pending.selected);
    let Some(position) = listed_at(app, depth, &location) else {
        return false;
    };
    if !selected {
        select(app, depth, position, tx);
        if let Some(pending) = &mut app.new_item.pending {
            pending.selected = true;
        }
        return true;
    }
    let Some(entry) = app.browser.entry_at(depth, position) else {
        return false;
    };
    if rename::begin_new_item(app, depth, entry, naming, tx) {
        app.new_item.pending = None;
    }
    true
}

#[cfg(test)]
mod tests;
