// SPDX-License-Identifier: MIT

//! 10xer mode (A11): modal keyboard navigation, footer indicators, chord prefix dispatch and
//! footer prompts, ported from `src/ui/tenxer_mode.rs`, `src/ui/window/keyboard/tenxer.rs` and
//! `src/ui/shortcut_footer/`.

use rune::{Cx, El, Tx};
use strata_components::{
    collection::keyboard::{Command as Nav, KeyHandle, RangeKind},
    filter_bar::{
        Flash,
        chord::{self, ChordMap, ChordOption, ChordState},
        flash,
        prompt::PromptState,
    },
};
use strata_core::{i18n::tr, model::FileEntry};
use strata_desktop::keys::{
    self as key_catalog, Command as KeyCommand, Keys, Modifiers, bindings, shown,
};

use crate::{
    App,
    commands::{Action, Holds},
    filter, shell,
    slots::{Frame, Later, Listener, Part, Place, Setup},
};

mod bookmarks;
mod chords;
mod go;
pub(crate) mod highlight;
mod keys;
mod preview_keys;
mod prompts;
#[cfg(test)]
mod tests;
mod verbs;

pub(crate) use prompts::{PromptKind, finish_filter_descent};

const FLASH_KEY: &str = "footer-flash";
const CHORD_KEY: &str = "footer-chord";
const PROMPT_KEY: &str = "prompt";

/// A pending two-key command (`Chord` in `src/ui/tenxer_mode.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChordKind {
    Go,
    Copy,
    Sort,
    Action,
    FolderAction,
    Tabs,
    BookmarkSet,
    BookmarkGo,
    /// `g` in a focused document or archive preview: `g` again goes to its top.
    PreviewTop,
}

impl ChordKind {
    pub(crate) fn mark(self) -> String {
        let prefix = match self {
            Self::Go | Self::PreviewTop => Keys::One(bindings::GO.shown()),
            Self::Copy => Keys::One(bindings::COPY_AS.shown()),
            Self::Sort => Keys::One(bindings::SORT_BY.shown()),
            Self::Action => Keys::One(bindings::ACTIONS.shown()),
            Self::FolderAction => Keys::Chord(shown![bindings::ACTIONS, bindings::ACTIONS_FOLDER]),
            Self::Tabs => Keys::One(bindings::TAB_CHORD.shown()),
            Self::BookmarkSet => Keys::One(bindings::BOOKMARK_SET.shown()),
            Self::BookmarkGo => Keys::One(bindings::BOOKMARK_GO.shown()),
        };
        format!("{}-", key_catalog::label(&prefix))
    }

    pub(crate) fn options(self) -> Vec<(String, String)> {
        let step = |cmd: KeyCommand, action: &str| {
            (key_catalog::label(&Keys::One(cmd.shown())), tr(action))
        };
        match self {
            Self::Go => go_options("First item"),
            Self::PreviewTop => go_options("Top"),
            Self::Copy => vec![
                step(bindings::COPY_AS_PATH, "Copy path"),
                step(bindings::COPY_AS_NAME, "Copy name"),
            ],
            Self::Sort => sort_options(),
            Self::Action | Self::FolderAction | Self::BookmarkGo => Vec::new(),
            Self::BookmarkSet => vec![(
                key_catalog::label(&Keys::Letters(bindings::BOOKMARK_SET_LETTERS.step())),
                tr("Save bookmark"),
            )],
            Self::Tabs => vec![
                step(bindings::TAB_CHORD_NEW, "New tab"),
                step(bindings::TAB_CHORD_CLOSE, "Close tab"),
                step(bindings::TAB_CHORD_PREVIOUS, "Previous tab"),
                (
                    key_catalog::label(&Keys::Digits(bindings::TAB_CHORD_NUMBERS.step())),
                    tr("Select tab"),
                ),
            ],
        }
    }
}

fn go_options(first: &str) -> Vec<(String, String)> {
    let one = |cmd: KeyCommand| Keys::One(cmd.shown());
    let rows = [
        (one(bindings::GO_FIRST), first),
        (one(bindings::GO_FOLLOW), "Follow search result"),
        (one(bindings::GO_HOME), "Home"),
        (one(bindings::GO_DOWNLOADS), "Downloads"),
        (one(bindings::GO_CONFIG), "Config"),
        (one(bindings::GO_TRASH), "Trash"),
        (one(bindings::GO_NETWORK), "Network"),
        (one(bindings::GO_RECENT), "Recent"),
        (one(bindings::GO_DOCUMENTS), "Documents"),
        (one(bindings::GO_MUSIC), "Music"),
        (one(bindings::GO_PICTURES), "Pictures"),
        (one(bindings::GO_VIDEOS), "Videos"),
        (Keys::Digits(bindings::PIN_NUMBERS.step()), "Pins"),
        (
            Keys::Any(shown![bindings::GO_PIN, bindings::GO_UNPIN]),
            "Pin / unpin folder",
        ),
        (one(bindings::GO_PROMPT), "Type a path"),
    ];
    rows.into_iter()
        .map(|(keys, action)| (key_catalog::label(&keys), tr(action)))
        .collect()
}

fn sort_options() -> Vec<(String, String)> {
    let steps = [
        (bindings::SORT_BY_NAME, "Name"),
        (bindings::SORT_BY_MODIFIED, "Modified"),
        (bindings::SORT_BY_SIZE, "Size"),
        (bindings::SORT_BY_TYPE, "Type"),
        (bindings::SORT_BY_EXTENSION, "Extension"),
        (bindings::SORT_BY_CREATED, "Created"),
        (bindings::SORT_BY_UNSORTED, "Unsorted"),
        (bindings::SORT_BY_NATURAL, "Natural"),
        (bindings::SORT_BY_ALPHABETICAL, "Alphabetical"),
        (bindings::SORT_BY_CASE_SENSITIVE, "Case-sensitive names"),
        (bindings::SORT_BY_FOLDERS_FIRST, "Folders first"),
    ];
    let mut rows: Vec<_> = steps
        .into_iter()
        .map(|(cmd, action)| (key_catalog::label(&Keys::One(cmd.shown())), tr(action)))
        .collect();
    let shift = key_catalog::modifiers_label_in(&rust_i18n::locale(), Modifiers::SHIFT_MASK);
    rows.push((shift, tr("Reverse")));
    rows
}

pub(crate) struct Tenxer {
    pub(crate) flash: Flash,
    pub(crate) chord: ChordState<()>,
    pub(crate) armed: Option<ChordKind>,
    /// What `;` or `; .` listed as it armed, which its digit is checked against.
    pub(crate) armed_actions: Option<chords::ArmedActions>,
    pub(crate) prompt: PromptState<prompts::PromptKind>,
    pub(crate) prompt_state: prompts::State,
    pub(crate) later: Later<App>,
    pub(crate) bookmark_revision: u64,
    pub(crate) bookmark_pending: bool,
    was_enabled: bool,
}

impl Part for Tenxer {
    const PLACE: Place<App> = Place::Overlays(build);

    const COMMANDS: &'static [Action<App>] = keys::COMMANDS;

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            flash: Flash::new(),
            chord: ChordState::new(),
            armed: None,
            armed_actions: None,
            prompt: PromptState::new(),
            prompt_state: prompts::State::default(),
            later: setup.later.clone(),
            bookmark_revision: 0,
            bookmark_pending: false,
            was_enabled: setup.settings.tenxer_mode,
        }
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, browser, heard| {
            if bookmarks::cancels_pending_bookmark(browser, event) {
                heard.then(|app: &mut App, _: &mut Tx| {
                    app.tenxer.bookmark_pending && cancel_chord(app)
                });
            }
        }))
    }

    fn keyboard(app: &App) -> Option<Holds> {
        app.tenxer.prompt.focused().then_some(Holds::Prompt)
    }

    fn tab_hidden(app: &mut App, _: &mut Tx) {
        cancel_chord(app);
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        let enabled = shell::preferences(app).tenxer_mode;
        let mut changed = false;
        if app.tenxer.was_enabled && !enabled {
            end_session(app, tx);
            changed = true;
        }
        app.tenxer.was_enabled = enabled;
        let reference_open = app.shortcuts.reference.is_open();
        app.tenxer.prompt.hold_open(reference_open);
        if reference_open {
            changed |= cancel_chord(app);
        }
        let polled = app.tenxer.prompt.tick();
        changed |= polled.changed;
        prompts::prompt_builder(app).route(app, polled, tx);
        changed |= app.tenxer.flash.tick(clock, tx);
        changed
    }
}

fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let has_options = app
        .tenxer
        .chord
        .armed()
        .is_some_and(|map| !map.options.is_empty());
    if has_options && let Some(panel) = chord::panel(&app.look, CHORD_KEY, &app.tenxer.chord) {
        frame.layer.push(panel);
    }
    None
}

/// Cancels any armed chord on `app`.
pub(crate) fn cancel_chord(app: &mut App) -> bool {
    if std::mem::replace(&mut app.tenxer.bookmark_pending, false) {
        app.tenxer.bookmark_revision = app.tenxer.bookmark_revision.wrapping_add(1);
    }
    crate::sidebar::show_keycaps(app, false);
    app.tenxer.armed = None;
    app.tenxer.chord.clear()
}

/// Arms `kind` with its default catalog options.
pub(crate) fn arm_default_chord(app: &mut App, kind: ChordKind) {
    let options = kind.options();
    arm_chord(app, kind, options);
}

/// Arms `kind` with `rows` (`(keycap, label)`), filtering out unbound keycaps.
pub(crate) fn arm_chord(app: &mut App, kind: ChordKind, rows: Vec<(String, String)>) {
    if kind != ChordKind::BookmarkGo && std::mem::replace(&mut app.tenxer.bookmark_pending, false) {
        app.tenxer.bookmark_revision = app.tenxer.bookmark_revision.wrapping_add(1);
    }
    crate::sidebar::show_keycaps(app, kind == ChordKind::Go);
    let options = rows
        .into_iter()
        .filter(|(cap, _)| !cap.is_empty())
        .map(|(cap, label)| ChordOption::mention(cap, label))
        .collect();
    let mark = kind.mark();
    app.tenxer.armed = Some(kind);
    app.tenxer.chord.arm(ChordMap {
        name: mark.clone(),
        mark,
        options,
    });
}

/// Ends an active 10xer session when leaving 10xer mode (`end_tenxer_session`).
pub(crate) fn end_session(app: &mut App, tx: &mut Tx) {
    cancel_chord(app);
    crate::open_with::cancel_target_lookup(app);
    app.tenxer.prompt_state.reset_all();
    let prompt_had_focus = app.tenxer.prompt.focused();
    let polled = app.tenxer.prompt.close();
    prompts::prompt_builder(app).route(app, polled, tx);
    filter::end_tenxer_filters(app, tx);
    end_all_ranges(app);
    if prompt_had_focus && let Some(view) = shell::view_focus(app) {
        tx.focus_when_shown(view, true);
    }
}

/// Shows `"Nothing to select"` in the footer flash.
pub(crate) fn flash_nothing_to_select(app: &mut App, tx: &mut Tx) {
    app.tenxer.flash.show(tr("Nothing to select"), tx);
}

/// Shows `message` in the footer flash.
pub(crate) fn show_flash(app: &mut App, message: String, tx: &mut Tx) {
    app.tenxer.flash.show(message, tx);
}

pub(crate) fn shows(app: &App, mode: &str) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)(mode))
}

/// `BrowserView::focused_listing_depth` (`src/ui/browser/view_selection.rs`).
pub(crate) fn focused_listing_depth(app: &App) -> Option<usize> {
    if shows(app, "columns") {
        Some(app.columns.active_depth())
            .filter(|depth| app.browser.location_at(*depth).is_some())
            .or_else(|| app.browser.active_depth())
    } else {
        app.browser.active_depth()
    }
}

/// `BrowserView::command_targets` (`src/ui/browser/file_commands.rs`).
pub(crate) fn command_targets(app: &mut App) -> Vec<FileEntry> {
    let tenxer = shell::preferences(app).tenxer_mode;
    if let Some(results) = filter::targets::selected(app) {
        if !results.is_empty() || !tenxer {
            return results;
        }
        return filter::targets::under_cursor(app)
            .map(|(_, entry)| entry)
            .into_iter()
            .collect();
    }
    shell::publish(app);
    if !tenxer {
        return app.browser.transfer_entries();
    }
    focused_listing_depth(app)
        .map(|depth| app.browser.command_entries(depth))
        .unwrap_or_default()
}

/// `BrowserView::focused_target` (`src/ui/browser/file_commands.rs`).
pub(crate) fn focused_target(app: &mut App) -> Option<FileEntry> {
    if filter::targets::selected(app).is_some() {
        return filter::targets::preview_result(app)
            .or_else(|| filter::targets::under_cursor(app))
            .map(|(_, entry)| entry);
    }
    shell::publish(app);
    focused_listing_depth(app).and_then(|depth| app.browser.cursor_entry(depth))
}

/// The `KeyHandle` of whichever collection currently shows.
pub(crate) fn active_key_handle(app: &App) -> Option<KeyHandle> {
    if app.filter.pane.shows() {
        return Some(app.filter.pane.keys_handle.clone());
    }
    let view = shell::shown_view(app)?;
    if (view.shows)("columns") {
        app.columns.active_key_handle()
    } else if (view.shows)("icons") {
        Some(app.icons.single.keys_handle.clone())
    } else if (view.shows)("list") {
        Some(app.list.single.keys_handle.clone())
    } else {
        None
    }
}

/// Queues `command` onto the shown collection's [`KeyHandle`] and ticks that collection at
/// once, so `commands::call`'s `settle` publishes the updated cursor and selection in the
/// same frame.
pub(crate) fn run_collection_command(app: &mut App, command: Nav, tx: &mut Tx) -> bool {
    let Some(handle) = active_key_handle(app) else {
        return false;
    };
    handle.run(command);
    tick_shown_collection(app, tx);
    true
}

fn tick_shown_collection(app: &mut App, tx: &mut Tx) {
    let Some(view) = shell::shown_view(app) else {
        return;
    };
    if (view.shows)("columns") {
        let depth = app.columns.active_depth();
        if let Some(collection) = crate::columns::view(app).column_collection(app, depth) {
            collection.tick(app, tx);
        }
    } else if (view.shows)("icons") {
        crate::icons::tick_active(app, tx);
    } else if (view.shows)("list") {
        crate::list::tick_active(app, tx);
    }
}

/// The active `VISUAL` or `UNSET` range for the footer.
pub(crate) fn active_range(app: &App) -> Option<RangeKind> {
    if !shell::preferences(app).tenxer_mode {
        return None;
    }
    active_key_handle(app)?.range()
}

/// Ends any active visual range on the currently shown results or listing.
pub(crate) fn end_active_range(app: &App) -> bool {
    active_key_handle(app).is_some_and(|handle| handle.end_range())
}

/// Ends any active visual range across every view and filter pane.
pub(crate) fn end_all_ranges(app: &App) -> bool {
    app.filter.pane.keys_handle.end_range()
        | app.columns.end_ranges()
        | app.icons.single.keys_handle.end_range()
        | app.list.single.keys_handle.end_range()
}

pub(crate) fn flash_el(app: &App) -> Option<El> {
    flash(&app.look, FLASH_KEY, &app.tenxer.flash)
}

pub(crate) fn chord_pill_el(app: &App) -> Option<El> {
    chord::pill(&app.look, CHORD_KEY, &app.tenxer.chord)
}

pub(crate) fn prompt_el(app: &App, cx: &mut Cx<App>, layer: &mut Vec<El>) -> Option<El> {
    prompts::prompt_builder(app).build(app, &app.look, cx, layer)
}
