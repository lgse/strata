// SPDX-License-Identifier: MIT

use std::cell::{Cell, RefCell};

use gtk::{gdk, glib, graphene, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct RotatedPaintable {
        pub inner: RefCell<Option<gdk::Paintable>>,
        pub quarter_turns_cw: Cell<u8>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RotatedPaintable {
        const NAME: &'static str = "StrataRotatedPaintable";
        type Type = super::RotatedPaintable;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for RotatedPaintable {}

    impl PaintableImpl for RotatedPaintable {
        fn intrinsic_width(&self) -> i32 {
            let inner = self.inner.borrow();
            let Some(inner) = inner.as_ref() else {
                return 0;
            };
            if self.quarter_turns_cw.get() % 2 == 1 {
                inner.intrinsic_height()
            } else {
                inner.intrinsic_width()
            }
        }

        fn intrinsic_height(&self) -> i32 {
            let inner = self.inner.borrow();
            let Some(inner) = inner.as_ref() else {
                return 0;
            };
            if self.quarter_turns_cw.get() % 2 == 1 {
                inner.intrinsic_width()
            } else {
                inner.intrinsic_height()
            }
        }

        fn intrinsic_aspect_ratio(&self) -> f64 {
            let (width, height) = (self.intrinsic_width(), self.intrinsic_height());
            if width <= 0 || height <= 0 {
                0.0
            } else {
                f64::from(width) / f64::from(height)
            }
        }

        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            let inner = self.inner.borrow();
            let Some(inner) = inner.as_ref() else {
                return;
            };
            let (width_f32, height_f32) = (width as f32, height as f32);
            match self.quarter_turns_cw.get() % 4 {
                0 => inner.snapshot(snapshot, width, height),
                1 => {
                    snapshot.save();
                    snapshot.translate(&graphene::Point::new(width_f32, 0.0));
                    snapshot.rotate(90.0);
                    inner.snapshot(snapshot, height, width);
                    snapshot.restore();
                }
                2 => {
                    snapshot.save();
                    snapshot.translate(&graphene::Point::new(width_f32, height_f32));
                    snapshot.rotate(180.0);
                    inner.snapshot(snapshot, width, height);
                    snapshot.restore();
                }
                _ => {
                    snapshot.save();
                    snapshot.translate(&graphene::Point::new(0.0, height_f32));
                    snapshot.rotate(270.0);
                    inner.snapshot(snapshot, height, width);
                    snapshot.restore();
                }
            }
        }
    }
}

glib::wrapper! {
    pub struct RotatedPaintable(ObjectSubclass<imp::RotatedPaintable>) @implements gdk::Paintable;
}

impl RotatedPaintable {
    pub fn new(inner: &impl IsA<gdk::Paintable>) -> Self {
        let paintable: Self = glib::Object::new();
        paintable.imp().inner.replace(Some(inner.as_ref().clone()));
        paintable
    }

    pub fn rotate_cw(&self) {
        self.set_quarter_turns(self.imp().quarter_turns_cw.get().wrapping_add(1));
    }

    pub fn rotate_ccw(&self) {
        self.set_quarter_turns(self.imp().quarter_turns_cw.get().wrapping_add(3));
    }

    fn set_quarter_turns(&self, quarter_turns: u8) {
        self.imp().quarter_turns_cw.set(quarter_turns % 4);
        self.invalidate_size();
        self.invalidate_contents();
    }

    #[cfg(test)]
    fn quarter_turns(&self) -> u8 {
        self.imp().quarter_turns_cw.get()
    }
}

#[cfg(test)]
mod tests;
