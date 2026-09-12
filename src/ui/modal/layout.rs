// SPDX-License-Identifier: MIT

use gtk::{glib, graphene, gsk, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ViewportLayout;

    #[glib::object_subclass]
    impl ObjectSubclass for ViewportLayout {
        const NAME: &'static str = "StrataModalViewportLayout";
        type Type = super::ViewportLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for ViewportLayout {}

    impl LayoutManagerImpl for ViewportLayout {
        fn measure(
            &self,
            _widget: &gtk::Widget,
            _orientation: gtk::Orientation,
            _for_size: i32,
        ) -> (i32, i32, i32, i32) {
            // An overlay must not enlarge the host window to fit a dialog.
            (1, 1, -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, height: i32, baseline: i32) {
            let Some(scroll) = widget.first_child().and_downcast::<gtk::ScrolledWindow>() else {
                return;
            };
            let Some(viewport) = scroll.child() else {
                return;
            };
            let available_width = (width - 24).max(1);
            let available_height = (height - 24).max(1);
            let child_width = viewport
                .measure(gtk::Orientation::Horizontal, -1)
                .1
                .min(available_width)
                .max(1);
            let child_height = viewport
                .measure(gtk::Orientation::Vertical, child_width)
                .1
                .min(available_height)
                .max(1);
            scroll.allocate(
                child_width,
                child_height,
                baseline,
                Some(gsk::Transform::new().translate(&graphene::Point::new(
                    ((width - child_width) / 2) as f32,
                    ((height - child_height) / 2) as f32,
                ))),
            );
        }
    }
}

glib::wrapper! {
    pub struct ViewportLayout(ObjectSubclass<imp::ViewportLayout>) @extends gtk::LayoutManager;
}

pub(in crate::ui) fn install(
    layer: &gtk::Box,
    content: &impl IsA<gtk::Widget>,
) -> gtk::ScrolledWindow {
    // ScrolledWindow clips its child, including CSS shadows. Reserve room inside
    // the clip for the largest dialog shadow (the settings panel's 28px blur).
    let shadow_space = gtk::Box::new(gtk::Orientation::Vertical, 0);
    shadow_space.set_margin_top(42);
    shadow_space.set_margin_bottom(42);
    shadow_space.set_margin_start(42);
    shadow_space.set_margin_end(42);
    shadow_space.append(content);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&shadow_space)
        .build();
    scroll.add_css_class("modal-viewport");
    layer.append(&scroll);
    layer.set_layout_manager(Some(glib::Object::new::<ViewportLayout>()));
    scroll
}
