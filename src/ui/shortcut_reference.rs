// SPDX-License-Identifier: MIT

use super::browser_modes::BrowserMode;

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

pub(crate) struct ReferenceSection {
    pub title: &'static str,
    pub rows: Vec<Shortcut>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Shortcut {
    pub keys: &'static str,
    /// A translatable template that places the untranslated `%{keys}` in context.
    pub context: Option<&'static str>,
    pub action: &'static str,
}

impl Shortcut {
    pub(crate) fn key_text(&self) -> String {
        let Some(context) = self.context else {
            return self.keys.to_owned();
        };
        use super::tenxer_mode::Prompt;
        rust_i18n::t!(
            context,
            keys = self.keys,
            go = Prompt::Go.label(),
            move_to = Prompt::MoveTo.label(),
            copy_to = Prompt::CopyTo.label(),
            jump = Prompt::Jump.label(),
            recent = Prompt::Recent.label()
        )
        .into_owned()
    }
}

const fn row(keys: &'static str, action: &'static str) -> Shortcut {
    Shortcut {
        keys,
        context: None,
        action,
    }
}

const fn within(context: &'static str, keys: &'static str, action: &'static str) -> Shortcut {
    Shortcut {
        keys,
        context: Some(context),
        action,
    }
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
    OpenMultiple,
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
) -> &'static str {
    if preferences.tenxer_mode() {
        tenxer_hint(hint)
    } else {
        default_hint(hint, preferences.type_to_search_active())
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
            row("↑ / ↓", "Move between items"),
            row("← / →", "Parent pane / enter folder"),
            within("%{keys} at first pane", "←", "Focus the visible sidebar"),
            row(
                "Backspace",
                "Close the current pane or go to the parent folder",
            ),
        ],
        BrowserMode::Icons => vec![
            row("↑ ↓ ← →", "Move spatially between tiles"),
            within("%{keys} at left edge", "←", "Focus the visible sidebar"),
            row("Backspace", "Go to the parent folder"),
        ],
        BrowserMode::List => vec![
            row("↑ / ↓", "Move between file rows"),
            row("←", "Focus the visible sidebar"),
            row("Backspace", "Go to the parent folder"),
        ],
    };
    shortcuts.extend_from_slice(&[
        row("h / j / k / l", "Same as ← ↓ ↑ → (type-to-search off)"),
        within("%{keys} at top", "↑", "Focus the navigation header"),
        within("%{keys} in header", "← / →", "Move between header controls"),
        within("%{keys} in header", "↓", "Return to the files"),
        within("%{keys} in sidebar", "→", "Return to the browser"),
        within(
            "%{keys} at sidebar top",
            "↑",
            "Focus the top navigation bar",
        ),
        within(
            "%{keys} in top bar",
            "← / →",
            "Move between top-bar controls",
        ),
        within(
            "%{keys} in top bar",
            "↓",
            "Return to the sidebar, or files when hidden",
        ),
        row("Alt+← / Alt+→", "Back / forward in history"),
        row("Alt+↑", "Go to the parent folder"),
        row("Alt+Home", "Go to Home"),
        row("Home / End", "First / last item"),
        row("Ctrl+↑ / Ctrl+↓", "First / last item"),
        row("PgUp / PgDn", "Move one page"),
        row("Tab / Shift+Tab", "Next / previous interface control"),
    ]);
    shortcuts
}

fn tenxer_navigation(mode: BrowserMode, chooser: bool) -> Vec<Shortcut> {
    let mut shortcuts = match mode {
        BrowserMode::Columns | BrowserMode::List => vec![
            row("j / k / ↑ / ↓", "Next / previous item"),
            row("h / ← / Backspace / Alt+↑", "Go to the parent folder"),
            row("l / →", "Open the focused directory"),
            within(
                "%{keys} on a file",
                "l / →",
                "Enter its preview; keys move into the drawer",
            ),
        ],
        BrowserMode::Icons => vec![
            row("h / j / k / l / ↑ ↓ ← →", "Move spatially between tiles"),
            row("Backspace / Alt+↑", "Go to the parent folder"),
        ],
    };
    // The chooser refuses i, and its own section describes Enter and o.
    if !chooser {
        shortcuts.extend_from_slice(&[
            row(
                "i",
                if mode == BrowserMode::Columns {
                    "Open the next column for the focused directory"
                } else {
                    "Toggle folder peek for the focused directory"
                },
            ),
            row("Enter / o", "Open the focused item"),
        ]);
    }
    shortcuts.extend_from_slice(&[
        row("H / L / Alt+← / Alt+→", "Back / forward in history"),
        row("Alt+Home", "Go to Home"),
        row("Home", "First item"),
        row("G / End", "Last item"),
        row("Ctrl+↑ / Ctrl+↓", "First / last item"),
        row("Ctrl+U / Ctrl+D", "Move half a page"),
        row("Ctrl+B / Ctrl+F / PgUp / PgDn", "Move one page"),
    ]);
    shortcuts
}

fn tenxer_places(chooser: bool) -> Vec<Shortcut> {
    let mut shortcuts = vec![
        row("g g", "First item"),
        row("g f", "Follow search result"),
        row("g h / g c", "Home / ~/.config"),
        row(
            "g d / g k / g m / g p / g v",
            "Downloads / Documents / Music / Pictures / Videos",
        ),
    ];
    if chooser {
        shortcuts.extend_from_slice(&[
            row("g r", "Recent"),
            row("g 1–9", "Visible PINNED rows that are local folders"),
        ]);
    } else {
        shortcuts.extend_from_slice(&[
            row("g t / g n / g r", "Trash / Network / Recent"),
            row("g 1–9", "Visible PINNED rows in sidebar order"),
            row("g + / g -", "Pin / unpin a folder"),
        ]);
    }
    shortcuts.extend_from_slice(&[
        row("g Space", "Go to a folder, typed path, or URI"),
        within(
            "%{keys} in %{go} / %{move_to} / %{copy_to}",
            "Tab",
            "Write the chosen folder into the prompt",
        ),
        row("z", "Jump to a visited folder"),
        row("Z", "Jump to a recent folder"),
        within(
            "%{keys} in %{jump} / %{recent}",
            "↑ / ↓",
            "Choose a visited folder",
        ),
        within("%{keys} after g", "Esc", "Cancel a pending chord"),
    ]);
    shortcuts
}

fn tenxer_preview(mode: BrowserMode, chooser: bool) -> Vec<Shortcut> {
    let mut shortcuts = Vec::new();
    if !chooser {
        shortcuts.push(within(
            "%{keys} on a file",
            "i",
            "Toggle the preview without taking focus",
        ));
    }
    shortcuts.push(row("J / K", "Scroll the open preview without taking focus"));
    shortcuts.push(row(
        "< / >",
        "Previous / next file of the same type while the preview shows audio or video",
    ));
    // Icons have no key that moves into the preview.
    if mode != BrowserMode::Icons {
        shortcuts.extend_from_slice(TENXER_PREVIEW_OWNED);
    }
    shortcuts
}

const TENXER_PREVIEW_OWNED: &[Shortcut] = &[
    within(
        "%{keys} in the preview",
        "j / k / ↑ / ↓",
        "Scroll the document",
    ),
    within(
        "%{keys} in the preview",
        "Ctrl+U / Ctrl+D",
        "Scroll half a page",
    ),
    within(
        "%{keys} in the preview",
        "Ctrl+B / Ctrl+F / PgUp / PgDn",
        "Scroll one page",
    ),
    within(
        "%{keys} in the preview",
        "g g / Home / G / End",
        "Top / bottom of the document",
    ),
    within(
        "%{keys} in an archive",
        "j / k / l / Enter",
        "Move / open an archive folder",
    ),
    within(
        "%{keys} in an archive",
        "g g / Home / G / End",
        "First / last archive member",
    ),
    within(
        "%{keys} in an archive",
        "h",
        "Archive parent; at the root, back to the listing",
    ),
    within(
        "%{keys} in media",
        "Space / ← → / ↑ ↓ / m / < >",
        "Play, seek, volume, mute, previous / next file of the same type",
    ),
    within(
        "%{keys} in a document, h in media",
        "h / ←",
        "Return to the listing; the preview stays open",
    ),
    within(IN_ANY_PREVIEW, "Shift+Tab", "Return to the listing"),
    within(
        "%{keys} in the preview",
        "Esc / i",
        "Close the preview and return to the listing",
    ),
];

const DEFAULT_FILES: &[Shortcut] = &[
    row("Enter", "Open the current item"),
    row("Space", "Preview a file, or open a folder"),
    row("Ctrl+C / Ctrl+X", "Copy / cut selected items"),
    row("Ctrl+V", "Paste into the indicated directory"),
    row("Ctrl+D", "Duplicate selected items"),
    row("Delete", "Move selected items to Trash, when supported"),
    row("Shift+Delete", "Permanently delete selected items"),
    row("Ctrl+Z", "Undo the last file operation"),
    row("Ctrl+Shift+Z / Ctrl+Y", "Redo the last file operation"),
    row("F2 / Ctrl+R", "Rename"),
    row("Ctrl+Shift+N", "Create a folder"),
    row("Ctrl+Alt+N", "Create a folder containing the selection"),
    row("Ctrl+A", "Select all items in the focused pane"),
    row("Shift+↑ / ↓", "Extend selection"),
    row("Shift+PgUp / PgDn", "Extend selection by one page"),
    row("Ctrl+Space", "Toggle the focused item in the selection"),
    row("Alt+Enter", "Show item properties"),
    row(
        "Ctrl+Enter / Shift+Enter",
        "Open the focused folder in a new tab / window",
    ),
    row("Menu / Shift+F10", "Open the context menu"),
    row("y / p", "Copy path / pin a folder (type-to-search off)"),
];

const TENXER_FILES: &[Shortcut] = &[
    row("y / x", "Yank / cut the selection, or the focused item"),
    row("Y / X", "Clear copy and cut marks"),
    row("p", "Paste; Keep Both is focused on conflicts"),
    row("P / Ctrl+V", "Paste; Replace is focused on conflicts"),
    row("d / Delete", "Move to Trash after confirming; d d confirms"),
    row("D / Shift+Delete", "Delete permanently after confirming"),
    row("a", "Create a file; end with / for a folder"),
    row("c c / c n", "Copy path / name"),
    row("Ctrl+C / Ctrl+X", "Copy / cut selected items"),
    row("Ctrl+Z", "Undo the last file operation"),
    row("Ctrl+Shift+Z / Ctrl+Y", "Redo the last file operation"),
    row("r / F2", "Rename the focused item in the footer"),
    row("O", "Open With for the selection, or the focused item"),
    row(
        "; 1–9 / ; 0",
        "Run one of the first ten matching custom actions",
    ),
    row("; t", "Open a terminal in the focused folder"),
    row("; c", "Compress the selection, or the focused item"),
    row("; e / ; E", "Extract the archive here / to a typed folder"),
    row("M / C", "Move / copy to a typed folder"),
    row("R", "Restore from Trash"),
    row("Ctrl+Shift+N", "Create a folder"),
    row("Ctrl+Alt+N", "Create a folder containing the selection"),
    row("Space", "Toggle the focused item and move down"),
    row("v / V", "Visual select / visual unset"),
    row("Ctrl+A", "Select all items in the focused pane"),
    row("Ctrl+R", "Invert the selection"),
    row("Alt+Enter", "Show item properties"),
    row(
        "Ctrl+Enter / Shift+Enter",
        "Open the focused folder in a new tab / window",
    ),
    row("Menu / Shift+F10", "Open the context menu"),
];

fn chooser_requests(chooser: ChooserScope) -> Vec<Shortcut> {
    let mut shortcuts = match chooser.request {
        ChooserRequest::Files => vec![
            within(
                "%{keys} on a file",
                "Enter / o",
                if chooser.multiple {
                    "Choose the filled items, or the file"
                } else {
                    "Choose the file"
                },
            ),
            within("%{keys} on a folder", "Enter / o", "Open the folder"),
        ],
        ChooserRequest::Folders => vec![
            within("%{keys} on a folder", "Enter / o", "Open the folder"),
            row("Ctrl+Enter", "Choose a folder, as Accept does"),
        ],
        ChooserRequest::SaveFile => vec![
            row("Enter", "Save the name in the current folder"),
            within(
                "%{keys} on a file",
                "o",
                "Save over that file after confirming",
            ),
            within("%{keys} on a folder", "o", "Open the folder"),
            row("r / F2", "Edit the name; Esc returns to the files"),
        ],
        ChooserRequest::SaveFiles => vec![
            row("Enter", "Save the files in the current folder"),
            within("%{keys} on a folder", "o", "Open the folder"),
        ],
    };
    shortcuts.push(row(
        "Esc",
        "Dismiss one interaction, then cancel the request",
    ));
    shortcuts
}

fn chooser_files(chooser: ChooserScope) -> Vec<Shortcut> {
    let mut shortcuts = vec![
        row("d / Delete", "Move to Trash after confirming; d d confirms"),
        row("D / Shift+Delete", "Delete permanently after confirming"),
        row("a", "Create a file; end with / for a folder"),
        row("c c / c n", "Copy path / name"),
    ];
    if chooser.request != ChooserRequest::SaveFile {
        shortcuts.push(row("r / F2", "Rename the focused item in the footer"));
    }
    shortcuts.push(row("Ctrl+Shift+N", "Create a folder"));
    if chooser.multiple {
        shortcuts.extend_from_slice(&[
            row("Space", "Toggle the focused item and move down"),
            row("v / V", "Visual select / visual unset"),
            row("Ctrl+A", "Select all items in the focused pane"),
            row("Ctrl+R", "Invert the selection"),
        ]);
    }
    shortcuts.extend_from_slice(&[
        row("Alt+Enter", "Show item properties"),
        row("Menu / Shift+F10", "Open the context menu"),
    ]);
    shortcuts
}

fn tenxer_mode_rows(
    mode: BrowserMode,
    chooser: bool,
    sections: &[ReferenceSection],
) -> Vec<Shortcut> {
    let mut shortcuts = Vec::new();
    if !chooser {
        shortcuts.push(row("Q", "Close the current window"));
    }
    shortcuts.extend_from_slice(&[
        row("Ctrl+Shift+M", "Toggle 10xer mode"),
        row("F1 / ~", "Show or hide this reference"),
    ]);
    let default = default_sections(mode);
    for &shortcut in sections.iter().flat_map(|section| &section.rows) {
        let keys = (shortcut.keys, shortcut.context);
        // Keep grouped aliases together when they include a 10xer binding.
        // Space is shared, but its selection action replaces ordinary preview.
        let shared = keys != ("Space", None)
            && (default
                .iter()
                .flat_map(|section| &section.rows)
                .any(|default| (default.keys, default.context) == keys)
                || matches!(
                    keys,
                    ("Home" | "Esc" | "Ctrl+Enter", None) | ("Shift+Tab", Some(IN_ANY_PREVIEW))
                ));
        if !shared && !shortcuts.contains(&shortcut) {
            shortcuts.push(shortcut);
        }
    }
    shortcuts
}

const DEFAULT_TOOLS: &[Shortcut] = &[
    row("Ctrl+F", "Filter the current pane"),
    row("Ctrl+K", "Open global search"),
    row("Ctrl+Shift+K", "Jump to a recent folder"),
    row("Alt+Enter", "Open containing folder (global search)"),
    row("Ctrl+L", "Edit the location"),
    row("Ctrl+T", "New tab"),
    row("Ctrl+W", "Close the active tab"),
    row("Ctrl+Tab / Ctrl+Shift+Tab", "Next / previous tab"),
    row("Ctrl+Page Up / Ctrl+Page Down", "Previous / next tab"),
    row(
        "Ctrl+Shift+Page Up / Page Down",
        "Move the active tab left / right",
    ),
    row(
        "Ctrl+Shift+1–9 / 0",
        "Select a tab (hold Ctrl+Shift for numbers)",
    ),
    row("Ctrl+Alt+T", "Open a terminal"),
    row("F5", "Refresh"),
    row("Ctrl+H / Ctrl+.", "Show or hide hidden files"),
    row("Ctrl+1 / 2 / 3", "Switch to Columns, Icons, or List"),
    row(
        "Ctrl++ / Ctrl+− / Ctrl+0",
        "Increase / decrease / reset text size",
    ),
    row("Ctrl+B", "Show or hide the sidebar"),
    row("Ctrl+Shift+B", "Switch focus between sidebar and browser"),
    row("Ctrl+\\", "Keep arrows in the file list on or off"),
    row("Ctrl+,", "Open Settings"),
    row("Ctrl+Shift+M", "Turn on 10xer mode"),
    row("Escape", "Close preview or cancel the current interaction"),
    row("F1", "Show or hide this reference"),
];

fn tenxer_tools(mode: BrowserMode, chooser: bool) -> Vec<Shortcut> {
    let mut shortcuts = vec![
        row("/ / ?", "Find the next / previous name in this listing"),
        row("n / N", "Repeat the last find / in reverse"),
        within("%{keys} in a prompt", "↑ / ↓", "Move through the listing"),
        within("%{keys} in a prompt", "Enter / Esc", "Apply / cancel it"),
        within("%{keys} after a find", "Esc", "Dismiss the find highlights"),
        row("f", "Filter this listing"),
        within("%{keys} after a filter", "Esc", "Clear the filter"),
        row("s", "Search this folder and its subfolders"),
        within(
            "%{keys} after a search",
            // Icons h and ← move between result icons instead.
            if mode == BrowserMode::Icons {
                "Esc"
            } else {
                "Esc / h / ←"
            },
            "Dismiss the hits",
        ),
        row("Tab", "Focus the window header"),
        within(
            "%{keys} in the header",
            "Enter / Space",
            "Activate the focused control",
        ),
        within("%{keys} in the header", "h / j", "Return to the files"),
        row("Ctrl+N", "Show or hide the sidebar"),
        row(
            "Ctrl+Shift+B",
            "Focus the visible sidebar, or return to the files",
        ),
        within(
            "%{keys} in the sidebar",
            "j / k / ↑ / ↓",
            "Move between places and device controls",
        ),
        within(
            "%{keys} in the sidebar",
            "l / Enter / Space",
            "Activate the focused place or device control",
        ),
        within(
            "%{keys} in the sidebar",
            "h / ← / Backspace",
            "Return to the files",
        ),
    ];
    if !chooser {
        shortcuts.extend_from_slice(&[
            row("t n / t x", "New / close tab"),
            row("t t", "Previous tab"),
            row("t 1–9 / t 0", "Select tab 1–9 / 10"),
            row("Ctrl+T", "New tab"),
            row("Ctrl+W", "Close the active tab"),
            row("Ctrl+Tab / Ctrl+Shift+Tab", "Next / previous tab"),
            row("Ctrl+Page Up / Ctrl+Page Down", "Previous / next tab"),
            row(
                "Ctrl+Shift+Page Up / Page Down",
                "Move the active tab left / right",
            ),
            row(
                "Ctrl+Shift+1–9 / 0",
                "Select a tab (hold Ctrl+Shift for numbers)",
            ),
            row("Ctrl+K", "Open global search"),
            row("Alt+Enter", "Open containing folder (global search)"),
        ]);
    }
    shortcuts.extend_from_slice(&[
        row("Ctrl+L", "Edit the location"),
        row("F5", "Refresh"),
        row(". / Ctrl+H / Ctrl+.", "Show or hide hidden files"),
        row(
            ", a / , m / , s / , e",
            "Sort by name / modified / size / type; Shift reverses",
        ),
        row("Ctrl+1 / 2 / 3", "Switch to Columns, Icons, or List"),
        row(
            "Ctrl++ / Ctrl+− / Ctrl+0",
            "Increase / decrease / reset text size",
        ),
    ]);
    if !chooser {
        shortcuts.push(row("Ctrl+,", "Open Settings"));
    }
    shortcuts.extend_from_slice(&[
        row(
            "Escape",
            "Close this reference or cancel the current interaction",
        ),
        row("F1 / ~", "Show or hide this reference"),
    ]);
    shortcuts
}

const MEDIA: &[Shortcut] = &[
    row("Ctrl+Alt+Space", "Play / pause"),
    row("Ctrl+Alt+← / →", "Seek −5 / +5 seconds"),
    row("Ctrl+Alt+↑ / ↓", "Volume up / down"),
    row("Ctrl+Alt+M", "Mute / unmute"),
    row("Ctrl+Alt+< / >", "Previous / next file of the same type"),
    within(
        "%{keys} on a video",
        "Enter",
        "Open it in the default app where the preview stopped",
    ),
];

fn default_hint(hint: ContextHint, type_to_search: bool) -> &'static str {
    match hint {
        ContextHint::None => "",
        ContextHint::Preview | ContextHint::ChooserPreview => "Space",
        ContextHint::CopyPath | ContextHint::CopyPaths | ContextHint::Pin if type_to_search => "",
        ContextHint::CopyPath | ContextHint::CopyPaths => "Y",
        ContextHint::Pin => "P",
        ContextHint::Rename => "F2 / Ctrl+R",
        ContextHint::Cut => "Ctrl+X",
        ContextHint::Copy => "Ctrl+C",
        ContextHint::Duplicate => "Ctrl+D",
        ContextHint::Paste => "Ctrl+V",
        ContextHint::MoveTo
        | ContextHint::CopyTo
        | ContextHint::Restore
        | ContextHint::RemoveFromRecent => "",
        ContextHint::Terminal => "Ctrl+Alt+T",
        ContextHint::Trash => "Del",
        ContextHint::PermanentDelete => "Shift+Del",
        ContextHint::Open => "↵",
        ContextHint::OpenMultiple => "Enter",
        ContextHint::OpenInNewTab => "Ctrl+Enter",
        ContextHint::OpenInNewWindow => "Shift+Enter",
        ContextHint::Properties | ContextHint::ContainingFolder => "Alt+Enter",
        ContextHint::NewFolder => "Ctrl+Shift+N",
        ContextHint::SelectAll => "Ctrl+A",
        ContextHint::Refresh => "F5",
        ContextHint::HiddenFiles => "Ctrl+H",
    }
}

fn tenxer_hint(hint: ContextHint) -> &'static str {
    match hint {
        ContextHint::Preview => "i",
        ContextHint::Cut => "x",
        ContextHint::Copy => "y",
        ContextHint::Paste => "p",
        ContextHint::Trash => "d",
        // GTK capitalizes menu accelerators; spell out Shift to distinguish D from d.
        ContextHint::PermanentDelete => "Shift+D",
        ContextHint::MoveTo => "Shift+M",
        ContextHint::CopyTo => "Shift+C",
        ContextHint::Restore => "Shift+R",
        ContextHint::None
        | ContextHint::ChooserPreview
        | ContextHint::CopyPath
        | ContextHint::CopyPaths
        | ContextHint::Duplicate
        | ContextHint::Pin
        | ContextHint::Terminal => "",
        ContextHint::Rename => "r",
        other => default_hint(other, false),
    }
}
