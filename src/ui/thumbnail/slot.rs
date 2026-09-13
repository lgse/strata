// SPDX-License-Identifier: MIT

use std::cell::{Cell, RefCell};

use gtk::{gdk, gdk::prelude::*, glib, graphene, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ThumbnailSlot {
        pub slot: Cell<i32>,
        pub content_inset: Cell<i32>,
        pub limit_fallback_height: Cell<bool>,
        pub fallback_scale: Cell<f64>,
        pub texture: RefCell<Option<gdk::Texture>>,
        pub fallback: RefCell<Option<gdk::Texture>>,
        pub fallback_icon: RefCell<Option<String>>,
        pub cut: Cell<bool>,
        pub hidden: Cell<bool>,
        pub base_opacity: Cell<f64>,
        #[cfg(test)]
        pub resize_calls: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ThumbnailSlot {
        const NAME: &'static str = "StrataThumbnailSlot";
        type Type = super::ThumbnailSlot;
        type ParentType = gtk::Widget;

        fn class_init(class: &mut Self::Class) {
            class.set_accessible_role(gtk::AccessibleRole::Img);
        }
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
            let is_cut = self.cut.get();

            let texture = if is_cut {
                crate::assets::primary_icon_paintable(crate::assets::icons::SCISSORS)
                    .or_else(|| self.texture.borrow().clone())
                    .or_else(|| self.fallback.borrow().clone())
            } else {
                self.texture
                    .borrow()
                    .clone()
                    .or_else(|| self.fallback.borrow().clone())
            };

            let Some(texture) = texture else {
                return;
            };

            let scale = if self.texture.borrow().is_some() || is_cut {
                1.0
            } else {
                self.fallback_scale.get()
            };

            let inset = f64::from(self.content_inset.get())
                .min((width - 1.0) / 2.0)
                .min((height - 1.0) / 2.0);

            snapshot.save();
            let draw_width = (width - 2.0 * inset) * scale;
            let draw_height = (height - 2.0 * inset) * scale;
            snapshot.translate(&graphene::Point::new(
                ((width - draw_width) / 2.0) as f32,
                ((height - draw_height) / 2.0) as f32,
            ));
            snapshot_texture(snapshot, &texture, draw_width, draw_height);
            snapshot.restore();
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

fn folder_height_scale(texture: &gdk::Texture) -> f64 {
    let mut downloader = gdk::TextureDownloader::new(texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (pixels, stride) = downloader.download_bytes();
    let width = texture.width() as usize;
    let visible = |row: usize| (0..width).any(|column| pixels[row * stride + column * 4 + 3] > 0);
    let height = texture.height() as usize;
    let Some(first) = (0..height).find(|row| visible(*row)) else {
        return 1.0;
    };
    let last = (first..height).rfind(|row| visible(*row)).unwrap_or(first);
    // Lucide's folder ink spans y=2..21, including its stroke, in a 24-unit viewBox.
    (19.0 / 24.0 * height as f64 / (last - first + 1) as f64).min(1.0)
}

fn same_texture(left: Option<&gdk::Texture>, right: Option<&gdk::Texture>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => std::ptr::eq(left.as_ptr(), right.as_ptr()),
        _ => false,
    }
}

#[cfg(test)]
mod tests;

glib::wrapper! {
    pub struct ThumbnailSlot(ObjectSubclass<imp::ThumbnailSlot>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ThumbnailSlot {
    pub(crate) fn new(slot: i32) -> Self {
        let widget: Self = glib::Object::new();
        widget.set_overflow(gtk::Overflow::Hidden);
        widget.imp().fallback_scale.set(1.0);
        widget.imp().base_opacity.set(1.0);
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

    pub(crate) fn set_content_inset(&self, inset: i32) {
        let inset = inset.max(0);
        if self.imp().content_inset.replace(inset) != inset {
            self.queue_draw();
        }
    }

    pub(crate) fn limit_fallback_height_to_folder(&self) {
        self.imp().limit_fallback_height.set(true);
    }

    pub(crate) fn set_texture(&self, texture: &gdk::Texture) {
        if same_texture(self.imp().texture.borrow().as_ref(), Some(texture)) {
            return;
        }
        self.imp().texture.replace(Some(texture.clone()));
        self.update_state_opacity();
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
        let scale =
            if self.imp().limit_fallback_height.get() && icon != crate::assets::icons::FOLDER {
                texture.map_or(1.0, folder_height_scale)
            } else {
                1.0
            };
        self.imp().fallback_scale.set(scale);
        self.imp().fallback_icon.replace(Some(icon.to_owned()));
        self.imp().fallback.replace(texture.cloned());
        self.update_state_opacity();
        self.queue_draw();
    }

    pub(crate) fn texture(&self) -> Option<gdk::Texture> {
        self.imp().texture.borrow().clone()
    }

    pub(crate) fn set_cut(&self, cut: bool) {
        if self.imp().cut.replace(cut) != cut {
            self.update_state_opacity();
            self.queue_draw();
        }
    }

    pub(crate) fn set_hidden(&self, hidden: bool) {
        if self.imp().hidden.replace(hidden) != hidden {
            self.update_state_opacity();
        }
    }

    pub(crate) fn set_base_opacity(&self, opacity: f64) {
        let opacity = opacity.clamp(0.0, 1.0);
        let current = self.imp().base_opacity.get();
        if (current - opacity).abs() > 1e-4 {
            self.imp().base_opacity.set(opacity);
            self.update_state_opacity();
        }
    }

    fn update_state_opacity(&self) {
        let opacity = if self.imp().hidden.get() {
            0.65
        } else if self.imp().cut.get() || self.imp().texture.borrow().is_some() {
            1.0
        } else {
            self.imp().base_opacity.get()
        };
        self.set_opacity(opacity);
    }

    #[cfg(test)]
    pub(crate) fn resize_calls(&self) -> u32 {
        self.imp().resize_calls.get()
    }
}
