// SPDX-License-Identifier: MIT

use gtk::{glib, prelude::*};
use std::{cell::RefCell, collections::HashSet, rc::Rc};

struct Target {
    id: &'static str,
    page: &'static str,
    title: &'static str,
    aliases: &'static str,
}

const TARGETS: &[Target] = &[
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
        id: "refresh",
        page: "general",
        title: "Auto-refresh folder",
        aliases: "performance timer reload network share monitors",
    },
    Target {
        id: "video",
        page: "general",
        title: "Hardware-accelerated video previews",
        aliases: "performance gpu thumbnails decoding backend automatic va api vulkan software",
    },
    Target {
        id: "desktop",
        page: "general",
        title: "Desktop integration",
        aliases: "configure portal file chooser open save dialog default file manager",
    },
    Target {
        id: "omarchy",
        page: "theme",
        title: "Follow Omarchy",
        aliases: "appearance system theme quattro",
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
        id: "text",
        page: "theme",
        title: "Text size",
        aliases: "appearance font typography pixels zoom scaling",
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
        id: "hints",
        page: "keybindings",
        title: "Show keybinding hints",
        aliases: "keyboard shortcuts footer navigation paste",
    },
    Target {
        id: "shortcuts",
        page: "keybindings",
        title: "Shortcut reference",
        aliases: "keyboard keys navigation selection files view application copy paste cut rename delete trash undo terminal refresh",
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
        id: "notes",
        page: "updates",
        title: "Release notes",
        aliases: "what new changes changelog",
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
];

pub(super) fn tag(widget: &impl IsA<gtk::Widget>, title: &str) {
    if let Some(target) = TARGETS.iter().find(|target| target.title == title) {
        widget.set_widget_name(&format!("settings-search-{}", target.id));
    }
}

pub(super) fn set_available(widget: &impl IsA<gtk::Widget>, available: bool) {
    if available {
        widget.remove_css_class("settings-search-unavailable");
    } else {
        widget.add_css_class("settings-search-unavailable");
    }
    widget.set_visible(available);
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

fn word_score(query: &str, word: &str) -> Option<i32> {
    if query == word {
        Some(80)
    } else if word.starts_with(query) {
        Some(60)
    } else if query.chars().count() >= 4
        && distance(query, word) <= if query.chars().count() >= 6 { 2 } else { 1 }
    {
        Some(20)
    } else {
        None
    }
}

fn score(query: &str, target: &Target) -> Option<i32> {
    let title = normalized(target.title);
    let shortcuts = if target.id == "shortcuts" {
        super::keybindings::search_text()
    } else {
        String::new()
    };
    let aliases = normalized(&format!(
        "{} {} settings {shortcuts}",
        target.aliases, target.page
    ));
    let mut total = 0;
    for query_word in query.split_whitespace() {
        let title_score = title
            .split_whitespace()
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
            + if title == query {
                1000
            } else if title.contains(query) {
                200
            } else {
                0
            },
    )
}

#[derive(Default)]
pub(super) struct Matches {
    ids: HashSet<&'static str>,
    pages: HashSet<&'static str>,
    best_page: Option<&'static str>,
    query: String,
}

fn find_matches(query: &str) -> Matches {
    if query.chars().take(129).count() > 128 {
        return Matches::default();
    }
    let mut result = Matches {
        query: query.to_owned(),
        ..Matches::default()
    };
    let mut best = -1;
    for target in TARGETS {
        if let Some(score) = score(query, target) {
            result.ids.insert(target.id);
            result.pages.insert(target.page);
            if score > best {
                best = score;
                result.best_page = Some(target.page);
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
        && TARGETS.iter().any(|target| target.id == id)
    {
        let visible = !widget.has_css_class("settings-search-unavailable")
            && matches.is_none_or(|matches| matches.ids.contains(id));
        widget.set_visible(visible);
        if id == "shortcuts" {
            let query = matches.map(|matches| matches.query.as_str()).unwrap_or("");
            let query = if normalized(&super::keybindings::search_text()).contains(query) {
                query
            } else {
                ""
            };
            filter_shortcuts(widget, query);
        }
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
    if widget.has_css_class("settings-preferences") {
        for (index, child) in children.iter().enumerate() {
            if child.has_css_class("settings-search-page-empty") {
                child.set_visible(matches.is_some() && !visible);
            }
            if child.has_css_class("menu-heading")
                || child.has_css_class("settings-section-description")
            {
                let following = results[index + 1..]
                    .iter()
                    .find_map(|result| *result)
                    .unwrap_or(true);
                child.set_visible(following);
            }
        }
    }
    Some(visible)
}

fn filter_shortcuts(widget: &gtk::Widget, query: &str) {
    if widget.has_css_class("shortcut-search")
        && let Some(entry) = widget.downcast_ref::<gtk::Entry>()
    {
        if entry.text().as_str() != query {
            entry.set_text(query);
        }
        return;
    }
    for child in children(widget) {
        filter_shortcuts(&child, query);
    }
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
    crate::ui::accessibility::set_label(&entry, "Search settings");
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
    compact.set_tooltip_text(Some("Search settings"));
    crate::ui::accessibility::set_label(&compact, "Search settings");
    compact.set_popover(Some(&popover));
    compact.add_css_class("settings-global-search-compact");
    compact.set_visible(false);
    navigation.append(&compact);
    crate::ui::accessibility::set_label(&popup_entry, "Search settings");
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
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape
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
        buttons: &Rc<RefCell<Vec<gtk::Button>>>,
    ) {
        let empty = gtk::Label::new(Some("No settings match your search."));
        empty.set_wrap(true);
        empty.add_css_class("settings-option-description");
        stack.add_named(&empty, Some("settings-search-empty"));
        let stack = stack.downgrade();
        let title = title.downgrade();
        let buttons = buttons
            .borrow()
            .iter()
            .map(|button| {
                (
                    button.downgrade(),
                    match button.tooltip_text().as_deref() {
                        Some("General") => "general",
                        Some("Appearance") => "theme",
                        Some("Keybindings") => "keybindings",
                        Some("Updates") => "updates",
                        _ => "about",
                    },
                )
            })
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
                        title.set_text("Search results");
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
