// SPDX-License-Identifier: MIT

//! What the shortcut reference lists in the default map: GTK's tables, ported from
//! `src/ui/shortcut_reference.rs` with the rows, their order and their words unchanged. Each
//! table and function keeps the name of the GTK item it copies. A change to one of them on `main` comes here by
//! hand: `tests.rs` holds every listed catalog command to a row, as GTK's
//! `every_listed_binding_has_its_own_row_in_its_maps` does, so a command added to the catalog
//! fails here until it has one. The 10xer map's sections are [A11]'s, with its mode.
//!
//! [A11]: ../../../../docs/rune/parity/packages/A11.md

use strata_desktop::keys::{
    Keys::{self, Any, One},
    Modifiers,
    bindings::*,
    shown,
};

/// The three views, as `BrowserMode` names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Mode {
    Columns,
    Icons,
    List,
}

/// `ReferenceSection`.
pub(crate) struct Section {
    pub(crate) title: &'static str,
    pub(crate) rows: Vec<Shortcut>,
}

/// `Shortcut`: a row's keys, the template that places them, its action and what the action
/// names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Shortcut {
    pub(crate) keys: Keys,
    /// A translatable template that places `%{keys}` in context.
    pub(crate) context: Option<&'static str>,
    pub(crate) action: &'static str,
    /// What the action's own `%{keys}` names.
    pub(crate) mention: Option<Mention>,
    /// The action's own `%{number}`.
    pub(crate) number: Option<usize>,
}

/// `Mention`: keys named inside action text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Mention {
    Keys(Keys),
    /// Modifiers held with the keys, such as the **Shift** that reverses a sort.
    Held(Modifiers),
}

impl Shortcut {
    const fn mentioning(self, mention: Mention) -> Self {
        Self {
            mention: Some(mention),
            ..self
        }
    }
}

const fn row(keys: Keys, action: &'static str) -> Shortcut {
    Shortcut {
        keys,
        context: None,
        action,
        mention: None,
        number: None,
    }
}

const fn within(context: &'static str, keys: Keys, action: &'static str) -> Shortcut {
    Shortcut {
        context: Some(context),
        ..row(keys, action)
    }
}

/// `reference_sections` for the window in the default map: the rows the active keymap binds,
/// in the sections that keep one.
pub(crate) fn sections(mode: Mode) -> Vec<Section> {
    let mut sections = default_sections(mode);
    for section in &mut sections {
        section.rows.retain(|row| row.keys.is_bound());
    }
    sections.retain(|section| !section.rows.is_empty());
    sections
}

/// `default_sections`.
pub(crate) fn default_sections(mode: Mode) -> Vec<Section> {
    vec![
        Section {
            title: navigation_title(mode),
            rows: default_navigation(mode),
        },
        Section {
            title: "Files and selection",
            rows: DEFAULT_FILES.to_vec(),
        },
        Section {
            title: "Search and tools",
            rows: DEFAULT_TOOLS.to_vec(),
        },
        Section {
            title: "Preview media",
            rows: MEDIA.to_vec(),
        },
        Section {
            title: "Custom actions",
            rows: custom_action_rows(),
        },
    ]
}

/// `custom_action_rows`: one row per slot; only the slots a user binds show.
fn custom_action_rows() -> Vec<Shortcut> {
    strata_desktop::keys::CUSTOM_ACTIONS
        .iter()
        .zip(1..)
        .map(|(slot, number)| Shortcut {
            number: Some(number),
            ..row(One(slot.shown()), CUSTOM_ACTION)
        })
        .collect()
}

/// `CUSTOM_ACTION`: a custom action slot's name.
pub(crate) const CUSTOM_ACTION: &str = "Custom action %{number}";

fn navigation_title(mode: Mode) -> &'static str {
    match mode {
        Mode::Columns => "Columns navigation",
        Mode::Icons => "Icons navigation",
        Mode::List => "List navigation",
    }
}

const COLUMNS_NAVIGATION: &[Shortcut] = &[
    row(
        Any(shown![UP.alternate(0), DOWN.alternate(0)]),
        "Move between items",
    ),
    row(
        Any(shown![LEFT.alternate(0), RIGHT.alternate(0)]),
        "Parent pane / enter folder",
    ),
    within(
        "%{keys} at first pane",
        One(LEFT.alternate(0).shown()),
        "Focus the visible sidebar",
    ),
    row(
        One(BACKSPACE.shown()),
        "Close the current pane or go to the parent folder",
    ),
    row(
        One(MAKE_FIRST_COLUMN.shown()),
        "Make the focused pane the first pane",
    ),
];

const ICONS_NAVIGATION: &[Shortcut] = &[
    row(
        Any(shown![
            UP.alternate(0),
            DOWN.alternate(0),
            LEFT.alternate(0),
            RIGHT.alternate(0)
        ]),
        "Move spatially between tiles",
    ),
    within(
        "%{keys} at left edge",
        One(LEFT.alternate(0).shown()),
        "Focus the visible sidebar",
    ),
    row(One(BACKSPACE.shown()), "Go to the parent folder"),
    row(
        Any(shown![
            THUMBNAILS_LARGER.alternate(0),
            THUMBNAILS_SMALLER,
            THUMBNAILS_RESET
        ]),
        "Increase / decrease / reset thumbnail size",
    ),
];

const LIST_NAVIGATION: &[Shortcut] = &[
    row(
        Any(shown![UP.alternate(0), DOWN.alternate(0)]),
        "Move between file rows",
    ),
    row(One(LEFT.alternate(0).shown()), "Focus the visible sidebar"),
    row(One(BACKSPACE.shown()), "Go to the parent folder"),
];

/// Navigation every default-map view shares, after its own.
const DEFAULT_NAVIGATION: &[Shortcut] = &[
    row(
        Any(shown![
            LEFT.alternate(1),
            DOWN.alternate(1),
            UP.alternate(1),
            RIGHT.alternate(1)
        ]),
        "Same as %{keys} (type-to-search off)",
    )
    .mentioning(Mention::Keys(Any(shown![
        LEFT.alternate(0),
        DOWN.alternate(0),
        UP.alternate(0),
        RIGHT.alternate(0)
    ]))),
    within(
        "%{keys} at top",
        One(UP.alternate(0).shown()),
        "Focus the navigation header",
    ),
    within(
        "%{keys} in header",
        Any(shown![LEFT.alternate(0), RIGHT.alternate(0)]),
        "Move between header controls",
    ),
    within(
        "%{keys} in header",
        One(DOWN.alternate(0).shown()),
        "Return to the files",
    ),
    within(
        "%{keys} in sidebar",
        One(RIGHT.alternate(0).shown()),
        "Return to the browser",
    ),
    within(
        "%{keys} at sidebar top",
        One(UP.alternate(0).shown()),
        "Focus the top navigation bar",
    ),
    within(
        "%{keys} in top bar",
        Any(shown![LEFT.alternate(0), RIGHT.alternate(0)]),
        "Move between top-bar controls",
    ),
    within(
        "%{keys} in top bar",
        One(DOWN.alternate(0).shown()),
        "Return to the sidebar, or files when hidden",
    ),
    row(
        Any(shown![HISTORY_BACK, HISTORY_FORWARD]),
        "Back / forward in history",
    ),
];

/// Only Icons and List panes have a header with a Back button.
const BACK_BUTTON_HISTORY: Shortcut = within(
    "%{keys} on the Back button",
    One(CONTEXT_MENU.shown()),
    "List the back history (also right-click or hold)",
);

/// Navigation every default-map view shares, after the history rows.
const DEFAULT_GO_TO: &[Shortcut] = &[
    row(One(PARENT_FOLDER.shown()), "Go to the parent folder"),
    row(One(HOME_FOLDER.shown()), "Go to Home"),
    row(Any(shown![FIRST_ITEM, LAST_ITEM]), "First / last item"),
    row(
        Any(shown![FIRST_ITEM_UP, LAST_ITEM_DOWN]),
        "First / last item",
    ),
    row(Any(shown![PAGE_UP, PAGE_DOWN]), "Move one page"),
    row(
        Any(shown![FOCUS_NEXT, FOCUS_PREVIOUS]),
        "Next / previous interface control",
    ),
];

fn default_navigation(mode: Mode) -> Vec<Shortcut> {
    let (own, back_button): (_, &[Shortcut]) = match mode {
        Mode::Columns => (COLUMNS_NAVIGATION, &[]),
        Mode::Icons => (ICONS_NAVIGATION, &[BACK_BUTTON_HISTORY]),
        Mode::List => (LIST_NAVIGATION, &[BACK_BUTTON_HISTORY]),
    };
    [own, DEFAULT_NAVIGATION, back_button, DEFAULT_GO_TO].concat()
}

const DEFAULT_FILES: &[Shortcut] = &[
    row(One(ENTER.shown()), "Open the current item"),
    row(One(PREVIEW.shown()), "Preview a file, or open a folder"),
    row(Any(shown![COPY, CUT]), "Copy / cut selected items"),
    row(One(PASTE.shown()), "Paste into the indicated directory"),
    row(One(DUPLICATE.shown()), "Duplicate selected items"),
    row(
        One(MOVE_TO_TRASH.shown()),
        "Move selected items to Trash, when supported",
    ),
    row(
        One(DELETE_PERMANENTLY.shown()),
        "Permanently delete selected items",
    ),
    row(One(UNDO.shown()), "Undo the last file operation"),
    row(One(REDO.shown()), "Redo the last file operation"),
    row(One(RENAME.shown()), "Rename"),
    within(
        "%{keys} in a rename",
        Any(shown![FOCUS_NEXT, FOCUS_PREVIOUS]),
        "Rename the next / previous item",
    ),
    row(One(NEW_FOLDER.shown()), "Create a folder"),
    row(
        One(NEW_FOLDER_WITH_SELECTION.shown()),
        "Create a folder containing the selection",
    ),
    row(
        One(SELECT_ALL.shown()),
        "Select all items in the focused pane",
    ),
    row(Any(shown![EXTEND_UP, EXTEND_DOWN]), "Extend selection"),
    row(
        Any(shown![EXTEND_PAGE_UP, EXTEND_PAGE_DOWN]),
        "Extend selection by one page",
    ),
    row(
        One(SELECTION_TOGGLE.shown()),
        "Toggle the focused item in the selection",
    ),
    row(One(PROPERTIES.shown()), "Show item properties"),
    row(
        Any(shown![OPEN_IN_NEW_TAB, OPEN_IN_NEW_WINDOW]),
        "Open the focused folder in a new tab / window",
    ),
    row(One(CONTEXT_MENU.shown()), "Open the context menu"),
    row(
        Any(shown![COPY_PATH, PIN]),
        "Copy path / pin a folder (type-to-search off)",
    ),
];

const DEFAULT_TOOLS: &[Shortcut] = &[
    row(One(FILTER.shown()), "Filter the current pane"),
    within(
        "%{keys} in a filter",
        One(FILTER_DESCEND.shown()),
        "Enter the first matching folder",
    ),
    row(One(SEARCH.shown()), "Open global search"),
    row(One(JUMP_TO_FOLDER.shown()), "Jump to a recent folder"),
    row(
        One(OPEN_CONTAINING_FOLDER.shown()),
        "Open containing folder (global search)",
    ),
    row(One(LOCATION.shown()), "Edit the location"),
    row(One(TAB_NEW.shown()), "New tab"),
    row(One(TAB_CLOSE.shown()), "Close the active tab"),
    row(One(TAB_REOPEN.shown()), "Reopen the last closed tab"),
    row(
        Any(shown![TAB_NEXT.alternate(0), TAB_PREVIOUS.alternate(0)]),
        "Next / previous tab",
    ),
    row(
        Any(shown![TAB_PREVIOUS.alternate(1), TAB_NEXT.alternate(1)]),
        "Previous / next tab",
    ),
    row(
        Any(shown![TAB_MOVE_LEFT, TAB_MOVE_RIGHT]),
        "Move the active tab left / right",
    ),
    SELECT_TAB,
    row(One(TERMINAL.shown()), "Open a terminal"),
    row(One(REFRESH.shown()), "Refresh"),
    row(One(HIDDEN_FILES.shown()), "Show or hide hidden files"),
    row(VIEW_MODES, "Switch to Columns, Icons, or List"),
    row(TEXT_SIZE, "Increase / decrease / reset text size"),
    row(One(SIDEBAR.shown()), "Show or hide the sidebar"),
    row(
        One(SIDEBAR_FOCUS.shown()),
        "Switch focus between sidebar and browser",
    ),
    row(
        One(ARROW_SCOPE.shown()),
        "Keep arrows in the file list on or off",
    ),
    row(One(SETTINGS.shown()), "Open Settings"),
    row(One(TENXER_MODE.shown()), "Turn on 10xer mode"),
    row(
        One(ESCAPE.shown()),
        "Close preview or cancel the current interaction",
    ),
    row(One(REFERENCE.shown()), "Show or hide this reference"),
];

const SELECT_TAB: Shortcut = row(
    Keys::Digits(TAB_NUMBERS),
    "Select a tab (hold %{keys} for numbers)",
)
.mentioning(Mention::Held(TAB_NUMBERS.modifiers()));

const VIEW_MODES: Keys = Any(shown![VIEW_COLUMNS, VIEW_ICONS, VIEW_LIST]);

const TEXT_SIZE: Keys = Any(shown![TEXT_LARGER.alternate(0), TEXT_SMALLER, TEXT_RESET]);

const MEDIA: &[Shortcut] = &[
    row(One(MEDIA_PLAY_PAUSE.shown()), "Play / pause"),
    row(
        Any(shown![MEDIA_SEEK_BACK, MEDIA_SEEK_FORWARD]),
        "Seek −5 / +5 seconds",
    ),
    row(
        Any(shown![MEDIA_VOLUME_UP, MEDIA_VOLUME_DOWN]),
        "Volume up / down",
    ),
    row(One(MEDIA_MUTE.shown()), "Mute / unmute"),
    row(
        Any(shown![MEDIA_PREVIOUS, MEDIA_NEXT]),
        "Previous / next file of the same type",
    ),
    within(
        "%{keys} on a video",
        One(ENTER.shown()),
        "Open it in the default app where the preview stopped",
    ),
];

#[cfg(test)]
mod tests;
