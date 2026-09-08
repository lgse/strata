// SPDX-License-Identifier: MIT

use std::cell::Cell;

use gtk::{glib, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct BlurBin {
        pub blurred: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BlurBin {
        const NAME: &'static str = "StrataBlurBin";
        type Type = super::BlurBin;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for BlurBin {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for BlurBin {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            self.obj()
                .first_child()
                .map(|child| child.measure(orientation, for_size))
                .unwrap_or((0, 0, -1, -1))
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.obj().first_child() {
                child.allocate(width, height, baseline, None);
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(child) = self.obj().first_child() else {
                return;
            };
            if self.blurred.get() {
                snapshot.push_blur(3.5);
            }
            self.obj().snapshot_child(&child, snapshot);
            if self.blurred.get() {
                snapshot.pop();
            }
        }
    }
}

glib::wrapper! {
    pub struct BlurBin(ObjectSubclass<imp::BlurBin>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl BlurBin {
    pub(super) fn new(child: &impl IsA<gtk::Widget>) -> Self {
        let blur: Self = glib::Object::new();
        child.set_parent(&blur);
        blur
    }

    pub(super) fn set_blurred(&self, blurred: bool) {
        if self.imp().blurred.replace(blurred) != blurred {
            self.queue_draw();
        }
    }
}
