// SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

use rune::Tx;
use strata_core::model::{FileEntry, Location};
use strata_desktop::keys::{
    Modifiers,
    bindings::{ESCAPE, ESCAPE_ANY, NEW_FOLDER, NEW_FOLDER_WITH_SELECTION},
};
use strata_fs::{app::BrowserEvent, services::Naming};

use crate::{
    App,
    commands::{Action, Call, Keys, On, Outcome, Stage},
    filter, menus, rename, shell,
    slots::{Frame, Listener, Part, Place, Setup},
    transfer,
};

/// How long a created item has to be listed and take the editor (`CreatedEntryWatch`).
const PATIENCE: Duration = Duration::from_secs(5);

/// How soon a poll looks again while an item waits for its row.
const AGAIN: Duration = Duration::from_millis(16);

/// What a new item is created as and what moves into it (`NewEntry` in `new_entry.rs`).
struct NewEntry {
    parent: Location,
    directory: bool,
    move_sources: Vec<Location>,
}

/// A new item asked for and not yet named (`PendingEntryRename`).
struct Pending {
    depth: usize,
    parent: Location,
    naming: Naming,
    move_sources: Vec<Location>,
    group_sources: Vec<Location>,
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
                        Heard::Ended => {
                            cancel(app);
                        }
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
        Action::new("group", group)
            .command(NEW_FOLDER_WITH_SELECTION)
            .keys(&[Keys::at(
                Stage::FileCommands,
                &[NEW_FOLDER_WITH_SELECTION.ignoring(Modifiers::empty())],
            )
            .when(|_, at| !at.within("preview"))]),
        // `inline_editing`: Escape gives up a naming that waits (`cancel_new_entry`).
        Action::new("new-item-cancel", cancelled)
            .command(ESCAPE)
            .keys(&[Keys::at(Stage::InlineEditing, &[ESCAPE_ANY])
                .when(|app, _| app.new_item.pending.is_some())]),
    ];
}

fn new_folder(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    begin(app, &call.on, true, tx)
}

fn new_file(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    begin(app, &call.on, false, tx)
}

fn group(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    match call.on {
        On::Row((depth, id)) => {
            if filter::targets::at(app, depth, id).is_none()
                && let Some((_, entries)) = menus::context_entries(app, depth, id)
            {
                begin_group(app, depth, &entries, tx);
            }
            Outcome::Ran
        }
        On::Focus => {
            if !create_for_selection(app, tx) {
                begin(app, &On::Focus, true, tx);
            }
            Outcome::Ran
        }
        On::Column(_) | On::Place(_) => Outcome::Ran,
    }
}

fn cancelled(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    if cancel(app) {
        Outcome::Ran
    } else {
        Outcome::Declined
    }
}

pub(crate) fn cancel(app: &mut App) -> bool {
    let Some(pending) = app.new_item.pending.take() else {
        return false;
    };
    if let Some((created, _)) = &pending.created {
        app.browser.clear_group_folder_for(created);
    }
    true
}

fn column(app: &App, on: &On) -> Option<usize> {
    match *on {
        On::Row((depth, _)) | On::Column(depth) => Some(depth),
        On::Focus => app.browser.active_depth().or_else(|| last_column(app)),
        On::Place(_) => None,
    }
}

fn last_column(app: &App) -> Option<usize> {
    (0..)
        .take_while(|depth| app.browser.location_at(*depth).is_some())
        .last()
}

/// `may_create_in`: neither the Trash nor Recent takes a new item.
pub(crate) fn may_create_in(location: &Location) -> bool {
    let trash = location
        .uri_value()
        .is_some_and(|uri| uri.starts_with("trash:"));
    !trash && !location.is_recent_location()
}

pub(crate) fn focused_create_parent(app: &App) -> Option<(usize, Location)> {
    let depth = crate::tenxer::focused_listing_depth(app).or_else(|| column(app, &On::Focus))?;
    let parent = app.browser.location_at(depth).filter(may_create_in)?;
    Some((depth, parent))
}

fn begin(app: &mut App, on: &On, directory: bool, tx: &mut Tx) -> Outcome {
    let Some((depth, parent)) =
        column(app, on).and_then(|depth| Some((depth, app.browser.location_at(depth)?)))
    else {
        return Outcome::Ran;
    };
    start_pending(
        app,
        depth,
        NewEntry {
            parent,
            directory,
            move_sources: Vec::new(),
        },
        tx,
    );
    Outcome::Ran
}

fn create_for_selection(app: &mut App, tx: &mut Tx) -> bool {
    if filter::targets::selected(app).is_some() {
        return false;
    }
    let Some(depth) = column(app, &On::Focus) else {
        return false;
    };
    shell::publish(app);
    let entries = app.browser.filled_entries(depth);
    !entries.is_empty() && begin_group(app, depth, &entries, tx)
}

fn begin_group(app: &mut App, depth: usize, entries: &[FileEntry], tx: &mut Tx) -> bool {
    let Some(parent) = app.browser.location_at(depth).filter(may_create_in) else {
        return false;
    };
    let search_shown = filter::targets::selected_with_depth(app).is_some_and(|(_, d)| d == depth);
    if search_shown || entries.is_empty() {
        return false;
    }
    let move_sources = entries.iter().map(|entry| entry.location.clone()).collect();
    start_pending(
        app,
        depth,
        NewEntry {
            parent,
            directory: true,
            move_sources,
        },
        tx,
    );
    true
}

fn start_pending(app: &mut App, depth: usize, entry: NewEntry, tx: &mut Tx) {
    let NewEntry {
        parent,
        directory,
        move_sources,
    } = entry;
    if !may_create_in(&parent) {
        return;
    }
    cancel(app);
    rename::cancel(app, tx);
    filter::clear_for_new_item(app, depth, tx);
    let naming = if move_sources.is_empty() {
        Naming::NewItem
    } else {
        Naming::Single
    };
    app.new_item.pending = Some(Pending {
        depth,
        parent: parent.clone(),
        naming,
        move_sources,
        group_sources: Vec::new(),
        created: None,
        selected: false,
    });
    if directory {
        app.browser.create_new_folder(parent);
    } else {
        app.browser.create_new_file(parent);
    }
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

/// `move_selection_into_created`: moves grouped entries into the newly listed folder.
fn move_into_created(app: &mut App, depth: usize, location: &Location) {
    let is_dir = listed_at(app, depth, location)
        .and_then(|pos| app.browser.entry_at(depth, pos))
        .is_some_and(|entry| entry.is_directory());
    let Some(pending) = app.new_item.pending.as_mut() else {
        return;
    };
    let sources = if is_dir {
        std::mem::take(&mut pending.move_sources)
    } else {
        pending.move_sources.clear();
        Vec::new()
    };
    if sources.is_empty() {
        return;
    }
    pending.group_sources = sources.clone();
    app.browser.expect_group_folder(location.clone());
    transfer::move_into_group_folder(app, location.clone(), sources);
}

fn group_sources_left(app: &App, depth: usize, group_sources: &[Location]) -> bool {
    let Some(snapshot) = app
        .browser
        .column_snapshot(depth)
        .filter(|snapshot| !snapshot.loading)
    else {
        return false;
    };
    group_sources.iter().all(|source| {
        app.browser
            .with_entries(depth, 0..snapshot.count, |entries| {
                !entries.iter().any(|entry| entry.location == *source)
            })
            .unwrap_or(true)
    })
}

fn ready_to_name(app: &App, depth: usize, location: &Location) -> bool {
    let Some(pending) = &app.new_item.pending else {
        return false;
    };
    pending.group_sources.is_empty()
        || (!transfer::is_preparing_to(app, location)
            && !app.browser.has_foreground_operation()
            && group_sources_left(app, depth, &pending.group_sources))
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
        move_into_created(app, depth, &location);
        if let Some(pending) = &mut app.new_item.pending {
            pending.selected = true;
        }
        return true;
    }
    if !ready_to_name(app, depth, &location) {
        return false;
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
