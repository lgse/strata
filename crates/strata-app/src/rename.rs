// SPDX-License-Identifier: MIT

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
    placed: Option<Location>,
    /// Where a plain rename puts the item, in its own folder: the row to show once it lands.
    renamed: Option<Location>,
    /// Tab took the editor on, so the cursor is no longer this rename's to place.
    advanced: bool,
}

pub(crate) struct Rename {
    pub(crate) state: RenameState,
    editing: Option<Editing>,
    asked: Option<Asked>,
    /// A completed rename whose row waits to be shown ([`shown`]).
    shown: Option<shown::Pending>,
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
            shown: None,
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

    const COMMANDS: &'static [Action<App>] = &[Action::new("rename", renamed)
        .command(RENAME)
        .keys(&[
            Keys::at(Stage::InlineEditing, &[RENAME.in_map(Map::Default)])
                .when(|_, at| !at.within("preview") && (at.filter() || !at.text())),
        ])];

    fn keyboard(app: &App) -> Option<Holds> {
        app.rename.state.has_keyboard().then_some(Holds::Edit)
    }
}

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
        On::Focus | On::Column(_) | On::Place(_) => Outcome::Declined,
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

pub(crate) fn can_rename(entry: &FileEntry) -> bool {
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

fn naming_of(entry: &FileEntry) -> Naming {
    match entry.location.native_path() {
        Some(_) => Naming::Path,
        None => Naming::Single,
    }
}

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

fn begin_focused(app: &mut App, tx: &mut Tx) -> bool {
    if app.rename.pending() {
        return false;
    }
    new_item::cancel(app);
    if crate::filter::targets::selected(app).is_some() {
        return match crate::filter::targets::under_cursor(app) {
            Some((depth, entry)) => {
                let naming = naming_of(&entry);
                open(app, depth, entry, naming, tx)
            }
            None => false,
        };
    }
    match app.browser.rename_item() {
        Some((depth, _, entry)) => {
            let naming = naming_of(&entry);
            open(app, depth, entry, naming, tx)
        }
        None => false,
    }
}

pub(crate) fn begin_on(app: &mut App, depth: usize, id: EntryId, tx: &mut Tx) {
    if app.rename.pending() {
        return;
    }
    new_item::cancel(app);
    if let Some(entry) = entry_of(app, depth, id) {
        let naming = naming_of(&entry);
        open(app, depth, entry, naming, tx);
    }
}

fn entry_of(app: &App, depth: usize, id: EntryId) -> Option<FileEntry> {
    if let Some(result) = crate::filter::targets::at(app, depth, id) {
        return Some(result);
    }
    let position = host(app).and_then(|view| (view.rows.position)(app, (depth, id)))?;
    app.browser.entry_at(depth, position)
}

pub(crate) fn begin_new_item(
    app: &mut App,
    depth: usize,
    entry: FileEntry,
    naming: Naming,
    tx: &mut Tx,
) -> bool {
    !app.rename.pending() && open(app, depth, entry, naming, tx)
}

pub(crate) fn cancel(app: &mut App, tx: &mut Tx) {
    let polled = app.rename.state.cancel(By::Host);
    for intent in polled.intents {
        heard(app, intent, tx);
    }
}

fn wanted(editing: &Editing, typed: &str) -> Option<Wanted> {
    let entry = &editing.entry;
    let directory = entry.is_directory();
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

fn placed(folder: Option<Location>, path: &TypedPath) -> Option<Location> {
    path.start(&folder?)?.child(OsStr::new(path.first()))
}

fn ask(app: &mut App, editing: Editing, typed: String) {
    let Some(wanted) = wanted(&editing, &typed) else {
        // A folder made for a selection keeps the name it was made with.
        app.browser.clear_group_folder();
        return;
    };
    let renamed = match &wanted {
        Wanted::Name(name) => editing
            .entry
            .location
            .parent()
            .and_then(|folder| folder.child(OsStr::new(name))),
        Wanted::Path(_) => None,
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
    if request.is_some() {
        crate::filter::follow_renamed(app, &editing.entry.location, renamed.as_ref());
    }
    app.rename.asked = Some(Asked {
        request,
        editing,
        placed,
        renamed,
        advanced: false,
    });
}

pub(crate) fn start_typed_rename(app: &mut App, depth: usize, entry: FileEntry, typed: String) {
    new_item::cancel(app);
    let id = shell::id_of(app, &entry.location);
    let naming = naming_of(&entry);
    ask(
        app,
        Editing {
            depth,
            id,
            entry,
            naming,
        },
        typed,
    );
}

fn advance(app: &mut App, (depth, id): (usize, EntryId), step: isize, tx: &mut Tx) {
    if crate::filter::targets::at(app, depth, id).is_some() {
        return;
    }
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
    if let Some(entry) = entry_of(app, depth, next) {
        let naming = naming_of(&entry);
        open(app, depth, entry, naming, tx);
    }
}

/// What the editor asked as it closed: from its own keys, from a poll, or from the host.
pub(crate) fn heard(app: &mut App, intent: RenameIntent, tx: &mut Tx) {
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
        _ => app.browser.clear_group_folder_for(&editing.entry.location),
    }
    if let Some(step) = step {
        advance(app, from, step, tx);
    }
    shell::settle(app, tx);
}

fn tick(app: &mut App, tx: &mut Tx) -> bool {
    let Some(view) = host(app) else {
        return false;
    };
    let polled = (view.editing.tick)(app);
    for intent in polled.intents {
        heard(app, intent, tx);
    }
    polled.changed | shown::tick(app, tx)
}

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

fn completed(app: &mut App, asked: &Asked, tx: &mut Tx) {
    if asked.advanced {
        return;
    }
    if let Some(target) = asked.placed.clone() {
        if let Some(folder) = target.parent()
            && app.browser.open_depth(&folder).is_some()
        {
            app.browser.reveal_locations(folder, vec![target]);
        }
        return;
    }
    if let Some(target) = asked.renamed.clone() {
        let old = asked.editing.entry.location.clone();
        app.rename.shown = Some(shown::Pending::new(asked.editing.depth, old, target));
        shown::tick(app, tx);
    }
}

fn failed(app: &mut App, asked: &Asked, message: String, tx: &mut Tx) {
    crate::filter::forget_renamed(app, &asked.editing.entry.location, asked.renamed.as_ref());
    app.browser
        .clear_group_folder_for(&asked.editing.entry.location);
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
                completed(app, &asked, tx);
            }
        }
        Answer::Abandoned(request) => {
            if let Some(asked) = asked_for(app, Some(request)) {
                crate::filter::forget_renamed(
                    app,
                    &asked.editing.entry.location,
                    asked.renamed.as_ref(),
                );
                app.browser
                    .clear_group_folder_for(&asked.editing.entry.location);
            }
        }
        Answer::Failed { request, message } => match asked_for(app, request) {
            Some(asked) => failed(app, &asked, message, tx),
            None => rename_failed::show(app, message),
        },
    }
}

mod shown;

#[cfg(test)]
mod tests;
