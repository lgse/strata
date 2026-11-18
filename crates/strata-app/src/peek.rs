// SPDX-License-Identifier: MIT

//! The folder peek (C22's [`FolderPeek`]) in Icons and List: 10xer's `i` on a folder opens a
//! card of its first rows beside it, and `i` again, Escape, navigating, a view or tab switch
//! close it (`toggle_folder_peek` and `open_keyboard_peek` in `src/ui/browser/view_modes.rs`
//! and `src/ui/browser/peek.rs`).
//!
//! - **The browser lists it.** As GTK's does, the peek asks the shared browser
//!   (`Browser::begin_peek`) and hears its `Peek*` events, so a close from anywhere
//!   (`Browser::close_peek`: Escape, a drag, a navigation) closes the card too.
//! - **Any move closes it.** GTK closes a peek on every cursor or selection move, through
//!   `keyboard_navigation` (`src/ui/browser/view_selection.rs`, `view_navigation.rs`): the keys,
//!   Select All, Invert Selection, find, a filter, a search. Here the part notes the view's
//!   selection as the peek opens, and closes the peek on the next settle once its cursor or its
//!   selected entries differ or results took the listing's place, whatever moved them
//!   (`Part::publish`). Reload keeps it, as GTK's does: its sync moves C1's revision only.
//! - **A snapshot.** The card lists the folder as it was when the peek opened; GTK's adds no
//!   file made in it later, nor does Reload list it again.
//! - **A resting pointer peeks** when the `folder_peeking` preference is on, never in the file
//!   chooser (GTK's `interactive && folder_peeking`, `src/ui/browser/preferences.rs`): C22's
//!   state follows the preference, live ([`follow_preference`]), and a keyboard peek turns it
//!   on while it is open, whatever the preference. Every close ends in [`ended`], which gives
//!   the state back to the preference. A drag starting or ending closes a peek, as GTK's
//!   `cancel_peek` in the item drag source's `drag-begin` and `drag-end` does
//!   (`src/ui/browser_modes/drag_drop.rs`), and a drag ending holds the collections' rest, so
//!   the folder it was dropped on never peeks until the pointer arrives on a row again.
//!   A menu, a popover or a modal layer over the views covers them: no row under it peeks.
//! - **Columns never peeks:** its `i` on a folder opens the folder's column.

use std::hash::{DefaultHasher, Hash, Hasher};

use rune::{El, Tx};
use strata_components::{
    collection::element::{Collection, RowMarks},
    contract::{EntryId, Rect},
    folder_peek::{
        Change, FolderPeek, More, Origin, PeekContents, PeekLabels, PeekState, PeekStatus,
    },
};
use strata_core::{
    i18n::tr,
    model::{FileEntry, Location},
};
use strata_fs::app::BrowserEvent;

use crate::{
    App,
    commands::Holds,
    icons::Icons,
    list::List,
    listing::{Column, Edit, Row},
    shell,
    single::SinglePane,
    slots::{Frame, Ids, Listener, Part, Place, Setup},
};

#[cfg(test)]
mod tests;

pub(crate) struct Peek {
    state: PeekState,
    /// The view a keyboard peek is open in, and its selection's revision as it opened.
    opened: Option<Opened>,
    title: String,
    status: PeekStatus,
    entries: Vec<FileEntry>,
    rows: Column,
    ids: Ids,
}

impl Part for Peek {
    const PLACE: Place<App> = Place::Overlays(build);

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            state: PeekState::new(),
            opened: None,
            title: String::new(),
            status: PeekStatus::Loading,
            entries: Vec::new(),
            rows: Column::new(Location::local("/"), true, None),
            ids: setup.ids.clone(),
        }
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, _, heard| {
            let event = match event {
                BrowserEvent::PeekEntriesAdded { entries } => Heard::Entries(entries.clone()),
                BrowserEvent::PeekFinished => Heard::Finished,
                BrowserEvent::PeekFailed { message } => Heard::Failed(message.clone()),
                BrowserEvent::PeekClosed => Heard::Closed,
                _ => return,
            };
            heard.then(move |app: &mut App, _: &mut Tx| hear(app, event));
        }))
    }

    /// Every settle comes here: after each command, and after each cursor or selection move the
    /// views hear (`single::moved`).
    fn publish(app: &mut App) {
        close_once_moved(app);
        follow_preference(app);
        follow_cover(app);
    }

    fn switched(app: &mut App, _: &mut Tx) {
        close(app);
    }

    fn tab_hidden(app: &mut App, _: &mut Tx) {
        close(app);
    }
}

#[derive(Clone, Copy)]
struct Opened {
    scope: &'static str,
    seen: Seen,
}

/// The selection of the view a peek is open in, as the user sees it.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Seen {
    /// Moves on with every change C1 reports, a listing's sync among them.
    revision: u64,
    /// A digest of the cursor and the selected entries.
    picked: u64,
}

/// The pane of the view keyed `scope`, while it shows its listing.
fn listing_pane<'a>(app: &'a App, scope: &str) -> Option<&'a crate::pane::Pane> {
    if app.filter.pane.shows() {
        return None;
    }
    match scope {
        Icons::KEY => Some(&Icons::single(app).pane),
        List::KEY => Some(&List::single(app).pane),
        _ => None,
    }
}

/// Whether the view keyed `scope` reads its folder again, a reload's: its rows go and come
/// back with the cursor and the selection they had.
fn relisting(app: &App, scope: &str) -> bool {
    listing_pane(app, scope)
        .and_then(crate::pane::Pane::listing)
        .is_some_and(crate::listing::Column::is_loading)
}

/// The selection of the view keyed `scope`, while it shows its listing.
fn seen(app: &App, scope: &str) -> Option<Seen> {
    let pane = listing_pane(app, scope)?;
    let selection = &pane.collection().selection;
    let mut picked = DefaultHasher::new();
    selection.cursor().hash(&mut picked);
    if let Some(listing) = pane.listing() {
        selection.selected_ids(listing).hash(&mut picked);
    }
    Some(Seen {
        revision: selection.revision(),
        picked: picked.finish(),
    })
}

/// Closes the peek once the cursor or the selection moved, or results replaced the listing. A
/// change that keeps both, a reload's sync, is no move: GTK's Reload keeps the peek.
fn close_once_moved(app: &mut App) -> bool {
    let Some(opened) = app.peek.opened else {
        return false;
    };
    if relisting(app, opened.scope) {
        return false;
    }
    match seen(app, opened.scope) {
        Some(now) if now == opened.seen => false,
        Some(now) if now.picked == opened.seen.picked => {
            app.peek.opened = Some(Opened {
                seen: now,
                ..opened
            });
            false
        }
        _ => close(app),
    }
}

/// The peek has gone, however it went: C22's state goes back to the preference's, which a
/// keyboard peek overrode while it was open.
fn ended(app: &mut App) {
    app.peek.opened = None;
    let on = pointer_peeking(app);
    app.peek.state.set_enabled(on);
}

/// Whether a resting pointer peeks: the `folder_peeking` preference, never in the chooser.
fn pointer_peeking(app: &App) -> bool {
    shell::preferences(app).folder_peeking && !app.browser.is_chooser_mode()
}

fn keyboard_peek_open(app: &App) -> bool {
    let target = app.peek.state.target();
    target.is_some_and(|target| target.origin == Origin::Keyboard)
}

/// Turns pointer peeking on or off as the preference changed, unless a keyboard peek holds it
/// on. A rest under way then opens nothing (C22's `hold_rest`), and turning it off closes a
/// pointer peek.
fn follow_preference(app: &mut App) {
    let on = pointer_peeking(app);
    if keyboard_peek_open(app) || app.peek.state.enabled() == on {
        return;
    }
    if app.peek.state.set_enabled(on) {
        app.browser.close_peek();
        app.peek.opened = None;
    }
    hold_rest(app);
}

/// Holds both views' rests until the pointer next arrives on a row.
pub(crate) fn hold_rest(app: &mut App) {
    Icons::single_mut(app).pane.parts_mut().1.hold_rest();
    List::single_mut(app).pane.parts_mut().1.hold_rest();
}

/// Rows under a menu, a popover or a modal layer start no peek (C22's `set_covered`): Rune
/// counts the rows under an overlay as hovered.
fn follow_cover(app: &mut App) {
    let covered = shell::parts(app).active().any(|part| {
        matches!(
            (part.keyboard)(app),
            Some(Holds::Menu | Holds::Popover | Holds::Modal { .. })
        )
    });
    if app.peek.state.covered() == covered {
        return;
    }
    if app.peek.state.set_covered(covered) == Change::Closed {
        app.browser.close_peek();
        ended(app);
    }
}

/// C22 opened a peek from a resting pointer: list its folder, as GTK's `schedule_peek` does
/// once the delay ran out with the row still hovered, unless the folder is open beside it.
fn pointer_opened(app: &mut App) {
    let Some(target) = app.peek.state.target().cloned() else {
        return;
    };
    if target.origin == Origin::Keyboard {
        return;
    }
    let found = scope_of(&target.collection).and_then(|scope| {
        let (depth, entry) = folder_of(app, scope, target.folder)?;
        Some((scope, depth, entry))
    });
    let Some((scope, depth, entry)) = found.filter(|(_, depth, entry)| {
        entry.is_directory() && !app.browser.is_open_child(*depth, &entry.location)
    }) else {
        close(app);
        return;
    };
    app.peek.opened = seen(app, scope).map(|seen| Opened { scope, seen });
    listen_to(app, depth, entry);
}

fn scope_of(collection: &str) -> Option<&'static str> {
    [Icons::KEY, List::KEY]
        .into_iter()
        .find(|key| *key == collection)
}

/// The entry `id` is in the view keyed `scope`, and the browser's column it shows.
fn folder_of(app: &App, scope: &str, id: EntryId) -> Option<(usize, FileEntry)> {
    let pane = match scope {
        Icons::KEY => &Icons::single(app).pane,
        _ => &List::single(app).pane,
    };
    let depth = pane.depth()?;
    let position = pane.position(depth, id)?;
    Some((depth, app.browser.entry_at(depth, position)?))
}

/// What the browser said of the peek's listing.
enum Heard {
    Entries(Vec<FileEntry>),
    Finished,
    Failed(String),
    Closed,
}

fn hear(app: &mut App, event: Heard) -> bool {
    if event_is_stale(app, &event) {
        return false;
    }
    match event {
        Heard::Entries(entries) => append(app, &entries),
        Heard::Finished => app.peek.status = PeekStatus::Ready,
        Heard::Failed(message) => app.peek.status = PeekStatus::Error(message),
        Heard::Closed => {
            app.peek.state.close();
            ended(app);
        }
    }
    true
}

/// A listing heard with no peek open in the views: a Columns peek's, which is not this part's.
fn event_is_stale(app: &App, event: &Heard) -> bool {
    app.peek.state.target().is_none() && !matches!(event, Heard::Closed)
}

fn append(app: &mut App, entries: &[FileEntry]) {
    let peek = &mut app.peek;
    peek.entries.extend_from_slice(entries);
    let rows = peek.entries.iter().map(|entry| Row::of(entry, &peek.ids));
    peek.rows.apply(Edit::Replace(rows.collect()));
}

fn labels() -> PeekLabels {
    let mut labels = PeekLabels::default();
    labels.peek = tr("Folder peek");
    labels.loading = tr("Loading");
    labels.empty = tr("This directory is empty");
    labels.unreadable = tr("Unable to read this directory");
    labels
}

fn peek() -> FolderPeek<App> {
    FolderPeek::new(
        |app: &App| &app.peek.state,
        |app: &mut App| &mut app.peek.state,
        |app: &App| {
            // GTK's card shows no "and N more" line.
            PeekContents::new(&app.peek.title, &app.peek.status, &app.peek.rows)
                .more(Some(More::Exactly(0)))
        },
        |app: &App| &app.look,
    )
    .labels(labels())
    .on_change(|app: &mut App, change, _| match change {
        Change::Closed => {
            app.browser.close_peek();
            ended(app);
        }
        Change::Opened => pointer_opened(app),
        Change::None => {}
    })
    .on_open(|app: &mut App, _, tx| {
        shell::publish(app);
        app.browser.commit_peek();
        shell::settle(app, tx);
    })
}

fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    peek().card(app, frame.layer);
    None
}

/// `base` with the anchor the card hangs from in its folder's row (Icons and List).
pub(crate) fn rows(base: Collection<App>) -> Collection<App> {
    peek().rows(base)
}

/// `marks` with the row a peek is open from marked, in collection `scope`.
pub(crate) fn with_marks(app: &App, scope: &str, id: EntryId, mut marks: RowMarks) -> RowMarks {
    marks.peek_anchor = app.peek.state.is_anchor(scope, id);
    marks
}

/// Closes an open peek, the card and the browser's listing: true when one was open, so
/// Escape stops there.
pub(crate) fn close(app: &mut App) -> bool {
    let card = app.peek.state.close();
    ended(app);
    app.browser.close_peek() | card
}

/// 10xer's `i` on the cursor's folder in Icons or List (`toggle_folder_peek`): closes an open
/// peek, whichever folder it is of, or else opens the folder's. False when the view shown is
/// neither, or the cursor is on no folder, so the key goes on to the file preview.
pub(crate) fn toggle_at_cursor(app: &mut App) -> bool {
    shell::publish(app);
    let Some((depth, _, entry)) = app.browser.focused_item() else {
        return false;
    };
    if !entry.is_directory() {
        return false;
    }
    let Some(spot) = cursor_spot(app, &entry) else {
        return false;
    };
    if app.peek.state.target().is_some() {
        close(app);
        return true;
    }
    app.peek.state.set_enabled(true);
    let change = app.peek.state.open(
        spot.scope,
        spot.id,
        Origin::Keyboard,
        spot.item,
        spot.viewport,
    );
    match change {
        Change::Opened => {
            app.peek.opened = seen(app, spot.scope).map(|seen| Opened {
                scope: spot.scope,
                seen,
            });
            listen_to(app, depth, entry);
        }
        _ => ended(app),
    }
    true
}

fn listen_to(app: &mut App, depth: usize, entry: FileEntry) {
    app.peek.title = entry.display_name.clone();
    app.peek.status = PeekStatus::Loading;
    app.peek.entries.clear();
    app.peek.rows = Column::new(entry.location.clone(), true, None);
    app.browser.begin_peek(depth, entry.location);
}

/// Where a keyboard peek opens: the view's key, the folder's id, its tile or row and the
/// viewport, in window points.
struct Spot {
    scope: &'static str,
    id: EntryId,
    item: Rect,
    viewport: Rect,
}

fn cursor_spot(app: &App, entry: &FileEntry) -> Option<Spot> {
    if app.filter.pane.shows() {
        return None;
    }
    let id = shell::id_of(app, &entry.location);
    if crate::tenxer::shows(app, "icons") {
        let collection = crate::icons::view::grid().collection(app);
        spot::<Icons>(app, &collection, id)
    } else if crate::tenxer::shows(app, "list") {
        let collection = crate::list::view::ticked(app);
        spot::<List>(app, &collection, id)
    } else {
        None
    }
}

fn spot<V: SinglePane>(app: &App, collection: &Collection<App>, id: EntryId) -> Option<Spot> {
    let pane = &V::single(app).pane;
    let index = pane.source().index_of(id)?;
    let viewport = peek().viewport(app, pane.collection().viewport());
    Some(Spot {
        scope: V::KEY,
        id,
        item: collection.window_rect_of(app, index),
        viewport,
    })
}
