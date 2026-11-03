// SPDX-License-Identifier: MIT

use gtk::{glib, prelude::*, subclass::prelude::*};
use std::cell::Cell;

mod imp {
    use super::*;

    /// Holds one child and reports its minimum width, raised to `column_width`, as its
    /// natural width. A `gtk::FlowBox` chooses its column count from natural widths, so
    /// wrapping labels would otherwise leave room for one card per line, while a narrow
    /// window can still shrink the child to its own minimum.
    #[derive(Default)]
    pub struct CompactBin {
        pub column_width: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CompactBin {
        const NAME: &'static str = "StrataSettingsCompactBin";
        type Type = super::CompactBin;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for CompactBin {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for CompactBin {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            self.obj()
                .first_child()
                .map_or(gtk::SizeRequestMode::ConstantSize, |child| {
                    child.request_mode()
                })
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(child) = self.obj().first_child() else {
                return (0, 0, -1, -1);
            };
            let measured = child.measure(orientation, for_size);
            if orientation == gtk::Orientation::Horizontal {
                (measured.0, measured.0.max(self.column_width.get()), -1, -1)
            } else {
                measured
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.obj().first_child() {
                child.allocate(width, height, baseline, None);
            }
        }
    }
}

glib::wrapper! {
    pub struct CompactBin(ObjectSubclass<imp::CompactBin>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl CompactBin {
    pub(super) fn new(child: &impl IsA<gtk::Widget>, column_width: i32) -> Self {
        let bin: Self = glib::Object::new();
        bin.imp().column_width.set(column_width);
        child.set_parent(&bin);
        bin
    }
}
