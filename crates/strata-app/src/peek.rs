// SPDX-License-Identifier: MIT

//! The folder peek (C22's [`FolderPeek`]) in Icons and List: 10xer's `i` on a folder opens a
//! card of its first rows beside it, and `i` again, Escape, navigating, a view or tab switch
//! close it (`toggle_folder_peek` and `open_keyboard_peek` in `src/ui/browser/view_modes.rs`
//! and `src/ui/browser/peek.rs`).
//!
//! - **The browser lists it.** As GTK's does, the peek asks the shared browser
//!   (`Browser::begin_peek`) and hears its `Peek*` events, so a close from anywhere
//!   (`Browser::close_peek`: Escape, a drag, a navigation) closes the card too.
//! - **Keyboard peeks only, for now.** C22's state opens nothing while it is off, and peeking
//!   from a resting pointer is the `folder_peeking` preference's, which [A19](../../../docs/rune/parity/packages/A19.md)
//!   wires with the drag rules. Here the state is on while a keyboard peek is open, and off
//!   again as it closes.
//! - **Columns never peeks:** its `i` on a folder opens the folder's column.

use rune::{El, Tx};
use strata_components::{
    collection::element::{Collection, RowMarks},
    contract::{EntryId, Rect},
    folder_peek::{Change, FolderPeek, More, PeekContents, PeekLabels, PeekState, PeekStatus},
};
use strata_core::{
    i18n::tr,
    model::{FileEntry, Location},
};
use strata_fs::app::BrowserEvent;

use crate::{
    App,
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

    fn switched(app: &mut App, _: &mut Tx) {
        close(app);
    }

    fn tab_hidden(app: &mut App, _: &mut Tx) {
        close(app);
    }
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
            app.peek.state.set_enabled(false);
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
    .on_change(|app: &mut App, change, _| {
        if change == Change::Closed {
            app.browser.close_peek();
            app.peek.state.set_enabled(false);
        }
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
    app.peek.state.set_enabled(false);
    app.browser.close_peek() | card
}

/// 10xer's `i` on the cursor's folder in Icons or List: opens its peek, or closes the one open
/// from it (`toggle_folder_peek`). False when the view shown is neither, or the cursor is on no
/// folder, so the key goes on to the file preview.
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
    app.peek.state.set_enabled(true);
    let change = app
        .peek
        .state
        .toggle(spot.scope, spot.id, spot.item, spot.viewport);
    match change {
        Change::Opened => listen_to(app, depth, entry),
        Change::Closed => {
            close(app);
        }
        Change::None => {
            app.peek.state.set_enabled(false);
        }
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
