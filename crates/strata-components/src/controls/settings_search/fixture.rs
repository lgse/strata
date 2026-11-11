//! Strata's settings search index, as test and gallery data.
//!
//! The rows, their pages, titles and aliases are ported from Strata's
//! `src/ui/settings/search/targets.rs` at upstream `fb7eb0c4e` (MIT, Copyright (c) LGSE), in
//! its order, with `%{reference_keys}` filled in as F1. The page titles are Strata's
//! navigation labels (`src/ui/settings/navigation.rs`). The section keys and the Keyboard
//! members (Strata builds those from its key registry at run time) are made up for tests and
//! the gallery.

use super::index::{IndexPage, IndexRow, SearchIndex};

/// The Keyboard page's group row (Strata's `KEYBOARD_SHORTCUTS`).
pub const KEYBOARD_SHORTCUTS: &str = "keyboard-shortcuts";

/// Strata's navigation, in order: key and title.
pub const PAGES: &[(&str, &str)] = &[
    ("general", "General"),
    ("theme", "Appearance"),
    ("actions", "Actions"),
    ("keyboard", "Keyboard"),
    ("integrations", "Integrations"),
    ("updates", "Updates"),
    ("diagnostics", "Diagnostics"),
    ("about", "About"),
];

/// Keyboard rows: key, names, then the key labels in both maps (Strata's `search_targets`).
pub const SHORTCUTS: &[(&str, &str, &str)] = &[
    ("keymap.copy", "Copy", "Ctrl+C"),
    ("keymap.undo", "Undo", "Ctrl+Z"),
    ("keymap.redo", "Redo", "Ctrl+Shift+Z Ctrl+Y"),
    ("keymap.refresh", "Refresh", "F5 Ctrl+R"),
    ("keymap.rename", "Rename", "F2"),
    ("keymap.toggle_hidden", "Show hidden files", "Ctrl+H Ctrl+."),
    ("keymap.sort_chord", "Sort chord", "s"),
    (
        "keymap.custom_action_1",
        "Run custom action 1 Resize images",
        "Ctrl+Alt+1",
    ),
];

fn row(key: &str, page: &str, title: &str, aliases: &str, section: Option<&str>) -> IndexRow {
    let r = IndexRow::new(key, page, title).keywords(aliases);
    match section {
        Some(s) => r.section(s),
        None => r,
    }
}

/// Strata's index in English, every page shown.
pub fn index() -> SearchIndex {
    let pages = PAGES
        .iter()
        .map(|(key, title)| IndexPage::new(*key, *title).keywords("settings"))
        .collect();
    let mut rows = vec![
        row(
            "units",
            "general",
            "Units",
            "metric imperial kilometres miles metres feet distance elevation route gpx fit kml",
            Some("language-units"),
        ),
        row(
            "language",
            "general",
            "Language",
            "locale translation auto detect restart English Français Deutsch Español 日本語 Português 한국어 Tiếng Việt Italiano Magyar Hungarian Русский 简体中文",
            Some("language-units"),
        ),
        row(
            "custom-actions",
            "actions",
            "Custom actions",
            "scripts context menu commands automation python bash plugin extension",
            None,
        ),
        row(
            "new-action",
            "actions",
            "New custom action",
            "create script context menu command",
            None,
        ),
        row(
            "import-action",
            "actions",
            "Import custom action",
            "share copy actions folder",
            None,
        ),
        row(
            "thumbnail-workers",
            "general",
            "Thumbnail workers",
            "performance parallel concurrency cpu memory sandbox decoder pool",
            Some("performance"),
        ),
        row(
            "default-directory",
            "general",
            "Default directory",
            "startup launch home folder reset",
            Some("startup"),
        ),
        row(
            "restore-tabs",
            "general",
            "Restore open tabs",
            "startup launch session reopen tabs",
            Some("startup"),
        ),
        row(
            "peeking",
            "general",
            "Folder peeking",
            "browsing preview folders pane",
            Some("browsing"),
        ),
        row(
            "previews",
            "general",
            "Single-click file previews",
            "browsing quick preview selecting supported files",
            Some("browsing"),
        ),
        row(
            "preview-autoplay",
            "general",
            "Autoplay media previews",
            "browsing video audio gif playback paused sound",
            Some("browsing"),
        ),
        row(
            "preview-video-start",
            "general",
            "Video preview start",
            "browsing video playback position seek skip intro offset percent",
            Some("browsing"),
        ),
        row(
            "render-documents",
            "general",
            "Render documents by default",
            "browsing preview markdown md html rendered source view",
            Some("browsing"),
        ),
        row(
            "arrow-scope",
            "general",
            "Keep arrows in file list",
            "browsing keyboard navigation focus sidebar toolbar",
            Some("browsing"),
        ),
        row(
            "columns-sizes",
            "general",
            "Show sizes in Columns view",
            "columns miller size sizes folders count counts items metadata filenames",
            Some("browsing"),
        ),
        row(
            "columns-mirror",
            "general",
            "Mirror columns selection",
            "browsing columns keyboard selection next pane folders",
            Some("browsing"),
        ),
        row(
            "tenxer",
            "general",
            "10xer mode",
            "browsing keyboard modal prompts footer commands experimental",
            Some("browsing"),
        ),
        row(
            "hints",
            "general",
            "Show F1 Shortcuts button",
            "browsing keyboard keybindings hints shortcuts reference help footer",
            Some("browsing"),
        ),
        row(
            "folder-views",
            "general",
            "Remember sort and icon size per folder",
            "browsing per folder sort order thumbnail size view state",
            Some("browsing"),
        ),
        row(
            "default-sort",
            "general",
            "Default sort",
            "browsing sort order name size modified type ascending descending",
            Some("browsing"),
        ),
        row(
            "forget-folder-views",
            "general",
            "Forget all folder settings",
            "browsing reset clear per folder sort thumbnail size",
            Some("browsing"),
        ),
        row(
            "type-search",
            "general",
            "Type to search",
            "filter keyboard typing pane",
            Some("search"),
        ),
        row(
            "subfolders",
            "general",
            "Include subfolders",
            "filter search recursive nested directories",
            Some("search"),
        ),
        row(
            "search-results",
            "general",
            "Open search results directly",
            "launch files quick preview",
            Some("search"),
        ),
        row(
            "search-time-limit",
            "general",
            "Search time limit",
            "search filter indexing walk timeout seconds minutes slow network mount partial results",
            Some("search"),
        ),
        row(
            "search-entry-limit",
            "general",
            "Search entry limit",
            "search filter indexing walk maximum files folders entries memory partial results",
            Some("search"),
        ),
        row(
            "search-exclusions",
            "general",
            "Global search exclusions",
            "search filter exclusions exclude folders directories ignore venv prune",
            Some("search"),
        ),
        row(
            "sidebar-places",
            "general",
            "Items shown in sidebar",
            "sidebar places hide show home trash network recent shares desktop documents downloads music pictures videos folder",
            Some("browsing"),
        ),
        row(
            "opening",
            "general",
            "Opening items",
            "single double click activation columns icons list files folders",
            Some("browsing"),
        ),
        row(
            "transfers",
            "general",
            "Drag & drop to another device",
            "file transfers copy move ask drive share cross volume",
            None,
        ),
        row(
            "open-after-drop",
            "general",
            "Open folder after dropping files",
            "file transfers drag drop destination navigate reveal copy move",
            None,
        ),
        row(
            "watch-network-folders",
            "general",
            "Watch network folders",
            "performance network share nfs smb sftp remote changes check refresh reload monitor",
            Some("performance"),
        ),
        row(
            "video",
            "general",
            "Hardware-accelerated video previews",
            "performance gpu thumbnails decoding backend automatic va api vulkan software",
            Some("performance"),
        ),
        row(
            "window-minimize",
            "general",
            "Show minimize button",
            "window buttons header top bar titlebar controls hide",
            Some("window"),
        ),
        row(
            "window-maximize",
            "general",
            "Show maximize button",
            "window buttons header top bar titlebar controls restore hide",
            Some("window"),
        ),
        row(
            "window-close",
            "general",
            "Show close button",
            "window buttons header top bar titlebar controls hide",
            Some("window"),
        ),
        row(
            "desktop",
            "general",
            "Desktop integration",
            "configure portal file chooser open save dialog default file manager encrypted udiskie",
            None,
        ),
        row(
            "udiskie-unlock",
            "general",
            "Unlock encrypted volumes",
            "encrypted luks password udiskie automount usb volume desktop integration omarchy",
            None,
        ),
        row(
            "system-light-dark-mode",
            "theme",
            "Follow system light/dark mode",
            "light dark mode day night appearance color scheme",
            None,
        ),
        row(
            "omarchy",
            "theme",
            "Follow Omarchy",
            "appearance system theme quattro",
            None,
        ),
        row(
            "omarchy-variant",
            "theme",
            "Omarchy variant",
            "appearance system theme quattro palette original normal darker dark high contrast brightness",
            None,
        ),
        row(
            "current-theme",
            "theme",
            "Current theme",
            "appearance palette colors selected theme",
            None,
        ),
        row(
            "themes",
            "theme",
            "Theme library",
            "appearance colors custom themes add edit light dark",
            None,
        ),
        row(
            "interface-font",
            "theme",
            "Interface font",
            "appearance font typeface family typography",
            Some("text"),
        ),
        row(
            "text",
            "theme",
            "Text size",
            "appearance font typography pixels zoom scaling",
            Some("text"),
        ),
        row(
            "date-format",
            "general",
            "Modified date format",
            "date time format relative iso 8601 long absolute modified timestamp clock appearance",
            Some("browsing"),
        ),
        row(
            "roundness",
            "theme",
            "Corner roundness",
            "appearance corners radius rounded square shape borders",
            None,
        ),
        row(
            "show-thumbnails",
            "theme",
            "Show thumbnails",
            "appearance images pictures photos videos documents thumbnails icons previews disable",
            None,
        ),
        row(
            "stl-thumbnails",
            "theme",
            "STL model thumbnails",
            "appearance 3d stl models printing mesh rendered thumbnails icons",
            None,
        ),
        row(
            "file-icon-colors",
            "theme",
            "File icon colors",
            "appearance file type icons brand palette accent theme monochrome catppuccin",
            None,
        ),
        row(
            "glow",
            "theme",
            "Element glow",
            "appearance effects dialogs menus shadows accent",
            None,
        ),
        row(
            "motion",
            "theme",
            "Reduce motion",
            "appearance disable animations",
            None,
        ),
        row(
            "renderer",
            "theme",
            "Interface renderer",
            "appearance graphics cairo gtk gpu text artifacts performance restart",
            None,
        ),
        row(
            "check",
            "updates",
            "Check for updates",
            "version status install download upgrade",
            None,
        ),
        row(
            "auto-updates",
            "updates",
            "Check for updates automatically",
            "startup github release",
            None,
        ),
        row(
            "channel",
            "updates",
            "Release channel",
            "stable preview nightly builds",
            None,
        ),
        row(
            "release",
            "updates",
            "Install a specific release",
            "rollback roll back downgrade older previous version choose pick",
            None,
        ),
        row(
            "notes",
            "updates",
            "Release notes",
            "what new changes changelog",
            None,
        ),
        row(
            "integrations",
            "integrations",
            "Integrations",
            "add-ons addons plugins extensions providers catalog install kde connect localsend nextcloud dropbox cloud devices phone",
            None,
        ),
        row(
            "diagnostics",
            "diagnostics",
            "Run diagnostics",
            "troubleshoot troubleshooting problems health check dependencies missing packages report bug sandbox bubblewrap ffmpeg gstreamer gvfs sftp smb udisks portal",
            None,
        ),
        row(
            "version",
            "about",
            "Version information",
            "strata build commit toolkit gtk copy author",
            None,
        ),
        row(
            "website",
            "about",
            "Website",
            "links stratafiles home",
            None,
        ),
        row(
            "source",
            "about",
            "Source code",
            "links github repository",
            None,
        ),
        row(
            "issue",
            "about",
            "Report an issue",
            "links bug feedback",
            None,
        ),
        row("license", "about", "License", "links mit copyright", None),
        row(
            "keyboard-shortcuts",
            "keyboard",
            "Keyboard shortcuts",
            "keymap keybindings hotkeys accelerators rebind remap keys 10xer default reset",
            None,
        ),
    ];
    rows.extend(SHORTCUTS.iter().map(|(key, names, keys)| {
        IndexRow::member(*key, "keyboard", KEYBOARD_SHORTCUTS, *names).keywords(keys)
    }));
    SearchIndex {
        revision: 1,
        pages,
        rows,
    }
}

/// [`index`] without the pages in `hidden` (a build without the updater has no Updates).
pub fn index_without(hidden: &[&str]) -> SearchIndex {
    let mut index = index();
    index.pages.retain(|p| !hidden.contains(&p.key.as_str()));
    index.rows.retain(|r| !hidden.contains(&r.page.as_str()));
    index.revision += 1;
    index
}
