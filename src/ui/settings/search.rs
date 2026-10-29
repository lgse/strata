// SPDX-License-Identifier: MIT

use gtk::{glib, prelude::*};
use std::{cell::RefCell, collections::HashSet, rc::Rc};

use crate::ui::keys::{self, bindings::ESCAPE_ANY};

struct Target {
    id: &'static str,
    page: &'static str,
    title: &'static str,
    aliases: &'static str,
}

const TARGETS: &[Target] = &[
    Target {
        id: "units",
        page: "general",
        title: "Units",
        aliases: "metric imperial kilometres miles metres feet distance elevation route gpx fit kml",
    },
    Target {
        id: "language",
        page: "general",
        title: "Language",
        aliases: "locale translation auto detect restart English Français Deutsch Español 日本語 Português 한국어 Tiếng Việt Italiano Magyar Hungarian Русский 简体中文",
    },
    Target {
        id: "custom-actions",
        page: "actions",
        title: "Custom actions",
        aliases: "scripts context menu commands automation python bash plugin extension",
    },
    Target {
        id: "new-action",
        page: "actions",
        title: "New custom action",
        aliases: "create script context menu command",
    },
    Target {
        id: "import-action",
        page: "actions",
        title: "Import custom action",
        aliases: "share copy actions folder",
    },
    Target {
        id: "thumbnail-workers",
        page: "general",
        title: "Thumbnail workers",
        aliases: "performance parallel concurrency cpu memory sandbox decoder pool",
    },
    Target {
        id: "default-directory",
        page: "general",
        title: "Default directory",
        aliases: "startup launch home folder reset",
    },
    Target {
        id: "restore-tabs",
        page: "general",
        title: "Restore open tabs",
        aliases: "startup launch session reopen tabs",
    },
    Target {
        id: "peeking",
        page: "general",
        title: "Folder peeking",
        aliases: "browsing preview folders pane",
    },
    Target {
        id: "previews",
        page: "general",
        title: "Single-click file previews",
        aliases: "browsing quick preview selecting supported files",
    },
    Target {
        id: "preview-autoplay",
        page: "general",
        title: "Autoplay media previews",
        aliases: "browsing video audio gif playback paused sound",
    },
    Target {
        id: "render-documents",
        page: "general",
        title: "Render documents by default",
        aliases: "browsing preview markdown md html rendered source view",
    },
    Target {
        id: "arrow-scope",
        page: "general",
        title: "Keep arrows in file list",
        aliases: "browsing keyboard navigation focus sidebar toolbar",
    },
    Target {
        id: "columns-sizes",
        page: "general",
        title: "Show sizes in Columns view",
        aliases: "columns miller size sizes folders count counts items metadata filenames",
    },
    Target {
        id: "columns-mirror",
        page: "general",
        title: "Mirror columns selection",
        aliases: "browsing columns keyboard selection next pane folders",
    },
    Target {
        id: "tenxer",
        page: "general",
        title: "10xer mode",
        aliases: "browsing keyboard modal prompts footer commands experimental",
    },
    Target {
        id: "hints",
        page: "general",
        title: "Show %{reference_keys} Shortcuts button",
        aliases: "browsing keyboard keybindings hints shortcuts reference help footer",
    },
    Target {
        id: "folder-views",
        page: "general",
        title: "Remember sort and icon size per folder",
        aliases: "browsing per folder sort order thumbnail size view state",
    },
    Target {
        id: "default-sort",
        page: "general",
        title: "Default sort",
        aliases: "browsing sort order name size modified type ascending descending",
    },
    Target {
        id: "forget-folder-views",
        page: "general",
        title: "Forget all folder settings",
        aliases: "browsing reset clear per folder sort thumbnail size",
    },
    Target {
        id: "type-search",
        page: "general",
        title: "Type to search",
        aliases: "filter keyboard typing pane",
    },
    Target {
        id: "subfolders",
        page: "general",
        title: "Include subfolders",
        aliases: "filter search recursive nested directories",
    },
    Target {
        id: "search-results",
        page: "general",
        title: "Open search results directly",
        aliases: "launch files quick preview",
    },
    Target {
        id: "search-time-limit",
        page: "general",
        title: "Search time limit",
        aliases: "search filter indexing walk timeout seconds minutes slow network mount partial results",
    },
    Target {
        id: "search-entry-limit",
        page: "general",
        title: "Search entry limit",
        aliases: "search filter indexing walk maximum files folders entries memory partial results",
    },
    Target {
        id: "search-exclusions",
        page: "general",
        title: "Global search exclusions",
        aliases: "search filter exclusions exclude folders directories ignore venv prune",
    },
    Target {
        id: "sidebar-places",
        page: "general",
        title: "Items shown in sidebar",
        aliases: "sidebar places hide show home trash network recent shares desktop documents downloads music pictures videos folder",
    },
    Target {
        id: "opening",
        page: "general",
        title: "Opening items",
        aliases: "single double click activation columns icons list files folders",
    },
    Target {
        id: "transfers",
        page: "general",
        title: "Drag & drop to another device",
        aliases: "file transfers copy move ask drive share cross volume",
    },
    Target {
        id: "open-after-drop",
        page: "general",
        title: "Open folder after dropping files",
        aliases: "file transfers drag drop destination navigate reveal copy move",
    },
    Target {
        id: "watch-network-folders",
        page: "general",
        title: "Watch network folders",
        aliases: "performance network share nfs smb sftp remote changes check refresh reload monitor",
    },
    Target {
        id: "video",
        page: "general",
        title: "Hardware-accelerated video previews",
        aliases: "performance gpu thumbnails decoding backend automatic va api vulkan software",
    },
    Target {
        id: "window-minimize",
        page: "general",
        title: "Show minimize button",
        aliases: "window buttons header top bar titlebar controls hide",
    },
    Target {
        id: "window-maximize",
        page: "general",
        title: "Show maximize button",
        aliases: "window buttons header top bar titlebar controls restore hide",
    },
    Target {
        id: "window-close",
        page: "general",
        title: "Show close button",
        aliases: "window buttons header top bar titlebar controls hide",
    },
    Target {
        id: "desktop",
        page: "general",
        title: "Desktop integration",
        aliases: "configure portal file chooser open save dialog default file manager encrypted udiskie",
    },
    Target {
        id: "udiskie-unlock",
        page: "general",
        title: "Unlock encrypted volumes",
        aliases: "encrypted luks password udiskie automount usb volume desktop integration omarchy",
    },
    Target {
        id: "omarchy",
        page: "theme",
        title: "Follow Omarchy",
        aliases: "appearance system theme quattro",
    },
    Target {
        id: "omarchy-variant",
        page: "theme",
        title: "Omarchy variant",
        aliases: "appearance system theme quattro palette original normal darker dark high contrast brightness",
    },
    Target {
        id: "current-theme",
        page: "theme",
        title: "Current theme",
        aliases: "appearance palette colors selected theme",
    },
    Target {
        id: "themes",
        page: "theme",
        title: "Theme library",
        aliases: "appearance colors custom themes add edit light dark",
    },
    Target {
        id: "interface-font",
        page: "theme",
        title: "Interface font",
        aliases: "appearance font typeface family typography",
    },
    Target {
        id: "text",
        page: "theme",
        title: "Text size",
        aliases: "appearance font typography pixels zoom scaling",
    },
    Target {
        id: "date-format",
        page: "general",
        title: "Modified date format",
        aliases: "date time format relative iso 8601 long absolute modified timestamp clock appearance",
    },
    Target {
        id: "roundness",
        page: "theme",
        title: "Corner roundness",
        aliases: "appearance corners radius rounded square shape borders",
    },
    Target {
        id: "show-thumbnails",
        page: "theme",
        title: "Show thumbnails",
        aliases: "appearance images pictures photos videos documents thumbnails icons previews disable",
    },
    Target {
        id: "stl-thumbnails",
        page: "theme",
        title: "STL model thumbnails",
        aliases: "appearance 3d stl models printing mesh rendered thumbnails icons",
    },
    Target {
        id: "file-icon-colors",
        page: "theme",
        title: "File icon colors",
        aliases: "appearance file type icons brand palette accent theme monochrome catppuccin",
    },
    Target {
        id: "glow",
        page: "theme",
        title: "Element glow",
        aliases: "appearance effects dialogs menus shadows accent",
    },
    Target {
        id: "motion",
        page: "theme",
        title: "Reduce motion",
        aliases: "appearance disable animations",
    },
    Target {
        id: "renderer",
        page: "theme",
        title: "Interface renderer",
        aliases: "appearance graphics cairo gtk gpu text artifacts performance restart",
    },
    Target {
        id: "check",
        page: "updates",
        title: "Check for updates",
        aliases: "version status install download upgrade",
    },
    Target {
        id: "auto-updates",
        page: "updates",
        title: "Check for updates automatically",
        aliases: "startup github release",
    },
    Target {
        id: "channel",
        page: "updates",
        title: "Release channel",
        aliases: "stable preview nightly builds",
    },
    Target {
        id: "release",
        page: "updates",
        title: "Install a specific release",
        aliases: "rollback roll back downgrade older previous version choose pick",
    },
    Target {
        id: "notes",
        page: "updates",
        title: "Release notes",
        aliases: "what new changes changelog",
    },
    Target {
        id: "integrations",
        page: "integrations",
        title: "Integrations",
        aliases: "add-ons addons plugins extensions providers catalog install kde connect localsend nextcloud dropbox cloud devices phone",
    },
    Target {
        id: "diagnostics",
        page: "diagnostics",
        title: "Run diagnostics",
        aliases: "troubleshoot troubleshooting problems health check dependencies missing packages report bug sandbox bubblewrap ffmpeg gstreamer gvfs sftp smb udisks portal",
    },
    Target {
        id: "version",
        page: "about",
        title: "Version information",
        aliases: "strata build commit toolkit gtk copy author",
    },
    Target {
        id: "website",
        page: "about",
        title: "Website",
        aliases: "links stratafiles home",
    },
    Target {
        id: "source",
        page: "about",
        title: "Source code",
        aliases: "links github repository",
    },
    Target {
        id: "issue",
        page: "about",
        title: "Report an issue",
        aliases: "links bug feedback",
    },
    Target {
        id: "license",
        page: "about",
        title: "License",
        aliases: "links mit copyright",
    },
    Target {
        id: KEYBOARD_SHORTCUTS,
        page: "keyboard",
        title: "Keyboard shortcuts",
        aliases: "keymap keybindings hotkeys accelerators rebind remap keys 10xer default reset",
    },
];

const KEYBOARD_SHORTCUTS: &str = "keyboard-shortcuts";

pub(super) fn tag(widget: &impl IsA<gtk::Widget>, title: &str) {
    if let Some(target) = TARGETS.iter().find(|target| target.title == title) {
        widget.set_widget_name(&format!("settings-search-{}", target.id));
    }
}

/// Registers a row that [`super::keyboard::search_targets`] lists by `id`.
pub(super) fn tag_id(widget: &impl IsA<gtk::Widget>, id: &str) {
    widget.set_widget_name(&format!("settings-search-{id}"));
}

pub(super) fn set_available(widget: &impl IsA<gtk::Widget>, available: bool) {
    crate::ui::desktop_integration::set_search_available(widget, available);
}

fn normalized(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn distance(a: &str, b: &str) -> usize {
    let b = b.chars().collect::<Vec<_>>();
    let mut previous = (0..=b.len()).collect::<Vec<_>>();
    for (i, a) in a.chars().enumerate() {
        let mut next = vec![i + 1];
        for (j, b) in b.iter().enumerate() {
            next.push(
                (previous[j + 1] + 1)
                    .min(next[j] + 1)
                    .min(previous[j] + usize::from(a != *b)),
            );
        }
        previous = next;
    }
    previous[b.len()]
}

/// Kana and Han ideographs.
fn is_unspaced_script(character: char) -> bool {
    matches!(
        character,
        '\u{3040}'..='\u{30ff}'
            | '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{f900}'..='\u{faff}'
            | '\u{20000}'..='\u{3134f}'
    )
}

fn word_score(query: &str, word: &str) -> Option<i32> {
    if query == word {
        Some(80)
    } else if word.starts_with(query) {
        Some(60)
    } else if query.chars().count() >= 2
        && query.chars().any(is_unspaced_script)
        && word.contains(query)
    {
        Some(40)
    } else if query.chars().count() >= 4
        && distance(query, word) <= if query.chars().count() >= 6 { 2 } else { 1 }
    {
        Some(20)
    } else {
        None
    }
}

fn score(query: &str, target: &Target) -> Option<i32> {
    score_text(
        query,
        target.title,
        target.page,
        &format!("{} {}", target.aliases, localized_keywords(target)),
    )
}

fn score_text(query: &str, title: &str, page: &str, aliases: &str) -> Option<i32> {
    let (source_title, localized_title) = if title.is_empty() {
        (String::new(), String::new())
    } else {
        (
            normalized(&super::translated_in("en", title)),
            normalized(&super::translated(title)),
        )
    };
    let title = format!("{source_title} {localized_title}");
    // Chinese and Japanese queries run across the spaces titles put around Latin words.
    let joined_title = localized_title.split_whitespace().collect::<String>();
    let aliases = normalized(&format!(
        "{} {} {} settings {}",
        aliases,
        page,
        page_title(page).map(crate::i18n::tr).unwrap_or_default(),
        crate::i18n::tr("Settings"),
    ));
    let mut total = 0;
    for query_word in query.split_whitespace() {
        let title_score = title
            .split_whitespace()
            .chain([joined_title.as_str()])
            .filter_map(|word| word_score(query_word, word))
            .max();
        let alias_score = aliases
            .split_whitespace()
            .filter_map(|word| word_score(query_word, word).map(|score| score / 2))
            .max();
        total += title_score.into_iter().chain(alias_score).max()?;
    }
    Some(
        total
            + if source_title.is_empty() {
                0
            } else if source_title == query || localized_title == query {
                1000
            } else if source_title.contains(query) || localized_title.contains(query) {
                200
            } else {
                0
            },
    )
}

/// Semantic keys; a missing entry adds nothing, since the English aliases are always searched.
fn localized_keywords(target: &Target) -> String {
    let key = format!("settings_keywords.{}", target.id);
    let keywords = crate::i18n::tr(&key);
    if keywords == key {
        String::new()
    } else {
        keywords
    }
}

fn page_title(page: &str) -> Option<&'static str> {
    super::NAVIGATION
        .iter()
        .find(|entry| entry.page == page)
        .map(|entry| entry.label)
}

#[derive(Default)]
pub(super) struct Matches {
    ids: HashSet<String>,
    pages: HashSet<&'static str>,
    best_page: Option<&'static str>,
}

fn find_matches(query: &str) -> Matches {
    if query.chars().take(129).count() > 128 {
        return Matches::default();
    }
    let mut result = Matches::default();
    let mut best = -1;
    for target in TARGETS
        .iter()
        .filter(|target| super::page_shown(target.page))
    {
        if let Some(score) = score(query, target) {
            result.ids.insert(target.id.to_owned());
            result.pages.insert(target.page);
            if score > best {
                best = score;
                result.best_page = Some(target.page);
            }
        }
    }
    // A query for the page itself lists every row; otherwise action names
    // score as aliases, so a setting keeps a query it ties.
    let whole_page = result.ids.contains(KEYBOARD_SHORTCUTS);
    for target in super::keyboard::search_targets() {
        let aliases = format!("{} {}", target.names, target.aliases);
        let scored = score_text(query, "", super::keyboard::PAGE, &aliases);
        if whole_page || scored.is_some() {
            result.ids.insert(target.id);
        }
        if let Some(score) = scored {
            // Keeps the table switch shown, since a row may match only in the hidden table.
            result.ids.insert(KEYBOARD_SHORTCUTS.to_owned());
            result.pages.insert(super::keyboard::PAGE);
            if score > best {
                best = score;
                result.best_page = Some(super::keyboard::PAGE);
            }
        }
    }
    result
}

pub(super) type State = Rc<RefCell<Option<Matches>>>;

pub(super) fn apply(widget: &impl IsA<gtk::Widget>, state: &State) {
    filter_tree(widget.upcast_ref(), state.borrow().as_ref());
}

fn children(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut children = Vec::new();
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        children.push(widget);
    }
    children
}

fn filter_tree(widget: &gtk::Widget, matches: Option<&Matches>) -> Option<bool> {
    if let Some(id) = widget.widget_name().strip_prefix("settings-search-")
        && (TARGETS.iter().any(|target| target.id == id) || super::keyboard::is_target(id))
    {
        let visible = !widget.has_css_class("settings-search-unavailable")
            && matches.is_none_or(|matches| matches.ids.contains(id));
        widget.set_visible(visible);
        return Some(visible);
    }
    let children = children(widget);
    let results = children
        .iter()
        .map(|child| filter_tree(child, matches))
        .collect::<Vec<_>>();
    if !results.iter().any(Option::is_some) {
        return None;
    }
    let visible = results.contains(&Some(true));
    if widget.has_css_class("settings-group") {
        widget.set_visible(visible);
    }
    if widget.has_css_class("settings-preferences")
        || widget.has_css_class("settings-search-section")
    {
        for (index, child) in children.iter().enumerate() {
            if child.has_css_class("settings-search-page-empty") {
                child.set_visible(matches.is_some() && !visible);
            }
            if child.has_css_class("menu-heading")
                || child.has_css_class("settings-section-description")
            {
                let mut any_tagged = false;
                let mut any_visible = false;
                for (result, sibling) in results[index + 1..].iter().zip(&children[index + 1..]) {
                    if sibling.has_css_class("menu-heading") {
                        break;
                    }
                    if let Some(visible) = *result {
                        any_tagged = true;
                        any_visible |= visible;
                    }
                }
                child.set_visible(any_visible || !any_tagged);
            }
        }
    }
    Some(visible)
}

pub(super) struct Search {
    pub entry: gtk::Entry,
    pub state: State,
}

pub(super) fn append(navigation: &gtk::Box) -> Search {
    let (field, entry, clear) = super::search_field("Search settings");
    field.add_css_class("settings-global-search");
    entry.add_css_class("settings-global-search-entry");
    entry.set_width_chars(1);
    crate::ui::accessibility::set_label(&entry, &crate::i18n::tr("Search settings"));
    navigation.append(&field);
    let (popup_field, popup_entry, popup_clear) = super::search_field("Search settings");
    popup_field.set_size_request(260, -1);
    popup_entry.set_width_chars(1);
    let popover = gtk::Popover::builder()
        .child(&popup_field)
        .has_arrow(false)
        .build();
    popover.add_css_class("settings-search-popover");
    let compact = gtk::MenuButton::new();
    compact.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::SEARCH,
        18,
    )));
    compact.set_tooltip_text(Some(&crate::i18n::tr("Search settings")));
    crate::ui::accessibility::set_label(&compact, &crate::i18n::tr("Search settings"));
    compact.set_popover(Some(&popover));
    compact.add_css_class("settings-global-search-compact");
    compact.set_visible(false);
    navigation.append(&compact);
    crate::ui::accessibility::set_label(&popup_entry, &crate::i18n::tr("Search settings"));
    entry
        .bind_property("text", &popup_entry, "text")
        .bidirectional()
        .sync_create()
        .build();
    for (source, clear) in [(&entry, clear), (&popup_entry, popup_clear)] {
        source.connect_changed(move |source| {
            clear.set_visible(!source.text().is_empty());
        });
        let keys = gtk::EventControllerKey::new();
        let field = source.downgrade();
        keys::on_press(&keys, move |_, press| {
            if press.is(ESCAPE_ANY)
                && let Some(field) = field.upgrade()
                && !field.text().is_empty()
            {
                field.set_text("");
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        source.add_controller(keys);
    }
    popover.connect_show(move |_| {
        popup_entry.grab_focus();
    });
    Search {
        entry,
        state: Rc::new(RefCell::new(None)),
    }
}

pub(super) fn set_compact(navigation: &gtk::Box, compact: bool) {
    let children = children(navigation.upcast_ref());
    let field = children
        .iter()
        .find(|child| child.has_css_class("settings-global-search"));
    let button = children
        .iter()
        .find(|child| child.has_css_class("settings-global-search-compact"));
    let (Some(field), Some(button)) = (field, button) else {
        return;
    };
    let transfer_focus = navigation
        .root()
        .and_then(|root| root.focus())
        .is_some_and(|focus| focus.is_ancestor(if compact { field } else { button }));
    field.set_visible(!compact);
    button.set_visible(compact);
    if transfer_focus {
        let target = if compact {
            button.clone()
        } else {
            field.first_child().expect("search entry")
        }
        .downgrade();
        glib::idle_add_local_once(move || {
            if let Some(target) = target.upgrade()
                && target.is_mapped()
            {
                if let Some(button) = target.downcast_ref::<gtk::MenuButton>() {
                    button.popup();
                } else {
                    target.grab_focus();
                }
            }
        });
    }
}

impl Search {
    pub(super) fn install(
        &self,
        stack: &gtk::Stack,
        title: &gtk::Label,
        buttons: &Rc<RefCell<Vec<(gtk::Button, &'static str)>>>,
    ) {
        let empty = gtk::Label::new(Some(&crate::i18n::tr("No settings match your search.")));
        empty.set_wrap(true);
        empty.add_css_class("settings-option-description");
        stack.add_named(&empty, Some("settings-search-empty"));
        let stack = stack.downgrade();
        let title = title.downgrade();
        let buttons = buttons
            .borrow()
            .iter()
            .map(|(button, page)| (button.downgrade(), *page))
            .collect::<Vec<_>>();
        let state = self.state.clone();
        self.entry
            .connect_notify_local(Some("text"), move |entry, _| {
                let Some(stack) = stack.upgrade() else {
                    return;
                };
                let query = normalized(&entry.text());
                let matches = if query.is_empty() {
                    None
                } else {
                    Some(find_matches(&query))
                };
                let destination = matches.as_ref().and_then(|matches| matches.best_page);
                let no_matches = matches
                    .as_ref()
                    .is_some_and(|matches| matches.ids.is_empty());
                for (button, page) in &buttons {
                    if let Some(button) = button.upgrade() {
                        button.set_visible(
                            matches
                                .as_ref()
                                .is_none_or(|matches| matches.pages.contains(page)),
                        );
                    }
                }
                state.replace(matches);
                let destination = destination.or_else(|| {
                    (stack.visible_child_name().as_deref() == Some("settings-search-empty"))
                        .then_some("general")
                });
                if no_matches {
                    stack.set_visible_child_name("settings-search-empty");
                    if let Some(title) = title.upgrade() {
                        title.set_text(&crate::i18n::tr("Search results"));
                    }
                } else if let Some(destination) = destination {
                    for (button, page) in &buttons {
                        if *page == destination
                            && let Some(button) = button.upgrade()
                        {
                            button.emit_clicked();
                            break;
                        }
                    }
                }
                for page in children(stack.upcast_ref()) {
                    apply(&page, &state);
                    reset_scroll(&page);
                }
            });
    }
}

fn reset_scroll(widget: &gtk::Widget) {
    if widget.has_css_class("settings-content-scroll")
        && let Some(scroll) = widget.downcast_ref::<gtk::ScrolledWindow>()
    {
        scroll.vadjustment().set_value(0.0);
        return;
    }
    for child in children(widget) {
        reset_scroll(&child);
    }
}

#[cfg(test)]
mod tests;
