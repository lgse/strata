// SPDX-License-Identifier: MIT

mod filter;
mod matching;
mod targets;

use gtk::{glib, prelude::*};
use std::{cell::RefCell, rc::Rc};

use crate::ui::keys::{self, KeyPress, bindings::ESCAPE_ANY};
use filter::{children, reset_scroll};
use matching::{Matches, find_matches, normalized};
use targets::TARGETS;

pub(super) use filter::{State, apply};

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
    bind_clear(&entry, clear);
    bind_clear(&popup_entry, popup_clear);
    popover.connect_show(move |_| {
        popup_entry.grab_focus();
    });
    Search {
        entry,
        state: Rc::new(RefCell::new(None)),
    }
}

/// Shows the clear button while the field has text, and Escape empties it.
fn bind_clear(entry: &gtk::Entry, clear: gtk::Button) {
    entry.connect_changed(move |entry| {
        clear.set_visible(!entry.text().is_empty());
    });
    let keys = gtk::EventControllerKey::new();
    let field = entry.downgrade();
    keys::on_press(&keys, move |_, press| match field.upgrade() {
        Some(field) if clears_field(press, &field) => {
            field.set_text("");
            glib::Propagation::Stop
        }
        _ => glib::Propagation::Proceed,
    });
    entry.add_controller(keys);
}

fn clears_field(press: &KeyPress, field: &gtk::Entry) -> bool {
    press.is(ESCAPE_ANY) && !field.text().is_empty()
}

pub(super) fn set_compact(navigation: &gtk::Box, compact: bool) {
    let children = children(navigation.upcast_ref());
    let find = |class: &str| children.iter().find(|child| child.has_css_class(class));
    let (Some(field), Some(button)) = (
        find("settings-global-search"),
        find("settings-global-search-compact"),
    ) else {
        return;
    };
    let hidden = if compact { field } else { button };
    let transfer_focus = navigation
        .root()
        .and_then(|root| root.focus())
        .is_some_and(|focus| focus.is_ancestor(hidden));
    field.set_visible(!compact);
    button.set_visible(compact);
    if transfer_focus {
        let target = if compact {
            button.clone()
        } else {
            field.first_child().expect("search entry")
        };
        focus_once_mapped(&target);
    }
}

fn focus_once_mapped(target: &gtk::Widget) {
    let target = target.downgrade();
    glib::idle_add_local_once(move || {
        let Some(target) = target.upgrade().filter(gtk::Widget::is_mapped) else {
            return;
        };
        match target.downcast_ref::<gtk::MenuButton>() {
            Some(button) => button.popup(),
            None => {
                target.grab_focus();
            }
        }
    });
}

const EMPTY_PAGE: &str = "settings-search-empty";

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
        stack.add_named(&empty, Some(EMPTY_PAGE));
        let pages = Pages {
            stack: stack.downgrade(),
            title: title.downgrade(),
            buttons: buttons
                .borrow()
                .iter()
                .map(|(button, page)| (button.downgrade(), *page))
                .collect(),
            state: self.state.clone(),
        };
        self.entry
            .connect_notify_local(Some("text"), move |entry, _| {
                if let Some(stack) = pages.stack.upgrade() {
                    pages.show(&stack, &normalized(&entry.text()));
                }
            });
    }
}

struct Pages {
    stack: glib::WeakRef<gtk::Stack>,
    title: glib::WeakRef<gtk::Label>,
    buttons: Vec<(glib::WeakRef<gtk::Button>, &'static str)>,
    state: State,
}

impl Pages {
    fn show(&self, stack: &gtk::Stack, query: &str) {
        let matches = (!query.is_empty()).then(|| find_matches(query));
        let destination = matches.as_ref().and_then(|matches| matches.best_page);
        let no_matches = matches
            .as_ref()
            .is_some_and(|matches| matches.ids.is_empty());
        self.show_page_buttons(matches.as_ref());
        self.state.replace(matches);
        // Clearing the query leaves the empty page, so fall back to the first page.
        let destination = destination.or_else(|| {
            (stack.visible_child_name().as_deref() == Some(EMPTY_PAGE)).then_some("general")
        });
        if no_matches {
            self.show_empty(stack);
        } else if let Some(destination) = destination {
            self.open(destination);
        }
        for page in children(stack.upcast_ref()) {
            apply(&page, &self.state);
            reset_scroll(&page);
        }
    }

    fn show_page_buttons(&self, matches: Option<&Matches>) {
        for (button, page) in &self.buttons {
            if let Some(button) = button.upgrade() {
                button.set_visible(matches.is_none_or(|matches| matches.pages.contains(page)));
            }
        }
    }

    fn show_empty(&self, stack: &gtk::Stack) {
        stack.set_visible_child_name(EMPTY_PAGE);
        if let Some(title) = self.title.upgrade() {
            title.set_text(&crate::i18n::tr("Search results"));
        }
    }

    fn open(&self, destination: &str) {
        let button = self
            .buttons
            .iter()
            .filter(|(_, page)| *page == destination)
            .find_map(|(button, _)| button.upgrade());
        if let Some(button) = button {
            button.emit_clicked();
        }
    }
}

#[cfg(test)]
mod tests;
