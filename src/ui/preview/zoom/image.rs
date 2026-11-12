// SPDX-License-Identifier: MIT

//! A fitted image becomes scrollable when magnified, without losing its aspect ratio.

use gtk::{glib, prelude::*, subclass::prelude::*};
use std::cell::{Cell, RefCell};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ZoomImage {
        pub picture: RefCell<Option<gtk::Picture>>,
        pub intrinsic: Cell<(i32, i32)>,
        pub viewport: Cell<(i32, i32)>,
        pub zoom: Cell<f64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ZoomImage {
        const NAME: &'static str = "StrataPreviewZoomImage";
        type Type = super::ZoomImage;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ZoomImage {
        fn dispose(&self) {
            if let Some(picture) = self.picture.borrow_mut().take() {
                picture.unparent();
            }
        }
    }

    impl WidgetImpl for ZoomImage {
        fn measure(&self, orientation: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            let (width, height) = self.viewport.get();
            let (iw, ih) = self.intrinsic.get();
            let (width, height) =
                super::super::super::media_layout::fitted_size(width, height, iw, ih);
            let dimension = if orientation == gtk::Orientation::Horizontal {
                width
            } else {
                height
            };
            let size = (f64::from(dimension) * self.zoom.get()).round().max(1.0) as i32;
            (size, size, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(picture) = self.picture.borrow().as_ref() {
                picture.allocate(width, height, baseline, None);
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if let Some(picture) = self.picture.borrow().as_ref() {
                self.obj().snapshot_child(picture, snapshot);
            }
        }
    }
}

glib::wrapper! {
    pub struct ZoomImage(ObjectSubclass<imp::ZoomImage>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ZoomImage {
    pub(super) fn set_zoom(&self, zoom: f64) {
        self.imp().zoom.set(zoom);
        self.queue_resize();
    }
}

fn sync_viewport(scroll: &gtk::ScrolledWindow, image: &ZoomImage) -> bool {
    let width = scroll.width();
    let height = scroll.height();
    let viewport = (
        width
            .min(super::super::media_layout::MAX_CONTENT_WIDTH)
            .saturating_sub(24)
            .max(1),
        height.saturating_sub(24).max(1),
    );
    if image.imp().viewport.replace(viewport) != viewport {
        image.queue_resize();
    }
    width > 0 && height > 0
}

pub(super) fn view(
    picture: &gtk::Picture,
    texture: &gtk::gdk::Texture,
) -> (gtk::ScrolledWindow, ZoomImage) {
    let image: ZoomImage = glib::Object::new();
    image
        .imp()
        .intrinsic
        .set((texture.width(), texture.height()));
    image.imp().zoom.set(1.0);
    image.imp().picture.replace(Some(picture.clone()));
    image.set_halign(gtk::Align::Center);
    image.set_valign(gtk::Align::Center);
    image.set_margin_start(12);
    image.set_margin_end(12);
    image.set_margin_top(12);
    image.set_margin_bottom(12);
    picture.set_parent(&image);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&image)
        .hexpand(true)
        .vexpand(true)
        .build();
    for adjustment in [scroll.hadjustment(), scroll.vadjustment()] {
        let weak_scroll = scroll.downgrade();
        let weak_image = image.downgrade();
        adjustment.connect_page_size_notify(move |_| {
            if let (Some(scroll), Some(image)) = (weak_scroll.upgrade(), weak_image.upgrade()) {
                sync_viewport(&scroll, &image);
            }
        });
    }
    let weak = image.downgrade();
    scroll.add_tick_callback(move |scroll, _| {
        let Some(image) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if sync_viewport(scroll, &image) {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    (scroll, image)
}
