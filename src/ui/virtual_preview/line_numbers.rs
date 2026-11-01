// SPDX-License-Identifier: MIT

//! Keep gutter layouts transient so prefetched rows retain no shaping caches.

use std::cell::{Cell, RefCell};

use gtk::{glib, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct LineNumbers {
        pub(super) text: RefCell<String>,
        pub(super) metrics: Cell<Option<(u32, i32, i32)>>,
    }

    impl LineNumbers {
        fn dimensions(&self) -> (i32, i32) {
            let widget = self.obj();
            // GTK updates this context when font, scale, or direction changes.
            let serial = widget.pango_context().serial();
            if let Some((cached_serial, width, height)) = self.metrics.get()
                && serial == cached_serial
            {
                return (width, height);
            }
            let layout = widget.create_pango_layout(Some(&self.text.borrow()));
            let (width, height) = layout.pixel_size();
            self.metrics.set(Some((serial, width, height)));
            (width, height)
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LineNumbers {
        const NAME: &'static str = "StrataPreviewLineNumbers";
        type Type = super::LineNumbers;
        type ParentType = gtk::Widget;

        fn class_init(class: &mut Self::Class) {
            class.set_accessible_role(gtk::AccessibleRole::Presentation);
        }
    }

    impl ObjectImpl for LineNumbers {}

    impl WidgetImpl for LineNumbers {
        fn measure(&self, orientation: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            let (width, height) = self.dimensions();
            let size = if orientation == gtk::Orientation::Horizontal {
                width
            } else {
                height
            };
            (size, size, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let layout = widget.create_pango_layout(Some(&self.text.borrow()));
            snapshot.append_layout(&layout, &widget.color());
        }
    }
}

glib::wrapper! {
    pub struct LineNumbers(ObjectSubclass<imp::LineNumbers>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl LineNumbers {
    pub(super) fn new(text: String) -> Self {
        let widget: Self = glib::Object::new();
        widget.imp().text.replace(text);
        widget.set_can_target(false);
        widget.add_css_class("preview-source-lines");
        widget.set_margin_start(2);
        widget.set_margin_end(6);
        widget
    }

    pub(super) fn set_text(&self, text: String) {
        if *self.imp().text.borrow() == text {
            return;
        }
        self.imp().text.replace(text);
        self.imp().metrics.set(None);
        self.queue_resize();
        self.queue_draw();
    }
}
