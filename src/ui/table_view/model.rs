// SPDX-License-Identifier: MIT

use std::cell::{OnceCell, Ref, RefCell};

use gio::subclass::prelude::ListModelImpl;
use gtk::{gio, glib, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct RowModel {
        pub order: RefCell<Vec<usize>>,
        pub objects: OnceCell<Vec<OnceCell<glib::BoxedAnyObject>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RowModel {
        const NAME: &'static str = "StrataTableRowModel";
        type Type = super::RowModel;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for RowModel {}

    impl ListModelImpl for RowModel {
        fn item_type(&self) -> glib::Type {
            glib::BoxedAnyObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.order.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            let row = *self.order.borrow().get(position as usize)?;
            // Cache by source row: reorders retain identity without creating unseen rows.
            self.objects.get()?.get(row).map(|object| {
                object
                    .get_or_init(|| glib::BoxedAnyObject::new(row))
                    .clone()
                    .upcast()
            })
        }
    }
}

glib::wrapper! {
    pub struct RowModel(ObjectSubclass<imp::RowModel>)
        @implements gio::ListModel;
}

impl RowModel {
    pub(super) fn new(rows: usize) -> Self {
        let model: Self = glib::Object::new();
        model
            .imp()
            .objects
            .set((0..rows).map(|_| OnceCell::new()).collect())
            .expect("row objects initialized once");
        model
    }

    pub(super) fn row(&self, position: u32) -> Option<usize> {
        self.imp().order.borrow().get(position as usize).copied()
    }

    pub(super) fn order(&self) -> Ref<'_, Vec<usize>> {
        self.imp().order.borrow()
    }

    pub(super) fn set_order(&self, order: Vec<usize>) {
        let previous = self.order();
        let prefix = previous
            .iter()
            .zip(&order)
            .take_while(|(a, b)| a == b)
            .count();
        if prefix == previous.len() && prefix == order.len() {
            return;
        }
        let suffix = previous[prefix..]
            .iter()
            .rev()
            .zip(order[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let removed = previous.len() - prefix - suffix;
        let added = order.len() - prefix - suffix;
        drop(previous);
        self.imp().order.replace(order);
        self.items_changed(prefix as u32, removed as u32, added as u32);
    }
}
