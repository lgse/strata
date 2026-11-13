// SPDX-License-Identifier: MIT

//! Inline rename: strata-components' editor (C5) in the row being renamed, Strata's rules
//! for the name in it, and `Browser`'s rename behind it.
//!
//! - **Opening.** F2 and Ctrl+R rename the item `Browser::rename_item` names; the view's slow
//!   second click and the context menu's Rename row name their own entry. All three end in
//!   [`open`].
//! - **The name.** The field is checked as it changes with `name_field_error`, the rule the
//!   GTK field asks (`src/ui/collection_edit.rs`), and its message is the catalogs'.
//! - **Ending.** The editor closes itself and says what it closed on. A changed, valid name
//!   goes to `Browser::rename`; one that is a path goes to `Browser::place_entry`, which moves
//!   the item (`submit_rename_entry` in `src/ui/browser/inline_edit.rs`). Tab and Shift+Tab
//!   then carry the editor to the neighbouring entry (`advance_rename`).
//! - **The answer.** The browser's events say how the rename ended ([`answered`]). A failure
//!   shows GTK's "Unable to rename item" dialog with the browser's message
//!   ([`crate::rename_failed`]); the entry keeps its name, and the editor stays closed
//!   (`on_rename_failed` in `src/ui/browser/events.rs`).
//!
//! - **A new item.** A folder or file just created opens the editor with `Naming::NewItem`
//!   ([`begin_new_item`], from `new_item.rs`): a file may then be named as a folder, and the
//!   name goes to `Browser::name_new_item`, whose undo trashes the item under its new name.
//!
//! Left out, with what brings each back: the label showing the new name before the rename
//! lands (`start_pending_rename`); the field's own menu (C21).

use std::ffi::OsStr;

use rune::{El, Tx};
use strata_components::{
    contract::EntryId,
    rename::{By, RenameEditor, RenameIntent, RenameState},
};
use strata_core::{
    i18n::tr,
    model::{FileEntry, Location},
};
use strata_desktop::keys::{Map, bindings::RENAME};
use strata_fs::{
    app::BrowserEvent,
    services::{Naming, OperationRequestId, TypedPath, name_field_error, validate_basename},
};

use crate::{
    App,
    commands::{Action, Call, Holds, Keys, On, Outcome, Stage},
    new_item, rename_failed, shell,
    slots::{Frame, Listener, Part, Place, Setup, View},
};

/// How a rename the browser was asked for ended.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Completed(OperationRequestId),
    /// A later operation took its place: it may still land, and nothing waits for it.
    Abandoned(OperationRequestId),
    /// No request when the browser refused the name before it started an operation.
    Failed {
        request: Option<OperationRequestId>,
        message: String,
    },
}

impl Answer {
    /// The answer `event` is, when it is one.
    fn of(event: &BrowserEvent) -> Option<Self> {
        match event {
            BrowserEvent::RenameCompleted { request_id } => Some(Self::Completed(*request_id)),
            BrowserEvent::RenameAbandoned { request_id } => Some(Self::Abandoned(*request_id)),
            BrowserEvent::RenameFailed {
                request_id,
                message,
            } => Some(Self::Failed {
                request: *request_id,
                message: message.clone(),
            }),
            _ => None,
        }
    }
}

/// The entry an open edit names.
struct Editing {
    depth: usize,
    id: EntryId,
    entry: FileEntry,
    naming: Naming,
}

/// A rename the browser was asked for and has not answered.
struct Asked {
    /// `None` when the browser started no operation.
    request: Option<OperationRequestId>,
    editing: Editing,
    /// What takes the item's row when a typed path moves it: the first folder of the path,
    /// or the item itself in the folder above.
    placed: Option<Location>,
    /// Tab took the editor on, so the cursor is no longer this rename's to place.
    advanced: bool,
}

pub(crate) struct Rename {
    pub(crate) state: RenameState,
    editing: Option<Editing>,
    asked: Option<Asked>,
    /// The field's accessible name, as the GTK field's (`src/ui/browser/columns/rows/widgets.rs`).
    label: String,
}

/// What a typed name asks of the browser.
enum Wanted {
    Name(String),
    Path(TypedPath),
}

impl Rename {
    fn translated() -> Self {
        Self {
            state: RenameState::new(),
            editing: None,
            asked: None,
            label: tr("Rename"),
        }
    }

    /// Strata's refusal of `name` for the entry being renamed, in the user's language.
    fn error(&self, name: &str) -> Option<String> {
        let editing = self.editing.as_ref()?;
        name_field_error(name, editing.naming, editing.entry.is_directory()).map(tr)
    }

    /// Whether a rename waits for the browser's answer (`rename_operation_pending`).
    fn pending(&self) -> bool {
        self.asked.is_some()
    }
}

/// The editor's message, over the rows.
fn message(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    frame.layer.extend(editor(app).message(app, &app.look));
    None
}

/// The editor's share of the window: its message, the browser's answers, its poll, and the
/// window's keys waiting while its field is open. The field itself is in the row, where the
/// view puts [`editor`]'s slot.
impl Part for Rename {
    const PLACE: Place<App> = Place::Overlays(message);

    fn new(_: &Setup<'_>) -> Self {
        Self::translated()
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, _, heard| {
            if let Some(answer) = Answer::of(event) {
                heard.then(move |app: &mut App, tx: &mut Tx| {
                    answered(app, answer, tx);
                    true
                });
            }
        }))
    }

    fn poll(app: &mut App, _: f64, tx: &mut Tx) -> bool {
        tick(app, tx)
    }

    const COMMANDS: &'static [Action<App>] =
        &[Action::new("rename", renamed).command(RENAME).keys(&[
            // `inline_editing` in `src/ui/window/keyboard/commands.rs`: not from a text field,
            // but for the filter's.
            Keys::at(Stage::InlineEditing, &[RENAME.in_map(Map::Default)])
                .when(|_, at| at.filter() || !at.text()),
        ])];

    fn keyboard(app: &App) -> Option<Holds> {
        app.rename.state.has_keyboard().then_some(Holds::Edit)
    }
}

/// The command: F2 and Ctrl+R rename the item the keyboard is on, and a stage that opened
/// no editor has not taken the key (`begin_rename`). The menu's row selects its entry alone
/// first, and then renames it as the key does (`rename_context_entry` in
/// `src/ui/browser/context_menu/focus.rs`).
fn renamed(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    match call.on {
        On::Focus if begin_focused(app, tx) => Outcome::Ran,
        On::Row((depth, id)) => {
            let Some(view) = host(app) else {
                return Outcome::Declined;
            };
            (view.rows.select_only)(app, (depth, id), tx);
            shell::publish(app);
            begin_on(app, depth, id, tx);
            Outcome::Ran
        }
        // No editor opened; and a menu with no entry has no Rename row.
        On::Focus | On::Column(_) => Outcome::Declined,
    }
}

/// The editor, as a view's rename slot and the window's message layer build it.
pub(crate) fn editor(app: &App) -> RenameEditor<App> {
    let editor = RenameEditor::new(
        |app: &App| &app.rename.state,
        |app: &mut App| &mut app.rename.state,
    )
    .validate(|app: &App, _, name| app.rename.error(name))
    .label(|app: &App, _| app.rename.label.clone())
    .on_intent(heard);
    // The keyboard goes back to the view: a column takes none of its own.
    match shell::view_focus(app) {
        Some(view) => editor.focus_return(view),
        None => editor,
    }
}

/// `can_rename` in `src/ui/browser/file_commands.rs`: nothing in the Trash is renamed.
fn can_rename(entry: &FileEntry) -> bool {
    !entry
        .location
        .uri_value()
        .is_some_and(|uri| uri.starts_with("trash:"))
}

/// The view the editor edits in: the rows it opens on and moves between.
fn host(app: &App) -> Option<&'static View<App>> {
    shell::shown_view(app)
}

/// An edit is open in `view`, or none is: what follows the cursor there waits while one is.
fn hold(app: &App, view: &View<App>, editing: bool) {
    if let Some(hold) = view.editing.hold {
        hold(app, editing);
    }
}

/// How `entry` is named when nothing says otherwise: a relative path moves a local item, and
/// elsewhere a name is a name (`begin` in `src/ui/collection_edit.rs`).
fn naming_of(entry: &FileEntry) -> Naming {
    match entry.location.native_path() {
        Some(_) => Naming::Path,
        None => Naming::Single,
    }
}

/// Opens the editor on `entry`, a row of column `depth`, named as `naming` says. False when
/// it has no row to open in.
fn open(app: &mut App, depth: usize, entry: FileEntry, naming: Naming, tx: &mut Tx) -> bool {
    let Some(view) = host(app).filter(|_| can_rename(&entry)) else {
        return false;
    };
    let id = shell::id_of(app, &entry.location);
    // An edit still open ends for this one, and keeps its name: nothing to act on.
    let Some(_ended) = (view.editing.begin)(app, (depth, id), tx) else {
        return false;
    };
    app.rename.editing = Some(Editing {
        depth,
        id,
        entry,
        naming,
    });
    hold(app, view, true);
    true
}

/// F2 and Ctrl+R: the focused item, or the folder the keyboard has entered. False when no
/// editor opened. A new item still waiting for its editor gives way, whether or not one
/// opens (`begin_rename` in `src/ui/browser/inline_edit.rs`).
fn begin_focused(app: &mut App, tx: &mut Tx) -> bool {
    if app.rename.pending() {
        return false;
    }
    new_item::cancel(app);
    match app.browser.rename_item() {
        Some((depth, _, entry)) => {
            let naming = naming_of(&entry);
            open(app, depth, entry, naming, tx)
        }
        None => false,
    }
}

/// The view's slow second click and the menu's Rename row: entry `id` of column `depth`. A
/// new item still waiting for its editor gives way, as to F2.
pub(crate) fn begin_on(app: &mut App, depth: usize, id: EntryId, tx: &mut Tx) {
    if app.rename.pending() {
        return;
    }
    new_item::cancel(app);
    let position = host(app).and_then(|view| (view.rows.position)(app, (depth, id)));
    if let Some(entry) = position.and_then(|position| app.browser.entry_at(depth, position)) {
        let naming = naming_of(&entry);
        open(app, depth, entry, naming, tx);
    }
}

/// A folder or file just created, `entry` in column `depth`: the editor opens on it with
/// `naming`, `Naming::NewItem` unless the folder takes a selection in. False while its row
/// cannot take the editor yet, or a rename waits for the browser.
pub(crate) fn begin_new_item(
    app: &mut App,
    depth: usize,
    entry: FileEntry,
    naming: Naming,
    tx: &mut Tx,
) -> bool {
    !app.rename.pending() && open(app, depth, entry, naming, tx)
}

/// Ends an open edit without renaming, before a new item takes the editor
/// (`cancel_rename`).
pub(crate) fn cancel(app: &mut App, tx: &mut Tx) {
    let polled = app.rename.state.cancel(By::Host);
    for intent in polled.intents {
        heard(app, intent, tx);
    }
}

/// What the name typed for `editing` asks. `None` renames nothing: the name is the one the
/// entry has, or one the browser would refuse.
fn wanted(editing: &Editing, typed: &str) -> Option<Wanted> {
    let entry = &editing.entry;
    let directory = entry.is_directory();
    // `Submission::path` in `src/ui/collection_edit.rs`: a new file may become a folder.
    let path = match editing.naming {
        Naming::Single => None,
        Naming::Path => TypedPath::parse(typed, directory, false).ok(),
        Naming::NewItem => TypedPath::parse(typed, directory, true).ok(),
    };
    let path = path.map(|path| match entry.location.parent() {
        Some(parent) => path.within(&parent),
        None => path,
    });
    let name = match path {
        Some(path) if !path.is_rename_of(directory) => return Some(Wanted::Path(path)),
        // A folder typed with a trailing slash is still just named.
        Some(path) => path.name,
        None => typed.to_owned(),
    };
    let changed = name != entry.display_name && validate_basename(&name).is_ok();
    changed.then_some(Wanted::Name(name))
}

/// Where the first part of `path` lands, for an item in `folder`: what the view selects
/// once the item has moved.
fn placed(folder: Option<Location>, path: &TypedPath) -> Option<Location> {
    path.start(&folder?)?.child(OsStr::new(path.first()))
}

/// Asks the browser for the rename `typed` names, and remembers it until it is answered
/// (`submit_rename_entry` in `src/ui/browser/inline_edit.rs`).
fn ask(app: &mut App, editing: Editing, typed: String) {
    let Some(wanted) = wanted(&editing, &typed) else {
        // A folder made for a selection keeps the name it was made with.
        app.browser.clear_group_folder();
        return;
    };
    let entry = editing.entry.clone();
    let new_item = editing.naming == Naming::NewItem;
    let (request, placed) = match wanted {
        Wanted::Name(name) if new_item => (app.browser.name_new_item(entry, name), None),
        Wanted::Name(name) => (app.browser.rename(entry, name), None),
        Wanted::Path(path) => {
            let placed = placed(entry.location.parent(), &path);
            (app.browser.place_entry(entry, path, new_item), placed)
        }
    };
    app.rename.asked = Some(Asked {
        request,
        editing,
        placed,
        advanced: false,
    });
}

/// Tab and Shift+Tab: the cursor and the editor go on to the neighbour `step` rows away,
/// unless the entry was the last that way (`advance_rename`).
fn advance(app: &mut App, (depth, id): (usize, EntryId), step: isize, tx: &mut Tx) {
    let Some(view) = host(app) else {
        return;
    };
    let Some(next) = (view.rows.neighbour)(app, (depth, id), step) else {
        return;
    };
    (view.rows.select_only)(app, (depth, next), tx);
    shell::publish(app);
    // The committed row must not pull the cursor back once its rename lands.
    if let Some(asked) = &mut app.rename.asked {
        asked.advanced = true;
    }
    if let Some(follow_now) = view.editing.follow_now {
        follow_now(app);
    }
    let position = (view.rows.position)(app, (depth, next));
    if let Some(entry) = position.and_then(|position| app.browser.entry_at(depth, position)) {
        let naming = naming_of(&entry);
        open(app, depth, entry, naming, tx);
    }
}

/// What the editor asked as it closed: from its own keys, from a poll, or from the host.
pub(crate) fn heard(app: &mut App, intent: RenameIntent, tx: &mut Tx) {
    // The field's own menu is C21's, which this interface has not: the edit stays open.
    if matches!(intent, RenameIntent::FieldMenu { .. }) {
        return;
    }
    let step = intent.by().and_then(By::step);
    let Some(editing) = app.rename.editing.take() else {
        return;
    };
    if let Some(view) = host(app) {
        hold(app, view, false);
    }
    let from = (editing.depth, editing.id);
    match intent {
        RenameIntent::Rename { name, .. } => ask(app, editing, name),
        RenameIntent::Move { path, .. } => ask(app, editing, path),
        // The name stays: Escape, an unchanged, an empty or a refused name.
        _ => {}
    }
    if let Some(step) = step {
        advance(app, from, step, tx);
    }
    shell::settle(app, tx);
}

/// One poll: the editor notices its field lost the keyboard, its row left the viewport or
/// its entry left the listing. True when the view has to run.
fn tick(app: &mut App, tx: &mut Tx) -> bool {
    let Some(view) = host(app) else {
        return false;
    };
    let polled = (view.editing.tick)(app);
    for intent in polled.intents {
        heard(app, intent, tx);
    }
    polled.changed
}

/// Ends an open edit as Enter would, before the window does something that takes the
/// listing from under it.
pub(crate) fn commit(app: &mut App, tx: &mut Tx) {
    let polled = app.rename.state.commit(By::Host);
    for intent in polled.intents {
        heard(app, intent, tx);
    }
}

/// The rename the browser answers, when it is the one it was last asked for.
fn asked_for(app: &mut App, request: Option<OperationRequestId>) -> Option<Asked> {
    app.rename.asked.take_if(|asked| asked.request == request)
}

/// The rename landed. A typed path moved the item out of its row: what took its place is
/// selected (`reveal_placed_entry` in `src/ui/browser/inline_edit/pending.rs`). A plain
/// rename keeps its row and its cursor under the new name, which the browser sees to.
fn completed(app: &mut App, asked: &Asked) {
    let Some(target) = asked.placed.clone().filter(|_| !asked.advanced) else {
        return;
    };
    if let Some(folder) = target.parent()
        && app.browser.open_depth(&folder).is_some()
    {
        app.browser.reveal_locations(folder, vec![target]);
    }
}

/// The rename failed: GTK's dialog says why (`on_rename_failed`). The entry kept its
/// location and its name. One that Tab moved on from takes the cursor back, and the editor
/// Tab opened on its neighbour closes (`fail_pending_rename_from_browser` in
/// `src/ui/browser/inline_edit/pending.rs`).
fn failed(app: &mut App, asked: &Asked, message: String, tx: &mut Tx) {
    if asked.advanced {
        cancel(app, tx);
        let location = &asked.editing.entry.location;
        if let Some(depth) = location
            .parent()
            .and_then(|parent| app.browser.open_depth(&parent))
        {
            app.browser
                .select_entries_by_location_at(depth, std::slice::from_ref(location));
        }
    }
    rename_failed::show(app, message);
}

/// How the browser's rename ended.
fn answered(app: &mut App, answer: Answer, tx: &mut Tx) {
    match answer {
        Answer::Completed(request) => {
            if let Some(asked) = asked_for(app, Some(request)) {
                completed(app, &asked);
            }
        }
        Answer::Abandoned(request) => {
            asked_for(app, Some(request));
        }
        Answer::Failed { request, message } => {
            if let Some(asked) = asked_for(app, request) {
                failed(app, &asked, message, tx);
            }
        }
    }
}

#[cfg(test)]
mod tests;
