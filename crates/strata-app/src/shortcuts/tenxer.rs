// SPDX-License-Identifier: MIT

use strata_desktop::keys::{
    Keys::{self, Any, Chord, Mixed, One},
    Modifiers,
    bindings::*,
    shown,
};

use super::contents::{
    MEDIA, Mention, Mode, SELECT_TAB, Section, Shortcut, TEXT_SIZE, VIEW_MODES, default_sections,
    navigation_title, row, within,
};

/// `IN_ANY_PREVIEW`.
const IN_ANY_PREVIEW: &str = "%{keys} in any preview";

/// `tenxer_sections` for the window (no chooser).
pub(crate) fn tenxer_sections(mode: Mode) -> Vec<Section> {
    let mut sections = vec![
        Section {
            title: navigation_title(mode),
            rows: tenxer_navigation(mode, false),
        },
        Section {
            title: "Places",
            rows: tenxer_places(false),
        },
        Section {
            title: "Files and selection",
            rows: TENXER_FILES.to_vec(),
        },
        Section {
            title: "Preview",
            rows: tenxer_preview(mode, false),
        },
        Section {
            title: "Search and tools",
            rows: tenxer_tools(mode, false),
        },
        Section {
            title: "Preview media",
            rows: MEDIA.to_vec(),
        },
    ];
    let mode_rows = tenxer_mode_rows(mode, false, &sections);
    let mode_index = sections
        .iter()
        .position(|section| section.title == "Search and tools")
        .expect("search section");
    sections.insert(
        mode_index,
        Section {
            title: "10xer mode",
            rows: mode_rows,
        },
    );
    sections
}

const TENXER_LINE_NAVIGATION: &[Shortcut] = &[
    row(
        Any(shown![
            DOWN.alternate(1),
            UP.alternate(1),
            UP.alternate(0),
            DOWN.alternate(0)
        ]),
        "Next / previous item",
    ),
    row(
        Any(shown![
            LEFT.alternate(1),
            LEFT.alternate(0),
            BACKSPACE,
            PARENT_FOLDER
        ]),
        "Go to the parent folder",
    ),
    row(
        Any(shown![RIGHT.alternate(1), RIGHT.alternate(0)]),
        "Open the focused directory",
    ),
    within(
        "%{keys} on a file",
        Any(shown![RIGHT.alternate(1), RIGHT.alternate(0)]),
        "Enter its preview; keys move into the drawer",
    ),
];

const TENXER_ICONS_NAVIGATION: &[Shortcut] = &[
    row(
        Any(shown![
            LEFT.alternate(1),
            DOWN.alternate(1),
            UP.alternate(1),
            RIGHT.alternate(1),
            UP.alternate(0),
            DOWN.alternate(0),
            LEFT.alternate(0),
            RIGHT.alternate(0)
        ]),
        "Move spatially between tiles",
    ),
    row(
        Any(shown![BACKSPACE, PARENT_FOLDER]),
        "Go to the parent folder",
    ),
    row(
        Any(shown![
            THUMBNAILS_LARGER.alternate(2),
            THUMBNAILS_SMALLER.alternate(1),
            THUMBNAILS_LARGER.alternate(0),
            THUMBNAILS_SMALLER.alternate(0),
        ]),
        "Increase / decrease thumbnail size",
    ),
    row(One(THUMBNAILS_RESET.shown()), "Reset thumbnail size"),
];

/// Navigation every 10xer view shares, after its own.
const TENXER_NAVIGATION: &[Shortcut] = &[
    row(
        Any(shown![
            HISTORY_BACK.alternate(1),
            HISTORY_FORWARD.alternate(1),
            HISTORY_BACK.alternate(0),
            HISTORY_FORWARD.alternate(0),
        ]),
        "Back / forward in history",
    ),
    row(One(HOME_FOLDER.shown()), "Go to Home"),
    row(One(FIRST_ITEM.shown()), "First item"),
    row(One(LAST_ITEM.shown()), "Last item"),
    row(
        Any(shown![FIRST_ITEM_UP, LAST_ITEM_DOWN]),
        "First / last item",
    ),
    row(
        Any(shown![HALF_PAGE_UP, HALF_PAGE_DOWN]),
        "Move half a page",
    ),
    row(
        Any(shown![
            PAGE_UP.alternate(1),
            PAGE_DOWN.alternate(1),
            PAGE_UP.alternate(0),
            PAGE_DOWN.alternate(0),
        ]),
        "Move one page",
    ),
];

fn tenxer_navigation(mode: Mode, chooser: bool) -> Vec<Shortcut> {
    let mut shortcuts = match mode {
        Mode::Columns | Mode::List => TENXER_LINE_NAVIGATION,
        Mode::Icons => TENXER_ICONS_NAVIGATION,
    }
    .to_vec();
    // The chooser refuses i, and its own section describes Enter and o.
    if !chooser {
        shortcuts.extend_from_slice(&tenxer_open_rows(mode));
    }
    shortcuts.extend_from_slice(TENXER_NAVIGATION);
    shortcuts
}

fn tenxer_open_rows(mode: Mode) -> [Shortcut; 2] {
    [
        row(
            One(PREVIEW.shown()),
            if mode == Mode::Columns {
                "Open the next column for the focused directory"
            } else {
                "Toggle folder peek for the focused directory"
            },
        ),
        row(Any(shown![ENTER, OPEN]), "Open the focused item"),
    ]
}

fn tenxer_places(chooser: bool) -> Vec<Shortcut> {
    let mut shortcuts = vec![
        row(Chord(shown![GO, GO_FIRST]), "First item"),
        row(Chord(shown![GO, GO_FOLLOW]), "Follow search result"),
        row(
            Mixed(&[Chord(shown![GO, GO_HOME]), Chord(shown![GO, GO_CONFIG])]),
            "Home / ~/.config",
        ),
        row(
            Mixed(&[
                Chord(shown![GO, GO_DOWNLOADS]),
                Chord(shown![GO, GO_DOCUMENTS]),
                Chord(shown![GO, GO_MUSIC]),
                Chord(shown![GO, GO_PICTURES]),
                Chord(shown![GO, GO_VIDEOS]),
            ]),
            "Downloads / Documents / Music / Pictures / Videos",
        ),
    ];
    shortcuts.extend(sidebar_places(chooser));
    shortcuts.extend_from_slice(&[
        row(
            Chord(shown![GO, GO_PROMPT]),
            "Go to a folder, typed path, or URI",
        ),
        within(
            "%{keys} in %{go} / %{move_to} / %{copy_to}",
            One(COMPLETE.shown()),
            "Write the chosen folder into the prompt",
        ),
        row(One(JUMP_VISITED.shown()), "Jump to a visited folder"),
        row(One(JUMP_RECENT.shown()), "Jump to a recent folder"),
        within(
            "%{keys} in %{jump} / %{recent}",
            Any(shown![UP.alternate(0), DOWN.alternate(0)]),
            "Choose a visited folder",
        ),
        row(Chord(shown![GO, ESCAPE]), "Cancel a pending chord"),
    ]);
    shortcuts
}

/// The places only the window's `g` reaches, or the chooser's own (`tenxer_places`).
fn sidebar_places(chooser: bool) -> Vec<Shortcut> {
    if chooser {
        vec![
            row(Chord(shown![GO, GO_RECENT]), "Recent"),
            row(
                Keys::Digits(PIN_NUMBERS),
                "Visible PINNED rows that are local folders",
            ),
        ]
    } else {
        vec![
            row(
                Mixed(&[
                    Chord(shown![GO, GO_TRASH]),
                    Chord(shown![GO, GO_NETWORK]),
                    Chord(shown![GO, GO_RECENT]),
                ]),
                "Trash / Network / Recent",
            ),
            row(
                Keys::Digits(PIN_NUMBERS),
                "Visible PINNED rows in sidebar order",
            ),
            row(
                Mixed(&[Chord(shown![GO, GO_PIN]), Chord(shown![GO, GO_UNPIN])]),
                "Pin / unpin a folder",
            ),
            row(
                Keys::Letters(BOOKMARK_SET_LETTERS),
                "Save current folder under a letter",
            ),
            row(
                Keys::Letters(BOOKMARK_GO_LETTERS),
                "Jump to a bookmarked folder by letter",
            ),
        ]
    }
}

fn tenxer_preview(mode: Mode, chooser: bool) -> Vec<Shortcut> {
    let mut shortcuts = Vec::new();
    if !chooser {
        shortcuts.push(within(
            "%{keys} on a file",
            One(PREVIEW.shown()),
            "Toggle the preview without taking focus",
        ));
    }
    shortcuts.push(row(
        Any(shown![SCROLL_PREVIEW_DOWN, SCROLL_PREVIEW_UP]),
        "Scroll the open preview without taking focus",
    ));
    shortcuts.push(row(
        Any(shown![TENXER_MEDIA_PREVIOUS, TENXER_MEDIA_NEXT]),
        "Previous / next file of the same type while the preview shows audio or video",
    ));
    // Icons have no key that moves into the preview.
    if mode != Mode::Icons {
        shortcuts.extend_from_slice(TENXER_PREVIEW_OWNED);
        if !chooser {
            shortcuts.push(within(
                "%{keys} in the preview",
                One(FIND_IN_PREVIEW.shown()),
                "Find in preview",
            ));
        }
    }
    shortcuts
}

const TENXER_PREVIEW_OWNED: &[Shortcut] = &[
    within(
        "%{keys} in the preview",
        Any(shown![
            DOWN.alternate(1),
            UP.alternate(1),
            UP.alternate(0),
            DOWN.alternate(0)
        ]),
        "Scroll the document",
    ),
    within(
        "%{keys} in the preview",
        Any(shown![HALF_PAGE_UP, HALF_PAGE_DOWN]),
        "Scroll half a page",
    ),
    within(
        "%{keys} in the preview",
        Any(shown![
            PAGE_UP.alternate(1),
            PAGE_DOWN.alternate(1),
            PAGE_UP_KEY,
            PAGE_DOWN_KEY
        ]),
        "Scroll one page",
    ),
    within(
        "%{keys} in the preview",
        TOP_AND_BOTTOM,
        "Top / bottom of the document",
    ),
    within(
        "%{keys} in an archive",
        Any(shown![
            DOWN.alternate(1),
            UP.alternate(1),
            RIGHT.alternate(1),
            ENTER
        ]),
        "Move / open an archive folder",
    ),
    within(
        "%{keys} in an archive",
        TOP_AND_BOTTOM,
        "First / last archive member",
    ),
    within(
        "%{keys} in an archive",
        One(LEFT.alternate(1).shown()),
        "Archive parent; at the root, back to the listing",
    ),
    within("%{keys} in media", One(SPACE.shown()), "Play / pause"),
    within(
        "%{keys} in media",
        Any(shown![LEFT.alternate(0), RIGHT.alternate(0)]),
        "Seek −5 / +5 seconds",
    ),
    within(
        "%{keys} in media",
        Any(shown![
            MEDIA_SEEK_BACK_PAGE,
            MEDIA_SEEK_FORWARD_PAGE,
            PAGE_DOWN_KEY,
            PAGE_UP_KEY,
        ]),
        "Seek −30 / +30 seconds",
    ),
    within(
        "%{keys} in media",
        Any(shown![MEDIA_SEEK_START, HOME_KEY, MEDIA_SEEK_END, END_KEY]),
        "Seek to the start / end",
    ),
    within(
        "%{keys} in media",
        Any(shown![
            UP.alternate(0),
            DOWN.alternate(0),
            MEDIA_VOLUME_STEP_UP.alternate(0),
            MEDIA_VOLUME_STEP_DOWN
        ]),
        "Volume up / down",
    ),
    within(
        "%{keys} in media",
        One(TENXER_MEDIA_MUTE.shown()),
        "Mute / unmute",
    ),
    within(
        "%{keys} in a document",
        Any(shown![LEFT.alternate(1), LEFT.alternate(0)]),
        "Return to the listing; the preview stays open",
    ),
    within(
        "%{keys} in media",
        One(LEFT.alternate(1).shown()),
        "Return to the listing; the preview stays open",
    ),
    within(
        IN_ANY_PREVIEW,
        One(FOCUS_PREVIOUS.shown()),
        "Return to the listing",
    ),
    within(
        "%{keys} in the preview",
        Any(shown![ESCAPE, PREVIEW]),
        "Close the preview and return to the listing",
    ),
];

const TOP_AND_BOTTOM: Keys = Mixed(&[
    Chord(shown![GO, GO_FIRST]),
    One(HOME_KEY.shown()),
    One(LAST_ITEM.alternate(0).shown()),
    One(END_KEY.shown()),
]);

const TRASH_AFTER_CONFIRMING: Shortcut = row(
    One(MOVE_TO_TRASH.shown()),
    "Move to Trash after confirming; %{keys} confirms",
)
.mentioning(Mention::Keys(Chord(shown![
    MOVE_TO_TRASH,
    TENXER_DELETE_CONFIRM,
])));

const TAB_IN_INLINE_RENAME: Shortcut = within(
    "%{keys} in an inline rename",
    Any(shown![FOCUS_NEXT, FOCUS_PREVIOUS]),
    "Rename the next / previous item",
);

const TENXER_FILES: &[Shortcut] = &[
    row(
        Any(shown![YANK, YANK_CUT]),
        "Yank / cut the selection, or the focused item",
    ),
    row(One(UNYANK.shown()), "Clear copy and cut marks"),
    row(
        One(PASTE_KEEP_BOTH.shown()),
        "Paste; Keep Both is focused on conflicts",
    ),
    row(
        Any(shown![PASTE_REPLACE, PASTE]),
        "Paste; Replace is focused on conflicts",
    ),
    TRASH_AFTER_CONFIRMING,
    row(
        One(DELETE_PERMANENTLY.shown()),
        "Delete permanently after confirming",
    ),
    row(
        One(CREATE.shown()),
        "Create a file; end with / for a folder",
    ),
    row(COPY_PATH_OR_NAME, "Copy path / name"),
    row(Any(shown![COPY, CUT]), "Copy / cut selected items"),
    row(One(UNDO.shown()), "Undo the last file operation"),
    row(One(REDO.shown()), "Redo the last file operation"),
    row(One(RENAME.shown()), "Rename the focused item in the footer"),
    TAB_IN_INLINE_RENAME,
    row(
        One(OPEN_WITH.shown()),
        "Open With for the selection, or the focused item",
    ),
    row(
        Keys::Digits(ACTIONS_NUMBERS),
        "Run one of the first ten matching custom actions",
    ),
    row(
        Keys::Digits(ACTIONS_FOLDER_NUMBERS),
        "Run one of the first ten custom actions for the focused folder",
    ),
    row(
        Chord(shown![ACTIONS, ACTIONS_TERMINAL]),
        "Open a terminal in the focused folder",
    ),
    row(
        Chord(shown![ACTIONS, ACTIONS_COMPRESS]),
        "Compress the selection, or the focused item",
    ),
    row(
        Mixed(&[
            Chord(shown![ACTIONS, ACTIONS_EXTRACT]),
            Chord(shown![ACTIONS, ACTIONS_EXTRACT_TO]),
        ]),
        "Extract the archive here / to a typed folder",
    ),
    row(
        Any(shown![MOVE_TO, COPY_TO]),
        "Move / copy to a typed folder",
    ),
    row(One(RESTORE.shown()), "Restore from Trash"),
    row(One(NEW_FOLDER.shown()), "Create a folder"),
    row(
        One(NEW_FOLDER_WITH_SELECTION.shown()),
        "Create a folder containing the selection",
    ),
    row(One(SPACE.shown()), "Toggle the focused item and move down"),
    row(
        Any(shown![VISUAL_SELECT, VISUAL_UNSET]),
        "Visual select / visual unset",
    ),
    row(
        One(SELECT_ALL.shown()),
        "Select all items in the focused pane",
    ),
    row(One(INVERT_SELECTION.shown()), "Invert the selection"),
    row(One(PROPERTIES.shown()), "Show item properties"),
    row(
        Any(shown![OPEN_IN_NEW_TAB, OPEN_IN_NEW_WINDOW]),
        "Open the focused folder in a new tab / window",
    ),
    row(One(CONTEXT_MENU.shown()), "Open the context menu"),
];

const COPY_PATH_OR_NAME: Keys = Mixed(&[
    Chord(shown![COPY_AS, COPY_AS_PATH]),
    Chord(shown![COPY_AS, COPY_AS_NAME]),
]);

/// 10xer rows that a default row already describes with other keys or context.
const SHARED_WITH_DEFAULT: [(Keys, Option<&str>); 4] = [
    (One(FIRST_ITEM.shown()), None),
    (One(CHOOSE_FOLDER.shown()), None),
    (One(FOCUS_PREVIOUS.shown()), Some(IN_ANY_PREVIEW)),
    (TAB_IN_INLINE_RENAME.keys, TAB_IN_INLINE_RENAME.context),
];

fn tenxer_mode_rows(mode: Mode, chooser: bool, sections: &[Section]) -> Vec<Shortcut> {
    let mut shortcuts = Vec::new();
    if !chooser {
        shortcuts.push(row(One(CLOSE_WINDOW.shown()), "Close the current window"));
    }
    shortcuts.extend_from_slice(&[
        row(One(TENXER_MODE.shown()), "Toggle 10xer mode"),
        row(One(REFERENCE.shown()), "Show or hide this reference"),
    ]);
    let default = default_sections(mode);
    for &shortcut in sections.iter().flat_map(|section| &section.rows) {
        let keys = (shortcut.keys, shortcut.context);
        // Keep grouped aliases together when they include a 10xer binding.
        let shared = default
            .iter()
            .flat_map(|section| &section.rows)
            .any(|default| {
                (default.keys, default.context) == keys && default.keys.same_in_both_maps()
            })
            || SHARED_WITH_DEFAULT.contains(&keys);
        if !shared && !shortcuts.contains(&shortcut) {
            shortcuts.push(shortcut);
        }
    }
    shortcuts
}

const TENXER_FIND: &[Shortcut] = &[
    row(
        Any(shown![FIND, FIND_BACKWARD]),
        "Find the next / previous name in this listing",
    ),
    row(
        Any(shown![REPEAT_FIND, REPEAT_FIND_REVERSED]),
        "Repeat the last find / in reverse",
    ),
    within(
        "%{keys} in a prompt",
        Any(shown![UP.alternate(0), DOWN.alternate(0)]),
        "Move through the listing",
    ),
    within(
        "%{keys} in a prompt",
        Any(shown![ENTER, ESCAPE]),
        "Apply / cancel it",
    ),
    within(
        "%{keys} after a find",
        One(ESCAPE.shown()),
        "Dismiss the find highlights",
    ),
    row(One(FILTER.shown()), "Filter this listing"),
    within(
        "%{keys} in a filter",
        One(FILTER_DESCEND.shown()),
        "Enter the highlighted or first matching folder",
    ),
    within(
        "%{keys} after a filter",
        One(ESCAPE.shown()),
        "Clear the filter",
    ),
    row(
        One(SEARCH_SUBFOLDERS.shown()),
        "Search this folder and its subfolders",
    ),
];

fn dismiss_search_hits(mode: Mode) -> Shortcut {
    within(
        "%{keys} after a search",
        // Icons h and ← move between result icons instead.
        if mode == Mode::Icons {
            One(ESCAPE.shown())
        } else {
            Any(shown![ESCAPE, LEFT.alternate(1), LEFT.alternate(0)])
        },
        "Dismiss the hits",
    )
}

const TENXER_HEADER_AND_SIDEBAR: &[Shortcut] = &[
    row(One(FOCUS_NEXT.shown()), "Focus the window header"),
    within(
        "%{keys} in the header",
        Any(shown![ENTER, SPACE]),
        "Activate the focused control",
    ),
    within(
        "%{keys} in the header",
        Any(shown![LEFT.alternate(1), DOWN.alternate(1)]),
        "Return to the files",
    ),
    row(One(SIDEBAR.shown()), "Show or hide the sidebar"),
    row(
        One(SIDEBAR_FOCUS.shown()),
        "Focus the visible sidebar, or return to the files",
    ),
    within(
        "%{keys} in the sidebar",
        Any(shown![
            DOWN.alternate(1),
            UP.alternate(1),
            UP.alternate(0),
            DOWN.alternate(0)
        ]),
        "Move between places and device controls",
    ),
    within(
        "%{keys} in the sidebar",
        Any(shown![RIGHT.alternate(1), ENTER, SPACE]),
        "Activate the focused place or device control",
    ),
    within(
        "%{keys} in the sidebar",
        Any(shown![LEFT.alternate(1), LEFT.alternate(0), BACKSPACE]),
        "Return to the files",
    ),
];

const TENXER_TABS: &[Shortcut] = &[
    row(
        Mixed(&[
            Chord(shown![TAB_CHORD, TAB_CHORD_NEW]),
            Chord(shown![TAB_CHORD, TAB_CHORD_CLOSE]),
        ]),
        "New / close tab",
    ),
    row(Chord(shown![TAB_CHORD, TAB_CHORD_PREVIOUS]), "Previous tab"),
    row(Keys::Digits(TAB_CHORD_NUMBERS), "Select tab 1–9 / 10"),
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
    row(One(SEARCH.shown()), "Open global search"),
    row(
        One(OPEN_CONTAINING_FOLDER.shown()),
        "Open containing folder (global search)",
    ),
];

const TENXER_VIEW: &[Shortcut] = &[
    row(One(LOCATION.shown()), "Edit the location"),
    row(One(REFRESH.shown()), "Refresh"),
    row(
        Any(shown![
            HIDDEN_FILES.alternate(2),
            HIDDEN_FILES.alternate(0),
            HIDDEN_FILES.alternate(1)
        ]),
        "Show or hide hidden files",
    ),
    row(
        Mixed(&[
            Chord(shown![SORT_BY, SORT_BY_NAME]),
            Chord(shown![SORT_BY, SORT_BY_MODIFIED]),
            Chord(shown![SORT_BY, SORT_BY_SIZE]),
            Chord(shown![SORT_BY, SORT_BY_TYPE]),
        ]),
        "Sort by name / modified / size / type; %{keys} reverses",
    )
    .mentioning(Mention::Held(Modifiers::SHIFT_MASK)),
    row(Chord(shown![SORT_BY, SORT_BY_EXTENSION]), "Extension"),
    row(Chord(shown![SORT_BY, SORT_BY_CREATED]), "Created"),
    row(Chord(shown![SORT_BY, SORT_BY_UNSORTED]), "Unsorted"),
    row(Chord(shown![SORT_BY, SORT_BY_NATURAL]), "Natural"),
    row(Chord(shown![SORT_BY, SORT_BY_ALPHABETICAL]), "Alphabetical"),
    row(
        Chord(shown![SORT_BY, SORT_BY_CASE_SENSITIVE]),
        "Case-sensitive names",
    ),
    row(
        Chord(shown![SORT_BY, SORT_BY_FOLDERS_FIRST]),
        "Folders first",
    ),
    row(VIEW_MODES, "Switch to Columns, Icons, or List"),
    row(TEXT_SIZE, "Increase / decrease / reset text size"),
];

const TENXER_REFERENCE: &[Shortcut] = &[
    row(
        One(ESCAPE.shown()),
        "Close this reference or cancel the current interaction",
    ),
    row(One(REFERENCE.shown()), "Show or hide this reference"),
];

fn tenxer_tools(mode: Mode, chooser: bool) -> Vec<Shortcut> {
    let mut shortcuts = TENXER_FIND.to_vec();
    shortcuts.push(dismiss_search_hits(mode));
    shortcuts.extend_from_slice(TENXER_HEADER_AND_SIDEBAR);
    if !chooser {
        shortcuts.extend_from_slice(TENXER_TABS);
    }
    shortcuts.extend_from_slice(TENXER_VIEW);
    if !chooser {
        shortcuts.push(row(One(SETTINGS.shown()), "Open Settings"));
    }
    shortcuts.extend_from_slice(TENXER_REFERENCE);
    shortcuts
}
