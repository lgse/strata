// SPDX-License-Identifier: MIT

//! Deleting: Move to Trash and Permanently Delete, by their keys and the entry menu's rows,
//! and the confirmation GTK asks first (`request_delete` in `src/ui/browser/trash.rs`).
//!
//! - **The keys.** The dispatcher's `dismissal` stage reads Delete and Shift+Delete in the
//!   default map, with Ctrl, Alt or Super held too, outside a text field (`delete_kind` and
//!   `confirms_delete` in `src/ui/window/keyboard/items.rs`). In Columns they act on the active
//!   column's selection, or its cursor's entry, else on the folder the keyboard entered; in the
//!   other views on the selection or that folder (`confirm_delete` in
//!   `src/ui/browser/view_edit.rs`). In the Trash, Delete deletes for good. 10xer's D and
//!   Shift+D are 10xer mode's own stage, A11's.
//! - **The menu rows** act on the selection when the row's entry is in it, else on the entry
//!   alone (`context_entries` in `src/ui/browser/context_menu.rs`).
//! - **What happens.** Nothing inside a trashed folder can be removed alone, and nothing is
//!   asked for one. A permanent delete, and a trash where the location deletes but cannot
//!   trash, ask first ([`dialog`]). Otherwise the items go to the Trash. Either way a folder
//!   whose column is open gives the keyboard to its row in the folder above first, so the
//!   deletion moves the cursor on to a neighbour (`leave_folder_being_deleted`). Undo puts
//!   trashed items back (`undo.rs`).
//! - **A trash that fails for some items** because the location cannot trash them asks to
//!   delete those permanently, when nothing else failed (`report_partial_failure` in
//!   `src/ui/browser/events/operations.rs`).
//! - **The dialog's buttons** only queue what they do: the part's `poll` hands it to the
//!   browser, with the handler's `Tx` the button lacks.
//!
//! Left out, with what brings each back:
//!
//! - **"Completed with errors"**, GTK's message for a failure with nothing to retry, and its
//!   form with a retry button beside failures that cannot be retried: the error dialog the
//!   location bar's package makes the window's (A5).
//! - **The fly-to-trash animation** (`prepare_fly_to_trash`): C23's trash flight.
//! - **A recursive search's own selection** (`selected_search_results`): A10's.

use rune::Tx;
use strata_core::model::{FileEntry, Location};
use strata_desktop::keys::{
    Map, Modifiers,
    bindings::{DELETE_PERMANENTLY, MOVE_TO_TRASH},
};
use strata_fs::app::BrowserEvent;

use crate::{
    App,
    commands::{Action, Call, Holds, Keys, On, Outcome, Stage},
    shell,
    slots::{At, Later, Listener, Part, Place, Setup},
};

mod dialog;
pub(crate) mod empty;
mod sizes;

use dialog::{Confirm, Kind};
use sizes::{Reach, deletion_reach, permanent_delete_subtitle, size_text};

/// What `delete_kind` lets be held.
const DELETE_MODIFIERS: Modifiers = Modifiers::CONTROL_MASK
    .union(Modifiers::ALT_MASK)
    .union(Modifiers::SUPER_MASK);

/// What a dialog button asked, for the part's next `poll`, with what it was asked about: a
/// confirmation that opens meanwhile changes nothing of it.
enum Queued {
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
    /// A confirmation asked for while another is open: it opens once that one closes, so an
    /// open one is never replaced under the keyboard (GTK stacks a second dialog).
    waiting: Option<(Vec<FileEntry>, Kind)>,
    /// The open confirmation's size task, which stops when it closes (GTK aborts it).
    sizing: Option<glib::JoinHandle<()>>,
    /// Counts the confirmations, so a size task's answer finds its own.
    opened: u64,
    /// What the last Move to Trash asked for (`pending_delete_entries`): what a partial
    /// failure's retry may delete.
    trashing: Vec<FileEntry>,
    /// Empty Trash's dialog ([`empty`]).
    empty: empty::Empty,
}

/// A trash that some items failed (`OperationCompletedWithErrors`).
struct Failed {
    retryable: Vec<Location>,
    others_failed: bool,
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
                retryable_locations,
                has_non_retryable_failures,
                ..
            } = event
            {
                let failed = Failed {
                    retryable: retryable_locations.clone(),
                    others_failed: *has_non_retryable_failures,
                };
                heard.then(move |app: &mut App, _: &mut Tx| completed_with_errors(app, failed));
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
            match queued {
                Queued::Delete { entries, remote } => {
                    if let Some((asked, _)) = &mut waiting {
                        not_again(asked, &entries);
                    }
                    delete_confirmed(app, entries, remote);
                }
                Queued::Cancel { cursor, entries } => return_cursor(app, cursor, &entries),
            }
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
            .when(|_, at| !at.text())]),
        Action::new("delete", deleted)
            .command(DELETE_PERMANENTLY)
            .keys(&[Keys::at(
                Stage::Dismissal,
                &[DELETE_PERMANENTLY
                    .in_map(Map::Default)
                    .ignoring(DELETE_MODIFIERS)],
            )
            .when(|_, at| !at.text())]),
        // No key empties the Trash: the sidebar's Trash menu asks for it.
        Action::new("empty-trash", empty::run),
    ];
}

fn is_trash(location: &Location) -> bool {
    location
        .uri_value()
        .is_some_and(|uri| uri.starts_with("trash:"))
}

/// `can_remove_location` in `src/ui/browser/paths.rs`: GVfs moves or deletes a whole
/// trashed item, not what is inside one.
fn can_remove(location: &Location) -> bool {
    let trash_item = || {
        location
            .parent()
            .is_some_and(|parent| parent.uri_value() == Some("trash:///"))
    };
    !is_trash(location) || trash_item()
}

/// Whether the shown view is Columns, where a column's cursor counts when nothing is
/// selected in it.
fn in_columns(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)("columns"))
}

/// What the key acts on, and whether its folder is the Trash (`confirm_delete`). In Columns:
/// the active column's selection, or its cursor's entry, else the folder the keyboard
/// entered. In the other views: the selection, or that folder, and never the cursor alone.
fn focused_entries(app: &App) -> (Vec<FileEntry>, bool) {
    let browser = &app.browser;
    let depth = browser.active_depth();
    let column = depth
        .filter(|_| in_columns(app))
        .map(|depth| browser.command_entries(depth))
        .filter(|entries| !entries.is_empty());
    let entries = column.unwrap_or_else(|| browser.deletion_entries());
    let folder = depth.and_then(|depth| browser.location_at(depth));
    (entries, folder.as_ref().is_some_and(is_trash))
}

/// What the menu opened on row `at` acts on (`context_entries`), and whether its folder is
/// the Trash.
fn row_entries(app: &App, (depth, id): At) -> (Vec<FileEntry>, bool) {
    let browser = &app.browser;
    let in_trash = browser.location_at(depth).as_ref().is_some_and(is_trash);
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

/// `trash_unsupported_for`: every item's folder is open in a column that can delete and
/// cannot trash, or the items are an administrator's.
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

/// `leave_folder_being_deleted`: the folder the active column shows is being deleted, so
/// its row in the folder above takes the keyboard first.
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
fn targets(app: &App, on: On) -> Option<(Vec<FileEntry>, bool)> {
    let (entries, in_trash) = match on {
        On::Focus => focused_entries(app),
        On::Row(at) => row_entries(app, at),
        // A folder's menu has no delete rows.
        On::Column(_) => return None,
    };
    (!entries.is_empty()).then_some((entries, in_trash))
}

/// Delete, and the Move to Trash row: for good in the Trash.
fn trashed(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    match targets(app, call.on) {
        Some((entries, in_trash)) => request(app, entries, in_trash),
        None => Outcome::Declined,
    }
}

/// Shift+Delete, and the Permanently Delete row.
fn deleted(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    match targets(app, call.on) {
        Some((entries, _)) => request(app, entries, true),
        None => Outcome::Declined,
    }
}

/// `show_delete_dialog`: the confirmation opens over the window, and its size task starts.
/// While one is open, the new one waits for it to close.
fn show(app: &mut App, entries: Vec<FileEntry>, kind: Kind) {
    if app.trash.open {
        app.trash.waiting = Some((entries, kind));
        return;
    }
    let reach = deletion_reach(&entries);
    let trash = &mut app.trash;
    trash.opened += 1;
    let mut confirm = Confirm::new(entries, kind, reach, trash.opened);
    confirm.cursor = app
        .browser
        .focused_item()
        .map(|(depth, _, entry)| (depth, entry.location));
    let entries = confirm.entries.clone();
    let generation = confirm.generation;
    trash.confirm = Some(confirm);
    trash.open = true;
    let task = size_in_background(&trash.later, entries, reach, generation);
    if let Some(earlier) = trash.sizing.replace(task) {
        earlier.abort();
    }
}

/// `spawn_delete_size_task`: the total, and each listed file link's size, off the interface
/// thread's turn.
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
            confirm.subtitle = subtitle;
            confirm.sizing = false;
            for (index, text) in links {
                if let Some(metadata) = confirm.metadata.get_mut(index) {
                    *metadata = text;
                }
            }
            true
        });
    })
}

/// Closes the open confirmation, and stops its size task: what the buttons and dismissal
/// share. `None` when none was open.
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

/// The delete button: the dialog closes and the items it asked about go for good
/// (`delete_confirmed_permanently`).
fn confirmed(app: &mut App) {
    let Some(confirm) = close(app) else {
        return;
    };
    let queued = Queued::Delete {
        entries: confirm.entries.clone(),
        remote: confirm.reach == Reach::Remote,
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

/// Leaves out of a waiting confirmation what a confirmed delete is taking already: nothing is
/// asked about twice.
fn not_again(asked: &mut Vec<FileEntry>, deleting: &[FileEntry]) {
    asked.retain(|entry| !deleting.iter().any(|gone| gone.location == entry.location));
}

fn delete_confirmed(app: &mut App, entries: Vec<FileEntry>, remote: bool) {
    leave_folder_being_deleted(app, &entries);
    app.browser.delete_permanently(entries, remote);
    app.browser.focus_active();
}

/// `return_cursor_on_cancel`: a cursor something moved while the dialog was open goes back
/// to the items it asked about.
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

/// `on_operation_completed_with_errors` and `report_partial_failure`, for what a trash
/// failed: the items the location could not trash may go for good, when nothing else failed.
fn completed_with_errors(app: &mut App, failed: Failed) -> bool {
    let trashing = std::mem::take(&mut app.trash.trashing);
    // `retryable_delete_entries`.
    let retryable: Vec<FileEntry> = trashing
        .into_iter()
        .filter(|entry| failed.retryable.contains(&entry.location))
        .collect();
    if retryable.is_empty() || failed.others_failed {
        tracing::warn!("an operation completed with errors, and this interface has no report yet");
        return false;
    }
    show(app, retryable, Kind::TrashUnavailable);
    true
}

#[cfg(test)]
mod tests;
