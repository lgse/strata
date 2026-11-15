// SPDX-License-Identifier: MIT

//! The window's clipboard (`src/ui/browser/clipboard.rs`): copied and cut files in the three
//! Linux file-manager formats (`strata_platform::clipboard::Files`), copied paths and names,
//! the copy and cut marks on rows across every view and tab (`Scope::Window`), and clearing or
//! updating a cut clipboard once its items move (`complete_cut_transfer`).
//!
//! On the windowed host `strata_platform::clipboard::Clipboard` binds to Wayland or X11. On
//! the headless offscreen host no native window is attached, so each `App` keeps its own
//! in-memory fallback (`Fallback`) isolated from other tests.

use std::collections::HashSet;

use rune::{AppHandle, El, Tx, WindowHandle};
use strata_components::{collection::element::RowMarks, contract::EntryId};
use strata_core::model::{FileEntry, Location};
use strata_desktop::keys::{
    Modifiers,
    bindings::{COPY, COPY_PATH, CUT},
};
use strata_platform::clipboard::{
    self as platform, Change, Clipboard as PlatformClipboard, Content, CopyId, Error, Files,
    Limits, Selection,
};

use crate::{
    App,
    commands::{Action, Call, Keys, On, Outcome, Stage},
    filter, menus, shell,
    slots::{Frame, Ids, Part, Place, Scope, Setup},
};

pub(crate) mod paths;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Fallback {
    #[default]
    Empty,
    Files(Files),
    Text(String),
}

pub(crate) struct Clipboard {
    ids: Ids,
    handle: PlatformClipboard<App>,
    owned: Option<CopyId>,
    has_files: bool,
    read_revision: u64,
    offer_serial: u64,
    pending_read: Option<u64>,
    fallback: Fallback,
    copied: Vec<Location>,
    cut: Vec<Location>,
    copied_ids: HashSet<EntryId>,
    cut_ids: HashSet<EntryId>,
}

fn nothing(_: &App, _: &mut Frame<'_, App>) -> Option<El> {
    None
}

impl Part for Clipboard {
    const PLACE: Place<App> = Place::Overlays(nothing);
    const SCOPE: Scope = Scope::Window;

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            ids: setup.ids.clone(),
            handle: PlatformClipboard::new().on_change(changed),
            owned: None,
            has_files: false,
            read_revision: 0,
            offer_serial: 0,
            pending_read: None,
            fallback: Fallback::Empty,
            copied: Vec::new(),
            cut: Vec::new(),
            copied_ids: HashSet::new(),
            cut_ids: HashSet::new(),
        }
    }

    const COMMANDS: &'static [Action<App>] = &[
        Action::new("copy", copied).command(COPY).keys(&[Keys::at(
            Stage::FileCommands,
            &[COPY.ignoring(Modifiers::empty())],
        )
        .when(|_, at| !at.text())]),
        Action::new("cut", cut).command(CUT).keys(&[Keys::at(
            Stage::FileCommands,
            &[CUT.ignoring(Modifiers::empty())],
        )
        .when(|_, at| !at.text())]),
        Action::new("copy-path", copy_path)
            .command(COPY_PATH)
            .keys(&[Keys::at(
                Stage::ItemNavigation,
                &[COPY_PATH.ignoring(Modifiers::empty())],
            )
            .when(|app, at| !at.text() && filter::targets::selected(app).is_none())]),
        Action::new("copy-paths", copy_path).command(COPY_PATH),
        Action::new("copy-name", copy_name),
        Action::new("copy-names", copy_name),
    ];
}

impl Clipboard {
    fn clear_marks(&mut self) {
        self.copied.clear();
        self.cut.clear();
        self.copied_ids.clear();
        self.cut_ids.clear();
    }

    fn set_marks(&mut self, copied: Vec<Location>, cut: Vec<Location>) {
        self.copied_ids = copied.iter().map(|loc| self.ids.of(loc)).collect();
        self.cut_ids = cut.iter().map(|loc| self.ids.of(loc)).collect();
        self.copied = copied;
        self.cut = cut;
    }

    fn bump_revision(&mut self) -> u64 {
        self.pending_read = None;
        self.read_revision = self.read_revision.wrapping_add(1);
        self.read_revision
    }

    fn forget_owned(&mut self) -> u64 {
        self.owned = None;
        self.clear_marks();
        self.has_files = false;
        self.bump_revision()
    }

    fn apply_offered_read(&mut self, revision: u64, has_files: bool) {
        if self.read_revision == revision {
            self.has_files = has_files;
            self.pending_read = None;
        }
    }

    fn write_files(&mut self, locations: &[Location], cut: bool) -> bool {
        let Some((files, text)) = paths::files_for_locations(locations, cut) else {
            return false;
        };
        let content = files.add_to(Content::new().text(&text));
        match self.handle.copy(Selection::Clipboard, content) {
            Ok(copy_id) => {
                self.owned = Some(copy_id);
                self.has_files = true;
                self.bump_revision();
                true
            }
            Err(Error::NotAttached) => {
                self.owned = None;
                self.fallback = Fallback::Files(files);
                self.has_files = true;
                self.bump_revision();
                true
            }
            Err(error) => {
                tracing::warn!(%error, "unable to write files to the clipboard");
                false
            }
        }
    }

    pub(crate) fn write_text(&mut self, text: &str) {
        self.clear_marks();
        match self
            .handle
            .copy(Selection::Clipboard, Content::new().text(text))
        {
            Ok(copy_id) => {
                self.owned = Some(copy_id);
                self.has_files = false;
                self.bump_revision();
            }
            Err(Error::NotAttached) => {
                self.owned = None;
                self.fallback = Fallback::Text(text.to_owned());
                self.has_files = false;
                self.bump_revision();
            }
            Err(error) => tracing::warn!(%error, "unable to write text to the clipboard"),
        }
    }
}

/// Binds the windowed host's `PlatformClipboard` handle to `app` before `on_window` attaches it.
pub(crate) fn bind(app: &mut App, handle: PlatformClipboard<App>) {
    app.clipboard.handle = handle;
}

/// Attaches `handle` to `window` on the windowed host.
pub(crate) fn attach(handle: &PlatformClipboard<App>, window: &WindowHandle, app: &AppHandle<App>) {
    if let Err(error) = handle.attach(window, app) {
        tracing::warn!(%error, "unable to attach the system clipboard");
    }
}

/// Tears down the clipboard and X11 hand-off before the event loop ends.
pub(crate) fn shutdown() {
    if let Err(error) = platform::shutdown() {
        tracing::warn!(%error, "unable to shut down the clipboard");
    }
}

/// Handles selection ownership and offer notifications from `strata_platform::clipboard`.
pub(crate) fn changed(app: &mut App, change: &Change, _: &mut Tx) {
    match change {
        Change::Offered {
            selection: Selection::Clipboard,
            mimes,
        } => {
            let revision = app.clipboard.forget_owned();
            app.clipboard.offer_serial = app.clipboard.offer_serial.wrapping_add(1);
            if Files::offered(mimes) {
                app.clipboard.pending_read = Some(revision);
                if app
                    .clipboard
                    .handle
                    .read_files(
                        Selection::Clipboard,
                        Limits::default(),
                        move |app, result, _| {
                            let any = result.ok().is_some_and(|files| {
                                !paths::locations_from_files(&files).is_empty()
                            });
                            app.clipboard.apply_offered_read(revision, any);
                        },
                    )
                    .is_err()
                {
                    app.clipboard.pending_read = None;
                }
            }
        }
        Change::Emptied {
            selection: Selection::Clipboard,
        } => {
            app.clipboard.forget_owned();
            app.clipboard.offer_serial = app.clipboard.offer_serial.wrapping_add(1);
        }
        Change::Lost {
            selection: Selection::Clipboard,
            copy,
            ..
        } if app.clipboard.owned == Some(*copy) => {
            app.clipboard.forget_owned();
        }
        _ => {}
    }
}

#[cfg(any(test, feature = "qa"))]
pub(crate) fn offer_serial(app: &App) -> u64 {
    app.clipboard.offer_serial
}

#[cfg(any(test, feature = "qa"))]
pub(crate) fn settled_since(app: &App, before: u64) -> bool {
    app.clipboard.offer_serial > before && app.clipboard.pending_read.is_none()
}

/// Whether the clipboard currently holds file URIs.
pub(crate) fn has_files(app: &App) -> bool {
    app.clipboard.has_files
}

#[cfg(test)]
pub(crate) fn fallback_text(app: &App) -> Option<&str> {
    match &app.clipboard.fallback {
        Fallback::Text(text) => Some(text.as_str()),
        Fallback::Empty | Fallback::Files(_) => None,
    }
}

#[cfg(test)]
pub(crate) fn write_test_files(app: &mut App, locations: &[Location]) {
    let _ = app.clipboard.write_files(locations, false);
}

/// Whether Paste should be enabled for `location`.
pub(crate) fn may_paste(app: &App, location: &Location) -> bool {
    !paths::is_trash(location) && !location.is_recent_location() && has_files(app)
}

/// Copy and cut marks for entry `id`.
pub(crate) fn marks(app: &App, id: EntryId) -> RowMarks {
    with_marks(app, id, RowMarks::default())
}

/// Adds copy and cut marks for entry `id` onto `marks`.
pub(crate) fn with_marks(app: &App, id: EntryId, mut marks: RowMarks) -> RowMarks {
    marks.cut = app.clipboard.cut_ids.contains(&id);
    marks.copied = !marks.cut && app.clipboard.copied_ids.contains(&id);
    marks
}

/// Whether `sources` matches the window's active cut selection.
pub(crate) fn is_cut_match(app: &App, sources: &[Location]) -> bool {
    paths::same_locations(sources, &app.clipboard.cut)
}

/// Consumes moved cut items after a transfer finishes (`ViewState::complete_cut_transfer`).
pub(crate) fn complete_cut_transfer(app: &mut App, transferred: &[Location]) {
    let consumed = app.clipboard.cut.iter().any(|cut| {
        transferred
            .iter()
            .any(|moved| paths::locations_equal(cut, moved))
    });
    if !consumed {
        return;
    }
    let mut remaining = std::mem::take(&mut app.clipboard.cut);
    paths::retain_untransferred(&mut remaining, transferred);
    app.clipboard.set_marks(Vec::new(), remaining.clone());
    if let Some(copy_id) = app.clipboard.owned {
        if remaining.is_empty() {
            if app.clipboard.handle.clear(copy_id).unwrap_or(false) {
                app.clipboard.owned = None;
                app.clipboard.has_files = false;
                app.clipboard.bump_revision();
            }
        } else {
            let _ = app.clipboard.write_files(&remaining, true);
        }
    } else if matches!(app.clipboard.fallback, Fallback::Files(_)) {
        if remaining.is_empty() {
            app.clipboard.fallback = Fallback::Empty;
            app.clipboard.has_files = false;
            app.clipboard.bump_revision();
        } else {
            let _ = app.clipboard.write_files(&remaining, true);
        }
    }
}

/// Reads file URIs and their cut marker from the clipboard, calling `deliver` on the UI thread.
pub(crate) fn read_files(
    app: &mut App,
    deliver: impl FnOnce(&mut App, Option<(Vec<Location>, bool)>, &mut Tx) + 'static,
    tx: &mut Tx,
) {
    let decode = |files: &Files| {
        let sources = paths::locations_from_files(files);
        (!sources.is_empty()).then_some((sources, files.cut))
    };
    let slot = std::rc::Rc::new(std::cell::RefCell::new(Some(deliver)));
    let slot_async = slot.clone();
    let read = app.clipboard.handle.read_files(
        Selection::Clipboard,
        Limits::default(),
        move |app, result, tx| {
            if let Some(cb) = slot_async.borrow_mut().take() {
                let decoded = result.ok().and_then(|files| decode(&files));
                cb(app, decoded, tx);
            }
        },
    );
    if matches!(read, Err(Error::NotAttached))
        && let Some(cb) = slot.borrow_mut().take()
    {
        let decoded = match &app.clipboard.fallback {
            Fallback::Files(files) => decode(files),
            Fallback::Empty | Fallback::Text(_) => None,
        };
        cb(app, decoded, tx);
    }
}

fn target_entries(app: &App, on: &On) -> Vec<FileEntry> {
    match *on {
        On::Row((depth, id)) => menus::context_entries(app, depth, id)
            .map(|(_, entries)| entries)
            .unwrap_or_default(),
        On::Focus => {
            filter::targets::selected(app).unwrap_or_else(|| app.browser.transfer_entries())
        }
        On::Column(_) | On::Place(_) => Vec::new(),
    }
}

fn copy_entries(app: &mut App, entries: &[FileEntry]) -> bool {
    let locations: Vec<Location> = entries.iter().map(|e| e.location.clone()).collect();
    if app.clipboard.write_files(&locations, false) {
        app.clipboard.set_marks(locations, Vec::new());
        return true;
    }
    false
}

fn cut_entries(app: &mut App, entries: &[FileEntry]) -> bool {
    if entries.is_empty() || entries.iter().any(|e| !paths::can_remove(&e.location)) {
        return false;
    }
    let locations: Vec<Location> = entries.iter().map(|e| e.location.clone()).collect();
    if app.clipboard.write_files(&locations, true) {
        app.clipboard.set_marks(Vec::new(), locations);
        return true;
    }
    false
}

fn copied(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    let entries = target_entries(app, &call.on);
    if entries.is_empty() {
        return match call.on {
            On::Focus => Outcome::Declined,
            _ => Outcome::Ran,
        };
    }
    copy_entries(app, &entries);
    Outcome::Ran
}

fn cut(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    let entries = target_entries(app, &call.on);
    if !cut_entries(app, &entries) && matches!(call.on, On::Focus) {
        return Outcome::Declined;
    }
    Outcome::Ran
}

fn copy_path_allowed_where_focused(app: &App, call: &Call<'_>, tx: &Tx) -> bool {
    let Some(press) = call.press else {
        return true;
    };
    if press
        .modifiers()
        .intersects(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK)
    {
        return true;
    }
    tx.focused() == shell::view_focus(app)
}

fn copy_path_entries(app: &App) -> Vec<FileEntry> {
    if let Some(selected) = filter::targets::selected(app) {
        if !selected.is_empty() {
            return selected;
        }
        return filter::targets::under_cursor(app)
            .map(|(_, entry)| entry)
            .into_iter()
            .collect();
    }
    let selected = app.browser.selected_entries();
    if !selected.is_empty() {
        return selected;
    }
    app.browser.focused_entry().into_iter().collect()
}

fn copy_path(app: &mut App, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    if !copy_path_allowed_where_focused(app, call, tx) {
        return Outcome::Declined;
    }
    match &call.on {
        On::Place(location) => {
            let text = paths::copy_path_text(location, true);
            app.clipboard.write_text(&text);
            Outcome::Ran
        }
        On::Row((depth, id)) => {
            if let Some((_, entries)) = menus::context_entries(app, *depth, *id) {
                copy_locations_text(app, &entries);
            }
            Outcome::Ran
        }
        On::Focus => {
            let entries = copy_path_entries(app);
            if entries.is_empty() {
                return Outcome::Declined;
            }
            copy_locations_text(app, &entries);
            Outcome::Ran
        }
        On::Column(_) => Outcome::Declined,
    }
}

fn copy_locations_text(app: &mut App, entries: &[FileEntry]) {
    let text = entries
        .iter()
        .map(|entry| paths::copy_path_text(&entry.location, entry.is_directory()))
        .collect::<Vec<_>>()
        .join("\n");
    app.clipboard.write_text(&text);
}

fn copy_name(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    let On::Row((depth, id)) = call.on else {
        return Outcome::Declined;
    };
    if let Some((_, entries)) = menus::context_entries(app, depth, id) {
        let text = entries
            .iter()
            .map(|entry| entry.display_name.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        app.clipboard.write_text(&text);
    }
    Outcome::Ran
}
