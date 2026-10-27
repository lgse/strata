// SPDX-License-Identifier: MIT

use gtk::gdk::ModifierType as Modifiers;

use super::{
    browser_modes::BrowserMode,
    keys::{
        self, Binding, Keys,
        Keys::{Any, Chord, Mixed, One},
        bindings::*,
    },
};

pub(crate) const EXPERIMENTAL_LABEL: &str = "(experimental feature, under active development)";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReferenceScope {
    pub mode: BrowserMode,
    pub chooser: Option<ChooserScope>,
}

impl ReferenceScope {
    pub(crate) fn window(mode: BrowserMode) -> Self {
        Self {
            mode,
            chooser: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ChooserScope {
    pub request: ChooserRequest,
    pub multiple: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ChooserRequest {
    Files,
    Folders,
    SaveFile,
    SaveFiles,
}

/// Lowercase, with the `−` and the `–` of digit ranges that labels show read
/// as a typed `-`.
pub(crate) fn search_folded(text: &str) -> String {
    text.to_lowercase().replace(['\u{2212}', '\u{2013}'], "-")
}

pub(crate) struct ReferenceSection {
    pub title: &'static str,
    pub rows: Vec<Shortcut>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Shortcut {
    pub keys: Keys,
    /// A translatable template that places `%{keys}` in context.
    pub context: Option<&'static str>,
    pub action: &'static str,
    /// What the action's own `%{keys}` names.
    pub mention: Option<Mention>,
}

/// Keys named inside action text or beside a chord step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Mention {
    Keys(Keys),
    /// Modifiers held with the keys, such as the **Shift** that reverses a sort.
    Held(Modifiers),
}

impl Mention {
    pub(crate) fn label(self) -> String {
        self.label_in(&rust_i18n::locale())
    }

    fn label_in(self, locale: &str) -> String {
        match self {
            Self::Keys(keys) => keys::label_in(locale, &keys),
            Self::Held(modifiers) => keys::modifiers_label_in(locale, modifiers),
        }
    }
}

impl Shortcut {
    pub(crate) fn key_text(&self) -> String {
        let keys = keys::label(&self.keys);
        let Some(context) = self.context else {
            return keys;
        };
        use super::tenxer_mode::Prompt;
        rust_i18n::t!(
            context,
            keys = keys,
            go = Prompt::Go.label(),
            move_to = Prompt::MoveTo.label(),
            copy_to = Prompt::CopyTo.label(),
            jump = Prompt::Jump.label(),
            recent = Prompt::Recent.label()
        )
        .into_owned()
    }

    pub(crate) fn action_text(&self) -> String {
        self.action_in(&rust_i18n::locale())
    }

    /// Matches keys and actions in the interface language or English. `query`
    /// is already `search_folded`.
    pub(crate) fn matches(&self, query: &str) -> bool {
        let found = |text: String| search_folded(&text).contains(query);
        query.is_empty()
            || found(keys::search_text(&self.keys))
            || found(self.key_text())
            || found(self.action_text())
            || found(self.action_in("en"))
    }

    fn action_in(&self, locale: &str) -> String {
        match self.mention {
            Some(mention) => rust_i18n::t!(
                self.action,
                locale = locale,
                keys = mention.label_in(locale)
            )
            .into_owned(),
            None => rust_i18n::t!(self.action, locale = locale).into_owned(),
        }
    }

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
    }
}

const fn within(context: &'static str, keys: Keys, action: &'static str) -> Shortcut {
    Shortcut {
        context: Some(context),
        ..row(keys, action)
    }
}

pub(crate) fn with_keys(message: &str, keys: Keys) -> String {
    rust_i18n::t!(message, keys = keys::label(&keys)).into_owned()
}

/// Pango markup for a legend: each shortcut in bold before its action.
pub(crate) fn legend_markup(hints: &[(Keys, &str)]) -> String {
    hints
        .iter()
        .map(|(keys, action)| {
            format!(
                "<b>{}</b> {}",
                gtk::glib::markup_escape_text(&keys::label(keys)),
                gtk::glib::markup_escape_text(&crate::i18n::tr(action))
            )
        })
        .collect::<Vec<_>>()
        .join("   ")
}

const IN_ANY_PREVIEW: &str = "%{keys} in any preview";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContextHint {
    None,
    Preview,
    /// The file chooser previews with Space only; it has no 10xer `i` preview.
    ChooserPreview,
    CopyPath,
    CopyPaths,
    Rename,
    Cut,
    Copy,
    Duplicate,
    Paste,
    Pin,
    MoveTo,
    CopyTo,
    Restore,
    RemoveFromRecent,
    Terminal,
    Trash,
    PermanentDelete,
    Open,
    OpenInNewTab,
    OpenInNewWindow,
    Properties,
    ContainingFolder,
    NewFolder,
    SelectAll,
    Refresh,
    HiddenFiles,
}

/// Only the 10xer map honors `scope.chooser`: portal choosers show the footer
/// and dispatch F1 only in 10xer mode, so the default map has no chooser view.
pub(crate) fn reference_sections(scope: ReferenceScope) -> Vec<ReferenceSection> {
    if super::tenxer_mode::chrome_suppressed() {
        tenxer_sections(scope.mode, scope.chooser)
    } else {
        default_sections(scope.mode)
    }
}

pub(crate) fn context_hint_for(
    hint: ContextHint,
    preferences: &super::preferences::PreferenceManager,
) -> Option<Keys> {
    if preferences.tenxer_mode() {
        tenxer_hint(hint)
    } else {
        default_hint(hint, preferences.type_to_search_active())
    }
}

/// The binding a menu row's `accel` carries; only the first alternative fits.
pub(crate) fn menu_accelerator(keys: Keys) -> Option<Binding> {
    match keys {
        One(binding) => Some(binding),
        Any(bindings) => bindings.first().copied(),
        Chord(_) | Keys::Digits(_) | Mixed(_) => None,
    }
}

fn default_sections(mode: BrowserMode) -> Vec<ReferenceSection> {
    vec![
        ReferenceSection {
            title: navigation_title(mode),
            rows: default_navigation(mode),
        },
        ReferenceSection {
            title: "Files and selection",
            rows: DEFAULT_FILES.to_vec(),
        },
        ReferenceSection {
            title: "Search and tools",
            rows: DEFAULT_TOOLS.to_vec(),
        },
        ReferenceSection {
            title: "Preview media",
            rows: MEDIA.to_vec(),
        },
    ]
}

fn tenxer_sections(mode: BrowserMode, chooser: Option<ChooserScope>) -> Vec<ReferenceSection> {
    let mut sections = vec![ReferenceSection {
        title: navigation_title(mode),
        rows: tenxer_navigation(mode, chooser.is_some()),
    }];
    if let Some(chooser) = chooser {
        sections.push(ReferenceSection {
            title: "File chooser",
            rows: chooser_requests(chooser),
        });
    }
    sections.extend([
        ReferenceSection {
            title: "Places",
            rows: tenxer_places(chooser.is_some()),
        },
        ReferenceSection {
            title: "Files and selection",
            rows: chooser.map_or_else(|| TENXER_FILES.to_vec(), chooser_files),
        },
        ReferenceSection {
            title: "Preview",
            rows: tenxer_preview(mode, chooser.is_some()),
        },
        ReferenceSection {
            title: "Search and tools",
            rows: tenxer_tools(mode, chooser.is_some()),
        },
        ReferenceSection {
            title: "Preview media",
            rows: MEDIA.to_vec(),
        },
    ]);
    let mode_rows = tenxer_mode_rows(mode, chooser.is_some(), &sections);
    let mode_index = sections
        .iter()
        .position(|section| section.title == "Search and tools")
        .expect("search section");
    sections.insert(
        mode_index,
        ReferenceSection {
            title: "10xer mode",
            rows: mode_rows,
        },
    );
    sections
}

fn navigation_title(mode: BrowserMode) -> &'static str {
    match mode {
        BrowserMode::Columns => "Columns navigation",
        BrowserMode::Icons => "Icons navigation",
        BrowserMode::List => "List navigation",
    }
}

fn default_navigation(mode: BrowserMode) -> Vec<Shortcut> {
    let mut shortcuts = match mode {
        BrowserMode::Columns => vec![
            row(Any(&[UP, DOWN]), "Move between items"),
            row(Any(&[LEFT, RIGHT]), "Parent pane / enter folder"),
            within(
                "%{keys} at first pane",
                One(LEFT),
                "Focus the visible sidebar",
            ),
            row(
                One(BACKSPACE),
                "Close the current pane or go to the parent folder",
            ),
        ],
        BrowserMode::Icons => vec![
            row(
                Any(&[UP, DOWN, LEFT, RIGHT]),
                "Move spatially between tiles",
            ),
            within(
                "%{keys} at left edge",
                One(LEFT),
                "Focus the visible sidebar",
            ),
            row(One(BACKSPACE), "Go to the parent folder"),
            row(
                Any(&[THUMBNAILS_LARGER, THUMBNAILS_SMALLER, THUMBNAILS_RESET]),
                "Increase / decrease / reset thumbnail size",
            ),
        ],
        BrowserMode::List => vec![
            row(Any(&[UP, DOWN]), "Move between file rows"),
            row(One(LEFT), "Focus the visible sidebar"),
            row(One(BACKSPACE), "Go to the parent folder"),
        ],
    };
    shortcuts.extend_from_slice(&[
        row(
            Any(&[VIM_LEFT, VIM_DOWN, VIM_UP, VIM_RIGHT]),
            "Same as %{keys} (type-to-search off)",
        )
        .mentioning(Mention::Keys(Any(&[LEFT, DOWN, UP, RIGHT]))),
        within("%{keys} at top", One(UP), "Focus the navigation header"),
        within(
            "%{keys} in header",
            Any(&[LEFT, RIGHT]),
            "Move between header controls",
        ),
        within("%{keys} in header", One(DOWN), "Return to the files"),
        within("%{keys} in sidebar", One(RIGHT), "Return to the browser"),
        within(
            "%{keys} at sidebar top",
            One(UP),
            "Focus the top navigation bar",
        ),
        within(
            "%{keys} in top bar",
            Any(&[LEFT, RIGHT]),
            "Move between top-bar controls",
        ),
        within(
            "%{keys} in top bar",
            One(DOWN),
            "Return to the sidebar, or files when hidden",
        ),
        row(
            Any(&[HISTORY_BACK, HISTORY_FORWARD]),
            "Back / forward in history",
        ),
        row(One(PARENT_FOLDER), "Go to the parent folder"),
        row(One(HOME_FOLDER), "Go to Home"),
        row(Any(&[FIRST_ITEM, LAST_ITEM]), "First / last item"),
        row(Any(&[FIRST_ITEM_UP, LAST_ITEM_DOWN]), "First / last item"),
        row(Any(&[PAGE_UP, PAGE_DOWN]), "Move one page"),
        row(
            Any(&[FOCUS_NEXT, FOCUS_PREVIOUS]),
            "Next / previous interface control",
        ),
    ]);
    shortcuts
}

fn tenxer_navigation(mode: BrowserMode, chooser: bool) -> Vec<Shortcut> {
    let mut shortcuts = match mode {
        BrowserMode::Columns | BrowserMode::List => vec![
            row(Any(&[VIM_DOWN, VIM_UP, UP, DOWN]), "Next / previous item"),
            row(
                Any(&[VIM_LEFT, LEFT, BACKSPACE, PARENT_FOLDER]),
                "Go to the parent folder",
            ),
            row(Any(&[VIM_RIGHT, RIGHT]), "Open the focused directory"),
            within(
                "%{keys} on a file",
                Any(&[VIM_RIGHT, RIGHT]),
                "Enter its preview; keys move into the drawer",
            ),
        ],
        BrowserMode::Icons => vec![
            row(
                Any(&[VIM_LEFT, VIM_DOWN, VIM_UP, VIM_RIGHT, UP, DOWN, LEFT, RIGHT]),
                "Move spatially between tiles",
            ),
            row(Any(&[BACKSPACE, PARENT_FOLDER]), "Go to the parent folder"),
            row(
                Any(&[
                    TENXER_THUMBNAILS_LARGER,
                    TENXER_THUMBNAILS_SMALLER,
                    THUMBNAILS_LARGER,
                    THUMBNAILS_SMALLER,
                ]),
                "Increase / decrease thumbnail size",
            ),
            row(One(THUMBNAILS_RESET), "Reset thumbnail size"),
        ],
    };
    // The chooser refuses i, and its own section describes Enter and o.
    if !chooser {
        shortcuts.extend_from_slice(&[
            row(
                One(PEEK),
                if mode == BrowserMode::Columns {
                    "Open the next column for the focused directory"
                } else {
                    "Toggle folder peek for the focused directory"
                },
            ),
            row(Any(&[ENTER, OPEN]), "Open the focused item"),
        ]);
    }
    shortcuts.extend_from_slice(&[
        row(
            Any(&[
                TENXER_HISTORY_BACK,
                TENXER_HISTORY_FORWARD,
                HISTORY_BACK,
                HISTORY_FORWARD,
            ]),
            "Back / forward in history",
        ),
        row(One(HOME_FOLDER), "Go to Home"),
        row(One(FIRST_ITEM), "First item"),
        row(Any(&[TENXER_LAST_ITEM, LAST_ITEM]), "Last item"),
        row(Any(&[FIRST_ITEM_UP, LAST_ITEM_DOWN]), "First / last item"),
        row(Any(&[HALF_PAGE_UP, HALF_PAGE_DOWN]), "Move half a page"),
        row(
            Any(&[TENXER_PAGE_UP, TENXER_PAGE_DOWN, PAGE_UP, PAGE_DOWN]),
            "Move one page",
        ),
    ]);
    shortcuts
}

fn tenxer_places(chooser: bool) -> Vec<Shortcut> {
    let mut shortcuts = vec![
        row(Chord(&[GO, GO_FIRST]), "First item"),
        row(Chord(&[GO, GO_FOLLOW]), "Follow search result"),
        row(
            Mixed(&[Chord(&[GO, GO_HOME]), Chord(&[GO, GO_CONFIG])]),
            "Home / ~/.config",
        ),
        row(
            Mixed(&[
                Chord(&[GO, GO_DOWNLOADS]),
                Chord(&[GO, GO_DOCUMENTS]),
                Chord(&[GO, GO_MUSIC]),
                Chord(&[GO, GO_PICTURES]),
                Chord(&[GO, GO_VIDEOS]),
            ]),
            "Downloads / Documents / Music / Pictures / Videos",
        ),
    ];
    if chooser {
        shortcuts.extend_from_slice(&[
            row(Chord(&[GO, GO_RECENT]), "Recent"),
            row(
                Keys::Digits(PIN_NUMBERS),
                "Visible PINNED rows that are local folders",
            ),
        ]);
    } else {
        shortcuts.extend_from_slice(&[
            row(
                Mixed(&[
                    Chord(&[GO, GO_TRASH]),
                    Chord(&[GO, GO_NETWORK]),
                    Chord(&[GO, GO_RECENT]),
                ]),
                "Trash / Network / Recent",
            ),
            row(
                Keys::Digits(PIN_NUMBERS),
                "Visible PINNED rows in sidebar order",
            ),
            row(
                Mixed(&[Chord(&[GO, GO_PIN]), Chord(&[GO, GO_UNPIN])]),
                "Pin / unpin a folder",
            ),
            row(One(BOOKMARK_SET), "Save current folder under a letter"),
            row(One(BOOKMARK_GO), "Jump to a bookmarked folder by letter"),
        ]);
    }
    shortcuts.extend_from_slice(&[
        row(
            Chord(&[GO, GO_PROMPT]),
            "Go to a folder, typed path, or URI",
        ),
        within(
            "%{keys} in %{go} / %{move_to} / %{copy_to}",
            One(COMPLETE),
            "Write the chosen folder into the prompt",
        ),
        row(One(JUMP_VISITED), "Jump to a visited folder"),
        row(One(JUMP_RECENT), "Jump to a recent folder"),
        within(
            "%{keys} in %{jump} / %{recent}",
            Any(&[UP, DOWN]),
            "Choose a visited folder",
        ),
        row(Chord(&[GO, ESCAPE]), "Cancel a pending chord"),
    ]);
    shortcuts
}

fn tenxer_preview(mode: BrowserMode, chooser: bool) -> Vec<Shortcut> {
    let mut shortcuts = Vec::new();
    if !chooser {
        shortcuts.push(within(
            "%{keys} on a file",
            One(PEEK),
            "Toggle the preview without taking focus",
        ));
    }
    shortcuts.push(row(
        Any(&[SCROLL_PREVIEW_DOWN, SCROLL_PREVIEW_UP]),
        "Scroll the open preview without taking focus",
    ));
    shortcuts.push(row(
        Any(&[TENXER_MEDIA_PREVIOUS, TENXER_MEDIA_NEXT]),
        "Previous / next file of the same type while the preview shows audio or video",
    ));
    // Icons have no key that moves into the preview.
    if mode != BrowserMode::Icons {
        shortcuts.extend_from_slice(TENXER_PREVIEW_OWNED);
        if !chooser {
            shortcuts.push(within(
                "%{keys} in the preview",
                One(TENXER_FIND_IN_PREVIEW),
                "Find in preview",
            ));
        }
    }
    shortcuts
}

const TENXER_PREVIEW_OWNED: &[Shortcut] = &[
    within(
        "%{keys} in the preview",
        Any(&[VIM_DOWN, VIM_UP, UP, DOWN]),
        "Scroll the document",
    ),
    within(
        "%{keys} in the preview",
        Any(&[HALF_PAGE_UP, HALF_PAGE_DOWN]),
        "Scroll half a page",
    ),
    within(
        "%{keys} in the preview",
        Any(&[TENXER_PAGE_UP, TENXER_PAGE_DOWN, PAGE_UP, PAGE_DOWN]),
        "Scroll one page",
    ),
    within(
        "%{keys} in the preview",
        TOP_AND_BOTTOM,
        "Top / bottom of the document",
    ),
    within(
        "%{keys} in an archive",
        Any(&[VIM_DOWN, VIM_UP, VIM_RIGHT, ENTER]),
        "Move / open an archive folder",
    ),
    within(
        "%{keys} in an archive",
        TOP_AND_BOTTOM,
        "First / last archive member",
    ),
    within(
        "%{keys} in an archive",
        One(VIM_LEFT),
        "Archive parent; at the root, back to the listing",
    ),
    within("%{keys} in media", One(SPACE), "Play / pause"),
    within(
        "%{keys} in media",
        Any(&[LEFT, RIGHT]),
        "Seek −5 / +5 seconds",
    ),
    within(
        "%{keys} in media",
        Any(&[
            MEDIA_SEEK_BACK_PAGE,
            MEDIA_SEEK_FORWARD_PAGE,
            PAGE_DOWN,
            PAGE_UP,
        ]),
        "Seek −30 / +30 seconds",
    ),
    within(
        "%{keys} in media",
        Any(&[MEDIA_SEEK_START, FIRST_ITEM, TENXER_LAST_ITEM, LAST_ITEM]),
        "Seek to the start / end",
    ),
    within(
        "%{keys} in media",
        Any(&[UP, DOWN, MEDIA_VOLUME_STEP_UP, MEDIA_VOLUME_STEP_DOWN]),
        "Volume up / down",
    ),
    within("%{keys} in media", One(TENXER_MEDIA_MUTE), "Mute / unmute"),
    within(
        "%{keys} in a document",
        Any(&[VIM_LEFT, LEFT]),
        "Return to the listing; the preview stays open",
    ),
    within(
        "%{keys} in media",
        One(VIM_LEFT),
        "Return to the listing; the preview stays open",
    ),
    within(IN_ANY_PREVIEW, One(FOCUS_PREVIOUS), "Return to the listing"),
    within(
        "%{keys} in the preview",
        Any(&[ESCAPE, PEEK]),
        "Close the preview and return to the listing",
    ),
];

const TOP_AND_BOTTOM: Keys = Mixed(&[
    Chord(&[GO, GO_FIRST]),
    One(FIRST_ITEM),
    One(TENXER_LAST_ITEM),
    One(LAST_ITEM),
]);

const DEFAULT_FILES: &[Shortcut] = &[
    row(One(ENTER), "Open the current item"),
    row(One(SPACE), "Preview a file, or open a folder"),
    row(Any(&[COPY, CUT]), "Copy / cut selected items"),
    row(One(PASTE), "Paste into the indicated directory"),
    row(One(DUPLICATE), "Duplicate selected items"),
    row(One(DELETE), "Move selected items to Trash, when supported"),
    row(One(DELETE_PERMANENTLY), "Permanently delete selected items"),
    row(One(UNDO), "Undo the last file operation"),
    row(Any(&[REDO, REDO_Y]), "Redo the last file operation"),
    row(Any(&[RENAME, RENAME_R]), "Rename"),
    row(One(NEW_FOLDER), "Create a folder"),
    row(
        One(NEW_FOLDER_WITH_SELECTION),
        "Create a folder containing the selection",
    ),
    row(One(SELECT_ALL), "Select all items in the focused pane"),
    row(Any(&[EXTEND_UP, EXTEND_DOWN]), "Extend selection"),
    row(
        Any(&[EXTEND_PAGE_UP, EXTEND_PAGE_DOWN]),
        "Extend selection by one page",
    ),
    row(
        One(SELECTION_TOGGLE),
        "Toggle the focused item in the selection",
    ),
    row(One(PROPERTIES), "Show item properties"),
    row(
        Any(&[OPEN_IN_NEW_TAB, OPEN_IN_NEW_WINDOW]),
        "Open the focused folder in a new tab / window",
    ),
    row(
        Any(&[CONTEXT_MENU, CONTEXT_MENU_F10]),
        "Open the context menu",
    ),
    row(
        Any(&[COPY_PATH, PIN]),
        "Copy path / pin a folder (type-to-search off)",
    ),
];

const TRASH_AFTER_CONFIRMING: Shortcut = row(
    Any(&[TENXER_DELETE, DELETE]),
    "Move to Trash after confirming; %{keys} confirms",
)
.mentioning(Mention::Keys(Chord(&[
    TENXER_DELETE,
    TENXER_DELETE_CONFIRM,
])));

const TENXER_FILES: &[Shortcut] = &[
    row(
        Any(&[YANK, YANK_CUT]),
        "Yank / cut the selection, or the focused item",
    ),
    row(Any(&[UNYANK, UNYANK_X]), "Clear copy and cut marks"),
    row(
        One(PASTE_KEEP_BOTH),
        "Paste; Keep Both is focused on conflicts",
    ),
    row(
        Any(&[PASTE_REPLACE, PASTE]),
        "Paste; Replace is focused on conflicts",
    ),
    TRASH_AFTER_CONFIRMING,
    row(
        Any(&[TENXER_DELETE_PERMANENTLY, DELETE_PERMANENTLY]),
        "Delete permanently after confirming",
    ),
    row(One(CREATE), "Create a file; end with / for a folder"),
    row(COPY_PATH_OR_NAME, "Copy path / name"),
    row(Any(&[COPY, CUT]), "Copy / cut selected items"),
    row(One(UNDO), "Undo the last file operation"),
    row(Any(&[REDO, REDO_Y]), "Redo the last file operation"),
    row(
        Any(&[TENXER_RENAME, RENAME]),
        "Rename the focused item in the footer",
    ),
    row(
        One(OPEN_WITH),
        "Open With for the selection, or the focused item",
    ),
    row(
        Keys::Digits(ACTIONS_NUMBERS),
        "Run one of the first ten matching custom actions",
    ),
    row(
        Chord(&[ACTIONS, ACTIONS_TERMINAL]),
        "Open a terminal in the focused folder",
    ),
    row(
        Chord(&[ACTIONS, ACTIONS_COMPRESS]),
        "Compress the selection, or the focused item",
    ),
    row(
        Mixed(&[
            Chord(&[ACTIONS, ACTIONS_EXTRACT]),
            Chord(&[ACTIONS, ACTIONS_EXTRACT_TO]),
        ]),
        "Extract the archive here / to a typed folder",
    ),
    row(Any(&[MOVE_TO, COPY_TO]), "Move / copy to a typed folder"),
    row(One(RESTORE), "Restore from Trash"),
    row(One(NEW_FOLDER), "Create a folder"),
    row(
        One(NEW_FOLDER_WITH_SELECTION),
        "Create a folder containing the selection",
    ),
    row(One(SPACE), "Toggle the focused item and move down"),
    row(
        Any(&[VISUAL_SELECT, VISUAL_UNSET]),
        "Visual select / visual unset",
    ),
    row(One(SELECT_ALL), "Select all items in the focused pane"),
    row(One(INVERT_SELECTION), "Invert the selection"),
    row(One(PROPERTIES), "Show item properties"),
    row(
        Any(&[OPEN_IN_NEW_TAB, OPEN_IN_NEW_WINDOW]),
        "Open the focused folder in a new tab / window",
    ),
    row(
        Any(&[CONTEXT_MENU, CONTEXT_MENU_F10]),
        "Open the context menu",
    ),
];

const COPY_PATH_OR_NAME: Keys = Mixed(&[
    Chord(&[COPY_AS, COPY_AS_PATH]),
    Chord(&[COPY_AS, COPY_AS_NAME]),
]);

fn chooser_requests(chooser: ChooserScope) -> Vec<Shortcut> {
    let mut shortcuts = match chooser.request {
        ChooserRequest::Files => vec![
            within(
                "%{keys} on a file",
                Any(&[ENTER, OPEN]),
                if chooser.multiple {
                    "Choose the filled items, or the file"
                } else {
                    "Choose the file"
                },
            ),
            within(
                "%{keys} on a folder",
                Any(&[ENTER, OPEN]),
                "Open the folder",
            ),
        ],
        ChooserRequest::Folders => vec![
            within(
                "%{keys} on a folder",
                Any(&[ENTER, OPEN]),
                "Open the folder",
            ),
            row(One(CHOOSE_FOLDER), "Choose a folder, as Accept does"),
        ],
        ChooserRequest::SaveFile => vec![
            row(One(ENTER), "Save the name in the current folder"),
            within(
                "%{keys} on a file",
                One(OPEN),
                "Save over that file after confirming",
            ),
            within("%{keys} on a folder", One(OPEN), "Open the folder"),
            row(
                Any(&[TENXER_RENAME, RENAME]),
                "Edit the name; %{keys} returns to the files",
            )
            .mentioning(Mention::Keys(One(ESCAPE))),
        ],
        ChooserRequest::SaveFiles => vec![
            row(One(ENTER), "Save the files in the current folder"),
            within("%{keys} on a folder", One(OPEN), "Open the folder"),
        ],
    };
    shortcuts.push(row(
        One(ESCAPE),
        "Dismiss one interaction, then cancel the request",
    ));
    shortcuts
}

fn chooser_files(chooser: ChooserScope) -> Vec<Shortcut> {
    let mut shortcuts = vec![
        TRASH_AFTER_CONFIRMING,
        row(
            Any(&[TENXER_DELETE_PERMANENTLY, DELETE_PERMANENTLY]),
            "Delete permanently after confirming",
        ),
        row(One(CREATE), "Create a file; end with / for a folder"),
        row(COPY_PATH_OR_NAME, "Copy path / name"),
    ];
    if chooser.request != ChooserRequest::SaveFile {
        shortcuts.push(row(
            Any(&[TENXER_RENAME, RENAME]),
            "Rename the focused item in the footer",
        ));
    }
    shortcuts.push(row(One(NEW_FOLDER), "Create a folder"));
    if chooser.multiple {
        shortcuts.extend_from_slice(&[
            row(One(SPACE), "Toggle the focused item and move down"),
            row(
                Any(&[VISUAL_SELECT, VISUAL_UNSET]),
                "Visual select / visual unset",
            ),
            row(One(SELECT_ALL), "Select all items in the focused pane"),
            row(One(INVERT_SELECTION), "Invert the selection"),
        ]);
    }
    shortcuts.extend_from_slice(&[
        row(One(PROPERTIES), "Show item properties"),
        row(
            Any(&[CONTEXT_MENU, CONTEXT_MENU_F10]),
            "Open the context menu",
        ),
    ]);
    shortcuts
}

/// 10xer rows that a default row already describes with other keys or context.
const SHARED_WITH_DEFAULT: [(Keys, Option<&str>); 3] = [
    (One(FIRST_ITEM), None),
    (One(CHOOSE_FOLDER), None),
    (One(FOCUS_PREVIOUS), Some(IN_ANY_PREVIEW)),
];

fn tenxer_mode_rows(
    mode: BrowserMode,
    chooser: bool,
    sections: &[ReferenceSection],
) -> Vec<Shortcut> {
    let mut shortcuts = Vec::new();
    if !chooser {
        shortcuts.push(row(One(CLOSE_WINDOW), "Close the current window"));
    }
    shortcuts.extend_from_slice(&[
        row(One(TENXER_MODE), "Toggle 10xer mode"),
        row(
            Any(&[REFERENCE, TENXER_REFERENCE]),
            "Show or hide this reference",
        ),
    ]);
    let default = default_sections(mode);
    for &shortcut in sections.iter().flat_map(|section| &section.rows) {
        let keys = (shortcut.keys, shortcut.context);
        // Keep grouped aliases together when they include a 10xer binding.
        // Space is shared, but its selection action replaces ordinary preview.
        let shared = keys != (One(SPACE), None)
            && (default
                .iter()
                .flat_map(|section| &section.rows)
                .any(|default| (default.keys, default.context) == keys)
                || SHARED_WITH_DEFAULT.contains(&keys));
        if !shared && !shortcuts.contains(&shortcut) {
            shortcuts.push(shortcut);
        }
    }
    shortcuts
}

const DEFAULT_TOOLS: &[Shortcut] = &[
    row(One(FILTER), "Filter the current pane"),
    row(One(SEARCH), "Open global search"),
    row(One(JUMP_TO_FOLDER), "Jump to a recent folder"),
    row(
        One(OPEN_CONTAINING_FOLDER),
        "Open containing folder (global search)",
    ),
    row(One(LOCATION), "Edit the location"),
    row(One(TAB_NEW), "New tab"),
    row(One(TAB_CLOSE), "Close the active tab"),
    row(One(TAB_REOPEN), "Reopen the last closed tab"),
    row(Any(&[TAB_NEXT, TAB_PREVIOUS]), "Next / previous tab"),
    row(
        Any(&[TAB_PREVIOUS_PAGE_UP, TAB_NEXT_PAGE_DOWN]),
        "Previous / next tab",
    ),
    row(
        Any(&[TAB_MOVE_LEFT, TAB_MOVE_RIGHT]),
        "Move the active tab left / right",
    ),
    SELECT_TAB,
    row(One(TERMINAL), "Open a terminal"),
    row(One(REFRESH), "Refresh"),
    row(
        Any(&[HIDDEN_FILES, HIDDEN_FILES_PERIOD]),
        "Show or hide hidden files",
    ),
    row(VIEW_MODES, "Switch to Columns, Icons, or List"),
    row(TEXT_SIZE, "Increase / decrease / reset text size"),
    row(One(SIDEBAR), "Show or hide the sidebar"),
    row(
        One(SIDEBAR_FOCUS),
        "Switch focus between sidebar and browser",
    ),
    row(One(ARROW_SCOPE), "Keep arrows in the file list on or off"),
    row(One(SETTINGS), "Open Settings"),
    row(One(TENXER_MODE), "Turn on 10xer mode"),
    row(
        One(ESCAPE),
        "Close preview or cancel the current interaction",
    ),
    row(One(REFERENCE), "Show or hide this reference"),
];

const SELECT_TAB: Shortcut = row(
    Keys::Digits(TAB_NUMBERS),
    "Select a tab (hold %{keys} for numbers)",
)
.mentioning(Mention::Held(TAB_NUMBERS.modifiers()));

const VIEW_MODES: Keys = Any(&[VIEW_COLUMNS, VIEW_ICONS, VIEW_LIST]);

const TEXT_SIZE: Keys = Any(&[TEXT_LARGER, TEXT_SMALLER, TEXT_RESET]);

fn tenxer_tools(mode: BrowserMode, chooser: bool) -> Vec<Shortcut> {
    let mut shortcuts = vec![
        row(
            Any(&[FIND, FIND_BACKWARD]),
            "Find the next / previous name in this listing",
        ),
        row(
            Any(&[REPEAT_FIND, REPEAT_FIND_REVERSED]),
            "Repeat the last find / in reverse",
        ),
        within(
            "%{keys} in a prompt",
            Any(&[UP, DOWN]),
            "Move through the listing",
        ),
        within(
            "%{keys} in a prompt",
            Any(&[ENTER, ESCAPE]),
            "Apply / cancel it",
        ),
        within(
            "%{keys} after a find",
            One(ESCAPE),
            "Dismiss the find highlights",
        ),
        row(One(FILTER_LISTING), "Filter this listing"),
        within("%{keys} after a filter", One(ESCAPE), "Clear the filter"),
        row(
            One(SEARCH_SUBFOLDERS),
            "Search this folder and its subfolders",
        ),
        within(
            "%{keys} after a search",
            // Icons h and ← move between result icons instead.
            if mode == BrowserMode::Icons {
                One(ESCAPE)
            } else {
                Any(&[ESCAPE, VIM_LEFT, LEFT])
            },
            "Dismiss the hits",
        ),
        row(One(FOCUS_NEXT), "Focus the window header"),
        within(
            "%{keys} in the header",
            Any(&[ENTER, SPACE]),
            "Activate the focused control",
        ),
        within(
            "%{keys} in the header",
            Any(&[VIM_LEFT, VIM_DOWN]),
            "Return to the files",
        ),
        row(One(TENXER_SIDEBAR), "Show or hide the sidebar"),
        row(
            One(SIDEBAR_FOCUS),
            "Focus the visible sidebar, or return to the files",
        ),
        within(
            "%{keys} in the sidebar",
            Any(&[VIM_DOWN, VIM_UP, UP, DOWN]),
            "Move between places and device controls",
        ),
        within(
            "%{keys} in the sidebar",
            Any(&[VIM_RIGHT, ENTER, SPACE]),
            "Activate the focused place or device control",
        ),
        within(
            "%{keys} in the sidebar",
            Any(&[VIM_LEFT, LEFT, BACKSPACE]),
            "Return to the files",
        ),
    ];
    if !chooser {
        shortcuts.extend_from_slice(&[
            row(
                Mixed(&[
                    Chord(&[TAB_CHORD, TAB_CHORD_NEW]),
                    Chord(&[TAB_CHORD, TAB_CHORD_CLOSE]),
                ]),
                "New / close tab",
            ),
            row(Chord(&[TAB_CHORD, TAB_CHORD_PREVIOUS]), "Previous tab"),
            row(Keys::Digits(TAB_CHORD_NUMBERS), "Select tab 1–9 / 10"),
            row(One(TAB_NEW), "New tab"),
            row(One(TAB_CLOSE), "Close the active tab"),
            row(One(TAB_REOPEN), "Reopen the last closed tab"),
            row(Any(&[TAB_NEXT, TAB_PREVIOUS]), "Next / previous tab"),
            row(
                Any(&[TAB_PREVIOUS_PAGE_UP, TAB_NEXT_PAGE_DOWN]),
                "Previous / next tab",
            ),
            row(
                Any(&[TAB_MOVE_LEFT, TAB_MOVE_RIGHT]),
                "Move the active tab left / right",
            ),
            SELECT_TAB,
            row(One(SEARCH), "Open global search"),
            row(
                One(OPEN_CONTAINING_FOLDER),
                "Open containing folder (global search)",
            ),
        ]);
    }
    shortcuts.extend_from_slice(&[
        row(One(LOCATION), "Edit the location"),
        row(One(REFRESH), "Refresh"),
        row(
            Any(&[TENXER_HIDDEN_FILES, HIDDEN_FILES, HIDDEN_FILES_PERIOD]),
            "Show or hide hidden files",
        ),
        row(
            Mixed(&[
                Chord(&[SORT_BY, SORT_BY_NAME]),
                Chord(&[SORT_BY, SORT_BY_MODIFIED]),
                Chord(&[SORT_BY, SORT_BY_SIZE]),
                Chord(&[SORT_BY, SORT_BY_TYPE]),
            ]),
            "Sort by name / modified / size / type; %{keys} reverses",
        )
        .mentioning(Mention::Held(Modifiers::SHIFT_MASK)),
        row(VIEW_MODES, "Switch to Columns, Icons, or List"),
        row(TEXT_SIZE, "Increase / decrease / reset text size"),
    ]);
    if !chooser {
        shortcuts.push(row(One(SETTINGS), "Open Settings"));
    }
    shortcuts.extend_from_slice(&[
        row(
            One(ESCAPE),
            "Close this reference or cancel the current interaction",
        ),
        row(
            Any(&[REFERENCE, TENXER_REFERENCE]),
            "Show or hide this reference",
        ),
    ]);
    shortcuts
}

const MEDIA: &[Shortcut] = &[
    row(One(MEDIA_PLAY_PAUSE), "Play / pause"),
    row(
        Any(&[MEDIA_SEEK_BACK, MEDIA_SEEK_FORWARD]),
        "Seek −5 / +5 seconds",
    ),
    row(
        Any(&[MEDIA_VOLUME_UP, MEDIA_VOLUME_DOWN]),
        "Volume up / down",
    ),
    row(One(MEDIA_MUTE), "Mute / unmute"),
    row(
        Any(&[MEDIA_PREVIOUS, MEDIA_NEXT]),
        "Previous / next file of the same type",
    ),
    within(
        "%{keys} on a video",
        One(ENTER),
        "Open it in the default app where the preview stopped",
    ),
];

fn default_hint(hint: ContextHint, type_to_search: bool) -> Option<Keys> {
    Some(One(match hint {
        ContextHint::None
        | ContextHint::MoveTo
        | ContextHint::CopyTo
        | ContextHint::Restore
        | ContextHint::RemoveFromRecent => return None,
        ContextHint::CopyPath | ContextHint::CopyPaths | ContextHint::Pin if type_to_search => {
            return None;
        }
        ContextHint::Preview | ContextHint::ChooserPreview => SPACE,
        ContextHint::CopyPath | ContextHint::CopyPaths => COPY_PATH,
        ContextHint::Pin => PIN,
        ContextHint::Rename => return Some(Any(&[RENAME, RENAME_R])),
        ContextHint::Cut => CUT,
        ContextHint::Copy => COPY,
        ContextHint::Duplicate => DUPLICATE,
        ContextHint::Paste => PASTE,
        ContextHint::Terminal => TERMINAL,
        ContextHint::Trash => DELETE,
        ContextHint::PermanentDelete => DELETE_PERMANENTLY,
        ContextHint::Open => ENTER,
        ContextHint::OpenInNewTab => OPEN_IN_NEW_TAB,
        ContextHint::OpenInNewWindow => OPEN_IN_NEW_WINDOW,
        ContextHint::Properties => PROPERTIES,
        ContextHint::ContainingFolder => OPEN_CONTAINING_FOLDER,
        ContextHint::NewFolder => NEW_FOLDER,
        ContextHint::SelectAll => SELECT_ALL,
        ContextHint::Refresh => REFRESH,
        ContextHint::HiddenFiles => HIDDEN_FILES,
    }))
}

fn tenxer_hint(hint: ContextHint) -> Option<Keys> {
    Some(One(match hint {
        ContextHint::Preview => PEEK,
        ContextHint::Cut => YANK_CUT,
        ContextHint::Copy => YANK,
        ContextHint::Paste => PASTE_KEEP_BOTH,
        ContextHint::Trash => TENXER_DELETE,
        ContextHint::PermanentDelete => TENXER_DELETE_PERMANENTLY,
        ContextHint::MoveTo => MOVE_TO,
        ContextHint::CopyTo => COPY_TO,
        ContextHint::Restore => RESTORE,
        ContextHint::Rename => TENXER_RENAME,
        ContextHint::None
        | ContextHint::ChooserPreview
        | ContextHint::CopyPath
        | ContextHint::CopyPaths
        | ContextHint::Duplicate
        | ContextHint::Pin
        | ContextHint::Terminal => return None,
        other => return default_hint(other, false),
    }))
}

/// The distinct rows of one map across all its scopes, each as the keys it
/// shows: its own and those its action names.
#[cfg(test)]
pub(in crate::ui) fn reference_rows(tenxer: bool) -> Vec<Vec<Keys>> {
    let mut sections = Vec::new();
    for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
        if !tenxer {
            sections.extend(default_sections(mode));
            continue;
        }
        sections.extend(tenxer_sections(mode, None));
        for request in [
            ChooserRequest::Files,
            ChooserRequest::Folders,
            ChooserRequest::SaveFile,
            ChooserRequest::SaveFiles,
        ] {
            for multiple in [false, true] {
                sections.extend(tenxer_sections(
                    mode,
                    Some(ChooserScope { request, multiple }),
                ));
            }
        }
    }
    let mut rows: Vec<Shortcut> = Vec::new();
    for shortcut in sections.into_iter().flat_map(|section| section.rows) {
        if !rows.contains(&shortcut) {
            rows.push(shortcut);
        }
    }
    rows.into_iter()
        .map(|shortcut| {
            let mentioned = match shortcut.mention {
                Some(Mention::Keys(keys)) => Some(keys),
                Some(Mention::Held(_)) | None => None,
            };
            std::iter::once(shortcut.keys).chain(mentioned).collect()
        })
        .collect()
}
