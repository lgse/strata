// SPDX-License-Identifier: GPL-3.0-or-later

//! The item type the tree view uses for rows below its root level. Those entries are
//! loaded by the view itself, so unlike root rows they cannot be resolved back to a
//! position in the browser's active column.

use std::cell::RefCell;

use gtk::{glib, prelude::*, subclass::prelude::*};

use crate::model::FileEntry;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct TreeEntry {
        pub entry: RefCell<Option<FileEntry>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TreeEntry {
        const NAME: &'static str = "StrataTreeEntry";
        type Type = super::TreeEntry;
    }

    impl ObjectImpl for TreeEntry {}
}

glib::wrapper! {
    pub struct TreeEntry(ObjectSubclass<imp::TreeEntry>);
}

impl TreeEntry {
    pub(crate) fn new(entry: FileEntry) -> Self {
        let object: Self = glib::Object::new();
        object.imp().entry.replace(Some(entry));
        object
    }

    pub(crate) fn entry(&self) -> Option<FileEntry> {
        self.imp().entry.borrow().clone()
    }
}

/// The entry a tree row stands for, whichever level it sits at. Root rows are plain
/// model values owned by the pane's source list; deeper rows carry their own entry.
pub(crate) fn row_entry(
    item: &glib::Object,
    resolve_root: impl FnOnce(&gtk::StringObject) -> Option<FileEntry>,
) -> Option<FileEntry> {
    if let Some(value) = item.downcast_ref::<gtk::StringObject>() {
        return resolve_root(value);
    }
    item.downcast_ref::<TreeEntry>().and_then(TreeEntry::entry)
}
