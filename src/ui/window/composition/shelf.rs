// SPDX-License-Identifier: MIT

//! Session-only references to files collected for a later transfer.

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

use gtk::{gdk, prelude::*};

use crate::{
    assets::{self, icons},
    model::Location,
    services::DropCommit,
    ui::browser::paths::is_trash_location,
    ui::browser::{
        BrowserView, file_drag_locations, locations_from_file_list_value, show_error_dialog,
    },
};

#[derive(Default)]
struct Shelf {
    locations: RefCell<Vec<Location>>,
    listeners: RefCell<Vec<Weak<dyn Fn()>>>,
}

thread_local! {
    static SHARED_SHELF: Rc<Shelf> = Rc::new(Shelf::default());
}

impl Shelf {
    fn shared() -> Rc<Self> {
        SHARED_SHELF.with(Rc::clone)
    }

    fn locations(&self) -> Vec<Location> {
        self.locations.borrow().clone()
    }

    fn add(&self, incoming: impl IntoIterator<Item = Location>) -> usize {
        let mut locations = self.locations.borrow_mut();
        let mut added = 0;
        for location in incoming {
            if !location.is_recent_location() && !locations.contains(&location) {
                locations.push(location);
                added += 1;
            }
        }
        drop(locations);
        if added != 0 {
            self.notify();
        }
        added
    }

    fn remove(&self, location: &Location) {
        let mut locations = self.locations.borrow_mut();
        let before = locations.len();
        locations.retain(|item| item != location);
        let removed = locations.len() != before;
        drop(locations);
        if removed {
            self.notify();
        }
    }

    fn clear(&self) {
        if !self.locations.borrow().is_empty() {
            self.locations.borrow_mut().clear();
            self.notify();
        }
    }

    fn notify(&self) {
        self.listeners.borrow_mut().retain(|listener| {
            if let Some(listener) = listener.upgrade() {
                listener();
                true
            } else {
                false
            }
        });
    }
}

pub(super) struct ShelfView {
    pub(super) widget: gtk::Box,
    _refresh: Rc<dyn Fn()>,
}

impl ShelfView {
    pub(super) fn new(browser: &BrowserView) -> Self {
        let shelf = Shelf::shared();
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("file-shelf");

        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.add_css_class("file-shelf-bar");
        let toggle = gtk::ToggleButton::with_label("Shelf (0)");
        toggle.add_css_class("shortcut-footer-button");
        toggle.add_css_class("file-shelf-toggle");
        bar.append(&toggle);
        let hint = gtk::Label::new(Some("Drop files here to collect them"));
        hint.add_css_class("file-shelf-hint");
        hint.set_hexpand(true);
        hint.set_xalign(0.0);
        bar.append(&hint);
        let add = gtk::Button::with_label("Add selection");
        add.add_css_class("shortcut-footer-button");
        bar.append(&add);
        widget.append(&bar);

        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideUp);
        toggle.connect_toggled({
            let revealer = revealer.clone();
            move |toggle| revealer.set_reveal_child(toggle.is_active())
        });
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.add_css_class("file-shelf-content");
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_max_content_height(180);
        scroll.set_propagate_natural_height(true);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("file-shelf-list");
        scroll.set_child(Some(&list));
        content.append(&scroll);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("file-shelf-actions");
        let clear = gtk::Button::with_label("Clear shelf");
        clear.add_css_class("shortcut-footer-button");
        clear.connect_clicked({
            let shelf = shelf.clone();
            move |_| shelf.clear()
        });
        actions.append(&clear);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        actions.append(&spacer);
        let copy = gtk::Button::with_label("Copy here");
        copy.add_css_class("shortcut-footer-button");
        let move_button = gtk::Button::with_label("Move here");
        move_button.add_css_class("shortcut-footer-button");
        for (button, commit) in [(&copy, DropCommit::Copy), (&move_button, DropCommit::Move)] {
            let browser = browser.downgrade();
            let shelf = shelf.clone();
            button.connect_clicked(move |button| {
                let Some(browser) = browser.upgrade() else {
                    return;
                };
                let Some(destination) = browser.browser().active_location() else {
                    show_error_dialog(
                        button,
                        "Unable to transfer",
                        "Choose a folder as the destination.",
                    );
                    return;
                };
                if destination.is_recent_location() || is_trash_location(&destination) {
                    show_error_dialog(
                        button,
                        "Unable to transfer",
                        "Choose a regular folder as the destination.",
                    );
                    return;
                }
                browser.commit_file_drop(destination, shelf.locations(), commit);
            });
        }
        actions.append(&copy);
        actions.append(&move_button);
        content.append(&actions);
        revealer.set_child(Some(&content));
        widget.append(&revealer);

        add.connect_clicked({
            let shelf = shelf.clone();
            let browser = browser.downgrade();
            let toggle = toggle.clone();
            move |_| {
                if let Some(browser) = browser.upgrade() {
                    let selected = browser
                        .browser()
                        .selected_entries()
                        .into_iter()
                        .map(|entry| entry.location);
                    if shelf.add(selected) != 0 {
                        toggle.set_active(true);
                    }
                }
            }
        });

        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        drop.connect_enter(|target, _, _| {
            if let Some(widget) = target.widget() {
                widget.add_css_class("drop-destination");
            }
            gdk::DragAction::COPY
        });
        drop.connect_leave(|target| {
            if let Some(widget) = target.widget() {
                widget.remove_css_class("drop-destination");
            }
        });
        drop.connect_drop({
            let shelf = shelf.clone();
            let toggle = toggle.clone();
            move |target, value, _, _| {
                if let Some(widget) = target.widget() {
                    widget.remove_css_class("drop-destination");
                }
                let Some(locations) = locations_from_file_list_value(value) else {
                    return false;
                };
                if shelf.add(locations) != 0 {
                    toggle.set_active(true);
                }
                true
            }
        });
        bar.add_controller(drop);

        let refresh: Rc<dyn Fn()> = Rc::new({
            let shelf = Rc::downgrade(&shelf);
            let list = list.downgrade();
            let toggle = toggle.downgrade();
            let clear = clear.downgrade();
            let copy = copy.downgrade();
            let move_button = move_button.downgrade();
            move || {
                let (
                    Some(shelf),
                    Some(list),
                    Some(toggle),
                    Some(clear),
                    Some(copy),
                    Some(move_button),
                ) = (
                    shelf.upgrade(),
                    list.upgrade(),
                    toggle.upgrade(),
                    clear.upgrade(),
                    copy.upgrade(),
                    move_button.upgrade(),
                )
                else {
                    return;
                };
                while let Some(row) = list.first_child() {
                    list.remove(&row);
                }
                let locations = shelf.locations();
                toggle.set_label(&format!("Shelf ({})", locations.len()));
                let has_items = !locations.is_empty();
                if !has_items {
                    toggle.set_active(false);
                }
                clear.set_sensitive(has_items);
                copy.set_sensitive(has_items);
                move_button.set_sensitive(has_items);
                for location in locations {
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    row.add_css_class("file-shelf-row");
                    let name = location
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| location.display_path());
                    let label = gtk::Label::new(Some(&name));
                    label.set_xalign(0.0);
                    label.set_hexpand(true);
                    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    row.append(&label);
                    let path = gtk::Label::new(Some(&location.display_path()));
                    path.add_css_class("file-shelf-path");
                    path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                    row.append(&path);
                    let remove = gtk::Button::new();
                    remove.set_child(Some(&assets::primary_icon(icons::X, 15)));
                    remove.set_tooltip_text(Some(&format!("Remove {name} from shelf")));
                    crate::ui::accessibility::set_label(
                        &remove,
                        &format!("Remove {name} from shelf"),
                    );
                    remove.add_css_class("shortcut-footer-button");
                    remove.add_css_class("file-shelf-remove");
                    remove.connect_clicked({
                        let shelf = shelf.clone();
                        let location = location.clone();
                        move |_| shelf.remove(&location)
                    });
                    row.append(&remove);
                    let drag = gtk::DragSource::builder()
                        .actions(gdk::DragAction::COPY | gdk::DragAction::MOVE)
                        .build();
                    drag.connect_prepare({
                        let location = location.clone();
                        move |_, _, _| file_drag_locations(std::slice::from_ref(&location))
                    });
                    row.add_controller(drag);
                    list.append(&row);
                }
            }
        });
        shelf.listeners.borrow_mut().push(Rc::downgrade(&refresh));
        refresh();
        Self {
            widget,
            _refresh: refresh,
        }
    }
}

#[cfg(test)]
mod tests;
