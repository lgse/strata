// SPDX-License-Identifier: MIT

use std::cell::{Cell, RefCell};

use gio::subclass::prelude::ListModelImpl;
use gtk::{gio, glib, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct CountingModel {
        pub source: RefCell<Option<gio::ListModel>>,
        pub reads: Cell<usize>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CountingModel {
        const NAME: &'static str = "StrataCountingIndexModel";
        type Type = super::CountingModel;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for CountingModel {}

    impl ListModelImpl for CountingModel {
        fn item_type(&self) -> glib::Type {
            gtk::StringObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.source
                .borrow()
                .as_ref()
                .map_or(0, |source| source.n_items())
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.reads.set(self.reads.get() + 1);
            self.source.borrow().as_ref()?.item(position)
        }
    }
}

glib::wrapper! {
    pub struct CountingModel(ObjectSubclass<imp::CountingModel>)
        @implements gio::ListModel;
}

impl CountingModel {
    pub(super) fn new(source: &impl IsA<gio::ListModel>) -> Self {
        let model: Self = glib::Object::new();
        model.imp().source.replace(Some(source.as_ref().clone()));
        let weak = model.downgrade();
        source.connect_items_changed(move |_, position, removed, added| {
            if let Some(model) = weak.upgrade() {
                model.items_changed(position, removed, added);
            }
        });
        model
    }

    pub(super) fn reads(&self) -> usize {
        self.imp().reads.get()
    }
}
