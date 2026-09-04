// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gio::subclass::prelude::ListModelImpl;
use gtk::{gio, glib, prelude::*, subclass::prelude::*};

pub(crate) type ResolveFn = Rc<dyn Fn(u32) -> Option<String>>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct EntryListModel {
        pub count: Cell<u32>,
        pub resolve: RefCell<Option<ResolveFn>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for EntryListModel {
        const NAME: &'static str = "StrataEntryListModel";
        type Type = super::EntryListModel;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for EntryListModel {}

    impl ListModelImpl for EntryListModel {
        fn item_type(&self) -> glib::Type {
            gtk::StringObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.count.get()
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            if position >= self.count.get() {
                return None;
            }
            let resolve = self.resolve.borrow().clone()?;
            resolve(position).map(|text| gtk::StringObject::new(&text).upcast())
        }
    }
}

glib::wrapper! {
    pub struct EntryListModel(ObjectSubclass<imp::EntryListModel>)
        @implements gio::ListModel;
}

impl EntryListModel {
    pub(crate) fn new(resolve: ResolveFn) -> Self {
        let model: Self = glib::Object::new();
        model.imp().resolve.replace(Some(resolve));
        model
    }

    pub(crate) fn value(&self, position: u32) -> Option<String> {
        if position >= self.imp().count.get() {
            return None;
        }
        let resolve = self.imp().resolve.borrow().clone()?;
        resolve(position)
    }

    pub(crate) fn replace(&self, count: u32) {
        let removed = self.imp().count.replace(count);
        // The authoritative entries may have been reordered or replaced
        // without changing their count, so every non-empty replacement must
        // invalidate the corresponding GTK items.
        if removed > 0 || count > 0 {
            self.items_changed(0, removed, count);
        }
    }

    pub(crate) fn splice(&self, position: u32, removed: u32, added: u32) {
        let count = self.imp().count.get();
        let position = position.min(count);
        let removed = removed.min(count - position);
        self.imp()
            .count
            .set((count - removed).saturating_add(added));
        if removed > 0 || added > 0 {
            self.items_changed(position, removed, added);
        }
    }
}

#[cfg(test)]
mod tests;
