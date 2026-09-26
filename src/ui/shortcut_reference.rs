// SPDX-License-Identifier: MIT

use super::browser_modes::BrowserMode;

pub(crate) const EXPERIMENTAL_LABEL: &str = "(experimental feature, under active development)";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Binding {
    pub category: &'static str,
    pub action: &'static str,
    pub note: &'static str,
    pub keys: &'static str,
}

pub(crate) struct ReferenceSection {
    pub title: &'static str,
    pub rows: Vec<(&'static str, &'static str)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContextHint {
    None,
    Preview,
    CopyPath,
    CopyPaths,
    Rename,
    Cut,
    Copy,
    Duplicate,
    Paste,
    Pin,
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

const DEFAULT_SETTINGS: &[Binding] = &[
    Binding {
        category: "Navigation",
        action: "Move through items",
        note: "← / → in Icons view",
        keys: "↑ / ↓",
    },
    Binding {
        category: "Navigation",
        action: "Jump to top / bottom",
        note: "",
        keys: "Ctrl + ↑ / Ctrl + ↓",
    },
    Binding {
        category: "Navigation",
        action: "Open item",
        note: "",
        keys: "Enter",
    },
    Binding {
        category: "Navigation",
        action: "Go to parent folder",
        note: "",
        keys: "Alt + ↑",
    },
    Binding {
        category: "Navigation",
        action: "Back / forward",
        note: "",
        keys: "Alt + ← / Alt + →",
    },
    Binding {
        category: "Navigation",
        action: "Move between column panes",
        note: "Columns view",
        keys: "← / →",
    },
    Binding {
        category: "Navigation",
        action: "Focus pane header",
        note: "when at top",
        keys: "↑",
    },
    Binding {
        category: "Navigation",
        action: "Focus sidebar",
        note: "when at left edge",
        keys: "←",
    },
    Binding {
        category: "Selection",
        action: "Select all",
        note: "",
        keys: "Ctrl + A",
    },
    Binding {
        category: "Selection",
        action: "Extend selection",
        note: "",
        keys: "Shift + ↑ / Shift + ↓",
    },
    Binding {
        category: "Selection",
        action: "Toggle item in selection",
        note: "",
        keys: "Ctrl + Space",
    },
    Binding {
        category: "Selection",
        action: "Clear selection",
        note: "",
        keys: "Esc",
    },
    Binding {
        category: "Files",
        action: "Quick preview",
        note: "",
        keys: "Space",
    },
    Binding {
        category: "Files",
        action: "Cut / copy / paste",
        note: "",
        keys: "Ctrl + X / C / V",
    },
    Binding {
        category: "Files",
        action: "Duplicate",
        note: "",
        keys: "Ctrl + D",
    },
    Binding {
        category: "Files",
        action: "Rename",
        note: "",
        keys: "F2 / Ctrl + R",
    },
    Binding {
        category: "Files",
        action: "Create new folder",
        note: "",
        keys: "Ctrl + Shift + N",
    },
    Binding {
        category: "Files",
        action: "Move to Trash",
        note: "",
        keys: "Delete",
    },
    Binding {
        category: "Files",
        action: "Delete permanently",
        note: "",
        keys: "Shift + Delete",
    },
    Binding {
        category: "Files",
        action: "Undo file operation",
        note: "",
        keys: "Ctrl + Z",
    },
    Binding {
        category: "Files",
        action: "Item properties",
        note: "",
        keys: "Alt + Enter",
    },
    Binding {
        category: "View",
        action: "Toggle hidden files",
        note: "",
        keys: "Ctrl + H / Ctrl + .",
    },
    Binding {
        category: "View",
        action: "Switch view",
        note: "Columns / Icons / List",
        keys: "Ctrl + 1 / 2 / 3",
    },
    Binding {
        category: "View",
        action: "Increase text size",
        note: "",
        keys: "Ctrl + +",
    },
    Binding {
        category: "View",
        action: "Decrease text size",
        note: "",
        keys: "Ctrl + −",
    },
    Binding {
        category: "View",
        action: "Reset text size",
        note: "",
        keys: "Ctrl + 0",
    },
    Binding {
        category: "View",
        action: "Toggle sidebar",
        note: "",
        keys: "Ctrl + B",
    },
    Binding {
        category: "Application",
        action: "Edit location",
        note: "",
        keys: "Ctrl + L",
    },
    Binding {
        category: "Application",
        action: "Filter items",
        note: "",
        keys: "Ctrl + F",
    },
    Binding {
        category: "Application",
        action: "Search",
        note: "",
        keys: "Ctrl + K",
    },
    Binding {
        category: "Application",
        action: "Open terminal",
        note: "",
        keys: "Ctrl + T",
    },
    Binding {
        category: "Application",
        action: "Refresh",
        note: "",
        keys: "F5",
    },
    Binding {
        category: "Application",
        action: "Open settings",
        note: "",
        keys: "Ctrl + ,",
    },
    Binding {
        category: "Application",
        action: "Shortcut reference",
        note: "",
        keys: "F1",
    },
    Binding {
        category: "Application",
        action: "Toggle arrow-key scope",
        note: "",
        keys: "Ctrl + \\",
    },
];

const TENXER_SETTINGS: &[Binding] = &[
    Binding {
        category: "Navigation",
        action: "Move through items",
        note: "List and Columns",
        keys: "j / k / ↑ / ↓",
    },
    Binding {
        category: "Navigation",
        action: "Move between icons",
        note: "Icons",
        keys: "↑ ↓ ← →",
    },
    Binding {
        category: "Navigation",
        action: "Go to parent folder",
        note: "h / ← in List and Columns",
        keys: "h / ← / Backspace / Alt + ↑",
    },
    Binding {
        category: "Navigation",
        action: "Open directory",
        note: "List and Columns",
        keys: "l / →",
    },
    Binding {
        category: "Navigation",
        action: "Open next column",
        note: "Columns; focus stays in place",
        keys: "i",
    },
    Binding {
        category: "Navigation",
        action: "Open item",
        note: "",
        keys: "Enter / o",
    },
    Binding {
        category: "Navigation",
        action: "Toggle folder peek",
        note: "Icons",
        keys: "i",
    },
    Binding {
        category: "Navigation",
        action: "Back / forward",
        note: "",
        keys: "H / L / Alt + ← / Alt + →",
    },
    Binding {
        category: "Navigation",
        action: "First / last item",
        note: "",
        keys: "Home / G / End / Ctrl + ↑ / Ctrl + ↓",
    },
    Binding {
        category: "Navigation",
        action: "Go to Home",
        note: "",
        keys: "Alt + Home",
    },
    Binding {
        category: "Navigation",
        action: "Move half a page",
        note: "",
        keys: "Ctrl + U / Ctrl + D",
    },
    Binding {
        category: "Navigation",
        action: "Move one page",
        note: "",
        keys: "Ctrl + B / Ctrl + F / PgUp / PgDn",
    },
    Binding {
        category: "Selection",
        action: "Select all",
        note: "",
        keys: "Ctrl + A",
    },
    Binding {
        category: "Selection",
        action: "Extend selection",
        note: "",
        keys: "Shift + ↑ / Shift + ↓",
    },
    Binding {
        category: "Selection",
        action: "Open the context menu",
        note: "",
        keys: "Menu / Shift + F10",
    },
    Binding {
        category: "Files",
        action: "Cut",
        note: "",
        keys: "Ctrl + X",
    },
    Binding {
        category: "Files",
        action: "Copy",
        note: "",
        keys: "Ctrl + C",
    },
    Binding {
        category: "Files",
        action: "Paste",
        note: "",
        keys: "Ctrl + V",
    },
    Binding {
        category: "Files",
        action: "Rename",
        note: "",
        keys: "F2",
    },
    Binding {
        category: "Files",
        action: "Create new folder",
        note: "",
        keys: "Ctrl + Shift + N",
    },
    Binding {
        category: "Files",
        action: "Move to Trash",
        note: "",
        keys: "Delete",
    },
    Binding {
        category: "Files",
        action: "Delete permanently",
        note: "",
        keys: "Shift + Delete",
    },
    Binding {
        category: "Files",
        action: "Undo file operation",
        note: "",
        keys: "Ctrl + Z",
    },
    Binding {
        category: "Files",
        action: "Redo file operation",
        note: "",
        keys: "Ctrl + Shift + Z",
    },
    Binding {
        category: "Files",
        action: "Item properties",
        note: "",
        keys: "Alt + Enter",
    },
    Binding {
        category: "View",
        action: "Toggle hidden files",
        note: "",
        keys: "Ctrl + H / Ctrl + .",
    },
    Binding {
        category: "View",
        action: "Switch view",
        note: "Columns / Icons / List",
        keys: "Ctrl + 1 / 2 / 3",
    },
    Binding {
        category: "View",
        action: "Increase text size",
        note: "",
        keys: "Ctrl + +",
    },
    Binding {
        category: "View",
        action: "Decrease text size",
        note: "",
        keys: "Ctrl + −",
    },
    Binding {
        category: "View",
        action: "Reset text size",
        note: "",
        keys: "Ctrl + 0",
    },
    Binding {
        category: "Preview media",
        action: "Play / pause",
        note: "",
        keys: "Ctrl + Alt + Space",
    },
    Binding {
        category: "Preview media",
        action: "Seek",
        note: "",
        keys: "Ctrl + Alt + ← / →",
    },
    Binding {
        category: "Preview media",
        action: "Volume",
        note: "",
        keys: "Ctrl + Alt + ↑ / ↓",
    },
    Binding {
        category: "Preview media",
        action: "Mute / unmute",
        note: "",
        keys: "Ctrl + Alt + M",
    },
    Binding {
        category: "Application",
        action: "Edit location",
        note: "",
        keys: "Ctrl + L",
    },
    Binding {
        category: "Application",
        action: "Search",
        note: "",
        keys: "Ctrl + K",
    },
    Binding {
        category: "Application",
        action: "Refresh",
        note: "",
        keys: "F5",
    },
    Binding {
        category: "Application",
        action: "Open settings",
        note: "",
        keys: "Ctrl + ,",
    },
    Binding {
        category: "Application",
        action: "Shortcut reference",
        note: "",
        keys: "F1 / ~",
    },
    Binding {
        category: "Application",
        action: "Toggle 10xer mode",
        note: "",
        keys: "Ctrl + Shift + M",
    },
    Binding {
        category: "Application",
        action: "Leave 10xer mode",
        note: "",
        keys: "q",
    },
    Binding {
        category: "Application",
        action: "Close window",
        note: "",
        keys: "Q",
    },
];

pub(crate) fn settings_bindings(tenxer: bool) -> &'static [Binding] {
    if tenxer {
        TENXER_SETTINGS
    } else {
        DEFAULT_SETTINGS
    }
}

pub(crate) fn active_settings_bindings() -> &'static [Binding] {
    settings_bindings(super::tenxer_mode::chrome_suppressed())
}

pub(crate) fn reference_sections(mode: BrowserMode) -> Vec<ReferenceSection> {
    if super::tenxer_mode::chrome_suppressed() {
        tenxer_sections(mode)
    } else {
        default_sections(mode)
    }
}

pub(crate) fn context_hint_for(hint: ContextHint, tenxer: bool) -> &'static str {
    if tenxer {
        tenxer_hint(hint)
    } else {
        default_hint(hint)
    }
}

fn default_sections(mode: BrowserMode) -> Vec<ReferenceSection> {
    vec![
        ReferenceSection {
            title: match mode {
                BrowserMode::Columns => "Columns navigation",
                BrowserMode::Icons => "Icons navigation",
                BrowserMode::List => "List navigation",
            },
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

fn tenxer_sections(mode: BrowserMode) -> Vec<ReferenceSection> {
    vec![
        ReferenceSection {
            title: match mode {
                BrowserMode::Columns => "Columns navigation",
                BrowserMode::Icons => "Icons navigation",
                BrowserMode::List => "List navigation",
            },
            rows: tenxer_navigation(mode),
        },
        ReferenceSection {
            title: "Files and selection",
            rows: TENXER_FILES.to_vec(),
        },
        ReferenceSection {
            title: "10xer mode",
            rows: TENXER_MODE.to_vec(),
        },
        ReferenceSection {
            title: "Search and tools",
            rows: TENXER_TOOLS.to_vec(),
        },
        ReferenceSection {
            title: "Preview media",
            rows: MEDIA.to_vec(),
        },
    ]
}

pub(crate) fn default_navigation(mode: BrowserMode) -> Vec<(&'static str, &'static str)> {
    let mut shortcuts = match mode {
        BrowserMode::Columns => vec![
            ("↑ / ↓", "Move between items"),
            ("← / →", "Parent pane / enter folder"),
            ("Space", "Open folder column"),
            ("← at first pane", "Focus the visible sidebar"),
            (
                "Backspace",
                "Close the current pane or go to the parent folder",
            ),
            (
                "h / j / k / l",
                "Move between items; l opens the item (type-to-search off)",
            ),
        ],
        BrowserMode::Icons => vec![
            ("↑ ↓ ← →", "Move spatially between tiles"),
            ("← at left edge", "Focus the visible sidebar"),
            ("Backspace", "Go to the parent folder"),
            ("h / l", "Parent folder / open item (type-to-search off)"),
            ("j / k", "Next / previous item (type-to-search off)"),
        ],
        BrowserMode::List => vec![
            ("↑ / ↓", "Move between file rows"),
            ("←", "Focus the visible sidebar"),
            ("Backspace", "Go to the parent folder"),
            ("h / l", "Parent folder / open item (type-to-search off)"),
            ("j / k", "Next / previous item (type-to-search off)"),
        ],
    };
    shortcuts.extend_from_slice(&[
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

fn tenxer_navigation(mode: BrowserMode) -> Vec<(&'static str, &'static str)> {
    match mode {
        BrowserMode::Columns | BrowserMode::List => tenxer_listing_navigation(mode),
        BrowserMode::Icons => vec![
            ("h / j / k / l / ↑ ↓ ← →", "Move spatially between tiles"),
            ("Enter / o", "Open the focused item"),
            ("i", "Toggle folder peek for the focused directory"),
            ("Backspace / Alt+↑", "Go to the parent folder"),
            ("H / L / Alt+← / Alt+→", "Back / forward in history"),
            ("Alt+Home", "Go to Home"),
            ("Home", "First item"),
            ("G / End", "Last item"),
            ("Ctrl+↑ / Ctrl+↓", "First / last item"),
            ("Ctrl+U / Ctrl+D", "Move half a page"),
            ("Ctrl+B / Ctrl+F / PgUp / PgDn", "Move one page"),
        ],
    }
}

/// List and Columns movement. `l` / Right open a directory and leave a file alone.
fn tenxer_listing_navigation(mode: BrowserMode) -> Vec<(&'static str, &'static str)> {
    let mut shortcuts = vec![
        ("j / k / ↑ / ↓", "Next / previous item"),
        ("h / ← / Backspace / Alt+↑", "Go to the parent folder"),
        ("l / →", "Open the focused directory"),
    ];
    if mode == BrowserMode::Columns {
        shortcuts.push(("i", "Open the next column for the focused directory"));
    }
    shortcuts.extend_from_slice(&[
        ("Enter / o", "Open the focused item"),
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

const DEFAULT_FILES: &[(&str, &str)] = &[
    ("Enter", "Open the current item"),
    ("Space", "Toggle file preview"),
    ("Ctrl+C / Ctrl+X", "Copy / cut selected items"),
    ("Ctrl+V", "Paste into the indicated directory"),
    ("Ctrl+D", "Duplicate selected items"),
    ("Delete", "Move selected items to Trash, when supported"),
    ("Shift+Delete", "Permanently delete selected items"),
    (
        "Ctrl+Z / Ctrl+Shift+Z",
        "Undo / redo the last file operation",
    ),
    ("F2 / Ctrl+R", "Rename"),
    ("Ctrl+Shift+N", "Create a folder"),
    ("Ctrl+A", "Select all items in the focused pane"),
    ("Shift+↑ / ↓", "Extend selection"),
    ("Alt+Enter", "Show item properties"),
    ("Menu / Shift+F10", "Open the context menu"),
    ("y / p", "Copy path / pin a folder (type-to-search off)"),
];

const TENXER_FILES: &[(&str, &str)] = &[
    ("Ctrl+C / Ctrl+X", "Copy / cut selected items"),
    ("Ctrl+V", "Paste into the indicated directory"),
    ("Delete", "Move selected items to Trash, when supported"),
    ("Shift+Delete", "Permanently delete selected items"),
    (
        "Ctrl+Z / Ctrl+Shift+Z",
        "Undo / redo the last file operation",
    ),
    ("F2", "Rename"),
    ("Ctrl+Shift+N", "Create a folder"),
    ("Ctrl+A", "Select all items in the focused pane"),
    ("Shift+↑ / ↓", "Extend selection"),
    ("Alt+Enter", "Show item properties"),
    ("Menu / Shift+F10", "Open the context menu"),
];

const TENXER_MODE: &[(&str, &str)] = &[
    ("q", "Leave 10xer mode"),
    ("Q", "Close the current window"),
    ("Ctrl+Shift+M", "Toggle 10xer mode"),
    ("F1 / ~", "Show or hide this reference"),
];

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
    ("Ctrl+B", "Show or hide the sidebar"),
    ("Ctrl+Shift+B", "Switch focus between sidebar and browser"),
    ("Ctrl+,", "Open Settings"),
    ("Escape", "Close preview or cancel the current interaction"),
    ("F1", "Show or hide this reference"),
];

const TENXER_TOOLS: &[(&str, &str)] = &[
    ("Ctrl+K", "Open global search"),
    ("Alt+Enter", "Open containing folder (global search)"),
    ("Ctrl+L", "Edit the location"),
    ("F5", "Refresh"),
    ("Ctrl+H / Ctrl+.", "Show or hide hidden files"),
    ("Ctrl+1 / 2 / 3", "Switch to Columns, Icons, or List"),
    ("Ctrl+,", "Open Settings"),
    (
        "Escape",
        "Close this reference or cancel the current interaction",
    ),
    ("F1 / ~", "Show or hide this reference"),
];

const MEDIA: &[(&str, &str)] = &[
    ("Ctrl+Alt+Space", "Play / pause"),
    ("Ctrl+Alt+← / →", "Seek −5 / +5 seconds"),
    ("Ctrl+Alt+↑ / ↓", "Volume up / down"),
    ("Ctrl+Alt+M", "Mute / unmute"),
];

fn default_hint(hint: ContextHint) -> &'static str {
    match hint {
        ContextHint::None => "",
        ContextHint::Preview => "Space",
        ContextHint::CopyPath | ContextHint::CopyPaths => "Y",
        ContextHint::Rename => "F2 / Ctrl+R",
        ContextHint::Cut => "Ctrl+X",
        ContextHint::Copy => "Ctrl+C",
        ContextHint::Duplicate => "Ctrl+D",
        ContextHint::Paste => "Ctrl+V",
        ContextHint::Pin => "P",
        ContextHint::Terminal => "Ctrl+T",
        ContextHint::Trash => "Del",
        ContextHint::PermanentDelete => "Shift+Del",
        ContextHint::Open => "↵",
        ContextHint::OpenMultiple => "Enter",
        ContextHint::Properties | ContextHint::ContainingFolder => "Alt+Enter",
        ContextHint::NewFolder => "Ctrl+Shift+N",
        ContextHint::SelectAll => "Ctrl+A",
        ContextHint::Refresh => "F5",
        ContextHint::HiddenFiles => "Ctrl+H",
    }
}

fn tenxer_hint(hint: ContextHint) -> &'static str {
    match hint {
        ContextHint::None
        | ContextHint::Preview
        | ContextHint::CopyPath
        | ContextHint::CopyPaths
        | ContextHint::Duplicate
        | ContextHint::Pin
        | ContextHint::Terminal => "",
        ContextHint::Rename => "F2",
        other => default_hint(other),
    }
}
