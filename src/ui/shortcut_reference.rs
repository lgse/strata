// SPDX-License-Identifier: MIT

//! Shared presentation for Settings and F1. Default F1 navigation remains view-specific.

pub(super) type Shortcut = (&'static str, &'static str, &'static str, &'static str);

pub(super) const DEFAULT_CATEGORIES: &[&str] = &[
    "Navigation",
    "Selection",
    "Files",
    "View",
    "Application",
    "Preview media",
];
pub(super) const MINIMAL_CATEGORIES: &[&str] = &[
    "Minimal navigation",
    "Minimal selection",
    "Minimal files",
    "Minimal places",
    "Minimal prompts",
    super::minimal_mode::LABELED_TITLE,
    "Preview media",
];

pub(super) const DEFAULT: &[Shortcut] = &[
    (
        "Navigation",
        "Move through items",
        "← / → in Icons view",
        "↑ / ↓",
    ),
    (
        "Navigation",
        "Jump to top / bottom",
        "",
        "Ctrl + ↑ / Ctrl + ↓",
    ),
    ("Files", "Open item", "", "Enter"),
    ("Navigation", "Go to parent folder", "", "Alt + ↑"),
    ("Navigation", "Back / forward", "", "Alt + ← / Alt + →"),
    (
        "Navigation",
        "Move between column panes",
        "Columns view",
        "← / →",
    ),
    ("Navigation", "Focus pane header", "when at top", "↑"),
    ("Navigation", "Focus sidebar", "when at left edge", "←"),
    ("Selection", "Select all", "focused pane", "Ctrl + A"),
    ("Selection", "Extend selection", "", "Shift + ↑ / Shift + ↓"),
    ("Selection", "Toggle item in selection", "", "Ctrl + Space"),
    (
        "Selection",
        "Clear selection",
        "after closing the current interaction",
        "Esc",
    ),
    ("Files", "Quick preview", "", "Space"),
    (
        "Files",
        "Cut / copy / paste",
        "paste into the indicated directory",
        "Ctrl + X / C / V",
    ),
    ("Files", "Duplicate", "", "Ctrl + D"),
    ("Files", "Rename", "", "F2 / Ctrl + R"),
    ("Files", "Create new folder", "", "Ctrl + Shift + N"),
    ("Files", "Move to Trash", "when supported", "Delete"),
    ("Files", "Delete permanently", "", "Shift + Delete"),
    ("Files", "Undo file operation", "", "Ctrl + Z"),
    ("Files", "Item properties", "", "Alt + Enter"),
    ("Files", "Open context menu", "", "Menu / Shift + F10"),
    (
        "Files",
        "Copy path / pin a folder",
        "type-to-search off",
        "y / p",
    ),
    ("View", "Toggle hidden files", "", "Ctrl + H / Ctrl + ."),
    (
        "View",
        "Switch view",
        "Columns / Icons / List",
        "Ctrl + 1 / 2 / 3",
    ),
    ("View", "Increase text size", "", "Ctrl + +"),
    ("View", "Decrease text size", "", "Ctrl + −"),
    ("View", "Reset text size", "", "Ctrl + 0"),
    ("View", "Toggle sidebar", "", "Ctrl + B"),
    ("View", "Focus sidebar / browser", "", "Ctrl + Shift + B"),
    ("Application", "Edit location", "", "Ctrl + L"),
    ("Application", "Filter items", "", "Ctrl + F"),
    ("Application", "Search", "", "Ctrl + K"),
    (
        "Application",
        "Jump to a recent folder",
        "",
        "Ctrl + Shift + K",
    ),
    (
        "Application",
        "Open containing folder",
        "global search",
        "Alt + Enter",
    ),
    ("Application", "Open terminal", "", "Ctrl + T"),
    ("Application", "Refresh", "", "F5"),
    ("Application", "Open Settings", "", "Ctrl + ,"),
    ("Application", "Shortcut reference", "", "F1"),
    ("Application", "Toggle arrow-key scope", "", "Ctrl + \\"),
    (
        "Application",
        "Toggle minimal mode",
        "also while editing text",
        "Ctrl + Shift + M",
    ),
];

pub(super) const MINIMAL: &[Shortcut] = &[
    (
        "Minimal navigation",
        "Parent / leave preview",
        "List and Columns; leaving preview keeps it open",
        "h / ←",
    ),
    (
        "Minimal navigation",
        "Open directory / enter preview",
        "List and Columns; repetition keeps the preview open",
        "l / →",
    ),
    (
        "Minimal navigation",
        "Move among icon tiles",
        "Icons; stays in the current folder, never previews",
        "h / j / k / l / ← / → / ↑ / ↓",
    ),
    ("Minimal navigation", "Open the focused item", "", "Enter"),
    (
        "Minimal navigation",
        "Next / previous item",
        "List and Columns listing order; scroll while preview owns keys",
        "j / k / ↑ / ↓",
    ),
    (
        "Minimal navigation",
        "First item",
        "preview start while it owns keys",
        "g g / Home",
    ),
    (
        "Minimal navigation",
        "Last item",
        "preview end while it owns keys",
        "G / End",
    ),
    (
        "Minimal navigation",
        "Half page up / down",
        "scroll while preview owns keys",
        "Ctrl + U / Ctrl + D",
    ),
    (
        "Minimal navigation",
        "Full page up / down",
        "scroll while preview owns keys",
        "Ctrl + B / Ctrl + F",
    ),
    (
        "Minimal navigation",
        "Full page up / down",
        "scroll while preview owns keys",
        "PgUp / PgDn",
    ),
    (
        "Minimal navigation",
        "Back / forward in history",
        "",
        "H / L",
    ),
    (
        "Minimal navigation",
        "Back / forward in history",
        "",
        "Alt + ← / Alt + →",
    ),
    (
        "Minimal navigation",
        "Parent folder / leave preview",
        "",
        "Alt + ↑ / Backspace",
    ),
    (
        "Minimal navigation",
        "Toggle the preview drawer",
        "Icons keeps listing focus; no preview-focus hotkey",
        "i",
    ),
    ("Minimal navigation", "Scroll the open preview", "", "J / K"),
    (
        "Minimal selection",
        "Toggle the focused item and move down",
        "",
        "Space",
    ),
    (
        "Minimal selection",
        "Visual select / visual unset",
        "",
        "v / V",
    ),
    (
        "Minimal selection",
        "Select all",
        "focused pane",
        "Ctrl + A",
    ),
    ("Minimal selection", "Invert the selection", "", "Ctrl + R"),
    (
        "Minimal selection",
        "Cancel the current interaction",
        "chord, filter/find, visual, preview, then selection; in recursive search close preview before hits",
        "Esc",
    ),
    (
        "Minimal files",
        "Yank / cut the selection",
        "or the focused item",
        "y / x",
    ),
    ("Minimal files", "Paste", "Keep Both on conflicts", "p"),
    ("Minimal files", "Paste; Replace on conflicts", "", "P"),
    (
        "Minimal files",
        "Clear clipboard marks and our payload",
        "never clears another application's clipboard",
        "Y / X",
    ),
    (
        "Minimal files",
        "Move to Trash",
        "with confirmation",
        "d / Delete",
    ),
    (
        "Minimal files",
        "Delete permanently",
        "with confirmation",
        "D / Shift + Delete",
    ),
    (
        "Minimal files",
        "Rename the focused item",
        "footer prompt",
        "r / F2",
    ),
    ("Minimal files", "Open / Open With", "", "o / O"),
    ("Minimal files", "Copy path / name", "", "c c / c n"),
    (
        "Minimal files",
        "Run a custom action",
        "first 10 matching the focused item or selection",
        "; 1–9 / 0",
    ),
    ("Minimal files", "Show or hide hidden files", "", "."),
    (
        "Minimal files",
        "Sort by name / modified / size / type",
        "shift reverses",
        ", a / m / s / e",
    ),
    ("Minimal files", "Undo file operation", "", "Ctrl + Z"),
    ("Minimal places", "First item", "", "g g"),
    ("Minimal places", "Home", "", "g h"),
    ("Minimal places", "Downloads", "", "g d"),
    ("Minimal places", "Config", "", "g c"),
    ("Minimal places", "Trash", "", "g t"),
    ("Minimal places", "Network", "", "g n"),
    ("Minimal places", "Recent", "", "g r"),
    ("Minimal places", "Documents", "", "g k"),
    ("Minimal places", "Pictures", "", "g p"),
    ("Minimal places", "Videos", "", "g v"),
    (
        "Minimal places",
        "Pinned places",
        "visible sidebar order",
        "g 1–9",
    ),
    (
        "Minimal places",
        "Go to a typed path or URI",
        "footer prompt",
        "g Space",
    ),
    (
        "Minimal prompts",
        "Find next name in this listing",
        "no filtering; Enter keeps highlights",
        "/",
    ),
    ("Minimal prompts", "Find previous name", "", "?"),
    (
        "Minimal prompts",
        "Repeat the last find",
        "N reverses",
        "n / N",
    ),
    (
        "Minimal prompts",
        "Filter this listing",
        "hides non-matches",
        "f",
    ),
    (
        "Minimal prompts",
        "Recursive name search",
        "Esc keeps hits; Esc again dismisses",
        "s",
    ),
    (
        "Minimal prompts",
        "Jump to a visited folder",
        "candidate list; fuzzy, frecency",
        "z",
    ),
    (
        "Minimal prompts",
        "Jump to a recent folder",
        "candidate list; last visit",
        "Z",
    ),
    (
        "Minimal prompts",
        "Create a file",
        "trailing / makes a folder",
        "a",
    ),
    (
        "Minimal prompts",
        "Submit / cancel the footer prompt",
        "",
        "Enter / Esc",
    ),
    (
        "Minimal prompts",
        "Move listing / pick history candidate",
        "without leaving the prompt",
        "Up / Down",
    ),
    (
        "Minimal prompts",
        "Cycle matching folders",
        "go prompt",
        "Tab / Shift + Tab",
    ),
    (
        super::minimal_mode::LABELED_TITLE,
        "Leave minimal mode",
        "",
        "q",
    ),
    (super::minimal_mode::LABELED_TITLE, "Close window", "", "Q"),
    (
        super::minimal_mode::LABELED_TITLE,
        "Toggle minimal mode",
        "also while editing text",
        "Ctrl + Shift + M",
    ),
    (
        super::minimal_mode::LABELED_TITLE,
        "Shortcut reference",
        "",
        "F1 / ~",
    ),
    (
        super::minimal_mode::LABELED_TITLE,
        "Open Settings",
        "also cancels an armed chord",
        "Ctrl + ,",
    ),
    (
        super::minimal_mode::LABELED_TITLE,
        "Open context menu",
        "",
        "Menu / Shift + F10",
    ),
    (
        super::minimal_mode::LABELED_TITLE,
        "Focus sidebar",
        "",
        "Ctrl + Shift + B",
    ),
    (
        super::minimal_mode::LABELED_TITLE,
        "Copy / cut / paste",
        "GUI",
        "Ctrl + C / X / V",
    ),
    (
        super::minimal_mode::LABELED_TITLE,
        "Show or hide hidden files",
        "",
        "Ctrl + H / Ctrl + .",
    ),
    (
        super::minimal_mode::LABELED_TITLE,
        "Edit location",
        "",
        "Ctrl + L",
    ),
    (
        super::minimal_mode::LABELED_TITLE,
        "Open global search",
        "",
        "Ctrl + K",
    ),
    (super::minimal_mode::LABELED_TITLE, "Refresh", "", "F5"),
    (
        super::minimal_mode::LABELED_TITLE,
        "Switch view",
        "Columns / Icons / List",
        "Ctrl + 1 / 2 / 3",
    ),
    (
        super::minimal_mode::LABELED_TITLE,
        "Create new folder",
        "",
        "Ctrl + Shift + N",
    ),
    (
        super::minimal_mode::LABELED_TITLE,
        "Item properties",
        "",
        "Alt + Enter",
    ),
    (
        super::minimal_mode::LABELED_TITLE,
        "Increase text size",
        "",
        "Ctrl + +",
    ),
    (
        super::minimal_mode::LABELED_TITLE,
        "Decrease text size",
        "",
        "Ctrl + −",
    ),
    (
        super::minimal_mode::LABELED_TITLE,
        "Reset text size",
        "",
        "Ctrl + 0",
    ),
];

pub(super) const MEDIA: &[Shortcut] = &[
    ("Preview media", "Play / pause", "", "Ctrl + Alt + Space"),
    (
        "Preview media",
        "Seek −5 / +5 seconds",
        "",
        "Ctrl + Alt + ← / →",
    ),
    (
        "Preview media",
        "Volume up / down",
        "",
        "Ctrl + Alt + ↑ / ↓",
    ),
    ("Preview media", "Mute / unmute", "", "Ctrl + Alt + M"),
];

pub(super) fn categories(minimal: bool) -> &'static [&'static str] {
    if minimal {
        MINIMAL_CATEGORIES
    } else {
        DEFAULT_CATEGORIES
    }
}

pub(crate) fn shortcuts(minimal: bool) -> impl Iterator<Item = &'static Shortcut> {
    (if minimal { MINIMAL } else { DEFAULT })
        .iter()
        .chain(MEDIA)
}
