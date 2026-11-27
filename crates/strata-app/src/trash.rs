// SPDX-License-Identifier: MIT

use rune::Tx;
use strata_core::model::{FileEntry, Location};
use strata_desktop::keys::{
    Map, Modifiers,
    bindings::{DELETE_PERMANENTLY, LEFT, MOVE_TO_TRASH, RIGHT, TENXER_DELETE_CONFIRM},
};
use strata_fs::app::BrowserEvent;

use crate::{
    App,
    commands::{Action, Call, Holds, Keys, On, Outcome, Stage},
    filter, shell,
    slots::{At, Later, Listener, Part, Place, Setup},
};

pub(crate) mod delete_error;
mod dialog;
pub(crate) mod empty;
mod sizes;

use dialog::{Confirm, Kind};
use sizes::{Reach, deletion_reach, permanent_delete_subtitle, size_text};

/// What `delete_kind` lets be held.
const DELETE_MODIFIERS: Modifiers = Modifiers::CONTROL_MASK
    .union(Modifiers::ALT_MASK)
    .union(Modifiers::SUPER_MASK);

const DIALOG_ARROW_MODIFIERS: Modifiers = Modifiers::SHIFT_MASK.union(Modifiers::SUPER_MASK);

enum Queued {
    Trash {
        entries: Vec<FileEntry>,
    },
    Delete {
        entries: Vec<FileEntry>,
        remote: bool,
    },
    Cancel {
        cursor: Option<(usize, Location)>,
        entries: Vec<FileEntry>,
    },
}

pub(crate) struct Trash {
    later: Later<App>,
    /// The confirmation, kept after it closes for its closing motion.
    confirm: Option<Confirm>,
    open: bool,
    queued: Vec<Queued>,
    waiting: Option<(Vec<FileEntry>, Kind)>,
    sizing: Option<glib::JoinHandle<()>>,
    /// Counts the confirmations, so a size task's answer finds its own.
    opened: u64,
    trashing: Vec<FileEntry>,
    /// Empty Trash's dialog ([`empty`]).
    empty: empty::Empty,
}

impl Part for Trash {
    const PLACE: Place<App> = Place::Dialogs(dialog::build);

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            later: setup.later.clone(),
            confirm: None,
            open: false,
            queued: Vec::new(),
            waiting: None,
            sizing: None,
            opened: 0,
            trashing: Vec::new(),
            empty: empty::Empty::default(),
        }
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, _, heard| {
            if let BrowserEvent::OperationCompletedWithErrors {
                message,
                retryable_locations,
                has_non_retryable_failures,
                blocker,
                report,
            } = event
            {
                let failed = delete_error::Failed {
                    message: crate::transfer::outcome_detail(message.clone(), report),
                    retryable: retryable_locations.clone(),
                    others_failed: *has_non_retryable_failures,
                    blocker: blocker.clone(),
                };
                heard.then(move |app: &mut App, _: &mut Tx| {
                    delete_error::completed_with_errors(app, failed)
                });
            }
        }))
    }

    fn poll(app: &mut App, _: f64, tx: &mut Tx) -> bool {
        let emptying = empty::poll(app);
        let queued = std::mem::take(&mut app.trash.queued);
        let mut waiting = app.trash.waiting.take_if(|_| !app.trash.open);
        if queued.is_empty() && waiting.is_none() {
            return emptying;
        }
        for queued in queued {
            apply_queued(app, queued, &mut waiting);
        }
        if let Some((entries, kind)) = waiting.filter(|(entries, _)| !entries.is_empty()) {
            show(app, entries, kind);
        }
        shell::settle(app, tx);
        true
    }

    fn keyboard(app: &App) -> Option<Holds> {
        (app.trash.open || empty::holds(app)).then_some(Holds::Modal { field: false })
    }

    const COMMANDS: &'static [Action<App>] = &[
        Action::new("trash", trashed)
            .command(MOVE_TO_TRASH)
            .keys(&[Keys::at(
                Stage::Dismissal,
                &[MOVE_TO_TRASH
                    .in_map(Map::Default)
                    .ignoring(DELETE_MODIFIERS)],
            )
            .when(|_, at| !at.text() && !at.within("preview"))]),
        Action::new("delete", deleted)
            .command(DELETE_PERMANENTLY)
            .keys(&[Keys::at(
                Stage::Dismissal,
                &[DELETE_PERMANENTLY
                    .in_map(Map::Default)
                    .ignoring(DELETE_MODIFIERS)],
            )
            .when(|_, at| !at.text() && !at.within("preview"))]),
        // No key empties the Trash: the sidebar's Trash menu asks for it.
        Action::new("empty-trash", empty::run),
        Action::new("trash-tenxer-confirm", tenxer_confirm)
            .command(TENXER_DELETE_CONFIRM)
            .keys(&[Keys::at(
                Stage::Entry,
                &[TENXER_DELETE_CONFIRM.ignoring(Modifiers::empty())],
            )
            .when(|app, _| shell::preferences(app).tenxer_mode && confirms_trash_dialog(app))]),
        Action::new("trash-dialog-focus-cancel", |app, _, tx| {
            focus_button(app, "secondary", tx)
        })
        .command(LEFT)
        .keys(&[Keys::at(
            Stage::Entry,
            &[LEFT.ignoring(DIALOG_ARROW_MODIFIERS).alternates()],
        )
        .when(|app, _| app.trash.open)]),
        Action::new("trash-dialog-focus-confirm", |app, _, tx| {
            focus_button(app, "primary", tx)
        })
        .command(RIGHT)
        .keys(&[Keys::at(
            Stage::Entry,
            &[RIGHT.ignoring(DIALOG_ARROW_MODIFIERS).alternates()],
        )
        .when(|app, _| app.trash.open)]),
    ];
}

fn apply_queued(app: &mut App, queued: Queued, waiting: &mut Option<(Vec<FileEntry>, Kind)>) {
    match queued {
        Queued::Trash { entries } => {
            if let Some((asked, _)) = waiting {
                not_again(asked, &entries);
            }
            move_to_trash(app, entries);
        }
        Queued::Delete { entries, remote } => {
            if let Some((asked, _)) = waiting {
                not_again(asked, &entries);
            }
            delete_confirmed(app, entries, remote);
        }
        Queued::Cancel { cursor, entries } => return_cursor(app, cursor, &entries),
    }
}

fn confirms_trash_dialog(app: &App) -> bool {
    app.trash.open
        && app
            .trash
            .confirm
            .as_ref()
            .is_some_and(|confirm| confirm.kind == Kind::Trash)
}

fn tenxer_confirm(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    confirmed(app);
    Outcome::Ran
}

fn focus_button(app: &mut App, suffix: &str, tx: &mut Tx) -> Outcome {
    let Some(confirm) = app.trash.confirm.as_ref() else {
        return Outcome::Declined;
    };
    tx.focus(
        format!("delete-confirm-{}-{suffix}", confirm.generation),
        true,
    );
    Outcome::Ran
}

fn is_trash(location: &Location) -> bool {
    location
        .uri_value()
        .is_some_and(|uri| uri.starts_with("trash:"))
}

fn can_remove(location: &Location) -> bool {
    let trash_item = || {
        location
            .parent()
            .is_some_and(|parent| parent.uri_value() == Some("trash:///"))
    };
    !is_trash(location) || trash_item()
}

fn in_columns(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)("columns"))
}

fn focused_entries(app: &App) -> (Vec<FileEntry>, bool) {
    let browser = &app.browser;
    let depth = browser.active_depth();
    let folder_is_trash = |depth: Option<usize>| {
        let folder = depth.and_then(|depth| browser.location_at(depth));
        folder.as_ref().is_some_and(is_trash)
    };
    if let Some((results, at)) = filter::targets::selected_with_depth(app) {
        return (results, folder_is_trash(Some(at)));
    }
    let column = depth
        .filter(|_| in_columns(app))
        .map(|depth| browser.command_entries(depth))
        .filter(|entries| !entries.is_empty());
    let entries = column.unwrap_or_else(|| browser.deletion_entries());
    (entries, folder_is_trash(depth))
}

fn row_entries(app: &App, (depth, id): At) -> (Vec<FileEntry>, bool) {
    let browser = &app.browser;
    let in_trash = browser.location_at(depth).as_ref().is_some_and(is_trash);
    if let Some(entries) = filter::targets::of_row(app, depth, id) {
        return (entries, in_trash);
    }
    let entry = shell::shown_view(app)
        .and_then(|view| (view.rows.position)(app, (depth, id)))
        .and_then(|position| browser.entry_at(depth, position));
    let Some(entry) = entry else {
        return (Vec::new(), in_trash);
    };
    let selected = browser.selected_entries();
    if selected
        .iter()
        .any(|other| other.location == entry.location)
    {
        (selected, in_trash)
    } else {
        (vec![entry], in_trash)
    }
}

fn trash_unsupported(app: &App, entries: &[FileEntry]) -> bool {
    let browser = &app.browser;
    let unsupported = |entry: &FileEntry| {
        if entry.location.is_administrator() {
            return true;
        }
        let parent = entry.location.parent();
        let depth = (0..)
            .map_while(|depth| browser.location_at(depth).map(|location| (depth, location)))
            .find(|(_, location)| Some(location) == parent.as_ref())
            .map(|(depth, _)| depth);
        depth.is_some_and(|depth| {
            browser.can_trash_at(depth) == Some(false) && browser.can_delete_at(depth) == Some(true)
        })
    };
    !entries.is_empty() && entries.iter().all(unsupported)
}

fn leave_folder_being_deleted(app: &App, entries: &[FileEntry]) {
    let browser = &app.browser;
    let open = browser
        .active_depth()
        .and_then(|depth| browser.location_at(depth));
    let leaving = open.is_some_and(|open| entries.iter().any(|entry| entry.location == open));
    if leaving && browser.can_start_operation() {
        browser.focus_parent();
    }
}

pub(crate) fn trash_dropped(app: &mut App, entries: Vec<FileEntry>) {
    let _ = request(app, entries, false);
}

/// `request_delete`.
fn request(app: &mut App, entries: Vec<FileEntry>, permanent: bool) -> Outcome {
    if entries.iter().any(|entry| !can_remove(&entry.location)) {
        return Outcome::Ran;
    }
    if permanent {
        show(app, entries, Kind::Permanent);
    } else if trash_unsupported(app, &entries) {
        show(app, entries, Kind::TrashUnavailable);
    } else {
        move_to_trash(app, entries);
    }
    Outcome::Ran
}

/// `move_to_trash`.
fn move_to_trash(app: &mut App, entries: Vec<FileEntry>) {
    leave_folder_being_deleted(app, &entries);
    app.trash.trashing.clone_from(&entries);
    app.browser.delete(entries, false);
    app.browser.focus_active();
}

/// The menu's or the key's items, and whether their folder is the Trash.
fn targets(app: &App, on: &On) -> Option<(Vec<FileEntry>, bool)> {
    let (entries, in_trash) = match *on {
        On::Focus => focused_entries(app),
        On::Row(at) => row_entries(app, at),
        // A folder's menu has no delete rows, and a place's none here.
        On::Column(_) | On::Place(_) => return None,
    };
    (!entries.is_empty()).then_some((entries, in_trash))
}

/// Delete, and the Move to Trash row: for good in the Trash.
fn trashed(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    match targets(app, &call.on) {
        Some((entries, in_trash)) => request(app, entries, in_trash),
        None => Outcome::Declined,
    }
}

/// Shift+Delete, and the Permanently Delete row.
fn deleted(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    match targets(app, &call.on) {
        Some((entries, _)) => request(app, entries, true),
        None => Outcome::Declined,
    }
}

pub(crate) fn confirm_tenxer_delete(app: &mut App, permanent: bool) -> bool {
    let mut entries = crate::tenxer::command_targets(app);
    if entries.is_empty() && empty_active_column_falls_back(app) {
        entries = app.browser.deletion_entries();
    }
    if entries.is_empty() {
        return false;
    }
    let in_trash = crate::tenxer::focused_listing_depth(app)
        .and_then(|depth| app.browser.location_at(depth))
        .as_ref()
        .is_some_and(is_trash);
    request_confirmed(app, entries, permanent || in_trash);
    true
}

fn empty_active_column_falls_back(app: &App) -> bool {
    let focused = crate::tenxer::focused_listing_depth(app);
    filter::targets::selected(app).is_none()
        && in_columns(app)
        && focused == app.browser.active_depth()
        && focused.is_some_and(|depth| {
            app.browser
                .column_snapshot(depth)
                .is_some_and(|column| column.count == 0)
        })
}

fn request_confirmed(app: &mut App, entries: Vec<FileEntry>, permanent: bool) {
    if entries.iter().any(|entry| !can_remove(&entry.location)) {
        return;
    }
    let kind = if permanent {
        Kind::Permanent
    } else if trash_unsupported(app, &entries) {
        Kind::TrashUnavailable
    } else {
        Kind::Trash
    };
    show(app, entries, kind);
}

fn show(app: &mut App, entries: Vec<FileEntry>, kind: Kind) {
    if app.trash.open {
        app.trash.waiting = Some((entries, kind));
        return;
    }
    if let Some((asked, _)) = &mut app.trash.waiting {
        not_again(asked, &entries);
        if asked.is_empty() {
            app.trash.waiting = None;
        }
    }
    let reach = deletion_reach(&entries);
    let trash = &mut app.trash;
    trash.opened += 1;
    let mut confirm = Confirm::new(entries, kind, reach, trash.opened);
    confirm.cursor = app
        .browser
        .focused_item()
        .map(|(depth, _, entry)| (depth, entry.location));
    let needs_size = confirm.needs_size_task();
    let entries = confirm.entries.clone();
    let generation = confirm.generation;
    trash.confirm = Some(confirm);
    trash.open = true;
    if !needs_size {
        return;
    }
    let task = size_in_background(&trash.later, entries, reach, generation);
    if let Some(earlier) = trash.sizing.replace(task) {
        earlier.abort();
    }
}

fn size_in_background(
    later: &Later<App>,
    entries: Vec<FileEntry>,
    reach: Reach,
    generation: u64,
) -> glib::JoinHandle<()> {
    let later = later.clone();
    glib::MainContext::default().spawn_local(async move {
        let mut links = Vec::new();
        let summary = sizes::aggregate_directory_summary(&entries, |index, size| {
            links.push((index, size_text(size)));
        })
        .await;
        let subtitle = permanent_delete_subtitle(reach, &summary);
        later.then(move |app: &mut App, _| {
            let Some(confirm) = app
                .trash
                .confirm
                .as_mut()
                .filter(|confirm| confirm.generation == generation)
            else {
                return false;
            };
            if confirm.kind != Kind::Trash {
                confirm.subtitle = subtitle;
                confirm.sizing = false;
            }
            for (index, text) in links {
                if let Some(metadata) = confirm.metadata.get_mut(index) {
                    *metadata = text;
                }
            }
            true
        });
    })
}

fn close(app: &mut App) -> Option<&Confirm> {
    let trash = &mut app.trash;
    if !trash.open {
        return None;
    }
    trash.open = false;
    if let Some(task) = trash.sizing.take() {
        task.abort();
    }
    trash.confirm.as_ref()
}

/// The confirm button: moves items to Trash for `Kind::Trash`, or deletes them permanently.
fn confirmed(app: &mut App) {
    let Some(confirm) = close(app) else {
        return;
    };
    let queued = if confirm.kind == Kind::Trash {
        Queued::Trash {
            entries: confirm.entries.clone(),
        }
    } else {
        Queued::Delete {
            entries: confirm.entries.clone(),
            remote: confirm.reach == Reach::Remote,
        }
    };
    app.trash.queued.push(queued);
}

/// Cancel, Escape, the close button and a click outside.
fn cancelled(app: &mut App) {
    let Some(confirm) = close(app) else {
        return;
    };
    let queued = Queued::Cancel {
        cursor: confirm.cursor.clone(),
        entries: confirm.entries.clone(),
    };
    app.trash.queued.push(queued);
}

fn not_again(asked: &mut Vec<FileEntry>, deleting: &[FileEntry]) {
    asked.retain(|entry| !deleting.iter().any(|gone| gone.location == entry.location));
}

fn delete_confirmed(app: &mut App, entries: Vec<FileEntry>, remote: bool) {
    leave_folder_being_deleted(app, &entries);
    app.browser.delete_permanently(entries, remote);
    app.browser.focus_active();
}

fn return_cursor(app: &mut App, cursor: Option<(usize, Location)>, entries: &[FileEntry]) {
    let Some((depth, cursor)) = cursor else {
        return;
    };
    let stayed = app
        .browser
        .focused_item()
        .is_some_and(|(now, _, entry)| now == depth && entry.location == cursor);
    if !stayed {
        let locations: Vec<Location> = std::iter::once(cursor)
            .chain(entries.iter().map(|entry| entry.location.clone()))
            .collect();
        app.browser
            .select_shown_entries_by_location_at(depth, &locations);
    }
}

#[cfg(test)]
mod tests;
