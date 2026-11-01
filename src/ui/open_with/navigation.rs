// SPDX-License-Identifier: MIT

use gtk::{glib, prelude::*};

use crate::ui::{
    controls::ModalLayout,
    keys::{
        self, KeyPress, SHORTCUT_MODIFIERS,
        bindings::{BACKSPACE, DOWN, FOCUS_NEXT, FOCUS_PREVIOUS, UP},
    },
};

/// The dialog's Tab stops, held weakly by the key handler.
struct TabStops {
    search: glib::WeakRef<gtk::SearchEntry>,
    list: glib::WeakRef<gtk::ListBox>,
    always: glib::WeakRef<gtk::CheckButton>,
    close: glib::WeakRef<gtk::Button>,
    cancel: glib::WeakRef<gtk::Button>,
}

struct FocusTargets {
    search: gtk::SearchEntry,
    list: gtk::ListBox,
    always: gtk::CheckButton,
    close: gtk::Button,
    cancel: gtk::Button,
}

impl TabStops {
    fn upgrade(&self) -> Option<FocusTargets> {
        Some(FocusTargets {
            search: self.search.upgrade()?,
            list: self.list.upgrade()?,
            always: self.always.upgrade()?,
            close: self.close.upgrade()?,
            cancel: self.cancel.upgrade()?,
        })
    }
}

impl FocusTargets {
    fn focus_selected_row(&self) -> bool {
        if self.list.is_visible() {
            self.list.selected_row().is_some_and(|row| row.grab_focus())
        } else {
            false
        }
    }

    /// A hidden widget still accepts focus grabs, so gate on visibility like the list.
    fn focus_always(&self) -> bool {
        self.always.is_visible() && self.always.grab_focus()
    }

    fn focus_search_or_close(&self) -> bool {
        if self.search.is_visible() {
            self.search.grab_focus()
        } else {
            self.close.grab_focus()
        }
    }

    fn focus_list_or_cancel(&self) -> bool {
        if self.focus_selected_row() || self.focus_always() {
            return true;
        }
        self.cancel.grab_focus()
    }

    fn from_search(&self, backward: bool) -> bool {
        if backward {
            return self.close.grab_focus();
        }
        self.focus_list_or_cancel()
    }

    fn from_list(&self, backward: bool) -> bool {
        if backward {
            return self.focus_search_or_close();
        }
        if self.focus_always() {
            return true;
        }
        self.cancel.grab_focus()
    }

    fn back_from_cancel(&self) -> bool {
        if self.focus_always() || self.focus_selected_row() {
            return true;
        }
        self.focus_search_or_close()
    }

    fn forward_from_close(&self) -> bool {
        if self.search.is_visible() {
            return self.search.grab_focus();
        }
        self.focus_list_or_cancel()
    }

    fn move_focus(&self, focus: &gtk::Widget, backward: bool) -> bool {
        if *focus == self.search || focus.is_ancestor(&self.search) {
            return self.from_search(backward);
        }
        if *focus == self.list || focus.is_ancestor(&self.list) {
            return self.from_list(backward);
        }
        if backward && *focus == self.cancel {
            return self.back_from_cancel();
        }
        if !backward && *focus == self.close {
            return self.forward_from_close();
        }
        false
    }
}

pub(super) fn install_list_tab_navigation(
    layout: &ModalLayout,
    search_entry: &gtk::SearchEntry,
    list: &gtk::ListBox,
    always_use: &gtk::CheckButton,
) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let stops = TabStops {
        search: search_entry.downgrade(),
        list: list.downgrade(),
        always: always_use.downgrade(),
        close: layout.close.downgrade(),
        cancel: layout.cancel.downgrade(),
    };
    keys::on_press(&keys, move |_, press| {
        let backward = press.is(FOCUS_PREVIOUS);
        if !backward && !press.is(FOCUS_NEXT) {
            return glib::Propagation::Proceed;
        }
        let Some(targets) = stops.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let Some(focus) = targets.list.root().and_then(|root| root.focus()) else {
            return glib::Propagation::Proceed;
        };
        if targets.move_focus(&focus, backward) {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layout.content.add_controller(keys);
}

/// Up and Down in the search field move the list selection past headings.
pub(super) fn install_search_arrows(
    search_entry: &gtk::SearchEntry,
    list: &gtk::ListBox,
    list_scroll: &gtk::ScrolledWindow,
) {
    let search_keys = gtk::EventControllerKey::new();
    search_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let list_for_search = list.downgrade();
    let scroll_for_search = list_scroll.downgrade();
    keys::on_press(&search_keys, move |_, press| {
        let down = press.is(DOWN);
        if !down && !press.is(UP) {
            return glib::Propagation::Proceed;
        }
        if let (Some(list), Some(scroll)) = (list_for_search.upgrade(), scroll_for_search.upgrade())
            && let Some(selected) = list.selected_row()
        {
            let row = next_selectable_row(&selected, down).unwrap_or(selected);
            list.select_row(Some(&row));
            scroll_to_row(&list, &scroll, &row);
        }
        glib::Propagation::Stop
    });
    search_entry.add_controller(search_keys);
}

fn sibling_row(widget: &gtk::Widget, down: bool) -> Option<gtk::Widget> {
    if down {
        widget.next_sibling()
    } else {
        widget.prev_sibling()
    }
}

fn next_selectable_row(selected: &gtk::ListBoxRow, down: bool) -> Option<gtk::ListBoxRow> {
    let mut sibling = sibling_row(selected.upcast_ref(), down);
    while let Some(candidate) = sibling {
        if let Some(row) = selectable_row(&candidate) {
            return Some(row);
        }
        sibling = sibling_row(&candidate, down);
    }
    None
}

fn selectable_row(candidate: &gtk::Widget) -> Option<gtk::ListBoxRow> {
    if !candidate.is_visible() {
        return None;
    }
    candidate
        .downcast_ref::<gtk::ListBoxRow>()
        .filter(|row| row.is_selectable())
        .cloned()
}

fn scroll_to_row(list: &gtk::ListBox, scroll: &gtk::ScrolledWindow, row: &gtk::ListBoxRow) {
    let Some(bounds) = row.compute_bounds(list) else {
        return;
    };
    let adjustment = scroll.vadjustment();
    let top = f64::from(bounds.y());
    let bottom = top + f64::from(bounds.height());
    if top < adjustment.value() {
        adjustment.set_value(top);
    } else if bottom > adjustment.value() + adjustment.page_size() {
        adjustment.set_value(bottom - adjustment.page_size());
    }
}

/// Typing in the list edits the search field, and Up from the first row returns to it.
pub(super) fn install_list_typing(list: &gtk::ListBox, search_entry: &gtk::SearchEntry) {
    let list_keys = gtk::EventControllerKey::new();
    let search_for_list = search_entry.downgrade();
    let list_weak = list.downgrade();
    keys::on_press(&list_keys, move |_, press| {
        let (Some(list), Some(search)) = (list_weak.upgrade(), search_for_list.upgrade()) else {
            return glib::Propagation::Proceed;
        };
        list_key(&list, &search, &press.untyped())
    });
    list.add_controller(list_keys);
}

fn list_key(list: &gtk::ListBox, search: &gtk::SearchEntry, press: &KeyPress) -> glib::Propagation {
    if press.is(UP.ignoring(SHORTCUT_MODIFIERS)) && up_returns_to_search(list, search) {
        search.grab_focus();
        return glib::Propagation::Stop;
    }
    if !search.is_visible() {
        return glib::Propagation::Proceed;
    }
    if press.is(BACKSPACE.ignoring(gtk::gdk::ModifierType::SHIFT_MASK)) {
        edit_search(search, |text| {
            text.pop();
        });
        return glib::Propagation::Stop;
    }
    if let Some(ch) = press.text() {
        edit_search(search, |text| text.push(ch));
        return glib::Propagation::Stop;
    }
    glib::Propagation::Proceed
}

fn up_returns_to_search(list: &gtk::ListBox, search: &gtk::SearchEntry) -> bool {
    let Some(selected) = list.selected_row() else {
        return false;
    };
    !has_previous_focusable(&selected) && search.is_visible()
}

fn has_previous_focusable(selected: &gtk::ListBoxRow) -> bool {
    let mut prev = selected.prev_sibling();
    while let Some(widget) = prev {
        if widget.is_visible() && widget.can_focus() {
            return true;
        }
        prev = widget.prev_sibling();
    }
    false
}

fn edit_search(search: &gtk::SearchEntry, edit: impl FnOnce(&mut String)) {
    search.grab_focus();
    let mut text = search.text().to_string();
    edit(&mut text);
    search.set_text(&text);
    search.set_position(-1);
}
