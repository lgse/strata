// SPDX-License-Identifier: MIT

use strata_components::controls::settings_search::{IndexPage, IndexRow, SearchIndex};
use strata_core::i18n::tr;
use strata_desktop::{
    build_info::updater_enabled,
    keys::{
        Keys::One,
        bindings::{REFERENCE, SIDEBAR_FOCUS, TENXER_MODE},
        label_in,
    },
};

pub(super) const KEYBOARD_SHORTCUTS: &str = "keyboard-shortcuts";

struct TargetDef {
    id: &'static str,
    page: &'static str,
    section: Option<&'static str>,
    title: &'static str,
    aliases: &'static str,
}

macro_rules! row {
    ($id:expr, $page:expr, $section:expr, $title:expr, $aliases:expr $(,)?) => {
        TargetDef {
            id: $id,
            page: $page,
            section: if $section.is_empty() {
                None
            } else {
                Some($section)
            },
            title: $title,
            aliases: $aliases,
        }
    };
}

const TARGETS: &[TargetDef] = &[
    row!(
        "language",
        "general",
        "language-units",
        "Language",
        "locale translation auto detect restart English Français Deutsch Español 日本語 Português 한국어 Tiếng Việt Italiano Magyar Hungarian Русский 简体中文",
    ),
    row!(
        "units",
        "general",
        "language-units",
        "Units",
        "metric imperial kilometres miles metres feet distance elevation route gpx fit kml",
    ),
    row!(
        "peeking",
        "general",
        "browsing",
        "Folder peeking",
        "browsing preview folders pane"
    ),
    row!(
        "previews",
        "general",
        "browsing",
        "Single-click file previews",
        "browsing quick preview selecting supported files",
    ),
    row!(
        "preview-autoplay",
        "general",
        "browsing",
        "Autoplay media previews",
        "browsing video audio gif playback paused sound",
    ),
    row!(
        "preview-video-start",
        "general",
        "browsing",
        "Video preview start",
        "browsing video playback position seek skip intro offset percent",
    ),
    row!(
        "render-documents",
        "general",
        "browsing",
        "Render documents by default",
        "browsing preview markdown md html rendered source view",
    ),
    row!(
        "arrow-scope",
        "general",
        "browsing",
        "Keep arrows in file list",
        "browsing keyboard navigation focus sidebar toolbar",
    ),
    row!(
        "columns-sizes",
        "general",
        "browsing",
        "Show sizes in Columns view",
        "columns miller size sizes folders count counts items metadata filenames",
    ),
    row!(
        "columns-mirror",
        "general",
        "browsing",
        "Mirror columns selection",
        "browsing columns keyboard selection next pane folders",
    ),
    row!(
        "tenxer",
        "general",
        "browsing",
        "10xer mode",
        "browsing keyboard modal prompts footer commands experimental",
    ),
    row!(
        "hints",
        "general",
        "browsing",
        "Show %{reference_keys} Shortcuts button",
        "browsing keyboard keybindings hints shortcuts reference help footer",
    ),
    row!(
        "folder-views",
        "general",
        "browsing",
        "Remember sort and icon size per folder",
        "browsing per folder sort order thumbnail size view state",
    ),
    row!(
        "default-sort",
        "general",
        "browsing",
        "Default sort",
        "browsing sort order name size modified type ascending descending",
    ),
    row!(
        "forget-folder-views",
        "general",
        "browsing",
        "Forget all folder settings",
        "browsing reset clear per folder sort thumbnail size",
    ),
    row!(
        "type-search",
        "general",
        "search",
        "Type to search",
        "filter keyboard typing pane"
    ),
    row!(
        "subfolders",
        "general",
        "search",
        "Include subfolders",
        "filter search recursive nested directories",
    ),
    row!(
        "search-results",
        "general",
        "search",
        "Open search results directly",
        "launch files quick preview",
    ),
    row!(
        "search-time-limit",
        "general",
        "search",
        "Search time limit",
        "search filter indexing walk timeout seconds minutes slow network mount partial results",
    ),
    row!(
        "search-entry-limit",
        "general",
        "search",
        "Search entry limit",
        "search filter indexing walk maximum files folders entries memory partial results",
    ),
    row!(
        "search-exclusions",
        "general",
        "search",
        "Global search exclusions",
        "search filter exclusions exclude folders directories ignore venv prune",
    ),
    row!(
        "sidebar-places",
        "general",
        "sidebar",
        "Items shown in sidebar",
        "sidebar places hide show home trash network recent shares desktop documents downloads music pictures videos folder",
    ),
    row!(
        "opening",
        "general",
        "opening",
        "Opening items",
        "single double click activation columns icons list files folders",
    ),
    row!(
        "transfers",
        "general",
        "transfers",
        "Drag & drop to another device",
        "file transfers copy move ask drive share cross volume",
    ),
    row!(
        "open-after-drop",
        "general",
        "transfers",
        "Open folder after dropping files",
        "file transfers drag drop destination navigate reveal copy move",
    ),
    row!(
        "date-format",
        "general",
        "date-time",
        "Modified date format",
        "date time format relative iso 8601 long absolute modified timestamp clock appearance",
    ),
    row!(
        "watch-network-folders",
        "general",
        "performance",
        "Watch network folders",
        "performance network share nfs smb sftp remote changes check refresh reload monitor",
    ),
    row!(
        "thumbnail-workers",
        "general",
        "performance",
        "Thumbnail workers",
        "performance parallel concurrency cpu memory sandbox decoder pool",
    ),
    row!(
        "video",
        "general",
        "performance",
        "Hardware-accelerated video previews",
        "performance gpu thumbnails decoding backend automatic va api vulkan software",
    ),
    row!(
        "window-minimize",
        "general",
        "window-buttons",
        "Show minimize button",
        "window buttons header top bar titlebar controls hide",
    ),
    row!(
        "window-maximize",
        "general",
        "window-buttons",
        "Show maximize button",
        "window buttons header top bar titlebar controls restore hide",
    ),
    row!(
        "window-close",
        "general",
        "window-buttons",
        "Show close button",
        "window buttons header top bar titlebar controls hide",
    ),
    row!(
        "default-directory",
        "general",
        "startup",
        "Default directory",
        "startup launch home folder reset",
    ),
    row!(
        "restore-tabs",
        "general",
        "startup",
        "Restore open tabs",
        "startup launch session reopen tabs",
    ),
    row!(
        "system-light-dark-mode",
        "theme",
        "theme-source",
        "Follow system light/dark mode",
        "light dark mode day night appearance color scheme",
    ),
    row!(
        "omarchy",
        "theme",
        "theme-source",
        "Follow Omarchy",
        "appearance system theme quattro",
    ),
    row!(
        "omarchy-variant",
        "theme",
        "theme-source",
        "Omarchy variant",
        "appearance system theme quattro palette original normal darker dark high contrast brightness",
    ),
    row!(
        "current-theme",
        "theme",
        "theme-source",
        "Current theme",
        "appearance palette colors selected theme",
    ),
    row!(
        "themes",
        "theme",
        "theme-library",
        "Theme library",
        "appearance colors custom themes add edit light dark",
    ),
    row!(
        "renderer",
        "theme",
        "rendering",
        "Interface renderer",
        "appearance graphics cairo gtk gpu text artifacts performance restart",
    ),
    row!(
        "interface-font",
        "theme",
        "text-group",
        "Interface font",
        "appearance font typeface family typography",
    ),
    row!(
        "text",
        "theme",
        "text-group",
        "Text size",
        "appearance font typography pixels zoom scaling"
    ),
    row!(
        "roundness",
        "theme",
        "shape",
        "Corner roundness",
        "appearance corners radius rounded square shape borders",
    ),
    row!(
        "file-icon-colors",
        "theme",
        "file-icons",
        "File icon colors",
        "appearance file type icons brand palette accent theme monochrome catppuccin",
    ),
    row!(
        "show-thumbnails",
        "theme",
        "file-icons",
        "Show thumbnails",
        "appearance images pictures photos videos documents thumbnails icons previews disable",
    ),
    row!(
        "stl-thumbnails",
        "theme",
        "file-icons",
        "STL model thumbnails",
        "appearance 3d stl models printing mesh rendered thumbnails icons",
    ),
    row!(
        "glow",
        "theme",
        "effects",
        "Element glow",
        "appearance effects dialogs menus shadows accent",
    ),
    row!(
        "transparency",
        "theme",
        "transparency",
        "Transparency effects",
        "appearance transparency opacity translucent alpha glass blur window see through",
    ),
    row!(
        "background-transparency",
        "theme",
        "transparency",
        "Background transparency",
        "appearance transparency opacity background pane preview translucent alpha",
    ),
    row!(
        "raised-transparency",
        "theme",
        "transparency",
        "Raised surface transparency",
        "appearance transparency opacity raised sidebar header tabs footer chrome translucent",
    ),
    row!(
        "frosted-glass",
        "theme",
        "transparency",
        "Frosted glass",
        "appearance frosted glass blur backdrop sheen translucent menus dialogs popovers compositor",
    ),
    row!(
        "unfocused-transparency",
        "theme",
        "transparency",
        "Unfocused window multiplier",
        "appearance transparency unfocused inactive focus window multiplier opacity",
    ),
    row!(
        "motion",
        "theme",
        "motion-group",
        "Reduce motion",
        "appearance disable animations"
    ),
    row!(
        "custom-actions",
        "actions",
        "actions-group",
        "Custom actions",
        "scripts context menu commands automation python bash plugin extension",
    ),
    row!(
        "new-action",
        "actions",
        "actions-group",
        "New custom action",
        "create script context menu command",
    ),
    row!(
        "import-action",
        "actions",
        "actions-group",
        "Import custom action",
        "share copy actions folder",
    ),
    row!(
        KEYBOARD_SHORTCUTS,
        "keyboard",
        "keyboard-group",
        "Keyboard shortcuts",
        "keymap keybindings hotkeys accelerators rebind remap keys 10xer default reset",
    ),
    row!(
        "integrations",
        "integrations",
        "integrations-group",
        "Integrations",
        "add-ons addons plugins extensions providers catalog install kde connect localsend nextcloud dropbox cloud devices phone",
    ),
    row!(
        "check",
        "updates",
        "updates-group",
        "Check for updates",
        "version status install download upgrade"
    ),
    row!(
        "auto-updates",
        "updates",
        "updates-group",
        "Check for updates automatically",
        "startup github release",
    ),
    row!(
        "channel",
        "updates",
        "updates-group",
        "Release channel",
        "stable preview nightly builds"
    ),
    row!(
        "diagnostics",
        "diagnostics",
        "diagnostics-group",
        "Run diagnostics",
        "troubleshoot troubleshooting problems health check dependencies missing packages report bug sandbox bubblewrap ffmpeg gstreamer gvfs sftp smb udisks portal",
    ),
    row!(
        "version",
        "about",
        "about-build",
        "Version information",
        "strata build commit toolkit rune copy author",
    ),
    row!(
        "website",
        "about",
        "about-links",
        "Website",
        "links stratafiles home"
    ),
    row!(
        "source",
        "about",
        "about-links",
        "Source code",
        "links github repository"
    ),
    row!(
        "issue",
        "about",
        "about-links",
        "Report an issue",
        "links bug feedback"
    ),
    row!(
        "license",
        "about",
        "about-links",
        "License",
        "links mit copyright"
    ),
];

pub(super) fn translated(message: &str) -> String {
    let locale = rust_i18n::locale();
    rust_i18n::t!(
        message,
        locale = &*locale,
        reference_keys = label_in(&locale, &One(REFERENCE.alternate(0).shown())),
        sidebar_focus_keys = label_in(&locale, &One(SIDEBAR_FOCUS.shown())),
        tenxer_mode_keys = label_in(&locale, &One(TENXER_MODE.shown()))
    )
    .into_owned()
}

pub(super) struct Availability {
    pub(super) system: bool,
    pub(super) omarchy: bool,
}

pub(super) fn build_index(revision: u64, avail: Availability) -> SearchIndex {
    let settings_words = format!("settings {}", tr("Settings"));
    let mut pages = vec![
        IndexPage::new("general", tr("General")).keywords(&settings_words),
        IndexPage::new("theme", tr("Appearance")).keywords(&settings_words),
        IndexPage::new("actions", tr("Actions")).keywords(&settings_words),
        IndexPage::new("keyboard", tr("Keyboard")).keywords(&settings_words),
        IndexPage::new("integrations", tr("Integrations")).keywords(&settings_words),
    ];
    if updater_enabled() {
        pages.push(IndexPage::new("updates", tr("Updates")).keywords(&settings_words));
    }
    pages.push(IndexPage::new("diagnostics", tr("Diagnostics")).keywords(&settings_words));
    pages.push(IndexPage::new("about", tr("About")).keywords(&settings_words));

    let mut rows = Vec::new();
    for target in TARGETS {
        if !target_available(target, &avail) {
            continue;
        }
        let shown = translated(target.title);
        let english = rust_i18n::t!(
            target.title,
            locale = "en",
            reference_keys = label_in("en", &One(REFERENCE.alternate(0).shown())),
            sidebar_focus_keys = label_in("en", &One(SIDEBAR_FOCUS.shown())),
            tenxer_mode_keys = label_in("en", &One(TENXER_MODE.shown()))
        )
        .into_owned();
        let mut entry =
            IndexRow::new(target.id, target.page, shown.clone()).keywords(target.aliases);
        if shown != english {
            entry = entry.source_title(english);
        }
        if let Some(sec) = target.section {
            entry = entry.section(sec);
        }
        rows.push(entry);
    }
    SearchIndex {
        revision,
        pages,
        rows,
    }
}

fn target_available(target: &TargetDef, avail: &Availability) -> bool {
    match target.id {
        "system-light-dark-mode" => avail.system,
        "omarchy" | "omarchy-variant" => avail.omarchy,
        _ if target.page == "updates" => updater_enabled(),
        _ => true,
    }
}
