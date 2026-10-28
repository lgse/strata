// SPDX-License-Identifier: MIT

use gtk::{gdk::ModifierType as Modifiers, prelude::*};

use super::{
    browser_modes::BrowserMode,
    keys::{
        self, Binding, Keys,
        Keys::{Any, Chord, Mixed, One},
        bindings::*,
        shown,
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
    /// The action's own `%{number}`.
    pub number: Option<usize>,
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
            None => match self.number {
                Some(number) => {
                    rust_i18n::t!(self.action, locale = locale, number = number).into_owned()
                }
                None => rust_i18n::t!(self.action, locale = locale).into_owned(),
            },
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
        number: None,
    }
}

const fn within(context: &'static str, keys: Keys, action: &'static str) -> Shortcut {
    Shortcut {
        context: Some(context),
        ..row(keys, action)
    }
}

/// `message` with its `%{keys}` filled in, or its keyless form while
/// nothing in `keys` is bound.
pub(crate) fn with_keys(message: &str, keys: Keys) -> String {
    let label = keys::label(&keys);
    if label.is_empty()
        && let Some((_, keyless)) = KEYLESS.iter().find(|(keyed, _)| *keyed == message)
    {
        return if keyless.is_empty() {
            String::new()
        } else {
            crate::i18n::tr(keyless)
        };
    }
    rust_i18n::t!(message, keys = label).into_owned()
}

/// What a message that names keys says once they are unbound; empty hides it.
const KEYLESS: &[(&str, &str)] = &[
    ("Back (%{keys})", "Back"),
    ("Cancel (%{keys})", "Cancel"),
    ("Cancel file selection (%{keys})", "Cancel file selection"),
    ("Close preview (%{keys})", "Close preview"),
    ("Close tab (%{keys})", "Close tab"),
    ("Compact sidebar (%{keys})", "Compact sidebar"),
    ("Decrease text size (%{keys})", "Decrease text size"),
    ("Filter icons (%{keys})", "Filter icons"),
    ("Filter list (%{keys})", "Filter list"),
    ("Filter this pane (%{keys})", "Filter this pane"),
    ("Forward (%{keys})", "Forward"),
    ("Hide sidebar (%{keys})", "Hide sidebar"),
    ("Increase text size (%{keys})", "Increase text size"),
    ("Location (%{keys})", "Location"),
    ("Mute/unmute (%{keys})", "Mute / unmute"),
    ("Navigate (%{keys})", "Navigate"),
    ("New Folder (%{keys})", "New Folder"),
    ("New tab (%{keys})", "New tab"),
    ("Next audio file (%{keys})", "Next audio file"),
    ("Next video (%{keys})", "Next video"),
    ("Parent folder (%{keys})", "Go to the parent folder"),
    ("Play/Pause (%{keys})", "Play / pause"),
    ("Previous audio file (%{keys})", "Previous audio file"),
    ("Previous video (%{keys})", "Previous video"),
    ("Refresh (%{keys})", "Refresh"),
    ("Reset text size (%{keys})", "Reset text size"),
    ("Search (%{keys})", "Search"),
    ("Select folder (%{keys})", "Select folder"),
    (
        "Show all file-view shortcuts (%{keys})",
        "Show all file-view shortcuts",
    ),
    ("Show sidebar (%{keys})", "Show sidebar"),
    ("Toggle sidebar (%{keys})", "Toggle sidebar"),
    ("%{keys} Shortcuts", "Shortcuts"),
    ("%{keys}  navigate", ""),
    ("%{keys}  open containing folder", ""),
    ("Press %{keys} to paste into a supported directory.", ""),
];

/// Runs `render` now and whenever the active map or its keys change, for text
/// that shows shortcuts on a widget built once.
pub(crate) fn follow_keys<W: IsA<gtk::Widget>>(widget: &W, render: impl Fn(&W) + 'static) {
    let preferences = super::preferences::PreferenceManager::shared();
    preferences.bind_preference(
        widget,
        super::preferences::PreferenceManager::shown_keys,
        move |widget, _| render(widget.downcast_ref::<W>().expect("widget that shows keys")),
    );
}

/// An icon-only button's tooltip that names its shortcut.
pub(crate) fn follow_tooltip(widget: &impl IsA<gtk::Widget>, message: &'static str, keys: Keys) {
    follow_keys(widget.upcast_ref::<gtk::Widget>(), move |widget| {
        widget.set_tooltip_text(Some(&with_keys(message, keys)));
    });
}

/// An accessible label that names a shortcut.
pub(crate) fn follow_label(widget: &impl IsA<gtk::Widget>, message: &'static str, keys: Keys) {
    follow_keys(widget.upcast_ref::<gtk::Widget>(), move |widget| {
        super::accessibility::set_label(widget, &with_keys(message, keys));
    });
}

/// An accessible description that names a shortcut.
pub(crate) fn follow_description(
    widget: &impl IsA<gtk::Widget>,
    message: &'static str,
    keys: Keys,
) {
    follow_keys(widget.upcast_ref::<gtk::Widget>(), move |widget| {
        super::accessibility::set_description(widget, Some(&with_keys(message, keys)));
    });
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
/// Rows whose keys the user unbound are left out.
pub(crate) fn reference_sections(scope: ReferenceScope) -> Vec<ReferenceSection> {
    let mut sections = if super::tenxer_mode::chrome_suppressed() {
        tenxer_sections(scope.mode, scope.chooser)
    } else {
        default_sections(scope.mode)
    };
    for section in &mut sections {
        section.rows.retain(|row| row.keys.is_bound());
    }
    sections.retain(|section| !section.rows.is_empty());
    sections
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
        One(shown) => shown.primary(),
        Any(shown) => shown.first().and_then(|shown| shown.primary()),
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
        ReferenceSection {
            title: "Custom actions",
            rows: custom_action_rows(),
        },
    ]
}

/// One row per slot; only the slots a user binds show.
fn custom_action_rows() -> Vec<Shortcut> {
    keys::CUSTOM_ACTIONS
        .iter()
        .zip(1..)
        .map(|(slot, number)| Shortcut {
            number: Some(number),
            ..row(One(slot.shown()), CUSTOM_ACTION)
        })
        .collect()
}

/// A custom action slot's name, in the reference and in Settings.
pub(crate) const CUSTOM_ACTION: &str = "Custom action %{number}";

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
        ],
        BrowserMode::Icons => vec![
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
        ],
        BrowserMode::List => vec![
            row(
                Any(shown![UP.alternate(0), DOWN.alternate(0)]),
                "Move between file rows",
            ),
            row(One(LEFT.alternate(0).shown()), "Focus the visible sidebar"),
            row(One(BACKSPACE.shown()), "Go to the parent folder"),
        ],
    };
    shortcuts.extend_from_slice(&[
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
    ]);
    shortcuts
}

fn tenxer_navigation(mode: BrowserMode, chooser: bool) -> Vec<Shortcut> {
    let mut shortcuts = match mode {
        BrowserMode::Columns | BrowserMode::List => vec![
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
        ],
        BrowserMode::Icons => vec![
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
        ],
    };
    // The chooser refuses i, and its own section describes Enter and o.
    if !chooser {
        shortcuts.extend_from_slice(&[
            row(
                One(PREVIEW.shown()),
                if mode == BrowserMode::Columns {
                    "Open the next column for the focused directory"
                } else {
                    "Toggle folder peek for the focused directory"
                },
            ),
            row(Any(shown![ENTER, OPEN]), "Open the focused item"),
        ]);
    }
    shortcuts.extend_from_slice(&[
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
    ]);
    shortcuts
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
    if chooser {
        shortcuts.extend_from_slice(&[
            row(Chord(shown![GO, GO_RECENT]), "Recent"),
            row(
                Keys::Digits(PIN_NUMBERS),
                "Visible PINNED rows that are local folders",
            ),
        ]);
    } else {
        shortcuts.extend_from_slice(&[
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
        ]);
    }
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

fn tenxer_preview(mode: BrowserMode, chooser: bool) -> Vec<Shortcut> {
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
    if mode != BrowserMode::Icons {
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

const TRASH_AFTER_CONFIRMING: Shortcut = row(
    One(MOVE_TO_TRASH.shown()),
    "Move to Trash after confirming; %{keys} confirms",
)
.mentioning(Mention::Keys(Chord(shown![
    MOVE_TO_TRASH,
    TENXER_DELETE_CONFIRM,
])));

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
    row(
        One(OPEN_WITH.shown()),
        "Open With for the selection, or the focused item",
    ),
    row(
        Keys::Digits(ACTIONS_NUMBERS),
        "Run one of the first ten matching custom actions",
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

fn chooser_requests(chooser: ChooserScope) -> Vec<Shortcut> {
    let mut shortcuts = match chooser.request {
        ChooserRequest::Files => vec![
            within(
                "%{keys} on a file",
                Any(shown![ENTER, OPEN]),
                if chooser.multiple {
                    "Choose the filled items, or the file"
                } else {
                    "Choose the file"
                },
            ),
            within(
                "%{keys} on a folder",
                Any(shown![ENTER, OPEN]),
                "Open the folder",
            ),
        ],
        ChooserRequest::Folders => vec![
            within(
                "%{keys} on a folder",
                Any(shown![ENTER, OPEN]),
                "Open the folder",
            ),
            row(
                One(CHOOSE_FOLDER.shown()),
                "Choose a folder, as Accept does",
            ),
        ],
        ChooserRequest::SaveFile => vec![
            row(One(ENTER.shown()), "Save the name in the current folder"),
            within(
                "%{keys} on a file",
                One(OPEN.shown()),
                "Save over that file after confirming",
            ),
            within("%{keys} on a folder", One(OPEN.shown()), "Open the folder"),
            row(
                One(RENAME.shown()),
                "Edit the name; %{keys} returns to the files",
            )
            .mentioning(Mention::Keys(One(ESCAPE.shown()))),
        ],
        ChooserRequest::SaveFiles => vec![
            row(One(ENTER.shown()), "Save the files in the current folder"),
            within("%{keys} on a folder", One(OPEN.shown()), "Open the folder"),
        ],
    };
    shortcuts.push(row(
        One(ESCAPE.shown()),
        "Dismiss one interaction, then cancel the request",
    ));
    shortcuts
}

fn chooser_files(chooser: ChooserScope) -> Vec<Shortcut> {
    let mut shortcuts = vec![
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
    ];
    if chooser.request != ChooserRequest::SaveFile {
        shortcuts.push(row(
            One(RENAME.shown()),
            "Rename the focused item in the footer",
        ));
    }
    shortcuts.push(row(One(NEW_FOLDER.shown()), "Create a folder"));
    if chooser.multiple {
        shortcuts.extend_from_slice(&[
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
        ]);
    }
    shortcuts.extend_from_slice(&[
        row(One(PROPERTIES.shown()), "Show item properties"),
        row(One(CONTEXT_MENU.shown()), "Open the context menu"),
    ]);
    shortcuts
}

/// 10xer rows that a default row already describes with other keys or context.
const SHARED_WITH_DEFAULT: [(Keys, Option<&str>); 3] = [
    (One(FIRST_ITEM.shown()), None),
    (One(CHOOSE_FOLDER.shown()), None),
    (One(FOCUS_PREVIOUS.shown()), Some(IN_ANY_PREVIEW)),
];

fn tenxer_mode_rows(
    mode: BrowserMode,
    chooser: bool,
    sections: &[ReferenceSection],
) -> Vec<Shortcut> {
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

const DEFAULT_TOOLS: &[Shortcut] = &[
    row(One(FILTER.shown()), "Filter the current pane"),
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

fn tenxer_tools(mode: BrowserMode, chooser: bool) -> Vec<Shortcut> {
    let mut shortcuts = vec![
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
            "%{keys} after a filter",
            One(ESCAPE.shown()),
            "Clear the filter",
        ),
        row(
            One(SEARCH_SUBFOLDERS.shown()),
            "Search this folder and its subfolders",
        ),
        within(
            "%{keys} after a search",
            // Icons h and ← move between result icons instead.
            if mode == BrowserMode::Icons {
                One(ESCAPE.shown())
            } else {
                Any(shown![ESCAPE, LEFT.alternate(1), LEFT.alternate(0)])
            },
            "Dismiss the hits",
        ),
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
    if !chooser {
        shortcuts.extend_from_slice(&[
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
        ]);
    }
    shortcuts.extend_from_slice(&[
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
        row(VIEW_MODES, "Switch to Columns, Icons, or List"),
        row(TEXT_SIZE, "Increase / decrease / reset text size"),
    ]);
    if !chooser {
        shortcuts.push(row(One(SETTINGS.shown()), "Open Settings"));
    }
    shortcuts.extend_from_slice(&[
        row(
            One(ESCAPE.shown()),
            "Close this reference or cancel the current interaction",
        ),
        row(One(REFERENCE.shown()), "Show or hide this reference"),
    ]);
    shortcuts
}

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

fn default_hint(hint: ContextHint, type_to_search: bool) -> Option<Keys> {
    // Type-to-search takes the plain keys these ship with.
    let typed_only = |command: keys::Command| {
        command
            .bindings_in(keys::Map::Default)
            .iter()
            .all(|binding| binding.is_typed())
    };
    Some(One(match hint {
        ContextHint::None
        | ContextHint::MoveTo
        | ContextHint::CopyTo
        | ContextHint::Restore
        | ContextHint::RemoveFromRecent => return None,
        ContextHint::CopyPath | ContextHint::CopyPaths
            if type_to_search && typed_only(COPY_PATH) =>
        {
            return None;
        }
        ContextHint::Pin if type_to_search && typed_only(PIN) => return None,
        ContextHint::Preview | ContextHint::ChooserPreview => PREVIEW.shown(),
        ContextHint::CopyPath | ContextHint::CopyPaths => COPY_PATH.shown(),
        ContextHint::Pin => PIN.shown(),
        ContextHint::Rename => RENAME.shown(),
        ContextHint::Cut => CUT.shown(),
        ContextHint::Copy => COPY.shown(),
        ContextHint::Duplicate => DUPLICATE.shown(),
        ContextHint::Paste => PASTE.shown(),
        ContextHint::Terminal => TERMINAL.shown(),
        ContextHint::Trash => MOVE_TO_TRASH.shown(),
        ContextHint::PermanentDelete => DELETE_PERMANENTLY.shown(),
        ContextHint::Open => ENTER.shown(),
        ContextHint::OpenInNewTab => OPEN_IN_NEW_TAB.shown(),
        ContextHint::OpenInNewWindow => OPEN_IN_NEW_WINDOW.shown(),
        ContextHint::Properties => PROPERTIES.shown(),
        ContextHint::ContainingFolder => OPEN_CONTAINING_FOLDER.shown(),
        ContextHint::NewFolder => NEW_FOLDER.shown(),
        ContextHint::SelectAll => SELECT_ALL.shown(),
        ContextHint::Refresh => REFRESH.shown(),
        ContextHint::HiddenFiles => HIDDEN_FILES.alternate(0).shown(),
    }))
}

fn tenxer_hint(hint: ContextHint) -> Option<Keys> {
    Some(One(match hint {
        ContextHint::Preview => PREVIEW.shown(),
        ContextHint::Cut => YANK_CUT.shown(),
        ContextHint::Copy => YANK.shown(),
        ContextHint::Paste => PASTE_KEEP_BOTH.shown(),
        ContextHint::Trash => MOVE_TO_TRASH.alternate(0).shown(),
        ContextHint::PermanentDelete => DELETE_PERMANENTLY.alternate(0).shown(),
        ContextHint::MoveTo => MOVE_TO.shown(),
        ContextHint::CopyTo => COPY_TO.shown(),
        ContextHint::Restore => RESTORE.shown(),
        ContextHint::Rename => RENAME.alternate(0).shown(),
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

/// The section titles the reference shows in `map`, in its order, each with
/// the commands its rows name across every browser mode and, in 10xer mode,
/// every file chooser request. The three navigation titles read as one.
pub(crate) fn command_sections(map: keys::Map) -> Vec<(&'static str, Vec<keys::Command>)> {
    let mut merged: Vec<(&'static str, Vec<keys::Command>)> = Vec::new();
    for sections in scoped_sections(map == keys::Map::Tenxer) {
        let mut previous = None;
        for section in sections {
            let title = if NAVIGATION_TITLES.contains(&section.title) {
                NAVIGATION
            } else {
                section.title
            };
            let index = match merged.iter().position(|(merged, _)| *merged == title) {
                Some(index) => index,
                None => {
                    let index = previous.map_or(0, |previous| previous + 1);
                    merged.insert(index, (title, Vec::new()));
                    index
                }
            };
            previous = Some(index);
            let commands = &mut merged[index].1;
            for shortcut in &section.rows {
                let mentioned = match shortcut.mention {
                    Some(Mention::Keys(keys)) => keys.commands(),
                    Some(Mention::Held(_)) | None => Vec::new(),
                };
                for command in shortcut.keys.commands().into_iter().chain(mentioned) {
                    if !commands.contains(&command) {
                        commands.push(command);
                    }
                }
            }
        }
    }
    merged
}

/// The one title [`command_sections`] gives the navigation sections.
const NAVIGATION: &str = "Navigation";

const NAVIGATION_TITLES: [&str; 3] = ["Columns navigation", "Icons navigation", "List navigation"];

/// One map's sections in every scope it has.
fn scoped_sections(tenxer: bool) -> Vec<Vec<ReferenceSection>> {
    let mut scopes = Vec::new();
    for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
        if !tenxer {
            scopes.push(default_sections(mode));
            continue;
        }
        scopes.push(tenxer_sections(mode, None));
        for request in [
            ChooserRequest::Files,
            ChooserRequest::Folders,
            ChooserRequest::SaveFile,
            ChooserRequest::SaveFiles,
        ] {
            for multiple in [false, true] {
                scopes.push(tenxer_sections(
                    mode,
                    Some(ChooserScope { request, multiple }),
                ));
            }
        }
    }
    scopes
}

/// The distinct rows of one map across all its scopes, each as the keys it
/// shows: its own and those its action names.
#[cfg(test)]
pub(in crate::ui) fn reference_rows(tenxer: bool) -> Vec<Vec<Keys>> {
    let mut rows: Vec<Shortcut> = Vec::new();
    for shortcut in scoped_sections(tenxer)
        .into_iter()
        .flatten()
        .flat_map(|section| section.rows)
    {
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
