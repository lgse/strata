// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::prelude::*;

use crate::{
    assets::{icons, primary_icon, set_primary_icon},
    services::{ArchiveDirectory, ArchiveNode, ArchivePreviewTree},
    ui::browser::icon_for_name,
};

use super::format_file_size;

const ROOT_LABEL: &str = "Contents";

fn directory_at<'a>(root: &'a ArchiveDirectory, path: &[usize]) -> &'a ArchiveDirectory {
    let mut directory = root;
    for &index in path {
        match directory.children.get(index) {
            Some(ArchiveNode::Directory(child)) => directory = child,
            _ => break,
        }
    }
    directory
}

fn child_summary(directory: &ArchiveDirectory) -> (usize, usize) {
    directory
        .children
        .iter()
        .fold((0, 0), |(files, folders), node| match node {
            ArchiveNode::Directory(_) => (files, folders + 1),
            ArchiveNode::File { .. } => (files + 1, folders),
        })
}

fn crumb_name(directory: &ArchiveDirectory) -> &str {
    if directory.name.is_empty() {
        ROOT_LABEL
    } else {
        &directory.name
    }
}

// Eager widget construction stalls large archives; keep rows virtualized.
pub(super) struct ArchiveBrowser {
    tree: Rc<ArchivePreviewTree>,
    path: Rc<RefCell<Vec<usize>>>,
    navigate: Rc<dyn Fn(usize)>,
    model: gtk::gio::ListStore,
    selection: gtk::SingleSelection,
    cursor: Cell<usize>,
    root: gtk::Box,
    crumbs: gtk::Box,
    count: gtk::Label,
    list: gtk::ListView,
    empty: gtk::Label,
}

impl ArchiveBrowser {
    /// `navigate` receives an absolute depth, with zero denoting the archive root.
    pub(super) fn new(tree: ArchivePreviewTree, navigate: Rc<dyn Fn(usize)>) -> Self {
        let tree = Rc::new(tree);
        let path = Rc::new(RefCell::new(Vec::new()));
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("preview-archive");

        let crumbs = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        crumbs.add_css_class("preview-archive-crumbs");
        let crumb_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .overlay_scrolling(false)
            .child(&crumbs)
            .build();
        crumb_scroll.add_css_class("preview-archive-crumbs-scroll");
        crumb_scroll.add_css_class("fixed-scrollbar");
        root.append(&crumb_scroll);

        let count = gtk::Label::new(None);
        count.add_css_class("preview-archive-count");
        count.set_xalign(0.0);
        count.set_ellipsize(gtk::pango::EllipsizeMode::End);
        root.append(&count);

        let model = gtk::gio::ListStore::new::<gtk::StringObject>();
        let selection = gtk::SingleSelection::new(Some(model.clone()));
        selection.set_autoselect(false);
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let row_content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row_content.add_css_class("preview-archive-row");
            let icon = primary_icon(icons::DOCUMENTS, 18);
            row_content.append(&icon);
            let name_label = gtk::Label::new(None);
            name_label.add_css_class("preview-archive-name");
            name_label.set_hexpand(true);
            name_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            name_label.set_xalign(0.0);
            row_content.append(&name_label);
            let size_label = gtk::Label::new(None);
            size_label.add_css_class("preview-archive-size");
            row_content.append(&size_label);
            let chevron = primary_icon(icons::CHEVRON_RIGHT, 14);
            row_content.append(&chevron);
            item.set_child(Some(&row_content));
        });
        let tree_for_bind = tree.clone();
        let path_for_bind = path.clone();
        factory.connect_bind(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let position = item.position() as usize;
            let path = path_for_bind.borrow();
            let directory = directory_at(&tree_for_bind.root, &path);
            let Some(node) = directory.children.get(position) else {
                return;
            };
            let Some(row_content) = item.child().and_downcast::<gtk::Box>() else {
                return;
            };
            let mut children = row_content.first_child();
            let icon = children.clone().and_downcast::<gtk::Image>();
            children = children.and_then(|child| child.next_sibling());
            let name_label = children.clone().and_downcast::<gtk::Label>();
            children = children.and_then(|child| child.next_sibling());
            let size_label = children.clone().and_downcast::<gtk::Label>();
            children = children.and_then(|child| child.next_sibling());
            let chevron = children.and_downcast::<gtk::Image>();
            let (Some(icon), Some(name_label), Some(size_label), Some(chevron)) =
                (icon, name_label, size_label, chevron)
            else {
                return;
            };
            match node {
                ArchiveNode::Directory(child) => {
                    set_primary_icon(&icon, icons::FOLDER);
                    // Archives may contain a file and a directory with the same name.
                    name_label.set_text(&format!("{}/", crumb_name(child)));
                    size_label.set_visible(false);
                    chevron.set_visible(true);
                    item.set_activatable(true);
                }
                ArchiveNode::File { name, size } => {
                    set_primary_icon(&icon, icon_for_name(name));
                    name_label.set_text(name);
                    size_label.set_text(&format_file_size(*size));
                    size_label.set_visible(true);
                    chevron.set_visible(false);
                    item.set_activatable(false);
                }
            }
        });
        let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
        list.add_css_class("preview-archive-list");
        list.set_single_click_activate(true);
        let list_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .overlay_scrolling(false)
            .child(&list)
            .vexpand(true)
            .build();
        list_scroll.add_css_class("fixed-scrollbar");
        root.append(&list_scroll);

        let empty = gtk::Label::new(Some("This folder is empty"));
        empty.add_css_class("preview-archive-empty");
        empty.set_halign(gtk::Align::Center);
        empty.set_valign(gtk::Align::Center);
        empty.set_vexpand(true);
        empty.set_hexpand(true);
        root.append(&empty);

        let browser = Self {
            tree,
            path,
            navigate,
            model,
            selection,
            cursor: Cell::new(0),
            root,
            crumbs,
            count,
            list,
            empty,
        };
        browser.refresh();
        browser
    }

    pub(super) fn root(&self) -> &gtk::Box {
        &self.root
    }

    pub(super) fn list(&self) -> &gtk::ListView {
        &self.list
    }

    #[cfg(test)]
    pub(super) fn cursor_index(&self) -> usize {
        self.cursor.get()
    }

    #[cfg(test)]
    pub(super) fn selected_index(&self) -> Option<usize> {
        let selected = self.selection.selected();
        (selected != gtk::INVALID_LIST_POSITION).then_some(selected as usize)
    }

    pub(super) fn move_cursor(&self, delta: isize) -> bool {
        let count = self.model.n_items() as usize;
        if count == 0 {
            return false;
        }
        self.sync_cursor_to_selection();
        let current = self.cursor.get().min(count - 1);
        let next = (current as isize + delta).clamp(0, count as isize - 1) as usize;
        if next == current {
            return false;
        }
        self.cursor.set(next);
        self.apply_selection();
        true
    }

    pub(super) fn open_cursor(&mut self) -> bool {
        self.sync_cursor_to_selection();
        let directory = directory_at(&self.tree.root, &self.path.borrow());
        if !matches!(
            directory.children.get(self.cursor.get()),
            Some(ArchiveNode::Directory(_))
        ) {
            return false;
        }
        self.open_child(self.cursor.get());
        true
    }

    pub(super) fn go_up(&mut self) -> bool {
        let depth = self.path.borrow().len();
        if depth == 0 {
            return false;
        }
        self.navigate_to(depth - 1);
        true
    }

    pub(super) fn open_child(&mut self, index: usize) {
        if matches!(
            directory_at(&self.tree.root, &self.path.borrow())
                .children
                .get(index),
            Some(ArchiveNode::Directory(_))
        ) {
            self.path.borrow_mut().push(index);
            self.cursor.set(0);
            self.refresh();
        } else {
            self.sync_cursor_to_selection();
        }
    }

    pub(super) fn navigate_to(&mut self, depth: usize) {
        let current = self.path.borrow().clone();
        let depth = depth.min(current.len());
        self.cursor.set(if depth < current.len() {
            current[depth]
        } else {
            0
        });
        self.path.borrow_mut().truncate(depth);
        self.refresh();
    }

    fn refresh(&self) {
        self.rebuild_crumbs();
        let directory = directory_at(&self.tree.root, &self.path.borrow());
        let count = directory.children.len();
        if count == 0 {
            self.cursor.set(0);
        } else if self.cursor.get() >= count {
            self.cursor.set(count - 1);
        }
        self.rebuild_rows();
        let (files, folders) = child_summary(directory);
        let summary = match (files, folders) {
            (0, 0) => "Empty folder".to_owned(),
            (1, 0) => "1 file".to_owned(),
            (files, 0) => format!("{files} files"),
            (0, 1) => "1 folder".to_owned(),
            (0, folders) => format!("{folders} folders"),
            _ => format!("{files} files, {folders} folders"),
        };
        self.count.set_text(&summary);
    }

    fn rebuild_crumbs(&self) {
        clear_box(&self.crumbs);
        let depth = self.path.borrow().len();
        if depth > 0 {
            let back = gtk::Button::new();
            let back_icon = primary_icon(icons::ARROW_LEFT, 14);
            back.set_child(Some(&back_icon));
            back.set_tooltip_text(Some("Go up one level"));
            back.add_css_class("preview-archive-crumb");
            let navigate = self.navigate.clone();
            back.connect_clicked(move |_| navigate(depth - 1));
            self.crumbs.append(&back);
        }

        let mut current = &self.tree.root;
        let mut descending = Vec::new();
        for &index in self.path.borrow().iter() {
            if let Some(ArchiveNode::Directory(child)) = current.children.get(index) {
                descending.push(child);
                current = child;
            } else {
                break;
            }
        }
        let mut labels = Vec::with_capacity(descending.len() + 1);
        labels.push(ROOT_LABEL.to_owned());
        labels.extend(descending.iter().map(|child| child.name.clone()));
        for (position, label) in labels.iter().enumerate() {
            if position == depth {
                let current = gtk::Label::new(Some(label));
                current.add_css_class("preview-archive-crumb");
                current.add_css_class("preview-archive-crumb-current");
                self.crumbs.append(&current);
            } else {
                let button = gtk::Button::with_label(label);
                button.add_css_class("preview-archive-crumb");
                let navigate = self.navigate.clone();
                button.connect_clicked(move |_| navigate(position));
                self.crumbs.append(&button);
            }
        }
    }

    fn rebuild_rows(&self) {
        let directory = directory_at(&self.tree.root, &self.path.borrow());
        let items: Vec<gtk::StringObject> = (0..directory.children.len())
            .map(|index| gtk::StringObject::new(&index.to_string()))
            .collect();
        self.model.splice(0, self.model.n_items(), &items);
        self.empty.set_visible(directory.children.is_empty());
        self.apply_selection();
    }

    fn apply_selection(&self) {
        let count = self.model.n_items();
        if count == 0 {
            self.selection.set_selected(gtk::INVALID_LIST_POSITION);
            return;
        }
        let index = self.cursor.get().min(count as usize - 1) as u32;
        self.selection.set_selected(index);
        // Scrolling without FOCUS keeps the drawer's focus in the browser list.
        self.list.scroll_to(index, gtk::ListScrollFlags::NONE, None);
    }

    fn sync_cursor_to_selection(&self) {
        let selected = self.selection.selected();
        if selected == gtk::INVALID_LIST_POSITION {
            return;
        }
        let count = self.model.n_items() as usize;
        if count > 0 {
            self.cursor.set((selected as usize).min(count - 1));
        }
    }
}

fn clear_box(box_: &gtk::Box) {
    while let Some(child) = box_.first_child() {
        box_.remove(&child);
    }
}

#[cfg(test)]
mod tests;
