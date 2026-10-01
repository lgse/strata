// SPDX-License-Identifier: MIT

use super::browser_modes::BrowserMode;

pub(crate) const EXPERIMENTAL_LABEL: &str = "(experimental feature, under active development)";

/// What the open reference describes: the view's navigation and, in a portal
/// file chooser, only the keys that request allows.
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
    /// Saves one file under the name in the chooser's name field.
    SaveFile,
    SaveFiles,
}

pub(crate) struct ReferenceSection {
    pub title: &'static str,
    pub rows: Vec<(&'static str, &'static str)>,
}

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
    Terminal,
    Trash,
    PermanentDelete,
    Open,
    OpenMultiple,
    Properties,
    ContainingFolder,
    NewFolder,
    SelectAll,
    Refresh,
    HiddenFiles,
}

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
            title: "10xer mode",
            rows: tenxer_mode_rows(chooser.is_some()),
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
    sections
}

fn navigation_title(mode: BrowserMode) -> &'static str {
    match mode {
        BrowserMode::Columns => "Columns navigation",
        BrowserMode::Icons => "Icons navigation",
        BrowserMode::List => "List navigation",
    }
}

fn default_navigation(mode: BrowserMode) -> Vec<(&'static str, &'static str)> {
    let mut shortcuts = match mode {
        BrowserMode::Columns => vec![
            ("↑ / ↓", "Move between items"),
            ("← / →", "Parent pane / enter folder"),
            ("← at first pane", "Focus the visible sidebar"),
            (
                "Backspace",
                "Close the current pane or go to the parent folder",
            ),
        ],
        BrowserMode::Icons => vec![
            ("↑ ↓ ← →", "Move spatially between tiles"),
            ("← at left edge", "Focus the visible sidebar"),
            ("Backspace", "Go to the parent folder"),
        ],
        BrowserMode::List => vec![
            ("↑ / ↓", "Move between file rows"),
            ("←", "Focus the visible sidebar"),
            ("Backspace", "Go to the parent folder"),
        ],
    };
    shortcuts.extend_from_slice(&[
        ("h / j / k / l", "Same as ← ↓ ↑ → (type-to-search off)"),
        ("↑ at top", "Focus the navigation header"),
        ("← / → in header", "Move between header controls"),
        ("↓ in header", "Return to the files"),
        ("→ in sidebar", "Return to the browser"),
        ("↑ at sidebar top", "Focus the top navigation bar"),
        ("← / → in top bar", "Move between top-bar controls"),
        (
            "↓ in top bar",
            "Return to the sidebar, or files when hidden",
        ),
        ("Alt+← / Alt+→", "Back / forward in history"),
        ("Alt+↑", "Go to the parent folder"),
        ("Alt+Home", "Go to Home"),
        ("Home / End", "First / last item"),
        ("Ctrl+↑ / Ctrl+↓", "First / last item"),
        ("PgUp / PgDn", "Move one page"),
        ("Tab / Shift+Tab", "Next / previous interface control"),
    ]);
    shortcuts
}

fn tenxer_navigation(mode: BrowserMode, chooser: bool) -> Vec<(&'static str, &'static str)> {
    let mut shortcuts = match mode {
        BrowserMode::Columns | BrowserMode::List => vec![
            ("j / k / ↑ / ↓", "Next / previous item"),
            ("h / ← / Backspace / Alt+↑", "Go to the parent folder"),
            ("l / →", "Open the focused directory"),
            (
                "l / → on a file",
                "Enter its preview; keys move into the drawer",
            ),
        ],
        BrowserMode::Icons => vec![
            ("h / j / k / l / ↑ ↓ ← →", "Move spatially between tiles"),
            ("Backspace / Alt+↑", "Go to the parent folder"),
        ],
    };
    // The chooser refuses i, and its own section describes Enter and o.
    if !chooser {
        shortcuts.extend_from_slice(&[
            (
                "i",
                if mode == BrowserMode::Columns {
                    "Open the next column for the focused directory"
                } else {
                    "Toggle folder peek for the focused directory"
                },
            ),
            ("Enter / o", "Open the focused item"),
        ]);
    }
    shortcuts.extend_from_slice(&[
        ("H / L / Alt+← / Alt+→", "Back / forward in history"),
        ("Alt+Home", "Go to Home"),
        ("Home", "First item"),
        ("G / End", "Last item"),
        ("Ctrl+↑ / Ctrl+↓", "First / last item"),
        ("Ctrl+U / Ctrl+D", "Move half a page"),
        ("Ctrl+B / Ctrl+F / PgUp / PgDn", "Move one page"),
    ]);
    shortcuts
}

fn tenxer_places(chooser: bool) -> Vec<(&'static str, &'static str)> {
    let mut shortcuts = vec![
        ("g g", "First item"),
        ("g f", "Follow search result"),
        ("g h / g c", "Home / ~/.config"),
        (
            "g d / g k / g m / g p / g v",
            "Downloads / Documents / Music / Pictures / Videos",
        ),
    ];
    if chooser {
        shortcuts.extend_from_slice(&[
            ("g r", "Recent"),
            ("g 1–9", "Visible PINNED rows that are local folders"),
        ]);
    } else {
        shortcuts.extend_from_slice(&[
            ("g t / g n / g r", "Trash / Network / Recent"),
            ("g 1–9", "Visible PINNED rows in sidebar order"),
            ("g + / g -", "Pin / unpin a folder"),
        ]);
    }
    shortcuts.extend_from_slice(&[
        ("g Space", "Go to a typed path or URI"),
        ("Tab / Shift+Tab in go ›", "Cycle matching folders"),
        ("z", "Jump to a visited folder"),
        ("Z", "Jump to a recent folder"),
        ("↑ / ↓ in jump › / recent ›", "Choose a visited folder"),
        ("Esc after g", "Cancel a pending chord"),
    ]);
    shortcuts
}

fn tenxer_preview(mode: BrowserMode, chooser: bool) -> Vec<(&'static str, &'static str)> {
    let mut shortcuts = Vec::new();
    if !chooser {
        shortcuts.push(("i on a file", "Toggle the preview without taking focus"));
    }
    shortcuts.push(("J / K", "Scroll the open preview without taking focus"));
    // Icons have no key that moves into the preview.
    if mode != BrowserMode::Icons {
        shortcuts.extend_from_slice(TENXER_PREVIEW_OWNED);
    }
    shortcuts
}

const TENXER_PREVIEW_OWNED: &[(&str, &str)] = &[
    ("j / k / ↑ / ↓ in the preview", "Scroll the document"),
    ("Ctrl+U / Ctrl+D in the preview", "Scroll half a page"),
    (
        "Ctrl+B / Ctrl+F / PgUp / PgDn in the preview",
        "Scroll one page",
    ),
    (
        "g g / Home / G / End in the preview",
        "Top / bottom of the document",
    ),
    (
        "j / k / l / Enter in an archive",
        "Move / open an archive folder",
    ),
    (
        "g g / Home / G / End in an archive",
        "First / last archive member",
    ),
    (
        "h in an archive",
        "Archive parent; at the root, back to the listing",
    ),
    ("Space / ← → / ↑ ↓ / m in media", "Play, seek, volume, mute"),
    (
        "h / ← in a document, h in media",
        "Return to the listing; the preview stays open",
    ),
    ("Shift+Tab in any preview", "Return to the listing"),
    (
        "Esc / i in the preview",
        "Close the preview and return to the listing",
    ),
];

const DEFAULT_FILES: &[(&str, &str)] = &[
    ("Enter", "Open the current item"),
    ("Space", "Preview a file, or open a folder"),
    ("Ctrl+C / Ctrl+X", "Copy / cut selected items"),
    ("Ctrl+V", "Paste into the indicated directory"),
    ("Ctrl+D", "Duplicate selected items"),
    ("Delete", "Move selected items to Trash, when supported"),
    ("Shift+Delete", "Permanently delete selected items"),
    ("Ctrl+Z", "Undo the last file operation"),
    ("Ctrl+Shift+Z / Ctrl+Y", "Redo the last file operation"),
    ("F2 / Ctrl+R", "Rename"),
    ("Ctrl+Shift+N", "Create a folder"),
    ("Ctrl+A", "Select all items in the focused pane"),
    ("Shift+↑ / ↓", "Extend selection"),
    ("Shift+PgUp / PgDn", "Extend selection by one page"),
    ("Ctrl+Space", "Toggle the focused item in the selection"),
    ("Alt+Enter", "Show item properties"),
    ("Menu / Shift+F10", "Open the context menu"),
    ("y / p", "Copy path / pin a folder (type-to-search off)"),
];

const TENXER_FILES: &[(&str, &str)] = &[
    ("y / x", "Yank / cut the selection, or the focused item"),
    ("Y / X", "Clear copy and cut marks"),
    ("p", "Paste; Keep Both is focused on conflicts"),
    ("P / Ctrl+V", "Paste; Replace is focused on conflicts"),
    ("d / Delete", "Move to Trash after confirming; d d confirms"),
    ("D / Shift+Delete", "Delete permanently after confirming"),
    ("a", "Create a file; end with / for a folder"),
    ("c c / c n", "Copy path / name"),
    ("Ctrl+C / Ctrl+X", "Copy / cut selected items"),
    ("Ctrl+Z", "Undo the last file operation"),
    ("Ctrl+Shift+Z / Ctrl+Y", "Redo the last file operation"),
    ("r / F2", "Rename the focused item in the footer"),
    ("O", "Open With for the selection, or the focused item"),
    (
        "; 1–9 / ; 0",
        "Run one of the first ten matching custom actions",
    ),
    ("; t", "Open a terminal in the focused folder"),
    ("; c", "Compress the selection, or the focused item"),
    ("; e / ; E", "Extract the archive here / to a typed folder"),
    ("M / C", "Move / copy to a typed folder"),
    ("R", "Restore from Trash"),
    ("Ctrl+Shift+N", "Create a folder"),
    ("Space", "Toggle the focused item and move down"),
    ("v / V", "Visual select / visual unset"),
    ("Ctrl+A", "Select all items in the focused pane"),
    ("Ctrl+R", "Invert the selection"),
    ("Alt+Enter", "Show item properties"),
    ("Menu / Shift+F10", "Open the context menu"),
];

fn chooser_requests(chooser: ChooserScope) -> Vec<(&'static str, &'static str)> {
    let mut shortcuts = match chooser.request {
        ChooserRequest::Files => vec![
            (
                "Enter / o on a file",
                if chooser.multiple {
                    "Choose the filled items, or the file"
                } else {
                    "Choose the file"
                },
            ),
            ("Enter / o on a folder", "Open the folder"),
        ],
        ChooserRequest::Folders => vec![
            ("Enter / o on a folder", "Open the folder"),
            ("Ctrl+Enter", "Choose a folder, as Accept does"),
        ],
        ChooserRequest::SaveFile => vec![
            ("Enter", "Save the name in the current folder"),
            ("o on a file", "Save over that file after confirming"),
            ("o on a folder", "Open the folder"),
            ("r / F2", "Edit the name; Esc returns to the files"),
        ],
        ChooserRequest::SaveFiles => vec![
            ("Enter", "Save the files in the current folder"),
            ("o on a folder", "Open the folder"),
        ],
    };
    shortcuts.push(("Esc", "Dismiss one interaction, then cancel the request"));
    shortcuts
}

fn chooser_files(chooser: ChooserScope) -> Vec<(&'static str, &'static str)> {
    let mut shortcuts = vec![
        ("d / Delete", "Move to Trash after confirming; d d confirms"),
        ("D / Shift+Delete", "Delete permanently after confirming"),
        ("a", "Create a file; end with / for a folder"),
        ("c c / c n", "Copy path / name"),
    ];
    if chooser.request != ChooserRequest::SaveFile {
        shortcuts.push(("r / F2", "Rename the focused item in the footer"));
    }
    shortcuts.push(("Ctrl+Shift+N", "Create a folder"));
    if chooser.multiple {
        shortcuts.extend_from_slice(&[
            ("Space", "Toggle the focused item and move down"),
            ("v / V", "Visual select / visual unset"),
            ("Ctrl+A", "Select all items in the focused pane"),
            ("Ctrl+R", "Invert the selection"),
        ]);
    }
    shortcuts.extend_from_slice(&[
        ("Alt+Enter", "Show item properties"),
        ("Menu / Shift+F10", "Open the context menu"),
    ]);
    shortcuts
}

fn tenxer_mode_rows(chooser: bool) -> Vec<(&'static str, &'static str)> {
    let mut shortcuts = Vec::new();
    if !chooser {
        shortcuts.push(("Q", "Close the current window"));
    }
    shortcuts.extend_from_slice(&[
        ("Ctrl+Shift+M", "Toggle 10xer mode"),
        ("F1 / ~", "Show or hide this reference"),
    ]);
    shortcuts
}

const DEFAULT_TOOLS: &[(&str, &str)] = &[
    ("Ctrl+F", "Filter the current pane"),
    ("Ctrl+K", "Open global search"),
    ("Ctrl+Shift+K", "Jump to a recent folder"),
    ("Alt+Enter", "Open containing folder (global search)"),
    ("Ctrl+L", "Edit the location"),
    ("Ctrl+T", "Open a terminal"),
    ("F5", "Refresh"),
    ("Ctrl+H / Ctrl+.", "Show or hide hidden files"),
    ("Ctrl+1 / 2 / 3", "Switch to Columns, Icons, or List"),
    (
        "Ctrl++ / Ctrl+− / Ctrl+0",
        "Increase / decrease / reset text size",
    ),
    ("Ctrl+B", "Show or hide the sidebar"),
    ("Ctrl+Shift+B", "Switch focus between sidebar and browser"),
    ("Ctrl+\\", "Keep arrows in the file list on or off"),
    ("Ctrl+,", "Open Settings"),
    ("Ctrl+Shift+M", "Turn on 10xer mode"),
    ("Escape", "Close preview or cancel the current interaction"),
    ("F1", "Show or hide this reference"),
];

fn tenxer_tools(mode: BrowserMode, chooser: bool) -> Vec<(&'static str, &'static str)> {
    let mut shortcuts = vec![
        ("/ / ?", "Find the next / previous name in this listing"),
        ("n / N", "Repeat the last find / in reverse"),
        ("↑ / ↓ in a prompt", "Move through the listing"),
        ("Enter / Esc in a prompt", "Apply / cancel it"),
        ("Esc after a find", "Dismiss the find highlights"),
        ("f", "Filter this listing"),
        ("Esc after a filter", "Clear the filter"),
        ("s", "Search this folder and its subfolders"),
        (
            // Icons h and ← move between result icons instead.
            if mode == BrowserMode::Icons {
                "Esc after a search"
            } else {
                "Esc / h / ← after a search"
            },
            "Dismiss the hits",
        ),
        ("Tab", "Focus the window header"),
        (
            "Enter / Space in the header",
            "Activate the focused control",
        ),
        ("h / j in the header", "Return to the files"),
        ("Ctrl+N", "Show or hide the sidebar"),
        (
            "Ctrl+Shift+B",
            "Focus the visible sidebar, or return to the files",
        ),
        (
            "j / k / ↑ / ↓ in the sidebar",
            "Move between places and device controls",
        ),
        (
            "l / Enter / Space in the sidebar",
            "Activate the focused place or device control",
        ),
        ("h / ← / Backspace in the sidebar", "Return to the files"),
    ];
    if !chooser {
        shortcuts.extend_from_slice(&[
            ("Ctrl+K", "Open global search"),
            ("Alt+Enter", "Open containing folder (global search)"),
        ]);
    }
    shortcuts.extend_from_slice(&[
        ("Ctrl+L", "Edit the location"),
        ("F5", "Refresh"),
        (". / Ctrl+H / Ctrl+.", "Show or hide hidden files"),
        (
            ", a / , m / , s / , e",
            "Sort by name / modified / size / type; Shift reverses",
        ),
        ("Ctrl+1 / 2 / 3", "Switch to Columns, Icons, or List"),
        (
            "Ctrl++ / Ctrl+− / Ctrl+0",
            "Increase / decrease / reset text size",
        ),
    ]);
    if !chooser {
        shortcuts.push(("Ctrl+,", "Open Settings"));
    }
    shortcuts.extend_from_slice(&[
        (
            "Escape",
            "Close this reference or cancel the current interaction",
        ),
        ("F1 / ~", "Show or hide this reference"),
    ]);
    shortcuts
}

const MEDIA: &[(&str, &str)] = &[
    ("Ctrl+Alt+Space", "Play / pause"),
    ("Ctrl+Alt+← / →", "Seek −5 / +5 seconds"),
    ("Ctrl+Alt+↑ / ↓", "Volume up / down"),
    ("Ctrl+Alt+M", "Mute / unmute"),
];

fn default_hint(hint: ContextHint, type_to_search: bool) -> &'static str {
    match hint {
        ContextHint::None => "",
        ContextHint::Preview | ContextHint::ChooserPreview => "Space",
        // Type-to-search claims plain letters.
        ContextHint::CopyPath | ContextHint::CopyPaths | ContextHint::Pin if type_to_search => "",
        ContextHint::CopyPath | ContextHint::CopyPaths => "Y",
        ContextHint::Pin => "P",
        ContextHint::Rename => "F2 / Ctrl+R",
        ContextHint::Cut => "Ctrl+X",
        ContextHint::Copy => "Ctrl+C",
        ContextHint::Duplicate => "Ctrl+D",
        ContextHint::Paste => "Ctrl+V",
        ContextHint::MoveTo | ContextHint::CopyTo | ContextHint::Restore => "",
        ContextHint::Terminal => "Ctrl+T",
        ContextHint::Trash => "Del",
        ContextHint::PermanentDelete => "Shift+Del",
        ContextHint::Open | ContextHint::OpenMultiple => "Enter",
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
