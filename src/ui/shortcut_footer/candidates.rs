// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{glib, prelude::*};

type ActivateListener = Rc<dyn Fn(PathBuf)>;

pub(super) struct Candidates {
    popover: gtk::Popover,
    scroll: gtk::ScrolledWindow,
    list: gtk::ListBox,
    paths: RefCell<Vec<PathBuf>>,
    chosen: Cell<usize>,
    activated: RefCell<Option<ActivateListener>>,
}

impl Candidates {
    pub(super) fn attach(entry: &gtk::Entry) -> Rc<Self> {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .activate_on_single_click(true)
            .hexpand(true)
            .can_focus(false)
            .focusable(false)
            .build();
        list.add_css_class("path-completion-list");
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .max_content_height(280)
            .propagate_natural_height(true)
            .can_focus(false)
            .focusable(false)
            .build();
        scroll.add_css_class("path-completion-scroll");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&scroll);
        let keys = gtk::Label::new(None);
        keys.set_markup(
            "<b>\u{2191}\u{2193}</b> Choose   <b>\u{21b5}</b> Open   <b>Esc</b> Cancel",
        );
        keys.set_xalign(0.0);
        keys.add_css_class("path-completion-shortcuts");
        content.append(&keys);
        let popover = gtk::Popover::builder()
            .has_arrow(false)
            .autohide(false)
            .position(gtk::PositionType::Top)
            .halign(gtk::Align::Start)
            .can_focus(false)
            .focusable(false)
            .child(&content)
            .build();
        popover.add_css_class("path-completion-popover");
        popover.set_offset(0, -6);
        popover.set_parent(entry);
        let weak_popover = popover.downgrade();
        entry.connect_destroy(move |_| {
            if let Some(popover) = weak_popover.upgrade()
                && popover.parent().is_some()
            {
                popover.unparent();
            }
        });
        let candidates = Rc::new(Self {
            popover,
            scroll,
            list: list.clone(),
            paths: RefCell::default(),
            chosen: Cell::new(0),
            activated: RefCell::default(),
        });
        let weak = Rc::downgrade(&candidates);
        list.connect_row_activated(move |_, row| {
            let Some(candidates) = weak.upgrade() else {
                return;
            };
            let path = usize::try_from(row.index())
                .ok()
                .and_then(|index| candidates.paths.borrow().get(index).cloned());
            let listener = candidates.activated.borrow().clone();
            if let (Some(path), Some(listener)) = (path, listener) {
                listener(path);
            }
        });
        candidates
    }

    pub(super) fn connect_activated(&self, listener: impl Fn(PathBuf) + 'static) {
        self.activated.replace(Some(Rc::new(listener)));
    }

    pub(super) fn set(&self, paths: Vec<PathBuf>) {
        while let Some(row) = self.list.first_child() {
            self.list.remove(&row);
        }
        let home = glib::home_dir();
        for path in &paths {
            self.list.append(&candidate_row(path, &home));
        }
        let empty = paths.is_empty();
        self.paths.replace(paths);
        self.choose(0);
        if empty {
            self.popover.popdown();
            return;
        }
        let Some(entry) = self.popover.parent() else {
            return;
        };
        if entry.width() > 0 {
            self.popover
                .set_size_request(entry.width().clamp(280, 560), -1);
        }
        if self.popover.is_visible() {
            // An open popover keeps its surface height when rows are removed.
            self.popover.present();
        } else {
            self.popover.popup();
        }
    }

    pub(super) fn clear(&self) {
        self.set(Vec::new());
    }

    pub(super) fn step(&self, delta: i32) {
        let count = self.paths.borrow().len();
        if count == 0 {
            return;
        }
        let count = count as i64;
        let next = (self.chosen.get() as i64 + i64::from(delta)).rem_euclid(count);
        self.choose(next as usize);
    }

    pub(super) fn chosen(&self) -> Option<PathBuf> {
        self.paths.borrow().get(self.chosen.get()).cloned()
    }

    pub(super) fn position(&self) -> Option<(usize, usize)> {
        let count = self.paths.borrow().len();
        (count > 0).then(|| (self.chosen.get(), count))
    }

    #[cfg(test)]
    pub(super) fn paths(&self) -> Vec<PathBuf> {
        self.paths.borrow().clone()
    }

    #[cfg(test)]
    pub(super) fn is_shown(&self) -> bool {
        self.popover.is_visible()
    }

    #[cfg(test)]
    pub(super) fn activate(&self, index: usize) {
        if let Some(row) = self.list.row_at_index(index as i32) {
            row.activate();
        }
    }

    fn choose(&self, index: usize) {
        self.chosen.set(index);
        let Some(row) = self.list.row_at_index(index as i32) else {
            self.list.unselect_all();
            return;
        };
        self.list.select_row(Some(&row));
        let adjustment = self.scroll.vadjustment();
        let Some(bounds) = row.compute_bounds(&self.list) else {
            if index == 0 {
                adjustment.set_value(adjustment.lower());
            }
            return;
        };
        let top = f64::from(bounds.y());
        let bottom = f64::from(bounds.y() + bounds.height());
        if top < adjustment.value() {
            adjustment.set_value(top);
        } else if bottom > adjustment.value() + adjustment.page_size() {
            adjustment.set_value(bottom - adjustment.page_size());
        }
    }
}

fn candidate_row(path: &Path, home: &Path) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::builder()
        .focusable(false)
        .selectable(true)
        .activatable(true)
        .tooltip_text(path.to_string_lossy())
        .build();
    row.add_css_class("path-completion-row");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    content.add_css_class("path-completion-row-content");
    let icon = crate::assets::primary_icon(crate::assets::icons::FOLDER, 16);
    icon.set_valign(gtk::Align::Center);
    content.append(&icon);
    let name = path
        .file_name()
        .map_or_else(|| path.to_string_lossy(), |name| name.to_string_lossy());
    let label = gtk::Label::new(Some(&name));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    content.append(&label);
    if let Some(parent) = path.parent() {
        let parent = gtk::Label::new(Some(&home_relative(parent, home)));
        parent.add_css_class("path-completion-parent");
        parent.set_xalign(1.0);
        parent.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        parent.set_max_width_chars(32);
        content.append(&parent);
    }
    row.set_child(Some(&content));
    row
}

fn home_relative(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~/{}", rest.to_string_lossy()),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

impl Drop for Candidates {
    fn drop(&mut self) {
        if self.popover.parent().is_some() {
            self.popover.unparent();
        }
    }
}
