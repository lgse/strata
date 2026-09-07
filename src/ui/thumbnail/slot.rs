// SPDX-License-Identifier: GPL-3.0-or-later

use std::cell::{Cell, RefCell};

use gtk::{gdk, gdk::prelude::*, glib, graphene, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ThumbnailSlot {
        pub slot: Cell<i32>,
        pub texture: RefCell<Option<gdk::Texture>>,
        pub fallback: RefCell<Option<gdk::Texture>>,
        pub fallback_icon: RefCell<Option<String>>,
        #[cfg(test)]
        pub resize_calls: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ThumbnailSlot {
        const NAME: &'static str = "StrataThumbnailSlot";
        type Type = super::ThumbnailSlot;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ThumbnailSlot {}

    impl WidgetImpl for ThumbnailSlot {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let size = self.slot.get().max(1);
            (size, size, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let width = f64::from(obj.width());
            let height = f64::from(obj.height());
            if width <= 0.0 || height <= 0.0 {
                return;
            }
            let texture = self
                .texture
                .borrow()
                .clone()
                .or_else(|| self.fallback.borrow().clone());
            let Some(texture) = texture else {
                return;
            };
            snapshot_texture(snapshot, &texture, width, height);
        }
    }
}

fn snapshot_texture(snapshot: &gtk::Snapshot, texture: &gdk::Texture, width: f64, height: f64) {
    let intrinsic_w = f64::from(texture.width().max(0));
    let intrinsic_h = f64::from(texture.height().max(0));
    let (draw_w, draw_h) = if intrinsic_w > 0.0 && intrinsic_h > 0.0 {
        let scale = (width / intrinsic_w).min(height / intrinsic_h);
        (intrinsic_w * scale, intrinsic_h * scale)
    } else {
        (width, height)
    };
    let x = ((width - draw_w) / 2.0) as f32;
    let y = ((height - draw_h) / 2.0) as f32;
    snapshot.append_texture(
        texture,
        &graphene::Rect::new(x, y, draw_w as f32, draw_h as f32),
    );
}

fn same_texture(left: Option<&gdk::Texture>, right: Option<&gdk::Texture>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => std::ptr::eq(left.as_ptr(), right.as_ptr()),
        _ => false,
    }
}

glib::wrapper! {
    pub struct ThumbnailSlot(ObjectSubclass<imp::ThumbnailSlot>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ThumbnailSlot {
    pub(crate) fn new(slot: i32) -> Self {
        let widget: Self = glib::Object::new();
        widget.set_overflow(gtk::Overflow::Hidden);
        widget.set_slot(slot);
        widget
    }

    pub(crate) fn slot_size(&self) -> i32 {
        self.imp().slot.get().max(1)
    }

    pub(crate) fn set_slot(&self, size: i32) {
        let size = size.max(1);
        if self.imp().slot.get() == size {
            return;
        }
        self.imp().slot.set(size);
        #[cfg(test)]
        self.imp()
            .resize_calls
            .set(self.imp().resize_calls.get() + 1);
        self.queue_resize();
    }

    pub(crate) fn set_texture(&self, texture: &gdk::Texture) {
        if same_texture(self.imp().texture.borrow().as_ref(), Some(texture)) {
            return;
        }
        self.imp().texture.replace(Some(texture.clone()));
        self.queue_draw();
    }

    pub(crate) fn set_fallback(&self, icon: &str, texture: Option<&gdk::Texture>) {
        if self.imp().texture.borrow().is_none()
            && self.imp().fallback_icon.borrow().as_deref() == Some(icon)
            && same_texture(self.imp().fallback.borrow().as_ref(), texture)
        {
            return;
        }
        self.imp().texture.replace(None);
        self.imp().fallback_icon.replace(Some(icon.to_owned()));
        self.imp().fallback.replace(texture.cloned());
        self.queue_draw();
    }

    pub(crate) fn texture(&self) -> Option<gdk::Texture> {
        self.imp().texture.borrow().clone()
    }

    #[cfg(test)]
    pub(crate) fn resize_calls(&self) -> u32 {
        self.imp().resize_calls.get()
    }
}
