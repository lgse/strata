// SPDX-License-Identifier: MIT

use gtk::{glib, prelude::*, subclass::prelude::*};
use std::cell::Cell;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ViewportLayout {
        original_width_request: Cell<Option<i32>>,
        unconstrained_minimum: Cell<i32>,
    }

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
            let Some(scroll_child) = scroll.child() else {
                return;
            };
            let shadow_space = scroll_child
                .clone()
                .downcast::<gtk::Box>()
                .ok()
                .or_else(|| scroll_child.first_child().and_downcast::<gtk::Box>());
            let Some(shadow_space) = shadow_space else {
                return;
            };
            let Some(content) = shadow_space.first_child() else {
                return;
            };
            let original_width = if let Some(width) = self.original_width_request.get() {
                width
            } else {
                let width = content.width_request();
                self.original_width_request.set(Some(width));
                width
            };
            let was_constrained = content.has_css_class("modal-constrained");
            let constrained = if was_constrained {
                self.unconstrained_minimum.get() + 84 > width
            } else {
                content.set_width_request(original_width);
                content.set_halign(gtk::Align::Center);
                let (minimum, _, _, _) = content.measure(gtk::Orientation::Horizontal, -1);
                self.unconstrained_minimum.set(minimum);
                minimum + 84 > width
            };
            let margin = if constrained { 12 } else { 42 };
            shadow_space.set_margin_top(margin);
            shadow_space.set_margin_bottom(margin);
            shadow_space.set_margin_start(margin);
            shadow_space.set_margin_end(margin);
            if constrained {
                content.add_css_class("modal-constrained");
                content.set_width_request(-1);
                content.set_halign(gtk::Align::Fill);
                shadow_space.set_halign(gtk::Align::Fill);
            } else {
                content.remove_css_class("modal-constrained");
                content.set_width_request(original_width);
                content.set_halign(gtk::Align::Center);
                shadow_space.set_halign(gtk::Align::Center);
            }
            // Keep overflow controls at the window edges, not beside the dialog.
            scroll.allocate(width.max(1), height.max(1), baseline, None);
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
    shadow_space.set_halign(gtk::Align::Center);
    shadow_space.set_valign(gtk::Align::Center);
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
