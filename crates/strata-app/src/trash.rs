// SPDX-License-Identifier: MIT

//! Move to Trash: Delete in the default map, and the entry menu's Move to Trash row
//! (`request_delete` in `src/ui/browser/trash.rs`).
//!
//! - **The key.** The dispatcher's `dismissal` stage reads it with Ctrl, Alt or Super held
//!   too, outside a text field (`delete_kind` and `confirms_delete` in
//!   `src/ui/window/keyboard/items.rs`). It acts on the active column's selection, or the
//!   cursor's entry when nothing is selected there, else on the folder the keyboard entered
//!   (`confirm_delete` in `src/ui/browser/view_edit.rs`). The 10xer map's D is 10xer mode's
//!   own stage, A11's.
//! - **The menu row** acts on the selection when the row's entry is in it, else on the entry
//!   alone (`context_entries` in `src/ui/browser/context_menu.rs`).
//! - **What happens.** Nothing inside a trashed folder can be removed alone, and nothing is
//!   asked for one. Otherwise the items go to the Trash: a folder whose column is open gives
//!   the keyboard to its row in the folder above first, so the deletion moves the cursor on
//!   to a neighbour (`leave_folder_being_deleted`). Undo puts them back (`undo.rs`).
//!
//! Left out, with what brings each back:
//!
//! - **The confirmation dialogs**: Shift+Delete and the Permanently Delete row, Delete in
//!   the Trash itself, and the fallback where a location can delete but not trash
//!   (`show_delete_dialog`). Until they land, those cases do nothing, and Delete there is
//!   left to the view.
//! - **The fly-to-trash animation** (`prepare_fly_to_trash`): C23's trash flight.
//! - **A recursive search's own selection** (`selected_search_results`): A10's.

use rune::Tx;
use strata_core::model::{FileEntry, Location};
use strata_desktop::keys::{Map, Modifiers, bindings::MOVE_TO_TRASH};

use crate::{
    App,
    commands::{Action, Call, Keys, On, Outcome, Stage},
    shell,
    slots::{At, Frame, Part, Place, Setup},
};

/// What `delete_kind` lets be held.
const DELETE_MODIFIERS: Modifiers = Modifiers::CONTROL_MASK
    .union(Modifiers::ALT_MASK)
    .union(Modifiers::SUPER_MASK);

/// The part holds nothing yet: what it will hold is the confirmation dialog it leaves out.
pub(crate) struct Trash;

/// Nothing is drawn: the rows leave, and the dialogs are left out.
fn nothing(app: &App, _: &mut Frame<'_, App>) -> Option<rune::El> {
    let Trash = &app.trash;
    None
}

impl Part for Trash {
    const PLACE: Place<App> = Place::Overlays(nothing);

    fn new(_: &Setup<'_>) -> Self {
        Self
    }

    const COMMANDS: &'static [Action<App>] = &[Action::new("trash", trashed)
        .command(MOVE_TO_TRASH)
        .keys(&[Keys::at(
            Stage::Dismissal,
            &[MOVE_TO_TRASH
                .in_map(Map::Default)
                .ignoring(DELETE_MODIFIERS)],
        )
        .when(|_, at| !at.text())])];
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

/// What the key acts on, and whether its folder is the Trash (`confirm_delete`).
fn focused_entries(app: &App) -> (Vec<FileEntry>, bool) {
    let browser = &app.browser;
    let depth = browser.active_depth();
    let column = depth
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

/// `request_delete` for what goes to the Trash. `Declined` where GTK asks first, which this
/// interface cannot yet.
fn request(app: &mut App, entries: Vec<FileEntry>, permanent: bool) -> Outcome {
    if entries.iter().any(|entry| !can_remove(&entry.location)) {
        return Outcome::Ran;
    }
    if permanent || trash_unsupported(app, &entries) {
        tracing::debug!("a deletion GTK confirms first, and this interface has no dialog yet");
        return Outcome::Declined;
    }
    leave_folder_being_deleted(app, &entries);
    app.browser.delete(entries, false);
    app.browser.focus_active();
    Outcome::Ran
}

fn trashed(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    let (entries, in_trash) = match call.on {
        On::Focus => focused_entries(app),
        On::Row(at) => row_entries(app, at),
        // A folder's menu has no Move to Trash row.
        On::Column(_) => return Outcome::Declined,
    };
    if entries.is_empty() {
        return Outcome::Declined;
    }
    request(app, entries, in_trash)
}

#[cfg(test)]
mod tests;
